#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Relative turn delta about to be applied to the local player (`Entity.turn`), in
    /// `Entity.turn` units where `1.0` equals `0.15` degrees (`x` is yaw, `y` is pitch).
    /// Client-side.
    pub(in crate::game) fn native_player_turn(
        &self,
        player_entity_id: i32,
        player_uuid: u128,
        x: &mut f64,
        y: &mut f64,
    ) {
        self.fire_player_turn(player_entity_id, player_uuid, x, y);
    }
}
