/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// NeoForge's `ClientPlayerChangeGameTypeEvent`: observe any `PlayerInfo` game type change.
    pub(in crate::game) fn native_client_player_change_game_type(
        &mut self,
        player_id: Option<i32>,
        player_uuid: u128,
        current_game_type: i32,
        new_game_type: i32,
    ) {
        self.fire_client_player_change_game_type(
            player_id,
            player_uuid,
            current_game_type,
            new_game_type,
        );
    }
}
