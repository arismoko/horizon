use super::Target;
use crate::game::commands::give::GiveItem;
use sunrise_items::items::ItemStack;

impl crate::game::ServerGame<'_> {
    /// `GiveCommand.giveItem`'s handing over of one stack to the acting player:
    /// `Inventory.add`, then what did not fit thrown from the eyes with no pickup delay and
    /// `owner` (the player's UUID) its only taker, or the pickup sound when it all went in.
    /// Effects go to the player's level, even if the turn borrows them into another one.
    /// If their level is unavailable, return what did not fit rather than drop it elsewhere.
    pub(in crate::game) fn give_acting_stack(
        &mut self,
        mut stack: ItemStack,
        owner: Option<[i32; 4]>,
    ) -> Option<ItemStack> {
        let uuid = self.native_acting_id();
        self.journal_player(uuid);
        let left = self.insert_stack_into_inventory(&mut stack);
        // A borrowed player's body is in place but their world may be parked. The shared
        // give effects put that world in place only while replaying the sound or drop.
        let dim = self.target_level(Target::Player);
        if dim != self.common.dim && self.server.levels.0.parked_world(dim).is_none() {
            return left.then_some(stack);
        }
        if left {
            let eye = self.give_eye();
            self.after_commit(
                sunrise_gameplay_common::game::transactions::After::GiveRest {
                    uuid,
                    rest: stack,
                    owner,
                    dim,
                    eye,
                },
            );
        } else {
            self.after_commit(
                sunrise_gameplay_common::game::transactions::After::GivePickup {
                    uuid,
                    dim,
                    prototype: ItemStack { count: 1, ..stack },
                },
            );
        }
        None
    }
    /// `GiveCommand.giveItem` configures the drop before `addFreshEntity`, without `Player.drop`.
    pub(in crate::game::server_side) fn join_give_item(
        &mut self,
        stack: ItemStack,
        eye: [f64; 3],
        velocity: [f64; 3],
        kind: GiveItem,
    ) {
        let at = [eye[0], eye[1] - f64::from(0.3f32), eye[2]];
        let mut random = sunrise_gameplay_common::game::spill_common::prepare_item_random(None);
        let delay = match kind {
            GiveItem::Overflow(owner) => {
                random.state.target = owner.map(sunrise_sim::lan::playerdata::ints_uuid);
                0
            }
            // ItemEntity.makeFakeItem: never pick up, one tick left before expiry.
            GiveItem::Fake => {
                random.state.age = 5999;
                32767
            }
        };
        self.push_item_entity(stack, at, Some(velocity), delay, Some(random));
        self.join_item_entity(self.server.next_entity, at);
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;
