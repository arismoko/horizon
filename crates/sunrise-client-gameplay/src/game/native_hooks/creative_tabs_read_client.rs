//! Client item-model lighting queries.
use super::*;
use sunrise_client::cit::Hand as CitHand;
use sunrise_gameplay_common::game::api_values::engine_display_context;

impl crate::game::ClientGame<'_> {
    /// `ItemStackRenderState.usesBlockLight` of `stack` resolved for `context`.
    pub fn item_uses_block_light(
        &self,
        stack: &ItemStack,
        context: mod_api::ItemDisplayContext,
    ) -> bool {
        let Some(display) = engine_display_context(context) else {
            return false;
        };
        self.resolve_item_in_level(stack, display, None, None, 0, CitHand::Main)
            .uses_block_light()
    }
}
