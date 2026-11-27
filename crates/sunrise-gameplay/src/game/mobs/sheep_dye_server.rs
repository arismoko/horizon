//! Server-side sheep dyeing (`DyeItem.interactLivingEntity`).
use crate::game::ServerGame;
use {sunrise_sim::first_person_sim::Hand, sunrise_sim::mobs::sheep_dye};

impl ServerGame<'_> {
    /// `DyeItem.interactLivingEntity` on the server: applies wool dye, broadcasts the
    /// sound excluding the actor, and consumes the dye in survival.
    pub(in crate::game) fn server_interact_sheep_dye(&mut self, index: usize, hand: Hand) -> bool {
        let Some(mut stack) = self.with_hand(hand, |g| g.held().cloned()) else {
            return false;
        };
        if !sheep_dye::interact(&mut self.common.mobs[index], &mut stack, false) {
            return false;
        }
        self.play_mob_bound_sound(index, "minecraft:item.dye.use", 7, 1.0, 1.0, true);
        if !self.common.creative {
            self.with_hand(hand, |g| g.consume_player_slot(g.held_slot(), 1));
        }
        true
    }
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;
