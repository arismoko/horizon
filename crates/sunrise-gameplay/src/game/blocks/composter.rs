//! `ComposterBlock`: compostable items go in by hand (`useItemOn`) or
//! through its input container (hoppers, droppers, crafters above), and the
//! fill feedback of level event 1500 (`handleFill`). Emptying a full
//! composter by hand is `useWithoutItem` in sunrise-world.
use super::commands::{Target, compute::ComputeContext};
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::composter_data::{Effects, is_composter};
use sunrise_items::items::ItemStack;
use sunrise_sim::mobs::villager;
/// Temporarily lends the provider the composter's random owner. Even unwinding restores
/// Level.random before the transaction guard rolls the local owner back.
struct ScopedRandom<'a, 'w> {
    game: &'a mut crate::game::ServerGame<'w>,
    pos: [i32; 3],
    world_random: Option<sunrise_core::random::LegacyRandom>,
}

impl Drop for ScopedRandom<'_, '_> {
    fn drop(&mut self) {
        let mut random = self.world_random.take().unwrap();
        std::mem::swap(&mut *self.game.world.random, &mut random);
        self.game.put_composter_random(self.pos, random);
    }
}

impl crate::game::ServerGame<'_> {
    /// The composter's `level` at `pos`, None when it isn't a composter.
    pub fn composter_level(&self, pos: [i32; 3]) -> Option<i32> {
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        let s = self.common.pack.assets.registry.state(state)?;
        if !is_composter(&s.name) {
            return None;
        }
        s.properties.get("level")?.parse().ok()
    }
    /// The item's `compostable` layers provider.
    fn compostable(&self, stack: &ItemStack) -> Option<serde_json::Value> {
        self.items().get(&stack.item)?.compostable.clone()
    }
    /// The shared `ComposterBlock.addLayer` block-interaction loot context and level random.
    fn composter_layers(
        &mut self,
        pos: [i32; 3],
        provider: &serde_json::Value,
        interacting: Option<Target>,
    ) -> i32 {
        let context = ComputeContext {
            block: Some(pos),
            target: None,
            interacting,
            // BLOCK_INTERACT: no THIS_ENTITY, ORIGIN = Vec3.atCenterOf(pos).
            without_this: Some(pos.map(|c| f64::from(c) + 0.5)),
        };
        self.provider_int(provider, &context).unwrap_or(0)
    }

    /// ComposterWrapper's private transactional random, using the unchanged vanilla layers
    /// provider. Sunrise initializes the owner from the current level stream once; it never
    /// advances or rewinds the regular level stream while evaluating this capability.
    pub fn composter_transactional_layers(
        &mut self,
        pos: [i32; 3],
        provider: &serde_json::Value,
    ) -> i32 {
        let mut random = self.take_composter_random(pos);
        std::mem::swap(&mut *self.common.world.random, &mut random);
        let scope = ScopedRandom {
            game: self,
            pos,
            world_random: Some(random),
        };
        scope.game.composter_layers(pos, provider, None)
    }
    /// `ComposterBlock.addLayer` and level event 1500 for immediate vanilla callers.
    fn composter_add(
        &mut self,
        pos: [i32; 3],
        layers: &serde_json::Value,
        interacting: Option<Target>,
    ) -> bool {
        let layers = self.composter_layers(pos, layers, interacting);
        let before = self.common.world.block(pos[0], pos[1], pos[2]);
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let after = self.with_level(|b, level, ticks| b.composter_insert(level, ticks, at, layers));
        self.common
            .world
            .modified
            .insert((pos[0] >> 4, pos[2] >> 4));
        // The game event and level event 1500 happen here, before the caller goes on.
        let _ = self.server_process_level_events();
        after != before
    }
    /// `ComposterBlock.useItemOn` with the main-hand stack: true when it
    /// consumed the action (SUCCESS; a full composter falls through to
    /// `useWithoutItem`, which empties it).
    pub fn use_composter(&mut self, pos: [i32; 3], held: Option<&ItemStack>) -> bool {
        let Some(level) = self.composter_level(pos) else {
            return false;
        };
        let Some(stack) = held else {
            return false;
        };
        let Some(layers) = self.compostable(stack) else {
            return false;
        };
        if level >= 8 {
            return false;
        }
        if level < 7 {
            self.composter_add(pos, &layers, Some(Target::Player));
            self.native_compost_item(pos, &stack.item);
            self.award_stat("used", &stack.item, 1);
            self.consume_held();
        }
        // ServerPlayerGameMode.useItemOn: the action was consumed.
        self.trigger_block("minecraft:item_used_on_block", pos, stack);
        true
    }
    /// `HopperBlockEntity.addItem` into the composter's `InputContainer`
    /// (levels 0 to 6; one item, compostable, from above): the remainder.
    pub(in crate::game) fn composter_container_insert(
        &mut self,
        pos: [i32; 3],
        mut stack: ItemStack,
        from_above: bool,
    ) -> Option<ItemStack> {
        let Some(level) = self.composter_level(pos) else {
            return Some(stack);
        };
        if !from_above || level >= 7 {
            return Some(stack);
        }
        let Some(layers) = self.compostable(&stack) else {
            return Some(stack);
        };
        // InputContainer.getMaxStackSize is 1: one item goes in, and its
        // setChanged composts it at once.
        stack.count -= 1;
        self.composter_add(pos, &layers, None);
        (stack.count > 0).then_some(stack)
    }

    /// `InputContainer.setChanged` through an ITEMS port. Slot acceptance and the one-item
    /// limit are the shared container rules; only the state write and feedback are deferred.
    pub(in crate::game::server_side) fn composter_capability_insert(
        &mut self,
        pos: [i32; 3],
        stack: &ItemStack,
    ) -> bool {
        if self.composter_level(pos).is_none_or(|level| level >= 7) {
            return false;
        }
        let Some(provider) = self.compostable(stack) else {
            return false;
        };
        let before = self.common.world.block(pos[0], pos[1], pos[2]);
        let layers = self.composter_transactional_layers(pos, &provider);
        let after = self
            .common
            .res
            .behaviours()
            .composter_state_after_layers(before, layers);
        if after != before && !self.journal_set_block(pos, after) {
            return false;
        }
        self.after_commit(
            sunrise_gameplay_common::game::transactions::After::Composter(Effects::Insert {
                pos,
                before,
                after,
            }),
        );
        true
    }

    /// `OutputContainer.setChanged` through an ITEMS port: no item entity or hand sound.
    pub(in crate::game::server_side) fn composter_capability_extract(
        &mut self,
        pos: [i32; 3],
        max: u32,
    ) -> Option<ItemStack> {
        if max == 0 || self.composter_level(pos) != Some(8) {
            return None;
        }
        let before = self.common.world.block(pos[0], pos[1], pos[2]);
        let after = self
            .common
            .res
            .behaviours()
            .composter_state_after_extract(before);
        if !self.journal_set_block(pos, after) {
            return None;
        }
        self.after_commit(
            sunrise_gameplay_common::game::transactions::After::Composter(Effects::Empty {
                pos,
                after,
            }),
        );
        Some(ItemStack::new("minecraft:bone_meal", 1))
    }

    pub(in crate::game) fn commit_composter_effects(&mut self, effects: Effects) {
        self.with_level(|b, level, ticks| match effects {
            Effects::Insert { pos, before, after } => {
                b.composter_insert_effects(
                    level,
                    ticks,
                    BlockPos::new(pos[0], pos[1], pos[2]),
                    before,
                    after,
                );
            }
            Effects::Empty { pos, after } => {
                b.composter_empty_effects(
                    level,
                    ticks,
                    BlockPos::new(pos[0], pos[1], pos[2]),
                    after,
                );
            }
        });
        let _ = self.server_process_level_events();
    }
    /// `WorkAtComposter.compostItems` for the villager at `mob`: a full
    /// composter is emptied, then up to 20 wheat or beetroot seeds go in
    /// (keeping 10 of each kind) until the composter reaches level 7. Level
    /// event 1500 plays once, at the end.
    pub(in crate::game::server_side) fn villager_compost_items(
        &mut self,
        mob: usize,
        pos: [i32; 3],
    ) {
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let Some(mut before) = self.composter_level(pos) else {
            return;
        };
        if before == 8 {
            self.with_level(|b, level, ticks| b.composter_extract(level, ticks, at));
            let _ = self.server_process_level_events();
            before = 0;
        }
        let mut remaining = 20;
        let mut amounts: Vec<(std::sync::Arc<str>, i32)> = Vec::new();
        // Pin one world generation for the whole work batch, including callbacks.
        let maps = std::sync::Arc::clone(self.mob_map_snapshot());
        let slots = villager::ext(&self.common.mobs[mob]).map_or(0, |e| e.inventory.len());
        'slots: for slot in (0..slots).rev() {
            if remaining <= 0 {
                break;
            }
            let stack = villager::ext_mut(&mut self.common.mobs[mob]).inventory[slot].clone();
            if !maps.villager_compostable(&stack.item) {
                continue;
            }
            let known = amounts.iter().position(|(item, _)| *item == stack.item);
            let total = known.map_or(0, |k| amounts[k].1) + stack.count as i32;
            match known {
                Some(k) => amounts[k].1 = total,
                None => amounts.push((stack.item.clone(), total)),
            }
            let uses = (total - 10).min(remaining).min(stack.count as i32);
            if uses <= 0 {
                continue;
            }
            remaining -= uses;
            for _ in 0..uses {
                self.villager_insert_item(mob, slot, &stack, pos);
                if self.composter_level(pos) == Some(7) {
                    break 'slots;
                }
            }
        }
        let after = self.composter_level(pos).unwrap_or(before);
        self.with_level(|b, level, ticks| {
            b.composter_fill_effects(level, ticks, at, after != before)
        });
        let _ = self.server_process_level_events();
    }
    /// `ComposterBlock.insertItem` with a villager's stack: below level 7 a
    /// compostable item is used up and may add a layer.
    fn villager_insert_item(&mut self, mob: usize, slot: usize, stack: &ItemStack, pos: [i32; 3]) {
        if self.composter_level(pos).is_none_or(|level| level >= 7) {
            return;
        }
        let Some(layers) = self.compostable(stack) else {
            return;
        };
        let interacting = Some(Target::Mob(self.common.mobs[mob].id));
        let layers = self.composter_layers(pos, &layers, interacting);
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| b.composter_layer(level, ticks, at, layers));
        self.common
            .world
            .modified
            .insert((pos[0] >> 4, pos[2] >> 4));
        let _ = self.server_process_level_events();
        let inventory = &mut villager::ext_mut(&mut self.common.mobs[mob]).inventory;
        inventory[slot].count -= 1;
        if inventory[slot].count == 0 {
            inventory.remove(slot);
        }
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;
