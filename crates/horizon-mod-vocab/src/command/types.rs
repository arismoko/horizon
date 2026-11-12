//! Commands and their authority.

/// A command a native mod adds, as one usage: words, then arguments
/// `<name:kind>`, optional ones last in brackets `[name:kind]`:
/// `pay <player:player> <amount:integer>`, `balance [player:player]`,
/// `eco give <player:player> <amount:integer>`. Kinds: `word`, `text` (the
/// rest of the line, last), `integer`, `number`, `boolean`, `player`, `players`,
/// `entity`, `entities`, `item`, `block_pos`, `vec3`, or a command argument type a mod registered with
/// `Registrar::register_command_argument`, named by its namespaced id:
/// `machine use <machine:mymod:machine>`. Usages of one command share their
/// words; the server merges them into one tree. Each executable usage keeps its own level:
/// a public `foo bar` lets everyone traverse `foo`, but does not make an
/// operator-only `foo` executable. Optional arguments also allow execution at
/// their preceding prefixes, at that usage's level.
///
/// Side: server. Selector arguments use vanilla's source position, permissions,
/// cardinality and selection order; `player`/`entity` require one result and
/// `players`/`entities` require a nonempty result. Unresolvable entity handles are
/// omitted; lists have no extra cap beyond the wire frame limits.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct Command {
    /// The usage.
    pub usage: String,
    /// The operator level a player needs to run this usage (0: everyone).
    /// Shared prefixes stay visible when needed to reach a lower-level usage.
    pub level: u8,
    /// The permission node a player needs to run the command
    /// (`Command::with_node`), None for the level alone.
    pub node: Option<String>,
}

impl Command {
    /// A usage every player may run.
    pub fn new(usage: &str) -> Self {
        Self {
            usage: usage.to_owned(),
            level: 0,
            node: None,
        }
    }
    /// The same, for operators of `level` and above.
    pub fn op(usage: &str, level: u8) -> Self {
        Self {
            usage: usage.to_owned(),
            level: level.min(4),
            node: None,
        }
    }
    /// The same, for players who hold permission `node`
    /// (`PlayerRef::has_permission`, Bukkit's `Command.setPermission`): a
    /// permissions mod's answer (`player_permission_check`) decides, and
    /// `level` only when no mod answers. The node gates this usage's literal
    /// path (`home admin`), as Brigadier's `requires` on that literal: a
    /// usage without a node takes its command's (the node of the command's
    /// first usage), and a node on the bare command (`home`) gates all of it.
    /// A path keeps one node; a usage naming another than one before it on
    /// the same path is left out, with why. Players without it neither run
    /// nor see that path; the console, functions and command blocks are not
    /// asked.
    pub fn with_node(mut self, node: &str) -> Self {
        self.node = Some(node.to_owned());
        self
    }
}

/// Which `Game` observes the phase; a mirror `Game` never fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum Authority {
    /// A single-player game with no server thread; never fires today.
    LocalAuthoritative,
    /// The server thread of a single-player or LAN world.
    IntegratedServer,
    /// A dedicated server.
    DedicatedServer,
    /// A client of someone else's server; never fires today.
    RemoteClient,
}

/// `PermissionLevel`: how much a player may do, as `op-permission-level` and
/// the operator list name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum PermissionLevel {
    /// 0: every player.
    All,
    /// 1: moderators.
    Moderators,
    /// 2: game masters.
    Gamemasters,
    /// 3: admins.
    Admins,
    /// 4: owners.
    Owners,
}

impl PermissionLevel {
    /// The level a list entry's number names; out of range keeps to 0 or 4.
    pub fn from_level(level: i32) -> Self {
        match level {
            i32::MIN..=0 => Self::All,
            1 => Self::Moderators,
            2 => Self::Gamemasters,
            3 => Self::Admins,
            _ => Self::Owners,
        }
    }
    /// The level as the list stores it, 0 to 4.
    pub fn level(self) -> u8 {
        self as u8
    }
}
