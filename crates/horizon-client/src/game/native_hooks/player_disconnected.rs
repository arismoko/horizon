use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Why `player` (`name`) left, right after `native_player_logged_out`:
    /// `reason` and the console's `message`. Server-side.
    pub(in crate::game) fn native_player_disconnected(
        &mut self,
        player: u128,
        name: &str,
        reason: mod_api::QuitReason,
        message: &str,
    ) {
        self.fire_player_disconnected(player, name, reason, message);
    }
}
