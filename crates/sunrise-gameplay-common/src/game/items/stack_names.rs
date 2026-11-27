//! Common item-stack display name and rarity colour helpers.
use sunrise_items::items::ItemStack;

impl crate::game::CommonGame<'_> {
    /// `ItemStack.getHoverName` as a component: custom name, book title, mod part, else vanilla.
    pub fn stack_hover_name_text(&self, stack: &ItemStack) -> sunrise_game_data::text::Text {
        sunrise_sim::stations::hover_name_in(&self.loop_context, &self.pack.names, stack)
    }

    /// `ItemStack.getItemName` as a component, without custom name or book title.
    pub fn stack_item_name_text(&self, stack: &ItemStack) -> sunrise_game_data::text::Text {
        sunrise_sim::stations::item_name_in(&self.loop_context, &self.pack.names, stack)
    }

    /// The resolved `ItemStack.getHoverName` rendered as plain text.
    pub fn stack_hover_name(&self, stack: &ItemStack) -> String {
        self.stack_hover_name_text(stack).plain()
    }
}

pub trait StackNamesGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// `ItemStack.getRarity().color()` in text's RGB format.
    fn stack_rarity_color(&self, stack: &ItemStack) -> u32 {
        use sunrise_game_data::text;
        let props = self.stations().enchantments.props(&stack.item);
        match sunrise_sim::stations::rarity(stack, props.map_or("common", |p| p.rarity.as_str())) {
            "uncommon" => text::YELLOW,
            "rare" => text::AQUA,
            "epic" => text::LIGHT_PURPLE,
            _ => text::WHITE,
        }
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> StackNamesGameExt1<'__sg> for G {}

use crate::game::tables::GameItems as _;
