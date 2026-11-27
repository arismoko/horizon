use super::*;

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Sunrise does.
impl crate::game::CommonGame<'_> {
    /// `ItemStack.getHoverName()` as a component: each viewer renders it in their language.
    pub fn hover_name(&self, stack: &ItemStack) -> sunrise_game_data::text::Text {
        self.stack_hover_name_text(stack)
    }
}

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Sunrise does.
pub trait EnchantmentsGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// Registered enchantments, in registry order.
    fn enchantments(&self) -> Vec<EnchantmentInfo> {
        let enchantments = self.item_enchantments();
        enchantments
            .list
            .iter()
            .map(|e| {
                mod_api::EnchantmentInfo::new(
                    e.id.to_string(),
                    e.max_level,
                    e.weight,
                    e.anvil_cost,
                    enchantments.is_curse(&e.id),
                    crate::game::api_values::TryIntoApi::try_into_api(&e.description_text)
                        .unwrap_or_else(|_| mod_api::Text::literal(e.description.clone())),
                )
            })
            .collect()
    }
    /// `Enchantment.getMinCost(level)`: the enchanting power needed, or None for an unknown id.
    fn enchantment_min_cost(&self, id: &str, level: u32) -> Option<i32> {
        self.item_enchantments()
            .get(id)
            .map(|e| e.min_cost.calculate(level))
    }
    /// Whether enchantment `id` can go on `item` (`Enchantment.isSupportedItem`).
    fn enchantment_supports(&self, id: &str, item: mod_api::ItemKey) -> bool {
        let Some(item) = mod_api::__private::item_key_id(item) else {
            return false;
        };
        self.item_enchantments().get(id).is_some_and(|e| {
            sunrise_items::enchant::Enchantments::item_in(self.items(), &e.supported, item)
        })
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> EnchantmentsGameExt1<'__sg> for G {}

use crate::game::tables::GameItems as _;
