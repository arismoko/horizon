/// The hook points. Each does nothing in Sunrise.
pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// A player's ability to harvest the block at `pos` may be changed.
    fn native_player_harvest_check(
        &mut self,
        player: u32,
        pos: [i32; 3],
        state: u32,
        can_harvest: &mut bool,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_player_harvest_check(
            self,
            player,
            pos,
            state,
            can_harvest,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
