/// The hook points. Each does nothing in Sunrise.
pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// The process is about to exit after its game has stopped.
    fn native_game_shutting_down(&mut self) {
        crate::game::native_hooks::fire::FireHooks::fire_game_shutting_down(self);
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
