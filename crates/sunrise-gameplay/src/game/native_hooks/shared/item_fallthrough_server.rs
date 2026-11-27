use super::*;

impl crate::game::ServerGame<'_> {
    pub(crate) fn use_item_fallthrough(
        &mut self,
        stack: &ItemStack,
        on_block: bool,
    ) -> Option<bool> {
        if on_block {
            return None;
        }
        if let Some(answer) = self.right_click_item(self.common.hand, stack) {
            if answer == mod_api::InteractionResult::SuccessServer {
                self.swing_arm();
            }
            return Some(answer.consumes_action());
        }
        self.use_item_in_hand(stack)
    }
    /// Finish all useOn handlers, including placement, before entering Item.use.
    pub(crate) fn use_target_then_item(
        &mut self,
        hit: &sunrise_gameplay_common::game::Hit,
    ) -> sunrise_game_data::Result<bool> {
        self.server.guest_use = Some(sunrise_gameplay_common::game::GuestUse::on_block(
            self.common.hand,
        ));
        let result = self.with_block_use_original(self.common.hand, |g| {
            let result = g.use_hand(Some(hit));
            if matches!(result, Ok(true)) && !g.server.use_failed {
                g.finish_block_use(hit.pos, g.hand);
            }
            result
        });
        self.server.guest_use = None;
        if result? {
            return Ok(true);
        }
        self.use_hand(None)
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::native_hooks::item_fallthrough::ItemFallthroughGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::use_item_hand::UseItemHandGameExt1 as _;
