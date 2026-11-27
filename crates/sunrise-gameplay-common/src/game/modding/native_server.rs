//! Server commands and players for native mods (docs/modding/src/how-to/native-mods.md,
//! "Server commands and players"): the commands mods add to the
//! server's command tree (`native_register_commands`), a player's command
//! reaching its bound executor, and the helpers a server mod
//! answers with: the players online, a line to one of them or to all, the
//! command's feedback and the world's folder; and the acting
//! player's plain items: count, take, give. They act on the server's own
//! players (the hub's: a dedicated server's, a LAN host's, single player's
//! server thread's), never on a client's.
#[path = "native_server/argument_kinds.rs"]
pub mod argument_kinds;
#[path = "native_server/registered_commands.rs"]
pub mod registered_commands;

use super::api_values::TryIntoEngine;
use argument_kinds::resource_or_tag_parser;
#[allow(unused_imports)]
pub use argument_kinds::{CommandArguments, command_arguments};
#[allow(unused_imports)]
pub use argument_kinds::{
    UNCENTERED, argument_type, bounded_parser, custom_parser, custom_value, dimension_value,
    nbt_value, parsed_type, parser, positional_value, resource_parser,
};
use mod_api::__private::game::*;
#[allow(unused_imports)]
pub use registered_commands::NativeCommandState;
// Their old home, for mixin code that names them here.
use crate::game::{
    api_values::{project_stack, stack_read_metadata},
    commands_common::Slot,
};
#[allow(unused_imports)]
pub use mod_api::{
    ActionOutcome, ActionRefused, FakeEffect, GameMode, GlobalPos, Identifier, PlayerAction,
    PlayerActionSetup, PlayerFood, PlayerRef, PlayerRespawnConfig,
};
use mod_api::{InventoryView, LevelRef};
pub use serde_json::{Map, Value, json};
pub use std::sync::{
    Mutex, PoisonError,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;
pub use std::{collections::HashMap, sync::Arc};
#[allow(unused_imports)]
pub use sunrise_command::{Arg, ArgType, Dispatcher};
#[cfg(test)]
pub use sunrise_core::position::BlockPos;
#[allow(unused_imports)]
use sunrise_game_data::text::Text;
use sunrise_items::{
    equipment::{BODY, SADDLE},
    items::ItemStack,
};
use sunrise_protocol::packets::play::{self, Clientbound as C};
#[allow(unused_imports)]
use sunrise_sim::{
    lan::{abilities::PlayerAbilities, data::component_nbt, guest::Guest},
    motion::PlayerMotion,
};

/// The report's name of an operator level (`commands.json`'s
/// `permissions`).
pub(in crate::game) const LEVELS: [&str; 5] = ["", "moderators", "gamemasters", "admins", "owners"];

/// One parsed word of a usage: its key in the tree and its node, without
/// children.
pub(in crate::game) fn usage_nodes(
    usage: &str,
    custom: &CommandArguments,
    client: bool,
) -> Result<Vec<(String, Value, bool)>, String> {
    let mut out = Vec::new();
    let mut optional_seen = false;
    let mut greedy_seen = false;
    for (index, word) in usage.split_whitespace().enumerate() {
        if greedy_seen {
            return Err(format!("`{usage}`: a text argument must be last"));
        }
        let (inner, optional) = if let Some(w) = word.strip_prefix('<') {
            (w.strip_suffix('>'), false)
        } else if let Some(w) = word.strip_prefix('[') {
            (w.strip_suffix(']'), true)
        } else {
            if optional_seen {
                return Err(format!("`{usage}`: `{word}` follows an optional argument"));
            }
            if !argument_kinds::command_word(word, index == 0) {
                return Err(format!("`{usage}`: `{word}` is not a command word"));
            }
            out.push((word.to_owned(), json!({"type": "literal"}), false));
            continue;
        };
        let Some((name, kind)) = inner.and_then(|w| w.split_once(':')) else {
            return Err(format!("`{usage}`: `{word}` is not `<name:kind>`"));
        };
        if client
            && matches!(
                kind,
                "player" | "players" | "entity" | "entities" | "dimension"
            )
        {
            return Err(format!(
                "`{usage}`: `{kind}` is a server-only argument kind and cannot be used in a client command"
            ));
        }
        if optional_seen && !optional {
            return Err(format!("`{usage}`: `{word}` follows an optional argument"));
        }
        if client && kind == "game_profile" {
            return Err(format!(
                "`{usage}`: `game_profile` requires a server command"
            ));
        }
        let resource = kind
            .strip_prefix("resource:")
            .map(|registry| resource_parser(registry).map_err(|e| format!("`{usage}`: {e}")))
            .transpose()?;
        let resource_or_tag = kind
            .strip_prefix("resource_or_tag:")
            .map(|registry| resource_or_tag_parser(registry).map_err(|e| format!("`{usage}`: {e}")))
            .transpose()?;
        let bounded = bounded_parser(kind)
            .transpose()
            .map_err(|e| format!("`{usage}`: {e}"))?;
        let builtin = resource
            .or(resource_or_tag)
            .or(bounded)
            .or_else(|| parser(kind).map(|(parser, properties, _)| (parser, properties)));
        let (parser, properties, greedy, custom) = match builtin {
            Some((parser, properties)) => (parser, properties, kind == "text", None),
            None => {
                let Some(argument) = custom.get(kind) else {
                    return Err(format!("`{usage}`: unknown kind `{kind}`"));
                };
                let declared =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| argument.kind()))
                        .map_err(|_| format!("`{usage}`: argument type `{kind}` panicked"))?;
                let Some((parser, properties)) = custom_parser(declared) else {
                    return Err(format!(
                        "`{usage}`: argument type `{kind}` uses a kind this build cannot read"
                    ));
                };
                let greedy = properties["type"] == "greedy";
                (parser, properties, greedy, Some(kind))
            }
        };
        optional_seen |= optional;
        greedy_seen |= greedy;
        let mut node = json!({"type": "argument", "parser": parser});
        if !properties.is_null() {
            node["properties"] = properties;
        }
        if matches!(kind, "vec3_uncentered" | "vec2_uncentered") {
            node[UNCENTERED] = true.into();
        }
        // A registered type goes out as the Brigadier or vanilla parser its kind names, which an
        // unmodded client can read; it and the free `id`, which has no list of its own, ask the
        // server (the mods' `tab_complete`).
        if custom.is_some() || kind == "id" {
            if let Some(id) = custom {
                node["custom_argument"] = id.into();
            }
            node["suggestions"] = "minecraft:ask_server".into();
        }
        out.push((name.to_owned(), node, optional));
    }
    if out.first().is_none_or(|n| n.1["type"] != "literal") {
        return Err(format!("`{usage}`: a command starts with its name"));
    }
    Ok(out)
}

/// The tree a mod's usages meet at their root word.
#[derive(Clone, Copy)]
pub enum BaseTree<'a> {
    /// The client's own chat tree: a client command whose root it has (`has` answers) is left
    /// out, so it never hides a server command.
    Reserved(&'a dyn Fn(&str) -> bool),
    /// The server's vanilla tree. A usage whose root vanilla registers merges into that root, as
    /// NeoForge's `RegisterCommandsEvent` registers into vanilla's `CommandDispatcher`, whose
    /// `register` adds the literal with Brigadier's `CommandNode.addChild`: vanilla's usages
    /// stay, the mod's children are added beside them, and vanilla's nodes keep their
    /// permissions. Sunrise's own tools (`/modprofile`, `/pregen`) are not merged into.
    Merged(&'a Dispatcher),
}

/// Native mods' commands as a root in the commands report's format
/// (`lan::commands`, which sends each player what their level allows), and
/// the traversal level of each command a mod owns: the lowest of its usages. A node is runnable
/// where only optional arguments follow it; like Brigadier's `CommandNode.addChild`, each
/// later accepted runnable registration overwrites the node's executor and execution level,
/// a vanilla node's executor too. A root merged into vanilla's keeps vanilla's level, so it has
/// none here. A usage that doesn't parse, meets a vanilla node it cannot share or names a root
/// `base` keeps is left out, with why.
#[cfg(any(test, feature = "test-support"))]
pub fn command_tree(
    commands: &[mod_api::Command],
    vanilla: &Dispatcher,
    custom: &CommandArguments,
) -> (Value, HashMap<String, u8>, Vec<String>) {
    command_tree_for(commands, BaseTree::Merged(vanilla), custom, false, None)
}

/// Build the client's local tree and its executor slots. Server handle kinds are unavailable
/// to client commands. No root is left out: the chat tree takes each one's missing children
/// (`register_command_nodes`).
pub fn client_command_tree(
    commands: &[mod_api::Command],
    custom: &CommandArguments,
    executors: Option<&mut CommandExecutorSlots>,
) -> (Value, HashMap<String, u8>, Vec<String>) {
    command_tree_for(
        commands,
        BaseTree::Reserved(&|_| false),
        custom,
        true,
        executors,
    )
}

/// Whether a usage merges into a vanilla root of `base`, or why it cannot go into the tree. Each
/// word under a merged root that vanilla has at the same place must be a literal on a literal:
/// Brigadier would keep vanilla's argument type for the mod's argument of the same name, and the
/// mod's executor would read a value of another kind.
pub(in crate::game) fn base_root(
    base: BaseTree<'_>,
    usage: &str,
    nodes: &[(String, Value, bool)],
) -> Result<bool, String> {
    let root = nodes[0].0.as_str();
    let vanilla = match base {
        BaseTree::Reserved(has) if has(root) => {
            return Err(format!("`{usage}`: vanilla has /{root}"));
        }
        BaseTree::Reserved(_) => return Ok(false),
        BaseTree::Merged(vanilla) => vanilla,
    };
    let Some(mut at) = vanilla.find(root) else {
        return Ok(false);
    };
    if sunrise_command::vanilla::is_session_tool(root) {
        return Err(format!("`{usage}`: /{root} is the game's own command"));
    }
    for (name, node, _) in &nodes[1..] {
        let Some(child) = vanilla.nodes[at]
            .children
            .iter()
            .copied()
            .find(|&child| vanilla.nodes[child].name() == name)
        else {
            break;
        };
        let literal = matches!(
            vanilla.nodes[child].kind,
            sunrise_command::NodeKind::Literal(_)
        );
        if !literal || node["type"] != "literal" {
            return Err(format!(
                "`{usage}`: `{name}` is a node of vanilla's /{root} that a mod argument cannot share"
            ));
        }
        at = child;
    }
    Ok(true)
}

// Executable paths (including argument names) mapped to the input registration slot.
pub(in crate::game) type CommandExecutorSlots = HashMap<Vec<String>, usize>;

pub fn command_tree_for(
    commands: &[mod_api::Command],
    base: BaseTree<'_>,
    custom: &CommandArguments,
    client: bool,
    mut executors: Option<&mut CommandExecutorSlots>,
) -> (Value, HashMap<String, u8>, Vec<String>) {
    let mut root: Map<String, Value> = Map::new();
    let mut levels: HashMap<String, u8> = HashMap::new();
    let mut problems = Vec::new();
    for (slot, command) in commands.iter().enumerate() {
        let nodes = match usage_nodes(&command.usage, custom, client) {
            Ok(n) => n,
            Err(e) => {
                problems.push(e);
                continue;
            }
        };
        let merged = match base_root(base, &command.usage, &nodes) {
            Ok(merged) => merged,
            Err(e) => {
                problems.push(e);
                continue;
            }
        };
        // A rejected usage must not open an earlier executable prefix or lower its level.
        let mut children = &root;
        let mut conflict = None;
        for (name, node, _) in &nodes {
            let Some(entry) = children.get(name) else {
                break;
            };
            // `word` and `text` share a parser; their properties tell them apart, and a custom
            // kind shares the word (or greedy) properties with them.
            if entry["type"] != node["type"]
                || entry["parser"] != node["parser"]
                || entry["properties"] != node["properties"]
                || entry["custom_argument"] != node["custom_argument"]
                || entry[UNCENTERED] != node[UNCENTERED]
            {
                conflict = Some(name);
                break;
            }
            children = entry["children"]
                .as_object()
                .expect("working tree children");
        }
        if let Some(name) = conflict {
            problems.push(format!(
                "`{}`: `{name}` is another kind in an earlier usage",
                command.usage
            ));
            continue;
        }
        let execution_level = u64::from(command.level.min(4));
        let mut children = &mut root;
        for (i, (name, node, _)) in nodes.iter().enumerate() {
            let runnable = nodes.get(i + 1).is_none_or(|n| n.2);
            let entry = children.entry(name.clone()).or_insert_with(|| {
                let mut n = node.clone();
                n["executable"] = false.into();
                n["level"] = 4.into();
                n
            });
            // Like Brigadier's `CommandNode.addChild`, each later accepted runnable
            // registration overwrites the node's executor and execution level.
            if runnable {
                entry["executable"] = true.into();
                entry["execution_level"] = execution_level.into();
                if let Some(executors) = executors.as_deref_mut() {
                    let path = nodes[..=i]
                        .iter()
                        .map(|(name, _, _)| name.clone())
                        .collect();
                    executors.insert(path, slot);
                }
            }
            let level = entry["level"]
                .as_u64()
                .unwrap_or(4)
                .min(command.level.into());
            entry["level"] = level.into();
            if !entry["children"].is_object() {
                entry["children"] = Value::Object(Map::new());
            }
            children = entry["children"].as_object_mut().expect("made above");
        }
        if !merged {
            let level = levels.entry(nodes[0].0.clone()).or_insert(4);
            *level = (*level).min(command.level);
        }
    }
    let tree = if root.is_empty() {
        Value::Null
    } else {
        let mut root = Value::Object(root);
        finish(&mut root);
        json!({"type": "root", "children": root})
    };
    (tree, levels, problems)
}

/// Turns the working `level` into the report's `permissions`, and drops
/// empty child lists.
pub(in crate::game) fn finish(children: &mut Value) {
    let Some(map) = children.as_object_mut() else {
        return;
    };
    for node in map.values_mut() {
        let level = node["level"].as_u64().unwrap_or(0) as usize;
        let object = node.as_object_mut().expect("a node");
        object.remove("level");
        if level > 0 {
            object.insert(
                "permissions".into(),
                json!({"type": "require", "permission": {"level": LEVELS[level.min(4)]}}),
            );
        }
        if object
            .get("children")
            .is_some_and(|c| c.as_object().is_some_and(Map::is_empty))
        {
            object.remove("children");
        } else if let Some(c) = object.get_mut("children") {
            finish(c);
        }
    }
}

/// The result of a command handled by the client's dispatcher, never sent to the server.
#[derive(Debug, PartialEq, Eq)]
pub enum ClientCommandOutcome {
    Failed,
    Success(i32),
}

/// Add a validated native report for descriptor-only consumers.
pub fn register_dispatcher(
    dispatcher: &mut Dispatcher,
    tree: &Value,
    handler: &'static str,
    arguments: &CommandArguments,
) {
    register_command_nodes(dispatcher, tree, handler, arguments, None);
}

/// Add the report's nodes with the same grammar on both sides. The authoritative
/// server also associates each runnable node ID with its registration path.
pub fn register_command_nodes(
    dispatcher: &mut Dispatcher,
    tree: &Value,
    handler: &'static str,
    arguments: &CommandArguments,
    executors: Option<&CommandExecutorSlots>,
) {
    fn add(
        dispatcher: &mut Dispatcher,
        parent: usize,
        children: &Map<String, Value>,
        handler: &'static str,
        arguments: &CommandArguments,
        path: &mut Vec<String>,
        on_executable: &mut impl FnMut(&[String], usize),
    ) {
        for (name, node) in children {
            path.push(name.clone());
            let before = dispatcher.nodes.len();
            let id = if node["type"] == "literal" {
                dispatcher.literal(parent, name)
            } else {
                dispatcher.argument(parent, name, argument_type(node, arguments))
            };
            // `CommandNode.addChild` merges into a node vanilla already has, which keeps its
            // own requirement; a node the mod adds takes the usage's level.
            if dispatcher.nodes.len() > before {
                let level = node["permissions"]["permission"]["level"]
                    .as_str()
                    .and_then(|level| LEVELS.iter().position(|known| *known == level))
                    .unwrap_or(0) as u8;
                dispatcher.requires(id, level);
            }
            if node["executable"] == true {
                dispatcher.executes(id, handler);
                dispatcher.requires_execution(
                    id,
                    node["execution_level"]
                        .as_u64()
                        .expect("runnable usage level") as u8,
                );
                on_executable(path, id);
            }
            if let Some(children) = node["children"].as_object() {
                add(
                    dispatcher,
                    id,
                    children,
                    handler,
                    arguments,
                    path,
                    on_executable,
                );
            }
            path.pop();
        }
    }
    let mut bindings = HashMap::new();
    if let Some(children) = tree["children"].as_object() {
        add(
            dispatcher,
            0,
            children,
            handler,
            arguments,
            &mut Vec::new(),
            &mut |path: &[String], id| {
                if let Some(executors) = executors {
                    bindings.insert(id, executors[path]);
                }
            },
        );
    }
    if executors.is_some() {
        dispatcher.replace_executor_bindings(bindings);
    }
}

/// The live player body, or a guest record outside a player swap.
pub enum PlayerRead<'a> {
    Acting {
        /// Server authoritative mode, or the local player's resolved `PlayerInfo` mode on a client.
        mode: Option<i32>,
        body: Option<&'a PlayerMotion>,
        /// The player's stored abilities, when it is a server player's turn.
        abilities: Option<PlayerAbilities>,
        /// Where the player last died: the acting guest's record, swapped
        /// into the level for their turn.
        last_death: Option<&'a (String, [i32; 3])>,
    },
    Guest(&'a Guest),
}

impl PlayerRead<'_> {
    pub(in crate::game) fn last_death(&self) -> Option<&(String, [i32; 3])> {
        match self {
            Self::Acting { last_death, .. } => *last_death,
            Self::Guest(guest) => guest.last_death.as_ref(),
        }
    }

    pub(in crate::game) fn mode(&self) -> Option<i32> {
        match self {
            Self::Acting { mode, .. } => *mode,
            Self::Guest(guest) => Some(guest.game_mode),
        }
    }

    pub(in crate::game) fn creative(&self) -> bool {
        self.mode() == Some(1)
    }

    pub(in crate::game) fn may_build(&self) -> bool {
        match self {
            Self::Acting {
                abilities: Some(a), ..
            } => a.may_build,
            Self::Guest(guest) => guest.abilities.may_build,
            Self::Acting { mode, .. } => mode.is_some_and(|m| m < 2),
        }
    }

    pub(in crate::game) fn body(&self) -> Option<&PlayerMotion> {
        match self {
            Self::Acting { body, .. } => *body,
            Self::Guest(guest) => guest.motion.as_deref(),
        }
    }

    pub(in crate::game) fn food(&self) -> Option<PlayerFood> {
        let food = &self.body()?.health.food;
        let (level, saturation) = match self {
            Self::Acting { .. } => (food.food, food.saturation),
            Self::Guest(guest) => (guest.food, guest.saturation),
        };
        Some(PlayerFood::from(mod_api::__private::built::PlayerFood {
            level,
            saturation,
            exhaustion: food.exhaustion,
        }))
    }

    pub fn abilities(&self) -> Option<mod_api::PlayerAbilities> {
        let a = match self {
            Self::Acting {
                abilities: Some(a), ..
            } => *a,
            Self::Acting { mode, body, .. } => {
                // No server turn: what the mode gives, with the body's flight.
                let body = body.as_ref()?;
                let mut a = PlayerAbilities {
                    flying: body.flying,
                    flying_speed: body.flying_speed,
                    ..Default::default()
                };
                a.update_for_mode(mode.unwrap_or(0));
                a.flying = body.flying;
                a
            }
            Self::Guest(guest) => guest.abilities,
        };
        Some(mod_api::PlayerAbilities::from(
            mod_api::__private::built::PlayerAbilities {
                invulnerable: a.invulnerable,
                flying: a.flying,
                may_fly: a.may_fly,
                instabuild: a.instabuild,
                may_build: a.may_build,
                fly_speed: a.flying_speed,
                walk_speed: a.walking_speed,
            },
        ))
    }
}

/// Helpers for mods, kept as stable as the hook points.
// Native mods use these; nothing in Sunrise does.
#[allow(dead_code)]
impl crate::game::CommonGame<'_> {
    /// The server's tick rate and average tick duration in milliseconds.
    pub fn tick_stats(&self) -> (f32, f64) {
        (
            self.clock.rate(),
            self.clock.average_nanos() as f64 / 1_000_000.0,
        )
    }

    /// `TickRateManager.isFrozen`; None on a client, which follows its server's.
    pub fn tick_rate_is_frozen(&self) -> Option<bool> {
        self.is_logical_server().then(|| self.clock.frozen())
    }
}

/// Helpers for mods, kept as stable as the hook points.
// Native mods use these; nothing in Sunrise does.
#[allow(dead_code)]
pub trait NativeServerGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// Resolve a real online player by UUID, never a fake.
    fn player(&self, uuid: u128) -> Option<PlayerRef> {
        self.side_player(uuid)
    }

    /// The dimension id and block of the player's last death, through the shared player resolver.
    fn native_player_last_death_record(&self, player: PlayerRef) -> Option<(String, [i32; 3])> {
        self.player_read_both(player)?.last_death().cloned()
    }

    /// `player_handle_read` for the reads that work on both sides.
    fn player_read_both(&self, player: PlayerRef) -> Option<PlayerRead<'_>> {
        self.side_player_read_both(player)
    }

    /// The dimension key of an online player (`ServerPlayer.level().dimension()`).
    fn player_dimension_key(&self, player: PlayerRef) -> Option<mod_api::DimensionKey> {
        self.side_player_dimension_key(player)
    }

    fn player_collision_flags(
        &self,
        player: PlayerRef,
    ) -> Result<Option<mod_api::CollisionFlags>, mod_api::LevelReadError> {
        self.side_player_collision_flags(player)
    }

    fn player_supporting_block(
        &self,
        player: PlayerRef,
    ) -> Result<Option<mod_api::BlockPos>, mod_api::LevelReadError> {
        Ok(self
            .player_read_both(player)
            .and_then(|read| read.body()?.ground.supporting_block))
    }

    fn player_on_block(
        &self,
        player: PlayerRef,
    ) -> Result<Option<mod_api::BlockPos>, mod_api::LevelReadError> {
        Ok(self.player_read_both(player).and_then(|read| {
            let body = read.body();
            let position = match read {
                PlayerRead::Acting { .. } => body?.position,
                PlayerRead::Guest(guest) => {
                    let [x, y, z] = guest.position;
                    sunrise_core::geometry::Vec3::new(x, y, z)
                }
            };
            let [x, y, z] = crate::game::native_entity_motion_values::on_block_position(
                body.and_then(|motion| motion.ground.supporting_block),
                position,
            );
            Some(mod_api::BlockPos::new(x, y, z))
        }))
    }

    /// `ServerPlayerGameMode.getGameModeForPlayer` on a server; `AbstractClientPlayer.gameMode` on a client.
    fn player_game_mode(&self, player: PlayerRef) -> Option<GameMode> {
        self.player_read_both(player)?
            .mode()
            .and_then(GameMode::from_id)
    }

    /// `Player.isCreative`; Paper `HumanEntity#getGameMode`.
    fn player_is_creative(&self, player: PlayerRef) -> Option<bool> {
        Some(self.player_read_both(player)?.creative())
    }

    /// `Inventory.getSelectedSlot`: the hotbar slot the player holds, 0..9.
    fn player_selected_slot(&self, player: PlayerRef) -> Option<usize> {
        Some(match self.player_read_both(player)? {
            PlayerRead::Acting { .. } => self.selected,
            PlayerRead::Guest(guest) => guest.selected,
        })
    }

    /// `Player.isSpectator`.
    fn player_is_spectator(&self, player: PlayerRef) -> Option<bool> {
        Some(self.player_read_both(player)?.mode() == Some(3))
    }

    /// `Player.mayBuild`; Paper `HumanEntity#getGameMode`.
    fn player_may_build(&self, player: PlayerRef) -> Option<bool> {
        Some(self.player_read_both(player)?.may_build())
    }

    /// `FoodData.getFoodLevel/getSaturationLevel/getExhaustionLevel`.
    fn player_food(&self, player: PlayerRef) -> Option<PlayerFood> {
        self.player_read_both(player)?.food()
    }

    /// The level the online player is in (`ServerPlayer.level`).
    fn player_level(&self, player: PlayerRef) -> Option<LevelRef> {
        let dim = self.player_dimension_key(player)?;
        Some(self.side_level_ref(player.fields().entity.epoch(), dim))
    }

    /// The dimension the online player is in (`ServerPlayer.level().dimension()`).
    fn player_dimension(&self, player: PlayerRef) -> Option<Identifier> {
        let level = self.player_level(player)?;
        Identifier::parse(level.dimension_id()).ok()
    }

    /// The brand the player's client announced when it connected (`ServerPlayer.clientBrand`).
    fn player_client_brand(&self, player: PlayerRef) -> Option<&str> {
        self.side_player_client_brand(player)
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeServerGameExt1<'__sg> for G {}

/// Whether an orb at `position` (its box: 0.5 wide and high, standing on
/// it) meets the box `min`..`max`, touching faces excluded (`AABB.intersects`).
pub fn orb_meets(position: [f64; 3], min: [f64; 3], max: [f64; 3]) -> bool {
    position[0] - 0.25 < max[0]
        && position[0] + 0.25 > min[0]
        && position[1] < max[1]
        && position[1] + 0.5 > min[1]
        && position[2] - 0.25 < max[2]
        && position[2] + 0.25 > min[2]
}

/// The box `min`..`max` with each axis's pair in order, as vanilla's `AABB` takes it;
/// None when a bound is not finite (NaN or infinite).
pub fn ordered_box(min: [f64; 3], max: [f64; 3]) -> Option<([f64; 3], [f64; 3])> {
    if !min.iter().chain(&max).all(|v| v.is_finite()) {
        return None;
    }
    Some((
        std::array::from_fn(|i| min[i].min(max[i])),
        std::array::from_fn(|i| min[i].max(max[i])),
    ))
}

pub fn system_chat(line: &Text) -> C {
    C::SystemChat(play::SystemChat {
        content: component_nbt(&line.component()),
        overlay: false,
    })
}

/// The temporary state of the same persistent guest, not another player or action engine.
/// Swapped back after the ordinary guest action has restored the server's outer state.
pub struct ScopedPlayerState {
    pub(in crate::game) motion: Option<Box<PlayerMotion>>,
    pub(in crate::game) position: [f64; 3],
    pub(in crate::game) yaw: f32,
    pub(in crate::game) pitch: f32,
    pub(in crate::game) head_yaw: f32,
    pub(in crate::game) on_ground: bool,
    pub(in crate::game) game_mode: i32,
    pub(in crate::game) previous_game_mode: Option<i32>,
    pub(in crate::game) abilities: PlayerAbilities,
    pub(in crate::game) sneaking: bool,
    pub(in crate::game) sprinting: bool,
    pub(in crate::game) fall_flying: bool,
    pub(in crate::game) inventory: Vec<Option<sunrise_items::items::ItemStack>>,
    pub(in crate::game) selected: usize,
    pub(in crate::game) armor: [Option<sunrise_items::items::ItemStack>; 4],
    pub(in crate::game) offhand: Option<sunrise_items::items::ItemStack>,
    pub(in crate::game) carried: Option<sunrise_items::items::ItemStack>,
    pub(in crate::game) equipment: sunrise_items::equipment::Equipment,
    pub(in crate::game) effects: sunrise_level::effects::PlayerEffects,
    pub(in crate::game) health: f32,
    pub(in crate::game) food: i32,
    pub(in crate::game) saturation: f32,
}
impl ScopedPlayerState {
    pub fn new(
        resources: &sunrise_sim::motion::MotionResources,
        items: &sunrise_items::items::Items,
        setup: PlayerActionSetup,
    ) -> Result<Self, ActionRefused> {
        // Standing eye height even for secondary use, just as a freshly positioned fake.
        let position = [
            setup.eye[0],
            setup.eye[1] - f64::from(1.62_f32),
            setup.eye[2],
        ];
        let mut motion = PlayerMotion::new(
            resources,
            sunrise_core::geometry::Vec3::new(position[0], position[1], position[2]),
        )
        .map_err(|_| ActionRefused::InvalidSetup)?;
        motion.shift_key_down = setup.secondary_use;
        let mut inventory = vec![None; 36];
        inventory[0] = setup
            .stack
            .try_into_engine(items)
            .map_err(ActionRefused::InvalidStack)?;
        Ok(Self {
            motion: Some(Box::new(motion)),
            position,
            yaw: setup.yaw,
            pitch: setup.pitch,
            head_yaw: setup.yaw,
            on_ground: false,
            game_mode: 0,
            previous_game_mode: None,
            abilities: PlayerAbilities::default(),
            sneaking: setup.secondary_use,
            sprinting: false,
            fall_flying: false,
            inventory,
            selected: 0,
            armor: Default::default(),
            offhand: None,
            carried: None,
            equipment: Default::default(),
            effects: Default::default(),
            health: 20.0,
            food: 20,
            saturation: 5.0,
        })
    }

    pub fn swap(&mut self, guest: &mut Guest) {
        std::mem::swap(&mut self.motion, &mut guest.motion);
        std::mem::swap(&mut self.position, &mut guest.position);
        std::mem::swap(&mut self.yaw, &mut guest.yaw);
        std::mem::swap(&mut self.pitch, &mut guest.pitch);
        std::mem::swap(&mut self.head_yaw, &mut guest.head_yaw);
        std::mem::swap(&mut self.on_ground, &mut guest.on_ground);
        std::mem::swap(&mut self.game_mode, &mut guest.game_mode);
        std::mem::swap(&mut self.previous_game_mode, &mut guest.previous_game_mode);
        std::mem::swap(&mut self.abilities, &mut guest.abilities);
        std::mem::swap(&mut self.sneaking, &mut guest.sneaking);
        std::mem::swap(&mut self.sprinting, &mut guest.sprinting);
        std::mem::swap(&mut self.fall_flying, &mut guest.fall_flying);
        std::mem::swap(&mut self.inventory, &mut guest.inventory);
        std::mem::swap(&mut self.selected, &mut guest.selected);
        std::mem::swap(&mut self.armor, &mut guest.armor);
        std::mem::swap(&mut self.offhand, &mut guest.offhand);
        std::mem::swap(&mut self.carried, &mut guest.carried);
        std::mem::swap(&mut self.equipment, &mut guest.equipment);
        std::mem::swap(&mut self.effects, &mut guest.effects);
        std::mem::swap(&mut self.health, &mut guest.health);
        std::mem::swap(&mut self.food, &mut guest.food);
        std::mem::swap(&mut self.saturation, &mut guest.saturation);
    }
}

thread_local! {
    static LEVEL_TICK_LISTENERS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Marks a server level tick's listeners while they run, with nothing open around the one
/// transaction their dispatch opens for each (#5750): a fake player's action may run inside
/// that one (`player_perform_action`), not inside one the listener opened itself. Dropping it
/// puts back the mark it replaced, also when the listeners unwind.
pub struct LevelTickListeners(bool);

impl LevelTickListeners {
    /// Sets the mark to `on` until the returned guard drops.
    pub fn mark(on: bool) -> Self {
        Self(LEVEL_TICK_LISTENERS.with(|mark| mark.replace(on)))
    }
    /// Whether a level tick's listeners are running, marked.
    pub fn marked() -> bool {
        LEVEL_TICK_LISTENERS.with(std::cell::Cell::get)
    }
}

impl Drop for LevelTickListeners {
    fn drop(&mut self) {
        LEVEL_TICK_LISTENERS.with(|mark| mark.set(self.0));
    }
}

/// A fake player's `perform_action` in progress: set for its length by [`Acting`], whose drop
/// clears it on every exit, an early return or a panic unwinding through a mod's hook, and the
/// effect it stopped, cleared as each action starts.
#[derive(Default)]
pub struct FakeAction {
    pub(in crate::game) acting: AtomicBool,
    /// The action is `PlayerRef::attack`: `Player.attack` alone, without its packet's swing.
    unswung: AtomicBool,
    /// The first effect stopped since the action started.
    pub(in crate::game) blocked: Mutex<Option<FakeEffect>>,
}
impl FakeAction {
    /// Whether a fake player's action is running.
    pub fn acting(&self) -> bool {
        self.acting.load(Ordering::Relaxed)
    }
    /// Whether the action running is a mod's `PlayerRef::attack`, which `swing_arm` leaves
    /// without a swing: vanilla's `Player.attack` has none, the swing is its packet's.
    pub fn unswung(&self) -> bool {
        self.unswung.load(Ordering::Relaxed)
    }
    pub fn block(&self, effect: FakeEffect) {
        // The first effect stopped is the one the action answers.
        self.blocked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert(effect);
    }
}
/// Marks a fake player's action as running until it drops.
pub struct Acting(pub(in crate::game) Arc<FakeAction>);
impl Acting {
    pub fn start(action: &Arc<FakeAction>) -> Self {
        *action
            .blocked
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        action.acting.store(true, Ordering::Relaxed);
        Self(Arc::clone(action))
    }
    /// [`Self::start`] for a mod's `PlayerRef::attack`, which runs without a swing.
    pub fn start_unswung(action: &Arc<FakeAction>) -> Self {
        action.unswung.store(true, Ordering::Relaxed);
        Self::start(action)
    }
    /// The effect this action stopped, if any.
    pub fn blocked(&self) -> Option<FakeEffect> {
        *self
            .0
            .blocked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}
impl Drop for Acting {
    fn drop(&mut self) {
        self.0.acting.store(false, Ordering::Relaxed);
        self.0.unswung.store(false, Ordering::Relaxed);
    }
}

/// The online mode a dedicated server's network config carries: it has an
/// `OnlineMode` key (and so signs its logins) only in online mode.
pub fn online_mode_of(config: &sunrise_sim::lan::NetConfig) -> bool {
    config.online.is_some()
}

/// `Inventory`'s slot numbers past the 36 main slots (`EQUIPMENT_SLOT_MAPPING`).
pub const SLOTS: u32 = 43;

/// 0..35 main (0..8 hotbar), 36..39 feet to head, 40 offhand, 41 body, 42 saddle.
/// ItemPlace uses 100..103 for armour and -106 for offhand; InventoryMenu has no 41/42.
/// The slot the game keeps `Inventory` slot `slot` in.
pub fn inventory_slot(slot: u32) -> Option<Slot> {
    Some(match slot {
        0..36 => Slot::Inv(slot as usize),
        // `armor` is head first: 36 feet is its last.
        36..40 => Slot::Armor((39 - slot) as usize),
        40 => Slot::Offhand,
        41 => Slot::Equipment(BODY),
        42 => Slot::Equipment(SADDLE),
        _ => return None,
    })
}

/// Lend `Inventory.getItem` over a player's 43 vanilla inventory slots.
pub fn lend_inventory_view(
    inventory: &[Option<ItemStack>],
    armor: &[Option<ItemStack>; 4],
    offhand: &Option<ItemStack>,
    equipment: &sunrise_items::equipment::Equipment,
    read: &mut dyn for<'a> FnMut(InventoryView<'a>),
) {
    let stack_at = |slot| match inventory_slot(slot)? {
        Slot::Inv(i) => inventory.get(i)?.as_ref(),
        Slot::Armor(i) => armor[i].as_ref(),
        Slot::Offhand => offhand.as_ref(),
        Slot::Equipment(i) => equipment.get(i),
        _ => None,
    };
    read(InventoryView::new(
        0..SLOTS,
        &|slot| stack_read_metadata(stack_at(slot)?).ok(),
        &|slot, metadata| project_stack(stack_at(slot)?, metadata).ok(),
    ));
}

pub trait NativeServerGameExt2<'__sg>: crate::game::SideGame<'__sg> {
    /// `ItemCooldowns.getCooldownGroup(stack)`: the group `stack`'s
    /// cooldown is filed under, the group its `use_cooldown` component names
    /// (its own, else its item's default), else its item id.
    fn item_cooldown_group(&self, stack: &ItemStack) -> String {
        let default = self
            .items()
            .get(&stack.item)
            .and_then(|info| info.use_cooldown.as_ref());
        sunrise_items::cooldowns::group(stack, default).to_string()
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeServerGameExt2<'__sg> for G {}

pub trait NativeServerGameExt3<'__sg>: crate::game::SideGame<'__sg> {
    /// `Player.isSecondaryUseActive`: whether the player uses items and
    /// blocks as with sneak held, as vanilla reads it from the shift-key
    /// flag (the same read as `EntityRef::is_shift_key_down`). None for a
    /// stale or unavailable player.
    fn player_is_secondary_use_active(&self, player: PlayerRef) -> Option<bool> {
        self.entity_is_shift_key_down(player.entity())
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeServerGameExt3<'__sg> for G {}

pub trait NativeServerGameExt4<'__sg>: crate::game::SideGame<'__sg> {
    /// `MinecraftServer.haveTime`: the current server tick has not reached the
    /// moment the next one is due. A client, and a server outside a scheduled
    /// tick, always has time, as NeoForge's client `LevelTickEvent.hasTime` does.
    fn server_has_time(&self) -> bool {
        !self.is_logical_server()
            || crate::game::Side::server_tick_deadline(self.side_state())
                .is_none_or(|end| Instant::now() < end)
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeServerGameExt4<'__sg> for G {}

pub trait NativeServerGameExt5<'__sg>: crate::game::SideGame<'__sg> {
    /// Lend `Inventory.getItem` over the resident slots.
    fn player_inventory_slots(
        &self,
        player: PlayerRef,
        read: &mut dyn for<'a> FnMut(InventoryView<'a>),
    ) -> Option<()> {
        let (inventory, armor, offhand, equipment) = match self.player_read_both(player)? {
            PlayerRead::Acting { .. } => {
                (&self.inventory, &self.armor, &self.offhand, &self.equipment)
            }
            PlayerRead::Guest(g) => (&g.inventory, &g.armor, &g.offhand, &g.equipment),
        };
        lend_inventory_view(inventory, armor, offhand, equipment, read);
        Some(())
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeServerGameExt5<'__sg> for G {}

use crate::game::side_ops::modapi_modding::ModapiModdingSideGameExt as _;

use crate::game::tables::GameItems as _;

use crate::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(test)]
#[path = "native_server/provider_tests.rs"]
pub(in crate::game) mod provider_tests;

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use super::*;

    #[cfg(any(test, feature = "test-support"))]
    pub type Heard = Vec<(&'static str, u32, u128, &'static str, [i32; 2])>;
    #[cfg(any(test, feature = "test-support"))]
    /// Logs each of the watch trio: (hook, player id, UUID, level, chunk).
    pub struct Watcher(pub std::sync::Arc<std::sync::Mutex<Heard>>);
    #[cfg(any(test, feature = "test-support"))]
    impl Watcher {
        pub fn log(&self, hook: &'static str, player: PlayerRef, chunk: mod_api::ChunkRef) {
            self.0.lock().unwrap().push((
                hook,
                player.entity().id(),
                player.uuid(),
                chunk.level().dimension_id(),
                chunk.pos(),
            ));
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Watcher {
        fn chunk_watch(&self, _: &mut dyn mod_api::ServerHost, e: &mut mod_api::ChunkWatchEvent) {
            self.log("watch", e.player, e.chunk);
        }
        fn chunk_sent(&self, _: &mut dyn mod_api::ServerHost, e: &mut mod_api::ChunkSentEvent) {
            self.log("sent", e.player, e.chunk);
        }
        fn chunk_unwatch(
            &self,
            _: &mut dyn mod_api::ServerHost,
            e: &mut mod_api::ChunkUnwatchEvent,
        ) {
            self.log("unwatch", e.player, e.chunk);
        }
    }

    pub fn player_guest() -> Guest {
        use std::sync::{
            Arc,
            atomic::{AtomicI32, AtomicUsize},
        };
        use sunrise_protocol::packets::configuration::ClientInformation;
        use sunrise_sim::lan::{Joined, PlayerSender};
        let (tx, _rx) = std::sync::mpsc::channel();
        Guest::new(
            Joined {
                conn: 1,
                uuid: 123,
                name: "guest".into(),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            1,
            [0.0; 3],
            10,
        )
    }

    #[cfg(test)]
    #[test]
    fn player_reads_use_guest_mode_and_abilities() {
        let mut guest = player_guest();
        guest.abilities.flying_speed = 0.08;
        let read = PlayerRead::Guest(&guest);
        assert_eq!(read.mode(), Some(0));
        assert_eq!(
            read.food(),
            None,
            "food is unavailable before body creation"
        );
        assert_eq!(
            read.abilities(),
            Some(mod_api::PlayerAbilities::from(
                mod_api::__private::built::PlayerAbilities {
                    invulnerable: false,
                    flying: false,
                    may_fly: false,
                    instabuild: false,
                    may_build: true,
                    fly_speed: 0.08,
                    walk_speed: 0.1,
                }
            ))
        );
        guest.game_mode = 1;
        guest.abilities.update_for_mode(1);
        guest.abilities.flying = true;
        let read = PlayerRead::Guest(&guest);
        assert_eq!(read.mode(), Some(1));
        assert_eq!(
            read.abilities(),
            Some(mod_api::PlayerAbilities::from(
                mod_api::__private::built::PlayerAbilities {
                    invulnerable: true,
                    flying: true,
                    may_fly: true,
                    instabuild: true,
                    may_build: true,
                    fly_speed: 0.08,
                    walk_speed: 0.1,
                }
            ))
        );
        guest.game_mode = 3;
        guest.abilities.update_for_mode(3);
        let abilities = PlayerRead::Guest(&guest).abilities().unwrap();
        assert!(!abilities.may_build && !abilities.instabuild && abilities.may_fly);
    }

    #[cfg(test)]
    pub(in crate::game) fn tree(
        commands: &[mod_api::Command],
    ) -> (Value, HashMap<String, u8>, Vec<String>) {
        command_tree(commands, &Dispatcher::new(), &HashMap::new())
    }

    #[cfg(test)]
    pub(in crate::game) fn stack(item: &str, count: u32) -> Option<ItemStack> {
        Some(ItemStack {
            item: item.into(),
            count,
            damage: 0,
            extra: None,
        })
    }

    #[cfg(test)]
    #[test]
    fn orbs_are_found_by_their_half_block_box() {
        // A block's cell, as a mod asks for a broken block's drops.
        let (min, max) = ([10.0, 64.0, -3.0], [11.0, 65.0, -2.0]);
        assert!(orb_meets([10.5, 64.2, -2.5], min, max), "inside");
        assert!(
            orb_meets([11.2, 64.0, -2.5], min, max),
            "its box reaches in"
        );
        assert!(
            orb_meets([10.5, 63.6, -2.5], min, max),
            "standing just below"
        );
        assert!(!orb_meets([11.25, 64.0, -2.5], min, max), "a face touching");
        assert!(!orb_meets([10.5, 63.5, -2.5], min, max), "its top touching");
        assert!(!orb_meets([10.5, 65.0, -2.5], min, max), "standing on top");
    }

    #[cfg(test)]
    #[test]
    fn whole_stacks_are_given_onto_equal_stacks_only() {
        use super::super::inventory_add;
        let mut inv = vec![stack("minecraft:dirt", 64); 36];
        inv[2] = stack("minecraft:iron_sword", 1);
        inv[2].as_mut().unwrap().damage = 5;
        inv[5] = stack("minecraft:arrow", 60);
        inv[7] = None;
        inv[20] = None;
        let mut off = stack("minecraft:arrow", 62);

        // A damaged sword never joins another stack: the first free slot.
        let mut sword = stack("minecraft:iron_sword", 1).unwrap();
        sword.damage = 40;
        assert!(!inventory_add(&mut inv, &mut off, 5, 1, &mut sword));
        assert_eq!(inv[7].as_ref().unwrap().damage, 40, "its damage is kept");
        assert_eq!(inv[2].as_ref().unwrap().damage, 5);

        // Arrows: the selected slot, then the off hand, then a free slot,
        // and what is left over comes back.
        let mut arrows = stack("minecraft:arrow", 80).unwrap();
        assert!(inventory_add(&mut inv, &mut off, 5, 64, &mut arrows));
        assert_eq!(inv[5].as_ref().unwrap().count, 64);
        assert_eq!(off.as_ref().unwrap().count, 64);
        assert_eq!(inv[20].as_ref().unwrap().count, 64);
        assert_eq!(arrows.count, 80 - 4 - 2 - 64);

        // A full inventory with an empty off hand takes nothing.
        let mut off = None;
        let mut apple = stack("minecraft:apple", 3).unwrap();
        assert!(inventory_add(&mut inv, &mut off, 0, 64, &mut apple));
        assert!(off.is_none(), "never into an empty off hand (getFreeSlot)");
        assert_eq!(apple.count, 3);
    }

    #[cfg(test)]
    /// The game's `Inventory.add` (pickups, `/give`, a bucket's or a bottle's result) takes
    /// its slots from `getSlotWithRemainingSpace` and `getFreeSlot`
    /// (`sunrise_client::menu::{slot_with_remaining_space, free_slot}`): one dirt at a time
    /// lands where they answer and nowhere else, in vanilla's order (the selected slot, the
    /// off hand, the main slots, then the first empty one).
    #[test]
    fn every_added_item_lands_where_the_slot_pickers_answer() {
        use super::super::inventory_add;
        use sunrise_sim::menu::{OFFHAND, free_slot, slot_with_remaining_space};
        let mut inv = vec![stack("minecraft:stone", 64); 36];
        for (slot, have) in [
            (0, stack("minecraft:stone", 5)),
            (1, None),
            (2, None),
            (3, stack("minecraft:dirt", 63)),
            (7, stack("minecraft:dirt", 63)),
            (20, None),
        ] {
            inv[slot] = have;
        }
        let mut off = stack("minecraft:dirt", 62);
        let selected = 3;
        let dirt = stack("minecraft:dirt", 1).unwrap();
        let mut landed = Vec::new();
        for _ in 0..6 {
            let answer = slot_with_remaining_space(&inv, off.as_ref(), selected, &dirt, 64)
                .or_else(|| free_slot(&inv))
                .unwrap();
            let (mut inv_after, mut off_after) = (inv.clone(), off.clone());
            let cell = if answer == OFFHAND {
                &mut off_after
            } else {
                &mut inv_after[answer]
            };
            match cell {
                Some(have) => have.count += 1,
                empty => *empty = Some(dirt.clone()),
            }
            let mut one = dirt.clone();
            assert!(!inventory_add(&mut inv, &mut off, selected, 64, &mut one));
            assert_eq!((&inv, &off), (&inv_after, &off_after), "into {answer}");
            landed.push(answer);
        }
        assert_eq!(landed, [3, OFFHAND, OFFHAND, 7, 1, 1]);
    }

    #[cfg(test)]
    /// A mod's rows that keep hotbar slots 6 to 8 free change the slot the game's
    /// `Inventory.add` takes (replaced on this thread through `rule_replacements::ADD_SLOT`):
    /// a dirt passes the stack in slot 6 with room and the selected slot 7 for slot 0.
    #[test]
    fn a_replaced_slot_choice_reaches_the_games_inventory_add() {
        use super::super::inventory_add;
        use super::super::rule_replacements::{ADD_SLOT, replaced};
        use sunrise_sim::menu::OFFHAND;
        fn open(i: usize) -> bool {
            !(6..=8).contains(&i)
        }
        fn locked(
            items: &[Option<ItemStack>],
            offhand: Option<&ItemStack>,
            selected: usize,
            stack: &ItemStack,
            max: u32,
        ) -> Option<usize> {
            let has = |have: Option<&ItemStack>| {
                have.is_some_and(|have| have.same_item_same_components(stack) && have.count < max)
            };
            if open(selected) && has(items[selected].as_ref()) {
                Some(selected)
            } else if has(offhand) {
                Some(OFFHAND)
            } else {
                (0..items.len())
                    .find(|&i| open(i) && has(items[i].as_ref()))
                    .or_else(|| (0..items.len()).find(|&i| open(i) && items[i].is_none()))
            }
        }
        let mut inv = vec![None; 36];
        inv[6] = stack("minecraft:dirt", 10);
        let mut vanilla = inv.clone();
        let mut off = None;
        let mut one = stack("minecraft:dirt", 1).unwrap();
        let left = replaced(&ADD_SLOT, locked, || {
            inventory_add(&mut inv, &mut off, 7, 64, &mut one)
        });
        assert!(!left);
        assert_eq!(
            (&inv[0], &inv[6], &inv[7]),
            (
                &stack("minecraft:dirt", 1),
                &stack("minecraft:dirt", 10),
                &None
            )
        );
        // Without the rows, vanilla's slot: the stack in slot 6 with room.
        let mut one = stack("minecraft:dirt", 1).unwrap();
        assert!(!inventory_add(&mut vanilla, &mut off, 7, 64, &mut one));
        assert_eq!(vanilla[6], stack("minecraft:dirt", 11));
    }

    #[cfg(test)]
    #[test]
    fn usages_merge_into_one_tree_with_runnable_nodes() {
        let (t, levels, problems) = tree(&[
            mod_api::Command::new("balance [player:player]"),
            mod_api::Command::new("pay <player:player> <amount:integer>"),
            mod_api::Command::op("eco give <player:player> <amount:integer>", 3),
            mod_api::Command::op("eco take <player:player> <amount:integer>", 3),
        ]);
        assert!(problems.is_empty(), "{problems:?}");
        let c = &t["children"];
        assert_eq!(c["balance"]["executable"], true);
        assert_eq!(c["balance"]["children"]["player"]["executable"], true);
        assert_eq!(
            c["balance"]["children"]["player"]["parser"],
            "minecraft:entity"
        );
        assert_eq!(c["pay"]["executable"], false);
        assert_eq!(c["pay"]["children"]["player"]["executable"], false);
        let amount = &c["pay"]["children"]["player"]["children"]["amount"];
        assert_eq!(amount["executable"], true);
        assert_eq!(amount["parser"], "brigadier:integer");
        assert!(amount.get("children").is_none());
        assert_eq!(c["eco"]["permissions"]["permission"]["level"], "admins");
        assert!(c["eco"]["children"]["give"].is_object());
        assert!(c["eco"]["children"]["take"].is_object());
        assert!(c["pay"].get("permissions").is_none());
        assert_eq!(levels["eco"], 3);
        assert_eq!(levels["pay"], 0);
    }

    #[cfg(test)]
    #[test]
    fn a_usage_with_another_string_kind_is_left_out_in_either_order() {
        for (first, second, line, other) in [
            (
                "foo <value:word>",
                "foo <value:text>",
                "foo hello",
                "foo hello world",
            ),
            (
                "foo <value:text>",
                "foo <value:word>",
                "foo hello world",
                "foo hello",
            ),
        ] {
            let (tree, _, problems) =
                tree(&[mod_api::Command::new(first), mod_api::Command::new(second)]);
            assert_eq!(problems.len(), 1, "{first} then {second}: {problems:?}");
            assert!(problems[0].starts_with(&format!("`{second}`")));
            let mut dispatcher = Dispatcher::new();
            register_dispatcher(
                &mut dispatcher,
                &tree,
                "native_mod_command",
                &HashMap::new(),
            );
            let registries = sunrise_command::Registries::default();
            assert!(dispatcher.parse(line, &registries).executable().is_ok());
            // The kept kind decides: `word` stops at the space, `text` takes it all.
            let kept_greedy = first.contains(":text");
            assert_eq!(
                dispatcher.parse(other, &registries).executable().is_ok(),
                kept_greedy,
                "{first} then {second}: {other}"
            );
        }
    }

    #[cfg(test)]
    pub(in crate::game) const POSITIONED: &str = "kinds <at:vec3> <by:vec3_uncentered> <flat:vec2> \
         <flat_raw:vec2_uncentered> <column:column_pos> <look:rotation> <level:dimension> <tag:nbt>";

    #[cfg(test)]
    #[test]
    fn position_rotation_dimension_and_nbt_kinds_are_vanillas_parsers() {
        let (tree, _, problems) = tree(&[mod_api::Command::new(POSITIONED)]);
        assert!(problems.is_empty(), "{problems:?}");
        let mut node = &tree["children"]["kinds"];
        for (name, parser) in [
            ("at", "minecraft:vec3"),
            ("by", "minecraft:vec3"),
            ("flat", "minecraft:vec2"),
            ("flat_raw", "minecraft:vec2"),
            ("column", "minecraft:column_pos"),
            ("look", "minecraft:rotation"),
            ("level", "minecraft:dimension"),
            ("tag", "minecraft:nbt_compound_tag"),
        ] {
            node = &node["children"][name];
            assert_eq!(node["parser"], parser, "{name}");
            assert_eq!(
                node["uncentered"] == true,
                matches!(name, "by" | "flat_raw"),
                "{name}"
            );
        }
    }

    #[cfg(test)]
    #[test]
    fn vec3_and_vec3_uncentered_do_not_share_a_node() {
        let (_, _, problems) = tree(&[
            mod_api::Command::new("go <to:vec3>"),
            mod_api::Command::new("go <to:vec3_uncentered>"),
        ]);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("another kind"), "{problems:?}");
    }

    #[cfg(test)]
    #[test]
    fn vec2_and_vec2_uncentered_do_not_share_a_node() {
        let (_, _, problems) = tree(&[
            mod_api::Command::new("go <to:vec2>"),
            mod_api::Command::new("go <to:vec2_uncentered>"),
        ]);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("another kind"), "{problems:?}");
    }

    #[cfg(test)]
    #[test]
    fn positions_and_rotations_resolve_against_the_source() {
        let (tree, _, problems) = tree(&[mod_api::Command::new(POSITIONED)]);
        assert!(problems.is_empty(), "{problems:?}");
        let mut dispatcher = Dispatcher::new();
        register_dispatcher(
            &mut dispatcher,
            &tree,
            "native_mod_command",
            &HashMap::new(),
        );
        let registries = sunrise_command::Registries::default();
        let (origin, rotation) = ([10.0, 70.0, -2.0], [45.0, 30.0]);
        let value = |line: &str, name: &str, kind: &str| {
            let parsed = dispatcher.parse(line, &registries);
            let (context, _) = parsed.executable().unwrap();
            let Some(Arg::Coords(coordinates)) = context.get(name) else {
                panic!("{name} is a position")
            };
            let range = context.args.iter().find(|a| a.name == name).unwrap().range;
            let ty = parsed_type(&dispatcher, &context.nodes, range);
            assert_eq!(
                format!("{ty:?}"),
                format!("{:?}", Some(parser(kind).unwrap().2)),
                "{name}: the type comes from the parsed node"
            );
            positional_value(
                ty,
                coordinates,
                coordinates.position(origin, rotation),
                rotation,
            )
        };
        let relative =
            "kinds ~1 ~ ~-1 ~1 ~ ~-1 ~1 ~-2 ~1 ~-2 ~3 ~-4 ~10 ~-5 minecraft:the_nether {}";
        assert_eq!(
            value(relative, "at", "vec3"),
            mod_api::CommandArgumentValue::Vec3([11.0, 70.0, -3.0])
        );
        assert_eq!(
            value(relative, "flat", "vec2"),
            mod_api::CommandArgumentValue::Vec2([11.0, -4.0])
        );
        assert_eq!(
            value(relative, "flat_raw", "vec2_uncentered"),
            mod_api::CommandArgumentValue::Vec2([11.0, -4.0])
        );
        assert_eq!(
            value(relative, "column", "column_pos"),
            mod_api::CommandArgumentValue::ColumnPos([13, -6])
        );
        assert_eq!(
            value(relative, "look", "rotation"),
            mod_api::CommandArgumentValue::Rotation([55.0, 25.0])
        );
        // A whole `x` and `z` is centred by `vec3`/`vec2` and kept by `*_uncentered`.
        let absolute = "kinds 5 64 -7 5 64 -7 5 7 5 7 0 0 0 0 minecraft:overworld {}";
        assert_eq!(
            value(absolute, "at", "vec3"),
            mod_api::CommandArgumentValue::Vec3([5.5, 64.0, -6.5])
        );
        assert_eq!(
            value(absolute, "by", "vec3_uncentered"),
            mod_api::CommandArgumentValue::Vec3([5.0, 64.0, -7.0])
        );
        assert_eq!(
            value(absolute, "flat", "vec2"),
            mod_api::CommandArgumentValue::Vec2([5.5, 7.5])
        );
        assert_eq!(
            value(absolute, "flat_raw", "vec2_uncentered"),
            mod_api::CommandArgumentValue::Vec2([5.0, 7.0])
        );
        // `vec2` refuses local `^` coordinates, matching `Vec2Argument.parse`.
        let local = "kinds 0 0 0 0 0 0 ^1 ^2 0 0 0 0 0 0 minecraft:overworld {}";
        assert!(dispatcher.parse(local, &registries).executable().is_err());
    }

    #[cfg(test)]
    #[test]
    fn a_dimension_the_world_lacks_is_refused_and_an_nbt_argument_is_a_compound() {
        let refusal = dimension_value("minecraft:nowhere", |_| None).unwrap_err();
        assert_eq!(refusal.key, "argument.dimension.invalid");
        let mut expected = sunrise_core::nbt::Compound::new();
        expected.insert("answer".into(), sunrise_core::nbt::Tag::Byte(42));
        assert_eq!(
            nbt_value("{answer:42b}").unwrap(),
            mod_api::CommandArgumentValue::Nbt(expected)
        );
    }

    #[cfg(test)]
    #[test]
    fn every_supported_kind_has_a_parser() {
        assert!(
            argument_kinds::KINDS
                .into_iter()
                .all(|kind| parser(kind).is_some())
        );
    }

    #[cfg(test)]
    /// A test mod's argument type: named machines, one word, completed by prefix.
    pub(in crate::game) struct MachineArgument;

    #[cfg(test)]
    impl mod_api::CommandArgumentType for MachineArgument {
        fn kind(&self) -> mod_api::CommandArgumentKind {
            mod_api::CommandArgumentKind::Word
        }

        fn parse(&self, input: &str) -> Result<String, mod_api::CommandArgumentError> {
            match input {
                "press" | "furnace" => Ok(input.to_owned()),
                _ => Err(mod_api::CommandArgumentError::new(
                    mod_api::Text::translate(
                        "test.commands.machine.unknown",
                        vec![mod_api::Text::literal(input)],
                    ),
                )),
            }
        }

        fn suggest(&self, request: &mod_api::SuggestionRequest) -> Vec<mod_api::Suggestion> {
            ["press", "furnace"]
                .into_iter()
                .filter(|machine| machine.starts_with(request.prefix.as_str()))
                .map(|machine| request.suggest(machine))
                .collect()
        }
    }

    #[cfg(test)]
    pub(in crate::game) struct HostileArgument;

    #[cfg(test)]
    impl mod_api::CommandArgumentType for HostileArgument {
        fn kind(&self) -> mod_api::CommandArgumentKind {
            mod_api::CommandArgumentKind::Word
        }
        fn parse(&self, input: &str) -> Result<String, mod_api::CommandArgumentError> {
            if input == "panic" {
                panic!("parser")
            }
            Ok(input.into())
        }
        fn suggest(&self, request: &mod_api::SuggestionRequest) -> Vec<mod_api::Suggestion> {
            match request.prefix.as_str() {
                "panic" => panic!("completer"),
                "text" => vec![request.suggest("a".repeat(32768))],
                "tip" => vec![request.suggest_with_tooltip("ok", "a".repeat(65536))],
                "unicode" => vec![request.suggest_with_tooltip("ok", "\0".repeat(32768))],
                _ => vec![request.suggest_with_tooltip("a".repeat(32767), "a".repeat(65535))],
            }
        }
    }

    #[cfg(test)]
    #[test]
    fn hostile_argument_callbacks_are_refused_without_losing_the_dispatcher() {
        let arguments: CommandArguments = HashMap::from([(
            "test:hostile".into(),
            Arc::new(HostileArgument) as Arc<dyn mod_api::CommandArgumentType>,
        )]);
        let (tree, _, problems) = crate::game::native_server::command_tree(
            &[mod_api::Command::new("probe <value:test:hostile>")],
            &sunrise_command::Dispatcher::new(),
            &arguments,
        );
        assert!(problems.is_empty());
        let mut dispatcher = Dispatcher::new();
        register_dispatcher(&mut dispatcher, &tree, "native_mod_command", &arguments);
        let registries = sunrise_command::Registries::default();
        for prefix in ["panic", "text", "tip", "unicode"] {
            let input = format!("probe {prefix}");
            let parsed = dispatcher.parse(&input, &registries);
            assert_eq!(parsed.executable().is_err(), prefix == "panic");
            let (_, suggestions) = dispatcher.suggestions(&parsed, input.len(), &registries);
            assert!(suggestions.is_empty(), "{prefix}");
        }
        let input = "probe valid";
        let parsed = dispatcher.parse(input, &registries);
        let (_, suggestions) = dispatcher.suggestions(&parsed, input.len(), &registries);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].text.len(), 32767);
    }

    #[cfg(test)]
    #[test]
    fn panicking_argument_kind_is_left_out_of_the_tree() {
        struct PanicKind;
        impl mod_api::CommandArgumentType for PanicKind {
            fn kind(&self) -> mod_api::CommandArgumentKind {
                panic!("kind")
            }
            fn parse(&self, _: &str) -> Result<String, mod_api::CommandArgumentError> {
                unreachable!()
            }
            fn suggest(&self, _: &mod_api::SuggestionRequest) -> Vec<mod_api::Suggestion> {
                unreachable!()
            }
        }
        let arguments: CommandArguments = HashMap::from([(
            "test:panic".into(),
            Arc::new(PanicKind) as Arc<dyn mod_api::CommandArgumentType>,
        )]);
        let (_, _, problems) = crate::game::native_server::command_tree(
            &[mod_api::Command::new("probe <value:test:panic>")],
            &sunrise_command::Dispatcher::new(),
            &arguments,
        );
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("panicked"));
    }

    #[cfg(test)]
    pub(in crate::game) struct Machines;

    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Machines {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.register_command_argument("machine", MachineArgument);
        }
    }

    #[cfg(test)]
    #[test]
    fn a_registered_argument_type_parses_and_completes_its_values() {
        mod_platform::__private::part_tests::install_parts_fixture(&[mod_api::ModEntry::new(
            "test",
            Box::new(Machines),
        )])
        .unwrap();
        let arguments = command_arguments();
        assert!(arguments.contains_key("test:machine"));

        let (tree, _, problems) = crate::game::native_server::command_tree(
            &[mod_api::Command::new("machine use <machine:test:machine>")],
            &sunrise_command::Dispatcher::new(),
            &arguments,
        );
        assert!(problems.is_empty(), "{problems:?}");
        let machine = &tree["children"]["machine"]["children"]["use"]["children"]["machine"];
        assert_eq!(machine["custom_argument"], "test:machine");
        assert_eq!(machine["suggestions"], "minecraft:ask_server");
        assert_eq!(machine["properties"], json!({"type": "word"}));

        let mut dispatcher = Dispatcher::new();
        register_dispatcher(&mut dispatcher, &tree, "native_mod_command", &arguments);
        let registries = sunrise_command::Registries::default();
        let parsed = dispatcher.parse("machine use press", &registries);
        let (context, _) = parsed.executable().unwrap();
        assert_eq!(
            context.get("machine"),
            Some(&Arg::Custom {
                kind: "test:machine".into(),
                value: "press".into(),
            })
        );
        assert!(
            dispatcher
                .parse("machine use banana", &registries)
                .executable()
                .is_err(),
            "the mod's parser rejects what it does not know"
        );

        let typed = "machine use pr";
        let parse = dispatcher.parse(typed, &registries);
        let (_, suggestions) = dispatcher.suggestions(&parse, typed.len(), &registries);
        assert_eq!(
            suggestions
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>(),
            ["press"]
        );
    }

    #[cfg(test)]
    #[test]
    fn the_id_kind_reads_a_free_identifier_with_no_registry() {
        let (tree, _, problems) = tree(&[mod_api::Command::new("map fullrender <world:id>")]);
        assert!(problems.is_empty(), "{problems:?}");
        let world = &tree["children"]["map"]["children"]["fullrender"]["children"]["world"];
        assert_eq!(world["parser"], "minecraft:resource_location");
        assert!(
            world.get("properties").is_none(),
            "ResourceLocationArgument carries no properties"
        );
        // The free `id` has no list of its own: its node asks the server, which answers from the
        // mods' own suggestions (`tab_complete`), so an unmodded client is not left with nothing.
        assert_eq!(world["suggestions"], "minecraft:ask_server");

        let mut dispatcher = Dispatcher::new();
        register_dispatcher(
            &mut dispatcher,
            &tree,
            "native_mod_command",
            &HashMap::new(),
        );
        let registries = sunrise_command::Registries::default();
        for (line, id) in [
            ("map fullrender minecraft:overworld", "minecraft:overworld"),
            ("map fullrender mymod:custom", "mymod:custom"),
            ("map fullrender overworld", "minecraft:overworld"),
        ] {
            let parsed = dispatcher.parse(line, &registries);
            let (context, _) = parsed.executable().unwrap_or_else(|e| {
                panic!("{line}: {}", e.message.plain());
            });
            assert_eq!(context.get("world"), Some(&Arg::Str(id.into())), "{line}");
        }
        for refused in [
            "map fullrender Minecraft:overworld",
            "map fullrender minecraft:overworld:again",
            "map fullrender 50%stone,50%dirt",
        ] {
            assert!(
                dispatcher.parse(refused, &registries).executable().is_err(),
                "{refused}"
            );
        }
    }

    #[cfg(test)]
    /// A test mod's argument type for a WorldEdit-style pattern list: the greedy text holds
    /// several tokens, and the completion is for the one being typed.
    pub(in crate::game) struct PatternArgument;

    #[cfg(test)]
    impl mod_api::CommandArgumentType for PatternArgument {
        fn kind(&self) -> mod_api::CommandArgumentKind {
            mod_api::CommandArgumentKind::Greedy
        }

        fn parse(&self, input: &str) -> Result<String, mod_api::CommandArgumentError> {
            Ok(input.trim().to_owned())
        }

        fn suggest(&self, request: &mod_api::SuggestionRequest) -> Vec<mod_api::Suggestion> {
            let prefix = request.prefix.as_str();
            let start = prefix.rfind(' ').map_or(0, |space| space + 1);
            ["stone", "dirt"]
                .into_iter()
                .filter(|block| block.starts_with(&prefix[start..]))
                .map(|block| request.suggest_from(start, block))
                .collect()
        }
    }

    #[cfg(test)]
    #[test]
    fn a_greedy_argument_completes_the_token_being_typed() {
        let arguments: CommandArguments = HashMap::from([(
            "test:pattern".into(),
            Arc::new(PatternArgument) as Arc<dyn mod_api::CommandArgumentType>,
        )]);
        let (tree, _, problems) = crate::game::native_server::command_tree(
            &[mod_api::Command::new("replace <pattern:test:pattern>")],
            &sunrise_command::Dispatcher::new(),
            &arguments,
        );
        assert!(problems.is_empty(), "{problems:?}");
        let mut dispatcher = Dispatcher::new();
        register_dispatcher(&mut dispatcher, &tree, "native_mod_command", &arguments);
        let registries = sunrise_command::Registries::default();

        // The second token: the range covers it, and the text in front of it is kept.
        let typed = "replace stone st";
        let parse = dispatcher.parse(typed, &registries);
        let (range, suggestions) = dispatcher.suggestions(&parse, typed.len(), &registries);
        assert_eq!(
            suggestions
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>(),
            ["stone"]
        );
        assert_eq!(range, (14, typed.len()), "the token, not the whole text");
        assert_eq!(suggestions[0].apply(typed), "replace stone stone");

        // The first token of the text replaces it whole, as before.
        let typed = "replace dir";
        let parse = dispatcher.parse(typed, &registries);
        let (range, suggestions) = dispatcher.suggestions(&parse, typed.len(), &registries);
        assert_eq!(range, (8, typed.len()));
        assert_eq!(suggestions[0].apply(typed), "replace dirt");
    }

    #[cfg(test)]
    /// A type whose completion mixes one the text can hold with one it cannot.
    pub(in crate::game) struct MixedOffsets;

    #[cfg(test)]
    impl mod_api::CommandArgumentType for MixedOffsets {
        fn kind(&self) -> mod_api::CommandArgumentKind {
            mod_api::CommandArgumentKind::Greedy
        }

        fn parse(&self, input: &str) -> Result<String, mod_api::CommandArgumentError> {
            Ok(input.to_owned())
        }

        fn suggest(&self, request: &mod_api::SuggestionRequest) -> Vec<mod_api::Suggestion> {
            vec![
                request.suggest_from(0, "stone"),
                request.suggest_from(request.prefix.len() + 1, "dirt"),
            ]
        }
    }

    #[cfg(test)]
    #[test]
    fn a_suggestion_with_an_offset_past_the_text_is_refused_and_the_others_stay() {
        let arguments: CommandArguments = HashMap::from([(
            "test:mixed".into(),
            Arc::new(MixedOffsets) as Arc<dyn mod_api::CommandArgumentType>,
        )]);
        let (tree, _, problems) = crate::game::native_server::command_tree(
            &[mod_api::Command::new("fill <blocks:test:mixed>")],
            &sunrise_command::Dispatcher::new(),
            &arguments,
        );
        assert!(problems.is_empty(), "{problems:?}");
        let mut dispatcher = Dispatcher::new();
        register_dispatcher(&mut dispatcher, &tree, "native_mod_command", &arguments);
        let registries = sunrise_command::Registries::default();
        let typed = "fill st";
        let parse = dispatcher.parse(typed, &registries);
        let (_, suggestions) = dispatcher.suggestions(&parse, typed.len(), &registries);
        assert_eq!(
            suggestions
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>(),
            ["stone"],
            "the suggestion past the typed text is left out, never applied"
        );
    }

    #[cfg(test)]
    #[test]
    fn resource_and_mod_id_kinds_accept_only_known_ids() {
        let mut arguments = command_arguments();
        // The test process loads no package: stand in for one the way `command_arguments` does.
        arguments.insert(
            argument_kinds::MOD_ID.into(),
            Arc::new(mod_api::ChoiceArgument::new(["demo"])),
        );
        let (tree, _, problems) = crate::game::native_server::command_tree(
            &[
                mod_api::Command::new("spawnof <kind:resource:minecraft:entity_type>"),
                mod_api::Command::new("about <id:mod_id>"),
                mod_api::Command::new("bad <kind:resource:NotOk>"),
                mod_api::Command::new("worse <kind:resource:a:b:c>"),
                mod_api::Command::new("worst <kind:resource:a/b:c>"),
            ],
            &sunrise_command::Dispatcher::new(),
            &arguments,
        );
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(
            problems
                .iter()
                .all(|p| p.contains("is not a `namespace:path` resource location")),
            "{problems:?}"
        );
        for refused in ["bad", "worse", "worst"] {
            assert!(tree["children"].get(refused).is_none(), "{refused}");
        }
        let kind = &tree["children"]["spawnof"]["children"]["kind"];
        assert_eq!(kind["parser"], "minecraft:resource_key");
        assert_eq!(
            kind["properties"],
            json!({"registry": "minecraft:entity_type"})
        );
        assert_eq!(
            tree["children"]["about"]["children"]["id"]["custom_argument"],
            argument_kinds::MOD_ID
        );

        let mut dispatcher = Dispatcher::new();
        register_dispatcher(&mut dispatcher, &tree, "native_mod_command", &arguments);
        let registries = sunrise_command::Registries {
            entity_types: vec!["minecraft:zombie".into()],
            ..Default::default()
        };
        let parsed = dispatcher.parse("spawnof minecraft:zombie", &registries);
        let (context, _) = parsed.executable().unwrap();
        assert_eq!(
            context.get("kind"),
            Some(&Arg::Resource("minecraft:zombie".into()))
        );
        for refused in ["spawnof minecraft:nope", "about no_such_mod"] {
            assert!(
                dispatcher.parse(refused, &registries).executable().is_err(),
                "{refused}"
            );
        }
        let parsed = dispatcher.parse("about demo", &registries);
        let (context, _) = parsed.executable().unwrap();
        let Some(Arg::Custom { kind, value }) = context.get("id").cloned() else {
            panic!("a mod id parses as the built-in custom kind");
        };
        assert_eq!(
            custom_value(&kind, value),
            mod_api::CommandArgumentValue::String("demo".into()),
            "a loaded mod id reaches the mod as its plain string"
        );
    }

    #[cfg(test)]
    #[test]
    fn native_usages_parse_with_their_permissions_in_the_server_dispatcher() {
        let (tree, _, problems) = tree(&[
            mod_api::Command::new("warp [name:word]"),
            mod_api::Command::op("warp set <amount:integer>", 2),
            mod_api::Command::new("toggle <enabled:boolean>"),
        ]);
        assert!(problems.is_empty(), "{problems:?}");
        let mut dispatcher = Dispatcher::new();
        register_dispatcher(
            &mut dispatcher,
            &tree,
            "native_mod_command",
            &HashMap::new(),
        );
        let mut registries = sunrise_command::Registries::default();
        assert!(dispatcher.parse("warp", &registries).executable().is_ok());
        assert!(
            dispatcher
                .parse("warp home", &registries)
                .executable()
                .is_ok()
        );
        assert!(
            dispatcher
                .parse("warp set 7", &registries)
                .executable()
                .is_err()
        );
        registries.permission = 2;
        assert!(
            dispatcher
                .parse("warp set 7", &registries)
                .executable()
                .is_ok()
        );
        assert!(
            dispatcher
                .parse("warp set nope", &registries)
                .executable()
                .is_err()
        );
        assert!(
            dispatcher
                .parse("warp set", &registries)
                .executable()
                .is_err()
        );
        assert!(
            dispatcher
                .parse("toggle true", &registries)
                .executable()
                .is_ok()
        );
        assert!(
            dispatcher
                .parse("toggle maybe", &registries)
                .executable()
                .is_err()
        );
    }

    #[cfg(test)]
    #[test]
    fn native_executable_permissions_do_not_follow_public_descendants() {
        for optional in [false, true] {
            for reverse in [false, true] {
                let admin = if optional {
                    "foo [value:integer]"
                } else {
                    "foo"
                };
                let mut commands = vec![
                    mod_api::Command::op(admin, 4),
                    mod_api::Command::new("foo bar"),
                ];
                if reverse {
                    commands.reverse();
                }
                let (tree, _, problems) = tree(&commands);
                assert!(problems.is_empty(), "{problems:?}");
                let mut dispatcher = Dispatcher::new();
                register_dispatcher(
                    &mut dispatcher,
                    &tree,
                    "native_mod_command",
                    &HashMap::new(),
                );
                for permission in 0..=4 {
                    let registries = sunrise_command::Registries {
                        permission,
                        ..Default::default()
                    };
                    for line in ["foo", "foo bar", "foo 7"] {
                        let allowed =
                            line == "foo bar" || permission == 4 && (line == "foo" || optional);
                        assert_eq!(
                            dispatcher.parse(line, &registries).executable().is_ok(),
                            allowed,
                            "{commands:?}, level {permission}, {line}"
                        );
                    }
                }
            }
        }
    }

    #[cfg(test)]
    #[test]
    fn native_public_prefixes_do_not_open_admin_children() {
        for reverse in [false, true] {
            let mut commands = vec![
                mod_api::Command::new("foo"),
                mod_api::Command::op("foo bar", 4),
            ];
            if reverse {
                commands.reverse();
            }
            let (tree, _, problems) = tree(&commands);
            assert!(problems.is_empty(), "{problems:?}");
            let mut dispatcher = Dispatcher::new();
            register_dispatcher(
                &mut dispatcher,
                &tree,
                "native_mod_command",
                &HashMap::new(),
            );
            for permission in 0..=4 {
                let registries = sunrise_command::Registries {
                    permission,
                    ..Default::default()
                };
                assert!(dispatcher.parse("foo", &registries).executable().is_ok());
                assert_eq!(
                    dispatcher
                        .parse("foo bar", &registries)
                        .executable()
                        .is_ok(),
                    permission == 4
                );
            }
        }
    }

    #[cfg(test)]
    #[test]
    fn native_shared_executable_prefix_uses_last_accepted_registration() {
        let cases = [
            (
                vec![
                    mod_api::Command::op("foo [value:integer]", 4),
                    mod_api::Command::op("foo", 2),
                    mod_api::Command::op("foo", 2), // Later registration wins on ties.
                    mod_api::Command::new("foo [value:word]"), // Rejected: parser conflict.
                ],
                (2, 2),
                (4, 0),
            ),
            (
                vec![
                    mod_api::Command::op("foo", 2),
                    mod_api::Command::op("foo [value:integer]", 4), // Later runnable overwrites earlier lower level.
                    mod_api::Command::new("foo [value:word]"),      // Rejected: parser conflict.
                ],
                (4, 1),
                (4, 1),
            ),
        ];
        for (commands, (prefix_level, prefix_slot), (argument_level, argument_slot)) in cases {
            let arguments = HashMap::new();
            let mut executors = HashMap::new();
            let (tree, _, problems) = command_tree_for(
                &commands,
                BaseTree::Merged(&Dispatcher::new()),
                &arguments,
                false,
                Some(&mut executors),
            );
            assert_eq!(problems.len(), 1, "{problems:?}");
            let mut dispatcher = Dispatcher::new();
            register_command_nodes(
                &mut dispatcher,
                &tree,
                "native_mod_command",
                &arguments,
                Some(&executors),
            );
            for permission in 0..=4 {
                let registries = sunrise_command::Registries {
                    permission,
                    ..Default::default()
                };
                for (input, level, slot) in [
                    ("foo", prefix_level, prefix_slot),
                    ("foo 7", argument_level, argument_slot),
                ] {
                    let parsed = dispatcher.parse(input, &registries);
                    if permission >= level {
                        let (context, _) = parsed.executable().unwrap();
                        assert_eq!(
                            context.executor.unwrap().slot(),
                            slot,
                            "{commands:?}, level {permission}, {input}"
                        );
                    } else {
                        assert!(
                            parsed.executable().is_err(),
                            "{commands:?}, level {permission}, {input}"
                        );
                    }
                }
            }
        }
    }

    #[cfg(test)]
    #[test]
    fn native_rejected_usage_cannot_open_an_executable_prefix() {
        let (tree, _, problems) = tree(&[
            mod_api::Command::op("foo <value:integer>", 4),
            mod_api::Command::new("foo [value:word]"),
        ]);
        assert_eq!(problems.len(), 1);
        let mut dispatcher = Dispatcher::new();
        register_dispatcher(
            &mut dispatcher,
            &tree,
            "native_mod_command",
            &HashMap::new(),
        );
        for permission in 0..=4 {
            let registries = sunrise_command::Registries {
                permission,
                ..Default::default()
            };
            assert!(dispatcher.parse("foo", &registries).executable().is_err());
            assert!(
                dispatcher
                    .parse("foo hello", &registries)
                    .executable()
                    .is_err()
            );
            assert_eq!(
                dispatcher.parse("foo 7", &registries).executable().is_ok(),
                permission == 4
            );
        }
    }

    #[cfg(test)]
    #[test]
    fn a_command_takes_its_lowest_level() {
        let (t, levels, _) = tree(&[
            mod_api::Command::new("warp [name:word]"),
            mod_api::Command::op("warp set <name:word>", 2),
        ]);
        assert_eq!(levels["warp"], 0);
        assert!(t["children"]["warp"].get("permissions").is_none());
        let set = &t["children"]["warp"]["children"]["set"];
        assert_eq!(set["permissions"]["permission"]["level"], "gamemasters");
    }

    #[cfg(test)]
    #[test]
    fn bad_usages_are_left_out() {
        let (t, levels, problems) = tree(&[
            mod_api::Command::new("a [x:integer] <y:integer>"),
            mod_api::Command::new("b <x:text> <y:word>"),
            mod_api::Command::new("c <x:colour>"),
            mod_api::Command::new("<x:integer>"),
            mod_api::Command::new("Bad"),
            mod_api::Command::new("d <x:integer>"),
            mod_api::Command::new("d <x:word>"),
        ]);
        assert_eq!(problems.len(), 6, "{problems:?}");
        assert_eq!(levels.keys().collect::<Vec<_>>(), ["d"]);
        assert_eq!(
            t["children"]["d"]["children"]["x"]["parser"],
            "brigadier:integer"
        );
        let (none, levels, _) = tree(&[]);
        assert!(none.is_null() && levels.is_empty());
    }

    #[cfg(test)]
    /// `integer(min,max)` and `long(min,max)` go out as vanilla's `IntegerArgumentInfo` and
    /// `LongArgumentInfo` write them (a bound at the type's extreme is left out), and the server
    /// refuses a value out of range with Brigadier's message before any executor runs.
    #[test]
    fn bounded_numbers_keep_their_bounds_in_the_tree_and_the_dispatcher() {
        let (t, _, problems) = tree(&[
            mod_api::Command::new("pay <amount:integer(0,1000)>"),
            mod_api::Command::new("bank <total:long(-5,9000000000)>"),
            mod_api::Command::new("floor <level:integer(0,)>"),
            mod_api::Command::new("whole <n:integer(-2147483648,2147483647)>"),
            mod_api::Command::new("plain <n:long>"),
            mod_api::Command::new("bad <n:integer(5,1)>"),
            mod_api::Command::new("bad <n:integer(0,2147483648)>"),
            mod_api::Command::new("bad <n:long(1)>"),
            mod_api::Command::new("bad <n:integer(a,)>"),
        ]);
        assert_eq!(problems.len(), 4, "{problems:?}");
        assert!(
            problems.iter().all(|p| p.starts_with("`bad ")),
            "{problems:?}"
        );
        let c = &t["children"];
        let node = |root: &str, name: &str| c[root]["children"][name].clone();
        assert_eq!(node("pay", "amount")["parser"], "brigadier:integer");
        assert_eq!(
            node("pay", "amount")["properties"],
            json!({"min": 0, "max": 1000})
        );
        assert_eq!(node("bank", "total")["parser"], "brigadier:long");
        assert_eq!(
            node("bank", "total")["properties"],
            json!({"min": -5, "max": 9_000_000_000_i64})
        );
        assert_eq!(node("floor", "level")["properties"], json!({"min": 0}));
        assert!(node("whole", "n").get("properties").is_none());
        assert_eq!(node("plain", "n")["parser"], "brigadier:long");
        assert!(node("plain", "n").get("properties").is_none());

        let mut dispatcher = Dispatcher::new();
        register_dispatcher(&mut dispatcher, &t, "native_mod_command", &HashMap::new());
        let registries = sunrise_command::Registries::default();
        let value = |line: &str, name: &str| {
            dispatcher
                .parse(line, &registries)
                .executable()
                .unwrap()
                .0
                .get(name)
                .cloned()
        };
        assert_eq!(value("pay 1000", "amount"), Some(Arg::Int(1000)));
        assert_eq!(
            value("bank 9000000000", "total"),
            Some(Arg::Long(9_000_000_000))
        );
        assert_eq!(
            value("plain -9223372036854775808", "n"),
            Some(Arg::Long(i64::MIN))
        );
        for (line, expected) in [
            (
                "pay 1001",
                Text::tr("argument.integer.big", &[&1000, &1001]),
            ),
            ("pay -1", Text::tr("argument.integer.low", &[&0, &-1])),
            ("floor -1", Text::tr("argument.integer.low", &[&0, &-1])),
            (
                "bank 9000000001",
                Text::tr(
                    "argument.long.big",
                    &[&9_000_000_000_i64, &9_000_000_001_i64],
                ),
            ),
            ("bank -6", Text::tr("argument.long.low", &[&-5, &-6])),
            ("bank 1.5", Text::tr("parsing.long.invalid", &[&"1.5"])),
        ] {
            let error = dispatcher
                .parse(line, &registries)
                .executable()
                .unwrap_err();
            assert_eq!(error.message, expected, "{line}");
        }
    }

    #[cfg(test)]
    /// A mod argument type that declares `CommandArgumentKind::Integer` goes out as vanilla's
    /// bounded `brigadier:integer`, so a client refuses what the server refuses; the server checks
    /// the bounds before the mod's parser.
    #[test]
    fn a_registered_integer_kind_goes_out_bounded() {
        struct Stack;
        impl mod_api::CommandArgumentType for Stack {
            fn kind(&self) -> mod_api::CommandArgumentKind {
                mod_api::CommandArgumentKind::Integer {
                    min: Some(1),
                    max: Some(64),
                }
            }
            fn parse(&self, input: &str) -> Result<String, mod_api::CommandArgumentError> {
                Ok(format!("{input} items"))
            }
            fn suggest(&self, _: &mod_api::SuggestionRequest) -> Vec<mod_api::Suggestion> {
                Vec::new()
            }
        }
        let arguments: CommandArguments = HashMap::from([(
            "test:stack".to_owned(),
            Arc::new(Stack) as Arc<dyn mod_api::CommandArgumentType>,
        )]);
        let (t, _, problems) = command_tree(
            &[mod_api::Command::new("stack <count:test:stack>")],
            &Dispatcher::new(),
            &arguments,
        );
        assert!(problems.is_empty(), "{problems:?}");
        let count = &t["children"]["stack"]["children"]["count"];
        assert_eq!(count["parser"], "brigadier:integer");
        assert_eq!(count["properties"], json!({"min": 1, "max": 64}));
        assert_eq!(count["custom_argument"], "test:stack");
        let mut dispatcher = Dispatcher::new();
        register_dispatcher(&mut dispatcher, &t, "native_mod_command", &arguments);
        let registries = sunrise_command::Registries::default();
        let parsed = dispatcher.parse("stack 64", &registries);
        assert_eq!(
            parsed.executable().unwrap().0.get("count"),
            Some(&Arg::Custom {
                kind: "test:stack".into(),
                value: "64 items".into(),
            })
        );
        let error = dispatcher
            .parse("stack 65", &registries)
            .executable()
            .unwrap_err();
        assert_eq!(error.message, Text::tr("argument.integer.big", &[&64, &65]));
    }

    #[cfg(test)]
    /// `<name:string>` and a type declaring `CommandArgumentKind::Quotable` read Brigadier's
    /// `StringArgumentType.string()`: a word or a quoted phrase, decoded, with an argument after
    /// it. Both go to clients as `brigadier:string` in its quotable mode; only the mod's type asks
    /// the server for completions, as `word` and a `Word` type do.
    #[test]
    fn a_string_usage_and_a_quotable_type_read_a_quoted_phrase_before_the_next_argument() {
        struct QuestObject;
        impl mod_api::CommandArgumentType for QuestObject {
            fn kind(&self) -> mod_api::CommandArgumentKind {
                mod_api::CommandArgumentKind::Quotable
            }
            fn parse(&self, input: &str) -> Result<String, mod_api::CommandArgumentError> {
                Ok(input.to_lowercase())
            }
            fn suggest(&self, _: &mod_api::SuggestionRequest) -> Vec<mod_api::Suggestion> {
                Vec::new()
            }
        }
        let arguments: CommandArguments = HashMap::from([(
            "test:quest_object".to_owned(),
            Arc::new(QuestObject) as Arc<dyn mod_api::CommandArgumentType>,
        )]);
        let (t, _, problems) = command_tree(
            &[
                mod_api::Command::new("quests open <object:test:quest_object> <count:integer>"),
                mod_api::Command::new("quests rename <title:string> <count:integer>"),
            ],
            &Dispatcher::new(),
            &arguments,
        );
        assert!(problems.is_empty(), "{problems:?}");
        let quests = &t["children"]["quests"]["children"];
        let object = &quests["open"]["children"]["object"];
        assert_eq!(object["parser"], "brigadier:string");
        assert_eq!(object["properties"], json!({"type": "phrase"}));
        assert_eq!(object["custom_argument"], "test:quest_object");
        assert_eq!(object["suggestions"], "minecraft:ask_server");
        let title = &quests["rename"]["children"]["title"];
        assert_eq!(title["parser"], "brigadier:string");
        assert_eq!(title["properties"], json!({"type": "phrase"}));
        assert_eq!(
            title["suggestions"],
            Value::Null,
            "vanilla's string suggests nothing"
        );

        let mut dispatcher = Dispatcher::new();
        register_dispatcher(&mut dispatcher, &t, "native_mod_command", &arguments);
        let registries = sunrise_command::Registries::default();
        for (line, value) in [
            (r##"quests open "#Chapter_One" 3"##, "#chapter_one"),
            ("quests open 1A2B3C4D5E6F7A8B 3", "1a2b3c4d5e6f7a8b"),
            (r#"quests open "" 3"#, ""),
        ] {
            let parsed = dispatcher.parse(line, &registries);
            let (context, _) = parsed
                .executable()
                .unwrap_or_else(|e| panic!("{line}: {e:?}"));
            assert_eq!(
                context.get("object"),
                Some(&Arg::Custom {
                    kind: "test:quest_object".into(),
                    value: value.into(),
                }),
                "{line}"
            );
            assert_eq!(context.get("count"), Some(&Arg::Int(3)), "{line}");
        }
        for (line, value) in [
            (
                r#"quests rename "Getting \"Started\"" 2"#,
                r#"Getting "Started""#,
            ),
            (r"quests rename 'back\\slash' 2", r"back\slash"),
            ("quests rename '' 2", ""),
            ("quests rename Plain 2", "Plain"),
        ] {
            let parsed = dispatcher.parse(line, &registries);
            let (context, _) = parsed
                .executable()
                .unwrap_or_else(|e| panic!("{line}: {e:?}"));
            assert_eq!(
                context.get("title"),
                Some(&Arg::Str(value.into())),
                "{line}"
            );
            assert_eq!(context.get("count"), Some(&Arg::Int(2)), "{line}");
        }
        for line in [r#"quests rename "Getting 2"#, r#"quests open "Getting 2"#] {
            let error = dispatcher
                .parse(line, &registries)
                .executable()
                .unwrap_err();
            assert_eq!(
                error.message,
                Text::tr("parsing.quote.expected.end", &[]),
                "{line}"
            );
            assert_eq!(error.cursor, line.len(), "{line}");
        }
    }

    #[cfg(test)]
    /// NeoForge's `RegisterCommandsEvent` registers into vanilla's dispatcher, whose literal
    /// merges by Brigadier's `CommandNode.addChild`: a mod usage under `/time` sits beside
    /// vanilla's, which stay; vanilla's node keeps its permission; an executor at a vanilla node
    /// replaces vanilla's, and a later one replaces that. A mod argument at a vanilla node's
    /// place, a mod argument on a vanilla literal and Sunrise's own tools are refused.
    #[test]
    fn usages_under_a_vanilla_root_merge_beside_its_usages() {
        let vanilla = sunrise_command::vanilla::session_dispatcher(
            &sunrise_game_data::gamerules::RuleRegistry::vanilla(),
            &[],
        );
        let commands = [
            mod_api::Command::new("time season <name:word>"),
            mod_api::Command::new("seed"),
            mod_api::Command::op("seed", 3),
            mod_api::Command::new("time set <time:integer>"),
            mod_api::Command::new("time <set:word>"),
            mod_api::Command::new("modprofile extra"),
            mod_api::Command::new("raidplus go"),
        ];
        let arguments = HashMap::new();
        let mut executors = HashMap::new();
        let (t, levels, problems) = command_tree_for(
            &commands,
            BaseTree::Merged(&vanilla),
            &arguments,
            false,
            Some(&mut executors),
        );
        assert_eq!(problems.len(), 3, "{problems:?}");
        for (problem, usage) in problems.iter().zip([
            "time set <time:integer>",
            "time <set:word>",
            "modprofile extra",
        ]) {
            assert!(problem.starts_with(&format!("`{usage}`")), "{problem}");
        }
        // A merged root keeps vanilla's level: only the mod's own root routes as a mod command.
        assert_eq!(levels.keys().collect::<Vec<_>>(), ["raidplus"]);
        let mut dispatcher = vanilla.clone();
        register_command_nodes(
            &mut dispatcher,
            &t,
            "native_mod_command",
            &arguments,
            Some(&executors),
        );
        let at = |permission| sunrise_command::Registries {
            permission,
            ..Default::default()
        };
        let run = |line: &str, permission| {
            dispatcher
                .parse(line, &at(permission))
                .executable()
                .map(|(context, command)| (command, context.executor.map(|key| key.slot())))
        };
        let vanilla_time = vanilla
            .parse("time set day", &at(2))
            .executable()
            .unwrap()
            .1;
        assert_eq!(run("time set day", 2), Ok((vanilla_time, None)));
        assert_eq!(
            run("time season spring", 2),
            Ok(("native_mod_command", Some(0)))
        );
        assert!(
            run("time season spring", 0).is_err(),
            "vanilla's /time keeps its permission"
        );
        assert!(run("seed", 2).is_err(), "the later usage's level runs it");
        assert_eq!(run("seed", 3), Ok(("native_mod_command", Some(2))));
        assert_eq!(run("raidplus go", 0), Ok(("native_mod_command", Some(6))));
    }

    #[cfg(test)]
    /// WorldEdit-style `//set`: the usage `/set <x:word>` (typed `//set` with the chat slash) is a
    /// root literal `/set` with its argument, and the server dispatches it as typed.
    #[test]
    fn a_first_word_may_start_with_one_slash() {
        let (t, levels, problems) = tree(&[mod_api::Command::new("/set <x:word>")]);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(levels.keys().collect::<Vec<_>>(), ["/set"]);
        assert_eq!(t["children"]["/set"]["type"], "literal");
        assert_eq!(t["children"]["/set"]["children"]["x"]["type"], "argument");
        for refused in ["//x", "/", "/ <x:word>", "a /b", "a /"] {
            let (_, levels, problems) = tree(&[mod_api::Command::new(refused)]);
            assert!(levels.is_empty(), "{refused}");
            assert!(
                problems.len() == 1 && problems[0].contains("is not a command word"),
                "{refused}: {problems:?}"
            );
        }
        let arguments = command_arguments();
        let mut dispatcher = Dispatcher::new();
        register_dispatcher(&mut dispatcher, &t, "native_mod_command", &arguments);
        let registries = sunrise_command::Registries::default();
        let parsed = dispatcher.parse("/set stone", &registries);
        let (context, _) = parsed.executable().expect("`//set stone` runs");
        assert_eq!(context.literals, ["/set"]);
        assert_eq!(context.get("x"), Some(&Arg::Str("stone".into())));
        assert!(
            dispatcher
                .parse("set stone", &registries)
                .executable()
                .is_err()
        );
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod fake_action_tests {
    use super::PlayerAction;
    #[cfg(test)]
    use super::{Acting, FakeEffect};
    use mod_api::{BlockHit, Hand};

    /// A click on the top of the block at `pos`.
    pub fn on_top(pos: [i32; 3]) -> PlayerAction {
        let [x, y, z] = pos.map(f64::from);
        PlayerAction::UseOn {
            hand: Hand::Main,
            pos,
            hit: BlockHit::new([x + 0.5, y + 1.0, z + 0.5], [0, 1, 0], false),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    /// A listener on `attack_entity` records the target's health where the hook
    /// runs; with `cancel` set it vetoes the blow.
    pub struct AttackWatcher {
        pub seen: std::sync::Arc<std::sync::Mutex<Vec<f32>>>,
        pub cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    #[cfg(any(test, feature = "test-support"))]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for AttackWatcher {
        fn attack_entity(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::AttackEntityEvent<'_>,
        ) -> mod_api::Flow {
            let health = u32::try_from(event.entity)
                .ok()
                .and_then(|id| host.entity(id))
                .and_then(|entity| host.living(entity))
                .and_then(|living| living.health(host))
                .expect("the watcher's target is a live zombie");
            self.seen.lock().unwrap().push(health);
            if self.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return mod_api::Flow::Cancel;
            }
            mod_api::Flow::Continue
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    /// A listener on `attack_entity` reads the attacking player's attack
    /// strength where the hook runs.
    pub struct ChargeWatcher(pub std::sync::Arc<std::sync::Mutex<Vec<Option<f32>>>>);

    #[cfg(any(test, feature = "test-support"))]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ChargeWatcher {
        fn attack_entity(
            &self,
            host: &mut dyn mod_api::ServerHost,
            _event: &mut mod_api::AttackEntityEvent<'_>,
        ) -> mod_api::Flow {
            let player = host.players().acting_player().expect("an attacking player");
            let scale = player.attack_strength_scale(host, 0.5);
            self.0.lock().unwrap().push(scale);
            mod_api::Flow::Continue
        }
    }

    #[cfg(test)]
    #[test]
    fn the_acting_mark_clears_on_a_panic_and_each_action_starts_unblocked() {
        let action = std::sync::Arc::new(super::FakeAction::default());
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _acting = Acting::start(&action);
            panic!("a mod's hook panics");
        }));
        assert!(unwound.is_err());
        assert!(!action.acting());
        {
            let acting = Acting::start(&action);
            action.block(FakeEffect::Sleep);
            action.block(FakeEffect::Menu);
            // The first effect stopped is the answer.
            assert_eq!(acting.blocked(), Some(FakeEffect::Sleep));
        }
        assert_eq!(Acting::start(&action).blocked(), None);
    }
}

#[cfg(test)]
pub(in crate::game) mod network_settings_tests {
    use super::*;
    #[test]
    #[ignore = "daily"]
    fn server_settings_follow_the_network_config() {
        use sunrise_sim::lan::{NetConfig, OnlineMode};
        let mut config = NetConfig::loopback(25565, String::new(), 20);
        // online-mode=false: no key.
        assert!(!online_mode_of(&config));
        // online-mode=true: the server's key.
        config.online = Some(std::sync::Arc::new(
            OnlineMode::generate(false, std::sync::Arc::new(|_, _, _| Ok(None))).unwrap(),
        ));
        assert!(online_mode_of(&config));
    }
}
