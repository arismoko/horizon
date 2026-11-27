use super::*;
use sunrise_items::items::BlockLootRoll;

impl GameLevel<'_> {
    /// A block's loot table reads an enchantment's level
    /// (`ApplyBonusCount.run`, `BonusLevelTableCondition.test`): NeoForge's
    /// `EnchantedBlockLootEvent`. Change `level` to change what that read
    /// uses. Server-side, authoritative levels only.
    #[inline(always)]
    pub(in crate::game) fn native_enchanted_block_loot(
        &self,
        call: super::fire::EnchantedBlockLootCall<'_>,
    ) {
        self.fire_enchanted_block_loot(call);
    }
    /// `Block.getDrops` of `state` at `pos` for `entity`, from its table's
    /// random sequence, else the level random, the mods having their say on
    /// the levels the table reads. The roll decides each read, the level
    /// fires the hook and the roll goes on with the answer; so the listeners
    /// can hold this level, the roll keeps the randoms until it ends.
    pub fn roll_block_loot(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        entity: Option<EntityRef>,
        roll: BlockLootRoll<'_>,
        source: Option<&sunrise_world::loot::copy::BlockSource>,
    ) -> Vec<ItemStack> {
        let at = [pos.x, pos.y, pos.z];
        let captured = source
            .is_none()
            .then(|| {
                self.pending.source(at).or_else(|| {
                    crate::game::spill_common::world_source(
                        self.world,
                        at,
                        &self.facts.get(state).name,
                        self.items,
                        |key, entity, block| self.encode_live_block_entity(key, entity, block),
                    )
                })
            })
            .flatten();
        sunrise_world::loot::copy::with_storage(self.loot_storage.clone(), || {
            sunrise_world::loot::copy::with_block_source(source.or(captured.as_deref()), || {
                let (items, registry, seed) = (self.items, self.registry, self.seed);
                // ORIGIN is the block's centre and its location checks read this level,
                // so the roll keeps the randoms until it ends.
                let mut random = self.world.random.clone();
                let mut sequences = std::mem::take(&mut *self.rand_sequences);
                let view = self.location_view();
                let read = |predicate: &serde_json::Value, at: [f64; 3]| {
                    sunrise_sim::advancements::location(predicate, at, &view)
                };
                let roll = BlockLootRoll {
                    location: Some(sunrise_sim::advancements::loot_location(
                        at.map(|c| f64::from(c) + 0.5),
                        &read,
                    )),
                    ..roll
                };
                if !self.native_authoritative
                    || !mod_api::__private::listened(mod_api::HookId::EnchantedBlockLoot)
                {
                    let drops = items.block_drops(
                        registry,
                        state,
                        roll,
                        &mut random,
                        Some((&mut sequences, seed)),
                    );
                    self.world.random = random;
                    *self.rand_sequences = sequences;
                    return drops;
                }
                let tool = roll.held;
                let level = &*self;
                let levels = |enchantment: &str, mut read: u32| {
                    if let Some(enchantment) =
                        sunrise_sim::enchantment_levels::identifier(enchantment)
                    {
                        level.native_enchanted_block_loot(super::fire::EnchantedBlockLootCall {
                            dimension: level.dimension,
                            pos,
                            state,
                            tool,
                            enchantment: &enchantment,
                            level: &mut read,
                            entity,
                        });
                    }
                    read
                };
                let roll = BlockLootRoll {
                    levels: Some(&levels),
                    ..roll
                };
                let drops = items.block_drops(
                    registry,
                    state,
                    roll,
                    &mut random,
                    Some((&mut sequences, seed)),
                );
                self.world.random = random;
                *self.rand_sequences = sequences;
                drops
            })
        })
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use super::*;
    use mod_api::{EnchantedBlockLootEvent, Hooks, LevelHost};
    use std::sync::{Arc, Mutex};

    #[derive(Debug, PartialEq)]
    pub struct Read {
        pub pos: [i32; 3],
        pub state: mod_api::BlockState,
        pub tool: Option<String>,
        pub enchantment: String,
        pub level: u32,
        pub entity: Option<u32>,
    }

    /// Records every read and answers with `answer`, or leaves it.
    pub struct Fortune {
        pub answer: Option<u32>,
        pub reads: Arc<Mutex<Vec<Read>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Fortune {
        fn enchanted_block_loot(&self, _: &dyn LevelHost, event: &mut EnchantedBlockLootEvent<'_>) {
            self.reads.lock().unwrap().push(Read {
                pos: [event.pos.x, event.pos.y, event.pos.z],
                state: event.state,
                tool: event.tool.map(|tool| {
                    mod_api::__private::item_key_id(tool.item())
                        .unwrap()
                        .to_string()
                }),
                enchantment: event.enchantment.to_string(),
                level: event.level,
                entity: event.entity.map(|entity| entity.id()),
            });
            if let Some(answer) = self.answer {
                event.level = answer;
            }
        }
    }

    pub const POS: [i32; 3] = [1, 64, 1];
    pub const ROLLS: usize = 64;
}
