use super::*;
use crate::game::api_values::with_stack_view;

pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// `IItemExtension.onEntitySwing`: the player in place (`entity`) is
    /// about to swing `stack` in `hand`, where `LivingEntity.swing` starts.
    /// Set `canceled` and no swing starts: no animation, no packet.
    fn native_on_entity_swing(
        &mut self,
        stack: mod_api::ItemStackView<'_>,
        entity: Option<i32>,
        hand: Hand,
        canceled: &mut bool,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_on_entity_swing(
            self, stack, entity, hand, canceled,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}

pub trait ItemAttackGameExt2<'__sg>: crate::game::SideGame<'__sg> {
    /// `ItemStack.onEntitySwing` for the player in place swinging `hand`:
    /// true when the item's Part canceled the swing. Nothing runs for a hand
    /// holding nothing or an item without the Part.
    fn native_entity_swing_event(&mut self, hand: Hand) -> bool {
        if !mod_api::__private::has_part_hook(mod_api::HookId::OnEntitySwing) {
            return false;
        }
        let stack = match hand {
            Hand::Main => self.held(),
            Hand::Off => self.offhand.as_ref(),
        };
        let Some(stack) = stack else {
            return false;
        };
        if !mod_api::__private::item_listens(mod_api::HookId::OnEntitySwing, &stack.item) {
            return false;
        }
        let stack = stack.clone();
        let entity = self.player.as_ref().map(|p| p.entity_id);
        let mut canceled = false;
        with_stack_view(Some(&stack), |view| {
            if let Some(view) = view {
                self.native_on_entity_swing(view, entity, hand, &mut canceled);
            }
        });
        canceled
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> ItemAttackGameExt2<'__sg> for G {}
