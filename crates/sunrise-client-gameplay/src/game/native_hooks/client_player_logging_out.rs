use mod_api::{GameMode, PlayerRef};

/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Before client disconnect teardown (`Minecraft.disconnect`, NeoForge's
    /// `ClientPlayerNetworkEvent.LoggingOut`). The player and game mode are
    /// absent on no-player disconnect transitions. Client-side.
    pub(in crate::game) fn native_client_player_logging_out(
        &mut self,
        player: Option<PlayerRef>,
        game_mode: Option<GameMode>,
    ) {
        self.fire_client_player_logging_out(player, game_mode);
    }
}
