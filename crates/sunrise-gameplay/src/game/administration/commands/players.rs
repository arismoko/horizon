//! Commands acting on the other players of a hosted world or a server
//! (EntitySelector.findPlayers and EntityArgument's player targets). The
//! player in place (the host's, or the guest whose turn it is) is
//! `Target::Player`; every other guest is `Target::Guest`, found by name,
//! UUID or selector, and what a command does to them runs with their state
//! swapped in (`for_player`), as their own turn, what it tells them going to
//! their chat.
use super::Target;

/// GameType.getName for a guest's game mode id.
pub(in crate::game) fn mode_name(id: i32) -> &'static str {
    match id {
        1 => "creative",
        2 => "adventure",
        3 => "spectator",
        _ => "survival",
    }
}

impl crate::game::ServerGame<'_> {
    /// `CommandSourceStack.getPlayerOrException` for the commands whose
    /// player argument is optional: the source's player (the player in
    /// place when it has none, as before).
    pub(in crate::game) fn implied_player(&self) -> Target {
        self.source_entity()
            .filter(|t| t.is_player())
            .unwrap_or(Target::Player)
    }

    /// The player whose nickname is `name` (any case, a leading `~` either
    /// side ignored).
    pub(in crate::game::server_side) fn player_nicknamed(&self, name: &str) -> Option<Target> {
        let bare = |s: &str| s.strip_prefix('~').unwrap_or(s).to_ascii_lowercase();
        let wanted = bare(name);
        let hub = &self.server.lan.as_ref()?.hub;
        let uuid = hub
            .nicknames
            .iter()
            .find(|(_, nick)| bare(nick) == wanted)?
            .0;
        self.target_by_uuid(*uuid)
    }

    /// A guest's experience level and game mode, for `level=` and `gamemode=`.
    pub(in crate::game) fn guest_level_and_mode(&self, conn: u32) -> Option<(i32, &'static str)> {
        self.command_guest(conn)
            .map(|g| (g.xp_level, mode_name(g.game_mode)))
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
