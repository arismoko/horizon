//! Decorated pots, which Sunrise keeps as loaded (the chunk's saved block
//! entities): the loot table a trial chamber pot holds, rolled into its item
//! the first time the item is read (`RandomizableContainer.unpackLootTable`),
//! and `DecoratedPotBlock.useItemOn` / `useWithoutItem`.
use crate::game::spill_common::saved_entity;
use sunrise_core::random::LegacyRandom;
use sunrise_items::random_sequences::RandomSequences;
use sunrise_sim::save;
use {
    sunrise_core::nbt::Tag,
    sunrise_world::loot::{ContainerLootContext, LootData},
};

pub(in crate::game) const DECORATED_POT: &str = "minecraft:decorated_pot";

/// `DecoratedPotBlockEntity.WobbleStyle` ordinals.
pub const WOBBLE_POSITIVE: i32 = 0;
pub const WOBBLE_NEGATIVE: i32 = 1;

/// `RandomizableContainer.unpackLootTable(null)` on a pot's saved tag: the
/// loot table, if any, is removed and `LootTable.fill` puts its roll in
/// `item`. The roll draws from `LootTableSeed`, else the table's random
/// sequence, else the level random (`LootContext.Builder.create`).
pub fn unpack_loot(
    tag: &mut Tag,
    loot: Option<&LootData>,
    random: &mut LegacyRandom,
    sequences: &mut RandomSequences,
    world_seed: i64,
    items: &sunrise_items::items::Items,
    context: ContainerLootContext<'_>,
) {
    let Tag::Compound(map) = tag else {
        return;
    };
    let Some(Tag::String(table)) = map.remove(&"LootTable".into()) else {
        return;
    };
    let seed = match map.remove(&"LootTableSeed".into()) {
        Some(Tag::Long(seed)) => seed,
        _ => 0,
    };
    map.remove(&"item".into());
    let Some(loot) = loot else {
        return;
    };
    let table = table.to_string_lossy();
    // No origin biome: the pot tables have no location checks.
    let slots = if seed != 0 {
        loot.fill_in(&table, &mut LegacyRandom::new(seed), 1, context)
    } else if let Some(sequence) = loot.random_sequence(&table) {
        loot.fill_in(&table, sequences.get(sequence, world_seed), 1, context)
    } else {
        loot.fill_in(&table, random, 1, context)
    };
    if let Some(stack) = slots.into_iter().next().flatten() {
        let item = sunrise_sim::structure_blocks::loot_item_with_components(&stack);
        map.insert("item".into(), save::item_to_nbt(&item, items));
    }
}

/// [`Game::pot_data`] over the world and the block facts.
pub(in crate::game) fn pot_data_in(
    world: &sunrise_level::world::World,
    facts: &sunrise_items::blocks::BlockFacts,
    pos: [i32; 3],
) -> Option<Tag> {
    let state = world.block(pos[0], pos[1], pos[2]);
    if &*facts.get(state).name != "minecraft:decorated_pot" {
        return None;
    }
    let mut tag = saved_entity(world, pos).unwrap_or_else(|| {
        Tag::compound([
            ("id", Tag::String("minecraft:decorated_pot".into())),
            ("x", Tag::Int(pos[0])),
            ("y", Tag::Int(pos[1])),
            ("z", Tag::Int(pos[2])),
        ])
    });
    tag = sunrise_game_data::block_entity_nbt::canonical(&tag)?;
    if let Tag::Compound(map) = &mut tag {
        map.remove(&"keepPacked".into());
    }
    Some(tag)
}

impl crate::game::CommonGame<'_> {
    /// `DecoratedPotBlockEntity.saveAdditional` for commands: a pure view of
    /// the existing saved state. Unlike pot_tag, this never unpacks loot.
    pub fn pot_data(&self, pos: [i32; 3]) -> Option<Tag> {
        pot_data_in(&self.world, self.res.facts(), pos)
    }
}
