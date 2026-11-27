//! The options screens bound to the game: drawing, input, applying settings
//! (video, sound, chat, font, language) and the F11 fullscreen toggle.
#[cfg(feature = "client")]
use super::shader_packs::{TogglePlan, active_pack, dev_pack, plan_shader_toggle};
use super::{ResizeState, Screen};
#[cfg(feature = "client")]
use crate::game::client_side::pack_discovery_client;
use serde_json::{Value, json};
use sunrise_client::{
    hdr_calibration::WindowState,
    hud::HudBatch,
    options_ui::{Event, Page},
    window::{FrameContext, InactivityFpsLimit, framerate_limit},
};
use sunrise_game_data::{Result, lang, text::Text};
use winit::{event::MouseButton, keyboard::KeyCode, window::Fullscreen};

const UNLIMITED_FRAMERATE: i32 = 260;
const SKIPPED_FRAME_RATE: i32 = 10;

/// `CommonLinks.ACCESSIBILITY_HELP`, the Accessibility page's footer link.
pub(in crate::game) const ACCESSIBILITY_HELP_LINK: &str =
    "https://aka.ms/MinecraftJavaAccessibility";

pub(in crate::game) const CONTROL_HELP: &[&str] = &[
    "options-get [KEY...]           options.txt values by key, with their type, range or choices",
    "options-set KEY VALUE          set an option (enums by name, keys as key.keyboard.w, lists as JSON), apply and save it",
];

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(in crate::game::client_side) fn save_title_options(&mut self) {
        if let Some(title) = &self.client.title.0
            && (title.last_server != self.client.settings.file.string("lastServer")
                || title.skip_multiplayer_warning
                    != self.client.settings.file.bool("skipMultiplayerWarning"))
        {
            self.client.settings.file.set(
                "lastServer",
                sunrise_client::options_file::V::Str(title.last_server.clone()),
            );
            self.client.settings.file.set(
                "skipMultiplayerWarning",
                sunrise_client::options_file::V::Bool(title.skip_multiplayer_warning),
            );
            self.client
                .settings
                .save(&sunrise_gameplay_common::game::options_path());
        }
    }
    /// `options-get` and `options-set`; None for other commands.
    pub(in crate::game::client_side) fn options_control(
        &mut self,
        words: &[&str],
        line: &str,
    ) -> Result<Option<Value>> {
        match words[0] {
            "options-get" => {
                let file = self.client.settings.to_file();
                let wanted: Vec<&str> = if words.len() > 1 {
                    words[1..].to_vec()
                } else {
                    sunrise_client::options_file::keys()
                };
                let mut options = serde_json::Map::new();
                for key in wanted {
                    let described = file.describe(key).ok_or_else(|| {
                        format!("unknown option {key}; options-get lists them all")
                    })?;
                    options.insert(key.to_owned(), described);
                }
                Ok(Some(json!({ "options": options })))
            }
            "options-set" => {
                let key = words.get(1).ok_or("options-set KEY VALUE")?;
                // The value runs to the end of the line (lists, server names).
                let value = line
                    .trim_start()
                    .strip_prefix("options-set")
                    .and_then(|rest| rest.trim_start().strip_prefix(key))
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .ok_or("options-set KEY VALUE: missing the value")?;
                let narrator = self.client.settings.narrator();
                let reduced = self.client.settings.reduced_debug_info;
                let before_atlas = self.client.settings.texture_atlas_settings();
                self.client.settings = self.client.settings.with_option(key, value)?;
                if self.client.settings.reduced_debug_info != reduced {
                    self.client.debug_entries.0.invalidate();
                }
                // As the options screen applies a change (OptionInstance.set
                // runs the narrator's onValueUpdate for a new value only).
                let mut events = vec![Event::Changed];
                if self.client.settings.narrator() != narrator {
                    events.push(Event::Narrator);
                }
                self.handle_option_events(events)?;
                match *key {
                    "lang" => self.load_language(&self.client.settings.language.clone()),
                    "resourcePacks" | "incompatibleResourcePacks" => self.reload_packs(),
                    _ if self.client.settings.texture_atlas_settings() != before_atlas => {
                        self.reload_packs();
                    }
                    _ => {}
                }
                // Labels change (a language, a value): `ui` reads the redrawn screen.
                self.client.ui_wait.0.clicked();
                let now = self.client.settings.to_file().describe(key);
                Ok(Some(json!({ "ok": true, "key": key, "option": now })))
            }
            _ => Ok(None),
        }
    }
    /// `shaderpacks/` in the game folder.
    pub(in crate::game::client_side) fn shader_packs_dir() -> std::path::PathBuf {
        sunrise_gameplay_common::game::game_dir().join(sunrise_client::shader_pack_list::DIR)
    }
    pub fn gui_size(&self) -> (f32, f32) {
        let size = self
            .client
            .renderer
            .0
            .as_ref()
            .map_or([1280, 720], |r| [r.config.width, r.config.height]);
        let b = HudBatch::new(size, self.client.settings.gui_scale_inputs());
        (b.width, b.height)
    }
    /// Close the options screens (`OptionsScreen.onClose`) back to where they
    /// were opened: the recipe book's settings, World Options, the title or
    /// the pause menu.
    pub fn leave_options(&mut self) {
        let to_game = self.client.options_from_title;
        let back = if let Some(screen) = self.client.panel.recipes_settings_return.take() {
            screen
        } else if std::mem::take(&mut self.client.options_to_world_options) {
            Screen::WorldOptions
        } else if self.client.options_from_title {
            Screen::Playing
        } else {
            Screen::Paused
        };
        self.set_screen(back);
        // Vanilla grabs only on `setScreen(null)`: back on the title, which
        // is still shown, the pointer stays free (#1999). The startup handoff
        // can enter a world while its options are open, and closing them then
        // returns to that world, as `capture_loaded_world` grabs.
        if to_game && self.client.title.0.is_none() {
            self.capture(true);
        }
        // Closed from calibration (sunrise-ctl, a lost window): out of it.
        self.show_calibration((0.0, 0.0));
    }
    /// Open the options screens (OptionsScreen) from the title or pause menu.
    pub fn open_options(&mut self, from_title: bool) {
        self.open_options_at(from_title, Page::Main);
    }
    pub fn open_options_at(&mut self, from_title: bool, page: Page) {
        // Gameplay keys must not stay held behind the options screen, where
        // bindings can also change before the physical key is released.
        self.clear_device_keys();
        self.set_screen(Screen::Options);
        self.client.options_from_title = from_title;
        let minecart_improvements = self.minecart_improvements();
        // Restore fullbright when leaving another server.
        self.hold_fullbright();
        let ui = &mut self.client.chat_ui.options;
        ui.open_at(!from_title && self.client.title.0.is_none(), page);
        ui.capture_video_atlas(&self.client.settings);
        ui.minecart_improvements = minecart_improvements;
        ui.languages = lang::available(&[&self.common.pack.source]);
        self.refresh_pack_lists();
        if let Page::NativeMod(index) = page {
            self.native_settings_refresh(index);
        }
    }
    /// While the resource pack screen is open, `resourcepacks/` is looked at once a second
    /// and the list found again when its packs changed (`PackSelectionScreen.tick`'s
    /// watcher).
    fn watch_pack_folder(&mut self) {
        if !self.client.chat_ui.options.resource_packs_open() {
            self.client.pack_folder = None;
            return;
        }
        let now = std::time::Instant::now();
        if self
            .client
            .pack_folder
            .as_ref()
            .is_some_and(|(at, _)| now.duration_since(*at).as_secs_f32() < 1.0)
        {
            return;
        }
        let signature: Vec<(String, u64, Option<std::time::SystemTime>)> =
            std::fs::read_dir(super::packs::packs_dir())
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| {
                    let meta = e.metadata().ok();
                    (
                        e.file_name().to_string_lossy().into_owned(),
                        meta.as_ref().map_or(0, |m| m.len()),
                        meta.and_then(|m| m.modified().ok()),
                    )
                })
                .collect();
        let changed = self
            .client
            .pack_folder
            .as_ref()
            .is_some_and(|(_, seen)| *seen != signature);
        self.client.pack_folder = Some((now, signature));
        if changed {
            let infos = self.resource_pack_infos();
            self.client.chat_ui.options.update_pack_infos(infos);
            self.client.chat_ui.options.pack_configs =
                super::packs::pack_configs(&self.common.vanilla_root());
        }
    }
    pub(in crate::game::client_side) fn draw_options(
        &mut self,
        batch: &mut HudBatch,
        allowed: bool,
    ) {
        if !allowed {
            self.client.chat_ui.options.discard_draw();
            return;
        }
        self.watch_pack_folder();
        let context = self.controller_settings_context();
        if self.client.chat_ui.options.controller_open() {
            self.client.chat_ui.options.set_controller_context(context);
        }
        let output = self
            .client
            .renderer
            .0
            .as_ref()
            .map_or(Default::default(), |r| r.output());
        self.client.chat_ui.options.draw_background(
            &self.client.settings,
            output,
            batch,
            &self.client.gui.atlas,
        );
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        self.client.chat_ui.options.draw_contents(
            &self.client.settings,
            batch,
            &self.client.gui.atlas,
            self.client.cursor,
        );
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
        self.client.chat_ui.options.draw_overlays(
            batch,
            &self.client.gui.atlas,
            self.client.cursor,
        );
        self.show_calibration((batch.width, batch.height));
    }
    fn controller_settings_context(&self) -> sunrise_client::controller_ui::Context {
        sunrise_client::controller_ui::Context {
            player: self.common.options.player_name(),
            device: self.client.pad.0.device_or_virtual(),
            detected: detected_controller_style(
                self.client.pad.0.device.as_ref(),
                self.client.pad.0.style,
            ),
            glyphs: self.client.chat_ui.options.pad_glyphs,
        }
    }
    /// HDR calibration's test picture to the renderer, full screen while it runs: the
    /// window goes fullscreen for it and back after, as it was (maximized, say).
    pub(in crate::game::client_side) fn show_calibration(&mut self, size: (f32, f32)) {
        let picture = (self.client.screen == Screen::Options)
            .then(|| {
                self.client
                    .chat_ui
                    .options
                    .calibration_picture(&self.client.gui.atlas, size)
            })
            .flatten();
        if let Some(r) = self.client.renderer.0.as_mut() {
            r.set_hdr_calibration(picture.map(|(step, nits, band)| {
                sunrise_client::gfx::HdrCalibration {
                    step: step.number(),
                    nits,
                    band,
                }
            }));
        }
        let Some(window) = &self.client.window.0 else {
            return;
        };
        match (
            picture.is_some(),
            self.client.chat_ui.options.calibration_window,
        ) {
            (true, None) => {
                // Before entering fullscreen, which un-maximizes a window on Windows.
                let was = WindowState::of(window.fullscreen().is_some(), window.is_maximized());
                if was != WindowState::Fullscreen {
                    let borderless = sunrise_client::window::borderless_fullscreen(window);
                    sunrise_client::window::enter_fullscreen(window, borderless);
                }
                self.client.chat_ui.options.calibration_window = Some(was);
            }
            (false, Some(was)) => {
                let (leave_fullscreen, maximize) = was.leaving();
                if leave_fullscreen {
                    window.set_fullscreen(None);
                }
                if maximize {
                    window.set_maximized(true);
                }
                self.client.chat_ui.options.calibration_window = None;
            }
            _ => {}
        }
    }
    pub(in crate::game::client_side) fn options_mouse(
        &mut self,
        pressed: bool,
        right: bool,
    ) -> Result<bool> {
        if !pressed {
            if right {
                return Ok(false);
            }
            let (events, handled) = self.client.chat_ui.options.release_with_outcome();
            self.handle_option_events(events)?;
            return Ok(handled);
        }
        let size = self.gui_size();
        let framebuffer = self.client.renderer.0.as_ref().map_or([1280, 720], |r| {
            [r.config.width as i32, r.config.height as i32]
        });
        let mut ui = std::mem::take(&mut self.client.chat_ui.options);
        ui.shift = self.client.modifiers.shift_key();
        let (events, handled) = ui.click_with_outcome(
            &mut self.client.settings,
            &self.client.gui.atlas,
            self.client.cursor.0,
            self.client.cursor.1,
            size,
            framebuffer,
            false,
            right,
        );
        self.client.chat_ui.options = ui;
        self.handle_option_events(events)?;
        Ok(handled)
    }
    pub(in crate::game::client_side) fn options_drag(&mut self) -> Result<bool> {
        let size = self.gui_size();
        let framebuffer = self.client.renderer.0.as_ref().map_or([1280, 720], |r| {
            [r.config.width as i32, r.config.height as i32]
        });
        let mut ui = std::mem::take(&mut self.client.chat_ui.options);
        let (handled, events) = ui.drag(
            &mut self.client.settings,
            &self.client.gui.atlas,
            self.client.cursor,
            size,
            framebuffer,
            false,
        );
        self.client.chat_ui.options = ui;
        self.handle_option_events(events)?;
        Ok(handled)
    }
    pub(in crate::game::client_side) fn options_key(
        &mut self,
        key: KeyCode,
        text: Option<&str>,
    ) -> Result<bool> {
        let size = self.gui_size();
        let shift = self.has_shift_down();
        let framebuffer = self.client.renderer.0.as_ref().map_or([1280, 720], |r| {
            [r.config.width as i32, r.config.height as i32]
        });
        self.client.chat_ui.options.size = size;
        self.client.chat_ui.options.framebuffer = framebuffer;
        let mut ui = std::mem::take(&mut self.client.chat_ui.options);
        ui.modifiers = self.modifiers().bits;
        let (handled, events) = ui.key_input(
            &mut self.client.settings,
            &self.client.gui.atlas,
            key,
            text,
            shift,
        );
        self.client.chat_ui.options = ui;
        self.handle_option_events(events)?;
        self.refresh_screen_widgets();
        Ok(handled)
    }
    pub(in crate::game::client_side) fn options_key_released(
        &mut self,
        key: KeyCode,
    ) -> Result<bool> {
        self.client.chat_ui.options.modifiers = self.modifiers().bits;
        let events = self
            .client
            .chat_ui
            .options
            .key_released(&mut self.client.settings, key);
        let handled = !events.is_empty();
        self.handle_option_events(events)?;
        Ok(handled)
    }
    /// Ctrl+A/C/V/X in the options' search boxes; false when none has focus.
    pub(in crate::game::client_side) fn options_shortcut(
        &mut self,
        shortcut: sunrise_client::clipboard::EditShortcut,
    ) -> bool {
        self.client.chat_ui.options.shortcut(
            &self.client.gui.atlas,
            shortcut,
            &mut self.client.clipboard.0,
        )
    }
    pub(in crate::game::client_side) fn options_scroll(&mut self, amount: f32) -> Result<bool> {
        if self.pad_stick_scroll() {
            let size = self.gui_size();
            self.client.chat_ui.options.scroll_at(
                &self.client.settings,
                size,
                self.client.cursor,
                amount,
            );
            return Ok(true);
        }
        let framebuffer = self.client.renderer.0.as_ref().map_or([1280, 720], |r| {
            [r.config.width as i32, r.config.height as i32]
        });
        if self.client.chat_ui.options.video_open() && self.has_control_down() {
            // VideoSettingsScreen.mouseScrolled: Ctrl with the wheel steps the GUI scale.
            let mut ui = std::mem::take(&mut self.client.chat_ui.options);
            let events = ui.video_wheel_gui_scale(&mut self.client.settings, framebuffer, amount);
            self.client.chat_ui.options = ui;
            self.handle_option_events(events)?;
            return Ok(true);
        }
        let size = self.gui_size();
        let (handled, events) = self.client.chat_ui.options.wheel_input(
            &mut self.client.settings,
            size,
            self.client.cursor,
            framebuffer,
            amount,
        );
        self.handle_option_events(events)?;
        Ok(handled)
    }
    /// Mouse buttons pressed while a key is being bound.
    pub(in crate::game::client_side) fn options_waiting_for_key(&self) -> bool {
        self.client.chat_ui.options.selected_key.is_some()
    }
    pub(in crate::game::client_side) fn options_bind_mouse(
        &mut self,
        button: MouseButton,
    ) -> Result<()> {
        self.client.chat_ui.options.modifiers = self.modifiers().bits;
        let events = self
            .client
            .chat_ui
            .options
            .bind_mouse(&mut self.client.settings, button);
        self.handle_option_events(events)
    }
    pub fn handle_option_events(&mut self, events: Vec<Event>) -> Result<()> {
        for e in events {
            match e {
                Event::ButtonClick => self.play_audio_button_click(),
                Event::DebugEntriesChanged => self.client.debug_entries.0.invalidate(),
                Event::Changed => {
                    self.client
                        .settings
                        .save(&sunrise_gameplay_common::game::options_path());
                    self.client.settings.keys.sync_action_bindings();
                    self.apply_settings();
                    self.switch_mods();
                    self.sync_native_switches();
                    self.sync_pregen_option();
                    self.sync_agent_control();
                    self.client.chat_ui.options.agent_status = self.agent_status_lines();
                }
                Event::SliderDrag => {
                    self.apply_settings();
                }
                // Options.createSoundSliderOptionInstance: the preview plays
                // only without a level (on the title screen's options).
                Event::Volume(source, volume) if self.client.title.0.is_some() => {
                    self.client.audio.0.preview(source, volume);
                }
                Event::Volume(..) => {}
                Event::CopyAgentDetails => {
                    use sunrise_client::clipboard::Clipboard;
                    let text = self.agent_details();
                    self.client.clipboard.0.set(&text);
                }
                // The AI Agent page's repro buttons (game/runtime/repro.rs).
                Event::ToggleRecording => self.toggle_recording_key(),
                Event::ExportRepro => self.export_repro_key(),
                // The Music page: the situations' tracks, and playing one now.
                Event::ListMusicTracks => {
                    self.client.chat_ui.options.music_groups = self.music_groups()
                }
                Event::PlayMusicTrack(event, track) => self.play_music_track(&event, &track),
                Event::Close if self.client.chat_ui.options.resource_error.is_some() => {}
                Event::Close => {
                    self.client
                        .settings
                        .save(&sunrise_gameplay_common::game::options_path());
                    self.leave_options();
                }
                Event::Language(code) => {
                    self.load_language(&code);
                }
                Event::Packs(_) => {
                    self.client
                        .settings
                        .save(&sunrise_gameplay_common::game::options_path());
                    self.reload_packs();
                }
                Event::ReloadTextures => {
                    self.client
                        .settings
                        .save(&sunrise_gameplay_common::game::options_path());
                    self.apply_settings();
                    self.reload_packs();
                }
                Event::PackOptions { entries, chosen } => {
                    super::packs::save_pack_options(&entries, chosen);
                    self.client.chat_ui.options.pack_configs =
                        super::packs::pack_configs(&self.common.vanilla_root());
                    self.reload_packs();
                }
                Event::OpenPackFolder => {
                    let dir = super::packs::packs_dir();
                    let _ = std::fs::create_dir_all(&dir);
                    super::packs::open_folder(&dir);
                }
                Event::ImportPacks(paths) => {
                    let additional = self
                        .discover_mod_packs(mod_api::PackType::ClientResources)
                        .into_iter()
                        .map(pack_discovery_client::into_resource_pack)
                        .collect::<Vec<_>>();
                    let vanilla = self.common.vanilla_root();
                    super::packs::import_resource_packs(
                        &mut self.client.chat_ui.options,
                        &vanilla,
                        &self.client.server_packs.0.active,
                        &paths,
                        &additional,
                    );
                }
                // Sunrise's own Add button: the screen picked a hit, the game
                // owns the folder it lands in.
                Event::DownloadPack(i) => {
                    let dir = super::packs::packs_dir();
                    if let Some(screen) = &mut self.client.chat_ui.options.pack_screen
                        && let Some(browser) = &mut screen.browser
                    {
                        browser.download(i, &dir, sunrise_core::VANILLA_VERSION);
                    }
                }
                Event::DataPacks(ids) => {
                    let resources = self.common.vanilla_root();
                    if let Some(title) = &mut self.client.title.0 {
                        title.offer_data_packs(&resources, ids);
                    }
                }
                Event::OpenDataPackFolder(dir) => {
                    let _ = std::fs::create_dir_all(&dir);
                    super::packs::open_folder(&dir);
                }
                Event::OpenModFolder => {
                    let dir = sunrise_gameplay_common::game::game_dir()
                        .join(sunrise_mod_package::mod_list::DIR);
                    let _ = std::fs::create_dir_all(&dir);
                    super::packs::open_folder(&dir);
                }
                // The Mods screen's links go through ConfirmLinkScreen, back to the options.
                Event::OpenLink(url) => self.confirm_untrusted_link(&url, Screen::Options),
                Event::CopyText(text) => {
                    use sunrise_client::clipboard::Clipboard;
                    self.client.clipboard.0.set(&text);
                }
                Event::ListShaderPacks => {
                    self.client.settings.shader_packs =
                        sunrise_client::shader_pack_list::scan(&Self::shader_packs_dir());
                    self.client.native_shaders.pick_hdr.get_mut().forget();
                }
                // Saved; the next frame's `shader_pack_inputs` loads the pick.
                Event::ShaderPack => self
                    .client
                    .settings
                    .save(&sunrise_gameplay_common::game::options_path()),
                Event::OpenShaderOptions => self.open_shader_options(),
                Event::ShaderOptions => self.save_shader_options(),
                Event::OpenShaderPackFolder => {
                    let dir = Self::shader_packs_dir();
                    let _ = std::fs::create_dir_all(&dir);
                    super::packs::open_folder(&dir);
                }
                Event::NativeSettings(i) => self.native_settings_refresh(i),
                Event::ModConfigScreen(i) => {
                    let Some(id) = self
                        .client
                        .settings
                        .mods
                        .installed
                        .get(i)
                        .map(|m| m.id.clone())
                    else {
                        continue;
                    };
                    match mod_api::__private::create_config_screen(self, &id) {
                        Some(Ok(true)) => {
                            self.client.native_mod_screen_parent = self.mod_screen_open(&id);
                        }
                        Some(Err(reason)) => log::error!("mod {id}: {reason}"),
                        Some(Ok(false)) | None => {}
                    }
                }
                Event::NativeSetting(i, j) => self.native_setting_event(i, j),
                Event::Credits => self.open_win_screen(false, super::win::Finish::Options),
                Event::HudEditor => self.open_hud_editor(true),
                // AccessibilityOptionsScreen's link button: ConfirmLinkScreen
                // back to the options screen.
                Event::AccessibilityHelp => {
                    self.confirm_trusted_link(ACCESSIBILITY_HELP_LINK, Screen::Options)
                }
                Event::Narrator => self.narrator_status_changed(),
                Event::OpenStudioReplay(path) => {
                    let path = path.to_string_lossy().to_string();
                    if let Err(e) = self.open_studio_replay(&path) {
                        log::warn!("Sunrise Studio: could not open {path}: {e}");
                    }
                }
                Event::OpenStudioFolder => {
                    let dir = std::path::PathBuf::from("recordings");
                    let _ = std::fs::create_dir_all(&dir);
                    super::packs::open_folder(&dir);
                }
            }
        }
        Ok(())
    }
    /// LanguageManager.setSelected + reload: translations, then re-wrap chat.
    pub(in crate::game::client_side) fn load_language(&mut self, code: &str) {
        lang::set(lang::Language::load(&[&self.common.pack.source], code));
        self.refresh_remote_language();
        self.client.chat_ui.chat.rescale(&self.client.gui.atlas);
        self.client.creative_screen.0.forget_items();
    }
    /// Window.updateFullscreenIfChanged: only a changed option asks the
    /// window to change, so applying settings never undoes a fullscreen
    /// the compositor set up.
    fn apply_fullscreen(&mut self) {
        if let Some(w) = self.client.window.0.clone() {
            let monitor = w.current_monitor().or_else(|| w.primary_monitor());
            if let Some(modes) = self
                .client
                .video_modes
                .0
                .refresh(monitor, sunrise_client::window::monitor_video_modes)
            {
                self.client.settings.video_modes = modes;
            }
            let in_video_screen =
                self.client.screen == Screen::Options && self.client.chat_ui.options.video_open();
            let wanted = self.client.settings.fullscreen;
            let exclusive = self.client.settings.file.bool("exclusiveFullscreen");
            let preferred = self
                .client
                .settings
                .file
                .fullscreen_resolution
                .as_deref()
                .and_then(sunrise_client::window::VideoMode::read);
            self.client
                .fullscreen
                .apply(&w, wanted, exclusive, preferred, in_video_screen);
        }
    }
    /// Settings that Sunrise applies beyond render distance, volume and GUI scale.
    pub fn apply_ui_settings(&mut self) {
        sunrise_sim::parallel::set_setting(self.client.settings.sunrise.parallel_tick);
        if sunrise_client_render::mipmap::set_dither_cutouts(
            self.client.settings.sunrise.dither_cutout_mips,
        ) {
            self.reload_packs();
        }
        self.apply_fullscreen();
        // A failed HDR start that named no driver becomes the window's driver's, and another
        // driver's is dropped, so HDR output is tried again.
        if let Some(driver) = self.client.renderer.0.as_ref().and_then(|r| r.hdr_driver())
            && self.client.settings.sunrise.settle_failed_start(driver)
            && let Some(dir) = sunrise_gameplay_common::game::options_path().parent()
        {
            self.client.settings.sunrise.save(dir);
        }
        self.hold_fullbright();
        let vsync = self.effective_vsync();
        let s = &self.client.settings;
        if let Some(r) = self.client.renderer.0.as_mut() {
            r.gamma = s.gamma as f32;
            r.improved_transparency = s.improved_transparency;
            r.set_present_mode(vsync, s.sunrise.present_mode);
            r.animated_textures = s.sunrise.animated_textures;
            r.screenshot_quality = s.sunrise.screenshot_quality;
            r.set_mipmap_levels(s.mipmap_levels.max(0) as u32);
            r.set_texture_filtering(
                s.file.int("textureFiltering"),
                s.file.int("maxAnisotropyBit"),
            );
            // Dynamic Resolution's scale where it is on; turned off, it starts over at the floor.
            if !s.sunrise.dynamic_resolution {
                self.client.dynamic_resolution.reset();
            }
            let scale =
                super::frame_autotune::world_scale(&s.sunrise, &self.client.dynamic_resolution);
            super::frame_autotune::apply_world_post(r, &s.sunrise, scale);
            r.set_frame_generation(
                s.sunrise.frame_generation != sunrise_client::graphics::FrameGeneration::Off,
            );
            r.set_hdr_lighting(s.sunrise.hdr_lighting);
            // Where it is offered (`Renderer::gpu_culling_offer`); unchanged, it does nothing.
            // Auto has it on while its measurements say so (`GpuCullingAuto`), and starts over
            // when the choice is another one.
            let gpu_culling = match s.sunrise.gpu_culling {
                sunrise_client::graphics::GpuCullingMode::Off => false,
                sunrise_client::graphics::GpuCullingMode::On => true,
                sunrise_client::graphics::GpuCullingMode::Auto => self.client.gpu_culling_auto.on(),
            };
            if s.sunrise.gpu_culling != sunrise_client::graphics::GpuCullingMode::Auto {
                self.client.gpu_culling_auto.reset();
            }
            let _ = r.set_gpu_culling(gpu_culling);
            r.update_output(
                &s.sunrise,
                &sunrise_gameplay_common::game::options_path()
                    .with_file_name(sunrise_client::graphics::HDR_MARKER),
            );
        }
        let mut volumes = s.volumes.map(|gain| gain as f32);
        volumes[0] = 1.0;
        sunrise_client::settings::set_source_volumes(volumes);
        super::narrator::sync(s);
        let chat = s.chat_options();
        let rescale = {
            let o = &self.client.chat_ui.chat.options;
            o.width != chat.width || o.scale != chat.scale || o.line_spacing != chat.line_spacing
        };
        self.client.chat_ui.chat.options = chat;
        if rescale {
            self.client.chat_ui.chat.rescale(&self.client.gui.atlas);
        }
        let font = &mut self.client.gui.atlas.font;
        if font.uniform != s.force_unicode || font.japanese != s.japanese_glyphs {
            font.uniform = s.force_unicode;
            font.japanese = s.japanese_glyphs;
            font.reset_cache();
            self.client.chat_ui.chat.rescale(&self.client.gui.atlas);
        }
        self.notify_window_resize();
        // GUI-scale changes rebuild mounted children before the next input.
        self.refresh_screen_widgets();
    }
    pub(in crate::game::client_side) fn resize_state_of(
        &self,
        size: winit::dpi::PhysicalSize<u32>,
    ) -> ResizeState {
        let viewport = self
            .client
            .renderer
            .0
            .as_ref()
            .map_or([size.width, size.height], |r| {
                [r.config.width, r.config.height]
            });
        ResizeState {
            size,
            gui_size: sunrise_client::window::GuiSize::new(
                viewport,
                self.client.settings.gui_scale,
                self.client.settings.force_unicode,
            )
            .size,
        }
    }
    /// Fires `window_resize` once when the physical or GUI-scaled dimensions change.
    pub fn notify_window_resize(&mut self) {
        // The size `ClientOptionsClientHost::window_size` reads, so the two agree.
        let Some([width, height]) = self.window_size() else {
            return;
        };
        let now = self.resize_state_of(winit::dpi::PhysicalSize::new(width, height));
        if now.differs_from(&mut self.client.resize_state) {
            self.native_window_resize(
                now.size.width,
                now.size.height,
                now.gui_size[0],
                now.gui_size[1],
            );
            self.refresh_screen_widgets();
        }
    }
    /// `Window.updateFullscreenState`: a fullscreen change made by the window
    /// system is adopted by the option (`Minecraft.fullscreenStateChanged`).
    pub(in crate::game::client_side) fn observe_fullscreen(&mut self) {
        // HDR calibration's own fullscreen is not the player's option: it
        // goes back to what it was when the calibration ends.
        if self.client.chat_ui.options.calibration_window.is_some() {
            return;
        }
        let Some(w) = &self.client.window.0 else {
            return;
        };
        if let Some(now) = self.client.fullscreen.observe(w.fullscreen().is_some())
            && self.client.settings.fullscreen != now
        {
            self.client.settings.fullscreen = now;
            self.client
                .settings
                .save(&sunrise_gameplay_common::game::options_path());
            self.notify_window_resize();
        }
    }
    /// F11 (key.fullscreen): Window.toggleFullScreen, remembered in options.txt.
    pub(in crate::game::client_side) fn toggle_fullscreen(&mut self) {
        self.client.settings.fullscreen = !self.client.settings.fullscreen;
        self.client
            .settings
            .save(&sunrise_gameplay_common::game::options_path());
        self.apply_ui_settings();
    }
    /// Toggle the persisted Shaders-page pick, leaving reload to the normal selection path.
    pub(in crate::game::client_side) fn toggle_shader_pack(&mut self) {
        let present = |key: &str| {
            let dir = Self::shader_packs_dir();
            !sunrise_client::shader_pack_list::unsupported(&dir, key) && dir.join(key).exists()
        };
        let plan = plan_shader_toggle(
            &self.client.settings.sunrise.shader_pack,
            &self.client.settings.extras.last_shader_pack,
            dev_pack().is_some(),
            present,
            |key| !active_pack(key, &self.client.settings.mods).is_empty(),
        );
        match plan {
            TogglePlan::Noop => {}
            TogglePlan::Hint => {
                self.client_system_message(Text::tr("sunrise.shaders.noPack", &[]));
            }
            TogglePlan::HintClearLast => {
                self.client.settings.extras.last_shader_pack.clear();
                self.client
                    .settings
                    .save(&sunrise_gameplay_common::game::options_path());
                self.client_system_message(Text::tr("sunrise.shaders.noPack", &[]));
            }
            TogglePlan::HintModOff(name) => {
                self.client_system_message(Text::tr("sunrise.shaders.modOff", &[&name]));
            }
            TogglePlan::Off(name) => {
                self.client.settings.set_shader_pick("");
                self.client
                    .settings
                    .save(&sunrise_gameplay_common::game::options_path());
                self.client_system_message(Text::tr("sunrise.shaders.suspended", &[&name]));
            }
            TogglePlan::On(name) => {
                self.client.settings.set_shader_pick(&name);
                self.client
                    .settings
                    .save(&sunrise_gameplay_common::game::options_path());
                self.client_system_message(Text::tr("sunrise.shaders.resumed", &[&name]));
            }
        }
    }
    /// `FramerateLimitTracker.getFramerateLimit` plus the mod override, where zero skips this render.
    pub(in crate::game) fn framerate_limit(&self) -> i32 {
        let context = FrameContext {
            inactivity: if self.client.settings.inactivity_fps_limit == "minimized" {
                InactivityFpsLimit::Minimized
            } else {
                InactivityFpsLimit::Afk
            },
            iconified: self
                .client
                .window
                .0
                .as_ref()
                .is_some_and(|w| w.is_minimized().unwrap_or(false)),
            focused: self.client.focused,
            // Window.isExclusiveFullscreen.
            exclusive_fullscreen: self
                .client
                .window
                .0
                .as_ref()
                .is_some_and(|w| matches!(w.fullscreen(), Some(Fullscreen::Exclusive(_)))),
            now_ms: self.client.started.elapsed().as_millis() as i64,
            latest_input_ms: self
                .client
                .last_input
                .duration_since(self.client.started)
                .as_millis() as i64,
            has_level: self.client.title.0.is_none(),
            has_screen: self.client.title.0.is_some()
                || self.client.screen != super::Screen::Playing,
            has_overlay: false,
        };
        let limit = framerate_limit(self.client.settings.max_fps as i32, context);
        let limit = self.client.settings.sunrise.battery_limit(
            limit,
            self.client.title.0.is_none(),
            self.client.focused,
        );
        let mut hooked = limit;
        self.native_framerate_limit(
            self.client.focused,
            context.iconified,
            self.client.title.0.is_none(),
            &mut hooked,
        );
        if hooked >= 0 {
            hooked.min(UNLIMITED_FRAMERATE)
        } else {
            limit
        }
    }
    /// `Minecraft.runTick`'s frame limiter: `FramerateLimiter.limitDisplayFPS`
    /// below 260 frames per second, sleeping then spin-waiting until the
    /// frame's slot ends (not in scripted frame-count runs).
    pub(in crate::game) fn limit_frame(&mut self, limit: i32) {
        if self.common.options.frames.is_some() {
            return;
        }
        let limit = if limit == 0 {
            SKIPPED_FRAME_RATE
        } else {
            limit
        };
        match self.client.settings.sunrise.pacing {
            sunrise_client::graphics::Pacing::Precise => sunrise_client::window::end_frame(
                &mut self.client.limiter,
                limit,
                &mut self.client.frame_clock,
            ),
            sunrise_client::graphics::Pacing::Efficient => {
                let mut clock = sunrise_client::window::SleepingClock(self.client.frame_clock);
                sunrise_client::window::end_frame_on_grid(
                    &mut self.client.limiter,
                    limit,
                    &mut clock,
                );
            }
        }
    }
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Fullbright as this window may draw it (`fullbright_held`): the renderer's flag and the
    /// graphics screen's row. Each tick (`tick_frame_visuals`) and when settings apply, so
    /// joining or leaving a server needs no call of its own.
    pub(in crate::game::client_side) fn hold_fullbright(&mut self) {
        let held = fullbright_held(
            self.client
                .remote
                .as_ref()
                .map(|_| self.client.integrated.is_some()),
            self.client.replay.playing(),
        );
        self.client.settings.fullbright_held = held;
        if let Some(r) = self.client.renderer.0.as_mut() {
            r.fullbright = self.client.settings.extras.fullbright && !held;
        }
    }
}

/// Fullbright is held off on another's server, where it would light what the server shows
/// dark: a connection without this window's integrated server, as a multiplayer server
/// or a LAN guest has. Single player, a LAN host and Replay Studio keep it.
/// `connection`: none, or whether it plays on this window's integrated server.
fn fullbright_held(connection: Option<bool>, studio: bool) -> bool {
    connection == Some(false) && !studio
}

/// The named device can be a connected pad that has not driven the game yet.
fn detected_controller_style(
    device: Option<&sunrise_client::controller_ui::Device>,
    fallback: sunrise_client::gamepad::GlyphStyle,
) -> sunrise_client::gamepad::GlyphStyle {
    device.map_or(fallback, |d| {
        sunrise_client::gamepad::GlyphStyle::of_device(d.vendor, d.product)
    })
}

#[cfg(feature = "client")]
#[cfg(test)]
mod fullbright_tests {
    use super::fullbright_held;

    #[test]
    fn fullbright_is_held_only_on_another_servers_world() {
        // The title screen, with no connection.
        assert!(!fullbright_held(None, false));
        // Single player and a LAN host play their own save.
        assert!(!fullbright_held(Some(true), false));
        // A multiplayer server, or another player's world opened to LAN.
        assert!(fullbright_held(Some(false), false));
        // Replay Studio plays a recording, not a server.
        assert!(!fullbright_held(Some(false), true));
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
mod controller_context_tests {
    use super::detected_controller_style;
    use sunrise_client::{controller_ui::Device, gamepad::GlyphStyle};

    #[test]
    fn detected_style_follows_the_named_device_even_before_it_is_used() {
        let mut device = Device {
            name: "pad".into(),
            vendor: Some(0x054c),
            product: None,
        };
        assert_eq!(
            detected_controller_style(Some(&device), GlyphStyle::Xbox),
            GlyphStyle::PlayStation
        );
        device.vendor = Some(0x057e);
        assert_eq!(
            detected_controller_style(Some(&device), GlyphStyle::PlayStation),
            GlyphStyle::Nintendo
        );
        assert_eq!(
            detected_controller_style(None, GlyphStyle::PlayStation),
            GlyphStyle::PlayStation
        );
    }
}

use sunrise_gameplay_common::game::mods::ModGameExt1 as _;

use sunrise_gameplay_common::game::pack_finders_common::CommonGameExt1 as _;

use sunrise_gameplay_common::game::minecarts::MinecartRailGameExt as _;
