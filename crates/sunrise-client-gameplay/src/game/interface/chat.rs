//! Chat, the command registries and the per-world state behind commands
//! (game rules, the world clock, difficulty), bound to the game.

#[cfg(all(test, feature = "client"))]
#[path = "chat/submit_order_tests.rs"]
mod submit_order_tests;

use super::Screen;
use std::path::Path;
use sunrise_client::{
    chat::{
        Chat, ChatAction, ChatScreen, ChatSource, Commands, QUEUE_EXPAND_ID, SYSTEM_INDICATOR,
        normalize_chat_message,
    },
    hud::HudBatch,
    screen_draw::Measure,
};
use sunrise_command::{ArgType, Dispatcher, MOD_COMMAND};
use sunrise_game_data::text::{ClickEvent, Style, Text};
use sunrise_gameplay_common::game::chat_common::{block_properties, command_export, resource_ids};
use sunrise_gameplay_common::game::native_server;

/// The tree the chat parses and completes with: `Remote::chat_tree`, which is
/// a server's, once it has sent one (ClientPacketListener.commands), else the
/// game's own, plus the commands native mods own (`chat_tree_for`).
pub fn chat_tree(
    remote: &Option<crate::game::client_side::remote_client::Remote>,
) -> &sunrise_command::Dispatcher {
    match remote.as_ref() {
        Some(server) => &server.chat_tree,
        None => sunrise_command::vanilla::game_dispatcher(),
    }
}
/// The arguments of a typed `/profiler` the client answers itself: only when the chat tree's `profiler` is Sunrise's, else the server or mod that owns the name runs it.
pub(in crate::game::client_side) fn sunrise_profiler_args<'a>(
    message: &'a str,
    server_tree: Option<&sunrise_command::Dispatcher>,
) -> Option<&'a str> {
    let args = message
        .strip_prefix('/')
        .and_then(super::profile_capture::profiler_args)?;
    server_tree
        .is_none_or(|tree| tree.has_sunrise_profiler())
        .then_some(args)
}
use winit::keyboard::KeyCode;

/// `base` with extra client-local commands added: each a literal taking the
/// rest of the line as typed (`Dispatcher::with_mod_commands`), so the chat
/// box colours and completes them as it does the game's own.
pub(in crate::game::client_side) fn with_native_commands(
    base: &sunrise_command::Dispatcher,
    names: &[&str],
) -> sunrise_command::Dispatcher {
    // A name the tree already has keeps that command (colour, completion).
    base.with_mod_commands(
        names
            .iter()
            .copied()
            .filter(|name| base.find(name).is_none()),
    )
}

/// Add Studio's local command grammar to the suggestion tree. Commands are
/// still dispatched by `studio_command`; this tree supplies coloring,
/// completion, and Tab acceptance in chat.
pub(in crate::game::client_side) fn with_studio_commands(base: &Dispatcher) -> Dispatcher {
    fn literal(d: &mut Dispatcher, parent: usize, name: &str) -> usize {
        let node = d.literal(parent, name);
        d.executes(node, MOD_COMMAND);
        node
    }
    fn argument(d: &mut Dispatcher, parent: usize, name: &str, ty: ArgType) -> usize {
        let node = d.argument(parent, name, ty);
        d.executes(node, MOD_COMMAND);
        node
    }

    let mut d = base.clone();
    let root = d.find("studio").unwrap_or_else(|| d.literal(0, "studio"));
    d.executes(root, MOD_COMMAND);

    let record = literal(&mut d, root, "record");
    let start = literal(&mut d, record, "start");
    argument(&mut d, start, "name", ArgType::Word);
    literal(&mut d, record, "stop");
    literal(&mut d, record, "status");

    let replay = literal(&mut d, root, "replay");
    let open = literal(&mut d, replay, "open");
    argument(&mut d, open, "path", ArgType::Greedy);
    literal(&mut d, replay, "pause");
    literal(&mut d, replay, "resume");
    let speed = literal(&mut d, replay, "speed");
    argument(
        &mut d,
        speed,
        "multiplier",
        ArgType::Double {
            min: 0.01,
            max: 20.0,
        },
    );
    let seek = literal(&mut d, replay, "seek");
    argument(
        &mut d,
        seek,
        "milliseconds",
        ArgType::Integer {
            min: 0,
            max: i32::MAX,
        },
    );
    literal(&mut d, replay, "status");
    literal(&mut d, replay, "close");

    let freecam = literal(&mut d, root, "freecam");
    argument(
        &mut d,
        freecam,
        "state",
        ArgType::StrEnum(&["on", "off"], "argument.enum.invalid"),
    );

    let keyframe = literal(&mut d, root, "keyframe");
    literal(&mut d, keyframe, "add");
    let remove = literal(&mut d, keyframe, "remove");
    argument(
        &mut d,
        remove,
        "index",
        ArgType::Integer {
            min: 0,
            max: i32::MAX,
        },
    );
    literal(&mut d, keyframe, "clear");

    let path = literal(&mut d, root, "path");
    argument(
        &mut d,
        path,
        "state",
        ArgType::StrEnum(&["lock", "unlock"], "argument.enum.invalid"),
    );
    d
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// The tree the chat box colours, completes and parses with for the
    /// world's connection: `base`, the tree the server sent, or the game's own
    /// before it does. Built where that base changes — once per connection —
    /// rather than at each keystroke, and asking the mods then, not per key.
    pub(in crate::game) fn chat_tree_for(
        &mut self,
        base: Option<&sunrise_command::Dispatcher>,
    ) -> sunrise_command::Dispatcher {
        let base = match base {
            Some(tree) => tree,
            None => sunrise_command::vanilla::game_dispatcher(),
        };
        let mut registrations = Vec::new();
        self.native_register_client_commands(&mut registrations);
        let mut usages = Vec::with_capacity(registrations.len());
        let mut executors = Vec::with_capacity(registrations.len());
        for registration in registrations {
            let owner = registration.mod_id;
            let (command, call) =
                mod_api::__private::split_client_command_registration(registration.answer);
            let index = mod_api::__private::mod_index(owner)
                .expect("client command registrations come from installed listeners");
            let executor = super::native_server_client::RegisteredClientExecutor::new(
                owner,
                index,
                &command.usage,
                call,
            );
            usages.push(command);
            executors.push(Some(executor));
        }
        let arguments = native_server::command_arguments();
        let mut slots = std::collections::HashMap::new();
        let (tree, _, problems) =
            native_server::client_command_tree(&usages, &arguments, Some(&mut slots));
        for problem in problems {
            log::warn!("Sunrise: a client command was left out: {problem}");
        }
        let mut client_dispatcher = Dispatcher::new();
        native_server::register_command_nodes(
            &mut client_dispatcher,
            &tree,
            MOD_COMMAND,
            &arguments,
            Some(&slots),
        );
        let retained: std::collections::HashSet<_> = slots.values().copied().collect();
        for (slot, executor) in executors.iter_mut().enumerate() {
            if !retained.contains(&slot) {
                *executor = None;
            }
        }
        self.client
            .native_client_commands
            .install(client_dispatcher, executors);
        let mut tree_with_mods = base.clone_with_unmerged_redirects();
        native_server::register_dispatcher(&mut tree_with_mods, &tree, MOD_COMMAND, &arguments);
        with_studio_commands(&with_native_commands(&tree_with_mods, &["studio"])).with_profiler()
    }
}

/// `ChatComponent.Draft`: the text the chat box kept, and whether it was a command.
pub struct ChatDraft {
    pub text: String,
    pub command: bool,
}
/// `ChatScreen.ExitReason`: why the chat screen went away.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChatExit {
    /// Another screen replaced it (death, a container, a disconnect).
    Interrupted,
    /// The player closed it (Escape).
    Intentional,
    /// The message was sent or transferred into the replacement chat screen.
    Done,
}
/// `ChatScreen.shouldDiscardDraft`.
pub(in crate::game::client_side) fn should_discard_draft(
    reason: ChatExit,
    save_chat_drafts: bool,
) -> bool {
    reason != ChatExit::Interrupted && (reason != ChatExit::Intentional || !save_chat_drafts)
}
/// Client-only chat, HUD visibility, screenshot, options, and death-screen state.
#[derive(Default)]
pub struct ChatUiState {
    pub chat: Chat,
    pub chat_screen: Option<ChatScreen>,
    /// `ChatComponent.latestDraft`.
    pub chat_draft: Option<ChatDraft>,
    /// When the chat screen opened (its input took the focus: the cursor blink).
    pub chat_opened: Option<std::time::Instant>,
    /// The chat screen is `InBedChatScreen`: the sleeper's, with a Leave Bed button.
    pub in_bed_chat: bool,
    /// Effective HUD visibility, which native mods can change.
    pub hide_gui: bool,
    /// The player's F1 preference, independent of native HUD mode changes.
    pub player_hides_gui: bool,
    /// Native HUD mode: keep hands visible while the HUD is hidden.
    pub show_hand_when_hidden: bool,
    /// What this frame's HUD shows that Sunrise's overlays keep clear of (`Game::hud_room`),
    /// set by `hud_layers` before anything draws.
    pub room: sunrise_client::hud_kit::room::Room,
    /// F2 screenshots not written yet: each reports in chat once its PNG is.
    pub screenshot_messages: Vec<std::path::PathBuf>,
    pub screenshot_writes: Vec<super::screenshot_writer::ScreenshotWrite>,
    /// The options screens.
    pub options: sunrise_client::options_ui::OptionsUi,
    /// Death screen cause and when the player died (buttons wait 20 ticks).
    pub death_text: Option<Text>,
    pub died_at: Option<std::time::Instant>,
    /// `DeathScreen.TitleConfirmScreen` and when it opened (its buttons wait
    /// 20 ticks, `setDelay`).
    pub death_confirm: Option<std::time::Instant>,
    /// `LivingEntity.deathTime` of the window's player: ticks since it died.
    pub death_time: u32,
}

impl ChatUiState {
    pub fn hud_mode(&self) -> mod_api::HudMode {
        use mod_api::HudMode;
        match (self.hide_gui, self.show_hand_when_hidden) {
            (false, _) => HudMode::AllVisible,
            (true, true) => HudMode::NoHud,
            (true, false) => HudMode::AllHidden,
        }
    }

    pub fn set_hud_mode(&mut self, mode: mod_api::HudMode) {
        use mod_api::HudMode;
        self.hide_gui = mode != HudMode::AllVisible;
        self.show_hand_when_hidden = mode == HudMode::NoHud;
    }

    pub(in crate::game::client_side) fn hand_hidden(&self) -> bool {
        self.hide_gui && !self.show_hand_when_hidden
    }

    /// The effective first-person hand gate, before per-hand mod cancellation.
    pub fn hands_drawn(
        &self,
        spectator: bool,
        sleeping: bool,
        first_person: bool,
        alive: bool,
        dead_screen: bool,
    ) -> bool {
        !spectator && !self.hand_hidden() && !sleeping && first_person && alive && !dead_screen
    }

    pub(in crate::game::client_side) fn toggle_hide_gui(&mut self) {
        use mod_api::HudMode;
        self.player_hides_gui = !self.player_hides_gui;
        self.set_hud_mode(if self.player_hides_gui {
            HudMode::AllHidden
        } else {
            HudMode::AllVisible
        });
    }

    /// The state a window starts with: the chat remembers the commands kept
    /// in the game folder (`ChatComponent`'s constructor).
    pub fn new() -> Self {
        let mut ui = Self::default();
        if let Ok(dir) = std::env::current_dir() {
            ui.chat.load_history(&dir);
        }
        ui
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod hud_mode_tests {
    use super::ChatUiState;
    use mod_api::HudMode;

    #[test]
    fn hud_modes_drive_the_effective_hud_and_hand_gate() {
        let mut ui = ChatUiState::default();
        for (mode, hidden, hand) in [
            (HudMode::AllVisible, false, true),
            (HudMode::NoHud, true, true),
            (HudMode::AllHidden, true, false),
        ] {
            ui.set_hud_mode(mode);
            assert_eq!(ui.hud_mode(), mode);
            // This is the same flag that every HUD gate and API 410 read.
            assert_eq!(ui.hide_gui, hidden);
            assert_eq!(ui.hands_drawn(false, false, true, true, false), hand);
            // No mode can bypass the other first-person hand restrictions.
            assert!(!ui.hands_drawn(true, false, true, true, false));
            assert!(!ui.hands_drawn(false, true, true, true, false));
            assert!(!ui.hands_drawn(false, false, false, true, false));
            assert!(!ui.hands_drawn(false, false, true, false, false));
            assert!(!ui.hands_drawn(false, false, true, true, true));
        }
    }

    #[test]
    fn vanilla_f1_remains_two_state_and_clears_the_hand_override() {
        let mut ui = ChatUiState::default();
        assert_eq!(ui.hud_mode(), HudMode::AllVisible);
        ui.toggle_hide_gui();
        assert_eq!(ui.hud_mode(), HudMode::AllHidden);
        assert!(ui.hand_hidden());
        ui.toggle_hide_gui();
        assert_eq!(ui.hud_mode(), HudMode::AllVisible);
        ui.set_hud_mode(HudMode::NoHud);
        ui.toggle_hide_gui();
        assert_eq!(ui.hud_mode(), HudMode::AllHidden);
        ui.toggle_hide_gui();
        assert_eq!(ui.hud_mode(), HudMode::AllVisible);
        // Existing hidden state also hides hands without a separate setting.
        let ui = ChatUiState {
            hide_gui: true,
            ..Default::default()
        };
        assert_eq!(ui.hud_mode(), HudMode::AllHidden);
        assert!(ui.hand_hidden());
    }
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    #[cfg(any(test, feature = "test-support"))]
    pub fn chat_ui_for_test(&self) -> &ChatUiState {
        &self.client.chat_ui
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn chat_ui_mut_for_test(&mut self) -> &mut ChatUiState {
        &mut self.client.chat_ui
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn chat_ui_and_font_for_test(&mut self) -> (&mut ChatUiState, &sunrise_client::font::Font) {
        (&mut self.client.chat_ui, &self.client.gui.atlas.font)
    }
    pub(in crate::game) fn client_sound_event_names(&self) -> Vec<String> {
        self.client.audio.0.event_names()
    }
    pub(in crate::game) fn client_sync_pack_tag_ids(&mut self) {}
    pub(in crate::game) fn client_sync_hub_clocks(&mut self) {}
    /// The caches `build_registries` and `join_server` read (`resource_ids`,
    /// `block_properties`, `command_export`, `builtin_registries`), filled on a helper
    /// thread while the title screen is up: the first world opened or server joined
    /// built them in its frame (20 ms of the Join click on an M4 Pro). Whoever needs one
    /// before it is done waits for it, as before.
    pub(in crate::game::client_side) fn preload_registries(&self) {
        if self.common.options.dedicated.is_some() {
            return;
        }
        let dirs = [
            self.common.options.resources.clone(),
            self.common.ui.vanilla_root.clone(),
        ];
        let registry = self.common.pack.assets.registry;
        let spawned = std::thread::Builder::new()
            .name("Registries preload".into())
            .spawn(move || {
                sunrise_threads::behind_server();
                sunrise_gameplay_common::game::villager_client::builtin_registries(&dirs[0]);
                resource_ids(&dirs[0]);
                block_properties(registry);
                for dir in &dirs {
                    if command_export(dir, true).is_some() {
                        break;
                    }
                }
            });
        if let Err(e) = spawned {
            log::warn!("Sunrise: registries preload: {e}");
        }
    }
    /// The block the player is looking at, for coordinate suggestions, and
    /// the entity, for entity arguments (ClientSuggestionProvider).
    /// `ClientSuggestionProvider.getRelevantCoordinates` and
    /// `getAbsoluteCoordinates` read `Minecraft.hitResult` only when it is a
    /// block: an entity in front of the block leaves the `~` defaults.
    pub(in crate::game::client_side) fn refresh_dynamic_registries(&mut self) {
        let hit = if self.pick_entity_hit().is_some() {
            None
        } else {
            self.pick()
        };
        self.common.ui.registries.target_block = hit.as_ref().map(|h| h.pos);
        self.common.ui.registries.target_point =
            hit.map(|h| h.point.map(sunrise_core::java_number::format_fixed2));
        self.common.ui.registries.selected_entities =
            self.crosshair_entity_uuid().into_iter().collect();
        // ClientPacketListener.getOnlinePlayerNames on a server.
        if let Some(names) = self.remote_online_names() {
            self.common.ui.registries.players = names;
        }
        // ClientPacketListener.levels(): the level keys of the login packet,
        // data-pack and mod dimensions included.
        if let Some(levels) = self
            .client
            .remote
            .as_ref()
            .map(|r| r.levels.clone())
            .filter(|l| !l.is_empty())
        {
            self.common.ui.registries.dimensions = levels;
        }
        self.common.ui.registries.custom_completions = self.remote_custom_completions();
        // The sender a mod's own argument completion answers for: this player, whose permission
        // level is `registries.permission`. A server completion sets it from the guest asking
        // (`guest_suggestions`); the console has none.
        self.common.ui.registries.sender = self
            .client
            .remote
            .as_ref()
            .filter(|remote| remote.logged_in && remote.profile != 0)
            .map(|remote| sunrise_game_data::registries::CommandSender {
                uuid: remote.profile,
                name: remote.profile_name.clone(),
            });
    }
    /// `ChatComponent.createScreen`: T or / opens the chat with the draft kept
    /// last time, when this method can restore it (a command draft only for
    /// /), else with its prefix.
    pub fn open_chat_method(&mut self, prefix: &str) {
        let command = prefix == "/";
        let draft = self
            .client
            .chat_ui
            .chat_draft
            .as_ref()
            .filter(|d| !command || d.command)
            .map(|d| d.text.clone());
        self.begin_chat_screen(draft.as_deref().unwrap_or(prefix), draft.is_some());
    }
    /// Open the chat box on `initial`; `is_draft` when that text is the saved
    /// draft (`ChatComponent.createScreen`), which draws grey italic until it
    /// is edited.
    pub fn begin_chat_screen(&mut self, initial: &str, is_draft: bool) -> bool {
        self.replace_chat_screen(initial, is_draft, ChatExit::Interrupted)
    }

    fn replace_chat_screen(&mut self, initial: &str, is_draft: bool, reason: ChatExit) -> bool {
        if self.common.player.is_none() {
            return false;
        }
        self.transition_screen(Screen::Chat, true, |game, accepted| {
            if accepted != Some(Screen::Chat) {
                game.remove_chat_screen(ChatExit::Interrupted);
                return;
            }
            game.remove_chat_screen(reason);
            game.refresh_dynamic_registries();
            let size = game
                .client
                .renderer
                .0
                .as_ref()
                .map_or([1280, 720], |r| [r.config.width, r.config.height]);
            let b = HudBatch::new(size, game.client.settings.gui_scale_inputs());
            let m = Measure(&game.client.gui.atlas.font);
            let c = Commands {
                dispatcher: chat_tree(&game.client.remote),
                registries: &game.common.ui.registries,
            };
            let mut screen = ChatScreen::new(
                &m,
                &c,
                initial,
                is_draft,
                game.client.chat_ui.chat.recent.len(),
                b.width as i32,
                b.height as i32,
                game.chat_abilities(),
            );
            screen.auto_suggestions = game.client.settings.auto_suggestions;
            game.client.chat_ui.chat_screen = Some(screen);
            game.client.chat_ui.chat_opened = Some(std::time::Instant::now());
            game.capture(false);
        });
        true
    }
    pub fn close_chat(&mut self, reason: ChatExit) {
        // InBedChatScreen.onClose: leaving the screen means getting up; the
        // screen goes once the player has woken (`tick_in_bed_chat`).
        if self.client.chat_ui.in_bed_chat && self.common.sleeping.is_some() {
            self.remote_stop_sleeping();
            return;
        }
        if self.client.screen == Screen::Chat {
            // `Gui.setScreen(null)`: Closing fires, no vetoable Opening does.
            self.dismiss_screen_with(|game| game.remove_chat_screen(reason));
            if self.client.screen == Screen::Playing {
                self.capture(true);
            }
        } else {
            self.remove_chat_screen(reason);
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_close_chat_intentionally(&mut self) {
        self.close_chat(ChatExit::Intentional);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_close_chat_done(&mut self) {
        self.close_chat(ChatExit::Done);
    }

    /// `ChatScreen.removed`, called only after the screen transition is accepted.
    pub fn remove_chat_screen(&mut self, reason: ChatExit) {
        if let Some(screen) = self.client.chat_ui.chat_screen.take() {
            self.keep_chat_draft(screen.value(), screen.is_draft(), reason);
            self.client.chat_ui.chat_opened = None;
            self.client.chat_ui.chat.reset_scroll();
        }
        self.client.chat_ui.in_bed_chat = false;
    }
    /// `ChatScreen.removed`: the text left in the box becomes the draft unless
    /// `shouldDiscardDraft` (closed on purpose without Save Chat Drafts, or
    /// sent); blank text discards the draft. A draft still shown unchanged
    /// stays as it is.
    fn keep_chat_draft(&mut self, text: &str, is_draft: bool, reason: ChatExit) {
        // During PvP & HUD's chat search the box holds the query: the draft is the text the
        // search put aside, whichever way the chat closed.
        let aside = self.client.pvp.chat_search_aside.take();
        let (text, is_draft) = aside.as_ref().map_or((text, is_draft), |(text, is_draft)| {
            (text.as_str(), *is_draft)
        });
        if should_discard_draft(reason, self.client.settings.file.bool("saveChatDrafts"))
            || text.trim().is_empty()
        {
            self.client.chat_ui.chat_draft = None;
        } else if !is_draft {
            self.client.chat_ui.chat_draft = Some(ChatDraft {
                text: text.to_owned(),
                command: text.starts_with('/'),
            });
        }
    }
    /// `Gui.tick`: a sleeper gets `InBedChatScreen`; when the player has
    /// woken (`onPlayerWokeUp`) it closes, or turns into the plain chat
    /// screen if something was typed.
    pub(in crate::game::client_side) fn tick_in_bed_chat(&mut self) {
        if self.client.screen != Screen::Chat {
            self.remove_chat_screen(ChatExit::Interrupted);
        }
        if self.client.chat_ui.in_bed_chat
            && self.common.sleeping.is_none()
            && !self.screen_over_playing()
        {
            // InBedChatScreen.onPlayerWokeUp: replace once, discarding the
            // outgoing draft only when non-draft typed text transfers into
            // ordinary chat; empty or untouched-draft wake dismisses via
            // `Gui.setScreen(null)` (Closing without a vetoable Opening).
            let typed = self.client.chat_ui.chat_screen.as_ref().and_then(|screen| {
                let (text, is_draft) = self
                    .client
                    .pvp
                    .chat_search_aside
                    .as_ref()
                    .map_or((screen.value(), screen.is_draft()), |(text, is_draft)| {
                        (text.as_str(), *is_draft)
                    });
                (!is_draft && !text.is_empty()).then(|| text.to_owned())
            });
            if let Some(text) = typed {
                self.replace_chat_screen(&text, false, ChatExit::Done);
                // A vetoed replacement keeps the outgoing instance and its draft; the
                // sleeping-to-awake transition has been offered once and is not repeated.
                self.client.chat_ui.in_bed_chat = false;
            } else {
                self.dismiss_screen_with(|game| game.remove_chat_screen(ChatExit::Interrupted));
                if self.client.screen == Screen::Playing {
                    self.capture(true);
                }
            }
        } else if self.common.sleeping.is_some()
            && self.client.screen == Screen::Playing
            && !self.screen_over_playing()
        {
            let draft = self
                .client
                .chat_ui
                .chat_draft
                .as_ref()
                .map(|draft| draft.text.clone());
            self.begin_chat_screen(draft.as_deref().unwrap_or(""), draft.is_some());
            self.client.chat_ui.in_bed_chat = self.client.screen == Screen::Chat;
        }
    }
    /// The Leave Bed button of `InBedChatScreen`: its bounds, laid out once
    /// per build (`ChatLayout`).
    pub(in crate::game::client_side) fn leave_bed_button(
        width: f32,
        height: f32,
    ) -> (i32, i32, i32, i32) {
        (width as i32 / 2 - 100, height as i32 - 40, 200, 20)
    }
    /// The Leave Bed button's draws. It registers as a button
    /// (`widgets::record`), so the pad's and the keyboard's focus reach it (the
    /// pad's starts on it) and `sunrise-ctl click-button` finds it.
    pub(in crate::game::client_side) fn leave_bed_draws(
        m: &Measure,
        mouse: (i32, i32),
        (x, y, w, h): (i32, i32, i32, i32),
    ) -> Vec<sunrise_client::screen_draw::Draw> {
        let label = Text::literal(sunrise_game_data::lang::get_or(
            "multiplayer.stopSleeping",
            "Leave Bed",
        ));
        sunrise_client::widgets::record(
            &label.plain(),
            [x as f32, y as f32, w as f32, h as f32],
            true,
        );
        let mut button = Vec::new();
        sunrise_client::screen_draw::button(
            &mut button,
            m,
            &label,
            x,
            y,
            w,
            h,
            true,
            sunrise_client::screen_draw::inside(mouse, x, y, w, h),
            sunrise_client::hud_overlays::now_ms(),
        );
        button
    }
    pub(in crate::game) fn chat_history_limit(&self) -> u32 {
        u32::try_from(self.client.chat_ui.chat.history_limit()).unwrap_or(u32::MAX)
    }

    pub(in crate::game) fn set_chat_history_limit(&mut self, limit: u32) {
        self.client
            .chat_ui
            .chat
            .set_history_limit(usize::try_from(limit).unwrap_or(usize::MAX));
    }

    /// `ChatComponent.addClientSystemMessage`: a line the client adds itself.
    /// It joins the connection's chat and its history, with the system
    /// marker; the Chat option's server-message filter does not hide it
    /// (`GuiMessageSource.SYSTEM_CLIENT`). Vanilla fires
    /// `ClientChatReceivedEvent` from `ChatListener` for the lines the server
    /// sent, so this path does not tell `client_chat_received`.
    pub(in crate::game) fn chat_line(&mut self, text: Text) {
        if self.client.remote.is_some() {
            self.add_chat_line(text, ChatSource::Client);
        } else {
            self.local_chat_line(text, Some(SYSTEM_INDICATOR), ChatSource::Client);
        }
    }
    /// `chat_line` for a line the calling mod keeps under `tag` (its own keys): a line it added
    /// under `tag` takes `text` in place, keeping its position, and is logged and counted for
    /// the control socket like a new line; otherwise the line is added as `chat_line` adds it.
    /// Outside a mod's call it is a plain `chat_line`. (The Chat option never hides it:
    /// `ChatAbilities::shows` lets the client's own lines through.)
    pub(in crate::game) fn chat_line_tagged(&mut self, tag: &str, text: Text) {
        let Some(owner) = mod_api::content::crash::current_native_mod() else {
            self.chat_line(text);
            return;
        };
        let ticks = self.visual_ticks();
        let focused = self.client.screen == Screen::Chat;
        let plain = text.plain();
        if self.client.chat_ui.chat.replace_keyed(
            &self.client.gui.atlas,
            owner,
            tag,
            text.clone(),
            ticks,
            focused,
        ) {
            self.record_received_chat(plain);
            return;
        }
        self.client.chat_ui.chat.set_next_key(owner, tag);
        self.chat_line(text);
        self.client.chat_ui.chat.clear_next_key();
    }
    /// Removes the calling mod's lines kept under `tag` from the chat and its history; how many
    /// went. Another mod's lines are never touched.
    pub(in crate::game) fn chat_remove_tagged(&mut self, tag: &str) -> usize {
        let Some(owner) = mod_api::content::crash::current_native_mod() else {
            return 0;
        };
        self.client
            .chat_ui
            .chat
            .remove_keyed(&self.client.gui.atlas, owner, tag)
    }
    /// A client-side system message written directly to `ui.chat`.
    pub fn client_system_message(&mut self, text: Text) {
        assert!(
            !self.is_logical_server(),
            "client_system_message called on server role"
        );
        if self.client.remote.is_some() {
            self.add_chat_line(text, ChatSource::Server);
        } else {
            self.local_chat_line(text, Some(SYSTEM_INDICATOR), ChatSource::Server);
        }
    }
    /// A line on the window's own chat when the Chat option lets `source`
    /// through.
    pub(in crate::game) fn local_chat_line(
        &mut self,
        text: Text,
        indicator: Option<u32>,
        source: ChatSource,
    ) {
        if !self.client.chat_ui.chat.shows(source) {
            return;
        }
        let focused = self.client.screen == Screen::Chat;
        self.client.chat_ui.chat.add(
            &self.client.gui.atlas,
            text,
            indicator,
            source,
            self.visual_ticks(),
            focused,
        );
    }
    /// Keys while the chat screen is open.
    pub(in crate::game::client_side) fn chat_key(
        &mut self,
        key: KeyCode,
        shortcut: Option<sunrise_client::clipboard::EditShortcut>,
    ) -> bool {
        if let Some(shortcut) = shortcut {
            let m = Measure(&self.client.gui.atlas.font);
            let c = Commands {
                dispatcher: chat_tree(&self.client.remote),
                registries: &self.common.ui.registries,
            };
            if let Some(screen) = self.client.chat_ui.chat_screen.as_mut() {
                screen.shortcut(&m, &c, shortcut, &mut self.client.clipboard.0);
                return true;
            }
            return false;
        }
        self.chat_key_code(key, None)
    }
    /// A key (and the text it typed) while the chat screen is open.
    pub fn chat_key_code(&mut self, key: KeyCode, typed_text: Option<&str>) -> bool {
        // The live modifiers, not `self.keys`: every ModifiersChanged runs
        // sync_toggle, which drops ShiftLeft and ControlLeft from `self.keys`
        // while a screen is open.
        let ctrl = self.has_control_down_with_quirk();
        let shift = self.has_shift_down();
        // PvP & HUD's chat search: Ctrl+F starts and stops it, Enter stops it (sends nothing).
        if self.pvp_chat().search
            && match key {
                KeyCode::KeyF => ctrl,
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    self.client.pvp.chat_search_aside.is_some()
                }
                _ => false,
            }
        {
            self.toggle_chat_search();
            return true;
        }
        match key {
            // ChatScreen.keyPressed: the chat scrolls a page.
            KeyCode::PageUp | KeyCode::PageDown
                if !self
                    .client
                    .chat_ui
                    .chat_screen
                    .as_ref()
                    .is_some_and(|s| s.suggestions_visible()) =>
            {
                let n = self.client.chat_ui.chat.lines_per_page(true) as i32 - 1;
                self.client
                    .chat_ui
                    .chat
                    .scroll_by(if key == KeyCode::PageUp { n } else { -n }, true);
                return true;
            }
            _ => {}
        }
        let Some(mut screen) = self.client.chat_ui.chat_screen.take() else {
            return false;
        };
        let m = Measure(&self.client.gui.atlas.font);
        let c = Commands {
            dispatcher: chat_tree(&self.client.remote),
            registries: &self.common.ui.registries,
        };
        let (handled, action) =
            screen.key_input(&m, &c, &self.client.chat_ui.chat.recent, key, shift, ctrl);
        match action {
            ChatAction::Close => {
                // Escape: the box still holds its text, which close_chat keeps as the draft
                // (a search's query gives way to the text it put aside: keep_chat_draft).
                self.client.chat_ui.chat_screen = Some(screen);
                self.close_chat(ChatExit::Intentional);
            }
            ChatAction::Submit(message) => {
                // ChatScreen.keyPressed: handleChatInput runs with the screen still open,
                // then `closeOnSubmit` closes it. InBedChatScreen sends without closing
                // (`closeOnSubmit` is false): the box empties and the screen stays.
                let close_on_submit =
                    !(self.client.chat_ui.in_bed_chat && self.common.sleeping.is_some());
                self.client.chat_ui.chat_screen = Some(screen);
                if !message.is_empty() {
                    self.handle_chat_input(&message);
                }
                if close_on_submit {
                    // A sent message discards the draft (DONE), whatever the box holds.
                    self.close_chat(ChatExit::Done);
                } else {
                    // The line is handled first, then the box empties and chat scrolls back.
                    if let Some(screen) = self.client.chat_ui.chat_screen.as_mut() {
                        let m = Measure(&self.client.gui.atlas.font);
                        let c = Commands {
                            dispatcher: chat_tree(&self.client.remote),
                            registries: &self.common.ui.registries,
                        };
                        screen.suggest_command(&m, &c, "");
                    }
                    self.client.chat_ui.chat.reset_scroll();
                }
            }
            ChatAction::None => {
                // charTyped: the text the key typed.
                if let Some(text) = typed_text
                    && self.types_text()
                {
                    screen.type_text(&m, &c, text);
                }
                self.client.chat_ui.chat_screen = Some(screen);
            }
        }
        self.notify_chat_suggestion_completion();
        handled
    }
    pub(in crate::game::client_side) fn notify_chat_suggestion_completion(&mut self) {
        if let Some(completion) = self
            .client
            .chat_ui
            .chat_screen
            .as_mut()
            .and_then(ChatScreen::take_suggestion_completion)
        {
            self.native_chat_suggestion_applied(
                &completion.original,
                completion.range.0,
                completion.range.1,
                &completion.text,
                &completion.input,
            );
        }
    }
    /// The server's completions for an `ask_server` request
    /// (ClientPacketListener.handleCommandSuggestions).
    pub(in crate::game::client_side) fn chat_server_suggestions(
        &mut self,
        id: i32,
        range: (usize, usize),
        list: Vec<(String, Option<String>)>,
    ) {
        let m = Measure(&self.client.gui.atlas.font);
        let c = Commands {
            dispatcher: chat_tree(&self.client.remote),
            registries: &self.common.ui.registries,
        };
        if let Some(screen) = self.client.chat_ui.chat_screen.as_mut() {
            screen.server_suggestions(&m, &c, id, range, list);
        }
    }
    /// Text typed into the chat box (control socket `chat`).
    pub fn chat_type(&mut self, text: &str) {
        let m = Measure(&self.client.gui.atlas.font);
        let c = Commands {
            dispatcher: chat_tree(&self.client.remote),
            registries: &self.common.ui.registries,
        };
        if let Some(screen) = self.client.chat_ui.chat_screen.as_mut() {
            screen.type_text(&m, &c, text);
        }
    }
    /// ChatScreen.handleChatInput: the message comes in normalized
    /// (`normalizeChatMessage`: trimmed, whitespace runs collapsed, cut to the
    /// 256 UTF-16 units the packet holds), into history, then as a command or
    /// a chat line. The Chat option's restrictions stop the message here too,
    /// so a control-socket `chat` cannot send what the player's setting hides.
    pub fn handle_chat_input(&mut self, message: &str) {
        let message = normalize_chat_message(message);
        if message.is_empty() || !self.chat_abilities().allows(&message) {
            return;
        }
        self.client.chat_ui.chat.add_recent(&message);
        if let Some(args) =
            sunrise_profiler_args(&message, self.client.remote.as_ref().map(|r| &r.chat_tree))
        {
            self.profiler_chat(args);
            return;
        }
        if self.client.remote.is_some() {
            self.send_signed_chat_input(&message);
            return;
        }
        if let Some(command) = message.strip_prefix('/') {
            self.run_client_native_command(command);
            return;
        }
        let line = Text::translate(
            "chat.type.text",
            &[
                Text::literal(self.common.options.player_name()),
                Text::untrusted(message.as_str()),
            ],
        );
        let focused = self.client.screen == Screen::Chat;
        self.client.chat_ui.chat.add(
            &self.client.gui.atlas,
            line,
            None,
            ChatSource::Player,
            self.visual_ticks(),
            focused,
        );
    }
    pub(in crate::game::client_side) fn chat_scroll(&mut self, amount: f32) {
        let cursor = self.client.cursor;
        let mouse = (cursor.0.floor() as i32, cursor.1.floor() as i32);
        if let Some(screen) = self.client.chat_ui.chat_screen.as_mut()
            && screen.scroll(mouse, amount as f64)
        {
            return;
        }
        // ChatScreen.mouseScrolled: `hasShiftDown`, either Shift key.
        let shift = self.has_shift_down();
        let amount = amount.clamp(-1.0, 1.0) * if shift { 1.0 } else { 7.0 };
        self.client.chat_ui.chat.scroll_by(amount as i32, true);
    }
    pub fn chat_click(&mut self) -> bool {
        let cursor = self.client.cursor;
        let height = self.client.renderer.0.as_ref().map_or(720.0, |r| {
            HudBatch::new(
                [r.config.width, r.config.height],
                self.client.settings.gui_scale_inputs(),
            )
            .height
        });
        let m = Measure(&self.client.gui.atlas.font);
        let c = Commands {
            dispatcher: chat_tree(&self.client.remote),
            registries: &self.common.ui.registries,
        };
        if self.client.chat_ui.in_bed_chat && self.common.sleeping.is_some() {
            let width = self.client.renderer.0.as_ref().map_or(1280.0, |r| {
                HudBatch::new(
                    [r.config.width, r.config.height],
                    self.client.settings.gui_scale_inputs(),
                )
                .width
            });
            let (x, y, w, h) = self
                .built_chat(width, height)
                .map_or_else(|| Self::leave_bed_button(width, height), |l| l.leave_bed);
            if sunrise_client::screen_draw::inside((cursor.0 as i32, cursor.1 as i32), x, y, w, h) {
                self.remote_stop_sleeping();
                return true;
            }
        }
        if let Some(screen) = self.client.chat_ui.chat_screen.as_mut() {
            let (x, y) = (f64::from(cursor.0), f64::from(cursor.1));
            let double_click = screen.live_double_click(x, y);
            if screen.click(&m, &c, x, y, double_click) {
                self.notify_chat_suggestion_completion();
                return true;
            }
        } else {
            return false;
        }
        let style = self
            .client
            .chat_ui
            .chat
            .style_at(
                &self.client.gui.atlas.font,
                height as i32,
                cursor.0 as f64,
                cursor.1 as f64,
            )
            .or_else(|| {
                self.client.chat_ui.chat.queue_style_at(
                    &self.client.gui.atlas.font,
                    height as i32,
                    cursor.0 as f64,
                    cursor.1 as f64,
                )
            });
        // PvP & HUD's copy on click: a message where the click has nothing else to do.
        if self.pvp_chat().copy
            && !self.has_shift_down()
            && style.as_ref().is_none_or(|s| s.click.is_none())
            && let Some(message) =
                self.client
                    .chat_ui
                    .chat
                    .message_at(height as i32, cursor.0 as f64, cursor.1 as f64)
        {
            use sunrise_client::clipboard::Clipboard;
            self.client.clipboard.0.set(message);
            let copied =
                Text::translate_with_fallback("sunrise.chat.copied", "Message copied", &[]);
            self.client.titles.set_overlay(copied, false);
            return true;
        }
        let Some(style) = style else {
            return false;
        };
        // ChatScreen.handleComponentClicked: with Shift held only an
        // insertion acts, else only the click event.
        if self.has_shift_down() {
            if let Some(insertion) = style.insertion
                && let Some(screen) = self.client.chat_ui.chat_screen.as_mut()
            {
                screen.type_text(&m, &c, &insertion);
                return true;
            }
            return false;
        }
        match style.click {
            Some(ClickEvent::CopyToClipboard(value)) => {
                use sunrise_client::clipboard::Clipboard;
                self.client.clipboard.0.set(&value);
                true
            }
            Some(ClickEvent::OpenFile(path)) => {
                super::packs::open_folder(std::path::Path::new(&path));
                true
            }
            Some(ClickEvent::OpenUrl(url)) => {
                self.click_url(url);
                true
            }
            Some(ClickEvent::SuggestCommand(command)) => {
                if let Some(screen) = self.client.chat_ui.chat_screen.as_mut() {
                    screen.suggest_command(&m, &c, &command);
                }
                true
            }
            Some(ClickEvent::RunCommand(command)) => {
                self.click_command(&command);
                true
            }
            Some(ClickEvent::Profiler(command)) => {
                if let Some(args) = command
                    .strip_prefix('/')
                    .and_then(super::profile_capture::profiler_args)
                {
                    self.profiler_chat(args);
                } else {
                    return false;
                }
                true
            }
            // Screen.defaultHandleGameClickEvent: the dialog opens over the
            // screen, and closes back to it.
            Some(ClickEvent::ShowDialog(dialog)) => {
                self.show_dialog_ref(&dialog);
                true
            }
            // Screen.defaultHandleGameClickEvent: custom actions retain chat.
            // ChatScreen.handleComponentClicked: the queue row's internal id
            // shows the next held-back message; other custom ids go on.
            Some(ClickEvent::Custom { id, .. }) if id == QUEUE_EXPAND_ID => {
                self.accept_next_delayed_message();
                true
            }
            Some(ClickEvent::Custom { id, payload }) => {
                if let Some(remote) = &self.client.remote {
                    remote
                        .session
                        .send_custom_click(&id, payload.map(std::sync::Arc::unwrap_or_clone));
                } else {
                    return false;
                }
                true
            }
            // Screen.defaultHandleClickEvent: a page link means nothing in chat.
            Some(ClickEvent::ChangePage(_)) | None => false,
        }
    }
    /// `Screen.clickUrlAction`: nothing with Web Links off, else the link
    /// opens, after the confirmation unless Prompt on Links is off.
    pub fn click_url(&mut self, url: String) {
        if !self.client.settings.chat_links {
            return;
        }
        if self.client.settings.chat_links_prompt {
            self.open_screen_overlay("link_prompt", |game| {
                game.client.link_prompt = Some(url.into())
            });
        } else {
            super::packs::open_uri(&url);
        }
    }

    /// `ConfirmLinkScreen.confirmLinkNow` for a mod: the confirmation opens over whatever
    /// screen is open, and either answer returns to it. The player's answer is the consent, so
    /// the chat's link options do not apply (they gate chat click events only). Only http and
    /// https pass, as `Util.parseAndValidateUntrustedUri` has it.
    pub(in crate::game) fn confirm_link(
        &mut self,
        uri: &str,
        trusted: bool,
    ) -> Result<(), mod_api::OpenLinkError> {
        use mod_api::OpenLinkError;
        if !self.native_window() {
            return Err(OpenLinkError::Unavailable);
        }
        if !sunrise_core::text::untrusted_http_url(uri) {
            return Err(OpenLinkError::Invalid);
        }
        self.open_screen_overlay("link_prompt", |game| {
            game.client.link_prompt = Some(LinkPrompt {
                url: uri.to_owned(),
                trusted,
            });
        });
        Ok(())
    }

    /// `Blaze3D.openPath` for a mod: the desktop's handler at once, with no confirmation.
    pub(in crate::game) fn open_path(&self, path: &Path) -> Result<(), mod_api::OpenPathError> {
        use mod_api::OpenPathError;
        if !self.native_window() {
            return Err(OpenPathError::Unavailable);
        }
        sunrise_client::desktop::open_path(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                OpenPathError::NotFound
            } else {
                OpenPathError::Failed(error.to_string())
            }
        })
    }
    /// `ConfirmLinkScreen` (untrusted): the link and its warning over the
    /// chat, answered yes to open it.
    pub(in crate::game::client_side) fn link_prompt_layout(
        &self,
        prompt: &LinkPrompt,
        size: (f32, f32),
    ) -> sunrise_client::confirm_screen::Choice {
        let message = if prompt.trusted {
            Text::literal(&prompt.url)
        } else {
            let warning = Text::translate("chat.link.warning", &[]).with_color(0xFF_CCCC);
            Text::literal(&prompt.url)
                .append(Text::literal("\n"))
                .append(warning)
        };
        // ConfirmLinkScreen.addButtons: yes, copy and no, 100 wide each.
        sunrise_client::confirm_screen::Choice::new(
            &self.client.gui.atlas.font,
            size,
            &Text::translate(prompt.title_key(), &[]),
            &message,
            3,
            100.0,
        )
    }
    pub(in crate::game::client_side) fn draw_link_prompt(&self, batch: &mut HudBatch) {
        let Some(prompt) = &self.client.link_prompt else {
            return;
        };
        sunrise_client::widgets::cover_frame();
        // ConfirmLinkScreen: the question and the link (its warning is a
        // widget of its own, not narrated).
        sunrise_client::widgets::begin_screen(|| {
            super::screen_info::confirm(
                "chat/confirm_link",
                &super::screen_info::tr(prompt.title_key(), &[]),
                &prompt.url,
            )
        });
        let atlas = &self.client.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        sunrise_client::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
        let labels = prompt.button_keys().map(|key| Text::translate(key, &[]));
        let mut layout = self.link_prompt_layout(prompt, (w, h));
        layout.declare_no(2);
        layout.draw(batch, atlas, self.client.cursor, &labels);
        self.draw_overlay_mounted_widgets(batch, "link_prompt");
    }
    /// The prompt's answer: the link opens on yes; either way the chat is back.
    pub fn answer_link_prompt(&mut self, yes: bool) {
        let Some(prompt) = self.client.link_prompt.take() else {
            return;
        };
        self.refresh_screen_widgets();
        if yes {
            super::packs::open_uri(&prompt.url);
        }
    }
    pub(in crate::game::client_side) fn link_prompt_click(&mut self) -> bool {
        let Some(prompt) = self.client.link_prompt.clone() else {
            return false;
        };
        let pressed = self
            .link_prompt_layout(&prompt, self.gui_size())
            .pressed_at(self.client.cursor);
        if let Some(button) = pressed {
            self.play_audio_button_click();
            if button == LINK_COPY {
                // ConfirmLinkScreen.copyToClipboard, then the callback's no.
                use sunrise_client::clipboard::Clipboard;
                self.client.clipboard.0.set(&prompt.url);
            }
            self.answer_link_prompt(button == LINK_YES);
        }
        pressed.is_some()
    }
    /// `Screen.clickCommandAction`: run a clicked command as the player
    /// without adding it to the chat history. A client mod's command goes
    /// first, as `ClientPacketListener.sendUnattendedCommand` lets client
    /// commands take it.
    pub(in crate::game::client_side) fn click_command(&mut self, command: &str) {
        if self.client.remote.is_some() {
            self.send_unattended_command(command);
        } else {
            let command = command.strip_prefix('/').unwrap_or(command);
            self.run_client_native_command(command);
        }
    }
    /// PvP & HUD's chat search on or off. The box holds the query: what was typed is put aside
    /// when the search starts and comes back when it stops.
    fn toggle_chat_search(&mut self) {
        let Some(screen) = self.client.chat_ui.chat_screen.as_mut() else {
            return;
        };
        let next = if let Some((text, _)) = self.client.pvp.chat_search_aside.take() {
            text
        } else {
            self.client.pvp.chat_search_aside =
                Some((screen.value().to_owned(), screen.is_draft()));
            String::new()
        };
        let m = Measure(&self.client.gui.atlas.font);
        let c = Commands {
            dispatcher: chat_tree(&self.client.remote),
            registries: &self.common.ui.registries,
        };
        screen.suggest_command(&m, &c, &next);
    }
    /// Chat lines over the HUD (focused while the chat screen is open).
    pub(in crate::game) fn draw_chat(&mut self, batch: &mut HudBatch) {
        let focused = self.client.screen == Screen::Chat;
        let options = self.pvp_chat();
        self.client
            .chat_ui
            .chat
            .set_qol(&self.client.gui.atlas, options.qol());
        // A search ends with its option; a chat closed some other way ends it in keep_chat_draft.
        if self.client.chat_ui.chat_screen.is_none() {
            // The chat went without closing (leaving the world): nothing to give back.
            self.client.pvp.chat_search_aside = None;
        } else if self.client.pvp.chat_search_aside.is_some() && !options.search {
            self.toggle_chat_search();
        }
        let searching = focused && self.client.pvp.chat_search_aside.is_some();
        let query = self
            .client
            .chat_ui
            .chat_screen
            .as_ref()
            .filter(|_| searching)
            .map(|s| s.value());
        self.client
            .chat_ui
            .chat
            .set_search(&self.client.gui.atlas, query);
        self.client.chat_ui.chat.search_prompt = searching;
        // `ChatComponent`'s queue row: the messages Chat Delay holds back.
        self.client.chat_ui.chat.queue = self.chat_queue_len();
        if self.client.chat_ui.hide_gui && !focused {
            return;
        }
        let revision = self.inline_images_revision();
        if self.client.chat_ui.chat.inline_images_stale(revision) {
            // Layout borrows a read-only Host while asking listeners.
            let mut chat = std::mem::take(&mut self.client.chat_ui.chat);
            chat.refresh_inline_images(&self.client.gui.atlas, revision, |text| {
                self.substitute_inline_images(text)
            });
            self.client.chat_ui.chat = chat;
        }
        self.client.chat_ui.chat.smooth = self.client.settings.extras.chat_animation;
        self.client.chat_ui.chat.hide_indicator = !self.client.settings.extras.chat_indicator;
        // The fractional tick makes the smooth chat extra's slide move every
        // frame; vanilla's fade reads it too.
        let now = self.visual_ticks() as f32 + self.common.clock.fraction() as f32;
        // NeoForge's `CustomizeGuiOverlayEvent.Chat`: a mod may move the chat's bottom-left
        // corner, and the chat is drawn shifted by as much (`Gui.renderChat`'s translate).
        let (width, height) = (batch.width as i32, batch.height as i32);
        let (home_x, home_y) = (0, height - 40);
        let (mut x, mut y) = (home_x, home_y);
        self.native_customize_chat_overlay(width, height, focused, &mut x, &mut y);
        let offset = [(x - home_x) as f32, (y - home_y) as f32];
        self.client
            .chat_ui
            .chat
            .draw_at(batch, &self.client.gui.atlas, now, focused, offset);
    }
    /// `HoverEvent.EntityTooltipInfo.getTooltipLines`: the name when the
    /// event has one, the entity type and the UUID.
    pub(in crate::game::client_side) fn entity_tooltip_lines(
        entity: &serde_json::Value,
    ) -> Vec<(Text, u32)> {
        let mut lines = Vec::new();
        if let Some(name) = entity.get("name") {
            let name = sunrise_game_data::component::to_text(name, &mut |s| Text::literal(s));
            lines.push((name, u32::MAX));
        }
        let id = entity
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let kind = Text::translate(&format!("entity.{}", id.replace(':', ".")), &[]);
        lines.push((
            Text::translate("gui.entity_tooltip.type", &[kind]),
            u32::MAX,
        ));
        if let Some(uuid) = entity.get("uuid").and_then(Self::hover_event_entity_uuid) {
            lines.push((Text::literal(uuid), u32::MAX));
        }
        lines
    }
    /// The UUID of a ShowEntity event: its string, or the int array
    /// `UUIDUtil.CODEC` writes (most significant word first).
    pub(in crate::game::client_side) fn hover_event_entity_uuid(
        v: &serde_json::Value,
    ) -> Option<String> {
        if let Some(s) = v.as_str() {
            return Some(s.to_owned());
        }
        let words = v.as_array().filter(|w| w.len() == 4)?;
        let mut bits = 0u128;
        for w in words {
            bits = (bits << 32) | u128::from(w.as_i64()? as u32);
        }
        Some(sunrise_protocol::crypto::uuid_string(bits))
    }
    pub(in crate::game::client_side) fn draw_chat_screen(&mut self, batch: &mut HudBatch) {
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        // ChatScreen extracts focused chat separately, outside the HUD chat slot.
        self.draw_chat(batch);
        let Some(mut screen) = self.client.chat_ui.chat_screen.take() else {
            self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
            return;
        };
        // ChatScreen.updateNarrationState: the restricted prompt is
        // `displayMode.showRestrictedPrompt`, the abilities' restrictions.
        let in_bed = self.client.chat_ui.in_bed_chat && self.common.sleeping.is_some();
        sunrise_client::widgets::begin_screen(|| {
            super::screen_info::chat(
                in_bed,
                screen.value(),
                screen.abilities.any_restriction(),
                &super::screen_info::tr,
            )
        });
        // The build made the box at this size (`ChatScreen.resize`).
        let m = Measure(&self.client.gui.atlas.font);
        // Options.getBackgroundColor(Integer.MIN_VALUE).
        let background = if self.client.settings.file.bool("backgroundForChatOnly") {
            i32::MIN
        } else {
            self.client.settings.text_background(0.0) as i32
        };
        let mouse = (
            self.client.cursor.0.floor() as i32,
            self.client.cursor.1.floor() as i32,
        );
        let blink = self
            .client
            .chat_ui
            .chat_opened
            .map_or(0, |t| t.elapsed().as_millis() as u64);
        let draws = screen.draw(&m, mouse, blink, background);
        self.client.chat_ui.chat_screen = Some(screen);
        let tooltip = sunrise_client::screen_draw::paint(batch, &self.client.gui.atlas, &draws);
        if in_bed {
            let (w, h) = (batch.width, batch.height);
            let rect = self
                .built_chat(w, h)
                .map_or_else(|| Self::leave_bed_button(w, h), |l| l.leave_bed);
            let button = Self::leave_bed_draws(&m, mouse, rect);
            let _ = sunrise_client::screen_draw::paint(batch, &self.client.gui.atlas, &button);
        }
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
        sunrise_client::screen_draw::paint_tooltip(batch, &self.client.gui.atlas, &draws, tooltip);
        // The hovered message part's tooltip (`renderComponentHoverEffect`),
        // or the text of the message tag under the cursor.
        let height = batch.height as i32;
        let (cx, cy) = (
            f64::from(self.client.cursor.0),
            f64::from(self.client.cursor.1),
        );
        // handleTagIcon: the chat-modified icons over the cursor or its message.
        let (tag_tooltip, tag_icons) =
            self.client
                .chat_ui
                .chat
                .focused_tags(&self.client.gui.atlas.font, height, cx, cy);
        sunrise_client::hud_layout::paint(batch, &self.client.gui.atlas, &tag_icons);
        let style = self
            .client
            .chat_ui
            .chat
            .style_at(&self.client.gui.atlas.font, height, cx, cy)
            .or_else(|| {
                self.client
                    .chat_ui
                    .chat
                    .queue_style_at(&self.client.gui.atlas.font, height, cx, cy)
            });
        if let Some(style) = style {
            // ActiveTextCollector.ClickableStyleFinder: a click event, or an
            // insertion while Shift is held (ChatScreen.includeInsertions).
            if style.click.is_some() || (style.insertion.is_some() && self.has_shift_down()) {
                sunrise_client::widgets::request_cursor(
                    sunrise_client::widgets::Cursor::PointingHand,
                );
            }
            self.component_hover_effect(batch, &style, self.client.cursor);
        } else if let Some(tag) = &tag_tooltip {
            // The message tag's text, split at 210.
            let lines = self.wrapped_rows(tag, 210);
            self.draw_tooltip_lines(batch, &lines, self.client.cursor.0, self.client.cursor.1);
        }
    }
    /// `GuiGraphicsExtractor.componentHoverEffect`: the hover of the text component under the
    /// cursor at `at`, as the chat, a book and a dialog draw it. `component_hover` is asked
    /// first and may cancel it. Then a text hover is split at half the screen's width (at
    /// least 200), an item hover draws its stack's tooltip, and an entity hover its name, type
    /// and UUID while advanced tooltips are on.
    pub fn component_hover_effect(
        &mut self,
        batch: &mut HudBatch,
        style: &sunrise_game_data::text::Style,
        at: (f32, f32),
    ) {
        if self.component_hover_canceled(batch, style, at) {
            return;
        }
        if let Some(hover) = &style.hover {
            let wrap = (batch.width as i32 / 2).max(200);
            let lines = self.wrapped_rows(hover, wrap);
            self.draw_tooltip_lines(batch, &lines, at.0, at.1);
        } else if let Some(stack) = style
            .hover_item
            .as_deref()
            .and_then(sunrise_items::item_data::stack_from_hover)
            .filter(|s| self.items().get(&s.item).is_some())
        {
            self.draw_stack_tooltip(batch, &stack, at.0, at.1);
        } else if let Some(entity) = style
            .hover_entity
            .as_deref()
            .filter(|_| self.client.settings.file.bool("advancedItemTooltips"))
        {
            // A ShowEntity tooltip only with advanced tooltips on.
            let lines = Self::entity_tooltip_lines(entity);
            self.draw_tooltip_lines(batch, &lines, at.0, at.1);
        }
    }
    /// `component_hover`, only while a mod listens: true when a mod cancelled the hover.
    pub(in crate::game::client_side) fn component_hover_canceled(
        &self,
        batch: &mut HudBatch,
        style: &sunrise_game_data::text::Style,
        at: (f32, f32),
    ) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::ComponentHover) {
            return false;
        }
        let Some(hover) = sunrise_sim::api_text::style_hover(style) else {
            return false;
        };
        let mut cancel = false;
        let mut draw = crate::game::host::draw::GameDrawContext::new(self, batch);
        self.native_component_hover(mod_api::__private::ComponentHoverCall {
            draw: &mut draw,
            hover: &hover,
            x: at.0 as i32,
            y: at.1 as i32,
            cancel: &mut cancel,
        });
        cancel
    }
    /// `Font.split`: a tooltip text wrapped at `width`, one white row per line.
    pub(in crate::game::client_side) fn wrapped_rows(
        &self,
        text: &Text,
        width: i32,
    ) -> Vec<(Text, u32)> {
        let split = self
            .client
            .gui
            .atlas
            .font
            .split(&text.resolve_legacy(), width as f32);
        split.into_iter().map(|row| (row, u32::MAX)).collect()
    }
    /// F1 hides the HUD (see `hud`); the screenshot key still works.
    pub fn toggle_hide_gui(&mut self) {
        self.client.chat_ui.toggle_hide_gui();
    }
    /// F2: ScreenShot.grab into `screenshots/` with vanilla's file name.
    pub(in crate::game::client_side) fn take_screenshot(&mut self) -> Option<std::path::PathBuf> {
        if self.client.screenshot_sequence.0.is_some() {
            return None;
        }
        let dir = sunrise_level::logging::player_dir().join("screenshots");
        if std::fs::create_dir_all(&dir).is_err() {
            return None;
        }
        let name = screenshot_name(&dir, &self.client.chat_ui.screenshot_messages);
        let renderer = self.client.renderer.0.as_mut()?;
        let path = dir.join(name);
        // A second shortcut before rendering replaces the queued frame. Release
        // that request's name: no readback will arrive to finish it.
        if let Some(replaced) = renderer.screenshot.as_ref() {
            self.client
                .chat_ui
                .screenshot_messages
                .retain(|p| *p != replaced.path);
        }
        renderer.screenshot = Some(sunrise_client::gfx::ScreenshotRequest {
            path: path.clone(),
            size: None,
            scaled: true,
            background: true,
        });
        self.client.chat_ui.screenshot_messages.push(path.clone());
        Some(path)
    }
    /// `Minecraft.handleGlobalKeyPress`'s screenshot branch: the
    /// `key.screenshot` mapping (F2 by default) takes the picture before a
    /// screen or a text box sees the key, and the debug modifier suppresses it.
    pub(in crate::game::client_side) fn screenshot_shortcut(
        &mut self,
        key: KeyCode,
        repeat: bool,
    ) -> bool {
        if repeat
            || sunrise_client::keybinds::key_name(key)
                != Some(self.client.settings.keys.get("key.screenshot"))
        {
            return false;
        }
        self.screenshot_action()
    }
    /// key.screenshot from any device: not while F3 is down, where F3+F2 is a
    /// debug key.
    pub(in crate::game::client_side) fn screenshot_action(&mut self) -> bool {
        if self.client.debug_keys.modifier_down {
            return false;
        }
        let _ = self.take_screenshot();
        true
    }
    /// Readback is complete: fire the client hook, then save the image it left.
    pub(in crate::game::client_side) fn save_screenshot(
        &mut self,
        mut pending: sunrise_client::gfx::PendingScreenshot,
    ) -> Option<std::result::Result<[u32; 2], String>> {
        if !self
            .client
            .chat_ui
            .screenshot_messages
            .contains(&pending.path)
        {
            if !pending.background {
                return Some(pending.save().map_err(|e| e.to_string()));
            }
            self.client.chat_ui.screenshot_writes.push(
                super::screenshot_writer::ScreenshotWrite::start(pending, None, None, false),
            );
            return None;
        }
        let at = self
            .client
            .chat_ui
            .screenshot_messages
            .iter()
            .position(|p| *p == pending.path)
            .unwrap();
        let mut file_name = pending
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let mut message = None;
        let mut cancel = false;
        self.native_screenshot(
            pending.image_mut(),
            &mut file_name,
            &mut message,
            &mut cancel,
        );
        if cancel {
            self.client.chat_ui.screenshot_messages.remove(at);
            self.post_screenshot_chat(screenshot_cancel_line(message));
            return None;
        }
        let root = sunrise_level::logging::player_dir().join("screenshots");
        self.client.chat_ui.screenshot_writes.push(
            super::screenshot_writer::ScreenshotWrite::start(
                pending,
                Some((root, file_name)),
                message,
                true,
            ),
        );
        None
    }
    /// After a frame: report a finished F2 screenshot in chat.
    pub(in crate::game) fn finish_screenshot(
        &mut self,
        result: &std::result::Result<[u32; 2], String>,
        message: Option<Text>,
    ) {
        if self.client.chat_ui.screenshot_messages.is_empty() {
            return;
        }
        let path = self.client.chat_ui.screenshot_messages.remove(0);
        let message = match result {
            Ok(_) => {
                log::info!("Saved screenshot to {}", path.display());
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                let link = Text::styled(
                    name,
                    Style {
                        underlined: Some(true),
                        click: Some(ClickEvent::OpenFile(path.to_string_lossy().into_owned())),
                        ..Style::default()
                    },
                );
                message.unwrap_or_else(|| Text::translate("screenshot.success", &[link]))
            }
            Err(e) => Text::translate("screenshot.failure", &[Text::literal(e.clone())]),
        };
        self.post_screenshot_chat(message);
    }

    pub(in crate::game::client_side) fn post_screenshot_chat(&mut self, message: Text) {
        let focused = self.client.screen == Screen::Chat;
        self.client.chat_ui.chat.add(
            &self.client.gui.atlas,
            message,
            None,
            ChatSource::Client,
            self.visual_ticks(),
            focused,
        );
    }
}

/// `ScreenshotEvent.getCancelMessage`: the listener's line, or the default reason.
pub(in crate::game::client_side) fn screenshot_cancel_line(message: Option<Text>) -> Text {
    message.unwrap_or_else(|| {
        Text::translate(
            "screenshot.failure",
            &[Text::translate("sunrise.screenshot.canceled", &[])],
        )
    })
}
/// `yyyy-MM-dd_HH.mm.ss[_n].png` (Util.getFilenameFormattedDateTime, in local time).
pub(in crate::game::client_side) fn screenshot_name(
    dir: &std::path::Path,
    pending: &[std::path::PathBuf],
) -> String {
    // Util.getFilenameFormattedDateTime: the local date and time.
    let base = chrono::Local::now().format("%Y-%m-%d_%H.%M.%S").to_string();
    let mut n = 1;
    loop {
        let name = if n == 1 {
            format!("{base}.png")
        } else {
            format!("{base}_{n}.png")
        };
        let path = dir.join(&name);
        if !path.exists() && !pending.contains(&path) {
            return name;
        }
        n += 1;
    }
}

/// `ConfirmLinkScreen`'s buttons, left to right: yes, copy, no.
pub(in crate::game) const LINK_YES: usize = 0;
pub(in crate::game) const LINK_COPY: usize = 1;

#[derive(Clone)]
pub struct LinkPrompt {
    url: String,
    trusted: bool,
}

impl From<String> for LinkPrompt {
    /// A chat click's link: untrusted.
    fn from(url: String) -> Self {
        Self {
            url,
            trusted: false,
        }
    }
}

impl From<&str> for LinkPrompt {
    /// An untrusted link, as a chat click opens it.
    fn from(url: &str) -> Self {
        url.to_owned().into()
    }
}

impl LinkPrompt {
    /// `ConfirmLinkScreen.confirmMessage`'s title.
    fn title_key(&self) -> &'static str {
        if self.trusted {
            "chat.link.confirmTrusted"
        } else {
            "chat.link.confirm"
        }
    }
    /// Labels for the chosen trust mode, with the same copy button in both.
    fn button_keys(&self) -> [&'static str; 3] {
        if self.trusted {
            ["chat.link.open", "chat.copy", "gui.cancel"]
        } else {
            ["gui.yes", "chat.copy", "gui.no"]
        }
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;
    use std::sync::Arc;
    use sunrise_client::chat::{ChatAbilities, ChatVisibility};
    use sunrise_command::Registries;
    use sunrise_gameplay_common::game::chat_common::summonable_entities;

    #[test]
    fn a_typed_profiler_is_the_clients_unless_the_server_owns_the_name() {
        let vanilla = sunrise_command::vanilla::game_dispatcher();
        let ours = vanilla.with_profiler();
        assert_eq!(
            super::sunrise_profiler_args("/profiler share", Some(&ours)),
            Some(" share")
        );
        assert_eq!(super::sunrise_profiler_args("/profiler", None), Some(""));
        assert_eq!(
            super::sunrise_profiler_args("/profilers", Some(&ours)),
            None
        );
        let mut server = vanilla.clone();
        let node = server.literal(0, "profiler");
        server.executes(node, "server:profiler");
        let theirs = server.with_profiler();
        assert_eq!(
            super::sunrise_profiler_args("/profiler share", Some(&theirs)),
            None
        );
    }

    /// A mod's text reaches the chat box as typing would leave it: emoji and
    /// accents stay, control characters, DEL and the section sign go.
    #[test]
    fn canceled_screenshot_keeps_a_custom_line_or_the_default_reason() {
        let custom = Text::literal("kept");
        assert_eq!(screenshot_cancel_line(Some(custom.clone())), custom);
        assert_eq!(
            screenshot_cancel_line(None),
            Text::translate(
                "screenshot.failure",
                &[Text::translate("sunrise.screenshot.canceled", &[])]
            )
        );
    }

    /// `ChatScreen.shouldDiscardDraft`.
    #[test]
    fn draft_discard_follows_the_exit_reason() {
        assert!(!should_discard_draft(ChatExit::Interrupted, false));
        assert!(!should_discard_draft(ChatExit::Interrupted, true));
        assert!(should_discard_draft(ChatExit::Intentional, false));
        assert!(!should_discard_draft(ChatExit::Intentional, true));
        assert!(should_discard_draft(ChatExit::Done, true));
    }

    /// `Screenshot.getFile` picks the first free name of the second; an F2
    /// screenshot still being written on its thread has no file yet, so a
    /// second press in the same second takes the next name, not the same one.
    #[test]
    fn a_screenshot_being_written_keeps_its_name() {
        let dir = std::env::temp_dir().join(format!("sunrise-shots-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = screenshot_name(&dir, &[]);
        let second = screenshot_name(&dir, &[dir.join(&first)]);
        assert_ne!(first, second);
        // Unless the clock turned to the next second in between.
        let same_second = second.starts_with(first.trim_end_matches(".png"));
        assert!(
            !same_second || second == first.replace(".png", "_2.png"),
            "{second}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resource_ids_are_listed_once_per_root() {
        let root = std::env::temp_dir().join(format!("sunrise-ids-{}", std::process::id()));
        let recipe = root.join("data/minecraft/recipe");
        std::fs::create_dir_all(recipe.join("sub")).unwrap();
        std::fs::write(recipe.join("b.json"), "{}").unwrap();
        std::fs::write(recipe.join("sub/a.json"), "{}").unwrap();
        std::fs::write(recipe.join("notes.txt"), "").unwrap();
        let first = resource_ids(&root);
        assert_eq!(first.recipes, ["minecraft:b", "minecraft:sub/a"]);
        std::fs::write(recipe.join("c.json"), "{}").unwrap();
        assert!(Arc::ptr_eq(&first, &resource_ids(&root)));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A content mod's recipes are suggested with vanilla's, so `/recipe`
    /// offers the ids the recipe index knows.
    #[test]
    fn recipes_of_every_namespace_are_listed() {
        let root = std::env::temp_dir().join(format!("sunrise-recipe-ids-{}", std::process::id()));
        for dir in ["data/minecraft/recipe", "data/mymod/recipe/press"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::write(root.join("data/minecraft/recipe/stick.json"), "{}").unwrap();
        std::fs::write(root.join("data/mymod/recipe/press/iron.json"), "{}").unwrap();
        assert_eq!(
            resource_ids(&root).recipes,
            ["minecraft:stick", "mymod:press/iron"]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn command_exports_are_read_once_per_folder() {
        let dir = std::env::temp_dir().join(format!("sunrise-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("command-registries.json"), r#"{"effects":["a"]}"#).unwrap();
        let first = command_export(&dir, true).unwrap();
        assert_eq!(first.effects, ["a"]);
        std::fs::write(dir.join("command-registries.json"), "{}").unwrap();
        assert!(Arc::ptr_eq(&first, &command_export(&dir, true).unwrap()));
        assert!(command_export(&dir.join("missing"), true).is_none());
        // A dedicated server's read is not kept.
        let other = dir.join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(
            other.join("command-registries.json"),
            r#"{"effects":["b"]}"#,
        )
        .unwrap();
        let once = command_export(&other, false).unwrap();
        assert_eq!(once.effects, ["b"]);
        assert!(!Arc::ptr_eq(&once, &command_export(&other, false).unwrap()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_client_commands_usage_is_parsed_by_the_chat_tree() {
        let reg = Registries::default();
        let vanilla = sunrise_command::vanilla::game_dispatcher();
        let commands = [mod_api::Command::new("xmode <mode:word>")];
        let arguments = native_server::command_arguments();
        let mut slots = std::collections::HashMap::new();
        let (tree, _, problems) =
            native_server::client_command_tree(&commands, &arguments, Some(&mut slots));
        assert!(problems.is_empty(), "{problems:?}");
        let mut client_dispatcher = Dispatcher::new();
        native_server::register_command_nodes(
            &mut client_dispatcher,
            &tree,
            MOD_COMMAND,
            &arguments,
            Some(&slots),
        );
        let parsed = client_dispatcher.parse("xmode fast", &reg);
        let (ctx, handler) = parsed.executable().unwrap();
        assert_eq!(handler, MOD_COMMAND);
        let key = ctx.executor.expect("client node binds an executor key");
        assert_eq!(key.slot(), 0);
        assert!(client_dispatcher.owns_executor(key));

        // Rebuilding the client dispatcher mints a fresh generation; stale keys are rejected.
        let mut rebuilt = Dispatcher::new();
        native_server::register_command_nodes(
            &mut rebuilt,
            &tree,
            MOD_COMMAND,
            &arguments,
            Some(&slots),
        );
        assert!(!rebuilt.owns_executor(key));

        let mut chat = vanilla.clone_with_unmerged_redirects();
        native_server::register_dispatcher(&mut chat, &tree, MOD_COMMAND, &arguments);
        let parse = chat.parse("xmode fast", &reg);
        assert_eq!(parse.executable().unwrap().1, MOD_COMMAND);
        assert!(
            chat.parse("xmode", &reg).executable().is_err(),
            "needs its mode"
        );
        assert!(
            chat.parse("xmode fast extra", &reg).executable().is_err(),
            "one word, not two"
        );
        // Server redirects and permissions in `base` remain intact after merging client commands.
        let tp = chat.find("tp").expect("tp alias remains");
        let tp_target = chat.nodes[tp].redirect.expect("tp redirect remains");
        assert_eq!(chat.nodes[tp_target].name(), "teleport");
        assert_eq!(chat.nodes[tp].permission, 2);
        let op_reg = Registries {
            permission: 2,
            ..Registries::default()
        };
        assert!(chat.parse("tp 0 64 0", &reg).executable().is_err());
        assert_eq!(
            chat.parse("tp 0 64 0", &op_reg).executable().unwrap().1,
            "teleport.location"
        );
    }

    #[test]
    fn display_merge_keeps_server_redirects_on_unmerged_server_targets() {
        let mut server = Dispatcher::new();
        let shared = server.literal(0, "shared");
        let remote = server.literal(shared, "server");
        server.executes(remote, "remote");
        let alias = server.literal(0, "alias");
        server.redirect(alias, shared);
        let run = server.literal(0, "run");
        server.redirect(run, 0);

        let arguments = native_server::command_arguments();
        let client_cmds = [
            mod_api::Command::new("shared local <value:integer>"),
            mod_api::Command::new("clientonly"),
        ];
        let (tree, _, problems) =
            native_server::client_command_tree(&client_cmds, &arguments, None);
        assert!(problems.is_empty(), "{problems:?}");

        let mut display = server.clone_with_unmerged_redirects();
        native_server::register_dispatcher(&mut display, &tree, MOD_COMMAND, &arguments);
        let reg = Registries::default();

        // Direct path displays both server and client branches.
        assert_eq!(
            display.parse("shared server", &reg).executable().unwrap().1,
            "remote"
        );
        assert_eq!(
            display
                .parse("shared local 7", &reg)
                .executable()
                .unwrap()
                .1,
            MOD_COMMAND
        );
        let direct_suggestions: Vec<String> = display
            .client_suggestions(&display.parse("shared ", &reg), 7, &reg)
            .1
            .into_iter()
            .map(|s| s.text)
            .collect();
        assert_eq!(direct_suggestions, ["local", "server"]);

        // Server alias to the shared root keeps the server branch and never exposes the client branch.
        assert_eq!(
            display.parse("alias server", &reg).executable().unwrap().1,
            "remote"
        );
        assert!(display.parse("alias local 7", &reg).executable().is_err());
        let alias_suggestions: Vec<String> = display
            .client_suggestions(&display.parse("alias ", &reg), 6, &reg)
            .1
            .into_iter()
            .map(|s| s.text)
            .collect();
        assert_eq!(alias_suggestions, ["server"]);

        // A server redirect to the dispatcher root traverses only the unmerged server tree
        // and preserves Brigadier's `...` smart-usage marker.
        assert_eq!(
            display
                .parse("run shared server", &reg)
                .executable()
                .unwrap()
                .1,
            "remote"
        );
        assert!(
            display
                .parse("run shared local 7", &reg)
                .executable()
                .is_err()
        );
        assert!(display.parse("run clientonly", &reg).executable().is_err());
        let run_suggestions: Vec<String> = display
            .client_suggestions(&display.parse("run ", &reg), 4, &reg)
            .1
            .into_iter()
            .map(|s| s.text)
            .collect();
        assert!(!run_suggestions.iter().any(|s| s == "clientonly"));
        let usage = display.smart_usage(0, 0);
        assert!(usage.iter().any(|(_, line)| line == "run ..."), "{usage:?}");
    }

    #[test]
    #[ignore = "daily"]
    fn studio_is_in_the_client_chat_tree_without_a_mod_registering_it() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let tree = game.chat_tree_for(None);
            let reg = Registries::default();
            assert!(tree.find("studio").is_some());
            assert_eq!(
                tree.parse("studio start", &reg).executable().unwrap().1,
                sunrise_command::MOD_COMMAND
            );
            let parse = tree.parse("stu", &reg);
            let offered = tree.client_suggestions(&parse, 3, &reg).1;
            assert!(offered.iter().any(|s| s.text == "studio"));
        });
    }

    /// A native mod's command is in the tree the chat box uses (#456): known
    /// rather than red, and offered as it types. With no mod claiming one the
    /// tree is the game's own, as it was before the mods were asked.
    #[test]
    fn native_mod_commands_join_the_chat_tree() {
        let reg = Registries::default();
        let vanilla = sunrise_command::vanilla::game_dispatcher();
        let plain = with_native_commands(vanilla, &[]);
        assert_eq!(plain.nodes.len(), vanilla.nodes.len());
        assert!(plain.find("flyspeed").is_none());
        let tree = with_native_commands(vanilla, &["flyspeed"]);
        assert!(tree.find("flyspeed").is_some());
        // The rest of the line is parsed as a greedy argument in the chat tree.
        let parse = tree.parse("flyspeed 4", &reg);
        assert_eq!(parse.executable().unwrap().1, sunrise_command::MOD_COMMAND);
        let parse = tree.parse("fly", &reg);
        let offered: Vec<String> = tree
            .client_suggestions(&parse, 3, &reg)
            .1
            .into_iter()
            .map(|s| s.text)
            .collect();
        assert!(offered.contains(&"flyspeed".to_owned()), "{offered:?}");
        // A name the game has keeps that command: the chat still knows it.
        let shared = with_native_commands(vanilla, &["help"]);
        assert!(shared.find("help").is_some());
        assert_eq!(
            shared.parse("help", &reg).executable().map(|e| e.1),
            vanilla.parse("help", &reg).executable().map(|e| e.1)
        );
        assert_eq!(shared.nodes.len(), vanilla.nodes.len());
    }

    #[test]
    fn studio_command_tree_suggests_subcommands_and_arguments() {
        let reg = Registries::default();
        let base = sunrise_command::vanilla::game_dispatcher();
        let tree = with_studio_commands(base);
        let suggestions = |input: &str| {
            let parse = tree.parse(input, &reg);
            tree.client_suggestions(&parse, input.len(), &reg)
                .1
                .into_iter()
                .map(|suggestion| suggestion.text)
                .collect::<Vec<_>>()
        };

        assert_eq!(
            suggestions("studio "),
            ["freecam", "keyframe", "path", "record", "replay"]
        );
        assert_eq!(suggestions("studio record "), ["start", "status", "stop"]);
        assert_eq!(
            suggestions("studio replay "),
            [
                "close", "open", "pause", "resume", "seek", "speed", "status"
            ]
        );
        assert_eq!(suggestions("studio freecam "), ["off", "on"]);
        assert_eq!(suggestions("studio keyframe "), ["add", "clear", "remove"]);
        assert_eq!(suggestions("studio path "), ["lock", "unlock"]);
        assert_eq!(
            tree.parse("studio replay open some-recording.mcpr", &reg)
                .executable()
                .unwrap()
                .1,
            MOD_COMMAND
        );
    }

    /// When `entity_types` is empty (no `entity-types.json`), fallback includes
    /// standard mobs, non-mob entities, and `wind_charge` / `breeze_wind_charge`.
    #[test]
    fn summonable_entities_empty_fallback_includes_mobs_and_non_mobs() {
        let mobs = ["cow", "zombie", "skeleton"];
        let summonable = summonable_entities(&[], mobs);
        assert!(summonable.contains(&"minecraft:cow".to_string()));
        assert!(summonable.contains(&"minecraft:zombie".to_string()));
        assert!(summonable.contains(&"minecraft:skeleton".to_string()));
        assert!(summonable.contains(&"minecraft:item".to_string()));
        assert!(summonable.contains(&"minecraft:arrow".to_string()));
        assert!(summonable.contains(&"minecraft:lightning_bolt".to_string()));
        assert!(summonable.contains(&"minecraft:minecart".to_string()));
        assert!(summonable.contains(&"minecraft:wind_charge".to_string()));
        assert!(summonable.contains(&"minecraft:breeze_wind_charge".to_string()));
    }

    /// Non-summonables `minecraft:player` and `minecraft:fishing_bobber` are
    /// never included in `reg.summonable`.
    #[test]
    fn summonable_entities_never_includes_player_or_fishing_bobber() {
        let all_types = vec![
            "minecraft:cow".to_string(),
            "minecraft:player".to_string(),
            "minecraft:fishing_bobber".to_string(),
            "minecraft:wind_charge".to_string(),
        ];
        let summonable = summonable_entities(&all_types, &[] as &[&str]);
        assert!(!summonable.contains(&"minecraft:player".to_string()));
        assert!(!summonable.contains(&"minecraft:fishing_bobber".to_string()));
        assert!(summonable.contains(&"minecraft:cow".to_string()));
        assert!(summonable.contains(&"minecraft:wind_charge".to_string()));

        let fallback = summonable_entities(&[], ["player", "fishing_bobber", "zombie"]);
        assert!(!fallback.contains(&"minecraft:player".to_string()));
        assert!(!fallback.contains(&"minecraft:fishing_bobber".to_string()));
        assert!(fallback.contains(&"minecraft:zombie".to_string()));
    }

    /// `minecraft:wind_charge` and `minecraft:breeze_wind_charge` are in
    /// `reg.summonable`, and `/summon wind` or `/summon minecraft:wind_charg`
    /// suggest them via `Suggests::SummonableEntities` / `resource_matches`.
    #[test]
    fn summonable_entities_suggests_wind_charge_variants() {
        let all_types = vec![
            "minecraft:wind_charge".to_string(),
            "minecraft:breeze_wind_charge".to_string(),
            "minecraft:zombie".to_string(),
        ];
        let summonable = summonable_entities(&all_types, &[] as &[&str]);
        assert!(summonable.contains(&"minecraft:wind_charge".to_string()));
        assert!(summonable.contains(&"minecraft:breeze_wind_charge".to_string()));

        assert!(sunrise_command::args::resource_matches(
            "wind",
            "minecraft:wind_charge"
        ));
        assert!(sunrise_command::args::resource_matches(
            "wind",
            "minecraft:breeze_wind_charge"
        ));
        assert!(sunrise_command::args::resource_matches(
            "minecraft:wind_charg",
            "minecraft:wind_charge"
        ));
        assert!(!sunrise_command::args::resource_matches(
            "minecraft:wind_charg",
            "minecraft:breeze_wind_charge"
        ));

        let reg = Registries {
            permission: 2,
            summonable,
            ..Default::default()
        };
        let vanilla = sunrise_command::vanilla::game_dispatcher();

        let parse = vanilla.parse("summon wind", &reg);
        let (_range, suggestions) = vanilla.client_suggestions(&parse, "summon wind".len(), &reg);
        let texts: Vec<String> = suggestions.into_iter().map(|s| s.text).collect();
        assert!(texts.contains(&"minecraft:wind_charge".to_string()));
        assert!(texts.contains(&"minecraft:breeze_wind_charge".to_string()));

        let parse = vanilla.parse("summon minecraft:wind_charg", &reg);
        let (_range, suggestions) =
            vanilla.client_suggestions(&parse, "summon minecraft:wind_charg".len(), &reg);
        let texts: Vec<String> = suggestions.into_iter().map(|s| s.text).collect();
        assert_eq!(texts, vec!["minecraft:wind_charge".to_string()]);
    }

    #[test]
    fn server_sound_commands_complete_from_builtins_without_audio() {
        let exported = Registries::from_export(&serde_json::json!({
            "by_registry": {"minecraft:sound_event": {"ids": ["test:sample_sound"]}}
        }));
        let mut reg = Registries {
            permission: 4,
            ..Default::default()
        };
        reg.fill_from_exported(&exported);
        let tree = sunrise_command::vanilla::game_dispatcher();
        for input in ["playsound test:sample", "stopsound @s master test:sample"] {
            let parse = tree.parse(input, &reg);
            let (_, options) = tree.client_suggestions(&parse, input.len(), &reg);
            assert_eq!(
                options
                    .into_iter()
                    .map(|suggestion| suggestion.text)
                    .collect::<Vec<_>>(),
                ["test:sample_sound"],
                "{input}"
            );
        }
    }

    /// `Registries::fill_from_export` and `fill_from_exported` populate
    /// `self.summonable` and `self.entity_types` when present in export JSON,
    /// and sort/dedup properly.
    #[test]
    fn export_propagation_populates_and_dedups_summonable_and_entity_types() {
        let export_json = serde_json::json!({
            "summonable": [
                "minecraft:zombie",
                "minecraft:arrow",
                "minecraft:zombie",
                "minecraft:wind_charge"
            ],
            "entity_types": [
                "minecraft:zombie",
                "minecraft:player",
                "minecraft:arrow",
                "minecraft:zombie"
            ]
        });
        let mut reg = Registries::default();
        reg.fill_from_export(&export_json);
        assert_eq!(
            reg.summonable,
            vec![
                "minecraft:arrow".to_string(),
                "minecraft:wind_charge".to_string(),
                "minecraft:zombie".to_string(),
            ]
        );
        assert_eq!(
            reg.entity_types,
            vec![
                "minecraft:arrow".to_string(),
                "minecraft:player".to_string(),
                "minecraft:zombie".to_string(),
            ]
        );

        let mut dest = Registries::default();
        dest.fill_from_exported(&reg);
        assert_eq!(dest.summonable, reg.summonable);
        assert_eq!(dest.entity_types, reg.entity_types);

        let mut prefilled = Registries {
            summonable: vec!["minecraft:pig".to_string()],
            entity_types: vec!["minecraft:pig".to_string()],
            ..Default::default()
        };
        prefilled.fill_from_exported(&reg);
        assert_eq!(prefilled.summonable, vec!["minecraft:pig".to_string()]);
        assert_eq!(prefilled.entity_types, vec!["minecraft:pig".to_string()]);
    }

    /// The sleeper's chat: Leave Bed is a button target for the pad and the
    /// keyboard, right above the chat box, and the D-pad steps between the
    /// two.
    #[test]
    fn leave_bed_is_a_focus_target_above_the_chat_box() {
        use super::super::key_focus;
        use std::collections::HashMap;
        use sunrise_client::{
            font::Font,
            gamepad::{Button, focus_step_strict},
            widgets::Kind,
        };
        // The missing-glyph metrics measure without game assets.
        let font = Font::load(
            &sunrise_assets::resource_source::ResourceSource::default(),
            &HashMap::new(),
            &[],
            (1, 1),
            (0, 0),
            sunrise_client::hud::Rect {
                u0: 0.0,
                v0: 0.0,
                u1: 1.0,
                v1: 1.0,
                w: 1,
                h: 1,
            },
        );
        let m = Measure(&font);
        let registries = Registries::default();
        let c = Commands {
            dispatcher: sunrise_command::vanilla::game_dispatcher(),
            registries: &registries,
        };
        let (width, height) = (427.0, 240.0);
        let mut screen = ChatScreen::new(
            &m,
            &c,
            "",
            false,
            0,
            width as i32,
            height as i32,
            ChatAbilities::new(ChatVisibility::Full),
        );
        screen.draw(&m, (0, 0), 0, 0);
        let (x, y, w, h) = crate::game::ClientGame::leave_bed_button(width, height);
        crate::game::ClientGame::leave_bed_draws(&m, (0, 0), (x, y, w, h));
        sunrise_client::widgets::end_frame(u64::MAX);

        let targets = key_focus::drawn_targets();
        let bed = targets
            .iter()
            .copied()
            .find(|t| t.rect == [x as f32, y as f32, w as f32, h as f32])
            .expect("Leave Bed is a focus target");
        assert_eq!(bed.kind, Kind::Button);
        assert!(key_focus::keyboard_targets(targets.clone(), false).contains(&bed));
        let field = targets
            .iter()
            .copied()
            .find(|t| t.kind == Kind::Field)
            .expect("the chat box is a focus target");
        let rects: Vec<_> = targets.iter().map(|t| t.rect).collect();
        let step =
            |from: [f32; 4], button| focus_step_strict(from, button, &rects).map(|i| targets[i]);
        assert_eq!(step(field.rect, Button::DPadUp), Some(bed));
        assert_eq!(step(bed.rect, Button::DPadDown), Some(field));
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod enchantment_snapshot_tests {}

#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod damage_type_registry_tests {}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;

#[cfg(all(test, feature = "client"))]
mod link_and_path_tests {
    use super::*;
    use crate::game::ClientGame;
    use crate::game::native_entity_refs::tests::with_client_game;
    use mod_api::{ClientHost, ClientTickPreEvent, Hooks, ModEntry, OpenLinkError, OpenPathError};
    use mod_platform::testing::with_mods;
    use std::sync::{Arc, Mutex};
    use sunrise_gameplay_common::game::execution_role::ExecutionRole;

    /// A mod that asks for a link from its client tick and keeps the answer.
    struct Asker {
        uri: &'static str,
        trusted: bool,
        answer: Arc<Mutex<Option<Result<(), OpenLinkError>>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Asker {
        fn client_tick_pre(&self, host: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
            *self.answer.lock().unwrap() = Some(host.confirm_link(self.uri, self.trusted));
        }
    }

    /// Run the mod's link request from a client tick.
    fn ask(
        uri: &'static str,
        trusted: bool,
        check: impl FnOnce(&mut ClientGame<'_>, Result<(), OpenLinkError>),
    ) {
        let answer = Arc::new(Mutex::new(None));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "linker",
            Box::new(Asker {
                uri,
                trusted,
                answer: answer.clone(),
            }),
        )]
        .into();
        with_mods(mods, || {
            with_client_game(|game| {
                game.common.options.headless = true;
                game.native_client_tick_pre();
                let result = answer.lock().unwrap().take().unwrap();
                check(game, result);
            });
        });
    }

    /// Trust changes the prompt labels, and declining returns to the same screen.
    #[test]
    #[ignore = "daily"]
    fn a_mod_asks_to_open_a_web_link() {
        ask("https://example.com/a?b=c", false, |game, result| {
            assert_eq!(result, Ok(()));
            let prompt = game.client.link_prompt.clone().expect("the prompt opens");
            assert_eq!(prompt.url, "https://example.com/a?b=c");
            assert!(!prompt.trusted);
            assert_eq!(prompt.title_key(), "chat.link.confirm");
            assert_eq!(prompt.button_keys(), ["gui.yes", "chat.copy", "gui.no"]);
            let was_playing = game.client.screen == Screen::Playing;
            game.answer_link_prompt(false);
            assert!(game.client.link_prompt.is_none());
            assert_eq!(game.client.screen == Screen::Playing, was_playing);
        });
        ask("https://example.com/a?b=c", true, |game, result| {
            assert_eq!(result, Ok(()));
            let prompt = game.client.link_prompt.clone().expect("the prompt opens");
            assert!(prompt.trusted);
            assert_eq!(prompt.title_key(), "chat.link.confirmTrusted");
            assert_eq!(
                prompt.button_keys(),
                ["chat.link.open", "chat.copy", "gui.cancel"]
            );
            game.answer_link_prompt(false);
            assert!(game.client.link_prompt.is_none());
        });
    }

    /// The mod prompt accepts HTTP(S) even when chat links are disabled.
    #[test]
    #[ignore = "daily"]
    fn a_mod_link_must_be_http_or_https() {
        with_client_game(|game| {
            game.common.options.headless = true;
            (
                game.client.settings.chat_links,
                game.client.settings.chat_links_prompt,
            ) = (false, false);
            assert_eq!(game.confirm_link("HTTPS://Example.com", false), Ok(()));
            assert!(game.client.link_prompt.is_some());
            game.answer_link_prompt(false);
            for bad in [
                "file:///etc/passwd",
                "javascript:alert(1)",
                "example.com",
                "ftp://example.com",
                "http://a b",
            ] {
                assert_eq!(
                    game.confirm_link(bad, false),
                    Err(OpenLinkError::Invalid),
                    "{bad}"
                );
                assert!(game.client.link_prompt.is_none(), "{bad}");
            }
        });
    }

    /// A path that does not exist is refused before any process starts.
    #[test]
    #[ignore = "daily"]
    fn open_path_refuses_a_missing_path() {
        with_client_game(|game| {
            game.common.options.headless = true;
            let missing = std::env::temp_dir().join("sunrise-open-path-missing/never-created");
            assert_eq!(game.open_path(&missing), Err(OpenPathError::NotFound));
        });
    }

    /// Without a native window both methods answer `Unavailable`.
    #[test]
    #[ignore = "daily"]
    fn link_and_path_are_unavailable_without_a_window() {
        for role in [
            ExecutionRole::IntegratedServer,
            ExecutionRole::DedicatedServer,
        ] {
            with_client_game(|game| {
                game.common.options.execution_role = role;
                assert_eq!(
                    game.confirm_link("https://example.com", false),
                    Err(OpenLinkError::Unavailable)
                );
                assert!(game.client.link_prompt.is_none());
                assert_eq!(
                    game.open_path(Path::new(".")),
                    Err(OpenPathError::Unavailable)
                );
            });
        }
    }
}
