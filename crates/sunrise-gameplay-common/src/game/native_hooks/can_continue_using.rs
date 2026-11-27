use super::*;

/// The hook points. Each does nothing in Sunrise.
pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// `IItemExtension.canContinueUsing`: does the use of `stack` go on with
    /// `held` in the used `hand`? `entity` is the user's id, a player's included.
    /// `can_continue` arrives with vanilla's `isSameItem`; the parts of
    /// `stack`'s item may set it either way. Both sides.
    #[inline(never)]
    fn native_can_continue_using(
        &self,
        stack: &ItemStack,
        held: &ItemStack,
        entity: Option<i32>,
        hand: Hand,
        can_continue: &mut bool,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_can_continue_using(
            self,
            stack,
            held,
            entity,
            hand,
            can_continue,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
