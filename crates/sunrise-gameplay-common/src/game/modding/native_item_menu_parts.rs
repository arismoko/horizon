//! The executable's side of the item hooks the client library asks inside its menus and
//! recipe matching. Each asks the parts of the stack's item, with the loop context of the
//! side that runs the menu or the match, and no `Host`.
use super::api_values::{TryIntoEngine, with_stack_view};
use sunrise_items::items::ItemStack;
use sunrise_world::native_load_context::LoopContext;

/// Ask the parts of `stack`'s item whether the grindstone repairs it.
pub fn can_grindstone_repair(context: &LoopContext, stack: &ItemStack, can_repair: &mut bool) {
    if mod_api::__private::item_listens(mod_api::HookId::CanGrindstoneRepair, &stack.item) {
        with_stack_view(Some(stack), |view| {
            if let Some(view) = view {
                sunrise_sim::stations::native_can_grindstone_repair(context, view, can_repair);
            }
        });
    }
}

/// Ask the parts of `stack`'s item whether two of it combine in a crafting grid.
pub fn is_combine_repairable(context: &LoopContext, stack: &ItemStack, repairable: &mut bool) {
    if mod_api::__private::item_listens(mod_api::HookId::IsCombineRepairable, &stack.item) {
        with_stack_view(Some(stack), |view| {
            if let Some(view) = view {
                sunrise_sim::crafting::native_hooks::native_is_combine_repairable(
                    context, view, repairable,
                );
            }
        });
    }
}

/// Ask the parts of `stack`'s item whether it may go inside an item that holds items.
pub fn can_fit_inside_container_items(context: &LoopContext, stack: &ItemStack, fits: &mut bool) {
    if mod_api::__private::item_listens(mod_api::HookId::CanFitInsideContainerItems, &stack.item) {
        with_stack_view(Some(stack), |view| {
            if let Some(view) = view {
                sunrise_sim::menu::container_items::native_can_fit_inside_container_items(
                    context, view, fits,
                );
            }
        });
    }
}

/// Ask the parts of `stack`'s item for its name (`Item.getName(ItemStack)`).
pub fn item_name(
    context: &LoopContext,
    stack: &ItemStack,
    name: &mut Option<sunrise_game_data::text::Text>,
) {
    if !mod_api::__private::item_listens(mod_api::HookId::ItemName, &stack.item) {
        return;
    }
    let mut answer: Option<mod_api::Text> = None;
    with_stack_view(Some(stack), |view| {
        if let Some(view) = view {
            sunrise_sim::stations::native_item_name(context, view, &mut answer);
        }
    });
    if let Some(text) = answer {
        match text.try_into_engine(&()) {
            Ok(text) => *name = Some(text),
            Err(error) => log::warn!(
                "Sunrise: item_name of {} skipped: its text can't be shown ({error:?}); the default name is kept",
                stack.item
            ),
        }
    }
}
