use super::*;

pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// `IItemExtension.isPiglinCurrency`: is `stack` barter currency to a piglin?
    /// `currency` arrives with vanilla's answer (a gold ingot); the parts of the
    /// stack's item may set it. Both sides.
    #[inline(never)]
    fn native_is_piglin_currency(&self, stack: &ItemStack, currency: &mut bool) {
        crate::game::native_hooks::fire::FireHooks::fire_is_piglin_currency(self, stack, currency);
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
