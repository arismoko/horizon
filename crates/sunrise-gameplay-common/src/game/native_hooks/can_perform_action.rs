use super::*;

/// `IItemExtension.canPerformAction`: can `stack` perform the item ability
/// `ability` (`shield_block`, `shears_harvest`...)?
/// `can_perform` arrives with vanilla's answer; the parts of the stack's
/// item may set it. Both sides.
#[inline(never)]
pub fn native_can_perform_action(
    host: &dyn mod_api::Host,
    stack: &ItemStack,
    ability: &str,
    can_perform: &mut bool,
) {
    let Ok(metadata) = crate::game::api_values::stack_read_metadata(stack) else {
        return;
    };
    let project = |metadata| crate::game::api_values::project_stack(stack, metadata).ok();
    let context = mod_api::ItemAbilityContext::new(host.tags());
    crate::game::native_hooks::fire::fire_can_perform_action(
        &context,
        mod_api::__private::CanPerformActionCall {
            stack: mod_api::ItemStackView::resident(metadata, &project),
            ability,
            can_perform,
        },
    );
}

/// The hook points. Each does nothing in Sunrise.
pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// `IItemExtension.canPerformAction`: can `stack` perform the item ability
    /// `ability` (`shield_block`, `shears_harvest`...)?
    /// `can_perform` arrives with vanilla's answer; the parts of the stack's
    /// item may set it. Both sides.
    #[inline(never)]
    fn native_can_perform_action(&self, stack: &ItemStack, ability: &str, can_perform: &mut bool) {
        native_can_perform_action(self.as_host(), stack, ability, can_perform);
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
