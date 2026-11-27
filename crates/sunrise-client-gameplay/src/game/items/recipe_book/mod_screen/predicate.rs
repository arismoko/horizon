//! Predicate stock is rebuilt only when the panel's component-aware stock cache changes.
use super::*;
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(super) fn mod_book_placement_stocks(&mut self) -> Vec<(ItemStack, i32)> {
        let Some(menu) = self.native_screen_menu() else {
            return Vec::new();
        };
        if mod_api::RecipeBookLayout::declaring_mod(menu).is_none() {
            return Vec::new();
        }
        let Some(layout) = self
            .common
            .native_net
            .placement
            .view
            .as_ref()
            .and_then(|view| view.index.layout(menu))
        else {
            return Vec::new();
        };
        let slots = match layout.declaration() {
            mod_api::RecipeBookLayout::Crafting { slots, .. } => slots.clone(),
            mod_api::RecipeBookLayout::Furnace { input, .. }
            | mod_api::RecipeBookLayout::Stonecutter { input, .. } => vec![*input],
            mod_api::RecipeBookLayout::Smithing {
                template,
                base,
                addition,
                ..
            } => vec![*template, *base, *addition],
            mod_api::RecipeBookLayout::Mod => Vec::new(),
        };
        let mut stacks: Vec<_> = self
            .common
            .inventory
            .iter()
            .flatten()
            .filter(|stack| sunrise_sim::menu::place::usable_for_crafting(stack))
            .cloned()
            .collect();
        let base = self.native_screen_container_slot_base();
        for slot in slots {
            if let Some(stack) = self.slot_mut(base + usize::from(slot)).clone() {
                stacks.push(stack);
            }
        }
        stacks
            .into_iter()
            .map(|mut stack| {
                let amount = i32::try_from(
                    stack
                        .count
                        .min(sunrise_sim::menu::item_max_stack(self.items(), &stack)),
                )
                .unwrap_or(i32::MAX);
                stack.count = 1;
                (stack, amount)
            })
            .collect()
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;
