use super::*;
use sunrise_gameplay_common::game::api_values::with_stack_view;

/// The hook points. Each does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// `ItemStack.onLeftClickEntity` for the acting player's blow at `entity`
    /// of `kind`, once `attack_entity` let it through: true when the held
    /// item's Part canceled the attack. Nothing runs for a hand holding an
    /// item without the Part.
    pub(crate) fn native_left_click_entity_event(&mut self, entity: i32, kind: &str) -> bool {
        if !mod_api::__private::has_part_hook(mod_api::HookId::OnLeftClickEntity) {
            return false;
        }
        let Some(stack) = self.common.held() else {
            return false;
        };
        if !mod_api::__private::item_listens(mod_api::HookId::OnLeftClickEntity, &stack.item) {
            return false;
        }
        // Keep the stack which started the action while the Part has a mutable Host.
        let stack = stack.clone();
        let player = self.native_acting_id();
        let mut canceled = false;
        with_stack_view(Some(&stack), |view| {
            if let Some(view) = view {
                self.native_on_left_click_entity(player, view, entity, kind, &mut canceled);
            }
        });
        canceled
    }
}

/// The hook points. Each does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// `IItemExtension.onLeftClickEntity`: `player`, holding `stack`, is
    /// about to attack the entity `entity` of type `kind`, where
    /// `Player.attack` starts, once `attack_entity` let the blow through and
    /// before any damage, `hurt_enemy` or wear. Set `canceled` and the attack
    /// ends there.
    pub(in crate::game) fn native_on_left_click_entity(
        &mut self,
        player: u128,
        stack: mod_api::ItemStackView<'_>,
        entity: i32,
        kind: &str,
        canceled: &mut bool,
    ) {
        self.fire_on_left_click_entity(player, stack, entity, kind, canceled);
    }
}
