pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// Dropped-item contact after vertical collision response and before block effects.
    fn native_item_after_vertical_collision(
        &mut self,
        call: crate::game::native_hooks::fire::ItemAfterVerticalCollisionCall<'_>,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_item_after_vertical_collision(self, call);
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
