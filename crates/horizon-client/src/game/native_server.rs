//! Server commands and players for native mods (docs/native-mods.md,
//! "Server commands and players", API 8): the commands mods add to the
//! server's command tree (`native_register_commands`), a player's command
//! reaching them (`native_perform_command`), and the helpers a server mod
//! answers with: the players online, a line to one of them or to all, the
//! command's feedback and the world's folder; and (API 9) the acting
//! player's plain items: count, take, give. They act on the server's own
//! players (the hub's: a dedicated server's, a LAN host's, single player's
//! server thread's), never on a client's.
use super::{Dim, Game, commands::execute, lan::LanHost};
// Their old home, for mixin code that names them here.
pub(crate) use horizon_client::mod_api::{NativeCommand, NativeServerPlayer};
use horizon_client::{
    command::{ArgType, Dispatcher},
    lan::{abilities::PlayerAbilities, data::component_nbt, guest::Guest},
    mod_api::{GameMode, NativePlayerAbilities, NativePlayerFood, PlayerRef},
    motion::PlayerMotion,
    text::Text,
};
use horizon_protocol::packets::play::{self, Clientbound as C};
use serde_json::{Map, Value, json};
use std::{collections::HashMap, path::PathBuf, sync::Arc};

/// A sidebar a server mod shows one player (`native_sidebar`): its title and
/// up to 15 lines, top first, each a text on the left and, where the score's
/// number would be, an optional text on the right. API 49.
#[allow(dead_code)]
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NativeSidebar {
    pub(crate) title: Text,
    pub(crate) lines: Vec<(Text, Option<Text>)>,
}

/// The objective a mod's sidebar is sent as: the player's client only; the
/// server's scoreboard never holds it.
const SIDEBAR_OBJECTIVE: &str = "horizon:native_sidebar";

/// The argument kinds a native usage may name, each with a [`parser`].
const KINDS: [&str; 8] = [
    "word",
    "text",
    "integer",
    "number",
    "boolean",
    "player",
    "item",
    "block_pos",
];

/// The brigadier parser of an argument kind, its properties and its dispatcher type.
fn parser(kind: &str) -> Option<(&'static str, Value, ArgType)> {
    Some(match kind {
        "word" => ("brigadier:string", json!({"type": "word"}), ArgType::Word),
        "text" => (
            "brigadier:string",
            json!({"type": "greedy"}),
            ArgType::Greedy,
        ),
        "integer" => (
            "brigadier:integer",
            Value::Null,
            ArgType::Integer {
                min: i32::MIN,
                max: i32::MAX,
            },
        ),
        "number" => (
            "brigadier:double",
            Value::Null,
            ArgType::Double {
                min: f64::MIN,
                max: f64::MAX,
            },
        ),
        "boolean" => ("brigadier:bool", Value::Null, ArgType::Bool),
        "player" => (
            "minecraft:entity",
            json!({"amount": "single", "type": "players"}),
            ArgType::Entity {
                single: true,
                players: true,
            },
        ),
        "item" => ("minecraft:item_stack", Value::Null, ArgType::Item),
        "block_pos" => ("minecraft:block_pos", Value::Null, ArgType::BlockPos),
        _ => return None,
    })
}

/// The report's name of an operator level (`commands.json`'s
/// `permissions`).
const LEVELS: [&str; 5] = ["", "moderators", "gamemasters", "admins", "owners"];

/// One parsed word of a usage: its key in the tree and its node, without
/// children.
fn usage_nodes(usage: &str) -> Result<Vec<(String, Value, bool)>, String> {
    let mut out = Vec::new();
    let mut optional_seen = false;
    let mut greedy_seen = false;
    for word in usage.split_whitespace() {
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
            let literal = !word.is_empty()
                && word
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_-".contains(c));
            if !literal {
                return Err(format!("`{usage}`: `{word}` is not a command word"));
            }
            out.push((word.to_owned(), json!({"type": "literal"}), false));
            continue;
        };
        let Some((name, kind)) = inner.and_then(|w| w.split_once(':')) else {
            return Err(format!("`{usage}`: `{word}` is not `<name:kind>`"));
        };
        if optional_seen && !optional {
            return Err(format!("`{usage}`: `{word}` follows an optional argument"));
        }
        let Some((parser, properties, _)) = parser(kind) else {
            return Err(format!("`{usage}`: unknown kind `{kind}`"));
        };
        optional_seen |= optional;
        greedy_seen |= kind == "text";
        let mut node = json!({"type": "argument", "parser": parser});
        if !properties.is_null() {
            node["properties"] = properties;
        }
        out.push((name.to_owned(), node, optional));
    }
    if out.first().is_none_or(|n| n.1["type"] != "literal") {
        return Err(format!("`{usage}`: a command starts with its name"));
    }
    Ok(out)
}

/// Native mods' commands as a root in the commands report's format
/// (`lan::commands`, which sends each player what their level allows), and
/// each command's level: the lowest of its usages. A node is runnable where
/// only optional arguments follow it. A command vanilla has (`vanilla`
/// answers) or a usage that doesn't parse is left out, with why.
pub(super) fn command_tree(
    commands: &[NativeCommand],
    vanilla: impl Fn(&str) -> bool,
) -> (Value, HashMap<String, u8>, Vec<String>) {
    let mut root = Map::new();
    let mut levels: HashMap<String, u8> = HashMap::new();
    let mut problems = Vec::new();
    for command in commands {
        let nodes = match usage_nodes(&command.usage) {
            Ok(n) => n,
            Err(e) => {
                problems.push(e);
                continue;
            }
        };
        if vanilla(&nodes[0].0) {
            problems.push(format!("`{}`: vanilla has /{}", command.usage, nodes[0].0));
            continue;
        }
        let mut children = &mut root;
        for (i, (name, node, _)) in nodes.iter().enumerate() {
            let runnable = nodes.get(i + 1).is_none_or(|n| n.2);
            let entry = children.entry(name.clone()).or_insert_with(|| {
                let mut n = node.clone();
                n["executable"] = false.into();
                n["level"] = 4.into();
                n
            });
            // `word` and `text` share a parser; their properties tell them apart.
            if entry["type"] != node["type"]
                || entry["parser"] != node["parser"]
                || entry["properties"] != node["properties"]
            {
                problems.push(format!(
                    "`{}`: `{name}` is another kind in an earlier usage",
                    command.usage
                ));
                break;
            }
            if runnable {
                entry["executable"] = true.into();
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
        let level = levels.entry(nodes[0].0.clone()).or_insert(4);
        *level = (*level).min(command.level);
    }
    let mut root = Value::Object(root);
    finish(&mut root);
    let tree = if levels.is_empty() {
        Value::Null
    } else {
        json!({"type": "root", "children": root})
    };
    (tree, levels, problems)
}

/// Turns the working `level` into the report's `permissions`, and drops
/// empty child lists.
fn finish(children: &mut Value) {
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

/// Add the already validated native report tree to the server's dispatcher.
fn register_dispatcher(dispatcher: &mut Dispatcher, tree: &Value) {
    fn add(dispatcher: &mut Dispatcher, parent: usize, children: &Map<String, Value>) {
        for (name, node) in children {
            let id = if node["type"] == "literal" {
                dispatcher.literal(parent, name)
            } else {
                let ty = KINDS
                    .into_iter()
                    .filter_map(parser)
                    .find_map(|(id, properties, ty)| {
                        (node["parser"] == id && node["properties"] == properties).then_some(ty)
                    })
                    .expect("native command tree contains registered parsers");
                dispatcher.argument(parent, name, ty)
            };
            let level = node["permissions"]["permission"]["level"]
                .as_str()
                .and_then(|level| LEVELS.iter().position(|known| *known == level))
                .unwrap_or(0) as u8;
            dispatcher.requires(id, level);
            if node["executable"] == true {
                dispatcher.executes(id, "native_mod_command");
            }
            if let Some(children) = node["children"].as_object() {
                add(dispatcher, id, children);
            }
        }
    }
    if let Some(children) = tree["children"].as_object() {
        add(dispatcher, 0, children);
    }
}

/// The live player body, or a guest record outside a player swap.
enum PlayerRead<'a> {
    Acting {
        mode: i32,
        body: Option<&'a PlayerMotion>,
        /// The player's stored abilities, when it is a server player's turn.
        abilities: Option<PlayerAbilities>,
    },
    Guest(&'a Guest),
}

impl PlayerRead<'_> {
    fn mode(&self) -> i32 {
        match self {
            Self::Acting { mode, .. } => *mode,
            Self::Guest(guest) => guest.game_mode,
        }
    }

    fn creative(&self) -> bool {
        self.mode() == 1
    }

    fn may_build(&self) -> bool {
        match self {
            Self::Acting {
                abilities: Some(a), ..
            } => a.may_build,
            Self::Guest(guest) => guest.abilities.may_build,
            Self::Acting { mode, .. } => *mode < 2,
        }
    }

    fn body(&self) -> Option<&PlayerMotion> {
        match self {
            Self::Acting { body, .. } => *body,
            Self::Guest(guest) => guest.motion.as_deref(),
        }
    }

    fn food(&self) -> Option<NativePlayerFood> {
        let food = &self.body()?.health.food;
        let (level, saturation) = match self {
            Self::Acting { .. } => (food.food, food.saturation),
            Self::Guest(guest) => (guest.food, guest.saturation),
        };
        Some(NativePlayerFood {
            level,
            saturation,
            exhaustion: food.exhaustion,
        })
    }

    fn abilities(&self) -> Option<NativePlayerAbilities> {
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
                a.update_for_mode(*mode);
                a.flying = body.flying;
                a
            }
            Self::Guest(guest) => guest.abilities,
        };
        Some(NativePlayerAbilities {
            invulnerable: a.invulnerable,
            flying: a.flying,
            may_fly: a.may_fly,
            instabuild: a.instabuild,
            may_build: a.may_build,
            fly_speed: a.flying_speed,
            walk_speed: a.walking_speed,
        })
    }
}

/// Helpers for server mods, kept as stable as the hook points.
// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// Resolve an online UUID on this server. During a nested player swap,
    /// outer players whose records are displaced return None. API @API:player_read@.
    pub(super) fn native_player_ref(&self, uuid: u128) -> Option<PlayerRef> {
        let epoch = self.native_lifecycle.epoch()?;
        let (_, id) = self.player_lookup(uuid)?;
        Some(PlayerRef {
            entity: crate::mod_api::EntityRef::new(epoch, id),
            uuid,
        })
    }

    fn player_lookup(&self, uuid: u128) -> Option<(PlayerRead<'_>, u32)> {
        if self.lan_acting_uuid == Some(uuid) {
            return Some((
                PlayerRead::Acting {
                    mode: self.mode_id(),
                    body: self.player.as_ref(),
                    abilities: self.acting_abilities,
                },
                self.lan_acting_entity?,
            ));
        }
        if let Some(guest) = self.lan.as_ref().and_then(|host| {
            host.hub.guests.iter().find(|guest| {
                guest.placed && guest.uuid == uuid && !self.lan_guest_swaps.contains(&guest.conn)
            })
        }) {
            return Some((PlayerRead::Guest(guest), guest.entity_id as u32));
        }
        if self.lan_acting_uuid.is_none()
            && self.serving.is_some()
            && self
                .level
                .player_uuid
                .map(horizon_client::lan::playerdata::ints_uuid)
                == Some(uuid)
        {
            let body = self.player.as_ref()?;
            return Some((
                PlayerRead::Acting {
                    mode: self.mode_id(),
                    body: Some(body),
                    abilities: self.acting_abilities,
                },
                body.entity_id as u32,
            ));
        }
        None
    }

    fn player_handle_read(&self, player: PlayerRef) -> Option<PlayerRead<'_>> {
        (self.native_lifecycle.epoch()? == player.entity.epoch()).then_some(())?;
        let (read, id) = self.player_lookup(player.uuid)?;
        (id == player.entity.id()).then_some(read)
    }

    /// `ServerPlayerGameMode.getGameModeForPlayer`; Paper `HumanEntity#getGameMode`.
    /// None for stale, offline or unavailable players (including displaced outer players).
    pub(super) fn native_player_game_mode(&self, player: PlayerRef) -> Option<GameMode> {
        GameMode::from_id(self.player_handle_read(player)?.mode())
    }

    /// Changes a player's game mode as `ServerPlayer.setGameMode` and
    /// `/gamemode` do, including the cancellable change hook and ability
    /// updates. Returns false for a stale or unavailable player, another
    /// player during a swap, no change, or a canceled change. API
    /// @API:player_game_mode_write@.
    pub(super) fn native_player_set_game_mode(
        &mut self,
        player: PlayerRef,
        mode: GameMode,
    ) -> bool {
        if self.player_handle_read(player).is_none() {
            return false;
        }
        let mut changed = false;
        self.native_as_player(player.uuid, |game| {
            changed = game.set_game_mode(super::game_mode::mode_from_id(mode.id()));
        }) && changed
    }

    /// Stores a server player's `Abilities` and tells their client
    /// (`ServerPlayer.onUpdateAbilities`). They stay as written until the
    /// player's game mode changes (`GameType.updatePlayerAbilities`), and are
    /// saved with the player. A speed that is not finite refuses the write
    /// (vanilla's setters take any float). Returns false for a
    /// stale or unavailable player, another player during a swap, and a speed
    /// that is not finite.
    pub(super) fn native_player_set_abilities(
        &mut self,
        player: PlayerRef,
        abilities: NativePlayerAbilities,
    ) -> bool {
        if !abilities.fly_speed.is_finite() || !abilities.walk_speed.is_finite() {
            return false;
        }
        let stored = PlayerAbilities {
            invulnerable: abilities.invulnerable,
            flying: abilities.flying,
            may_fly: abilities.may_fly,
            instabuild: abilities.instabuild,
            may_build: abilities.may_build,
            flying_speed: abilities.fly_speed,
            walking_speed: abilities.walk_speed,
        };
        if self.native_player_ref(player.uuid()) != Some(player) {
            return false;
        }
        let mut stored_here = false;
        self.native_as_player(player.uuid(), |game| {
            let Some(slot) = game.acting_abilities.as_mut() else {
                return;
            };
            *slot = stored;
            if let Some(body) = game.player.as_mut() {
                body.invulnerable = stored.invulnerable;
                body.may_fly = stored.may_fly;
                body.flying = stored.flying;
                body.flying_speed = stored.flying_speed;
            }
            stored_here = true;
        }) && stored_here
    }

    /// `Player.isCreative`; Paper `HumanEntity#getGameMode`.
    pub(super) fn native_player_is_creative(&self, player: PlayerRef) -> Option<bool> {
        Some(self.player_handle_read(player)?.creative())
    }

    /// `Inventory.getSelectedSlot`: the hotbar slot the player holds, 0..9. API
    /// @API:item_places@.
    pub(super) fn native_player_selected_slot(&self, player: PlayerRef) -> Option<usize> {
        Some(match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } => self.selected,
            PlayerRead::Guest(guest) => guest.selected,
        })
    }

    /// `Player.isSpectator`; Paper `HumanEntity#getGameMode`.
    pub(super) fn native_player_is_spectator(&self, player: PlayerRef) -> Option<bool> {
        Some(self.player_handle_read(player)?.mode() == 3)
    }

    /// `Player.mayBuild`; Paper `HumanEntity#getGameMode`.
    pub(super) fn native_player_may_build(&self, player: PlayerRef) -> Option<bool> {
        Some(self.player_handle_read(player)?.may_build())
    }

    /// `FoodData.getFoodLevel/getSaturationLevel/getExhaustionLevel`; Paper
    /// `HumanEntity#getFoodLevel/getSaturation/getExhaustion`. None until the
    /// guest's body exists, as well as for stale or unavailable players.
    pub(super) fn native_player_food(&self, player: PlayerRef) -> Option<NativePlayerFood> {
        self.player_handle_read(player)?.food()
    }

    /// `Player.getAbilities`; Paper `Player#getAllowFlight/isFlying/getFlySpeed/getWalkSpeed`.
    pub(super) fn native_player_abilities(
        &self,
        player: PlayerRef,
    ) -> Option<NativePlayerAbilities> {
        self.player_handle_read(player)?.abilities()
    }

    /// The players on this server, in the order they joined. Empty on a
    /// client and in a world nobody else can join.
    pub(super) fn native_server_players(&self) -> Vec<NativeServerPlayer> {
        let Some(host) = &self.lan else {
            return Vec::new();
        };
        host.hub
            .online_players()
            .map(|g| NativeServerPlayer {
                uuid: g.uuid,
                name: g.name.clone(),
                permission: g.permission,
                dimension: g.dimension.clone(),
                position: g.position,
                game_mode: g.game_mode,
                health: g.health,
                yaw: g.yaw,
                pitch: g.pitch,
            })
            .collect()
    }
    /// A system line in that player's chat, also inside a player's hook for
    /// the acting player; false when they aren't on.
    pub(super) fn native_tell(&mut self, uuid: u128, line: Text) -> bool {
        self.native_send(uuid, system_chat(&line))
    }
    /// A line over that player's hotbar (the action bar), also inside a
    /// player's hook for the acting player; false when they aren't on. API 21.
    pub(super) fn native_action_bar(&mut self, uuid: u128, line: Text) -> bool {
        let packet = C::SystemChat(play::SystemChat {
            content: component_nbt(&line.component()),
            overlay: true,
        });
        self.native_send(uuid, packet)
    }
    /// Sends `packet` to that player, also when they are the acting player
    /// (their packets then go after the hook); false when they aren't on.
    pub(super) fn native_send(&mut self, uuid: u128, packet: C) -> bool {
        self.native_send_all(uuid, vec![packet])
    }
    fn native_send_all(&mut self, uuid: u128, packets: Vec<C>) -> bool {
        if self.lan_acting_uuid == Some(uuid) {
            self.guest_packets.extend(packets);
            return true;
        }
        let Some(host) = &self.lan else {
            return false;
        };
        let Some(g) = host.hub.guests.iter().find(|g| g.placed && g.uuid == uuid) else {
            return false;
        };
        for packet in &packets {
            host.hub.send_to(g.conn, packet);
        }
        true
    }
    /// Shows `sidebar` on that player's screen, in place of the one shown
    /// before, or takes it away (None). Only their client holds it: the
    /// server's scoreboard, its `/scoreboard` and the other players don't
    /// see it, and it is gone when they leave (a mod sends it again when
    /// they come back). It takes the sidebar's place from an objective the
    /// world shows there, for this player, until they rejoin. False when
    /// they aren't on. API 49.
    pub(super) fn native_sidebar(&mut self, uuid: u128, sidebar: Option<&NativeSidebar>) -> bool {
        let objective = SIDEBAR_OBJECTIVE.to_owned();
        // Removed first, so the new one's lines replace all the old ones.
        let mut packets = vec![C::SetObjective(play::SetObjective {
            name: objective.clone(),
            method: 1,
            display: None,
        })];
        if let Some(sidebar) = sidebar {
            let title = component_nbt(&sidebar.title.component());
            packets.push(C::SetObjective(play::SetObjective {
                name: objective.clone(),
                method: 0,
                display: Some((title, 0, Some(play::NumberFormat::Blank))),
            }));
            // DisplaySlot.SIDEBAR.
            packets.push(C::SetDisplayObjective(play::SetDisplayObjective {
                slot: 1,
                objective: objective.clone(),
            }));
            let lines = &sidebar.lines[..sidebar.lines.len().min(15)];
            for (i, (left, right)) in lines.iter().enumerate() {
                packets.push(C::SetScore(play::SetScore {
                    // Highest first. The display name shows, not the owner;
                    // an owner starting with '#' is hidden from the sidebar
                    // (ScoreHolder's isHidden), and ':' is in no player's name.
                    owner: format!("horizon:line{i:02}"),
                    objective: objective.clone(),
                    score: (lines.len() - i) as i32,
                    display: Some(component_nbt(&left.component())),
                    number_format: Some(match right {
                        Some(t) => play::NumberFormat::Fixed(component_nbt(&t.component())),
                        None => play::NumberFormat::Blank,
                    }),
                }));
            }
        }
        self.native_send_all(uuid, packets)
    }
    /// The lines over and under that player's list of players (Tab):
    /// `ClientboundTabListPacket`, an empty text for none. False when they
    /// aren't on. API 49.
    pub(super) fn native_tab_list(&mut self, uuid: u128, header: &Text, footer: &Text) -> bool {
        let packet = C::TabList(play::TabList {
            header: component_nbt(&header.component()),
            footer: component_nbt(&footer.component()),
        });
        self.native_send(uuid, packet)
    }
    /// The server's tick rate (20 unless `/tick rate` changed it) and the
    /// milliseconds its ticks took on average lately, as `/tick query`
    /// reports them. The ticks a second it keeps up is the rate, or
    /// 1000 / milliseconds when that is lower. API 49.
    pub(super) fn native_tick_stats(&self) -> (f32, f64) {
        (
            self.clock.rate(),
            self.clock.average_nanos() as f64 / 1_000_000.0,
        )
    }
    /// Whether that player counts for `players_sleeping_percentage`: an
    /// ignored player who isn't in bed is left out of the players the night
    /// needs asleep, as a spectator is (an away-from-keyboard mod). Kept
    /// until a mod sets it back or the server stops, also if they leave and
    /// come back. Only where no player's state is swapped in, as
    /// `native_run_command`; false there and when they aren't on. API 49.
    pub(super) fn native_sleep_ignored(&mut self, uuid: u128, ignored: bool) -> bool {
        let Some(host) = &mut self.lan else {
            return false;
        };
        if !host.hub.guests.iter().any(|g| g.placed && g.uuid == uuid) {
            return false;
        }
        if ignored {
            host.sleep_ignored.insert(uuid);
        } else {
            host.sleep_ignored.remove(&uuid);
        }
        true
    }
    /// Hides that player (a vanish) from the players below permission
    /// `level`, or shows them again (None): from the next tick those players
    /// lose them from the Tab list and the world (their client stops
    /// tracking the player, as it does a spectator), and don't read their
    /// joining or leaving; players of `level` or more still see them, and
    /// a player made an operator meanwhile sees them again. By UUID: kept
    /// for a player who is off (who joins hidden), until a mod sets it again
    /// or the server stops. Works anywhere on a server, inside a player's
    /// hook too; false on a client and in a world nobody else can join.
    /// API 314.
    pub(super) fn native_hide_player(&mut self, uuid: u128, level: Option<u8>) -> bool {
        let Some(host) = &mut self.lan else {
            return false;
        };
        match level {
            Some(level) => host.hub.hidden.insert(uuid, level),
            None => host.hub.hidden.remove(&uuid),
        };
        true
    }
    /// Shows `nickname` in place of that player's name in chat (their
    /// messages, `/me`, `/msg`) and in the Tab list; None shows their name
    /// again. Hovering the shown name gives the real one, which stays in the
    /// log, commands, selectors and the server's events. Kept until a mod
    /// sets it again or the server stops (also if they leave and come back).
    /// Plain text: the mod checks it (`native_text_allowed`, its length,
    /// other players' names). Only where no player's state is swapped in,
    /// as `native_run_command`; false there and when they aren't on. API
    /// 62.
    pub(super) fn native_set_nickname(&mut self, uuid: u128, nickname: Option<&str>) -> bool {
        let Some(host) = &mut self.lan else {
            return false;
        };
        if !host.hub.guests.iter().any(|g| g.placed && g.uuid == uuid) {
            return false;
        }
        host.hub.set_nickname(uuid, nickname.map(str::to_owned));
        true
    }
    /// Whether the server's chat filter (`server.toml` `[chat_filter]`)
    /// lets `text` through: true without a filter, None inside a player's
    /// hook (where the server is out of reach). API 62.
    pub(super) fn native_text_allowed(&self, text: &str) -> Option<bool> {
        let host = self.lan.as_ref()?;
        Some(
            host.hub
                .chat_filter
                .as_ref()
                .is_none_or(|g| !g.filter.matches(text)),
        )
    }
    /// A system line in every player's chat and the server's log.
    pub(super) fn native_broadcast(&self, mut line: Text) {
        let mut cancel = false;
        self.native_broadcast_message(&mut line, &mut cancel);
        if cancel {
            return;
        }
        let plain = line.plain();
        if self.options.dedicated.is_some() {
            super::dedicated::log("INFO", &plain);
        } else {
            log::info!("{plain}");
        }
        let Some(host) = &self.lan else {
            return;
        };
        let packet = system_chat(&line);
        for g in host.hub.guests.iter().filter(|g| g.placed) {
            host.hub.send_to(g.conn, &packet);
        }
    }
    /// Posts a line to the dedicated server's event stream (`GET /api/v1/events`)
    /// as an `announce` event, which horizon-server's Discord bridge relays when
    /// `[discord] events` lists `announce`: `name` and `uuid` say who it is
    /// about (empty and 0 for no one), `message` is the line (`§` codes colour it).
    /// Only on a dedicated server; false elsewhere and for an empty message.
    /// API 163.
    pub(super) fn native_server_event(&self, name: &str, uuid: u128, message: &str) -> bool {
        if self.options.dedicated.is_none() || message.trim().is_empty() {
            return false;
        }
        super::dedicated::events::record("announce", name, uuid, json!({ "message": message }));
        true
    }
    /// A line of the command's answer, inside `native_perform_command`:
    /// sent to the player whose command it is after the call, as vanilla
    /// commands' feedback is.
    pub(super) fn native_feedback(&mut self, line: Text) {
        match &mut self.chat_capture {
            Some(lines) => lines.push(line),
            None => self.system_message(line),
        }
    }
    /// The player whose action or command a hook hears, as their state is
    /// swapped in (`native_perform_command`, `native_right_click_block`,
    /// `native_use_item_on`, `native_destroy_block`...): None outside such a
    /// hook. API 21.
    pub(super) fn native_acting_player(&self) -> Option<NativeServerPlayer> {
        let p = self.player.as_ref()?;
        Some(NativeServerPlayer {
            uuid: self.lan_acting_uuid?,
            name: self.lan_acting_name.clone()?,
            permission: self.lan_acting_permission?,
            dimension: self.dim.id().to_owned(),
            position: [p.position.x, p.position.y, p.position.z],
            game_mode: self.mode_id(),
            health: p.health.health(),
            yaw: p.yaw,
            pitch: p.pitch,
        })
    }
    /// Runs `line` (no slash) as the server's console does: permission
    /// level 4, from the overworld's spawn, with no output. A teleport to
    /// another level (`execute in minecraft:the_nether run tp Alex 0 70 0`)
    /// happens at the next tick's start, as the console's. Whether it ran
    /// without failing. Only where no player's state is swapped in
    /// (`native_tick`, `native_menu_clicked`): false inside a player's hook,
    /// where the other players are out of reach. API 21.
    pub(super) fn native_run_command(&mut self, line: &str) -> bool {
        if self.lan_acting.is_some() || self.lan.is_none() {
            return false;
        }
        let server = execute::Source {
            dimension: Dim::Overworld,
            ..self.server_source()
        };
        let outer = self.command_source.replace(server);
        let ok = self.as_console(|g| g.run_command(line));
        self.command_source = outer;
        ok
    }
    /// The open world's folder (`world/`, beside its `level.dat`): a server
    /// mod keeps its data in a folder of its own there, so the world's
    /// backups carry it. None when no world is open.
    pub(super) fn native_world_dir(&self) -> Option<PathBuf> {
        (!self.world_root.as_os_str().is_empty()).then(|| self.world_root.clone())
    }
}

/// An experience orb as a server mod reads it (`native_experience_orbs`).
/// Read by mods only.
#[derive(Clone, Debug)]
pub struct NativeExperienceOrb {
    /// The entity's ID, as `native_entity_join_level` names it.
    pub id: u32,
    pub position: [f64; 3],
    /// `ExperienceOrb.value`: the points one pickup gives.
    pub value: u32,
    /// `ExperienceOrb.count`: orbs merged into this one, each worth `value`.
    pub count: u32,
}

/// Whether an orb at `position` (its box: 0.5 wide and high, standing on
/// it) meets the box `min`..`max`, touching faces excluded (`AABB.intersects`).
fn orb_meets(position: [f64; 3], min: [f64; 3], max: [f64; 3]) -> bool {
    position[0] - 0.25 < max[0]
        && position[0] + 0.25 > min[0]
        && position[1] < max[1]
        && position[1] + 0.5 > min[1]
        && position[2] - 0.25 < max[2]
        && position[2] + 0.25 > min[2]
}

/// Experience for server mods. API 101.
#[allow(dead_code)]
impl Game<'_> {
    /// The experience orbs of the server level `dimension` whose box meets
    /// the box `min`..`max`, in the level's order: what a mod reads before it
    /// cancels an orb in `native_entity_join_level`, to give its points with
    /// `PlayerRef::give_experience_points`. Empty for a level not in place and on a
    /// client of a server.
    pub(super) fn native_experience_orbs(
        &self,
        dimension: &str,
        min: [f64; 3],
        max: [f64; 3],
    ) -> Vec<NativeExperienceOrb> {
        if (self.serving.is_none() && self.options.dedicated.is_none())
            || dimension != self.dim.id()
        {
            return Vec::new();
        }
        self.orbs
            .iter()
            .filter(|o| o.count > 0)
            .map(|o| NativeExperienceOrb {
                id: o.id,
                position: [o.position.x, o.position.y, o.position.z],
                value: o.value,
                count: o.count,
            })
            .filter(|o| orb_meets(o.position, min, max))
            .collect()
    }
    /// Writes the experience orb `id`'s `value` (the points a pickup gives), `count`
    /// (orbs merged into it, each worth `value`) and `age` (ticks lived; an orb
    /// despawns at 6000); a `None` is left as it is. API 128.
    /// False, and nothing written, for an orb not in the server level `dimension`, a
    /// `value` or `count` of 0, and on a client of a server.
    pub(super) fn native_set_experience_orb(
        &mut self,
        dimension: &str,
        id: u32,
        value: Option<u32>,
        count: Option<u32>,
        age: Option<u32>,
    ) -> bool {
        if (self.serving.is_none() && self.options.dedicated.is_none())
            || dimension != self.dim.id()
            || value == Some(0)
            || count == Some(0)
        {
            return false;
        }
        let Some(orb) = self.orbs.iter_mut().find(|o| o.id == id) else {
            return false;
        };
        orb.value = value.unwrap_or(orb.value);
        orb.count = count.unwrap_or(orb.count);
        orb.age = age.unwrap_or(orb.age);
        true
    }
    /// Runs `f` with the player `uuid`'s state in place: theirs already when
    /// they are the acting player (the host, or a guest swapped in), else a
    /// guest's through `as_guest`, only where no other player is swapped in.
    /// False when it didn't run: no such player here, or dead.
    pub(super) fn native_as_player(&mut self, uuid: u128, f: impl FnOnce(&mut Self)) -> bool {
        if uuid == self.native_acting_id() {
            if self
                .player
                .as_ref()
                .is_none_or(|p| p.health.health() <= 0.0)
            {
                return false;
            }
            f(self);
            return true;
        }
        if self.lan_acting.is_some() {
            return false;
        }
        let Some(mut host) = self.lan.take() else {
            return false;
        };
        let conn = host
            .hub
            .guests
            .iter()
            .find(|g| g.placed && !g.dead && g.uuid == uuid)
            .map(|g| g.conn);
        let ran = conn
            .and_then(|conn| self.as_guest(&mut host, conn, f))
            .is_some();
        self.lan = Some(host);
        ran
    }
}

fn system_chat(line: &Text) -> C {
    C::SystemChat(play::SystemChat {
        content: component_nbt(&line.component()),
        overlay: false,
    })
}

/// Native command metadata and the server dispatcher containing their parsed usages.
pub(super) struct NativeCommands {
    levels: HashMap<String, u8>,
    pub(super) dispatcher: Arc<Dispatcher>,
}

/// Where the game calls the server hook points.
impl Game<'_> {
    /// Asks the mods for their commands once per hub, and puts them in every
    /// player's command tree.
    pub(super) fn register_native_commands(&mut self, host: &mut LanHost) {
        if host.native_commands.is_some() {
            return;
        }
        let mut commands = Vec::new();
        self.native_register_commands(&mut commands);
        let vanilla = horizon_client::command::vanilla::game_dispatcher_with_features(
            &self.level.enabled_features,
        );
        let (tree, levels, problems) = command_tree(&commands, |name| {
            host.hub.vanilla_command(name) || vanilla.find(name).is_some()
        });
        for p in problems {
            log::warn!("Horizon: a native mod's command was left out: {p}");
        }
        let mut dispatcher = Dispatcher::new();
        register_dispatcher(&mut dispatcher, &tree);
        let dispatcher = Arc::new(dispatcher);
        host.hub.set_mod_commands(0, tree);
        host.native_commands = Some(NativeCommands { levels, dispatcher });
    }

    /// Asks the mods what root commands each player's tree keeps
    /// (`native_player_commands_sent`) and stores the answer in the hub,
    /// which sends the trees without what the mods took out: for `only`, or
    /// for every placed player.
    pub(super) fn native_filter_commands(&mut self, host: &mut LanHost, only: Option<u32>) {
        let players: Vec<(u32, u128, String)> = host
            .hub
            .guests
            .iter()
            .filter(|g| only.map_or(g.placed, |conn| g.conn == conn))
            .map(|g| (g.conn, g.uuid, g.name.clone()))
            .collect();
        for (conn, uuid, name) in players {
            let names = host.hub.command_names(conn);
            let mut kept = names.clone();
            let placed = host.hub.guest(conn).is_some_and(|g| g.placed);
            if !placed {
                // A joining player has no world yet: nothing can be told them.
                self.native_player_commands_sent(uuid, &name, &mut kept);
            } else if self.lan_acting == Some(conn) {
                self.native_player_commands_sent(uuid, &name, &mut kept);
            } else {
                self.as_guest(host, conn, |g| {
                    g.native_player_commands_sent(uuid, &name, &mut kept);
                });
            }
            if let Some(g) = host.hub.guest_mut(conn) {
                g.hidden_commands = names.into_iter().filter(|n| !kept.contains(n)).collect();
            }
        }
    }

    /// `Hub::resend_commands` with the mods' say on each tree.
    pub(super) fn resend_commands_filtered(&mut self) {
        let Some(mut host) = self.lan.take() else {
            return;
        };
        host.hub.refresh_permissions();
        self.native_filter_commands(&mut host, None);
        host.hub.resend_commands();
        self.lan = Some(host);
    }

    /// The level a guest needs for a mod's command (`line`, without its
    /// slash), when a mod registered it.
    pub(super) fn native_command_level(host: &LanHost, line: &str) -> Option<u8> {
        let name = line.split(' ').next().unwrap_or_default();
        host.native_commands.as_ref()?.levels.get(name).copied()
    }

    /// Parse a registered command using the source's permission before dispatching it.
    fn native_command_valid(&mut self, line: &str, permission: u8) -> bool {
        let Some(commands) = self
            .lan
            .as_ref()
            .and_then(|host| host.native_commands.as_ref())
        else {
            return false;
        };
        let dispatcher = commands.dispatcher.clone();
        let previous = std::mem::replace(&mut self.ui.registries.permission, permission);
        let parse = dispatcher.parse(line, &self.ui.registries);
        self.ui.registries.permission = previous;
        if let Err(error) = parse.executable() {
            self.syntax_failure(&error, line);
            return false;
        }
        true
    }

    /// A guest's command for the mods, inside their swap
    /// (`run_guest_command`): true when one ran it.
    pub(super) fn perform_native_command(&mut self, conn: u32, permission: u8, line: &str) -> bool {
        let Some(host) = &self.lan else {
            return false;
        };
        if Self::native_command_level(host, line).is_none_or(|level| level > permission) {
            return false;
        }
        let Some(g) = host.hub.guest(conn) else {
            return false;
        };
        let player = NativeServerPlayer {
            uuid: g.uuid,
            name: g.name.clone(),
            permission,
            dimension: g.dimension.clone(),
            position: g.position,
            game_mode: g.game_mode,
            health: g.health,
            yaw: g.yaw,
            pitch: g.pitch,
        };
        if !self.native_command_valid(line, permission) {
            return true;
        }
        let mut ran = false;
        self.native_perform_command(&player, line, &mut ran);
        if !ran {
            // Registered, but no mod took it: vanilla's unknown command.
            self.native_feedback(
                Text::translate("command.unknown.command", &[])
                    .with_color(horizon_client::text::RED),
            );
        }
        true
    }
}

impl Game<'_> {
    /// A console line for the mods: true when a mod registered its command
    /// (`native_register_commands`). The console is level 4, so a registered
    /// usage is always allowed; a command no mod takes answers as vanilla's
    /// unknown command.
    pub(super) fn perform_native_console_command(&mut self, line: &str) -> bool {
        let Some(host) = &self.lan else {
            return false;
        };
        if Self::native_command_level(host, line).is_none() {
            return false;
        }
        if !self.native_command_valid(line, 4) {
            return true;
        }
        let mut ran = false;
        self.native_perform_console_command(line, &mut ran);
        if !ran {
            self.native_feedback(
                Text::translate("command.unknown.command", &[])
                    .with_color(horizon_client::text::RED),
            );
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::items::ItemStack;

    fn player_guest() -> Guest {
        use horizon_client::lan::{Joined, PlayerSender};
        use horizon_protocol::packets::configuration::ClientInformation;
        use std::sync::{
            Arc,
            atomic::{AtomicI32, AtomicUsize},
        };
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
            },
            1,
            [0.0; 3],
            10,
        )
    }

    #[test]
    fn player_reads_use_guest_mode_and_abilities() {
        let mut guest = player_guest();
        guest.abilities.flying_speed = 0.08;
        let read = PlayerRead::Guest(&guest);
        assert_eq!(read.mode(), 0);
        assert_eq!(
            read.food(),
            None,
            "food is unavailable before body creation"
        );
        assert_eq!(
            read.abilities(),
            Some(NativePlayerAbilities {
                invulnerable: false,
                flying: false,
                may_fly: false,
                instabuild: false,
                may_build: true,
                fly_speed: 0.08,
                walk_speed: 0.1,
            })
        );
        guest.game_mode = 1;
        guest.abilities.update_for_mode(1);
        guest.abilities.flying = true;
        let read = PlayerRead::Guest(&guest);
        assert_eq!(read.mode(), 1);
        assert_eq!(
            read.abilities(),
            Some(NativePlayerAbilities {
                invulnerable: true,
                flying: true,
                may_fly: true,
                instabuild: true,
                may_build: true,
                fly_speed: 0.08,
                walk_speed: 0.1,
            })
        );
        guest.game_mode = 3;
        guest.abilities.update_for_mode(3);
        let abilities = PlayerRead::Guest(&guest).abilities().unwrap();
        assert!(!abilities.may_build && !abilities.instabuild && abilities.may_fly);
    }

    fn tree(commands: &[NativeCommand]) -> (Value, HashMap<String, u8>, Vec<String>) {
        command_tree(commands, |name| name == "give")
    }

    fn stack(item: &str, count: u32) -> Option<ItemStack> {
        Some(ItemStack {
            item: item.into(),
            count,
            damage: 0,
            extra: None,
        })
    }

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

    #[test]
    fn usages_merge_into_one_tree_with_runnable_nodes() {
        let (t, levels, problems) = tree(&[
            NativeCommand::new("balance [player:player]"),
            NativeCommand::new("pay <player:player> <amount:integer>"),
            NativeCommand::op("eco give <player:player> <amount:integer>", 3),
            NativeCommand::op("eco take <player:player> <amount:integer>", 3),
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
                tree(&[NativeCommand::new(first), NativeCommand::new(second)]);
            assert_eq!(problems.len(), 1, "{first} then {second}: {problems:?}");
            assert!(problems[0].starts_with(&format!("`{second}`")));
            let mut dispatcher = Dispatcher::new();
            register_dispatcher(&mut dispatcher, &tree);
            let registries = horizon_client::command::Registries::default();
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

    #[test]
    fn every_supported_kind_has_a_parser() {
        assert!(KINDS.into_iter().all(|kind| parser(kind).is_some()));
    }

    #[test]
    fn native_usages_parse_with_their_permissions_in_the_server_dispatcher() {
        let (tree, _, problems) = tree(&[
            NativeCommand::new("warp [name:word]"),
            NativeCommand::op("warp set <amount:integer>", 2),
            NativeCommand::new("toggle <enabled:boolean>"),
        ]);
        assert!(problems.is_empty(), "{problems:?}");
        let mut dispatcher = Dispatcher::new();
        register_dispatcher(&mut dispatcher, &tree);
        let mut registries = horizon_client::command::Registries::default();
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

    #[test]
    fn a_command_takes_its_lowest_level() {
        let (t, levels, _) = tree(&[
            NativeCommand::new("warp [name:word]"),
            NativeCommand::op("warp set <name:word>", 2),
        ]);
        assert_eq!(levels["warp"], 0);
        assert!(t["children"]["warp"].get("permissions").is_none());
        let set = &t["children"]["warp"]["children"]["set"];
        assert_eq!(set["permissions"]["permission"]["level"], "gamemasters");
    }

    #[test]
    fn bad_usages_and_vanilla_names_are_left_out() {
        let (t, levels, problems) = tree(&[
            NativeCommand::new("give <x:integer>"),
            NativeCommand::new("a [x:integer] <y:integer>"),
            NativeCommand::new("b <x:text> <y:word>"),
            NativeCommand::new("c <x:colour>"),
            NativeCommand::new("<x:integer>"),
            NativeCommand::new("Bad"),
            NativeCommand::new("d <x:integer>"),
            NativeCommand::new("d <x:word>"),
        ]);
        assert_eq!(problems.len(), 7, "{problems:?}");
        assert_eq!(levels.keys().collect::<Vec<_>>(), ["d"]);
        assert_eq!(
            t["children"]["d"]["children"]["x"]["parser"],
            "brigadier:integer"
        );
        let (none, levels, _) = tree(&[]);
        assert!(none.is_null() && levels.is_empty());
    }
}
