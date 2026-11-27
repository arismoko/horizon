//! Commands from the control socket (`sunrise-ctl`), run on the game thread.
use super::Screen;
use super::event_loop::EventLoopCtl;
use serde_json::{Value, json};
use std::{
    sync::mpsc::Sender,
    time::{Duration, Instant},
};
use sunrise_client::gfx::pack_timing;
use sunrise_client::held::Source;
use sunrise_core::geometry::Vec3;
use sunrise_game_data::Result;
use sunrise_gameplay_common::game::integrated_link::NO_ANSWER;
use sunrise_gameplay_common::game::owner_link::{ControlRequest, OwnerRequest};
use sunrise_items::items::ItemStack;
use winit::{event::MouseButton, keyboard::KeyCode};

/// The control protocol's version (`docs/api/stability.md`): before 1.0 an
/// addition makes the next patch and a breaking change the next line.
pub(in crate::game) const PROTOCOL: &str = "0.1.0";

/// The command groups the game serves, as `help` chains them: what `hello` reports as its features.
pub(in crate::game) const FEATURES: &[&str] = &[
    "game", "station", "blocks", "lan", "friends", "render", "sculk", "agent", "ui", "options",
    "events", "saves", "pad", "studio",
];

/// `hello`: the protocol the socket speaks, the game's release and the command groups it serves.
pub(in crate::game) fn hello() -> Value {
    json!({
        "protocol": PROTOCOL,
        "release": sunrise_core::release::VERSION,
        "features": FEATURES,
    })
}

pub(in crate::game) const HELP: &[&str] = &[
    "hello                          the control protocol's version, the game's release and the command groups it serves",
    "status                         game, player, streaming and renderer state",
    "simulation                     authoritative projectiles in the owner's level",
    "server-hold on|off             hold simulation and player packets at a control-fixture barrier",
    "stall MILLISECONDS             stall the window thread (0..5000), leaving transport alive",
    "key KEY down|up|tap|repeat     press, release or auto-repeat a key (w a s d space shift ctrl e q esc f3 1-9 arrows ...)",
    "release                        release every key and mouse button",
    "mouse left|right|middle down|up|click",
    "look YAW PITCH                 set the view direction in degrees",
    "turn DYAW DPITCH               rotate the view",
    "tp X Y Z                       teleport the player",
    "hotbar 1-9                     select a hotbar slot",
    "give ITEM [COUNT]              add items to the inventory",
    "render-distance N              chunks, 2..128",
    "lod-distance N                 far surface chunks, 0 disables, up to 1024",
    "time TICKS                     set the time of day",
    "summon KIND [X Y Z] [baby]     spawn a mob (in front of the player without a position)",
    "mobs [RADIUS]                  list mobs near the player with their AI state",
    "hurtlog                        the hurts taken and melee attacks made since the last call, per player (host and guests)",
    "fill X1 Y1 Z1 X2 Y2 Z2 BLOCK   set a box of loaded blocks",
    "resume                         leave the pause menu",
    "screenshot PATH                write the next frame as PNG",
    "screenshot-sequence FRAMES STEP_MS PATH|cancel  a shader pack's first frames on a set clock (SUNRISE_STILL), the last as PNG",
    "chat TEXT                      send a chat line or /command as if typed",
    "type TEXT                      type into the open chat box (key enter tap sends)",
    "chat-log [N]                   the newest N chat messages (default 10)",
    "options [PAGE]                 open the options (video sound controls keybinds mouse chat accessibility language font packs mods graphics graphics-advanced graphics-hdr video-vanilla)",
    "stats [reset] [raw]            frame-time summary since the last reset (raw: every frame time too)",
    "profile [client|server] start|stop|status|mark NAME  capture the performance of the window and, in single player, its server thread to reports/ (sunrise-ctl profile --help)",
    "narration                      the narrator's status and the last messages it was asked to say",
    "bench SECONDS [raw]            reset stats, wait, then reply with the summary (raw: every frame time too)",
    "fly X1 Y1 Z1 X2 Y2 Z2 SECONDS [PITCH [raw]]  fly the camera along a line at constant speed, then reply with the frame summary (raw: every frame time too)",
    "open WORLD [SEED] [creative]   open or create a world from the title screen",
    "save                           save the world",
    "save-all                       save the world, rewriting every resident chunk",
    "connect HOST[:PORT] [NAME]     join a multiplayer server (offline mode) from the title screen",
    "chat MESSAGE                   send chat or a /command to the server",
    "disconnect                     leave the server for the server list",
    "respawn                        respawn after death on a server",
    "credits [poem] [SCROLL]        roll the credits (with the end poem), or hold them SCROLL pixels in",
    "quit                           save and exit",
];

/// A straight camera path at constant speed, for repeatable benchmarks: the
/// player is placed on it every frame, facing along it.
pub struct Flight {
    pub(in crate::game) from: Vec3,
    pub(in crate::game) to: Vec3,
    pub(in crate::game) pitch: f32,
    pub(in crate::game) started: Instant,
    pub(in crate::game) seconds: f64,
    /// Reply with every frame time too (`fly ... PITCH raw`).
    pub(in crate::game) raw: bool,
    pub(in crate::game) upload_queue_work: [usize; 2],
    pub(in crate::game) mesh_uploads: [u64; 2],
    pub(in crate::game) reply: Sender<String>,
    /// The world's smoothed tick time each frame (the F3 server line): the
    /// server thread's when single player runs on one.
    pub(in crate::game) tick_ms: Vec<f32>,
}

impl Flight {
    pub(in crate::game::client_side) fn note_upload_queue_work(&mut self, work: [usize; 2]) {
        self.upload_queue_work = work;
    }

    pub(in crate::game::client_side) fn note_mesh_uploads(&mut self, count: usize, bytes: u64) {
        self.mesh_uploads[0] += count as u64;
        self.mesh_uploads[1] += bytes;
    }
}

impl crate::game::ClientGame<'_> {
    pub(in crate::game::client_side) fn record_streaming_work(
        &mut self,
        rendered: bool,
        block_entity_work: [u64; 2],
        spans: &[sunrise_level::spans::Span],
    ) {
        if let Some(flight) = self.client.flight.0.as_mut().filter(|flight| flight.raw) {
            let visited = self
                .common
                .world
                .block_entities
                .take_column_unload_entries();
            let [sorted, merged] = std::mem::take(&mut flight.upload_queue_work);
            let mesh_uploads = std::mem::take(&mut flight.mesh_uploads);
            if rendered {
                self.client.frame_stats.record_streaming_work(
                    visited,
                    sorted,
                    merged,
                    mesh_uploads,
                    block_entity_work,
                    spans,
                );
            }
        }
    }

    /// Move the camera along the running flight; reply when it ends.
    pub(in crate::game::client_side) fn advance_flight(&mut self) {
        let tick_ms = self
            .client
            .integrated
            .as_ref()
            .map_or(0.0, |s| s.smoothed_tick_ms());
        let Some(flight) = self.client.flight.0.as_mut() else {
            return;
        };
        flight.tick_ms.push(tick_ms);
        let flight = &*flight;
        let t = (flight.started.elapsed().as_secs_f64() / flight.seconds).min(1.0);
        let (a, b) = (flight.from, flight.to);
        let d = [b.x - a.x, b.y - a.y, b.z - a.z];
        let at = Vec3::new(a.x + d[0] * t, a.y + d[1] * t, a.z + d[2] * t);
        // Minecraft yaw: 0 faces +Z, 90 faces -X.
        self.common.yaw = (-d[0]).atan2(d[2]).to_degrees() as f32;
        self.common.pitch = flight.pitch;
        if let Some(player) = self.common.player.as_mut() {
            player.position = at;
            player.velocity = Vec3::new(0.0, 0.0, 0.0);
            player.fall_distance = 0.0;
            // Stay where the flight ends (creative flight).
            player.flying = true;
            self.common.previous_position = at;
        }
        if t >= 1.0
            && let Some(flight) = self.client.flight.0.take()
        {
            let mut summary = self.client.frame_stats.summary();
            self.add_async_occlusion_stats(&mut summary);
            let n = flight.tick_ms.len().max(1) as f32;
            summary["tick_ms"] = (flight.tick_ms.iter().sum::<f32>() / n).into();
            summary["tick_ms_max"] = flight.tick_ms.iter().copied().fold(0.0, f32::max).into();
            if flight.raw {
                self.add_raw_series(&mut summary);
                self.client.frame_stats.add_flight_series(&mut summary);
            }
            let _ = flight.reply.send(summary.to_string());
        }
    }
    /// The game's settings a render capture records beside the renderer's own state.
    pub(in crate::game::client_side) fn render_capture_settings(
        &self,
    ) -> sunrise_client::gfx::capture::Settings {
        let s = &self.client.settings;
        sunrise_client::gfx::capture::Settings {
            graphics_api: format!("{:?}", s.sunrise.api),
            translucent_batching: s.sunrise.translucent_batching,
            texture_filtering: s.file.int("textureFiltering"),
            anisotropy_bit: s.file.int("maxAnisotropyBit"),
            mipmap_levels: s.mipmap_levels.max(0) as u32,
            resolution_scale: s.sunrise.resolution_scale as f32 / 100.0,
            fxaa: s.sunrise.fxaa,
            taa: s.sunrise.taa,
            upscale: s.sunrise.upscaling.ratio(),
            render_distance: self.client.streaming.radius,
            lod_distance: self.far_terrain_choice().0,
            resources: self.common.options.resources.clone(),
            atlas: sunrise_client::gfx::capture::AtlasCheck::of(&self.client.client_pack.render),
        }
    }
    /// The frame summary, with Sunrise's own passes' median GPU times while they are timed.
    pub(in crate::game::client_side) fn summary_with_passes(&self) -> Value {
        let mut summary = self.client.frame_stats.summary();
        let passes = self
            .client
            .renderer
            .0
            .as_ref()
            .map(|r| r.own_pass_times())
            .unwrap_or_default();
        if !passes.is_empty() {
            summary["gpu_passes"] = Value::Object(
                passes
                    .into_iter()
                    .map(|(name, ms)| (name, json!(ms)))
                    .collect(),
            );
        }
        if let Some(r) = self.client.renderer.0.as_ref().filter(|r| r.pass_detail())
            && let Some(support) = r.pass_detail_support()
            && let Value::Object(detail) =
                pack_timing::detail_json(&r.own_pass_parts(), &r.own_pass_counters(), support)
            && let Some(summary) = summary.as_object_mut()
        {
            summary.extend(detail);
        }
        self.add_async_occlusion_stats(&mut summary);
        summary
    }

    pub(in crate::game::client_side) fn add_async_occlusion_stats(&self, summary: &mut Value) {
        if let Some(culler) = &self.client.occlusion.0.async_culler {
            summary["async_occlusion_culling"] = culler.stats_value();
        }
    }

    /// Adds the allocations since the last rendered frame to the frame stats; the first frame has no mark.
    pub(in crate::game::client_side) fn record_frame_allocs(&mut self) {
        use sunrise_threads::alloc_count::{here, process};
        let now = (here(), process());
        if let Some((window, all)) = self.client.alloc_mark.replace(now) {
            self.client.frame_stats.record_allocs(
                now.0.since(window),
                now.1.since(all),
                Instant::now(),
            );
        }
    }
    /// Every frame's times, and its allocations in builds that count (`raw`).
    pub(in crate::game::client_side) fn add_raw_series(&self, summary: &mut Value) {
        self.client.frame_stats.add_series(summary);
        if sunrise_threads::alloc_count::ON {
            summary["alloc_count"] = self.client.frame_stats.alloc_counts().into();
            summary["alloc_bytes"] = self.client.frame_stats.alloc_bytes().into();
        }
    }
    pub(in crate::game::client_side) fn reset_frame_stats(&mut self) {
        self.client.frame_stats.reset();
        if let Some(culler) = &self.client.occlusion.0.async_culler {
            culler.reset_stats();
        }
    }
    pub(in crate::game::client_side) fn poll_control(&mut self, event_loop: &dyn EventLoopCtl) {
        if let Some((deadline, reply, raw)) = self.client.pending_bench.0.take() {
            if Instant::now() >= deadline {
                let mut summary = self.summary_with_passes();
                if raw {
                    self.add_raw_series(&mut summary);
                }
                let _ = reply.send(summary.to_string());
            } else {
                self.client.pending_bench.0 = Some((deadline, reply, raw));
            }
        }
        sunrise_client::widgets::end_frame(self.client.frames);
        self.narrate_screen();
        self.poll_repro(event_loop);
        self.poll_profile();
        self.advance_agent();
        for request in self.client.ui_wait.0.ready() {
            self.answer(request, event_loop);
        }
        while let Some(request) = self.common.control.as_ref().and_then(|c| c.try_recv()) {
            // Once per line received, not once per handler: a deferred `ui`
            // comes back through `ui_wait` and must not be traced twice.
            self.note_socket_line(&request.line);
            self.agent_command_seen();
            self.answer(request, event_loop);
            if self.client.pending_save.0.is_some() {
                break;
            }
        }
        // A recording made from the F3 keys or the Settings page has no socket,
        // and still wants its events.
        if self.common.control.is_some() || self.client.repro.0.recording_active() {
            self.watch_events();
        }
        if self.client.screen == Screen::Options
            && self.client.chat_ui.options.page() == sunrise_client::options_ui::Page::Agent
        {
            self.client.chat_ui.options.set_agent_status(
                self.agent_status_lines(),
                self.client.repro.0.recording_active(),
            );
        }
    }

    pub(in crate::game) fn answer(
        &mut self,
        request: sunrise_sim::control::Request,
        event_loop: &dyn EventLoopCtl,
    ) {
        let reply = match self.control_command(&request.line, &request.reply, event_loop) {
            Ok(Some(value)) => value,
            // Deferred: the command keeps its own reply channel.
            Ok(None) => return,
            Err(e) => json!({ "error": e.to_string() }),
        };
        let _ = request.reply.send(reply.to_string());
    }

    /// Text typed at the open screen's text box by the control socket's
    /// `type`: its `\b` is a backspace sentinel for the boxes that have always
    /// taken one, not the characters `\` and `b`. `Err` when no text box of
    /// the screen takes it.
    pub fn type_into_screen(&mut self, text: &str) -> Result<()> {
        if text == "\\b" {
            let instance = self.client.screen_state.0.screen_instance;
            if self.mounted_widget_key_pressed(
                KeyCode::Backspace,
                false,
                crate::game::client_side::key_codes::glfw_modifiers(self.client.modifiers),
            ) {
                return Ok(());
            }
            if self.client.screen_state.0.screen_instance != instance
                || self.client.screen_state.0.widgets.has_focus()
            {
                return Err("no focused editor accepted Backspace".into());
            }
            if self.dialog_open() && self.dialog_type(text) {
                return Ok(());
            }
            if self.prompt_open() {
                return Err("no focused editor accepted Backspace".into());
            }
            match self.client.screen {
                Screen::WorldOptions => {
                    self.world_options_text(None);
                    return Ok(());
                }
                Screen::Inventory | Screen::Creative if self.mod_browser_typing() => {
                    self.mod_browser_text(None, true);
                    return Ok(());
                }
                Screen::Social => {
                    self.social_backspace();
                    return Ok(());
                }
                _ => return Err("no focused editor accepted Backspace".into()),
            }
        }
        self.type_chars_into_screen(text)
    }

    /// Characters the open screen's focused text box takes: the control
    /// socket's `type` and a key press's surviving characters (`charTyped`).
    /// Nothing here is a key — the caller drops control characters. `Err` when
    /// no text box of the screen takes them.
    pub(in crate::game) fn type_chars_into_screen(&mut self, text: &str) -> Result<()> {
        if self.client.screen_state.0.widgets.has_focus() {
            return Err("focused mounted widget declined character input".into());
        }
        if self.dialog_open() && self.dialog_type(text) {
            return Ok(());
        }
        if self.prompt_open() {
            return Err("no focused editor accepted character input".into());
        }
        if self.client.book.is_some() {
            if matches!(
                self.client.book,
                Some(super::books::BookScreen::Edit { .. })
            ) {
                self.book_type(text);
                return Ok(());
            }
            return Err("the open book is not editable".into());
        }
        if self.client.sign_edit.is_some() {
            self.sign_input(None, Some(text));
            return Ok(());
        }
        // The title's screens: the world name, the seed, a server's address.
        // A screen opened over the title (Options, say) keeps its own text.
        if self.preset_editor_title().is_some() {
            return if self.preset_editor_type_text(text) {
                Ok(())
            } else {
                Err("preset editor did not consume character".into())
            };
        }
        if title_owns_text(&self.client.screen, self.client.title.0.is_some()) {
            if self.vanilla_text_field_focused() {
                self.title_edit(Some(text), false);
                return Ok(());
            }
            return Err("the title screen has no focused editor".into());
        }
        let screen = self.client.screen;
        match screen {
            Screen::Chat => self.chat_type(text),
            Screen::CommandBlock => self.command_block_type(text),
            Screen::Social => self.social_type(text),
            Screen::BlockEditor if self.vanilla_text_field_focused() => {
                self.block_editor_type(text)
            }
            Screen::DebugOptions if self.vanilla_text_field_focused() => {
                self.debug_options_type(text)
            }
            Screen::Options
                if self.client.chat_ui.options.typing() && !self.options_waiting_for_key() =>
            {
                self.client
                    .chat_ui
                    .options
                    .type_text(&self.client.gui.atlas, text)
            }
            // The port, or the game rules' search and number boxes.
            Screen::WorldOptions if self.world_options_typing() => {
                self.world_options_text(Some(text))
            }
            Screen::Inventory | Screen::Creative if self.mod_browser_typing() => {
                self.mod_browser_text(Some(text), false)
            }
            Screen::Inventory if self.recipe_search_focused() => {
                self.recipe_text(Some(text), false)
            }
            // The anvil's rename box, while it has an input to name.
            Screen::Inventory if self.station_typing() => self.station_text(Some(text), false),
            Screen::Creative if self.client.creative_screen.0.tab().search => {
                // CreativeModeInventoryScreen.charTyped: the chat key that
                // opened the tab does not type its letter.
                if std::mem::take(&mut self.client.creative_ignore_text) {
                    return Ok(());
                }
                // As typed: replaces a selected query, keeps the length limit.
                self.creative_search_edit(Some(text))
            }
            _ => {
                return Err(
                    "type needs the chat screen (key t tap), a sign or book, the language search, the creative search, the anvil or World Options".into(),
                );
            }
        }
        Ok(())
    }

    pub fn control_command(
        &mut self,
        line: &str,
        reply: &Sender<String>,
        event_loop: &dyn EventLoopCtl,
    ) -> Result<Option<Value>> {
        // A tap's or an up's release must run even when a dialog, the sign
        // editor or a screen consumes the key, there is no world, or a
        // command handler returns an error: a lost release keeps Shift down.
        let released = released_modifier(line);
        let result = self.control_command_inner(line, reply, event_loop);
        self.settle_title_mode();
        // After handling, as the window's: the mods' mappings and `input_key` hear the key.
        if let Some((key, action)) = socket_key(line) {
            self.socket_key_input(key, action);
        }
        if let Some(key) = released {
            self.press_modifier_key(key, false);
        }
        result
    }

    /// A recorded control line fed back by a replay: not new input, so the
    /// trace does not see it, and its reply has no one waiting.
    pub fn replay_line(&mut self, line: &str, event_loop: &dyn EventLoopCtl) {
        let (reply, _) = std::sync::mpsc::channel();
        if let Err(e) = self.control_command(line, &reply, event_loop) {
            log::warn!("Sunrise: replay: {line}: {e}");
        }
    }

    pub(in crate::game::client_side) fn control_command_inner(
        &mut self,
        line: &str,
        reply: &Sender<String>,
        event_loop: &dyn EventLoopCtl,
    ) -> Result<Option<Value>> {
        let words: Vec<&str> = line.split_whitespace().collect();
        self.screenshot_sequence_gate(words.first().copied())?;
        if words.first() == Some(&"screenshot-sequence") {
            return self.screenshot_sequence_command(line, reply);
        }
        if self.client.pending_save.0.is_some() && words.first() != Some(&"quit") {
            return Err("a world save is in progress".into());
        }
        let arg = |i: usize| {
            words
                .get(i)
                .copied()
                .ok_or_else(|| format!("missing argument {i} for {}", words[0]))
        };
        let ok = || Ok(Some(json!({ "ok": true })));
        if let Some(value) = self.block_command(&words)? {
            return Ok(Some(value));
        }
        match self.ui_command(&words, line, reply, event_loop)? {
            super::agent::Handled::Reply(value) => return Ok(Some(value)),
            super::agent::Handled::Deferred => return Ok(None),
            super::agent::Handled::No => {}
        }
        if let Some(value) = self.events_command(&words, reply)? {
            return Ok(value);
        }
        if let Some(value) = self.client.input_trace.0.command(&words)? {
            return Ok(Some(value));
        }
        if let Some(value) = self.repro_command(&words, line, reply)? {
            return Ok(value);
        }
        if words[0] == "profile" {
            return self.profile_command(&words[1..], reply);
        }
        if words[0] == "trace" {
            return self.trace_command(&words[1..], reply);
        }
        if let Some(value) = self.saves_control(&words, line, reply)? {
            return Ok(value);
        }
        if let Some(value) = self.options_control(&words, line)? {
            return Ok(Some(value));
        }
        if let Some(value) = self.lan_op_control(&words)? {
            return Ok(Some(value));
        }
        if let Some(value) = self.lan_control(&words)? {
            return Ok(Some(value));
        }
        if let Some(value) = self.friends_control(&words)? {
            return Ok(Some(value));
        }
        if let Some(value) = self.studio_control(&words)? {
            return Ok(Some(value));
        }
        match self.agent_command(&words, line, reply, event_loop)? {
            super::agent::Handled::Reply(value) => return Ok(Some(value)),
            super::agent::Handled::Deferred => return Ok(None),
            super::agent::Handled::No => {}
        }
        match words[0] {
            "help" => Ok(Some(json!({ "commands": HELP
                .iter()
                .chain(super::station_control::HELP)
                .chain(super::blockctl::HELP)
                .chain(sunrise_gameplay_common::game::lan_common::HELP)
                .chain(super::friends::HELP)
                .chain(super::render_control::HELP)
                .chain(sunrise_gameplay_common::game::sculk_control_common::HELP)
                .chain(super::agent::HELP)
                .chain(super::ui_control::HELP)
                .chain(super::options::CONTROL_HELP)
                .chain(super::events::HELP)
                .chain(super::saves_control::HELP)
                .chain(super::pad::HELP)
                .chain(super::input_trace::HELP)
                .chain(super::repro::HELP)
                .chain(super::studio::HELP)
                .collect::<Vec<_>>() }))),
            "setblock" | "fill" | "particle" | "signedit" => self.render_command(&words),
            "section-draws" => {
                let r = self.client.renderer.0.as_mut().ok_or("no renderer")?;
                match words.get(1).copied() {
                    None => {}
                    Some("flat") => r.set_flat_draws(true)?,
                    Some("indirect") => r.set_flat_draws(false)?,
                    Some(other) => return Err(format!("unknown section draw mode {other}").into()),
                }
                Ok(Some(json!({ "ok": true, "flat": r.flat_draws() })))
            }
            "section-layout" => {
                let r = self.client.renderer.0.as_mut().ok_or("no renderer")?;
                let compact = match words.get(1).copied() {
                    Some("compact") => r.set_packed_layout(true),
                    Some("wide") => r.set_packed_layout(false),
                    _ => return Err("section-layout compact|wide".into()),
                };
                Ok(Some(json!({ "ok": true, "compact": compact })))
            }
            "section-cull" => {
                let r = self.client.renderer.0.as_mut().ok_or("no renderer")?;
                match words.get(1).copied() {
                    None => {}
                    Some("on") => r.set_section_cull(true)?,
                    Some("off") => r.set_section_cull(false)?,
                    Some(other) => return Err(format!("unknown section cull mode {other}").into()),
                }
                Ok(Some(json!({ "ok": true, "cull": r.section_cull() })))
            }
            "gpu-passes" => {
                (self.client.gpu_passes, self.client.gpu_pass_detail) = gpu_pass_switches(
                    words.get(1).copied(),
                    (self.client.gpu_passes, self.client.gpu_pass_detail),
                )?;
                let support = self
                    .client
                    .renderer
                    .0
                    .as_ref()
                    .and_then(|r| r.pass_detail_support());
                Ok(Some(gpu_passes_reply(
                    self.client.gpu_passes,
                    self.client.gpu_pass_detail,
                    support,
                )))
            }
            "render-capture" => match words.get(1).copied() {
                None => {
                    let r = self.client.renderer.0.as_ref().ok_or("no renderer")?;
                    Ok(Some(
                        json!({ "ok": true, "keeping": r.keeps_capture_copies() }),
                    ))
                }
                Some("on") => {
                    let r = self.client.renderer.0.as_mut().ok_or("no renderer")?;
                    if !r.keeps_capture_copies() {
                        r.keep_capture_copies(true);
                        // The copies start empty: every section and far tile is meshed
                        // again, as after a relayout.
                        self.clear_lod();
                        self.all_sections_changed();
                    }
                    Ok(Some(json!({ "ok": true, "keeping": true })))
                }
                Some("off") => {
                    let r = self.client.renderer.0.as_mut().ok_or("no renderer")?;
                    r.keep_capture_copies(false);
                    Ok(Some(json!({ "ok": true, "keeping": false })))
                }
                Some(_) => {
                    if self.client.pending_render_capture.0.is_some() {
                        return Err("a render capture is already pending".into());
                    }
                    // The rest of the line: a path may hold spaces.
                    let dir = line
                        .trim_start()
                        .strip_prefix("render-capture")
                        .unwrap_or("");
                    let settings = self.render_capture_settings();
                    let r = self.client.renderer.0.as_mut().ok_or("no renderer")?;
                    if !r.keeps_capture_copies() {
                        return Err("no mesh copies kept: run render-capture on (or start with \
                            SUNRISE_RENDER_CAPTURE=1) and let the terrain settle first"
                            .into());
                    }
                    r.capture = Some(sunrise_client::gfx::capture::Request {
                        dir: dir.trim().into(),
                        settings,
                    });
                    self.client.pending_render_capture.0 = Some(reply.clone());
                    Ok(None)
                }
            },
            "section-sides" => {
                let r = self.client.renderer.0.as_mut().ok_or("no renderer")?;
                match words.get(1).copied() {
                    None => {}
                    Some("on") => r.set_section_sides(true),
                    Some("off") => r.set_section_sides(false),
                    Some(other) => return Err(format!("unknown section sides mode {other}").into()),
                }
                Ok(Some(json!({ "ok": true, "sides": r.section_sides() })))
            }
            "parallel-tick" => {
                use sunrise_client::graphics::ParallelTickMode as Mode;
                match words.get(1).copied() {
                    None => {}
                    Some(mode @ ("off" | "auto" | "on")) => {
                        // The setting, as its switch sets it; not saved here (as `gpu-culling`).
                        self.client.settings.sunrise.parallel_tick = match mode {
                            "on" => Mode::On,
                            "auto" => Mode::Auto,
                            _ => Mode::Off,
                        };
                        sunrise_sim::parallel::set_setting(
                            self.client.settings.sunrise.parallel_tick,
                        );
                    }
                    Some(other) => return Err(format!("unknown parallel tick mode {other}").into()),
                }
                let (setting, on, auto) = sunrise_sim::parallel::status();
                Ok(Some(
                    json!({ "ok": true, "setting": setting, "on": on, "auto": auto }),
                ))
            }
            "gpu-culling" => {
                let r = self.client.renderer.0.as_mut().ok_or("no renderer")?;
                match words.get(1).copied() {
                    None => {}
                    Some(mode @ ("on" | "off" | "auto")) => {
                        // Refused, with the reason, where it isn't offered.
                        if let Some(why) = r.gpu_culling_offer().reason() {
                            return Err(why.into());
                        }
                        // Auto starts from off and decides from the frames that follow.
                        self.client.gpu_culling_auto.reset();
                        r.set_gpu_culling(mode == "on")?;
                        // The setting, as its switch sets it; not saved here (as
                        // `lod-distance`): settings changes and new renderers keep it.
                        self.client.settings.sunrise.gpu_culling = match mode {
                            "on" => sunrise_client::graphics::GpuCullingMode::On,
                            "auto" => sunrise_client::graphics::GpuCullingMode::Auto,
                            _ => sunrise_client::graphics::GpuCullingMode::Off,
                        };
                    }
                    Some(other) => return Err(format!("unknown GPU culling mode {other}").into()),
                }
                Ok(Some(json!({
                    "ok": true,
                    "gpu_culling": r.gpu_culling(),
                    "mode": self.client.settings.sunrise.gpu_culling.name(),
                    "auto": self.client.gpu_culling_auto.state(),
                })))
            }
            "sculk" => {
                let server = self
                    .client
                    .integrated
                    .as_ref()
                    .ok_or("sculk requires a single-player world")?;
                let words = words.iter().map(|w| (*w).to_owned()).collect();
                let value = server
                    .ask(|reply| OwnerRequest::Control(ControlRequest::Sculk { words, reply }))
                    .ok_or(NO_ANSWER)??;
                Ok(Some(value))
            }
            "hello" => Ok(Some(hello())),
            "status" => Ok(Some(self.control_status())),
            "memory" => Ok(Some(self.memory_value())),
            "stall" => {
                let millis: u64 = arg(1)?.parse()?;
                if millis > 5000 {
                    return Err("stall is limited to 5000 milliseconds".into());
                }
                std::thread::sleep(Duration::from_millis(millis));
                ok()
            }
            "simulation" => {
                self.require_world()?;
                // `simulation blocks X1 Y1 Z1 X2 Y2 Z2`: the server's own blocks in a box. `blocks` reads this
                // window's copy, which the server's changes reach in the next tick's broadcast.
                if words.get(1) == Some(&"blocks") {
                    let mut c = [0i32; 6];
                    for (i, v) in c.iter_mut().enumerate() {
                        *v = arg(i + 2)?.parse()?;
                    }
                    let lo = [0, 1, 2].map(|i| c[i].min(c[i + 3]));
                    let hi = [0, 1, 2].map(|i| c[i].max(c[i + 3]));
                    let volume: i64 = (0..3).map(|i| i64::from(hi[i] - lo[i] + 1)).product();
                    if volume > 32768 {
                        return Err(format!("{volume} blocks; at most 32768 per request").into());
                    }
                    return Ok(Some(if let Some(server) = &self.client.integrated {
                        server
                            .ask(|reply| {
                                OwnerRequest::Control(ControlRequest::Blocks { lo, hi, reply })
                            })
                            .ok_or(NO_ANSWER)?
                    } else {
                        self.common.blocks_json(lo, hi)
                    }));
                }
                let value = if let Some(server) = &self.client.integrated {
                    server
                        .ask(|reply| OwnerRequest::Control(ControlRequest::Simulation(reply)))
                        .ok_or(NO_ANSWER)?
                } else {
                    json!({ "game_time": self.world.game_time, "projectiles": self.client_projectiles_status() })
                };
                Ok(Some(value))
            }
            "server-hold" => {
                self.require_world()?;
                let held = match arg(1)? {
                    "on" => true,
                    "off" => false,
                    _ => return Err("server-hold on|off".into()),
                };
                if let Some(server) = &self.client.integrated {
                    server
                        .ask(|done| {
                            OwnerRequest::Control(ControlRequest::HoldServer { held, done })
                        })
                        .ok_or(NO_ANSWER)?;
                } else if self.client.remote.is_some() {
                    return Err("server-hold requires the world's host".into());
                } else {
                    self.client.agent.hold_server = held;
                }
                ok()
            }
            // The real event, so focus loss reaches all that a window's does (the radial menu
            // cancels, held keys go).
            "pad" if words.get(1) == Some(&"focus") => {
                let focused = match arg(2)? {
                    "true" => true,
                    "false" => false,
                    _ => return Err("pad focus true|false".into()),
                };
                let id = self
                    .client
                    .window
                    .0
                    .as_ref()
                    .map_or_else(winit::window::WindowId::dummy, |w| w.id());
                self.handle_window_event(
                    event_loop,
                    id,
                    winit::event::WindowEvent::Focused(focused),
                    super::input_trace::InputOrigin::Agent,
                );
                ok()
            }
            "pad" => Ok(Some(self.pad_command(&words)?)),
            "key" => {
                let key = key_code(arg(1)?).ok_or_else(|| format!("unknown key {}", words[1]))?;
                let action = arg(2)?;
                if !matches!(action, "down" | "up" | "tap" | "repeat") {
                    return Err(format!(
                        "key action must be down, up, tap or repeat, not {action}"
                    )
                    .into());
                }
                if action != "up" {
                    self.input_keyboard_used();
                    // As a window's key press: the radial menu closes, and the key goes on.
                    self.pad_radial_cancel();
                }
                if self.observed_screen_view().is_none()
                    && !self.title_input_active()
                    && !self.dialog_open()
                    && !self.prompt_open()
                {
                    self.require_world()?;
                }
                super::narrator::narrate_key(&mut self.client.narration, key, action != "up");
                // KeyboardHandler.keyPress: the debug keys see the physical key first.
                let physical = key;
                let repeat = action == "repeat";
                let menu_active = self.context_menu_key_active();
                let menu_modal = menu_active
                    && (self.context_menu_open()
                        || ((action == "up" || repeat)
                            && self.context_menu_owns_key(Source::Agent, physical)));
                {
                    use sunrise_client::debug_keys::KeyAction;
                    if action == "up" {
                        self.press_modifier_key(key, false);
                        if !menu_active
                            || !self
                                .client
                                .screen_state
                                .0
                                .context_menu_input
                                .holds_other_source(Source::Agent, key)
                        {
                            self.client.physical_keys.remove(&key);
                        }
                        if menu_modal {
                            self.debug_key_release_cleanup(key);
                        } else {
                            self.debug_key(key, KeyAction::Release);
                        }
                    } else {
                        self.press_modifier_key(key, true);
                        self.client.physical_keys.insert(key);
                        if !menu_modal && self.debug_key(key, KeyAction::Press) {
                            if action == "tap" {
                                self.press_modifier_key(key, false);
                                if !self
                                    .client
                                    .screen_state
                                    .0
                                    .context_menu_input
                                    .holds_other_source(Source::Agent, key)
                                {
                                    self.client.physical_keys.remove(&key);
                                }
                                self.debug_key(key, KeyAction::Release);
                            }
                            return ok();
                        }
                    }
                }
                if action != "up" {
                    if self.screenshot_shortcut(physical, repeat) {
                        if action == "tap" {
                            self.finish_control_tap_release(physical);
                        }
                        return ok();
                    }
                    let name = arg(1)?;
                    let letter = name.chars().next().filter(|_| name.len() == 1);
                    let shortcut = self.edit_shortcut_for_letter(letter);
                    if !self.dispatch_screen_key(
                        event_loop,
                        physical,
                        true,
                        repeat,
                        shortcut,
                        Source::Agent,
                    )? && !self.key_press_canceled(physical, true, repeat)
                    {
                        self.native_panel_key_event(physical, true, repeat);
                        self.mapped_key_event_from(physical, true, repeat, Source::Agent)?;
                    }
                }
                if action == "up" || action == "tap" {
                    if action == "tap" {
                        self.settle_title_mode();
                        self.finish_control_tap_release(physical);
                    }
                    if !self.dispatch_screen_key(
                        event_loop,
                        physical,
                        false,
                        false,
                        None,
                        Source::Agent,
                    )? && !self.key_press_canceled(physical, false, false)
                    {
                        self.native_panel_key_event(physical, false, false);
                        self.mapped_key_event_from(physical, false, false, Source::Agent)?;
                        self.release_control_keyboard_standin(physical);
                    }
                }
                ok()
            }
            "release" => {
                self.clear_device_keys();
                self.client.physical_keys.clear();
                self.client.screen_state.0.context_menu_input.clear_agent();
                self.client.mod_keys.set_all_up();
                self.client.modifiers = winit::keyboard::ModifiersState::empty();
                let options = self.toggle_options();
                self.client
                    .vanilla_mappings
                    .set_keys_up_on_focus_loss(options);
                self.set_mapping_held("key.attack", false);
                self.set_mapping_held("key.use", false);
                ok()
            }
            "mouse" => {
                let button = match arg(1)? {
                    "left" => MouseButton::Left,
                    "right" => MouseButton::Right,
                    "middle" => MouseButton::Middle,
                    other => return Err(format!("unknown mouse button {other}").into()),
                };
                self.require_world()?;
                if self.client.screen != Screen::Playing {
                    return Err(format!(
                        "mouse needs the playing screen, not {}",
                        self.screen_name()
                    )
                    .into());
                }
                // As a window's click: the radial menu closes, and the click goes on.
                if arg(2)? != "up" {
                    self.pad_radial_cancel();
                }
                let action = arg(2)?;
                if matches!(action, "down" | "click") {
                    self.mod_mappings_button(button, true);
                }
                if matches!(action, "up" | "click") {
                    self.mod_mappings_button(button, false);
                }
                let input = sunrise_client::keybinds::mouse_name(button);
                match action {
                    "down" => {
                        self.mapped_mouse_event_from(button, &input, true, Source::Agent)?;
                    }
                    "up" => {
                        self.mapped_mouse_event_from(button, &input, false, Source::Agent)?;
                    }
                    "click" => {
                        self.mapped_mouse_event_from(button, &input, true, Source::Agent)?;
                        self.mapped_mouse_event_from(button, &input, false, Source::Agent)?;
                    }
                    other => {
                        return Err(
                            format!("mouse action must be down, up or click, not {other}").into(),
                        );
                    }
                }
                ok()
            }
            "look" => {
                self.common.yaw = arg(1)?.parse()?;
                self.common.pitch = arg(2)?.parse::<f32>()?.clamp(-90.0, 90.0);
                ok()
            }
            "turn" => {
                let yaw = arg(1)?.parse::<f32>()?;
                let pitch = arg(2)?.parse::<f32>()?;
                // Degrees, as `Entity.turn` degrees.
                self.turn_player(f64::from(yaw / 0.15), f64::from(pitch / 0.15));
                ok()
            }
            "tp" => {
                let to = Vec3::new(arg(1)?.parse()?, arg(2)?.parse()?, arg(3)?.parse()?);
                let player = self.common.player.as_mut().ok_or("no player yet")?;
                player.position = to;
                player.velocity = Vec3::new(0.0, 0.0, 0.0);
                player.fall_distance = 0.0;
                self.common.previous_position = to;
                ok()
            }
            "hotbar" => {
                let slot: usize = arg(1)?.parse()?;
                if !(1..=9).contains(&slot) {
                    return Err("hotbar slot must be 1..9".into());
                }
                self.common.selected = slot - 1;
                ok()
            }
            "give" => {
                self.require_world()?;
                let name = arg(1)?;
                let item = if name.contains(':') {
                    name.to_owned()
                } else {
                    format!("minecraft:{name}")
                };
                let info = self
                    .common
                    .res
                    .items()
                    .get(&item)
                    .ok_or_else(|| format!("unknown item {item}"))?;
                let count: u32 = words.get(2).map_or(Ok(1), |c| c.parse())?;
                // Single player's inventory is its server thread's player's.
                if let Some(server) = &self.client.integrated {
                    let command = format!("give @s {item} {count}");
                    server
                        .ask(|reply| {
                            OwnerRequest::Control(ControlRequest::ConsoleCommand { command, reply })
                        })
                        .ok_or(NO_ANSWER)?;
                    return Ok(Some(json!({ "ok": true, "given": count })));
                }
                let max = info.max_stack;
                let mut given = 0;
                // Stop once the inventory is full.
                while given < count {
                    let n = (count - given).min(max);
                    let rest = self
                        .add_stack_to_inventory(ItemStack {
                            item: item.as_str().into(),
                            count: n,
                            damage: 0,
                            extra: None,
                        })
                        .map_or(0, |r| r.count);
                    given += n - rest;
                    if rest > 0 {
                        break;
                    }
                }
                Ok(Some(json!({ "ok": true, "given": given })))
            }
            "render-distance" => {
                let n: i32 = arg(1)?.parse()?;
                let max = sunrise_level::sim_settings::MAX_RENDER_DISTANCE;
                if !(2..=max).contains(&n) {
                    return Err(format!("render distance must be 2..{max}").into());
                }
                self.client.settings.render_distance = n;
                self.apply_settings();
                ok()
            }
            "lod-distance" => {
                let n: u32 = arg(1)?.parse()?;
                if n > sunrise_client::lod::MAX_CHUNKS {
                    return Err(format!(
                        "LOD distance must be 0..{}",
                        sunrise_client::lod::MAX_CHUNKS
                    )
                    .into());
                }
                self.client.settings.sunrise.lod_distance = n;
                self.apply_settings();
                ok()
            }
            "time" => {
                let time: i64 = arg(1)?.parse()?;
                self.common.world.day_time = time;
                // Single player's clock is its server thread's.
                if let Some(server) = &self.client.integrated {
                    server
                        .ask(|done| {
                            OwnerRequest::Control(ControlRequest::SetDayTime { time, done })
                        })
                        .ok_or(NO_ANSWER)?;
                }
                ok()
            }
            "summon" => {
                self.require_world()?;
                let baby = words.contains(&"baby");
                let nums: Vec<f64> = words[2..].iter().filter_map(|w| w.parse().ok()).collect();
                let at = (nums.len() >= 3).then(|| [nums[0], nums[1], nums[2]]);
                let kind = arg(1)?.to_owned();
                // Single player on its server thread: spawn in the owner's level
                // so the mob enters the server tick loop and is tracked to clients.
                if let Some(server) = &self.client.integrated {
                    let at = match at {
                        Some(a) => a,
                        None => self.common.default_summon_position()?,
                    };
                    let id = server
                        .ask(|reply| {
                            OwnerRequest::Control(ControlRequest::Summon {
                                kind,
                                at,
                                baby,
                                reply,
                            })
                        })
                        .ok_or(NO_ANSWER)??;
                    return Ok(Some(json!({ "ok": true, "id": id })));
                }
                Err("summon requires an integrated server".into())
            }
            "mobs" => {
                self.require_world()?;
                let radius: f64 = words.get(1).map_or(Ok(64.0), |r| r.parse())?;
                if let Some(server) = &self.client.integrated {
                    let center = self
                        .common
                        .player
                        .as_ref()
                        .map_or(Vec3::ZERO, |p| p.position);
                    let value = server
                        .ask(|reply| {
                            OwnerRequest::Control(ControlRequest::MobsAt {
                                radius,
                                center,
                                reply,
                            })
                        })
                        .ok_or(NO_ANSWER)?;
                    return Ok(Some(value));
                }
                Ok(Some(self.common.control_mobs(radius)))
            }
            "hurtlog" => {
                self.require_world()?;
                if let Some(server) = &self.client.integrated {
                    let value = server
                        .ask(|reply| OwnerRequest::Control(ControlRequest::HurtLog(reply)))
                        .ok_or(NO_ANSWER)?;
                    return Ok(Some(value));
                }
                Ok(Some(self.client_control_hurt_log()))
            }
            "resume" => {
                if self.client.screen == Screen::Paused {
                    self.set_screen(Screen::Playing);
                }
                ok()
            }
            "screenshot" => {
                let renderer = self.client.renderer.0.as_mut().ok_or("no renderer yet")?;
                if self.client.pending_screenshot.0.is_some() {
                    return Err("a screenshot is already pending".into());
                }
                arg(1)?;
                // The rest of the line: a path may hold spaces.
                let path = line.trim_start().strip_prefix("screenshot").unwrap_or("");
                renderer.screenshot = Some(sunrise_client::gfx::ScreenshotRequest {
                    path: path.trim().into(),
                    size: None,
                    scaled: false,
                    background: false,
                });
                self.client.pending_screenshot.0 = Some(reply.clone());
                Ok(None)
            }
            "chat" => {
                let text = line.trim_start().strip_prefix("chat").unwrap_or("").trim();
                if self.client.remote.is_some() {
                    // As typed into the chat screen: history, then the client chat event.
                    self.handle_chat_input(text);
                    return ok();
                }
                self.require_world()?;
                // As if typed: a client mod's command runs before the line
                // reaches the server, and the Chat option's restrictions and
                // the recent list apply to a connected server too.
                self.handle_chat_input(text);
                Ok(Some(
                    json!({ "ok": true, "chat": self.client.chat_ui.chat.recent_lines(5) }),
                ))
            }
            "type" => {
                let text = line
                    .trim_start()
                    .strip_prefix("type")
                    .unwrap_or("")
                    .strip_prefix(' ')
                    .unwrap_or("");
                if self.observed_screen_view().is_none()
                    && !self.dialog_open()
                    && !self.title_input_active()
                {
                    self.require_world()?;
                }
                self.client.narration.keyboard_action();
                if self.context_menu_open() {
                    return ok();
                }
                if text == "\\b" {
                    if self.preset_editor_title().is_some() {
                        self.dispatch_screen_key(
                            event_loop,
                            KeyCode::Backspace,
                            true,
                            false,
                            None,
                            Source::Agent,
                        )?;
                    } else {
                        self.type_into_screen(text)?;
                    }
                    return ok();
                }
                let mut typed = Ok(());
                if self.observed_screen_view().is_some() {
                    self.screen_chars_dispatch(text, |game, c| {
                        if game.preset_editor_title().is_some() {
                            return game.preset_editor_type_text(c);
                        }
                        if typed.is_err() {
                            return false;
                        }
                        match game.type_into_screen(c) {
                            Ok(()) => true,
                            Err(e) => {
                                typed = Err(e);
                                false
                            }
                        }
                    });
                } else {
                    self.screen_text_each(text, |game, c| {
                        if typed.is_ok() {
                            typed = game.type_into_screen(c);
                        }
                    });
                }
                typed?;
                ok()
            }
            "options" => {
                use sunrise_client::options_ui::Page;
                let page = match words.get(1).copied().unwrap_or("main") {
                    "main" => Page::Main,
                    // `video` keeps its published target, Sunrise's graphics screen (as `graphics`);
                    // `video-vanilla` is vanilla's Video Settings, which Video Settings... opens.
                    "video" => Page::Graphics,
                    "video-vanilla" => Page::Video,
                    "sound" => Page::Sound,
                    "controls" => Page::Controls,
                    "keybinds" => Page::KeyBinds,
                    "controller" => Page::Controller,
                    "controller-binds" => Page::PadBinds,
                    "controller-radial" => Page::ControllerRadial,
                    "mouse" => Page::Mouse,
                    "chat" => Page::Chat,
                    "accessibility" => Page::Accessibility,
                    "language" => Page::Language,
                    "font" => Page::Font,
                    "packs" => Page::Packs,
                    "mods" => Page::Mods,
                    "shader-packs" => Page::ShaderPacks,
                    "graphics" => Page::Graphics,
                    "graphics-advanced" => Page::GraphicsAdvanced,
                    "graphics-hdr" => Page::GraphicsHdr,
                    "extras" => Page::Extras,
                    "pvp" => Page::Pvp,
                    "credits" => Page::Credits,
                    "sunrise" => Page::Sunrise,
                    "agent" => Page::Agent,
                    other => return Err(format!("unknown options page {other}").into()),
                };
                if self.client.screen == Screen::Playing && self.client.title.0.is_none() {
                    self.capture(false);
                }
                self.open_options(self.client.title.0.is_some());
                self.client.chat_ui.options.show(page);
                if sunrise_client::options_ui::OptionsUi::lists_shader_packs(page) {
                    self.client.settings.shader_packs =
                        sunrise_client::shader_pack_list::scan(&Self::shader_packs_dir());
                    // A pack may have been replaced under the same name.
                    self.client.native_shaders.pick_hdr.get_mut().forget();
                    self.open_shader_options();
                }
                ok()
            }
            "chat-log" => {
                let n = words.get(1).map_or(Ok(10), |n| n.parse())?;
                let lines = self
                    .remote_chat(n)
                    .unwrap_or_else(|| self.client.chat_ui.chat.recent_lines(n));
                Ok(Some(json!({ "chat": lines })))
            }
            "narration" => {
                // NarratorStatus's options.txt names: off, all, chat, system.
                let status = sunrise_narrator::status().key().rsplit('.').next();
                Ok(Some(json!({
                    "status": status,
                    "active": sunrise_narrator::is_active(),
                    "recent": sunrise_narrator::recent(),
                })))
            }
            "stats" => {
                let mut summary = self.summary_with_passes();
                if words[1..].contains(&"raw") {
                    self.add_raw_series(&mut summary);
                }
                if words[1..].contains(&"reset") {
                    self.reset_frame_stats();
                }
                Ok(Some(summary))
            }
            "bench" => {
                let seconds: f64 = arg(1)?.parse()?;
                // `Duration::from_secs_f64` panics on a negative, NaN or huge number.
                if !(seconds.is_finite() && (0.0..=86_400.0).contains(&seconds)) {
                    return Err("bench takes 0 to 86400 seconds".into());
                }
                if self.client.pending_bench.0.is_some() {
                    return Err("a benchmark is already running".into());
                }
                self.reset_frame_stats();
                self.client.pending_bench.0 = Some((
                    Instant::now() + Duration::from_secs_f64(seconds),
                    reply.clone(),
                    words[2..].contains(&"raw"),
                ));
                Ok(None)
            }
            "fly" => {
                self.require_world()?;
                if self.client.flight.0.is_some() || self.client.pending_bench.0.is_some() {
                    return Err("a benchmark is already running".into());
                }
                let n = |i: usize| -> Result<f64> { Ok(arg(i)?.parse()?) };
                let pitch = words.get(8).map_or(Ok(10.0), |p| p.parse::<f32>())?;
                let raw = words.get(9) == Some(&"raw");
                if raw {
                    self.common
                        .world
                        .block_entities
                        .take_column_unload_entries();
                }
                self.reset_frame_stats();
                self.client.flight.0 = Some(Flight {
                    from: Vec3::new(n(1)?, n(2)?, n(3)?),
                    to: Vec3::new(n(4)?, n(5)?, n(6)?),
                    pitch,
                    started: Instant::now(),
                    seconds: n(7)?.max(0.1),
                    raw,
                    upload_queue_work: [0; 2],
                    mesh_uploads: [0; 2],
                    reply: reply.clone(),
                    tick_ms: Vec::new(),
                });
                Ok(None)
            }
            "open" => {
                if self.client.title.0.is_none() {
                    return Err("a world is already open".into());
                }
                let folder = arg(1)?;
                let seed = words.get(2).map(|s| s.parse::<i64>()).transpose()?;
                let creative = words.get(3) == Some(&"creative");
                self.open_world(folder, Some(folder), seed, creative, None)?;
                ok()
            }
            "connect" => {
                if self.client.title.0.is_none() {
                    return Err("leave the current world first".into());
                }
                let name = words
                    .get(2)
                    .map_or_else(|| self.common.options.player_name(), |n| (*n).to_owned());
                self.connect_server(arg(1)?, &name)?;
                ok()
            }
            "credits" => {
                let poem = words.get(1) == Some(&"poem");
                let hold = words
                    .get(1 + usize::from(poem))
                    .map(|s| s.parse::<f32>())
                    .transpose()?;
                if self.client.screen != Screen::Win {
                    let finish = if self.client.screen == Screen::Options {
                        super::win::Finish::Options
                    } else {
                        super::win::Finish::Closed
                    };
                    self.open_win_screen(poem, finish);
                }
                if let Some(scroll) = hold {
                    self.hold_win_screen(scroll);
                }
                ok()
            }
            "respawn" => {
                if self.client.remote.is_none() || self.client.screen != Screen::Dead {
                    return Err("respawn needs the death screen on a server".into());
                }
                self.remote_respawn();
                ok()
            }
            "disconnect" => {
                if self.client.remote.is_none() {
                    return Err("not connected to a server".into());
                }
                self.leave_server(None)?;
                ok()
            }
            "save" | "save-all" => {
                self.require_world()?;
                // `save-all` is like `save-all flush`: it also rewrites chunks
                // that were not modified (the save round-trip harness compares
                // them with vanilla's files).
                let all = words[0] == "save-all";
                // Single player's world is its server thread's.
                if let Some(server) = &self.client.integrated {
                    server
                        .ask(|reply| OwnerRequest::Control(ControlRequest::Save { all, reply }))
                        .ok_or(NO_ANSWER)??;
                } else {
                    return Err("save requires a single-player world".into());
                }
                ok()
            }
            "dragon-hurt" => {
                // Test hook: damage the dragon's head as the player would.
                let amount: f32 = arg(1)?.parse()?;
                let server = self
                    .client
                    .integrated
                    .as_ref()
                    .ok_or("dragon-hurt requires a single-player world")?;
                server
                    .ask(|reply| {
                        OwnerRequest::Control(ControlRequest::DragonHurt { amount, reply })
                    })
                    .ok_or(NO_ANSWER)??;
                ok()
            }
            "quit" => {
                self.begin_closing_save(true)?;
                ok()
            }
            other => match self.station_command(&words)? {
                Some(value) => Ok(Some(value)),
                None => Err(format!("unknown command {other}; try help").into()),
            },
        }
    }

    pub(in crate::game::client_side) fn require_world(&self) -> Result<()> {
        if self.client.title.0.is_some() {
            return Err("no world open; use: open WORLD [SEED] [creative]".into());
        }
        Ok(())
    }

    /// Release modifier, physical-key and debug-key state for the up half of a socket `key … tap`.
    #[inline(always)]
    fn finish_control_tap_release(&mut self, physical: KeyCode) {
        self.press_modifier_key(physical, false);
        let tap_menu = self.context_menu_key_active();
        if !tap_menu
            || !self
                .client
                .screen_state
                .0
                .context_menu_input
                .holds_other_source(Source::Agent, physical)
        {
            self.client.physical_keys.remove(&physical);
        }
        if tap_menu
            && (self.context_menu_open() || self.context_menu_owns_key(Source::Agent, physical))
        {
            self.debug_key_release_cleanup(physical);
        } else {
            self.debug_key(physical, sunrise_client::debug_keys::KeyAction::Release);
        }
    }

    /// Release any untagged keyboard-standin holds on non-toggle keys bound to `physical`.
    pub(in crate::game::client_side) fn release_control_keyboard_standin(
        &mut self,
        physical: KeyCode,
    ) {
        for key in self
            .client
            .settings
            .keys
            .translate_all(physical)
            .into_iter()
            .flatten()
        {
            self.client.physical_keys.remove(&key);
            if !matches!(key, KeyCode::ShiftLeft | KeyCode::ControlLeft)
                && self.client.keys.held_by(&key, Source::Keyboard)
            {
                self.release_device_key(&key, Source::Keyboard);
            }
        }
    }

    pub(in crate::game::client_side) fn screen_name(&self) -> &'static str {
        if self.client.screen == Screen::Win {
            return "win";
        }
        if self.client.title.0.is_some() {
            return "title";
        }
        match self.client.screen {
            Screen::Playing => "playing",
            Screen::GameModeSwitcher => "gamemode-switcher",
            Screen::Paused => "paused",
            Screen::Options => "options",
            Screen::Inventory => "inventory",
            Screen::Creative => "creative",
            Screen::Dead => "dead",
            Screen::Chat => "chat",
            Screen::Stats => "stats",
            Screen::Advancements => "advancements",
            Screen::WorldOptions => "world-options",
            Screen::Friends => "friends",
            Screen::Social => "social",
            Screen::ServerLinks => "server-links",
            Screen::ConfirmLink => "confirm-link",
            Screen::DemoIntro => "demo-intro",
            Screen::DialogWarning => "dialog-warning",
            Screen::CommandBlock => "command-block",
            Screen::BlockEditor => "block-editor",
            Screen::Win => "win",
            Screen::DebugOptions => "debug-options",
            Screen::Mod => "mod",
            Screen::HudEditor => "hud-editor",
        }
    }

    /// The hurts and melee attacks the local player logged since the last call
    /// (`PlayerMotion::hurt_log`, `attack_log`), taken out.
    pub(in crate::game) fn client_control_hurt_log(&mut self) -> Value {
        fn drain(
            name: &str,
            hurt_log: &mut Vec<sunrise_sim::health::HurtRecord>,
            attack_log: &mut Vec<sunrise_sim::health::AttackRecord>,
        ) -> Value {
            let hurts: Vec<Value> = hurt_log
                .drain(..)
                .map(|r| {
                    json!({
                        "t": r.game_time, "source": r.source, "amount": r.amount,
                        "blocked": r.blocked, "taken": r.taken, "hurt": r.hurt,
                        "armor": r.armor, "toughness": r.toughness, "health": r.health,
                        "fire_ticks": r.fire_ticks, "knockback": r.knockback,
                    })
                })
                .collect();
            let attacks: Vec<Value> = attack_log
                .drain(..)
                .map(|r| {
                    json!({
                        "t": r.game_time, "target": r.target, "id": r.id, "killed": r.killed, "amount": r.amount,
                        "scale": r.scale, "crit": r.crit, "landed": r.landed, "dealt": r.dealt,
                    })
                })
                .collect();
            json!({ "name": name, "hurts": hurts, "attacks": attacks })
        }
        let mut players = Vec::new();
        if let Some(p) = self.common.player.as_mut() {
            players.push(drain("host", &mut p.hurt_log, &mut p.attack_log));
        }
        json!({ "ok": true, "players": players })
    }

    pub(in crate::game::client_side) fn control_status(&self) -> Value {
        let player = self.common.player.as_ref().map(|p| {
            json!({
                "position": [p.position.x, p.position.y, p.position.z],
                "velocity": [p.velocity.x, p.velocity.y, p.velocity.z],
                "on_ground": p.ground.on_ground,
                "health": p.health.health(),
                // The maximum the HUD's hearts last read (its cached value).
                "max_health": p
                    .attributes
                    .lookup(p.attributes.key("minecraft:max_health").as_ref())
                    .and_then(|h| p.attributes.instance(&h).ok())
                    .map(|i| i.cached_value()),
                "food": p.health.food.food,
            })
        });
        let target = self.pick().map(|hit| {
            let state = self.common.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
            json!({ "pos": hit.pos, "block": &*self.res.facts().get(state).name })
        });
        let renderer = self.client.renderer.0.as_ref().map(|r| {
            let [arena_bytes, section_vertex_bytes] = r.section_vertex_bytes();
            // Device memory by the backend's allocator (Vulkan; absent elsewhere).
            let memory = r.device.generate_allocator_report();
            json!({
                "gpu": r.info.name,
                "vendor": vendor_name(r.info.vendor),
                "driver": format!("{} {}", r.info.driver, r.info.driver_info).trim(),
                "device_type": match r.info.device_type {
                    wgpu::DeviceType::IntegratedGpu => "integrated",
                    wgpu::DeviceType::DiscreteGpu => "discrete",
                    wgpu::DeviceType::VirtualGpu => "virtual",
                    wgpu::DeviceType::Cpu => "cpu",
                    wgpu::DeviceType::Other => "other",
                },
                "backend": format!("{:?}", r.info.backend),
                "size": [r.config.width, r.config.height],
                "present_mode": format!("{:?}", r.config.present_mode),
                "drawn_sections": r.drawn_sections,
                "section_draws": r.section_draws,
                "section_draw_calls": r.section_draw_calls,
                "culled_sections": r.culled_sections(),
                "gpu_culling": r.gpu_culling(),
                "gpu_culling_mode": self.client.settings.sunrise.gpu_culling.name(),
                "gpu_culling_auto": self.client.gpu_culling_auto.state(),
                // The world's resolution scale in percent, Dynamic Resolution's where it is on.
                "resolution_scale": super::frame_autotune::world_scale(
                    &self.client.settings.sunrise,
                    &self.client.dynamic_resolution
                ),
                "dynamic_resolution": self.client.settings.sunrise.dynamic_resolution,
                "gpu_frame": r.gpu_frame(),
                "gpu_fading": r.gpu_fading(),
                "gpu_culling_offer": format!("{:?}", r.gpu_culling_offer()),
                "gpu_occluding": r.gpu_occluding(),
                "stats_lag": r.stats_lag(),
                "uploaded_bytes": r.uploaded_bytes,
                "resident_sections": r.resident_sections(),
                "arena_bytes": arena_bytes,
                "section_vertex_bytes": section_vertex_bytes,
                "packed_layout": r.packed_layout(),
                "vertex_bytes_split": r.vertex_bytes_split(),
                "gpu_allocated_bytes": memory.as_ref().map(|m| m.total_allocated_bytes),
                "gpu_reserved_bytes": memory.as_ref().map(|m| m.total_reserved_bytes),
            })
        });
        // The keyboard as the OS reports it (screens read Shift and Ctrl here).
        let mut physical_keys: Vec<_> = self
            .client
            .physical_keys
            .iter()
            .map(|k| format!("{k:?}"))
            .collect();
        physical_keys.sort();
        let lod = json!({
            "distance": self.client.settings.sunrise.lod_distance,
            "effective_distance": self.lod_distance(),
            "terrain_distance": self.terrain_distance(),
            "source": if self.client.lod.0.is_none() { "none" } else if self.client.integrated.is_some() { "integrated" } else if self.server_far_terrain().is_some() { "server" } else if self.client.remote.is_some() { "server-terrain" } else { "local" },
            "source_attempted": self.client.lod_source_attempted,
            "active_tiles": self.client.lod.0.as_ref().map_or(0, |lod| lod.stats().0),
            "cached_tiles": self.client.lod.0.as_ref().map_or(0, |lod| lod.stats().1),
            "far_detail": self.client.settings.sunrise.far_detail.name(),
            "resident_quads": self.client.renderer.0.as_ref().map_or(0, |r| r.lod_quads()),
            "levels": self.client.renderer.0.as_ref().map_or_else(Vec::new, |r| r.lod_levels()),
            "pending_observations": self.client.lod_observe.len(),
            "refined_samples": self.client.lod.0.as_ref().map_or(0, |lod| lod.refined_samples()),
        });
        // The other players' cracks this client draws (LevelRenderer.destroyingBlocks).
        let destroying: Vec<_> = self
            .client
            .destroying
            .iter()
            .map(|(id, crack)| json!({ "id": id, "pos": crack.pos, "stage": crack.stage }))
            .collect();
        let mut status = json!({
            "screen": self.screen_name(),
            "debug": {
                "overlay": self.client.debug,
                "hitboxes": self.debug_entry("entity_hitboxes"),
                "chunk_borders": self.debug_entry("chunk_borders"),
                "pause_menu": self.client.pause_menu,
                "advanced_tooltips": self.client.settings.file.bool("advancedItemTooltips"),
                "pause_on_lost_focus": self.client.settings.file.bool("pauseOnLostFocus"),
                "clipboard": self.client.clipboard.0.copied(),
            },
            "dimension": self.dim.id(),
            "creative": self.creative,
            "player": player,
            "yaw": self.yaw,
            "pitch": self.pitch,
            "target": target,
            "selected": self.selected + 1,
            "held": self.held().map(|s| json!({ "item": &*s.item, "count": s.count })),
            "offhand": self.offhand.as_ref().map(|s| json!({ "item": &*s.item, "count": s.count, "damage": s.damage })),
            "keys": self.client.keys.iter().map(|k| format!("{k:?}")).collect::<Vec<_>>(),
            // The keyboard as the OS reports it (Shift and Ctrl in screens).
            "fps": self.client.fps,
            "frames": self.client.frames,
            "ticks": self.ticks,
            "day_time": self.world.day_time,
            "music": self.client.music.current(&self.client.audio.0.engine),
            "sounds": self.client.audio.0.census(),
            "tridents": self.tridents.len(),
            "rockets": self.rockets.len(),
            // A client of a server (single player's server thread) knows the
            // carts it was sent.
            "carts": self.remote_carts().unwrap_or_else(|| self.carts.iter().map(|c| serde_json::json!({"kind": c.kind.id(), "position": [c.position.x, c.position.y, c.position.z], "velocity": [c.velocity.x, c.velocity.y, c.velocity.z]})).collect::<Vec<_>>()),
            "riding": self.riding,
            "using": self.using,
            "render_distance": self.client.streaming.radius,
            "chunks": self.world.chunks.len(),
            "mobs": self.mobs.len(),
            "block_entities": self.world.block_entities.len(),
            "pending_chunks": self.client.streaming.pending() + self.client.pending_uploads.0.len() + self.remote_backlog(),
            "generated_chunks": self.client.streaming.generated_total,
            "meshed_sections": self.client.streaming.meshed_total,
            "streaming": self.client.streaming.stages().into_iter().map(|(k, v)| (k.to_string(), v.into())).collect::<serde_json::Map<_, _>>(),
            "dirty_light": self.world.dirty_light.len(),
            "renderer": renderer,
            "display": self.client.window.0.as_deref().and_then(sunrise_client::monitor::of).map(|m| json!({
                "width": m.size().width,
                "height": m.size().height,
                "scale": m.scale_factor(),
                "refresh_hz": m.refresh_rate_millihertz().map(|r| r.div_ceil(1000)),
            })),
            "uptime_seconds": self.client.started.elapsed().as_secs_f64(),
            "server": self.remote_status(),
            // The names the name tags show now, seen from the eyes.
            "name_tags": self
                .eye_position()
                .map(|e| self.shown_name_tags(glam::DVec3::from_array(e))),
        });
        let (fishing, fight) = self
            .client
            .integrated
            .as_ref()
            .and_then(|server| {
                server.ask(|reply| OwnerRequest::Control(ControlRequest::StatusExtras(reply)))
            })
            .and_then(std::result::Result::ok)
            .unwrap_or_else(|| {
                (
                    self.common.hook_status(),
                    json!({
                        "dragon": self.dragon.as_ref().map(|d| json!({
                            "id": d.id,
                            "position": [d.position.x, d.position.y, d.position.z],
                            "health": d.health,
                            "phase": d.phase.id(),
                            "death_time": d.death_time,
                            "strafing": Value::Null,
                        })),
                        "players": 0,
                        "crystals": self.crystals.len(),
                        "crystal_list": self.crystals.iter().map(|c| json!({
                            "id": c.id,
                            "position": [c.position.x, c.position.y, c.position.z],
                        })).collect::<Vec<_>>(),
                        "alive_crystals": self.crystals.len(),
                        "killed": false,
                        "exit_portal": Value::Null,
                        "respawn": Value::Null,
                    }),
                )
            });
        status["fishing"] = json!(fishing);
        status["fight"] = fight;
        status["lod"] = lod;
        // Parallel Tick: the setting, whether the next tick shares its work, and Auto's state.
        let (setting, on, auto) = sunrise_sim::parallel::status();
        status["parallel_tick"] = json!({ "setting": setting, "on": on, "auto": auto });
        let (target, sampled) = self
            .client
            .lod
            .0
            .as_ref()
            .map_or((0, 0), |lod| lod.progress());
        status["lod"]["target_tiles"] = target.into();
        status["lod"]["sampled_target_tiles"] = sampled.into();
        let (kept, kept_bytes) = self.client.lod.0.as_ref().map_or((0, 0), |lod| lod.kept());
        status["lod"]["kept_tiles"] = kept.into();
        status["lod"]["kept_mb"] = (kept_bytes >> 20).into();
        let (wanted, refined) = self
            .client
            .lod
            .0
            .as_ref()
            .map_or((0, 0), |lod| lod.detail_progress());
        status["lod"]["detail_target_columns"] = wanted.into();
        status["lod"]["refined_target_columns"] = refined.into();
        status["pending_uploads"] = self.client.pending_uploads.0.len().into();
        status["destroying"] = json!(destroying);
        status["physical_keys"] = json!(physical_keys);
        status["heard"] = json!(self.client.audio.0.heard);
        status["stopped_sounds"] = json!(self.client.audio.0.stopped);
        // Boats, minecarts, armor stands, item frames and paintings.
        status["objects"] = json!(self.objects_status());
        status["lightning"] = json!({"bolts": self.bolts.len(), "struck": self.client.lightning_seen[0],
            "flashes": self.client.lightning_seen[1], "flash": self.flash});
        // Primed TNT, falling blocks, projectiles, tridents and end crystals.
        status["projectiles"] = json!(self.client_projectiles_status());
        let mut fangs_vertices = Vec::new();
        self.fangs_geometry(glam::DVec3::ZERO, 0.5, &mut fangs_vertices);
        status["fangs_vertices"] = json!(fangs_vertices.len());
        status["maps"] = json!(self.maps_status());
        // The particles made so far, by type.
        status["particles"] = json!(self.client.particles.0.made());
        // The pickup particles made so far, by what was taken.
        status["pickups"] = json!(self.client.pickups.0.made());
        // The world border this side knows of.
        status["border"] = json!({ "center": self.border.center(), "size": self.border.size() });
        // The scoreboard sidebar the HUD shows.
        status["sidebar"] = self.sidebar_status();
        // Who caused the input, and whether a repro recording runs.
        let mut input = self.client.input_trace.0.summary();
        self.client
            .repro
            .0
            .fill_summary(&mut input, self.client.input_trace.0.events());
        status["input"] = input;
        status
    }
}

/// PCI vendor IDs of the GPU makers players have (Apple's is wgpu's Metal value).
pub(in crate::game) fn vendor_name(id: u32) -> String {
    match id {
        0x10de => "nvidia".into(),
        0x1002 | 0x1022 => "amd".into(),
        0x8086 => "intel".into(),
        0x106b => "apple".into(),
        0x5143 => "qualcomm".into(),
        0x13b5 => "arm".into(),
        0x10005 => "mesa".into(),
        0 => String::new(),
        other => format!("{other:#06x}"),
    }
}

/// The key a `key <name> tap|up` command releases, whatever its handler does.
/// A `key KEY down|up|tap|repeat` line's key and action.
pub(in crate::game) fn socket_key(line: &str) -> Option<(KeyCode, &str)> {
    let mut words = line.split_whitespace();
    match (words.next(), words.next(), words.next()) {
        (Some("key"), Some(key), Some(action @ ("down" | "up" | "tap" | "repeat"))) => {
            Some((key_code(key)?, action))
        }
        _ => None,
    }
}

pub(in crate::game) fn released_modifier(line: &str) -> Option<KeyCode> {
    let mut words = line.split_whitespace();
    match (words.next(), words.next(), words.next()) {
        (Some("key"), Some(key), Some("tap" | "up")) => key_code(key),
        _ => None,
    }
}

pub(in crate::game) fn key_code(name: &str) -> Option<KeyCode> {
    let name = name.to_ascii_lowercase();
    // The game's own names (`key.keyboard.w`), which a recording writes.
    if let Some(code) = sunrise_client::keybinds::key_code(&name) {
        return Some(code);
    }
    let letters = [
        KeyCode::KeyA,
        KeyCode::KeyB,
        KeyCode::KeyC,
        KeyCode::KeyD,
        KeyCode::KeyE,
        KeyCode::KeyF,
        KeyCode::KeyG,
        KeyCode::KeyH,
        KeyCode::KeyI,
        KeyCode::KeyJ,
        KeyCode::KeyK,
        KeyCode::KeyL,
        KeyCode::KeyM,
        KeyCode::KeyN,
        KeyCode::KeyO,
        KeyCode::KeyP,
        KeyCode::KeyQ,
        KeyCode::KeyR,
        KeyCode::KeyS,
        KeyCode::KeyT,
        KeyCode::KeyU,
        KeyCode::KeyV,
        KeyCode::KeyW,
        KeyCode::KeyX,
        KeyCode::KeyY,
        KeyCode::KeyZ,
    ];
    let digits = [
        KeyCode::Digit0,
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ];
    if let [c] = name.as_bytes() {
        return match c {
            b'a'..=b'z' => Some(letters[usize::from(c - b'a')]),
            b'0'..=b'9' => Some(digits[usize::from(c - b'0')]),
            _ => None,
        };
    }
    Some(match name.as_str() {
        "space" => KeyCode::Space,
        "shift" | "lshift" => KeyCode::ShiftLeft,
        "ctrl" | "control" | "lctrl" => KeyCode::ControlLeft,
        // The edit modifier on macOS (Cmd+A selects all there).
        "super" | "cmd" | "meta" => KeyCode::SuperLeft,
        "esc" | "escape" => KeyCode::Escape,
        "tab" => KeyCode::Tab,
        "enter" | "return" => KeyCode::Enter,
        "f1" => KeyCode::F1,
        "f2" => KeyCode::F2,
        "f3" => KeyCode::F3,
        "f4" => KeyCode::F4,
        "f5" => KeyCode::F5,
        "f6" => KeyCode::F6,
        "f11" => KeyCode::F11,
        "slash" | "/" => KeyCode::Slash,
        "backspace" => KeyCode::Backspace,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "left" => KeyCode::ArrowLeft,
        "right" => KeyCode::ArrowRight,
        "up" => KeyCode::ArrowUp,
        "down" => KeyCode::ArrowDown,
        _ => return None,
    })
}

/// The (times, detail) switches `gpu-passes [on|off|detail]` leaves, given the current ones.
pub(in crate::game) fn gpu_pass_switches(
    word: Option<&str>,
    now: (bool, bool),
) -> Result<(bool, bool)> {
    match word {
        None => Ok(now),
        Some("on") => Ok((true, false)),
        Some("detail") => Ok((true, true)),
        Some("off") => Ok((false, false)),
        Some(other) => Err(format!("unknown gpu pass mode {other}").into()),
    }
}

/// The `gpu-passes` reply: the switches and what detail can show on this device.
pub(in crate::game) fn gpu_passes_reply(
    passes: bool,
    detail: bool,
    support: Option<pack_timing::DetailSupport>,
) -> Value {
    let shown = support.map(pack_timing::support_json);
    let field = |key: &str, none: Value| shown.as_ref().map_or(none, |s| s[key].clone());
    json!({
        "ok": true,
        "gpu_passes": passes,
        "detail": detail,
        "parts_mode": field("parts_mode", Value::Null),
        "counters": field("counters", json!(false)),
        "fragment_unit": field("fragment_unit", Value::Null),
    })
}

/// The title's own screens (not one opened over it, as Options is) take the
/// text: the world name, the seed, a server's address.
pub(in crate::game::client_side) fn title_owns_text(screen: &Screen, title_up: bool) -> bool {
    title_up && *screen == Screen::Playing
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::memory_common::MemoryCommonGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;

    #[test]
    fn the_title_takes_text_only_on_its_own_screens() {
        assert!(title_owns_text(&Screen::Playing, true));
        assert!(!title_owns_text(&Screen::Playing, false));
        assert!(!title_owns_text(&Screen::Options, true));
        assert!(!title_owns_text(&Screen::Chat, true));
    }

    #[test]
    fn hello_names_the_protocol_the_release_and_the_command_groups() {
        let reply = hello();
        assert_eq!(reply["release"], sunrise_core::release::VERSION);
        let protocol = reply["protocol"].as_str().unwrap();
        let parts: Vec<_> = protocol.split('.').collect();
        assert!(
            parts.len() == 3 && parts.iter().all(|p| p.parse::<u32>().is_ok()),
            "{protocol}"
        );
        let features = reply["features"].as_array().unwrap();
        assert!(!features.is_empty());
        let mut names: Vec<_> = features.iter().filter_map(Value::as_str).collect();
        assert_eq!(names.len(), features.len());
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), features.len(), "a group is named twice");
    }

    #[test]
    fn gpu_passes_detail_replies_detail_and_on_turns_it_off_again() {
        let support = pack_timing::DetailSupport {
            parts: pack_timing::PartsMode::SeparatePasses,
            encoder_stamps: false,
            counters: true,
            fragment_unit: Some(pack_timing::FragmentUnit::Quad),
        };
        let (passes, detail) = gpu_pass_switches(Some("detail"), (false, false)).unwrap();
        let reply = gpu_passes_reply(passes, detail, Some(support));
        assert_eq!(reply["detail"], true);
        assert_eq!(reply["gpu_passes"], true);
        assert_eq!(reply["parts_mode"], "separate_passes");
        assert_eq!(reply["counters"], true);
        assert_eq!(reply["fragment_unit"], "quad");
        let (passes, detail) = gpu_pass_switches(Some("on"), (passes, detail)).unwrap();
        let reply = gpu_passes_reply(passes, detail, None);
        assert_eq!(
            (reply["gpu_passes"].as_bool(), reply["detail"].as_bool()),
            (Some(true), Some(false))
        );
        assert_eq!(
            (reply["parts_mode"].is_null(), reply["counters"].as_bool()),
            (true, Some(false))
        );
        assert_eq!(
            gpu_pass_switches(Some("off"), (true, true)).unwrap(),
            (false, false)
        );
        assert_eq!(gpu_pass_switches(None, (true, true)).unwrap(), (true, true));
        assert!(gpu_pass_switches(Some("nope"), (false, false)).is_err());
    }

    #[test]
    fn taps_and_ups_always_release_their_key() {
        for line in ["key shift tap", "key shift up", "key  Shift  up"] {
            assert_eq!(released_modifier(line), Some(KeyCode::ShiftLeft), "{line}");
        }
        for line in [
            "key shift down",
            "key shift repeat",
            "key shift",
            "keys shift up",
            "key nope up",
        ] {
            assert_eq!(released_modifier(line), None, "{line}");
        }
    }

    #[test]
    fn a_benchmark_profile_only_goes_with_a_benchmark() {
        let parse = |args: &[&str]| {
            super::super::Options::parse(args.iter().map(|a| a.to_string())).map(|o| o.unwrap())
        };
        assert!(
            parse(&["--benchmark", "village", "--profile"])
                .unwrap()
                .benchmark_profile
        );
        assert!(
            !parse(&["--benchmark", "village"])
                .unwrap()
                .benchmark_profile
        );
        assert!(parse(&["--profile"]).is_err());
    }

    #[test]
    fn a_benchmark_set_only_goes_with_the_trip() {
        let parse = |args: &[&str]| {
            super::super::Options::parse(args.iter().map(|a| a.to_string())).map(|o| o.unwrap())
        };
        let set = parse(&["--benchmark", "travel", "--benchmark-set", "gpu-culling=on"]).unwrap();
        assert_eq!(set.benchmark_set, Some(("gpu-culling".into(), "on".into())));
        let before = parse(&["--benchmark-set", "gpu-culling=on", "--benchmark", "travel"]);
        assert!(before.is_ok());
        for args in [
            &[
                "--benchmark",
                "standard",
                "--benchmark-set",
                "gpu-culling=on",
            ][..],
            &["--benchmark-set", "gpu-culling=on"],
            &[
                "--benchmark",
                "travel",
                "--benchmark-set",
                "gpu-culling=auto",
            ],
        ] {
            assert!(parse(args).is_err(), "{args:?}");
        }
    }
}
