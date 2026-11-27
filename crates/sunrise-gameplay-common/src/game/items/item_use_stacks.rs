//! Report destruction only where an item action actually consumes or breaks a stack.
use super::*;
use crate::game::native_hooks::player_destroy_item::GameHooks as _;

impl crate::game::CommonGame<'_> {
    /// A slot's hand, or no hand for armour and inventory ammunition.
    pub fn item_slot_hand(&self, slot: usize) -> Option<Hand> {
        if slot == held_items::OFFHAND_SLOT {
            Some(Hand::Off)
        } else if slot == self.selected {
            Some(Hand::Main)
        } else {
            None
        }
    }
}

pub trait ItemUseStacksGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// Report an applied destruction, not a hand edit made by a listener.
    fn player_item_destroyed(&mut self, original: &ItemStack, hand: Option<Hand>) {
        if original.count == 0
            || &*original.item == "minecraft:air"
            || !mod_api::__private::listened(mod_api::HookId::PlayerDestroyItem)
        {
            return;
        }
        let (player, player_entity_id) = self.native_clicker();
        self.native_player_destroy_item(player, player_entity_id, original, hand);
    }

    /// Consume from a player slot after the caller's creative-mode gate.
    /// Notify before a use remainder replaces the now-empty stack.
    fn consume_player_slot(&mut self, slot: usize, amount: u32) {
        let original = mod_api::__private::listened(mod_api::HookId::PlayerDestroyItem)
            .then(|| self.stack_in(slot).cloned())
            .flatten();
        let hand = self.item_slot_hand(slot);
        let edit = |cell: &mut Option<ItemStack>| {
            if let Some(stack) = cell.as_mut() {
                stack.count = stack.count.saturating_sub(amount);
                if stack.count == 0 {
                    *cell = None;
                }
            }
        };
        if slot == self.held_slot() {
            self.side_consume_held_slot(amount);
        } else {
            edit(self.stack_in_mut(slot));
        }
        if self.stack_in(slot).is_none_or(|stack| stack.count == 0)
            && let Some(original) = original
        {
            self.player_item_destroyed(&original, hand);
        }
    }

    fn consume_held(&mut self) {
        if !self.creative {
            self.consume_player_slot(self.held_slot(), 1);
        }
    }

    /// `BlockItem.place`'s shrink of the stack it placed, and
    /// `SolidBucketItem.useOn`'s `BucketItem.getEmptySuccessItem`: a placed
    /// powder snow bucket leaves an empty bucket in that hand, unless the player
    /// has infinite materials. The client predicts the same as the server does.
    fn consume_placed(&mut self, placed: &ItemStack) {
        if self.creative {
            return;
        }
        let slot = self.held_slot();
        let last = self.stack_in(slot).is_some_and(|stack| stack.count == 1);
        self.consume_player_slot(slot, 1);
        if &*placed.item == "minecraft:powder_snow_bucket"
            && (!last || self.stack_in(slot).is_none())
        {
            *self.stack_in_mut(slot) = Some(ItemStack::new("minecraft:bucket", 1));
        }
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> ItemUseStacksGameExt1<'__sg> for G {}

use crate::game::side_ops::items::ItemsSideGameExt as _;

use crate::game::native_hooks::ModGameExt1 as _;
