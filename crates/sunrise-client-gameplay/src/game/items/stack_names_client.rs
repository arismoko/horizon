use crate::game::ClientGame;
use sunrise_items::items::ItemStack;

impl ClientGame<'_> {
    /// `ItemStack.getStyledHoverName`, without the rarity colour applied by the tooltip.
    pub fn styled_stack_name(&self, stack: &ItemStack) -> sunrise_game_data::text::Text {
        let name = self.common.stack_hover_name_text(stack);
        if stack.data().is_some_and(|data| data.custom_name.is_some()) {
            sunrise_game_data::text::Text::empty().append(name).italic()
        } else {
            name
        }
    }
}
