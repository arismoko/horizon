//! Stack-aware remainder decisions made before borrowing the brewing stand for consumption.
use super::*;
use sunrise_items::items::Items;

pub(in crate::game) struct BrewingRemainder(Option<Option<ItemStack>>);

impl BrewingRemainder {
    pub(in crate::game::server_side) fn resolve(
        &self,
        items: &Items,
        stack: &ItemStack,
    ) -> Option<ItemStack> {
        self.0.clone().unwrap_or_else(|| {
            sunrise_sim::crafting::CraftingRemainder::crafting_remainder(items, stack)
        })
    }
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn brewing_fuel_remainder(
        &self,
        pos: (i32, i32, i32),
    ) -> BrewingRemainder {
        let answer = match self.common.world.block_entities.get(&pos) {
            Some(BlockEntity::Brewing(b)) => b.items[4]
                .as_ref()
                .filter(|s| {
                    b.fuel <= 0
                        && mod_api::__private::item_listens(
                            mod_api::HookId::CraftingRemainder,
                            &s.item,
                        )
                        && sunrise_level::brewing::brewing_fuel(world_items!(self), s).is_some()
                })
                .map(|s| self.crafting_remainder(s)),
            _ => None,
        };
        BrewingRemainder(answer)
    }
}

use sunrise_gameplay_common::game::native_block_states::NativeBlockStatesGameExt1 as _;
