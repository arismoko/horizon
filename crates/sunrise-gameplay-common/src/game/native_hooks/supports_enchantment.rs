use super::*;

pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// Resolve the item's support answer with its default stack when parts install.
    fn native_supports_enchantment(
        &self,
        stack: &ItemStack,
        enchantment: &str,
        answer: &mut Option<bool>,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_supports_enchantment(
            self,
            stack,
            enchantment,
            answer,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
