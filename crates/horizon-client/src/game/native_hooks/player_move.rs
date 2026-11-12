use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Server player `player` (their UUID) moved from `from` to `to` (or turned from
    /// `from_rotation` to `to_rotation`) since they were last heard; set
    /// `cancel` to teleport them back to `from`. Server-side.
    pub(in crate::game) fn native_player_move(
        &mut self,
        player: u128,
        from: [f64; 3],
        from_rotation: [f32; 2],
        to: [f64; 3],
        to_rotation: [f32; 2],
        cancel: &mut bool,
    ) {
        self.fire_player_move(player, from, from_rotation, to, to_rotation, cancel);
    }
}
