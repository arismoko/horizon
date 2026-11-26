//! Chat, the command registries and the per-world state behind commands
//! (game rules, the world clock, difficulty), bound to the game.
#[cfg(test)]
#[path = "chat/submit_order_tests.rs"]
mod submit_order_tests;
#[cfg(test)]
#[path = "chat/suggestion_completion_tests.rs"]
mod suggestion_completion_tests;

use super::{Game, Screen};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use sunrise_client::{
    chat::{
        Chat, ChatAbilities, ChatAction, ChatScreen, ChatSource, ChatVisibility, Commands,
        QUEUE_EXPAND_ID, SYSTEM_INDICATOR, normalize_chat_message,
    },
    command::{ArgType, Dispatcher, MOD_COMMAND, Registries},
    gamerules::GameRules,
    hud::HudBatch,
    screen_draw::Measure,
    text::{ClickEvent, Style, Text},
};
use sunrise_world::registry::{BlockRegistry, BlockStateId};

/// The tree the chat parses and completes with: `Remote::chat_tree`, which is
/// a server's, once it has sent one (ClientPacketListener.commands), else the
/// game's own, plus the commands native mods own (`chat_tree_for`).
pub(super) fn chat_tree(
    remote: &Option<super::remote::Remote>,
) -> &sunrise_client::command::Dispatcher {
    match remote.as_ref() {
        Some(server) => &server.chat_tree,
        None => sunrise_client::command::vanilla::game_dispatcher(),
    }
}
/// The arguments of a typed `/profiler` the client answers itself: only when the chat tree's `profiler` is Sunrise's, else the server or mod that owns the name runs it.
fn sunrise_profiler_args<'a>(
    message: &'a str,
    server_tree: Option<&sunrise_client::command::Dispatcher>,
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
pub(super) fn with_native_commands(
    base: &sunrise_client::command::Dispatcher,
    names: &[&str],
) -> sunrise_client::command::Dispatcher {
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
fn with_studio_commands(base: &Dispatcher) -> Dispatcher {
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

impl Game<'_> {
    /// The tree the chat box colours, completes and parses with for the
    /// world's connection: `base`, the tree the server sent, or the game's own
    /// before it does. Built where that base changes — once per connection —
    /// rather than at each keystroke, and asking the mods then, not per key.
    pub(super) fn chat_tree_for(
        &mut self,
        base: Option<&sunrise_client::command::Dispatcher>,
    ) -> sunrise_client::command::Dispatcher {
        let base = match base {
            Some(tree) => tree,
            None => sunrise_client::command::vanilla::game_dispatcher(),
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
            let executor = super::native_server::RegisteredClientExecutor::new(
                owner,
                index,
                &command.usage,
                call,
            );
            usages.push(command);
            executors.push(Some(executor));
        }
        let arguments = super::native_server::command_arguments();
        let mut slots = super::native_server::CommandExecutorSlots::new();
        let (tree, _, problems) =
            super::native_server::client_command_tree(&usages, &arguments, Some(&mut slots));
        for problem in problems {
            log::warn!("Sunrise: a client command was left out: {problem}");
        }
        let mut client_dispatcher = Dispatcher::new();
        super::native_server::register_command_nodes(
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
        self.native_client_commands
            .install(client_dispatcher, executors);
        let mut tree_with_mods = base.clone_with_unmerged_redirects();
        super::native_server::register_dispatcher(
            &mut tree_with_mods,
            &tree,
            MOD_COMMAND,
            &arguments,
        );
        with_studio_commands(&with_native_commands(&tree_with_mods, &["studio"])).with_profiler()
    }
}

/// The local player's name (offline single player).
pub const PLAYER_NAME: &str = "Player";

pub(in crate::game) use sunrise_gameplay::world_clock::OVERWORLD_CLOCK;
pub use sunrise_gameplay::world_clock::{TimeMarker, WorldClock};
/// `ChatComponent.Draft`: the text the chat box kept, and whether it was a command.
pub struct ChatDraft {
    pub text: String,
    pub command: bool,
}
/// `ChatScreen.ExitReason`: why the chat screen went away.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::game) enum ChatExit {
    /// Another screen replaced it (death, a container, a disconnect).
    Interrupted,
    /// The player closed it (Escape).
    Intentional,
    /// The message was sent or transferred into the replacement chat screen.
    Done,
}
/// `ChatScreen.shouldDiscardDraft`.
fn should_discard_draft(reason: ChatExit, save_chat_drafts: bool) -> bool {
    reason != ChatExit::Interrupted && (reason != ChatExit::Intentional || !save_chat_drafts)
}
/// An owned command tree, initialized even before a world is opened.
pub struct SessionDispatcher(pub Arc<sunrise_client::command::Dispatcher>);
impl Default for SessionDispatcher {
    fn default() -> Self {
        let registry = sunrise_client::gamerules::RuleRegistry::vanilla();
        Self(Arc::new(
            sunrise_client::command::vanilla::session_dispatcher(&registry, &[]),
        ))
    }
}
/// UI and command state that lives beside the game.
#[derive(Default)]
pub struct UiState {
    pub chat: Chat,
    pub chat_screen: Option<ChatScreen>,
    /// `ChatComponent.latestDraft`.
    pub chat_draft: Option<ChatDraft>,
    /// When the chat screen opened (its input took the focus: the cursor blink).
    pub chat_opened: Option<std::time::Instant>,
    /// The chat screen is `InBedChatScreen`: the sleeper's, with a Leave Bed button.
    pub in_bed_chat: bool,
    pub registries: Registries,
    pub rules: GameRules,
    /// The server session command tree.
    pub dispatcher: SessionDispatcher,
    pub clock: WorldClock,
    /// The totals and state of the clocks the world has met, the Overworld's total among them
    /// while another clock is the one in `active_clock`. The active clock's total is
    /// World.day_time; the Overworld's state stays in `clock`.
    pub command_clocks: BTreeMap<String, (i64, WorldClock)>,
    /// The clock `World.day_time` follows: the default clock of the level in place, the
    /// Overworld's while empty. The totals of the others are in `command_clocks`.
    pub active_clock: String,
    pub end_clock: WorldClock,
    /// Time markers by `(clock_id, marker_id)` and timelines by `timeline_id -> (clock_id, period)`
    /// from the world's `minecraft:timeline` registry.
    pub markers: BTreeMap<(String, String), TimeMarker>,
    pub timelines: BTreeMap<String, (String, Option<i64>)>,
    /// 0 peaceful .. 3 hard.
    pub difficulty: u8,
    /// Effective HUD visibility, which native mods can change.
    pub hide_gui: bool,
    /// The player's F1 preference, independent of native HUD mode changes.
    pub(super) player_hides_gui: bool,
    /// Native HUD mode: keep hands visible while the HUD is hidden.
    pub show_hand_when_hidden: bool,
    /// Death message key set by the cause of death (e.g. /kill).
    pub death_cause: Option<String>,
    /// What this frame's HUD shows that Sunrise's overlays keep clear of (`Game::hud_room`),
    /// set by `hud_layers` before anything draws.
    pub room: sunrise_client::hud_kit::room::Room,
    /// F2 screenshots not written yet: each reports in chat once its PNG is.
    pub screenshot_messages: Vec<std::path::PathBuf>,
    pub screenshot_writes: Vec<super::screenshot_writer::ScreenshotWrite>,
    /// The options screens.
    pub options: sunrise_client::options_ui::OptionsUi,
    /// The prepared resources before resource packs are layered on.
    pub vanilla_root: std::path::PathBuf,
    /// Death screen cause and when the player died (buttons wait 20 ticks).
    pub death_text: Option<Text>,
    pub died_at: Option<std::time::Instant>,
    /// `DeathScreen.TitleConfirmScreen` and when it opened (its buttons wait
    /// 20 ticks, `setDelay`).
    pub death_confirm: Option<std::time::Instant>,
    /// `LivingEntity.deathTime` of the window's player: ticks since it died.
    pub death_time: u32,
}

impl UiState {
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

    pub fn hand_hidden(&self) -> bool {
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

    fn toggle_hide_gui(&mut self) {
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

#[cfg(test)]
mod hud_mode_tests {
    use super::UiState;
    use mod_api::HudMode;

    #[test]
    fn hud_modes_drive_the_effective_hud_and_hand_gate() {
        let mut ui = UiState::default();
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
        let mut ui = UiState::default();
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
        let ui = UiState {
            hide_gui: true,
            ..Default::default()
        };
        assert_eq!(ui.hud_mode(), HudMode::AllHidden);
        assert!(ui.hand_hidden());
    }
}

/// The ids `build_registries` lists from the prepared resources' folders.
/// The folders don't change while the game runs, so each root is read once
/// instead of at every world opening (thousands of files).
struct ResourceIds {
    recipes: Vec<String>,
    templates: Vec<String>,
    /// `entity-types.json`'s keys, sorted (empty when the file is missing).
    entity_types: Vec<String>,
    enchantments: Vec<String>,
    structures: Vec<String>,
    biomes: Vec<String>,
}
static RESOURCE_IDS: Mutex<Vec<(PathBuf, Arc<ResourceIds>)>> = Mutex::new(Vec::new());

/// The `minecraft:` ids of the `.json` files directly in `dir`, sorted.
fn json_ids_in(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_str()?
                .strip_suffix(".json")
                .map(|n| format!("minecraft:{n}"))
        })
        .collect();
    out.sort();
    out
}

fn resource_ids(root: &Path) -> Arc<ResourceIds> {
    let mut cache = RESOURCE_IDS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, ids)) = cache.iter().find(|(cached, _)| cached == root) {
        return ids.clone();
    }
    let data = root.join("data/minecraft");
    let entity_types = super::entity_type_rows::entity_type_rows(root).ids.clone();
    let ids = Arc::new(ResourceIds {
        // `/recipe <id>` must suggest a content mod's recipes too.
        recipes: json_ids_of_every_namespace(root, "recipe"),
        templates: templates_under(&root.join("worldgen/structure")),
        entity_types,
        // A content mod's enchantments too (`/enchant`, `/give` components).
        enchantments: json_ids_of_every_namespace(root, "enchantment"),
        structures: json_ids_in(&data.join("worldgen/structure")),
        biomes: json_ids_in(&data.join("worldgen/biome")),
    });
    cache.push((root.to_owned(), ids.clone()));
    ids
}

/// Every block's properties and their values, as `/setblock` completes them.
pub(super) type BlockProperties = BTreeMap<String, Vec<(String, Vec<String>)>>;
static BLOCK_PROPERTIES: Mutex<Vec<(usize, Arc<BlockProperties>)>> = Mutex::new(Vec::new());

/// `BlockProperties` of a block registry, listed once: a registry never
/// changes once built, and walking its tens of thousands of states took
/// about 30 ms of every world opening.
pub(super) fn block_properties(registry: &BlockRegistry) -> Arc<BlockProperties> {
    // The payload is retained for the process, so equal data shares one table whatever borrows it.
    let key = registry.payload_identity();
    let mut cache = BLOCK_PROPERTIES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, properties)) = cache.iter().find(|(cached, _)| *cached == key) {
        return properties.clone();
    }
    let mut blocks: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();
    for id in 0..registry.len() {
        let Some(state) = registry.state(BlockStateId(id as u32)) else {
            continue;
        };
        let props = blocks.entry(state.name.clone()).or_default();
        for (k, v) in &state.properties {
            let values = props.entry(k.clone()).or_default();
            if !values.contains(v) {
                values.push(v.clone());
            }
        }
    }
    let properties: Arc<BlockProperties> = Arc::new(
        blocks
            .into_iter()
            .map(|(k, v)| (k, v.into_iter().collect()))
            .collect(),
    );
    cache.push((key, properties.clone()));
    properties
}

static COMMAND_EXPORTS: Mutex<Vec<(PathBuf, Option<Arc<Registries>>)>> = Mutex::new(Vec::new());

/// What the registries take from a folder's `command-registries.json`
/// (vanilla's built-in registries), read once like the resource ids: parsing
/// its 1.6 MB again took about 15 ms of every world opening. Only the part
/// the registries fill from stays, not the parsed export (4.2 MB, against
/// 2.2 MB). A dedicated server opens its world once and keeps nothing.
pub(super) fn command_export(dir: &Path, keep: bool) -> Option<Arc<Registries>> {
    let mut cache = COMMAND_EXPORTS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, export)) = cache.iter().find(|(cached, _)| cached == dir) {
        return export.clone();
    }
    let export = std::fs::read(dir.join("command-registries.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .map(|export| Arc::new(Registries::from_export(&export)));
    if keep {
        cache.push((dir.to_owned(), export.clone()));
    }
    export
}

/// The ids of the `.json` files in `data/<namespace>/<kind>` of every
/// namespace of `root` (vanilla's and the content mods'), sorted.
fn json_ids_of_every_namespace(root: &Path, kind: &str) -> Vec<String> {
    let data = root.join("data");
    let mut namespaces: std::collections::BTreeSet<String> = std::fs::read_dir(&data)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    // A prepared root's namespaces live in its data file, not on the disk.
    if let Some(files) = sunrise_core::prepared_files::files_under(&data) {
        namespaces.extend(
            files
                .iter()
                .filter_map(|f| Some(f.split_once('/')?.0.to_owned())),
        );
    }
    let mut out: Vec<String> = namespaces
        .iter()
        .flat_map(|ns| ids_in(&data.join(ns).join(kind), "json", ns))
        .collect();
    out.sort();
    out
}

/// The templates of every namespace folder under `root` (vanilla's and the
/// content mods'), as `namespace:path`, sorted.
fn templates_under(root: &Path) -> Vec<String> {
    let Ok(namespaces) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<String> = namespaces
        .flatten()
        .filter(|e| e.path().is_dir())
        .flat_map(|e| ids_in(&e.path(), "nbt", &e.file_name().to_string_lossy()))
        .collect();
    out.sort();
    out
}

/// The `namespace:` ids of the `extension` files under `root`, sorted.
fn ids_in(root: &Path, extension: &str, namespace: &str) -> Vec<String> {
    let suffix = format!(".{extension}");
    // A prepared root's folder: from its data file, without listing the disk.
    if let Some(files) = sunrise_core::prepared_files::files_under(root) {
        let mut out: Vec<String> = files
            .iter()
            .filter_map(|file| file.strip_suffix(&suffix))
            .filter(|path| !path.is_empty() && !path.ends_with('/'))
            .map(|path| format!("{namespace}:{path}"))
            .collect();
        out.sort();
        return out;
    }
    let mut out = Vec::new();
    let mut stack = vec![root.to_owned()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            // The listing's own type, without another look at each file; a
            // link is followed, as `Path::is_dir` does.
            let is_dir = e
                .file_type()
                .is_ok_and(|t| t.is_dir() || (t.is_symlink() && path.is_dir()));
            if is_dir {
                stack.push(path);
            } else if path.extension().and_then(|x| x.to_str()) == Some(extension)
                && let Ok(rel) = path.strip_prefix(root)
            {
                out.push(format!(
                    "{namespace}:{}",
                    rel.with_extension("").to_string_lossy().replace('\\', "/")
                ));
            }
        }
    }
    out.sort();
    out
}

/// `SuggestionProviders.SUMMONABLE_ENTITIES`: Entity types available for
/// `/summon` autocomplete, filtering out types that cannot be summoned.
pub(super) fn summonable_entities(
    entity_types: &[String],
    mob_types: impl IntoIterator<Item = impl AsRef<str>>,
) -> Vec<String> {
    let mut summonable: Vec<String> = if !entity_types.is_empty() {
        entity_types
            .iter()
            .filter(|id| !matches!(id.as_str(), "minecraft:player" | "minecraft:fishing_bobber"))
            .cloned()
            .collect()
    } else {
        let mut list: Vec<String> = mob_types
            .into_iter()
            .map(|k| sunrise_client::command::args::normalize(k.as_ref()))
            .filter(|id| !matches!(id.as_str(), "minecraft:player" | "minecraft:fishing_bobber"))
            .collect();
        list.extend(
            [
                "minecraft:item",
                "minecraft:experience_orb",
                "minecraft:falling_block",
                "minecraft:arrow",
                "minecraft:lightning_bolt",
                "minecraft:minecart",
                "minecraft:chest_minecart",
                "minecraft:hopper_minecart",
                "minecraft:furnace_minecart",
                "minecraft:tnt_minecart",
                "minecraft:wind_charge",
                "minecraft:breeze_wind_charge",
            ]
            .map(String::from),
        );
        list
    };
    // content mods' thrown types.
    let thrown = sunrise_client::projectiles::thrown_types().iter();
    summonable.extend(thrown.map(|t| t.id.to_owned()));
    summonable.sort();
    summonable.dedup();
    summonable
}

impl Game<'_> {
    /// The caches `build_registries` and `join_server` read (`resource_ids`,
    /// `block_properties`, `command_export`, `builtin_registries`), filled on a helper
    /// thread while the title screen is up: the first world opened or server joined
    /// built them in its frame (20 ms of the Join click on an M4 Pro). Whoever needs one
    /// before it is done waits for it, as before.
    pub(super) fn preload_registries(&self) {
        if self.options.dedicated.is_some() {
            return;
        }
        let dirs = [self.options.resources.clone(), self.ui.vanilla_root.clone()];
        let registry = self.pack.assets.registry;
        let spawned = std::thread::Builder::new()
            .name("Registries preload".into())
            .spawn(move || {
                sunrise_threads::behind_server();
                super::remote::builtin_registries(&dirs[0]);
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
    /// `PlayerList.isOp` for the single-player owner: the world's Allow Commands.
    pub(super) fn allow_commands(&self) -> bool {
        self.level.allow_commands == Some(true)
    }
    /// `IntegratedServer.setWorldAllowCommands`, then `updatePermissions`.
    pub(super) fn set_allow_commands(&mut self, allow: bool) {
        self.level.allow_commands = Some(allow);
        self.ui.registries.permission = owner_permission(allow);
        if let Some(host) = self.lan.as_mut() {
            host.hub.allow_commands = allow;
        }
    }
    /// Commands the game issues itself (the server console, harnesses, the
    /// control socket) run at the owner's level whatever Allow Commands says.
    pub(super) fn as_console<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let level = std::mem::replace(&mut self.ui.registries.permission, 4);
        let out = f(self);
        self.ui.registries.permission = level;
        out
    }
    /// `ResourceOrIdArgument`'s loot tables are the server's: every table the
    /// world plays with, the game's, the content mods' and its packs'.
    pub(in crate::game) fn sync_loot_table_ids(&mut self) {
        let mut ids: Vec<String> = self.items().loot_table_ids().map(str::to_owned).collect();
        ids.sort();
        if let Some(entry) = self
            .ui
            .registries
            .by_registry
            .get_mut("minecraft:loot_table")
        {
            entry.ids.clone_from(&ids);
        }
        self.ui.registries.loot_tables = ids;
    }
    /// Registry contents for argument parsing (built when a world opens).
    pub(super) fn build_registries(&mut self) {
        let permission = owner_permission(self.allow_commands());
        let reg = &mut self.ui.registries;
        reg.permission = permission;
        let ids = resource_ids(&self.options.resources);
        self.ui.registries.recipes = ids.recipes.clone();
        self.ui.registries.damage_types = self
            .health_resources
            .damage
            .entries()
            .map(|(_, d)| d.name.clone())
            .collect();
        self.sync_loot_table_ids();
        // ResourceOrIdArgument decodes an inline feature while the command is
        // parsed, with the codec and blocks placement uses (the world's live features,
        // installed while it was prepared, before its registries are built).
        self.ui.registries.inline_feature = self.res.behaviours().live_features().map(|features| {
            sunrise_client_core::registries::InlineFeatureCheck::new(move |tag| {
                let json = sunrise_client::scoreboard::tag_to_json(tag).to_string();
                features
                    .decode_inline(&json)
                    .map(drop)
                    .map_err(|e| e.to_string())
            })
        });
        self.ui.registries.templates = ids.templates.clone();
        self.ui.registries.sounds = self.audio.event_names();
        self.ui.registries.attributes = [
            "armor",
            "armor_toughness",
            "attack_damage",
            "attack_knockback",
            "attack_speed",
            "block_break_speed",
            "block_interaction_range",
            "burning_time",
            "camera_distance",
            "explosion_knockback_resistance",
            "entity_interaction_range",
            "fall_damage_multiplier",
            "flying_speed",
            "follow_range",
            "gravity",
            "jump_strength",
            "knockback_resistance",
            "luck",
            "max_absorption",
            "max_health",
            "mining_efficiency",
            "movement_efficiency",
            "movement_speed",
            "oxygen_bonus",
            "safe_fall_distance",
            "scale",
            "sneaking_speed",
            "spawn_reinforcements",
            "step_height",
            "submerged_mining_speed",
            "sweeping_damage_ratio",
            "tempt_range",
            "water_movement_efficiency",
        ]
        .iter()
        .map(|a| format!("minecraft:{a}"))
        .collect();
        let reg = &mut self.ui.registries;
        let mut items: Vec<String> = self
            .res
            .items()
            .order
            .iter()
            .map(|s| s.to_string())
            .collect();
        items.sort();
        reg.items = items;
        reg.item_tags = self
            .res
            .items()
            .item_tags
            .keys()
            .map(|k| sunrise_client::command::args::normalize(k))
            .collect();
        reg.item_tags.sort();
        reg.block_tags = self
            .res
            .items()
            .block_tags
            .keys()
            .map(|k| sunrise_client::command::args::normalize(k))
            .collect();
        reg.block_tags.sort();
        reg.blocks = BlockProperties::clone(&block_properties(self.pack.assets.registry));
        reg.summonable = summonable_entities(&ids.entity_types, self.res.mob_types().keys());
        reg.entity_types.clone_from(&ids.entity_types);
        // The mob effect registry: every effect `/effect` and the
        // `mob_effect` resource arguments accept.
        reg.effects = sunrise_client::brewing::all_effects()
            .map(|e| sunrise_client::brewing::namespaced(e.id).to_owned())
            .collect();
        reg.effects.sort();
        reg.enchantments = ids.enchantments.clone();
        reg.structures = ids.structures.clone();
        reg.biomes = ids.biomes.clone();
        // The mods' clocks (`register_world_clock`) for `/time of`; the world's own
        // join them when it opens (`sync_timeline_registries`).
        reg.clocks = Vec::new();
        reg.clocks
            .extend(mod_api::__private::world_clock_declarations());
        reg.clocks.sort();
        reg.dimensions = vec![
            "minecraft:overworld".into(),
            "minecraft:the_nether".into(),
            "minecraft:the_end".into(),
        ];
        reg.players = vec![PLAYER_NAME.into()];
        // Vanilla's built-in registries (mob effects, components, dialogs...).
        for dir in [&self.options.resources, &self.ui.vanilla_root] {
            if let Some(export) = command_export(dir, self.options.dedicated.is_none()) {
                self.ui.registries.fill_from_exported(&export);
                break;
            }
        }
        let reg = &mut self.ui.registries;
        let particles: Vec<_> = mod_api::__private::particle_ids()
            .map(str::to_owned)
            .collect();
        if !particles.is_empty() {
            reg.by_registry
                .entry("minecraft:particle_type".into())
                .or_default()
                .ids = particles;
            reg.particle_options = Some(|name, fields| {
                if let Some(result) = mod_api::__private::decode_particle_map(name, fields) {
                    return Some(result.map(mod_api::ParticleOptions::from_typed));
                }
                let key = mod_api::__private::particle_named_key(name)?;
                (mod_api::__private::particle_kind(key)
                    == Some(mod_api::__private::ParticleKind::Custom))
                .then(|| Ok(mod_api::ParticleOptions::simple(key)))
            });
        }
        if reg.entity_types.is_empty() {
            reg.entity_types.clone_from(&reg.summonable);
            reg.entity_types.push("minecraft:player".into());
        }
        reg.entity_types.extend(
            super::general::general_type_catalog()
                .keys()
                .map(str::to_owned),
        );
        reg.entity_types.sort();
        reg.entity_types.dedup();
        // Selector validation prefers the exported registry entry. Replace its
        // vanilla IDs with the live list, keeping its tags.
        if let Some(entry) = reg.by_registry.get_mut("minecraft:entity_type") {
            entry.ids.clone_from(&reg.entity_types);
        }
        // Native component names were validated and frozen before resources loaded.
        // The same inventory supplies both wire IDs and command components.
        for (_, name) in sunrise_protocol::item::custom::entries() {
            if !reg.persistent_components.iter().any(|entry| entry == name) {
                reg.persistent_components.push(name.to_owned());
            }
        }
        self.sync_timeline_registries();
        self.sync_enchantment_registries();
        // After the export, which keys the registry with vanilla's tags.
        self.sync_stat_registries();
        self.sync_pack_tag_ids();
        self.sync_test_registry_ids();
    }
    /// The block the player is looking at, for coordinate suggestions, and
    /// the entity, for entity arguments (ClientSuggestionProvider).
    /// `ClientSuggestionProvider.getRelevantCoordinates` and
    /// `getAbsoluteCoordinates` read `Minecraft.hitResult` only when it is a
    /// block: an entity in front of the block leaves the `~` defaults.
    fn refresh_dynamic_registries(&mut self) {
        let hit = if self.pick_entity_hit().is_some() {
            None
        } else {
            self.pick()
        };
        self.ui.registries.target_block = hit.as_ref().map(|h| h.pos);
        self.ui.registries.target_point =
            hit.map(|h| h.point.map(sunrise_core::java_number::format_fixed2));
        self.ui.registries.selected_entities = self.crosshair_entity_uuid().into_iter().collect();
        // ClientPacketListener.getOnlinePlayerNames on a server.
        if let Some(names) = self.remote_online_names() {
            self.ui.registries.players = names;
        }
        // ClientPacketListener.levels(): the level keys of the login packet,
        // data-pack and mod dimensions included.
        if let Some(levels) = self
            .remote
            .as_ref()
            .map(|r| r.levels.clone())
            .filter(|l| !l.is_empty())
        {
            self.ui.registries.dimensions = levels;
        }
        self.ui.registries.custom_completions = self.remote_custom_completions();
        // The sender a mod's own argument completion answers for: this player, whose permission
        // level is `registries.permission`. A server completion sets it from the guest asking
        // (`guest_suggestions`); the console has none.
        self.ui.registries.sender = self
            .remote
            .as_ref()
            .filter(|remote| remote.logged_in && remote.profile != 0)
            .map(|remote| sunrise_client_core::registries::CommandSender {
                uuid: remote.profile,
                name: remote.profile_name.clone(),
            });
    }
    /// `ChatComponent.createScreen`: T or / opens the chat with the draft kept
    /// last time, when this method can restore it (a command draft only for
    /// /), else with its prefix.
    pub(super) fn open_chat_method(&mut self, prefix: &str) {
        let command = prefix == "/";
        let draft = self
            .ui
            .chat_draft
            .as_ref()
            .filter(|d| !command || d.command)
            .map(|d| d.text.clone());
        self.begin_chat_screen(draft.as_deref().unwrap_or(prefix), draft.is_some());
    }
    /// Open the chat box on `initial`; `is_draft` when that text is the saved
    /// draft (`ChatComponent.createScreen`), which draws grey italic until it
    /// is edited.
    /// Returns false without a local player. Otherwise the ordinary screen lifecycle
    /// can open, cancel, or replace the requested screen.
    pub(super) fn begin_chat_screen(&mut self, initial: &str, is_draft: bool) -> bool {
        self.replace_chat_screen(initial, is_draft, ChatExit::Interrupted)
    }

    fn replace_chat_screen(&mut self, initial: &str, is_draft: bool, reason: ChatExit) -> bool {
        if self.player.is_none() {
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
                .renderer
                .as_ref()
                .map_or([1280, 720], |r| [r.config.width, r.config.height]);
            let b = HudBatch::new(size, game.settings.gui_scale_inputs());
            let m = Measure(&game.gui.atlas.font);
            let c = Commands {
                dispatcher: chat_tree(&game.remote),
                registries: &game.ui.registries,
            };
            let mut screen = ChatScreen::new(
                &m,
                &c,
                initial,
                is_draft,
                game.ui.chat.recent.len(),
                b.width as i32,
                b.height as i32,
                game.chat_abilities(),
            );
            screen.auto_suggestions = game.settings.auto_suggestions;
            game.ui.chat_screen = Some(screen);
            game.ui.chat_opened = Some(std::time::Instant::now());
            game.capture(false);
        });
        true
    }
    pub(in crate::game) fn close_chat(&mut self, reason: ChatExit) {
        // InBedChatScreen.onClose: leaving the screen means getting up; the
        // screen goes once the player has woken (`tick_in_bed_chat`).
        if self.ui.in_bed_chat && self.sleeping.is_some() {
            self.wake_up(false, true);
            return;
        }
        if self.screen == Screen::Chat {
            // `Gui.setScreen(null)`: Closing fires, no vetoable Opening does.
            self.dismiss_screen_with(|game| game.remove_chat_screen(reason));
            if self.screen == Screen::Playing {
                self.capture(true);
            }
        } else {
            self.remove_chat_screen(reason);
        }
    }
    /// `ChatScreen.removed`, called only after the screen transition is accepted.
    pub(in crate::game) fn remove_chat_screen(&mut self, reason: ChatExit) {
        if let Some(screen) = self.ui.chat_screen.take() {
            self.keep_chat_draft(screen.value(), screen.is_draft(), reason);
            self.ui.chat_opened = None;
            self.ui.chat.reset_scroll();
        }
        self.ui.in_bed_chat = false;
    }
    /// `ChatScreen.removed`: the text left in the box becomes the draft unless
    /// `shouldDiscardDraft` (closed on purpose without Save Chat Drafts, or
    /// sent); blank text discards the draft. A draft still shown unchanged
    /// stays as it is.
    fn keep_chat_draft(&mut self, text: &str, is_draft: bool, reason: ChatExit) {
        // During PvP & HUD's chat search the box holds the query: the draft is the text the
        // search put aside, whichever way the chat closed.
        let aside = self.pvp.chat_search_aside.take();
        let (text, is_draft) = aside.as_ref().map_or((text, is_draft), |(text, is_draft)| {
            (text.as_str(), *is_draft)
        });
        if should_discard_draft(reason, self.settings.file.bool("saveChatDrafts"))
            || text.trim().is_empty()
        {
            self.ui.chat_draft = None;
        } else if !is_draft {
            self.ui.chat_draft = Some(ChatDraft {
                text: text.to_owned(),
                command: text.starts_with('/'),
            });
        }
    }
    /// `Gui.tick`: a sleeper gets `InBedChatScreen`; when the player has
    /// woken (`onPlayerWokeUp`) it closes, or turns into the plain chat
    /// screen if something was typed.
    pub(super) fn tick_in_bed_chat(&mut self) {
        if self.screen != Screen::Chat {
            self.remove_chat_screen(ChatExit::Interrupted);
        }
        if self.ui.in_bed_chat && self.sleeping.is_none() && !self.screen_over_playing() {
            // InBedChatScreen.onPlayerWokeUp: replace once, discarding the
            // outgoing draft only when non-draft typed text transfers into
            // ordinary chat; empty or untouched-draft wake dismisses via
            // `Gui.setScreen(null)` (Closing without a vetoable Opening).
            let typed = self.ui.chat_screen.as_ref().and_then(|screen| {
                let (text, is_draft) = self
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
                self.ui.in_bed_chat = false;
            } else {
                self.dismiss_screen_with(|game| game.remove_chat_screen(ChatExit::Interrupted));
                if self.screen == Screen::Playing {
                    self.capture(true);
                }
            }
        } else if self.sleeping.is_some()
            && self.screen == Screen::Playing
            && !self.screen_over_playing()
        {
            let draft = self.ui.chat_draft.as_ref().map(|draft| draft.text.clone());
            self.begin_chat_screen(draft.as_deref().unwrap_or(""), draft.is_some());
            self.ui.in_bed_chat = self.screen == Screen::Chat;
        }
    }
    /// The Leave Bed button of `InBedChatScreen`: its bounds, laid out once
    /// per build (`ChatLayout`).
    pub(super) fn leave_bed_button(width: f32, height: f32) -> (i32, i32, i32, i32) {
        (width as i32 / 2 - 100, height as i32 - 40, 200, 20)
    }
    /// The Leave Bed button's draws. It registers as a button
    /// (`widgets::record`), so the pad's and the keyboard's focus reach it (the
    /// pad's starts on it) and `sunrise-ctl click-button` finds it.
    fn leave_bed_draws(
        m: &Measure,
        mouse: (i32, i32),
        (x, y, w, h): (i32, i32, i32, i32),
    ) -> Vec<sunrise_client::screen_draw::Draw> {
        let label = Text::literal(sunrise_client::lang::get_or(
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
    /// `Minecraft.computeChatAbilities` from the Chat option.
    pub(super) fn chat_abilities(&self) -> ChatAbilities {
        ChatAbilities::new(ChatVisibility::from_option(self.settings.chat_visibility))
    }
    /// A chat message from the game (ChatComponent.addServerSystemMessage).
    pub(super) fn system_message(&mut self, text: Text) {
        if let Some(capture) = self.chat_capture.as_mut() {
            capture.push(text);
            return;
        }
        // Connected to a server (single player's own included), the chat on
        // screen is the connection's: the game's own messages join it.
        if self.remote.is_some() {
            self.add_chat_line(text, ChatSource::Server);
            return;
        }
        // The Chat option's Hidden level takes the server's messages too.
        self.local_chat_line(text, Some(SYSTEM_INDICATOR), ChatSource::Server);
    }
    /// `ChatComponent.MAX_CHAT_HISTORY`, or the cap a mod set.
    pub(super) fn chat_history_limit(&self) -> u32 {
        u32::try_from(self.ui.chat.history_limit()).unwrap_or(u32::MAX)
    }
    /// A mod's chat history cap, clamped as [`sunrise_client::chat::Chat::set_history_limit`] says.
    pub(super) fn set_chat_history_limit(&mut self, limit: u32) {
        self.ui
            .chat
            .set_history_limit(usize::try_from(limit).unwrap_or(usize::MAX));
    }
    /// `ChatComponent.addClientSystemMessage`: a line the client adds itself.
    /// It joins the connection's chat and its history, with the system
    /// marker; the Chat option's server-message filter does not hide it
    /// (`GuiMessageSource.SYSTEM_CLIENT`). Vanilla fires
    /// `ClientChatReceivedEvent` from `ChatListener` for the lines the server
    /// sent, so this path does not tell `client_chat_received`.
    pub(super) fn chat_line(&mut self, text: Text) {
        if self.remote.is_some() {
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
    pub(super) fn chat_line_tagged(&mut self, tag: &str, text: Text) {
        let Some(owner) = mod_api::content::crash::current_native_mod() else {
            self.chat_line(text);
            return;
        };
        let ticks = self.visual_ticks();
        let focused = self.screen == Screen::Chat;
        let plain = text.plain();
        if self
            .ui
            .chat
            .replace_keyed(&self.gui.atlas, owner, tag, text.clone(), ticks, focused)
        {
            self.record_received_chat(plain);
            return;
        }
        self.ui.chat.set_next_key(owner, tag);
        self.chat_line(text);
        self.ui.chat.clear_next_key();
    }
    /// Removes the calling mod's lines kept under `tag` from the chat and its history; how many
    /// went. Another mod's lines are never touched.
    pub(super) fn chat_remove_tagged(&mut self, tag: &str) -> usize {
        let Some(owner) = mod_api::content::crash::current_native_mod() else {
            return 0;
        };
        self.ui.chat.remove_keyed(&self.gui.atlas, owner, tag)
    }
    /// A player's chat line on the window's own chat: a guest's message on
    /// the server's own chat. `ChatListener`
    /// checks `canReceivePlayerMessages` for these, so Commands Only and
    /// Hidden drop them; command feedback (`/list`, `/tp`) is a system
    /// message and takes the system-message permission instead.
    pub(super) fn player_message(&mut self, text: Text) {
        // A server's client receives the line as a packet (PlayerChat or
        // DisguisedChat) and filters it there.
        if self.remote.is_none() {
            self.local_chat_line(text, None, ChatSource::Player);
        }
    }
    /// A line on the window's own chat when the Chat option lets `source`
    /// through.
    fn local_chat_line(&mut self, text: Text, indicator: Option<u32>, source: ChatSource) {
        if !self.ui.chat.shows(source) {
            return;
        }
        let focused = self.screen == Screen::Chat;
        self.ui.chat.add(
            &self.gui.atlas,
            text,
            indicator,
            source,
            self.visual_ticks(),
            focused,
        );
    }
    /// Keys while the chat screen is open.
    pub(super) fn chat_key(
        &mut self,
        key: KeyCode,
        shortcut: Option<sunrise_client::clipboard::EditShortcut>,
    ) -> bool {
        if let Some(shortcut) = shortcut {
            let m = Measure(&self.gui.atlas.font);
            let c = Commands {
                dispatcher: chat_tree(&self.remote),
                registries: &self.ui.registries,
            };
            if let Some(screen) = self.ui.chat_screen.as_mut() {
                screen.shortcut(&m, &c, shortcut, &mut self.clipboard);
                return true;
            }
            return false;
        }
        self.chat_key_code(key, None)
    }
    /// A key (and the text it typed) while the chat screen is open.
    pub(super) fn chat_key_code(&mut self, key: KeyCode, typed_text: Option<&str>) -> bool {
        // The live modifiers, not `self.keys`: every ModifiersChanged runs
        // sync_toggle, which drops ShiftLeft and ControlLeft from `self.keys`
        // while a screen is open.
        let ctrl = self.has_control_down_with_quirk();
        let shift = self.has_shift_down();
        // PvP & HUD's chat search: Ctrl+F starts and stops it, Enter stops it (sends nothing).
        if self.pvp_chat().search
            && match key {
                KeyCode::KeyF => ctrl,
                KeyCode::Enter | KeyCode::NumpadEnter => self.pvp.chat_search_aside.is_some(),
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
                    .ui
                    .chat_screen
                    .as_ref()
                    .is_some_and(|s| s.suggestions_visible()) =>
            {
                let n = self.ui.chat.lines_per_page(true) as i32 - 1;
                self.ui
                    .chat
                    .scroll_by(if key == KeyCode::PageUp { n } else { -n }, true);
                return true;
            }
            _ => {}
        }
        let Some(mut screen) = self.ui.chat_screen.take() else {
            return false;
        };
        let m = Measure(&self.gui.atlas.font);
        let c = Commands {
            dispatcher: chat_tree(&self.remote),
            registries: &self.ui.registries,
        };
        let (handled, action) = screen.key_input(&m, &c, &self.ui.chat.recent, key, shift, ctrl);
        match action {
            ChatAction::Close => {
                // Escape: the box still holds its text, which close_chat keeps as the draft
                // (a search's query gives way to the text it put aside: keep_chat_draft).
                self.ui.chat_screen = Some(screen);
                self.close_chat(ChatExit::Intentional);
            }
            ChatAction::Submit(message) => {
                // ChatScreen.keyPressed: handleChatInput runs with the screen still open,
                // then `closeOnSubmit` closes it. InBedChatScreen sends without closing
                // (`closeOnSubmit` is false): the box empties and the screen stays.
                let close_on_submit = !(self.ui.in_bed_chat && self.sleeping.is_some());
                self.ui.chat_screen = Some(screen);
                if !message.is_empty() {
                    self.handle_chat_input(&message);
                }
                if close_on_submit {
                    // A sent message discards the draft (DONE), whatever the box holds.
                    self.close_chat(ChatExit::Done);
                } else {
                    // The line is handled first, then the box empties and chat scrolls back.
                    if let Some(screen) = self.ui.chat_screen.as_mut() {
                        let m = Measure(&self.gui.atlas.font);
                        let c = Commands {
                            dispatcher: chat_tree(&self.remote),
                            registries: &self.ui.registries,
                        };
                        screen.suggest_command(&m, &c, "");
                    }
                    self.ui.chat.reset_scroll();
                }
            }
            ChatAction::None => {
                // charTyped: the text the key typed.
                if let Some(text) = typed_text
                    && self.types_text()
                {
                    screen.type_text(&m, &c, text);
                }
                self.ui.chat_screen = Some(screen);
            }
        }
        self.notify_chat_suggestion_completion();
        handled
    }
    fn notify_chat_suggestion_completion(&mut self) {
        if let Some(completion) = self
            .ui
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
    pub(super) fn chat_server_suggestions(
        &mut self,
        id: i32,
        range: (usize, usize),
        list: Vec<(String, Option<String>)>,
    ) {
        let m = Measure(&self.gui.atlas.font);
        let c = Commands {
            dispatcher: chat_tree(&self.remote),
            registries: &self.ui.registries,
        };
        if let Some(screen) = self.ui.chat_screen.as_mut() {
            screen.server_suggestions(&m, &c, id, range, list);
        }
    }
    /// Text typed into the chat box (control socket `chat`).
    pub(super) fn chat_type(&mut self, text: &str) {
        let m = Measure(&self.gui.atlas.font);
        let c = Commands {
            dispatcher: chat_tree(&self.remote),
            registries: &self.ui.registries,
        };
        if let Some(screen) = self.ui.chat_screen.as_mut() {
            screen.type_text(&m, &c, text);
        }
    }
    /// ChatScreen.handleChatInput: the message comes in normalized
    /// (`normalizeChatMessage`: trimmed, whitespace runs collapsed, cut to the
    /// 256 UTF-16 units the packet holds), into history, then as a command or
    /// a chat line. The Chat option's restrictions stop the message here too,
    /// so a control-socket `chat` cannot send what the player's setting hides.
    pub(super) fn handle_chat_input(&mut self, message: &str) {
        let message = normalize_chat_message(message);
        if message.is_empty() || !self.chat_abilities().allows(&message) {
            return;
        }
        self.ui.chat.add_recent(&message);
        if let Some(args) =
            sunrise_profiler_args(&message, self.remote.as_ref().map(|r| &r.chat_tree))
        {
            self.profiler_chat(args);
            return;
        }
        if self.remote.is_some() {
            self.send_signed_chat_input(&message);
            return;
        }
        if let Some(command) = message.strip_prefix('/') {
            // A native mod's command first.
            if self.run_client_native_command(command).is_none() {
                self.run_command(command);
            }
        } else {
            let line = Text::translate(
                "chat.type.text",
                &[
                    self.player_display_name(),
                    Text::untrusted(message.as_str()),
                ],
            );
            let focused = self.screen == Screen::Chat;
            self.ui.chat.add(
                &self.gui.atlas,
                line,
                None,
                ChatSource::Player,
                self.visual_ticks(),
                focused,
            );
        }
    }
    pub(super) fn chat_scroll(&mut self, amount: f32) {
        let cursor = self.cursor;
        let mouse = (cursor.0.floor() as i32, cursor.1.floor() as i32);
        if let Some(screen) = self.ui.chat_screen.as_mut()
            && screen.scroll(mouse, amount as f64)
        {
            return;
        }
        // ChatScreen.mouseScrolled: `hasShiftDown`, either Shift key.
        let shift = self.has_shift_down();
        let amount = amount.clamp(-1.0, 1.0) * if shift { 1.0 } else { 7.0 };
        self.ui.chat.scroll_by(amount as i32, true);
    }
    pub(super) fn chat_click(&mut self) -> bool {
        let cursor = self.cursor;
        let height = self.renderer.as_ref().map_or(720.0, |r| {
            HudBatch::new(
                [r.config.width, r.config.height],
                self.settings.gui_scale_inputs(),
            )
            .height
        });
        let m = Measure(&self.gui.atlas.font);
        let c = Commands {
            dispatcher: chat_tree(&self.remote),
            registries: &self.ui.registries,
        };
        if self.ui.in_bed_chat && self.sleeping.is_some() {
            let width = self.renderer.as_ref().map_or(1280.0, |r| {
                HudBatch::new(
                    [r.config.width, r.config.height],
                    self.settings.gui_scale_inputs(),
                )
                .width
            });
            let (x, y, w, h) = self
                .built_chat(width, height)
                .map_or_else(|| Self::leave_bed_button(width, height), |l| l.leave_bed);
            if sunrise_client::screen_draw::inside((cursor.0 as i32, cursor.1 as i32), x, y, w, h) {
                self.wake_up(false, true);
                return true;
            }
        }
        if let Some(screen) = self.ui.chat_screen.as_mut() {
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
            .ui
            .chat
            .style_at(
                &self.gui.atlas.font,
                height as i32,
                cursor.0 as f64,
                cursor.1 as f64,
            )
            .or_else(|| {
                self.ui.chat.queue_style_at(
                    &self.gui.atlas.font,
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
                self.ui
                    .chat
                    .message_at(height as i32, cursor.0 as f64, cursor.1 as f64)
        {
            use sunrise_client::clipboard::Clipboard;
            self.clipboard.set(message);
            let copied =
                Text::translate_with_fallback("sunrise.chat.copied", "Message copied", &[]);
            self.titles.set_overlay(copied, false);
            return true;
        }
        let Some(style) = style else {
            return false;
        };
        // ChatScreen.handleComponentClicked: with Shift held only an
        // insertion acts, else only the click event.
        if self.has_shift_down() {
            if let Some(insertion) = style.insertion
                && let Some(screen) = self.ui.chat_screen.as_mut()
            {
                screen.type_text(&m, &c, &insertion);
                return true;
            }
            return false;
        }
        match style.click {
            Some(ClickEvent::CopyToClipboard(value)) => {
                use sunrise_client::clipboard::Clipboard;
                self.clipboard.set(&value);
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
                if let Some(screen) = self.ui.chat_screen.as_mut() {
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
                if let Some(remote) = &self.remote {
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
    pub(super) fn click_url(&mut self, url: String) {
        if !self.settings.chat_links {
            return;
        }
        if self.settings.chat_links_prompt {
            self.open_screen_overlay("link_prompt", |game| game.link_prompt = Some(url.into()));
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
            game.link_prompt = Some(LinkPrompt {
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
    /// `ConfirmLinkScreen`: the link over the screen it was opened from, with its warning
    /// unless trusted, answered yes to open it.
    fn link_prompt_layout(
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
            &self.gui.atlas.font,
            size,
            &Text::translate(prompt.title_key(), &[]),
            &message,
            3,
            100.0,
        )
    }
    pub(super) fn draw_link_prompt(&self, batch: &mut HudBatch) {
        let Some(prompt) = &self.link_prompt else {
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
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        sunrise_client::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
        let labels = prompt.button_keys().map(|key| Text::translate(key, &[]));
        let mut layout = self.link_prompt_layout(prompt, (w, h));
        layout.declare_no(2);
        layout.draw(batch, atlas, self.cursor, &labels);
        self.draw_overlay_mounted_widgets(batch, "link_prompt");
    }
    /// The prompt's answer: the link opens on yes; either way the chat is back.
    pub(super) fn answer_link_prompt(&mut self, yes: bool) {
        let Some(prompt) = self.link_prompt.take() else {
            return;
        };
        self.refresh_screen_widgets();
        if yes {
            super::packs::open_uri(&prompt.url);
        }
    }
    pub(super) fn link_prompt_click(&mut self) -> bool {
        let Some(prompt) = self.link_prompt.clone() else {
            return false;
        };
        let pressed = self
            .link_prompt_layout(&prompt, self.gui_size())
            .pressed_at(self.cursor);
        if let Some(button) = pressed {
            self.play_audio_button_click();
            if button == LINK_COPY {
                // ConfirmLinkScreen.copyToClipboard, then the callback's no.
                use sunrise_client::clipboard::Clipboard;
                self.clipboard.set(&prompt.url);
            }
            self.answer_link_prompt(button == LINK_YES);
        }
        pressed.is_some()
    }
    /// `Screen.clickCommandAction`: run a clicked command as the player
    /// without adding it to the chat history. A client mod's command goes
    /// first, as `ClientPacketListener.sendUnattendedCommand` lets client
    /// commands take it.
    pub(super) fn click_command(&mut self, command: &str) {
        if self.remote.is_some() {
            self.send_unattended_command(command);
        } else {
            let command = command.strip_prefix('/').unwrap_or(command);
            if self.run_client_native_command(command).is_none() {
                self.run_command(command);
            }
        }
    }
    /// PvP & HUD's chat search on or off. The box holds the query: what was typed is put aside
    /// when the search starts and comes back when it stops.
    fn toggle_chat_search(&mut self) {
        let Some(screen) = self.ui.chat_screen.as_mut() else {
            return;
        };
        let next = if let Some((text, _)) = self.pvp.chat_search_aside.take() {
            text
        } else {
            self.pvp.chat_search_aside = Some((screen.value().to_owned(), screen.is_draft()));
            String::new()
        };
        let m = Measure(&self.gui.atlas.font);
        let c = Commands {
            dispatcher: chat_tree(&self.remote),
            registries: &self.ui.registries,
        };
        screen.suggest_command(&m, &c, &next);
    }
    /// Chat lines over the HUD (focused while the chat screen is open).
    pub(super) fn draw_chat(&mut self, batch: &mut HudBatch) {
        let focused = self.screen == Screen::Chat;
        let options = self.pvp_chat();
        self.ui.chat.set_qol(&self.gui.atlas, options.qol());
        // A search ends with its option; a chat closed some other way ends it in keep_chat_draft.
        if self.ui.chat_screen.is_none() {
            // The chat went without closing (leaving the world): nothing to give back.
            self.pvp.chat_search_aside = None;
        } else if self.pvp.chat_search_aside.is_some() && !options.search {
            self.toggle_chat_search();
        }
        let searching = focused && self.pvp.chat_search_aside.is_some();
        let query = self
            .ui
            .chat_screen
            .as_ref()
            .filter(|_| searching)
            .map(|s| s.value());
        self.ui.chat.set_search(&self.gui.atlas, query);
        self.ui.chat.search_prompt = searching;
        // `ChatComponent`'s queue row: the messages Chat Delay holds back.
        self.ui.chat.queue = self.chat_queue_len();
        if self.ui.hide_gui && !focused {
            return;
        }
        let revision = self.inline_images_revision();
        if self.ui.chat.inline_images_stale(revision) {
            // Layout borrows a read-only Host while asking listeners.
            let mut chat = std::mem::take(&mut self.ui.chat);
            chat.refresh_inline_images(&self.gui.atlas, revision, |text| {
                self.substitute_inline_images(text)
            });
            self.ui.chat = chat;
        }
        self.ui.chat.smooth = self.settings.extras.chat_animation;
        self.ui.chat.hide_indicator = !self.settings.extras.chat_indicator;
        // The fractional tick makes the smooth chat extra's slide move every
        // frame; vanilla's fade reads it too.
        let now = self.visual_ticks() as f32 + self.clock.fraction() as f32;
        // NeoForge's `CustomizeGuiOverlayEvent.Chat`: a mod may move the chat's bottom-left
        // corner, and the chat is drawn shifted by as much (`Gui.renderChat`'s translate).
        let (width, height) = (batch.width as i32, batch.height as i32);
        let (home_x, home_y) = (0, height - 40);
        let (mut x, mut y) = (home_x, home_y);
        self.native_customize_chat_overlay(width, height, focused, &mut x, &mut y);
        let offset = [(x - home_x) as f32, (y - home_y) as f32];
        self.ui
            .chat
            .draw_at(batch, &self.gui.atlas, now, focused, offset);
    }
    /// `HoverEvent.EntityTooltipInfo.getTooltipLines`: the name when the
    /// event has one, the entity type and the UUID.
    fn entity_tooltip_lines(entity: &serde_json::Value) -> Vec<(Text, u32)> {
        let mut lines = Vec::new();
        if let Some(name) = entity.get("name") {
            let name = sunrise_client::component::to_text(name, &mut |s| Text::literal(s));
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
    fn hover_event_entity_uuid(v: &serde_json::Value) -> Option<String> {
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
    pub(super) fn draw_chat_screen(&mut self, batch: &mut HudBatch) {
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        // ChatScreen extracts focused chat separately, outside the HUD chat slot.
        self.draw_chat(batch);
        let Some(mut screen) = self.ui.chat_screen.take() else {
            self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
            return;
        };
        // ChatScreen.updateNarrationState: the restricted prompt is
        // `displayMode.showRestrictedPrompt`, the abilities' restrictions.
        let in_bed = self.ui.in_bed_chat && self.sleeping.is_some();
        sunrise_client::widgets::begin_screen(|| {
            super::screen_info::chat(
                in_bed,
                screen.value(),
                screen.abilities.any_restriction(),
                &super::screen_info::tr,
            )
        });
        // The build made the box at this size (`ChatScreen.resize`).
        let m = Measure(&self.gui.atlas.font);
        // Options.getBackgroundColor(Integer.MIN_VALUE).
        let background = if self.settings.file.bool("backgroundForChatOnly") {
            i32::MIN
        } else {
            self.settings.text_background(0.0) as i32
        };
        let mouse = (self.cursor.0.floor() as i32, self.cursor.1.floor() as i32);
        let blink = self
            .ui
            .chat_opened
            .map_or(0, |t| t.elapsed().as_millis() as u64);
        let draws = screen.draw(&m, mouse, blink, background);
        self.ui.chat_screen = Some(screen);
        let tooltip = sunrise_client::screen_draw::paint(batch, &self.gui.atlas, &draws);
        if in_bed {
            let (w, h) = (batch.width, batch.height);
            let rect = self
                .built_chat(w, h)
                .map_or_else(|| Self::leave_bed_button(w, h), |l| l.leave_bed);
            let button = Self::leave_bed_draws(&m, mouse, rect);
            let _ = sunrise_client::screen_draw::paint(batch, &self.gui.atlas, &button);
        }
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
        sunrise_client::screen_draw::paint_tooltip(batch, &self.gui.atlas, &draws, tooltip);
        // The hovered message part's tooltip (`renderComponentHoverEffect`),
        // or the text of the message tag under the cursor.
        let height = batch.height as i32;
        let (cx, cy) = (f64::from(self.cursor.0), f64::from(self.cursor.1));
        // handleTagIcon: the chat-modified icons over the cursor or its message.
        let (tag_tooltip, tag_icons) =
            self.ui
                .chat
                .focused_tags(&self.gui.atlas.font, height, cx, cy);
        sunrise_client::hud_layout::paint(batch, &self.gui.atlas, &tag_icons);
        let style = self
            .ui
            .chat
            .style_at(&self.gui.atlas.font, height, cx, cy)
            .or_else(|| {
                self.ui
                    .chat
                    .queue_style_at(&self.gui.atlas.font, height, cx, cy)
            });
        if let Some(style) = style {
            // ActiveTextCollector.ClickableStyleFinder: a click event, or an
            // insertion while Shift is held (ChatScreen.includeInsertions).
            if style.click.is_some() || (style.insertion.is_some() && self.has_shift_down()) {
                sunrise_client::widgets::request_cursor(
                    sunrise_client::widgets::Cursor::PointingHand,
                );
            }
            self.component_hover_effect(batch, &style, self.cursor);
        } else if let Some(tag) = &tag_tooltip {
            // The message tag's text, split at 210.
            let lines = self.wrapped_rows(tag, 210);
            self.draw_tooltip_lines(batch, &lines, self.cursor.0, self.cursor.1);
        }
    }
    /// `GuiGraphicsExtractor.componentHoverEffect`: the hover of the text component under the
    /// cursor at `at`, as the chat, a book and a dialog draw it. `component_hover` is asked
    /// first and may cancel it. Then a text hover is split at half the screen's width (at
    /// least 200), an item hover draws its stack's tooltip, and an entity hover its name, type
    /// and UUID while advanced tooltips are on.
    pub(super) fn component_hover_effect(
        &mut self,
        batch: &mut HudBatch,
        style: &sunrise_client::text::Style,
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
            .and_then(sunrise_client::item_data::stack_from_hover)
            .filter(|s| self.items().get(&s.item).is_some())
        {
            self.draw_stack_tooltip(batch, &stack, at.0, at.1);
        } else if let Some(entity) = style
            .hover_entity
            .as_deref()
            .filter(|_| self.settings.file.bool("advancedItemTooltips"))
        {
            // A ShowEntity tooltip only with advanced tooltips on.
            let lines = Self::entity_tooltip_lines(entity);
            self.draw_tooltip_lines(batch, &lines, at.0, at.1);
        }
    }
    /// `component_hover`, only while a mod listens: true when a mod cancelled the hover.
    fn component_hover_canceled(
        &self,
        batch: &mut HudBatch,
        style: &sunrise_client::text::Style,
        at: (f32, f32),
    ) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::ComponentHover) {
            return false;
        }
        let Some(hover) = sunrise_client_lib::api_text::style_hover(style) else {
            return false;
        };
        let mut cancel = false;
        let mut draw = super::host::draw::GameDrawContext::new(self, batch);
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
    pub(super) fn wrapped_rows(&self, text: &Text, width: i32) -> Vec<(Text, u32)> {
        let split = self
            .gui
            .atlas
            .font
            .split(&text.resolve_legacy(), width as f32);
        split.into_iter().map(|row| (row, u32::MAX)).collect()
    }
    /// Command suggestions include the world's currently named random sequences.
    pub(super) fn suggestion_registries(&mut self) -> &mut Registries {
        self.ui.registries.random_sequences = self.rand_sequences.ids();
        &mut self.ui.registries
    }
    /// Advance every clock of the world's `minecraft:world_clock` registry, the mods' own included,
    /// and every other clock the world has met (ServerClockManager.tick) when `advance_time`
    /// allows it: each at its own rate, unless paused. The active clock's total is World.day_time.
    pub(super) fn advance_clock(&mut self) {
        if !self
            .ui
            .rules
            .bool_at(sunrise_client::gamerules::vanilla::ADVANCE_TIME)
        {
            // The rule may have just been turned off: tell the guests the clock stopped.
            self.sync_hub_clocks();
            return;
        }
        let mut names: Vec<String> = self.ui.command_clocks.keys().cloned().collect();
        let active = self.active_clock().to_owned();
        // ServerClockManager.init holds every `minecraft:world_clock` entry, the mods' own
        // (`register_world_clock`) among them, and `tick` advances each one, touched or not.
        for name in [
            OVERWORLD_CLOCK.to_owned(),
            "minecraft:the_end".to_owned(),
            active,
        ]
        .into_iter()
        .chain(self.ui.registries.clocks.iter().cloned())
        .chain(mod_api::__private::world_clock_declarations())
        {
            if !names.contains(&name) {
                names.push(name);
            }
        }
        for name in names {
            let state = self.command_clock_state(&name);
            if state.paused {
                continue;
            }
            state.partial += state.rate;
            let whole = state.partial.floor();
            state.partial -= whole;
            let total = self.command_clock_time(&name) + whole as i64;
            self.set_command_clock_time(&name, total);
        }
        self.sync_hub_clocks();
    }
    /// The clock `World.day_time` follows (`UiState::active_clock`).
    pub(in crate::game) fn active_clock(&self) -> &str {
        if self.ui.active_clock.is_empty() {
            OVERWORLD_CLOCK
        } else {
            &self.ui.active_clock
        }
    }
    /// `ServerClockManager.getTotalTicks` of `clock`: the active one's is the world's day time,
    /// another's the table's (0 for a clock the world has not met).
    pub(in crate::game) fn command_clock_time(&self, clock: &str) -> i64 {
        if clock == self.active_clock() {
            self.world.day_time
        } else if clock == "minecraft:the_end" {
            self.world.end_clock
        } else {
            self.ui.command_clocks.get(clock).map_or(0, |c| c.0)
        }
    }
    /// Every world clock's total ticks by id, for the loot conditions that read one.
    pub(in crate::game) fn loot_clock_ticks(&self) -> std::collections::BTreeMap<String, i64> {
        let mut clocks: std::collections::BTreeMap<String, i64> = self
            .ui
            .command_clocks
            .iter()
            .map(|(name, state)| (name.clone(), state.0))
            .collect();
        for name in [self.active_clock(), OVERWORLD_CLOCK, "minecraft:the_end"] {
            clocks.insert(name.to_owned(), self.command_clock_time(name));
        }
        clocks
    }
    pub(in crate::game) fn set_command_clock_time(&mut self, clock: &str, total: i64) {
        if clock == "minecraft:the_end" {
            self.world.end_clock = total;
        }
        if clock == self.active_clock() {
            self.world.day_time = total;
        } else {
            self.ui
                .command_clocks
                .entry(clock.to_owned())
                .or_default()
                .0 = total;
        }
    }
    /// `ServerClockManager.getInstance`: the rate, partial tick and pause of `clock`.
    pub(in crate::game) fn command_clock_state(&mut self, clock: &str) -> &mut WorldClock {
        if clock == OVERWORLD_CLOCK {
            &mut self.ui.clock
        } else if clock == "minecraft:the_end" {
            &mut self.ui.end_clock
        } else {
            &mut self
                .ui
                .command_clocks
                .entry(clock.to_owned())
                .or_default()
                .1
        }
    }
    /// The clock the day time of `dim`'s level follows: its dimension type's default clock, else
    /// the Overworld's.
    pub(in crate::game) fn level_clock_name(&self, dim: crate::game::Dim) -> String {
        self.dimension_type_of(dim)
            .default_clock
            .clone()
            .unwrap_or_else(|| OVERWORLD_CLOCK.to_owned())
    }
    /// What the LAN hub's time packets need: the Overworld's total while `World.day_time` follows
    /// another clock, and that clock with its rate.
    pub(in crate::game) fn sync_hub_clocks(&mut self) {
        let active = self.active_clock().to_owned();
        let overworld = self.command_clock_time(OVERWORLD_CLOCK);
        let advancing = self
            .ui
            .rules
            .bool_at(sunrise_client::gamerules::vanilla::ADVANCE_TIME);
        // A paused clock, or `advance_time` off, is a rate of 0 to the guests, as for the Overworld's.
        let guest_rate = |state: &WorldClock| {
            if advancing && !state.paused {
                state.rate
            } else {
                0.0
            }
        };
        let rate = guest_rate(self.command_clock_state(&active));
        let end_total = self.command_clock_time("minecraft:the_end");
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        // The clocks neither field below covers: the End's and every other the world has met.
        let others = std::iter::once(("minecraft:the_end", end_total, &self.ui.end_clock)).chain(
            self.ui
                .command_clocks
                .iter()
                .map(|(name, (total, state))| (name.as_str(), *total, state)),
        );
        for (name, total, state) in others {
            if name != active && name != OVERWORLD_CLOCK {
                host.hub.set_clock_state(name, total, guest_rate(state));
            }
        }
        if active == OVERWORLD_CLOCK {
            host.hub.overworld_day = None;
            host.hub.level_clock = None;
        } else {
            host.hub.overworld_day = Some(overworld);
            host.hub.level_clock = Some((active, rate));
        }
    }
    /// Makes `to` the clock `World.day_time` follows, as the level entered has it as its default
    /// clock: the clock that was followed keeps its total in the table. Returns the total
    /// `World.day_time` takes.
    pub(in crate::game) fn switch_clock(&mut self, to: &str) -> i64 {
        let from = self.active_clock().to_owned();
        if from == to {
            return self.world.day_time;
        }
        let total = self.world.day_time;
        if from == "minecraft:the_end" {
            self.world.end_clock = total;
        }
        self.ui.command_clocks.entry(from).or_default().0 = total;
        self.ui.active_clock = to.to_owned();
        if to == "minecraft:the_end" {
            self.world.end_clock
        } else {
            self.ui.command_clocks.entry(to.to_owned()).or_default().0
        }
    }
    /// `ServerClockInstance.packNetworkState`: the rate players' clients
    /// advance the overworld clock at, 0 while it is paused (`/time pause`)
    /// or `advance_time` is off.
    pub(super) fn network_clock_rate(&self) -> f32 {
        if self
            .ui
            .rules
            .bool_at(sunrise_client::gamerules::vanilla::ADVANCE_TIME)
            && !self.ui.clock.paused
        {
            self.ui.clock.rate
        } else {
            0.0
        }
    }
    /// The engine-side rule values (block behaviour and health).
    pub(super) fn level_rules(&self) -> sunrise_world::level::GameRules {
        let r = &self.ui.rules;
        sunrise_world::level::GameRules {
            water_source_conversion: r
                .bool_at(sunrise_client::gamerules::vanilla::WATER_SOURCE_CONVERSION),
            lava_source_conversion: r
                .bool_at(sunrise_client::gamerules::vanilla::LAVA_SOURCE_CONVERSION),
            random_tick_speed: r.int_at(sunrise_client::gamerules::vanilla::RANDOM_TICK_SPEED),
            block_drops: r.bool_at(sunrise_client::gamerules::vanilla::BLOCK_DROPS),
            mob_griefing: r.bool_at(sunrise_client::gamerules::vanilla::MOB_GRIEFING),
            max_snow_accumulation_height: r
                .int_at(sunrise_client::gamerules::vanilla::MAX_SNOW_ACCUMULATION_HEIGHT),
            spawn_mobs: r.bool_at(sunrise_client::gamerules::vanilla::SPAWN_MOBS),
            spawn_monsters: r.bool_at(sunrise_client::gamerules::vanilla::SPAWN_MONSTERS),
            fire_spread_radius_around_player: r
                .int_at(sunrise_client::gamerules::vanilla::FIRE_SPREAD_RADIUS_AROUND_PLAYER),
            spread_vines: r.bool_at(sunrise_client::gamerules::vanilla::SPREAD_VINES),
        }
    }
    /// Apply rules mirrored in other state (natural regeneration, difficulty).
    pub(super) fn sync_rules(&mut self) {
        self.world.health_rules.natural_regeneration = self
            .ui
            .rules
            .bool_at(sunrise_client::gamerules::vanilla::NATURAL_HEALTH_REGENERATION);
        // Player.isInvulnerableTo: the damage-type rules.
        self.world.health_rules.drowning_damage = self
            .ui
            .rules
            .bool_at(sunrise_client::gamerules::vanilla::DROWNING_DAMAGE);
        self.world.health_rules.fall_damage = self
            .ui
            .rules
            .bool_at(sunrise_client::gamerules::vanilla::FALL_DAMAGE);
        self.world.health_rules.fire_damage = self
            .ui
            .rules
            .bool_at(sunrise_client::gamerules::vanilla::FIRE_DAMAGE);
        self.world.health_rules.freeze_damage = self
            .ui
            .rules
            .bool_at(sunrise_client::gamerules::vanilla::FREEZE_DAMAGE);
        sunrise_client::mobs::sulfur_cube::set_tnt_explodes(
            self.ui
                .rules
                .bool_at(sunrise_client::gamerules::vanilla::TNT_EXPLODES),
        );
        use sunrise_world::food::Difficulty;
        self.world.health_rules.difficulty = match self.ui.difficulty {
            0 => Difficulty::Peaceful,
            1 => Difficulty::Easy,
            3 => Difficulty::Hard,
            _ => Difficulty::Normal,
        };
    }
    /// `ServerClockManager.ServerClockInstance.loadFrom`: restore the existing
    /// Overworld state when a world opens; clear the other command-state cache.
    /// The carrier store's open-time state settles here too, before the
    /// first tick and any player: a refusal fails the open.
    pub(super) fn load_world_state(&mut self) -> crate::Result<()> {
        self.ui.dispatcher = SessionDispatcher(Arc::new(
            sunrise_client::command::vanilla::session_dispatcher(
                &self.level.rule_registry,
                &self.level.enabled_features,
            ),
        ));
        self.restore_native_commands();
        self.ui.clock = WorldClock::default();
        self.ui.end_clock = WorldClock::default();
        self.ui.command_clocks.clear();
        // PrimaryLevelData: the difficulty comes back from level.dat's difficulty_settings.
        self.ui.difficulty = self.level.difficulty_id();
        self.ui.active_clock = OVERWORLD_CLOCK.to_owned();
        if let Ok(bytes) = std::fs::read(self.world_root.join("data/minecraft/world_clocks.dat"))
            && let Ok(raw) = sunrise_world::compression::Compression::Gzip.decode(&bytes, 1 << 20)
            && let Ok((root, _)) =
                sunrise_core::nbt::read_named(&raw, sunrise_core::nbt::Limits::default())
            && let Some(sunrise_core::nbt::Tag::Compound(clocks)) = root.tag.get("data")
        {
            use sunrise_core::nbt::Tag;
            for (name, c) in clocks {
                let mut state = WorldClock::default();
                if let Some(Tag::Float(r)) = c.get("rate") {
                    state.rate = f32::from_bits(*r);
                }
                if let Some(Tag::Float(p)) = c.get("partial_tick") {
                    state.partial = f32::from_bits(*p);
                }
                if let Some(Tag::Byte(p)) = c.get("paused") {
                    state.paused = *p != 0;
                }
                let total = match c.get("total_ticks") {
                    Some(Tag::Long(t)) => *t,
                    _ => 0,
                };
                let name = name.to_string_lossy();
                if name == OVERWORLD_CLOCK {
                    self.ui.clock = state;
                } else if name == "minecraft:the_end" {
                    self.ui.end_clock = state;
                } else {
                    self.ui.command_clocks.insert(name, (total, state));
                }
            }
        }
        // The level the world opens in follows its own default clock; the Overworld's total
        // (level.dat's day time, already in World.day_time) goes to the table.
        let clock = self.level_clock_name(self.dim);
        self.world.day_time = self.switch_clock(&clock);
        self.ui.chat.clear();
        self.build_registries();
        self.sync_rules();
        self.carrier_world_opened()
    }
    /// Save the clock beside level.dat (Level.write saves the game rules: a
    /// second writer of game_rules.dat could leave it with the rules loaded
    /// when the world opened).
    pub(super) fn save_world_state(&mut self) {
        if self.world_root.as_os_str().is_empty() {
            return;
        }
        use sunrise_core::nbt::Tag;
        // Every clock the world has met is saved; the fields of a clock Sunrise does not model
        // stay as saved, and (like vanilla's codec) rate, partial tick and pause are omitted
        // while they have their defaults.
        let path = self.world_root.join("data/minecraft/world_clocks.dat");
        let existing = sunrise_client::save::read_dat(&path);
        let mut data = match existing.as_ref().and_then(|r| r.get("data")) {
            Some(d @ Tag::Compound(_)) => d.clone(),
            _ => Tag::Compound(Default::default()),
        };
        let mut clocks = vec![(
            OVERWORLD_CLOCK.to_owned(),
            self.command_clock_time(OVERWORLD_CLOCK),
            self.ui.clock,
        )];
        clocks.push((
            "minecraft:the_end".to_owned(),
            self.command_clock_time("minecraft:the_end"),
            self.ui.end_clock,
        ));
        for (name, (_, state)) in &self.ui.command_clocks {
            if name != OVERWORLD_CLOCK && name != "minecraft:the_end" {
                clocks.push((name.clone(), self.command_clock_time(name), *state));
            }
        }
        for (name, total, c) in clocks {
            let mut clock = match data.get(&name) {
                Some(c @ Tag::Compound(_)) => c.clone(),
                _ => Tag::Compound(Default::default()),
            };
            if let Tag::Compound(map) = &mut clock {
                map.insert("total_ticks".into(), Tag::Long(total));
                for (key, value, default) in [
                    (
                        "partial_tick",
                        Tag::Float(c.partial.to_bits()),
                        c.partial == 0.0,
                    ),
                    ("rate", Tag::Float(c.rate.to_bits()), c.rate == 1.0),
                    ("paused", Tag::Byte(i8::from(c.paused)), !c.paused),
                ] {
                    if default {
                        map.remove(&key.into());
                    } else {
                        map.insert(key.into(), value);
                    }
                }
            }
            if let Tag::Compound(map) = &mut data {
                map.insert(name.as_str().into(), clock);
            }
        }
        let root = Tag::compound([
            ("data", data),
            ("DataVersion", Tag::Int(sunrise_world::DATA_VERSION)),
        ]);
        if let Err(e) = sunrise_client::save::write_dat(&path, root) {
            log::warn!("the world clocks were not written: {e}");
        }
    }
    /// F1 hides the HUD (see `hud`); the screenshot key still works.
    pub(super) fn toggle_hide_gui(&mut self) {
        self.ui.toggle_hide_gui();
    }
    /// F2: ScreenShot.grab into `screenshots/` with vanilla's file name.
    pub(super) fn take_screenshot(&mut self) -> Option<std::path::PathBuf> {
        if self.screenshot_sequence.is_some() {
            return None;
        }
        let dir = sunrise_client::logging::player_dir().join("screenshots");
        if std::fs::create_dir_all(&dir).is_err() {
            return None;
        }
        let name = screenshot_name(&dir, &self.ui.screenshot_messages);
        let renderer = self.renderer.as_mut()?;
        let path = dir.join(name);
        // A second shortcut before rendering replaces the queued frame. Release
        // that request's name: no readback will arrive to finish it.
        if let Some(replaced) = renderer.screenshot.as_ref() {
            self.ui.screenshot_messages.retain(|p| *p != replaced.path);
        }
        renderer.screenshot = Some(sunrise_client::gfx::ScreenshotRequest {
            path: path.clone(),
            size: None,
            scaled: true,
            background: true,
        });
        self.ui.screenshot_messages.push(path.clone());
        Some(path)
    }
    /// `Minecraft.handleGlobalKeyPress`'s screenshot branch: the
    /// `key.screenshot` mapping (F2 by default) takes the picture before a
    /// screen or a text box sees the key, and the debug modifier suppresses it.
    pub(super) fn screenshot_shortcut(&mut self, key: KeyCode, repeat: bool) -> bool {
        if repeat
            || sunrise_client::keybinds::key_name(key)
                != Some(self.settings.keys.get("key.screenshot"))
        {
            return false;
        }
        self.screenshot_action()
    }
    /// key.screenshot from any device: not while F3 is down, where F3+F2 is a
    /// debug key.
    pub(super) fn screenshot_action(&mut self) -> bool {
        if self.debug_keys.modifier_down {
            return false;
        }
        let _ = self.take_screenshot();
        true
    }
    /// Readback is complete: fire the client hook, then save the image it left.
    pub(super) fn save_screenshot(
        &mut self,
        mut pending: sunrise_client::gfx::PendingScreenshot,
    ) -> Option<std::result::Result<[u32; 2], String>> {
        if !self.ui.screenshot_messages.contains(&pending.path) {
            if !pending.background {
                return Some(pending.save().map_err(|e| e.to_string()));
            }
            self.ui
                .screenshot_writes
                .push(super::screenshot_writer::ScreenshotWrite::start(
                    pending, None, None, false,
                ));
            return None;
        }
        let at = self
            .ui
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
            self.ui.screenshot_messages.remove(at);
            self.post_screenshot_chat(screenshot_cancel_line(message));
            return None;
        }
        let root = sunrise_client::logging::player_dir().join("screenshots");
        self.ui
            .screenshot_writes
            .push(super::screenshot_writer::ScreenshotWrite::start(
                pending,
                Some((root, file_name)),
                message,
                true,
            ));
        None
    }
    /// After a frame: report a finished F2 screenshot in chat.
    pub(super) fn finish_screenshot(
        &mut self,
        result: &std::result::Result<[u32; 2], String>,
        message: Option<Text>,
    ) {
        if self.ui.screenshot_messages.is_empty() {
            return;
        }
        let path = self.ui.screenshot_messages.remove(0);
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

    fn post_screenshot_chat(&mut self, message: Text) {
        let focused = self.screen == Screen::Chat;
        self.ui.chat.add(
            &self.gui.atlas,
            message,
            None,
            ChatSource::Client,
            self.visual_ticks(),
            focused,
        );
    }
}

/// `ScreenshotEvent.getCancelMessage`: the listener's line, or the default reason.
pub(super) fn screenshot_cancel_line(message: Option<Text>) -> Text {
    message.unwrap_or_else(|| {
        Text::translate(
            "screenshot.failure",
            &[Text::translate("sunrise.screenshot.canceled", &[])],
        )
    })
}
/// `yyyy-MM-dd_HH.mm.ss[_n].png` (Util.getFilenameFormattedDateTime, in local time).
pub fn screenshot_name(dir: &std::path::Path, pending: &[std::path::PathBuf]) -> String {
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

/// `IntegratedServer.getProfilePermissions` for the owner: `OWNER` (4) when
/// commands are allowed, else `ALL` (0).
fn owner_permission(allow_commands: bool) -> u8 {
    if allow_commands { 4 } else { 0 }
}

/// `ConfirmLinkScreen`'s buttons, left to right: yes, copy, no.
const LINK_YES: usize = 0;
const LINK_COPY: usize = 1;

/// A link waiting for the player's answer (`ConfirmLinkScreen`). `trusted` is vanilla's
/// argument: it decides only the title, the warning line and the buttons' labels.
#[derive(Clone)]
pub(super) struct LinkPrompt {
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
    /// The yes, copy and no labels: `GUI_YES` / `GUI_NO`, or `GUI_OPEN_IN_BROWSER` /
    /// `GUI_CANCEL` when trusted; `GUI_COPY_TO_CLIPBOARD` either way.
    fn button_keys(&self) -> [&'static str; 3] {
        if self.trusted {
            ["chat.link.open", "chat.copy", "gui.cancel"]
        } else {
            ["gui.yes", "chat.copy", "gui.no"]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typed_profiler_is_the_clients_unless_the_server_owns_the_name() {
        let vanilla = sunrise_client::command::vanilla::game_dispatcher();
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

    #[test]
    #[ignore = "daily"]
    fn native_persistent_components_are_available_to_give() {
        sunrise_protocol::item::custom::install(1000, vec!["example:marker".into()]).unwrap();
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(
                sunrise_client::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::ZERO,
                )
                .unwrap(),
            );
            game.build_registries();
            game.build_registries();
            game.ui.registries.permission = 4;
            assert_eq!(
                game.ui
                    .registries
                    .persistent_components
                    .iter()
                    .filter(|id| *id == "example:marker")
                    .count(),
                1
            );
            let mut suggestions =
                sunrise_command::SuggestionsBuilder::new("minecraft:apple[example:", 0);
            sunrise_command::item::suggest_stack(&mut suggestions, &game.ui.registries);
            assert!(
                suggestions
                    .out
                    .iter()
                    .any(|entry| entry.text == "example:marker=")
            );
            assert!(game.run_command("give @s minecraft:apple[example:marker={probe:1}]"));
            let item = game
                .inventory
                .iter()
                .flatten()
                .find(|item| &*item.item == "minecraft:apple")
                .unwrap();
            assert_eq!(
                item.data().and_then(|data| data.other("example:marker")),
                Some(&sunrise_core::nbt::Tag::compound([(
                    "probe",
                    sunrise_core::nbt::Tag::Int(1)
                )]))
            );
            assert!(!game.run_command("give @s minecraft:apple[minecraft:creative_slot_lock={}]"));
        });
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

    /// The export is read once per folder; a folder without one stays without.
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

    /// A client command's usage becomes real nodes in the chat tree and binds
    /// its executor slot to the client dispatcher generation while preserving
    /// server redirects and permissions in `base`.
    #[test]
    fn a_client_commands_usage_is_parsed_by_the_chat_tree() {
        let reg = Registries::default();
        let vanilla = sunrise_client::command::vanilla::game_dispatcher();
        let commands = [mod_api::Command::new("xmode <mode:word>")];
        let arguments = super::super::native_server::command_arguments();
        let mut slots = super::super::native_server::CommandExecutorSlots::new();
        let (tree, _, problems) = super::super::native_server::client_command_tree(
            &commands,
            &arguments,
            Some(&mut slots),
        );
        assert!(problems.is_empty(), "{problems:?}");
        let mut client_dispatcher = Dispatcher::new();
        super::super::native_server::register_command_nodes(
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
        super::super::native_server::register_command_nodes(
            &mut rebuilt,
            &tree,
            MOD_COMMAND,
            &arguments,
            Some(&slots),
        );
        assert!(!rebuilt.owns_executor(key));

        let mut chat = vanilla.clone_with_unmerged_redirects();
        super::super::native_server::register_dispatcher(&mut chat, &tree, MOD_COMMAND, &arguments);
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

        let arguments = super::super::native_server::command_arguments();
        let client_cmds = [
            mod_api::Command::new("shared local <value:integer>"),
            mod_api::Command::new("clientonly"),
        ];
        let (tree, _, problems) =
            super::super::native_server::client_command_tree(&client_cmds, &arguments, None);
        assert!(problems.is_empty(), "{problems:?}");

        let mut display = server.clone_with_unmerged_redirects();
        super::super::native_server::register_dispatcher(
            &mut display,
            &tree,
            MOD_COMMAND,
            &arguments,
        );
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
        super::super::native_entity_refs::tests::with_game(|game| {
            let tree = game.chat_tree_for(None);
            let reg = Registries::default();
            assert!(tree.find("studio").is_some());
            assert_eq!(
                tree.parse("studio start", &reg).executable().unwrap().1,
                sunrise_client::command::MOD_COMMAND
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
        let vanilla = sunrise_client::command::vanilla::game_dispatcher();
        let plain = with_native_commands(vanilla, &[]);
        assert_eq!(plain.nodes.len(), vanilla.nodes.len());
        assert!(plain.find("flyspeed").is_none());
        let tree = with_native_commands(vanilla, &["flyspeed"]);
        assert!(tree.find("flyspeed").is_some());
        // The rest of the line is parsed as a greedy argument in the chat tree.
        let parse = tree.parse("flyspeed 4", &reg);
        assert_eq!(
            parse.executable().unwrap().1,
            sunrise_client::command::MOD_COMMAND
        );
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
        let base = sunrise_client::command::vanilla::game_dispatcher();
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

        assert!(sunrise_client::command::args::resource_matches(
            "wind",
            "minecraft:wind_charge"
        ));
        assert!(sunrise_client::command::args::resource_matches(
            "wind",
            "minecraft:breeze_wind_charge"
        ));
        assert!(sunrise_client::command::args::resource_matches(
            "minecraft:wind_charg",
            "minecraft:wind_charge"
        ));
        assert!(!sunrise_client::command::args::resource_matches(
            "minecraft:wind_charg",
            "minecraft:breeze_wind_charge"
        ));

        let reg = Registries {
            permission: 2,
            summonable,
            ..Default::default()
        };
        let vanilla = sunrise_client::command::vanilla::game_dispatcher();

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
        let tree = sunrise_client::command::vanilla::game_dispatcher();
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
            dispatcher: sunrise_client::command::vanilla::game_dispatcher(),
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
        let (x, y, w, h) = Game::leave_bed_button(width, height);
        Game::leave_bed_draws(&m, (0, 0), (x, y, w, h));
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

#[cfg(test)]
mod enchantment_snapshot_tests {
    use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};

    #[test]
    #[ignore = "daily"]
    fn synchronized_definitions_refresh_existing_enchantment_views() {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();

        let dir = std::env::temp_dir().join(format!("sunrise-h024-model-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pack = dir.join("datapacks/model");
        let tags = pack.join("data/h024/tags/enchantment");
        std::fs::create_dir_all(&tags).unwrap();
        std::fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"fixture"}}"#,
        )
        .unwrap();
        std::fs::write(tags.join("empty.json"), r#"{"values":[]}"#).unwrap();
        std::fs::write(
            dir.join("datapacks.json"),
            r#"{"enabled":["file/model"],"disabled":[]}"#,
        )
        .unwrap();
        game.world_root = dir;
        let stack = game.reload_datapacks(false, &[]).unwrap();
        let root = game.options.resources.clone();
        game.world_tables = game
            .stage_world_tables(&stack, false, &root)
            .unwrap()
            .tables;
        assert!(
            game.world_tables.is_some(),
            "the real active table container must be available"
        );
        let base = game.stations().enchantments.clone();
        assert!(base.get("minecraft:sharpness").is_some());
        // Same definition independently admitted by fresh Java. This is controlled
        // synchronized model input, NOT a world-pack or packet-delivery claim.
        let definition = serde_json::json!({"description":"H024 audit","supported_items":"minecraft:diamond_sword",
            "weight":1,"max_level":1,"min_cost":{"base":1,"per_level_above_first":0},
            "max_cost":{"base":1,"per_level_above_first":0},"anvil_cost":1,"slots":["mainhand"],"effects":{}});
        let synced = std::sync::Arc::new(
            base.with_entries([("h024:audit", &definition)])
                .unwrap()
                .unwrap(),
        );
        game.world_tables.as_mut().unwrap().stations.enchantments = synced.clone();
        std::sync::Arc::make_mut(&mut game.world_tables.as_mut().unwrap().items).enchantments =
            synced;
        assert!(
            game.active_enchantment_definitions()
                .get("h024:audit")
                .is_some(),
            "actual authority adopts the definition first"
        );
        let tags = vec!["h024:retained_tag".to_owned()];
        game.ui
            .registries
            .by_registry
            .entry("minecraft:enchantment".into())
            .or_default()
            .tags = tags.clone();
        let item_ids = game.ui.registries.ids("minecraft:item").to_vec();
        let effects = game.ui.registries.effects.clone();
        game.sync_function_registries();
        for phase in [0, 1] {
            if phase == 1 {
                game.build_registries();
            }
            let reg = &game.ui.registries;
            assert!(
                reg.ids("minecraft:enchantment")
                    .contains(&"h024:audit".to_owned()),
                "active custom ID reaches actual Game sync/build snapshot"
            );
            assert!(
                reg.ids("minecraft:enchantment")
                    .contains(&"minecraft:sharpness".to_owned())
            );
            assert!(
                !reg.ids("minecraft:enchantment")
                    .contains(&"h024:missing".to_owned())
            );
            assert_eq!(
                reg.ids("minecraft:enchantment"),
                reg.enchantments.as_slice()
            );
            assert_eq!(reg.by_registry["minecraft:enchantment"].tags, tags);
            assert_eq!(reg.ids("minecraft:item"), item_ids);
            assert_eq!(reg.effects, effects);
        }
        game.world_tables.as_mut().unwrap().stations.enchantments = base.clone();
        std::sync::Arc::make_mut(&mut game.world_tables.as_mut().unwrap().items).enchantments =
            base;
        assert!(
            game.active_enchantment_definitions()
                .get("h024:audit")
                .is_none()
        );
        game.sync_function_registries();
        assert!(
            !game
                .ui
                .registries
                .ids("minecraft:enchantment")
                .contains(&"h024:audit".to_owned()),
            "refresh replaces rather than accumulating stale IDs"
        );
        assert_eq!(
            game.ui.registries.ids("minecraft:enchantment"),
            game.ui.registries.enchantments.as_slice()
        );
        assert_eq!(
            game.ui.registries.by_registry["minecraft:enchantment"].tags,
            tags
        );
    }
}

#[cfg(test)]
mod damage_type_registry_tests {
    use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};

    /// A world pack's damage type is a `/damage` type once the world opens,
    /// beside vanilla's, and the registry keeps vanilla's tags.
    #[test]
    #[ignore = "daily"]
    fn a_world_packs_damage_type_parses_in_damage() {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();

        let dir = std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
            .join("damage-type-ids");
        let pack = dir.join("datapacks/model");
        let types = pack.join("data/example/damage_type");
        std::fs::create_dir_all(&types).unwrap();
        std::fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"test","pack_format":121}}"#,
        )
        .unwrap();
        std::fs::write(
            types.join("test.json"),
            r#"{"message_id":"example.test","scaling":"never","exhaustion":0.0}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("datapacks.json"),
            r#"{"enabled":["file/model"],"disabled":[]}"#,
        )
        .unwrap();
        game.world_root = dir;
        let stack = game.reload_datapacks(false, &[]).unwrap();
        let staged = game.stage_world_tables(&stack, false, &root).unwrap();
        game.world_tables = staged.tables;
        game.health_resources = staged.health;
        game.level.allow_commands = Some(true);
        game.build_registries();

        let reg = &game.ui.registries;
        assert!(
            reg.damage_types.contains(&"example:test".to_owned()),
            "the damage catalog holds the pack's type"
        );
        let parse = |input: &str| {
            sunrise_client::command::vanilla::dispatcher()
                .parse(input, reg)
                .executable()
                .map(|_| ())
                .map_err(|e| format!("{e:?}"))
        };
        assert_eq!(parse("damage @s 1 example:test"), Ok(()));
        assert_eq!(parse("damage @s 1 minecraft:magic"), Ok(()));
        assert!(parse("damage @s 1 example:missing").is_err());
        assert!(
            reg.tags("minecraft:damage_type")
                .contains(&"minecraft:is_fire".to_owned()),
            "vanilla's damage type tags stay"
        );
    }
}

#[cfg(test)]
#[path = "tests/end_clock_tests.rs"]
mod end_clock_tests;

#[cfg(test)]
mod link_and_path_tests {
    use super::*;
    use mod_api::{ClientHost, ClientTickPreEvent, Hooks, ModEntry, OpenLinkError, OpenPathError};
    use mod_platform::testing::with_mods;
    use std::sync::{Arc, Mutex};

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

    /// Runs one client tick of a mod that asks for `uri`, in a client game, and hands the
    /// game and the answer to `check`.
    fn ask(
        uri: &'static str,
        trusted: bool,
        check: impl FnOnce(&mut Game<'_>, Result<(), OpenLinkError>),
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
            super::super::native_entity_refs::tests::with_game(|game| {
                game.options.execution_role = super::super::execution_role::ExecutionRole::Client;
                game.options.headless = true;
                game.native_client_tick_pre();
                let result = answer.lock().unwrap().take().unwrap();
                check(game, result);
            });
        });
    }

    /// From a tick, the confirmation opens holding the link, titled and labelled by `trusted`;
    /// No (or Cancel) closes it and opens nothing.
    #[test]
    #[ignore = "daily"]
    fn a_mod_asks_to_open_a_web_link() {
        ask("https://example.com/a?b=c", false, |game, result| {
            assert_eq!(result, Ok(()));
            let prompt = game.link_prompt.clone().expect("the prompt opens");
            assert_eq!(prompt.url, "https://example.com/a?b=c");
            assert!(!prompt.trusted);
            assert_eq!(prompt.title_key(), "chat.link.confirm");
            assert_eq!(prompt.button_keys(), ["gui.yes", "chat.copy", "gui.no"]);
            let was_playing = game.screen == Screen::Playing;
            game.answer_link_prompt(false);
            assert!(game.link_prompt.is_none());
            assert_eq!(game.screen == Screen::Playing, was_playing);
        });
        ask("https://example.com/a?b=c", true, |game, result| {
            assert_eq!(result, Ok(()));
            let prompt = game.link_prompt.clone().expect("the prompt opens");
            assert!(prompt.trusted);
            assert_eq!(prompt.title_key(), "chat.link.confirmTrusted");
            assert_eq!(
                prompt.button_keys(),
                ["chat.link.open", "chat.copy", "gui.cancel"]
            );
            game.answer_link_prompt(false);
            assert!(game.link_prompt.is_none());
        });
    }

    /// Only http and https open a prompt, the scheme in any case, whatever the chat's link
    /// options say; nothing panics.
    #[test]
    #[ignore = "daily"]
    fn a_mod_link_must_be_http_or_https() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.options.execution_role = super::super::execution_role::ExecutionRole::Client;
            game.options.headless = true;
            (game.settings.chat_links, game.settings.chat_links_prompt) = (false, false);
            assert_eq!(game.confirm_link("HTTPS://Example.com", false), Ok(()));
            assert!(game.link_prompt.is_some());
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
                assert!(game.link_prompt.is_none(), "{bad}");
            }
        });
    }

    /// A path that does not exist is refused before any process starts.
    #[test]
    #[ignore = "daily"]
    fn open_path_refuses_a_missing_path() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.options.execution_role = super::super::execution_role::ExecutionRole::Client;
            game.options.headless = true;
            let missing = std::env::temp_dir().join("sunrise-open-path-missing/never-created");
            assert_eq!(game.open_path(&missing), Err(OpenPathError::NotFound));
        });
    }

    /// Without a native window both methods answer `Unavailable`.
    #[test]
    #[ignore = "daily"]
    fn link_and_path_are_unavailable_without_a_window() {
        for role in [
            super::super::execution_role::ExecutionRole::IntegratedServer,
            super::super::execution_role::ExecutionRole::DedicatedServer,
        ] {
            super::super::native_entity_refs::tests::with_game_role(
                super::super::native_entity_refs::tests::fixture_root(),
                role,
                |game| {
                    assert_eq!(
                        game.confirm_link("https://example.com", false),
                        Err(OpenLinkError::Unavailable)
                    );
                    assert!(game.link_prompt.is_none());
                    assert_eq!(
                        game.open_path(Path::new(".")),
                        Err(OpenPathError::Unavailable)
                    );
                },
            );
        }
    }
}
