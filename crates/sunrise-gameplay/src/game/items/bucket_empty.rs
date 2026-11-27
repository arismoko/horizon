//! Emptying a filled bucket from a player's hand: the `empty_contents` Part's say, the guest's
//! sounds and the client's prediction.
use super::Hit;
use sunrise_core::{geometry::Direction, position::BlockPos};
use sunrise_items::items::ItemStack;
use sunrise_sim::first_person_sim::UseResult;

impl crate::game::ServerGame<'_> {
    /// `BucketItem.use`'s `emptyContents` for the acting player's `stack`,
    /// poured at `target` from the clicked `hit`: the use's result when the
    /// item's `empty_contents` Part answered, `None` for vanilla's pour. The
    /// Part is asked in a transaction with the use's apply: one that changed
    /// the hand it was asked about is refused and undone, never re-decided.
    pub fn player_empty_contents(
        &mut self,
        stack: &ItemStack,
        target: [i32; 3],
        hit: &Hit,
    ) -> Option<UseResult> {
        let face = match hit.face {
            [0, 1, 0] => Direction::Up,
            [0, -1, 0] => Direction::Down,
            [0, 0, -1] => Direction::North,
            [0, 0, 1] => Direction::South,
            [-1, 0, 0] => Direction::West,
            _ => Direction::East,
        };
        let clicked = BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]);
        let pos = BlockPos::new(target[0], target[1], target[2]);
        if !self.empty_contents_listens(&stack.item) {
            return None;
        }
        // The decision's preconditions: the hand's slot and its whole stack.
        let slot = self.common.held_slot();
        let held = self.common.stack_in(slot).cloned();
        let used = self.storage_transaction(|game, _| {
            match game.native_empty_contents_event(pos, Some((clicked, face)), stack) {
                None => return Ok(None),
                Some(false) => return Ok(Some(UseResult::Fail)),
                Some(true) => {}
            }
            if game.held_slot() != slot || game.stack_in(slot) != held.as_ref() {
                return Err(());
            }
            game.trigger_block("minecraft:placed_block", target, stack);
            // `BucketItem.getEmptySuccessItem`: the empty bucket, kept full in creative.
            if !game.creative {
                *game.held_mut() = Some(ItemStack::new("minecraft:bucket", 1));
            }
            Ok(Some(UseResult::Success { swing: true }))
        });
        used.unwrap_or(Some(UseResult::Fail))
    }

    /// `BucketItem.emptyContents` plays `playSound(player, …)`: a guest's own client already
    /// played the empty or evaporate `sound` from its prediction (#4684), so those go to the
    /// guest's own list instead of being sent back to them.
    pub(in crate::game::server_side) fn keep_guest_bucket_sounds(&mut self, sound: &str) {
        if self.server.lan_acting.is_none() {
            return;
        }
        if !self.server.audio_capture_active {
            return;
        }
        let want = sound.trim_start_matches("minecraft:");
        let mut keep = Vec::with_capacity(self.server.pending_sounds.len());
        for entry in std::mem::take(&mut self.server.pending_sounds) {
            let name = entry.event.trim_start_matches("minecraft:");
            if name == want || name == "block.fire.extinguish" {
                self.server.guest_player_sounds.push((
                    entry.event,
                    entry.position,
                    entry.volume,
                    entry.pitch,
                ));
            } else {
                keep.push(entry);
            }
        }
        self.server.pending_sounds = keep;
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
