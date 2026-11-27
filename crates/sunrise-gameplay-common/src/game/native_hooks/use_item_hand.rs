use super::*;

pub trait UseItemHandGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// Apply a consuming result's swing on its owning logical side.
    fn interaction_consumed(&mut self, answer: mod_api::InteractionResult) -> bool {
        use mod_api::InteractionResult as Answer;
        if (answer == Answer::Success && !self.is_logical_server())
            || (answer == Answer::SuccessServer && self.is_logical_server())
        {
            self.swing_arm();
        }
        answer.consumes_action()
    }

    /// `ItemStack.useOn` after the block and before the item's implementation.
    fn item_on_interaction(
        &mut self,
        hit: &crate::game::Hit,
        hand: Hand,
        stack: &ItemStack,
    ) -> Option<mod_api::InteractionResult> {
        if !mod_api::__private::item_listens(mod_api::HookId::UseOn, &stack.item) {
            return None;
        }
        let snapshot = stack.try_into_api().ok()?;
        let (player, player_entity_id) = self.native_clicker();
        let mut result = None;
        self.native_use_on(mod_api::__private::UseOnCall {
            stack: &snapshot,
            pos: hit.pos,
            face: hit.face,
            hand,
            hit: hit.native(),
            player,
            player_entity_id,
            result: &mut result,
        });
        result.filter(|answer| {
            !matches!(
                answer,
                mod_api::InteractionResult::Pass | mod_api::InteractionResult::TryWithEmptyHand
            )
        })
    }

    /// Dispatch the item Part and apply the result on this logical side.
    fn use_item_in_hand(&mut self, stack: &ItemStack) -> Option<bool> {
        use mod_api::InteractionResult as Result;
        let mut result = None;
        if mod_api::__private::has_part_hook(mod_api::HookId::UseItem)
            && let Ok(snapshot) = stack.try_into_api()
        {
            let (player, player_entity_id) = self.native_clicker();
            self.native_use_item(mod_api::__private::UseItemCall {
                stack: &snapshot,
                hand: self.hand,
                player,
                player_entity_id,
                result: &mut result,
            });
        }
        match result? {
            Result::Pass | Result::TryWithEmptyHand => None,
            Result::Success => {
                if !self.is_logical_server() {
                    self.swing_arm();
                }
                Some(true)
            }
            Result::SuccessServer => {
                if self.is_logical_server() {
                    self.swing_arm();
                }
                Some(true)
            }
            Result::Consume => Some(true),
            Result::Fail => Some(false),
            _ => None,
        }
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> UseItemHandGameExt1<'__sg> for G {}

use crate::game::native_hooks::base::GameHooks as _;

use crate::game::side_ops::presentation::PresentationSideGameExt as _;
