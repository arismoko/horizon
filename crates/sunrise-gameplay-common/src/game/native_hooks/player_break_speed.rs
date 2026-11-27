/// The hook points. Each does nothing in Sunrise.
pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// A player's destroy speed for the block at `pos` may be changed or canceled.
    fn native_player_break_speed(
        &mut self,
        player: u32,
        pos: [i32; 3],
        state: u32,
        original_speed: f32,
        new_speed: &mut f32,
        canceled: &mut bool,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_player_break_speed(
            self,
            player,
            pos,
            state,
            original_speed,
            new_speed,
            canceled,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
