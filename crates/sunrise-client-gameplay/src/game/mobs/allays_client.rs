//! Client-side allay interaction prediction.
use super::level_sounds_common::LevelSound;
use mod_api::SoundSource;
use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;
use sunrise_items::items::ItemStack;
use sunrise_sim::first_person_sim::Hand;
use sunrise_sim::{lan::playerdata::uuid_ints, mobs::allay};

impl crate::game::ClientGame<'_> {
    fn client_acting_uuid(&self) -> [i32; 4] {
        self.client
            .remote
            .as_ref()
            .and_then(|r| r.player_uuid(r.entity_id as u32))
            .map(uuid_ints)
            .or(self.common.level.player_uuid)
            .unwrap_or_default()
    }

    /// The client's `Allay.mobInteract` runs before the server's response.
    /// Whether `predict_allay_interaction` would act, on a copy of the allay.
    pub(in crate::game::client_side) fn allay_would_take(
        &self,
        index: usize,
        hand: Hand,
        offered: Option<ItemStack>,
    ) -> bool {
        let duplicates = offered.as_ref().is_some_and(|s| {
            self.items()
                .item_tags
                .get("minecraft:duplicates_allays")
                .is_some_and(|tag| tag.contains(&*s.item))
        });
        allay::interaction::predict(
            &mut self.common.mobs[index].clone(),
            offered.as_ref(),
            hand == Hand::Main,
            duplicates,
            self.client_acting_uuid(),
        )
        .is_some()
    }

    pub(in crate::game::client_side) fn predict_allay_interaction(
        &mut self,
        index: usize,
        hand: Hand,
        offered: Option<ItemStack>,
    ) -> bool {
        use allay::interaction::{Interaction, predict};
        let duplicates = offered.as_ref().is_some_and(|s| {
            self.items()
                .item_tags
                .get("minecraft:duplicates_allays")
                .is_some_and(|tag| tag.contains(&*s.item))
        });
        let actor = self.client_acting_uuid();
        let Some(result) = predict(
            &mut self.common.mobs[index],
            offered.as_ref(),
            hand == Hand::Main,
            duplicates,
            actor,
        ) else {
            return false;
        };
        let mob = &self.common.mobs[index];
        if !mob.silent {
            let mut sound = LevelSound::new(
                result.sound(),
                [mob.position.x, mob.position.y, mob.position.z],
                2.0,
                1.0,
            )
            .in_source(SoundSource::ALL[6]);
            sound.emitter = Some(mob.id);
            let seed = self.client.audio.0.next_sound_seed();
            self.play_client_level_sound(sound, seed);
        }
        match result {
            Interaction::Duplicate | Interaction::Give => {
                self.with_hand(hand, Self::consume_held);
            }
            Interaction::Take(stack) => {
                let _ = self.add_stack_to_inventory(stack);
            }
        }
        true
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
