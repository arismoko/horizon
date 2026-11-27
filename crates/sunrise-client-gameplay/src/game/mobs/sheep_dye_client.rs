use super::level_sounds_common::LevelSound;
use crate::game::ClientGame;
use mod_api::SoundSource;
use sunrise_sim::first_person_sim::Hand;
use sunrise_sim::mobs::sheep_dye;

impl ClientGame<'_> {
    /// Whether `interact_sheep_dye` would act on a client, on a copy of the sheep.
    pub(in crate::game::client_side) fn sheep_would_take_dye(
        &mut self,
        index: usize,
        hand: Hand,
    ) -> bool {
        let Some(mut stack) = self.with_hand(hand, |g| g.held().cloned()) else {
            return false;
        };
        sheep_dye::interact(&mut self.common.mobs[index].clone(), &mut stack, true)
    }

    /// Client-side sheep dyeing prediction: plays local sound and leaves wool/dye to server sync.
    pub(in crate::game) fn client_interact_sheep_dye(&mut self, index: usize, hand: Hand) -> bool {
        let Some(mut stack) = self.with_hand(hand, |g| g.held().cloned()) else {
            return false;
        };
        if !sheep_dye::interact(&mut self.common.mobs[index], &mut stack, true) {
            return false;
        }
        let mob = &self.common.mobs[index];
        if !mob.silent {
            let at = [mob.position.x, mob.position.y, mob.position.z];
            self.play_client_local_sound(
                LevelSound::new("minecraft:item.dye.use", at, 1.0, 1.0)
                    .in_source(SoundSource::Players),
            );
        }
        true
    }
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
