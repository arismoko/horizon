//! The debug keys in the game (26.3 `KeyboardHandler.keyPress`,
//! `handleDebugKeys` and `tick`): F3 combinations, F3+Escape, the held
//! debug-crash keys, and the debug renderers they switch on.
use super::Screen;
use std::time::{Duration, Instant};
use sunrise_client::{
    debug_keys::{CrashNotice, DebugAction, DebugHost, KeyAction, OpenScreen},
    debug_overlay,
    gizmos::Gizmos,
    hud::HudBatch,
    hud_layout, keybinds,
};
use sunrise_command::args::GameMode;
use sunrise_core::geometry::{Axis, Vec3};
use sunrise_core::nbt::Tag;
use sunrise_game_data::{
    Result,
    text::{ClickEvent, Style, Text},
};
use sunrise_protocol::packets::play::{
    BlockEntityTagQuery, DebugSubscriptionRequest, EntityTagQuery, Serverbound,
};
use winit::keyboard::KeyCode;

/// `ChatFormatting.YELLOW` and `RED`.
const YELLOW: u32 = 0xFF_FF55;
const RED: u32 = 0xFF_5555;

/// The overlay's CPU name. Reading it can start a process (`reg`, `sysctl`), which the render
/// thread shouldn't wait for, so [`warm_debug_cpu`] reads it ahead.
static DEBUG_CPU: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Reads the overlay's CPU name in the background when the window opens, for F3's first frame.
pub(in crate::game::client_side) fn warm_debug_cpu() {
    // A thread that can't start only leaves the read to that first frame.
    let _ = std::thread::Builder::new()
        .name("debug-cpu-name".into())
        .spawn(|| {
            DEBUG_CPU.get_or_init(sunrise_level::crash::cpu);
        });
}

/// The game as `KeyboardHandler` sees it while it runs one key event.
#[cfg(feature = "client")]
pub struct Host<'g, 'a> {
    game: &'g mut crate::game::ClientGame<'a>,
}

#[cfg(feature = "client")]
impl DebugHost for Host<'_, '_> {
    fn screen(&self) -> OpenScreen {
        self.game.open_screen()
    }
    fn toggle_overlay(&mut self) {
        self.game.set_debug_overlay(!self.game.client.debug);
    }
    fn pause_game(&mut self, without_menu: bool) {
        // Gui.setPauseScreen: PauseScreen(false) only when the game really
        // pauses (single player, not open to the network). Escape alone opens
        // the menu as it always has (key_pressed).
        let g = &mut *self.game;
        if without_menu && g.client.screen == Screen::Playing {
            g.client.pause_menu = !g.has_singleplayer_server() || g.is_published();
            g.client.pause_focus = None;
            g.set_screen(Screen::Paused);
            g.capture(false);
            g.set_client_mining(None);
        }
    }
    fn debug_action(&mut self, action: DebugAction, shift: bool) -> bool {
        self.game.run_debug_action(action, shift)
    }
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `DebugScreenEntryList.setOverlayVisible`: rebuild on each visibility transition.
    fn set_debug_overlay(&mut self, visible: bool) {
        if self.client.debug != visible {
            self.client.debug = visible;
            self.client.debug_entries.0.invalidate();
        }
    }

    fn open_screen(&self) -> OpenScreen {
        if self.client.title.0.is_some() {
            return OpenScreen::Other;
        }
        match self.client.screen {
            Screen::GameModeSwitcher => OpenScreen::GameModeSwitcher,
            Screen::Playing if self.client.book.is_none() && self.client.sign_edit.is_none() => {
                OpenScreen::None
            }
            Screen::Paused if !self.client.pause_menu => OpenScreen::PauseWithoutMenu,
            _ => OpenScreen::Other,
        }
    }
    /// Clear held debug keys on context-menu releases without firing debug or screen actions.
    #[inline(always)]
    pub(super) fn debug_key_release_cleanup(&mut self, key: KeyCode) {
        if !self.client.debug_keys.modifier_down && self.client.debug_keys.crash_key_time <= 0 {
            return;
        }
        let Some(name) = keybinds::key_name(key) else {
            return;
        };
        if name == self.client.settings.keys.get("key.debug.modifier") {
            self.client.debug_keys.modifier_down = false;
            self.client.debug_keys.used_debug_key_as_modifier = false;
        }
        if self.client.debug_keys.crash_key_time > 0 {
            let crash_down = keybinds::key_code(self.client.settings.keys.get("key.debug.crash"))
                .is_some_and(|k| self.client.physical_keys.contains(&k));
            if !crash_down || !self.client.debug_keys.modifier_down {
                self.client.debug_keys.crash_key_time = -1;
            }
        }
    }

    /// A physical key event (already in `physical_keys`) before the game's
    /// own handling. Returns true when it is used up (a debug combination,
    /// F3 itself, F3+Escape).
    pub fn debug_key(&mut self, key: KeyCode, action: KeyAction) -> bool {
        let Some(name) = keybinds::key_name(key) else {
            return false;
        };
        let pressed = action != KeyAction::Release;
        let switcher_open = self.client.screen == Screen::GameModeSwitcher;
        if switcher_open && self.context_menu_open() {
            if !pressed {
                self.debug_key_release_cleanup(key);
            }
            return false;
        }
        if switcher_open && pressed {
            if name == self.client.settings.keys.get("key.debug.switchGameMode") {
                if let Some(switcher) = &mut self.client.gamemode_switcher {
                    switcher.cycle();
                }
            } else if key == KeyCode::Escape {
                self.close_gamemode_switcher(false);
            } else if name == self.client.settings.keys.get("key.debug.debugOptions") {
                // The debug options screen interrupts the switcher too
                // (`canInterruptScreen`): it closes, then F6 opens over the game.
                self.close_gamemode_switcher(false);
                self.toggle_debug_options();
            } else if matches!(key, KeyCode::F2 | KeyCode::F11) {
                return false;
            }
            return true;
        }
        // Vanilla types into a focused EditBox through charTyped, which no
        // debug key can swallow. Here the character comes with the key press,
        // so a screen with a focused text box takes every press first;
        // releases still reach KeyboardHandler.
        let typing = matches!(
            self.client.screen,
            Screen::Chat
                | Screen::Options
                | Screen::WorldOptions
                | Screen::CommandBlock
                | Screen::BlockEditor
        ) || self.client.sign_edit.is_some()
            || self.client.book.is_some()
            || (self.client.screen == Screen::Creative
                && self.client.creative_screen.0.tab().search)
            || self.recipe_search_focused()
            || self.station_typing()
            || (self.title_input_active()
                && self
                    .client
                    .title
                    .0
                    .as_mut()
                    .is_some_and(|t| t.focused_field().is_some()));
        // Container screens use Escape, the inventory key and the hotbar and
        // drop keys first; any other screen closes on Escape.
        let screen_key = match self.client.screen {
            Screen::Inventory | Screen::Creative => matches!(
                key,
                KeyCode::Escape
                    | KeyCode::KeyE
                    | KeyCode::KeyF
                    | KeyCode::KeyQ
                    | KeyCode::Digit1
                    | KeyCode::Digit2
                    | KeyCode::Digit3
                    | KeyCode::Digit4
                    | KeyCode::Digit5
                    | KeyCode::Digit6
                    | KeyCode::Digit7
                    | KeyCode::Digit8
                    | KeyCode::Digit9
            ),
            Screen::Playing => false,
            _ => key == KeyCode::Escape,
        };
        if pressed && (typing || screen_key) {
            return false;
        }
        let crash_down = keybinds::key_code(self.client.settings.keys.get("key.debug.crash"))
            .is_some_and(|k| self.client.physical_keys.contains(&k));
        let shift = self.client.physical_keys.contains(&KeyCode::ShiftLeft)
            || self.client.physical_keys.contains(&KeyCode::ShiftRight);
        let now = self.debug_millis();
        let binds = self.client.settings.keys.clone();
        let mut keys = std::mem::take(&mut self.client.debug_keys);
        let modifier_was_down = keys.modifier_down;
        let to_game = keys.key_press(
            &mut Host { game: self },
            &binds,
            name,
            action,
            shift,
            crash_down,
            now,
        );
        self.client.debug_keys = keys;
        if switcher_open {
            if name == binds.get("key.debug.modifier") {
                self.close_gamemode_switcher(true);
                return true;
            }
            return false;
        }
        if !pressed {
            return false;
        }
        // The debug modifier does nothing else.
        if name == binds.get("key.debug.modifier") || name == binds.get("key.debug.overlay") {
            return true;
        }
        if self.open_screen() == OpenScreen::None {
            !to_game
        } else {
            modifier_was_down
        }
    }
    /// `Util.getMillis` (positive: KeyboardHandler treats 0 as unset).
    pub(in crate::game::client_side) fn debug_millis(&self) -> i64 {
        self.client.started.elapsed().as_millis() as i64 + 1_000_000
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_debug_millis(&self) -> i64 {
        self.debug_millis()
    }

    /// `KeyboardHandler.tick`: the debug crash countdown.
    pub(in crate::game::client_side) fn tick_debug_keys(&mut self) -> Result<()> {
        let now = self.debug_millis();
        match self.client.debug_keys.tick(now) {
            None => {}
            Some(CrashNotice::Message) => {
                let args = [
                    self.key_text("key.debug.modifier"),
                    self.key_text("key.debug.crash"),
                ];
                self.debug_feedback(Text::translate("debug.crash.message", &args));
            }
            Some(CrashNotice::Warning(seconds)) => self.debug_warning(Text::translate(
                "debug.crash.warning",
                &[Text::literal(seconds.to_string())],
            )),
            Some(CrashNotice::Crash) => {
                // Minecraft.emergencySaveAndCrash: the world is saved, then the
                // game stops; main writes the crash report.
                let crash = sunrise_level::crash::DebugCrash::new();
                if let Err(e) = self.save_for_exit() {
                    log::error!("the emergency save before the crash failed: {e}");
                }
                return Err(crash.into());
            }
        }
        Ok(())
    }
    /// The network charts' samples each tick: `BandwidthDebugMonitor.tick` always, and the
    /// integrated server's instant ping (a remote one is `tick_debug_connection`).
    pub(in crate::game::client_side) fn tick_debug_charts(&mut self) {
        let shown = self.client.debug && self.client.debug_charts.network;
        let Some(r) = &self.client.remote else {
            // The integrated server answers within the tick.
            if shown {
                self.client.debug_charts.ping.log(0);
            }
            return;
        };
        let received = r.session.counters.lock().unwrap().as_ref().map(|c| {
            let order = std::sync::atomic::Ordering::Relaxed;
            [
                c.bytes_in.load(order),
                c.sent.load(order),
                c.received.load(order),
            ]
        });
        if let Some([bytes, sent, received]) = received {
            self.client.debug_charts.log_bandwidth(bytes);
            self.client.debug_charts.log_packets([sent, received]);
        }
        r.session
            .drain_debug_pings(|delay| self.client.debug_charts.ping.log(delay));
    }
    /// `ClientPacketListener.tick` after the carried slot and the chat session: the ping while the
    /// network charts show, then, with a level, `ClientDebugSubscriber.tick`'s request.
    pub(in crate::game::client_side) fn tick_debug_connection(&mut self) {
        let Some(r) = &self.client.remote else {
            return;
        };
        let has_level = r.has_level();
        if self.client.debug && self.client.debug_charts.network {
            use sunrise_protocol::packets::status::Ping;
            let time = sunrise_client::net::ping_millis();
            self.send(Serverbound::PingRequest(Ping { time }));
        }
        if has_level
            && let Some(wanted) = self
                .client
                .debug_charts
                .subscription_request(self.client.debug)
        {
            // `dedicated_server_tick_time` is registry id 0.
            let subscriptions = if wanted { vec![0] } else { vec![] };
            self.send(Serverbound::DebugSubscriptionRequest(
                DebugSubscriptionRequest { subscriptions },
            ));
        }
    }
    /// The overlay's chart lines (`DebugScreenOverlay`'s "Debug charts").
    pub(in crate::game::client_side) fn debug_chart_lines(&self) -> [String; 3] {
        let keys = &self.client.settings.keys;
        let modifier = keys.get("key.debug.modifier");
        let key = |mapping: &str| {
            let prefix = if modifier == "key.keyboard.unknown" {
                String::new()
            } else {
                format!("{}+", keybinds::display_name(modifier).plain())
            };
            format!(
                "[{prefix}{}]",
                keybinds::display_name(keys.get(mapping)).plain()
            )
        };
        let chart = |mapping: &str, name: &str, on: bool| {
            format!(
                "{} {name} {}",
                key(mapping),
                if on { "visible" } else { "hidden" }
            )
        };
        // `DebugScreenOverlay`: the single-player variants for a local server.
        let remote = !self.has_singleplayer_server();
        let c = &self.client.debug_charts;
        [
            format!(
                "Debug charts: {}; {};",
                chart("key.debug.profilingChart", "Profiler", false),
                chart(
                    "key.debug.fpsCharts",
                    if remote { "fps" } else { "fps + tps" },
                    c.fps
                ),
            ),
            format!(
                "{}; {}",
                chart(
                    "key.debug.networkCharts",
                    if remote { "Bandwidth + Ping" } else { "Ping" },
                    c.network
                ),
                chart("key.debug.lightmapTexture", "Lightmap", c.lightmap),
            ),
            format!("To edit: press {}", key("key.debug.debugOptions")),
        ]
    }
    /// `Gui.extractRenderState`'s debug overlay, over everything the frame
    /// drew (menus too); the debug options screen draws it under itself.
    pub(in crate::game::client_side) fn draw_debug_overlay(
        &self,
        batch: &mut HudBatch,
        size: [u32; 2],
    ) {
        if self.client.screen != Screen::DebugOptions {
            self.draw_debug_overlay_under(batch, size);
        }
    }
    /// Whether this client's debug overlay passes the draw gate this frame.
    pub fn debug_overlay_visible(&self) -> bool {
        let screen_open = self.client.title.0.is_some() || self.client.screen != Screen::Playing;
        self.client.debug_entries.0.any_enabled(
            &self.client.registered_debug_entries.0,
            self.client.debug,
            self.reduced_debug_info(),
        ) && (!self.client.chat_ui.hide_gui || screen_open)
    }

    /// `DebugScreenOverlay.extractRenderState`: the enabled entries' lines
    /// (`debug-profile.json`), at the debug GUI scale, and the charts while
    /// the overlay is open. F1 hides it unless a screen is open. Sunrise paints
    /// it 20 times a second and copies those vertices in between.
    pub fn draw_debug_overlay_under(&self, batch: &mut HudBatch, size: [u32; 2]) {
        if !self.debug_overlay_visible() {
            // Nothing reads the server's lines now: it stops computing them each tick.
            if let Some(server) = &self.client.integrated {
                server.request_debug(None);
            }
            return;
        }
        let now = Instant::now();
        // The lightmap chart's pixels: none until the texture drawn now has been read
        // (`Renderer::lightmap_sample`). Whether there are any is part of the cache's key, so
        // a sample that stops being current is not painted again from it.
        let lightmap = self
            .client
            .renderer
            .0
            .as_ref()
            .filter(|_| self.debug_shows_lightmap())
            .and_then(|r| r.lightmap_sample());
        let layout = (
            size,
            batch.scale.to_bits(),
            self.client.gui.atlas.font.epoch(),
            lightmap.is_some(),
        );
        if let Some(vertices) = self.client.debug_paint.borrow().fresh(now, layout) {
            batch.vertices.extend_from_slice(vertices);
            return;
        }
        let first_vertex = batch.vertices.len();
        let framebuffer = size.map(|v| v as i32);
        let (scale, [width, height]) = debug_overlay::scale(
            framebuffer,
            batch.scale as i32,
            self.client.settings.file.int("debugGuiScale"),
            self.client.settings.force_unicode,
            [batch.width as i32, batch.height as i32],
        );
        let mut d = debug_overlay::Displayer::default();
        self.debug_entry_lines(&mut d, framebuffer);
        // The chart keys and the charts only while the overlay is open.
        let footer = if self.client.debug {
            self.debug_chart_lines().to_vec()
        } else {
            Vec::new()
        };
        let (left, right) = d.columns(&footer);
        let atlas = &self.client.gui.atlas;
        let mut draws = debug_overlay::draw(&atlas.font, [&left, &right], scale, width);
        if self.client.debug {
            let charts = self.client.debug_charts.draw(
                &atlas.font,
                width,
                height,
                self.client.settings.max_fps,
                &self.common.clock,
                !self.has_singleplayer_server(),
            );
            draws.push(hud_layout::GuiDraw::Push(vec![hud_layout::PoseOp::Scale(
                scale, scale,
            )]));
            draws.extend(charts);
            if let Some(pixels) = lightmap {
                draws.extend(sunrise_client::debug_chart::lightmap(pixels, width, height));
            }
            draws.push(hud_layout::GuiDraw::Pop);
        }
        hud_layout::paint(batch, atlas, &draws);
        self.client
            .debug_paint
            .borrow_mut()
            .store(now, layout, &batch.vertices[first_vertex..]);
    }
    /// Whether F3's lightmap chart (F3+4) is on screen: the overlay drawn, open and charting
    /// the lightmap. Only then does the renderer read a pack-drawn light texture back for it
    /// (`Renderer::set_lightmap_readback`).
    pub fn debug_shows_lightmap(&self) -> bool {
        self.debug_overlay_visible() && self.client.debug && self.client.debug_charts.lightmap
    }
    /// Whether the overlay is listing GPU pass times (its `system_specs` lines), the only reason
    /// it has to time passes.
    pub(in crate::game::client_side) fn debug_lists_gpu_passes(&self) -> bool {
        self.client.debug
            && self.client.title.0.is_none()
            && !(self.client.chat_ui.hide_gui && self.client.screen == Screen::Playing)
            && self.debug_entry("system_specs")
    }
    /// The monitor's refresh rate in millihertz, asked of the OS at most once a second (on
    /// Windows, winit calls `EnumDisplaySettingsExW` each time).
    fn debug_refresh_rate(&self) -> Option<u32> {
        if let Some((at, rate)) = self.client.debug_refresh.get()
            && at.elapsed() < Duration::from_secs(1)
        {
            return rate;
        }
        let rate = self
            .client
            .window
            .0
            .as_ref()
            .and_then(|w| w.current_monitor())
            .and_then(|m| m.refresh_rate_millihertz());
        self.client.debug_refresh.set(Some((Instant::now(), rate)));
        rate
    }
    /// `DebugEntryFps`.
    pub(in crate::game::client_side) fn debug_fps_line(&self, d: &mut debug_overlay::Displayer) {
        let limit = self.framerate_limit();
        let present = self
            .client
            .renderer
            .0
            .as_ref()
            .map_or("", |r| match r.config.present_mode {
                wgpu::PresentMode::Immediate => " (immediate)",
                wgpu::PresentMode::Mailbox => " (mailbox)",
                wgpu::PresentMode::Fifo => " (fifo)",
                wgpu::PresentMode::FifoRelaxed => " (fifo relaxed)",
                _ => "",
            });
        let refresh = self.debug_refresh_rate().map_or("0".into(), |mhz| {
            sunrise_client_input::window::refresh_rate_label(mhz as f32 / 1000.0)
        });
        // Sunrise: with frame generation running, the frames reaching the screen too.
        let generated = self
            .client
            .renderer
            .0
            .as_ref()
            .and_then(|r| r.frame_generation_share())
            .map_or(String::new(), |share| {
                format!(
                    " ({} fps FSR 3.1)",
                    (self.client.fps * (1.0 + share)).round() as i32
                )
            });
        d.priority(format!(
            "{} fps{generated} T: {}{present} @{refresh}Hz{}",
            self.client.fps.round() as i32,
            if limit == 260 {
                "inf".into()
            } else {
                limit.to_string()
            },
            self.debug_fps_suffix(),
        ));
    }

    /// The mods' texts for the frame rate line, each after a space, in mod load order (the order
    /// `set_debug_fps_text` keeps them in); a disabled mod's is left out. Written straight into
    /// the line, with nothing collected first.
    fn debug_fps_suffix(&self) -> impl std::fmt::Display + '_ {
        struct Suffix<'a>(std::cell::Ref<'a, Vec<(&'static str, String)>>);
        impl std::fmt::Display for Suffix<'_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                for (id, text) in self.0.iter() {
                    if mod_api::__private::mod_index(id)
                        .is_some_and(mod_api::__private::mod_index_on)
                    {
                        write!(f, " {text}")?;
                    }
                }
                Ok(())
            }
        }
        Suffix(self.client.debug_fps_texts.borrow())
    }

    /// `DebugEntryFps.display` with a mod's own text after vanilla's, as Dynamic FPS adds its
    /// state there: `text` replaces what the calling mod set before, and `None` or an empty text
    /// takes it away. Line breaks become spaces, as the line is one line. The texts are kept in
    /// mod load order, so the line is drawn without sorting. False, setting nothing, when no
    /// mod is calling.
    pub(in crate::game) fn set_debug_fps_text(&self, text: Option<&str>) -> bool {
        let Some(id) = mod_api::content::crash::current_native_mod() else {
            return false;
        };
        let mut texts = self.client.debug_fps_texts.borrow_mut();
        texts.retain(|(owner, _)| *owner != id);
        if let Some(text) = text.filter(|text| !text.is_empty()) {
            let index = mod_api::__private::mod_index(id);
            let at =
                texts.partition_point(|(owner, _)| mod_api::__private::mod_index(owner) <= index);
            texts.insert(at, (id, text.replace(['\n', '\r'], " ")));
        }
        true
    }
    /// `DebugEntrySimplePerformanceImpactors`.
    pub(in crate::game::client_side) fn debug_performance_lines(
        &self,
        d: &mut debug_overlay::Displayer,
    ) {
        let s = &self.client.settings;
        d.line(format!(
            "{}{}B: {}",
            if s.improved_transparency {
                "improved-transparency "
            } else {
                ""
            },
            match self.effective_clouds() {
                "false" => "",
                "fast" => "fast-clouds ",
                _ => "fancy-clouds ",
            },
            s.biome_blend
        ));
        d.line("Filtering: None");
        d.line(format!(
            "Terrain Rendering: {}",
            if self
                .client
                .renderer
                .0
                .as_ref()
                .is_some_and(|r| r.multi_draw)
            {
                "multidrawindirect"
            } else {
                "naive"
            }
        ));
    }
    /// `DebugEntrySystemSpecs` (no Java line: Sunrise has no JVM).
    pub(in crate::game::client_side) fn debug_system_lines(
        &self,
        d: &mut debug_overlay::Displayer,
        framebuffer: [i32; 2],
    ) {
        if let Some(r) = self.client.renderer.0.as_ref() {
            d.group(
                "system",
                [format!(
                    "CPU: {}",
                    DEBUG_CPU.get_or_init(sunrise_level::crash::cpu)
                )],
            );
            let info = &r.info;
            d.group(
                "system",
                [format!(
                    "Display: {}x{} ({})",
                    framebuffer[0],
                    framebuffer[1],
                    // Metal adapters report no PCI vendor.
                    if info.vendor == 0 && info.backend == wgpu::Backend::Metal {
                        "Apple".into()
                    } else {
                        debug_overlay::vendor_name(info.vendor)
                    }
                )],
            );
            if let Some(w) = self.client.window.0.as_ref() {
                let density = w.scale_factor();
                let logical = winit::dpi::PhysicalSize::new(framebuffer[0], framebuffer[1])
                    .to_logical::<f64>(density);
                d.group(
                    "system",
                    [format!(
                        "Window: {}x{} ({}x pixel density)",
                        logical.width.round() as i32,
                        logical.height.round() as i32,
                        sunrise_core::java_number::format_fixed2(f64::from(density as f32))
                    )],
                );
            }
            // Dynamic Resolution's scale, which moves on its own.
            if self.client.settings.sunrise.dynamic_resolution {
                let scale = super::frame_autotune::world_scale(
                    &self.client.settings.sunrise,
                    &self.client.dynamic_resolution,
                );
                d.group("system", [format!("Resolution Scale: {scale}% (dynamic)")]);
            }
            let kind = match info.device_type {
                wgpu::DeviceType::IntegratedGpu => " (iGPU)",
                wgpu::DeviceType::DiscreteGpu => " (dGPU)",
                wgpu::DeviceType::VirtualGpu => " (vGPU)",
                wgpu::DeviceType::Cpu => " (software)",
                wgpu::DeviceType::Other => "",
            };
            let driver = format!("{} {}", info.driver, info.driver_info);
            d.group(
                "system",
                [
                    format!("{}{kind}", info.name),
                    format!(
                        "{:?} {}",
                        info.backend,
                        driver.trim().lines().next().unwrap_or("")
                    )
                    .trim_end()
                    .to_owned(),
                ],
            );
        }
        // Sunrise's own: HDR's output while the setting is on, under the GPU (a group of its
        // own would move others to the other column).
        if let Some(r) = self.client.renderer.0.as_ref()
            && r.output().requests_hdr(self.client.settings.sunrise.hdr)
        {
            d.group("system", [r.hdr_line()]);
        }
    }
    /// `DebugEntryTps` for local and remote connections.
    pub(in crate::game::client_side) fn debug_tps_line(&self, d: &mut debug_overlay::Displayer) {
        if self.common.player.is_some() {
            use sunrise_core::java_number::format_fixed;
            let [tx, rx] = self
                .client
                .debug_charts
                .packet_rates
                .map(|n| format_fixed(f64::from(n), 0));
            let (target, status) = if self.common.clock.sprinting() {
                ("-".to_owned(), " (sprinting)")
            } else {
                let target = format_fixed(f64::from(1000.0 / self.common.clock.rate()), 1);
                let status = if self.common.clock.stepping() {
                    " (frozen - stepping)"
                } else if self.common.clock.frozen() {
                    " (frozen)"
                } else {
                    ""
                };
                (target, status)
            };
            if let Some(server) = &self.client.integrated {
                let tick_ms = server.smoothed_tick_ms();
                d.line(format!(
                    "Integrated server @ {}/{target} ms{status}, {tx} tx, {rx} rx",
                    format_fixed(f64::from(tick_ms), 1),
                ));
            } else if let Some(remote) = &self.client.remote {
                d.line(format!(
                    "\"{}\" server{status}, {tx} tx, {rx} rx",
                    remote.brand
                ));
            }
        }
    }
    /// `KeyMapping.getTranslatedKeyMessage`.
    fn key_text(&self, mapping: &str) -> Text {
        keybinds::display_name(self.client.settings.keys.get(mapping))
    }
    /// `KeyboardHandler.decorateDebugComponent`: `[Debug]:` in bold, a space,
    /// the message, shown with `Minecraft.showDebugChat`.
    fn debug_decorated(&mut self, color: u32, message: Text) {
        let prefix = Text::translate("debug.prefix", &[])
            .with_color(color)
            .bold();
        self.show_debug_chat(
            Text::empty()
                .append(prefix)
                .append(Text::literal(" "))
                .append(message),
        );
    }
    /// `Minecraft.showDebugChat`: a client system message the narrator says.
    fn show_debug_chat(&mut self, message: Text) {
        self.say_system_queued(&message);
        self.client_system_message(message);
    }
    fn debug_feedback(&mut self, message: Text) {
        self.debug_decorated(YELLOW, message);
    }
    fn debug_warning(&mut self, message: Text) {
        self.debug_decorated(RED, message);
    }
    fn debug_translated(&mut self, key: &str) {
        self.debug_feedback(Text::translate(key, &[]));
    }
    /// `LocalPlayer.isReducedDebugInfo`: the server's rule or the option.
    pub(in crate::game::client_side) fn reduced_debug_info(&self) -> bool {
        self.client.settings.reduced_debug_info
            || self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::REDUCED_DEBUG_INFO)
            || self
                .client
                .remote
                .as_ref()
                .is_some_and(|r| r.reduced_debug_info)
    }
    /// `DebugScreenEntryList.isCurrentlyEnabled` for an entry's path.
    pub(in crate::game::client_side) fn debug_entry(&self, path: &str) -> bool {
        self.client.debug_entries.0.enabled(
            &self.client.registered_debug_entries.0,
            path,
            self.client.debug,
            self.reduced_debug_info(),
        )
    }
    /// The `3d_crosshair` entry.
    pub(in crate::game::client_side) fn debug_crosshair(&self) -> bool {
        self.debug_entry("3d_crosshair")
    }
    /// `Permissions.COMMANDS_GAMEMASTER` (`GameModeCommand.PERMISSION_CHECK`).
    pub(in crate::game::client_side) fn gamemaster(&self) -> bool {
        self.common.ui.registries.permission >= 2
    }
    fn toggle_option(&mut self, key: &str) -> bool {
        let on = !self.client.settings.file.bool(key);
        self.client
            .settings
            .file
            .set(key, sunrise_client::options_file::V::Bool(on));
        self.client
            .settings
            .save(&sunrise_gameplay_common::game::options_path());
        on
    }
    /// One matching mapping of `handleDebugKeys`; whether it counts.
    fn run_debug_action(&mut self, action: DebugAction, shift: bool) -> bool {
        let player = self.common.player.is_some() && !self.reduced_debug_info();
        match action {
            DebugAction::ReloadChunks => {
                self.all_sections_changed();
                self.debug_translated("debug.reload_chunks.message");
                true
            }
            DebugAction::ShowHitboxes if player => {
                let on = self
                    .client
                    .debug_entries
                    .0
                    .toggle_status("entity_hitboxes", self.client.debug);
                self.debug_translated(if on {
                    "debug.show_hitboxes.on"
                } else {
                    "debug.show_hitboxes.off"
                });
                true
            }
            DebugAction::ClearChat => {
                // ChatComponent.clearMessages(false): sent history stays.
                self.client.chat_ui.chat.clear();
                true
            }
            DebugAction::ShowChunkBorders if player => {
                let on = self
                    .client
                    .debug_entries
                    .0
                    .toggle_status("chunk_borders", self.client.debug);
                self.debug_translated(if on {
                    "debug.chunk_boundaries.on"
                } else {
                    "debug.chunk_boundaries.off"
                });
                true
            }
            DebugAction::AdvancedTooltips => {
                // The message comes before Options.save here.
                let on = !self.client.settings.file.bool("advancedItemTooltips");
                self.debug_translated(if on {
                    "debug.advanced_tooltips.on"
                } else {
                    "debug.advanced_tooltips.off"
                });
                self.toggle_option("advancedItemTooltips");
                true
            }
            DebugAction::CopyRecreateCommand => {
                if player {
                    self.copy_recreate_command(self.gamemaster(), !shift);
                }
                true
            }
            DebugAction::Spectate => {
                if self.common.player.is_none() || !self.gamemaster() {
                    self.debug_translated("debug.creative_spectator.error");
                } else if !self.own_player_is_spectator() {
                    self.change_game_mode(GameMode::Spectator);
                } else {
                    let back = self.common.previous_mode.unwrap_or(GameMode::Creative);
                    self.change_game_mode(back);
                }
                true
            }
            DebugAction::SwitchGameMode if self.open_screen() == OpenScreen::None => {
                if self.common.player.is_none() || !self.gamemaster() {
                    self.debug_translated("debug.gamemodes.error");
                } else {
                    self.open_gamemode_switcher();
                }
                true
            }
            DebugAction::FocusPause => {
                let on = self.toggle_option("pauseOnLostFocus");
                self.debug_translated(if on {
                    "debug.pause_focus.on"
                } else {
                    "debug.pause_focus.off"
                });
                true
            }
            DebugAction::ReloadResourcePacks => {
                self.debug_translated("debug.reload_resourcepacks.message");
                self.reload_packs();
                true
            }
            DebugAction::CopyLocation if player => {
                self.debug_translated("debug.copy_location.message");
                if let Some(p) = &self.common.player {
                    use sunrise_core::java_number::format_fixed2 as f2;
                    let command = format!(
                        "/execute in {} run tp @s {} {} {} {} {}",
                        self.dim.id(),
                        f2(p.position.x),
                        f2(p.position.y),
                        f2(p.position.z),
                        f2(f64::from(self.yaw)),
                        f2(f64::from(self.pitch)),
                    );
                    self.set_clipboard(&command);
                }
                true
            }
            DebugAction::DumpVersion => {
                self.debug_translated("debug.version.header");
                for line in self.common.version_lines() {
                    self.show_debug_chat(line);
                }
                true
            }
            DebugAction::ImprovedTransparency => {
                self.client.settings.improved_transparency =
                    !self.client.settings.improved_transparency;
                self.client
                    .settings
                    .save(&sunrise_gameplay_common::game::options_path());
                self.apply_settings();
                self.debug_translated(if self.client.settings.improved_transparency {
                    "debug.improvedTransparency.on"
                } else {
                    "debug.improvedTransparency.off"
                });
                true
            }
            DebugAction::FpsCharts => {
                let mut visible = self.client.debug;
                self.client.debug_charts.toggle_fps(&mut visible);
                self.set_debug_overlay(visible);
                true
            }
            DebugAction::NetworkCharts => {
                let mut visible = self.client.debug;
                self.client.debug_charts.toggle_network(&mut visible);
                self.set_debug_overlay(visible);
                true
            }
            DebugAction::LightmapTexture => {
                let mut visible = self.client.debug;
                self.client.debug_charts.toggle_lightmap(&mut visible);
                self.set_debug_overlay(visible);
                true
            }
            // Other title submenus still need their own interruption/close policy.
            DebugAction::DebugOptions
                if self.client.title.0.as_ref().is_some_and(|title| {
                    !matches!(title.mode, sunrise_client::title::Mode::Main)
                }) =>
            {
                false
            }
            DebugAction::DebugOptions => {
                self.toggle_debug_options();
                true
            }
            // Sunrise's own: a repro recording for a bug report (game/runtime/repro.rs).
            // Off unless the AI Agent page enables the shortcuts.
            DebugAction::RecordSession if !self.client.settings.extras.repro_keys => false,
            DebugAction::ExportRepro if !self.client.settings.extras.repro_keys => false,
            DebugAction::RecordSession => {
                self.toggle_recording_key();
                true
            }
            DebugAction::ExportRepro => {
                self.export_repro_key();
                true
            }
            DebugAction::DumpDynamicTextures => {
                // `KeyboardHandler`: `TextureUtil.getDebugTexturePath` is the game
                // folder's `screenshots/debug`; `TextureManager.dumpAllSheets` then
                // writes every dumpable texture there and the chat names the folder.
                let dir = sunrise_level::logging::player_dir()
                    .join("screenshots")
                    .join("debug");
                let atlas = self
                    .client
                    .gui
                    .atlas
                    .dump_contents(&dir, "minecraft_textures_atlas_gui");
                // Each client mod's dynamic texture is a GPU texture of its own
                // (`DynamicTexture.dumpContents`), read back beside the atlas.
                if let Some(renderer) = &self.client.renderer.0
                    && let Err(e) = renderer.hud.dynamic().dump_contents(
                        &renderer.device,
                        &renderer.queue,
                        self.client.dynamic_textures.live(),
                        &dir,
                    )
                {
                    log::error!("Failed to dump mod dynamic textures: {e}");
                }
                match atlas {
                    Ok(_) => {
                        log::info!("Dumped dynamic textures to {}", dir.display());
                        let link = Text::styled(
                            "screenshots/debug",
                            Style {
                                underlined: Some(true),
                                click: Some(ClickEvent::OpenFile(
                                    dir.to_string_lossy().into_owned(),
                                )),
                                ..Style::default()
                            },
                        );
                        self.debug_feedback(Text::translate(
                            "debug.dump_dynamic_textures",
                            &[link],
                        ));
                    }
                    Err(e) => log::error!("Failed to dump dynamic textures: {e}"),
                }
                true
            }
            // Screens and overlays Sunrise does not draw yet (the profiler and
            // its chart): the keys are used up as in vanilla.
            DebugAction::Profiling | DebugAction::ProfilingChart => true,
            DebugAction::ShowHitboxes
            | DebugAction::ShowChunkBorders
            | DebugAction::SwitchGameMode
            | DebugAction::CopyLocation => false,
        }
    }
    /// Send `ServerboundChangeGameModePacket` to the server.
    pub(in crate::game) fn change_game_mode(&mut self, mode: GameMode) {
        debug_assert!(
            !self.is_logical_server(),
            "change_game_mode called on Server role"
        );
        use sunrise_protocol::packets::play::ChangeGameMode;
        let mode = match mode {
            GameMode::Survival => 0,
            GameMode::Creative => 1,
            GameMode::Adventure => 2,
            GameMode::Spectator => 3,
        };
        self.send(Serverbound::ChangeGameMode(ChangeGameMode { mode }));
    }
    /// `KeyboardHandler.copyRecreateCommand`: the targeted block or mob as a
    /// `/setblock` or `/summon` command. The server's data (no Shift) comes
    /// from the server the game is connected to, the integrated one included
    /// (`DebugQueryHandler`); without one, the game's own data is the server's.
    fn copy_recreate_command(&mut self, add_nbt: bool, from_server: bool) {
        let query = add_nbt && from_server && self.client.remote.is_some();
        let side = if add_nbt && from_server {
            "server"
        } else {
            "client"
        };
        if self.entity_targeted()
            && let Some((index, _, _)) = self.mob_hit()
        {
            let (kind, position, id) = {
                let m = &self.common.mobs[index];
                (m.kind.id.clone(), m.position, m.id)
            };
            if query {
                // DebugQueryHandler.queryEntityTag: the answer copies it.
                let reply = QueryReply::Entity { id, kind, position };
                if let Some(transaction) = self
                    .client
                    .remote
                    .as_mut()
                    .map(|r| r.debug_query.start(reply))
                {
                    self.send(Serverbound::EntityTagQuery(EntityTagQuery {
                        transaction,
                        entity: id as i32,
                    }));
                }
                return;
            }
            // Entity.saveWithoutId.
            let tag = None;
            self.copy_create_entity_command(&kind, position, tag);
            self.debug_translated(&format!("debug.inspect.{side}.entity"));
            return;
        }
        let Some(hit) = self.pick() else {
            return;
        };
        let pos = hit.pos;
        let state = self
            .common
            .state_string(self.common.world.block(pos[0], pos[1], pos[2]));
        if query {
            // DebugQueryHandler.queryBlockEntityTag: the answer copies it.
            let reply = QueryReply::Block { pos, state };
            if let Some(transaction) = self
                .client
                .remote
                .as_mut()
                .map(|r| r.debug_query.start(reply))
            {
                self.send(Serverbound::BlockEntityTagQuery(BlockEntityTagQuery {
                    transaction,
                    pos,
                }));
            }
            return;
        }
        let tag = if add_nbt {
            self.common
                .world
                .block_entities
                .get(&(pos[0], pos[1], pos[2]))
                .map(|e| {
                    let name = self.common.data_block_name(pos);
                    let mut tag =
                        self.client_encode_live_block_entity((pos[0], pos[1], pos[2]), e, &name);
                    if let Tag::Compound(c) = &mut tag {
                        for key in ["id", "x", "y", "z", "keepPacked"] {
                            c.remove(&key.into());
                        }
                    }
                    tag
                })
        } else {
            None
        };
        self.copy_create_block_command(pos, &state, tag.as_ref());
        self.debug_translated(&format!("debug.inspect.{side}.block"));
    }
    /// `ClientPacketListener.handleTagQueryPacket`: the server's answer to
    /// the last debug query (`DebugQueryHandler.handleResponse`) completes
    /// it; an answer to an older query is ignored.
    pub(in crate::game::client_side) fn answer_tag_query(&mut self, transaction: i32, tag: Tag) {
        let Some(reply) = self
            .client
            .remote
            .as_mut()
            .and_then(|r| r.debug_query.answer(transaction))
        else {
            return;
        };
        // FriendlyByteBuf.readNbt: an End tag is no compound.
        let tag = match tag {
            Tag::End => None,
            tag => Some(tag),
        };
        match reply {
            QueryReply::Block { pos, state } => {
                self.copy_create_block_command(pos, &state, tag.as_ref());
                self.debug_translated("debug.inspect.server.block");
            }
            QueryReply::Entity { id, kind, position } => {
                // Entity.position() when the answer comes: where it is now,
                // else where it was last.
                let position = self
                    .common
                    .mobs
                    .iter()
                    .find(|m| m.id == id)
                    .map_or(position, |m| m.position);
                self.copy_create_entity_command(&kind, position, tag);
                self.debug_translated("debug.inspect.server.entity");
            }
        }
    }
    /// `KeyboardHandler.copyCreateBlockCommand`.
    fn copy_create_block_command(&mut self, pos: [i32; 3], state: &str, tag: Option<&Tag>) {
        let mut description = state.to_owned();
        if let Some(tag) = tag {
            description.push_str(&sunrise_game_data::snbt::to_snbt(tag));
        }
        self.set_clipboard(&format!(
            "/setblock {} {} {} {description}",
            pos[0], pos[1], pos[2]
        ));
    }
    /// `KeyboardHandler.copyCreateEntityCommand`: the data without its UUID
    /// or position.
    fn copy_create_entity_command(&mut self, kind: &str, pos: Vec3, tag: Option<Tag>) {
        use sunrise_core::java_number::format_fixed2 as f2;
        let mut command = format!("/summon {kind} {} {} {}", f2(pos.x), f2(pos.y), f2(pos.z));
        if let Some(mut tag) = tag {
            if let Tag::Compound(c) = &mut tag {
                for key in ["UUID", "Pos"] {
                    c.remove(&key.into());
                }
            }
            command.push(' ');
            command.push_str(&sunrise_game_data::snbt::pretty(&tag).plain());
        }
        self.set_clipboard(&command);
    }
    /// `KeyboardHandler.setClipboard`.
    fn set_clipboard(&mut self, text: &str) {
        use sunrise_client::clipboard::Clipboard;
        self.client.clipboard.0.set(text);
    }
    /// The debug renderers' gizmos for a frame (`DebugRenderer.emitGizmos`).
    pub fn debug_gizmos(&self, alpha: f32, camera: [f64; 3]) -> Gizmos {
        let mut g = Gizmos::default();
        self.bounding_box_gizmos(camera, &mut g);
        self.debug_renderer_gizmos(&mut g);
        if self.debug_entry("chunk_borders")
            && let Some(pos) = self.camera_entity_position()
        {
            let block = [
                pos[0].floor() as i32,
                pos[1].floor() as i32,
                pos[2].floor() as i32,
            ];
            let min_y = self.common.world.bottom_y();
            g.chunk_borders(
                block,
                min_y,
                min_y + self.common.world.layout().height() - 1,
            );
        }
        if !self.debug_entry("entity_hitboxes") {
            return g;
        }
        let hide_camera = self
            .client
            .camera_type
            .is_first_person()
            .then(|| {
                self.client
                    .remote
                    .as_ref()
                    .filter(|r| r.detached_camera.is_none())
                    .and_then(|r| r.camera)
                    .map(|id| id as u32)
            })
            .flatten();
        let lerp = |a: f64, b: f64| a + f64::from(alpha) * (b - a);
        let lerpf = |a: f32, b: f32| a + alpha * (b - a);
        for m in self.common.mobs.iter() {
            if m.removed || m.is_invisible() || hide_camera == Some(m.id) {
                continue;
            }
            let pos = [
                lerp(m.previous.x, m.position.x),
                lerp(m.previous.y, m.position.y),
                lerp(m.previous.z, m.position.z),
            ];
            let view = (
                lerpf(m.previous_head_pitch, m.head_pitch),
                lerpf(m.previous_head_yaw, m.head_yaw),
            );
            g.hitbox(
                pos,
                (m.width(), m.height()),
                m.eye_height() as f32,
                true,
                view,
            );
        }
        // A client's copy of the server's entity: the size its `entity_size` answer gave it
        // (`refreshDimensions`), else its kind's.
        let sized = |id: u32, size: (f32, f32), eye: f32| match self.client_copy_answer(id) {
            Some([width, height, eye]) => ((width, height), eye),
            None => (size, eye),
        };
        for item in &self.common.dropped {
            if hide_camera == Some(item.id) {
                continue;
            }
            let pos = [
                lerp(item.previous.x, item.position.x),
                lerp(item.previous.y, item.position.y),
                lerp(item.previous.z, item.position.z),
            ];
            // EntityType.ITEM: 0.25 by 0.25, eyes at 0.2125.
            let (size, eye) = sized(item.id, (0.25, 0.25), 0.2125);
            g.hitbox(pos, size, eye, false, (0.0, 0.0));
        }
        // The other entities (EntityTypes sizes; eyes at 0.85 of the height
        // unless the type sets one). `at` is the interpolated position.
        let at = |previous: Vec3, now: Vec3| {
            [
                lerp(previous.x, now.x),
                lerp(previous.y, now.y),
                lerp(previous.z, now.z),
            ]
        };
        for p in &self.common.paintings {
            if hide_camera == Some(p.id) {
                continue;
            }
            let (min, max) = p.bounds();
            let centre = std::array::from_fn(|i| (min[i] + max[i]) / 2.0);
            g.entity_box(min, max, centre, 0.425, false, step_view(p.step()));
        }
        for f in self
            .common
            .item_frames
            .iter()
            .filter(|f| !f.invisible && hide_camera != Some(f.id))
        {
            let (min, max) = f.bounds();
            let centre = std::array::from_fn(|i| (min[i] + max[i]) / 2.0);
            g.entity_box(min, max, centre, 0.0, false, step_view(f.step()));
        }
        for c in &self.common.cushions {
            if hide_camera == Some(c.id) {
                continue;
            }
            let (min, max) = c.bounds();
            let p = c.position;
            g.entity_box(min, max, [p.x, p.y, p.z], 0.2125, false, (0.0, c.yaw));
        }
        for (&id, i) in &self.client.interactions {
            if hide_camera == Some(id as u32) {
                continue;
            }
            let pos = at(i.previous, i.position);
            // Its `entity_size` answer's size on a client's copy, else its synched one.
            let (width, height, eye) = self
                .client_copy_answer(id as u32)
                .map_or((i.width, i.height, i.eye_height()), |[w, h, e]| (w, h, e));
            let half = f64::from(width) / 2.0;
            let b = sunrise_core::geometry::Aabb::new(
                Vec3::new(pos[0] - half, pos[1], pos[2] - half),
                Vec3::new(pos[0] + half, pos[1] + f64::from(height), pos[2] + half),
            );
            g.entity_box(
                [b.min(Axis::X), b.min(Axis::Y), b.min(Axis::Z)],
                [b.max(Axis::X), b.max(Axis::Y), b.max(Axis::Z)],
                pos,
                eye,
                false,
                (
                    lerpf(i.previous_pitch, i.pitch),
                    lerpf(i.previous_yaw, i.yaw),
                ),
            );
        }
        for a in self
            .common
            .armor_stands
            .iter()
            .filter(|a| !a.invisible && hide_camera != Some(a.id))
        {
            // ArmorStand.getDimensions: a marker has none, a small one is
            // half scale with its eyes at 0.9875.
            let (size, eye) = match (a.marker, a.small) {
                (true, _) => ((0.0, 0.0), 0.0),
                (_, true) => ((0.25, 0.9875), 0.9875),
                _ => ((0.5, 1.975), 1.7775),
            };
            let (size, eye) = sized(a.id, size, eye);
            g.hitbox(
                at(a.previous, a.position),
                size,
                eye,
                true,
                (0.0, a.render_yaw(alpha)),
            );
        }
        for b in &self.common.boats {
            if hide_camera == Some(b.id) {
                continue;
            }
            let h = sunrise_gameplay_common::game::boats_common::HEIGHT as f32;
            let (size, eye) = sized(
                b.id,
                (sunrise_gameplay_common::game::boats_common::WIDTH as f32, h),
                h,
            );
            g.hitbox(at(b.previous, b.position), size, eye, false, (0.0, b.yaw));
        }
        for c in &self.common.carts {
            if hide_camera == Some(c.id) {
                continue;
            }
            let (w, h) = (
                super::minecarts::WIDTH as f32,
                super::minecarts::HEIGHT as f32,
            );
            let (size, eye) = sized(c.id, (w, h), h * 0.85);
            g.hitbox(at(c.previous, c.position), size, eye, false, (0.0, c.yaw));
        }
        for o in &self.common.orbs {
            if hide_camera == Some(o.id) {
                continue;
            }
            let (size, eye) = sized(o.id, (0.5, 0.5), 0.425);
            g.hitbox(at(o.previous, o.position), size, eye, false, (0.0, 0.0));
        }
        for f in &self.common.falling {
            if hide_camera == Some(f.id) {
                continue;
            }
            let (size, eye) = sized(f.id, (0.98, 0.98), 0.833);
            g.hitbox(at(f.previous, f.position), size, eye, false, (0.0, 0.0));
        }
        for t in &self.common.tnts {
            if hide_camera == Some(t.id) {
                continue;
            }
            let (size, eye) = sized(t.id, (0.98, 0.98), 0.15);
            g.hitbox(at(t.previous, t.position), size, eye, false, (0.0, 0.0));
        }
        for c in &self.common.crystals {
            if hide_camera == Some(c.id) {
                continue;
            }
            let p = c.position;
            let (size, eye) = sized(c.id, (2.0, 2.0), 1.7);
            g.hitbox([p.x, p.y, p.z], size, eye, false, (0.0, 0.0));
        }
        for p in &self.common.creatures.projectiles {
            if hide_camera == Some(p.id) {
                continue;
            }
            use sunrise_sim::projectiles::ProjectileKind as K;
            let (size, eye) = match p.kind {
                K::Arrow { .. } | K::Trident => (0.5, 0.13),
                K::Snowball
                | K::Egg
                | K::EnderPearl
                | K::LlamaSpit
                | K::Potion { .. }
                | K::ExperienceBottle
                | K::Thrown(_) => (0.25, 0.2125),
                K::SmallFireball | K::ShulkerBullet { .. } | K::WitherSkull { .. } => {
                    (0.3125, 0.265_625)
                }
                K::WindCharge | K::BreezeWindCharge => (0.3125, 0.0),
                K::LargeFireball { .. } | K::DragonFireball => (1.0, 0.85),
            };
            let (size, eye) = sized(p.id, (size, size), eye);
            g.hitbox(
                at(p.previous, p.position),
                size,
                eye,
                false,
                (p.pitch, p.yaw),
            );
        }
        for t in &self.common.tridents {
            if hide_camera == Some(t.id) {
                continue;
            }
            let (size, eye) = sized(t.id, (0.5, 0.5), 0.13);
            g.hitbox(
                at(t.previous, t.position),
                size,
                eye,
                false,
                (t.pitch, t.yaw),
            );
        }
        // EntityType.EYE_OF_ENDER: 0.25 by 0.25, never turned.
        for (previous, now) in self.common.end.eye_motions() {
            let v = |p: glam::DVec3| Vec3::new(p.x, p.y, p.z);
            g.hitbox(
                at(v(previous), v(now)),
                (0.25, 0.25),
                0.2125,
                false,
                (0.0, 0.0),
            );
        }
        for r in &self.common.rockets {
            if hide_camera == Some(r.id) {
                continue;
            }
            // Projectile.updateRotation: the look follows the motion.
            let v = r.velocity;
            let yaw = v.x.atan2(v.z).to_degrees() as f32;
            let pitch = v.y.atan2(v.x.hypot(v.z)).to_degrees() as f32;
            let (size, eye) = sized(r.id, (0.25, 0.25), 0.2125);
            g.hitbox(at(r.previous, r.position), size, eye, false, (pitch, yaw));
        }
        for h in self
            .common
            .hook
            .iter()
            .map(|h| (h.previous, h.position))
            .chain(
                self.client
                    .remote_hooks
                    .iter()
                    .map(|h| (h.previous, h.position)),
            )
        {
            g.hitbox(at(h.0, h.1), (0.25, 0.25), 0.2125, false, (0.0, 0.0));
        }
        for b in &self.common.bolts {
            g.hitbox(b.at, (0.0, 0.0), 0.0, false, (0.0, 0.0));
        }
        // AreaEffectCloud.getDimensions: diameter follows synced radius,
        // height is 0.5; EntityDimensions.scalable gives eyes at 85% height.
        for c in &self.client.remote_clouds {
            let (size, eye) = sized(c.id, (c.radius * 2.0, 0.5), 0.425);
            g.hitbox(c.at, size, eye, false, (0.0, 0.0));
        }
        // The dragon: 16 by 8, a living entity; its parts follow.
        if let Some(d) = self
            .common
            .dragon
            .as_ref()
            .filter(|d| hide_camera != Some(d.id))
        {
            g.hitbox(
                at(d.previous, d.position),
                (16.0, 8.0),
                6.8,
                true,
                (0.0, d.yaw),
            );
        }
        // The dragon's parts.
        for (id, b) in self.common.special_targets() {
            if (sunrise_gameplay_common::game::dragon_common::SPECIAL
                ..sunrise_gameplay_common::game::dragon_common::SPECIAL + 8)
                .contains(&id)
            {
                g.cuboid(
                    [b.min(Axis::X), b.min(Axis::Y), b.min(Axis::Z)],
                    [b.max(Axis::X), b.max(Axis::Y), b.max(Axis::Z)],
                    sunrise_client::gizmos::DRAGON_PART,
                    sunrise_client::gizmos::STROKE_WIDTH,
                );
            }
        }
        // The player's own, unless the camera is in its head.
        if (!self.client.camera_type.is_first_person() || !self.controls_camera())
            && let Some(p) = &self.common.player
        {
            let d = p.dimensions;
            let pos = [
                lerp(self.common.previous_position.x, p.position.x),
                lerp(self.common.previous_position.y, p.position.y),
                lerp(self.common.previous_position.z, p.position.z),
            ];
            g.hitbox(
                pos,
                (d.width, d.height),
                d.eye_height,
                true,
                (self.common.pitch, self.common.yaw),
            );
        }
        g
    }
}

/// The (x, y) rotations whose view vector is the step of a direction.
fn step_view(step: [i32; 3]) -> (f32, f32) {
    let [x, y, z] = step.map(f64::from);
    (
        (-y).asin().to_degrees() as f32,
        (-x.atan2(z)).to_degrees() as f32,
    )
}

/// `DebugQueryHandler`: the last debug NBT query (F3+I) sent to the server
/// and what its answer completes. Only the newest query is answered.
#[derive(Debug)]
pub(in crate::game) struct DebugQuery {
    transaction: i32,
    pending: Option<QueryReply>,
}
impl Default for DebugQuery {
    fn default() -> Self {
        Self {
            transaction: -1,
            pending: None,
        }
    }
}
/// What a debug query's answer is copied into: the block command
/// (`copyCreateBlockCommand`) or the entity's (`copyCreateEntityCommand`).
#[derive(Clone, Debug, PartialEq)]
pub(in crate::game) enum QueryReply {
    Block {
        pos: [i32; 3],
        state: String,
    },
    Entity {
        id: u32,
        kind: String,
        position: Vec3,
    },
}
impl DebugQuery {
    /// `DebugQueryHandler.startTransaction`: the next transaction id, which
    /// replaces any query still waiting.
    fn start(&mut self, reply: QueryReply) -> i32 {
        self.pending = Some(reply);
        self.transaction = self.transaction.wrapping_add(1);
        self.transaction
    }
    /// `DebugQueryHandler.handleResponse`: what the answer to `transaction`
    /// completes, once, when it answers the newest query.
    fn answer(&mut self, transaction: i32) -> Option<QueryReply> {
        if transaction == self.transaction {
            self.pending.take()
        } else {
            None
        }
    }
}

/// `DebugEntryChunkGeneration`'s `lastPos` and `result`: F3's `Density` line is sampled again
/// only when the feet block or the dimension changes. Vanilla adds the group only inside its
/// server-level branch, so with no generator (a remote server, a LAN world joined, the server
/// thread not there yet) nothing is drawn and nothing of the last world is kept.
#[derive(Default)]
pub(in crate::game) struct DensityCache(Option<(&'static str, [i32; 3], Option<String>)>);
impl DensityCache {
    /// The line to draw; `sample` is `None` without a generator.
    pub(in crate::game::client_side) fn line(
        &mut self,
        dim: &'static str,
        feet: [i32; 3],
        sample: Option<impl FnOnce() -> Option<String>>,
    ) -> Option<&str> {
        let Some(sample) = sample else {
            self.0 = None;
            return None;
        };
        if !self
            .0
            .as_ref()
            .is_some_and(|(d, f, _)| *d == dim && *f == feet)
        {
            self.0 = Some((dim, feet, sample()));
        }
        self.0.as_ref().and_then(|(_, _, line)| line.as_deref())
    }
    /// Another world: its generator, not the last one's line.
    pub(in crate::game::client_side) fn clear(&mut self) {
        self.0 = None;
    }
}

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

use sunrise_gameplay_common::game::integrated_common::IntegratedCommonGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;

#[cfg(feature = "client")]
#[cfg(test)]
mod tests {
    use super::*;

    /// A mod's text follows F3's frame rate line, several in mod load order; `None` takes it
    /// away, and a call outside a mod sets nothing.
    #[test]
    #[ignore = "daily"]
    fn mods_add_their_text_to_the_frame_rate_line() {
        use mod_api::{Hooks, ModEntry};
        use sunrise_level::crash::{ModCall, enter_native_mod};
        struct Quiet;
        #[mod_api::__subscriptions]
        impl Hooks for Quiet {}
        let as_mod = |id: &'static str, f: &mut dyn FnMut()| {
            let _call = enter_native_mod(ModCall {
                id,
                hook: "client_tick_post",
                part: None,
            });
            f()
        };
        let mods = vec![
            ModEntry::new("first", Box::new(Quiet)),
            ModEntry::new("second", Box::new(Quiet)),
        ];
        mod_platform::testing::with_mods(mods.into(), || {
            crate::game::native_entity_refs::tests::with_client_game(|game| {
                let line = |game: &crate::game::ClientGame<'_>| {
                    let mut d = debug_overlay::Displayer::default();
                    game.debug_fps_line(&mut d);
                    let (left, _) = d.columns(&[]);
                    left.into_iter().next().unwrap()
                };
                let vanilla = line(game);
                assert!(!game.set_debug_fps_text(Some("outside")));
                assert_eq!(line(game), vanilla);
                as_mod("second", &mut || {
                    assert!(game.set_debug_fps_text(Some("§c(2 fps)\nidle")));
                });
                as_mod("first", &mut || {
                    assert!(game.set_debug_fps_text(Some("one")));
                });
                assert_eq!(line(game), format!("{vanilla} one §c(2 fps) idle"));
                as_mod("first", &mut || {
                    assert!(game.set_debug_fps_text(None));
                });
                assert_eq!(line(game), format!("{vanilla} §c(2 fps) idle"));
                // An empty text takes the mod's text away too, as `None` does.
                as_mod("second", &mut || {
                    assert!(game.set_debug_fps_text(Some("")));
                });
                assert_eq!(line(game), vanilla);
            });
        });
    }

    fn block(x: i32) -> QueryReply {
        QueryReply::Block {
            pos: [x, 64, 0],
            state: "minecraft:chest[facing=north,type=single,waterlogged=false]".into(),
        }
    }

    #[test]
    fn a_debug_query_is_answered_once_and_only_the_newest() {
        let mut q = DebugQuery::default();
        // The first transaction is 0 (the handler starts at -1).
        assert_eq!(q.start(block(1)), 0);
        assert_eq!(q.answer(0), Some(block(1)));
        assert_eq!(q.answer(0), None);
        // A second press before the first answer: the older answer is ignored
        // and the newer one copies the newer target.
        assert_eq!(q.start(block(2)), 1);
        assert_eq!(q.start(block(3)), 2);
        assert_eq!(q.answer(1), None);
        assert_eq!(q.answer(2), Some(block(3)));
    }

    #[test]
    fn the_density_line_is_sampled_when_the_block_or_the_dimension_changes() {
        let mut cache = DensityCache::default();
        let samples = std::cell::Cell::new(0);
        let at = |cache: &mut DensityCache, dim, feet, present: bool| {
            let sample = || {
                samples.set(samples.get() + 1);
                Some(format!("Density {}", samples.get()))
            };
            cache
                .line(dim, feet, if present { Some(sample) } else { None })
                .map(str::to_owned)
        };
        // Standing still draws the first sample again and again.
        assert_eq!(
            at(&mut cache, "o", [0, 70, 0], true).as_deref(),
            Some("Density 1")
        );
        assert_eq!(
            at(&mut cache, "o", [0, 70, 0], true).as_deref(),
            Some("Density 1")
        );
        // A new block, then a new dimension at the same block, sample again.
        assert_eq!(
            at(&mut cache, "o", [1, 70, 0], true).as_deref(),
            Some("Density 2")
        );
        assert_eq!(
            at(&mut cache, "n", [1, 70, 0], true).as_deref(),
            Some("Density 3")
        );
        // No generator: nothing is drawn, nothing is kept, even on the same block.
        assert_eq!(at(&mut cache, "n", [1, 70, 0], false), None);
        assert_eq!(
            at(&mut cache, "n", [1, 70, 0], true).as_deref(),
            Some("Density 4")
        );
        // Another world clears it too.
        cache.clear();
        assert_eq!(
            at(&mut cache, "n", [1, 70, 0], true).as_deref(),
            Some("Density 5")
        );
    }
}
