pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// Declare that a registered block entity type uses actual-state creation.
    /// Asked once per block per level. Set true and never clear an earlier answer.
    fn native_stateful_block_entity(&self, block: &str, stateful: &mut bool) {
        crate::game::native_hooks::fire::FireHooks::fire_stateful_block_entity(
            self, block, stateful,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
