use super::*;

/// The hook points. Each does nothing in Sunrise.
pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// `IItemExtension.getCreatorModId`: the mod that made `stack`.
    /// `mod_id` arrives as the item id's namespace; set it to another mod,
    /// or `None`.
    fn native_creator_mod_id(&self, stack: &ItemStack, mod_id: &mut Option<String>) {
        crate::game::native_hooks::fire::FireHooks::fire_creator_mod_id(self, stack, mod_id);
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
