//! Permission nodes for native server mods (`PlayerRef::has_permission`,
//! `PlayerRef::permission_value`, `Command::with_node`): a permissions mod
//! answers `player_permission_check`; without an answer, vanilla's operator
//! levels decide.
use super::Game;
use mod_api::PlayerRef;

/// The permission level a node falls back to when no mod answers: vanilla's
/// `Commands.LEVEL_GAMEMASTERS`, as Bukkit's `PermissionDefault.OP`.
pub(super) const NODE_FALLBACK_LEVEL: u8 = 2;

impl Game<'_> {
    /// The permissions mods' answer for `node`, asked of the player by UUID
    /// and name (a joining player has no handle yet).
    pub(super) fn permission_answer(&self, uuid: u128, name: &str, node: &str) -> Option<bool> {
        let mut value = None;
        self.native_player_permission_check(uuid, name, node, &mut value);
        value
    }

    /// The permissions mods' answer for permission `node`: None when no mod
    /// set it, for a stale or unavailable player, and on a client.
    pub(super) fn player_permission_value(&self, player: PlayerRef, node: &str) -> Option<bool> {
        if !self.is_logical_server() {
            return None;
        }
        let name = self.player_name(player)?;
        self.permission_answer(player.uuid(), &name, node)
    }

    /// Whether the player holds permission `node`: the mods' answer, else a
    /// permission level of 2 or more. False for a stale or unavailable player.
    pub(super) fn player_has_permission(&self, player: PlayerRef, node: &str) -> bool {
        let Some(level) = self.player_permission_level(player) else {
            return false;
        };
        self.player_permission_value(player, node)
            .unwrap_or(level >= NODE_FALLBACK_LEVEL)
    }
}
