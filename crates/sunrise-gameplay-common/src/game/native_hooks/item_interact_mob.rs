use super::*;

pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// an item fallback after a native mob's ordinary interaction passed.
    fn native_item_interact_mob(
        &mut self,
        hand: Hand,
        entity: i32,
        kind: &str,
        stack: &ItemStack,
        result: &mut Option<mod_api::InteractionResult>,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_item_interact_mob(
            self, hand, entity, kind, stack, result,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
