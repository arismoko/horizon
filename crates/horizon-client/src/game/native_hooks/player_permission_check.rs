use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The permission provider: whether `player` (`name`) holds permission
    /// `node`, as a permissions mod keeps it; `value` stays None when no mod
    /// sets it. Server-side.
    pub(in crate::game) fn native_player_permission_check(
        &self,
        player: u128,
        name: &str,
        node: &str,
        value: &mut Option<bool>,
    ) {
        self.fire_player_permission_check(player, name, node, value);
    }
}
