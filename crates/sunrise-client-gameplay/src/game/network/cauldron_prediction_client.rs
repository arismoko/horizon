//! Client-side vanilla cauldron prediction.
use super::Hit;
use std::sync::Arc;
use sunrise_client::actions::Action as Mapping;
use sunrise_gameplay_common::game::remote::DryUse;
use sunrise_items::items::ItemStack;
use sunrise_sim::cauldron::{
    self, Outcome,
    interactions::{Dispatcher, Entry},
};
use sunrise_world::level::InteractionResult;

struct DryHand<'a> {
    creative: bool,
    hand: Option<ItemStack>,
    inventory: &'a [Option<ItemStack>],
}
impl cauldron::Player for DryHand<'_> {
    fn creative(&self) -> bool {
        self.creative
    }
    fn hand(&mut self) -> &mut Option<ItemStack> {
        &mut self.hand
    }
    fn add(&mut self, _: &mut ItemStack) -> bool {
        true
    }
    fn contains(&self, stack: &ItemStack) -> bool {
        self.inventory
            .iter()
            .flatten()
            .any(|have| have.same_item_same_components(stack))
    }
    fn drop_stack(&mut self, _: ItemStack) {}
}

impl crate::game::ClientGame<'_> {
    /// The server's registration snapshot, not the client's installed mods,
    /// determines whether the click needs an authoritative block result.
    pub(in crate::game) fn has_server_cauldron_handler(
        &self,
        pos: [i32; 3],
        held: Option<&ItemStack>,
    ) -> bool {
        let (Some(table), Some(held)) = (
            self.client
                .remote
                .as_ref()
                .and_then(|r| r.cauldron_entries.as_ref()),
            held,
        ) else {
            return false;
        };
        let block = self.block_name(pos);
        let items = self.items();
        matches!(
            table.select(
                &block,
                &held.item,
                cauldron::removes_dye(items, &held.item),
                |tag| { items.item_in_tag(tag, &held.item) }
            ),
            Some(Entry::Mod(()))
        )
    }

    /// Predict vanilla cauldron clicks without invoking server-registered handlers.
    pub fn client_cauldron_prediction(
        &self,
        pos: [i32; 3],
        held: Option<&ItemStack>,
    ) -> (Outcome, DryUse) {
        let pass = || (Outcome::Pass, DryUse::default());
        let Some(held) = held else {
            return pass();
        };
        if self.held_mapping(Mapping::Sneak) {
            return pass();
        }
        let Some((state, underwater)) = self.common.cauldron_at(pos) else {
            return pass();
        };
        let table = Dispatcher::vanilla();
        let removes_dye = cauldron::removes_dye(self.items(), &held.item);
        let items = self.items();
        let entry = table.select(&state.name, &held.item, removes_dye, |tag| {
            items.item_in_tag(tag, &held.item)
        });
        let mut hand = DryHand {
            creative: self.common.creative,
            hand: Some(held.clone()),
            inventory: &self.common.inventory,
        };
        let result = cauldron::interact_selected(&state, underwater, entry.as_ref(), &mut hand);
        let at = sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
        let changes = result
            .block
            .and_then(|block| self.common.pack.assets.registry.resolve(&block).ok())
            .map(|block| (at, block))
            .into_iter()
            .collect();
        let sounds = result
            .sound
            .map(|sound| (at, Arc::from(sound), mod_api::SoundSource::Blocks, 1.0, 1.0))
            .into_iter()
            .collect();
        (result.outcome, (changes, sounds))
    }

    /// Visual cauldron prediction matching the item-use consumption guard.
    pub fn remote_cauldron_reacts(
        &self,
        hit: &Hit,
        held: Option<&ItemStack>,
    ) -> Option<(InteractionResult, DryUse)> {
        let (outcome, changes) = self.client_cauldron_prediction(hit.pos, held);
        let answer = match outcome {
            Outcome::Pass => return None,
            Outcome::Consume => InteractionResult::Consume,
            Outcome::Success => InteractionResult::Success,
        };
        Some((answer, changes))
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;
