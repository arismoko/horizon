//! Block entities removed by the block behaviour (`LevelChunk.setBlockState`
//! running `BlockEntity.preRemoveSideEffects`) and the resources a destroyed
//! block drops (`Block.dropResources`), drawn from the level random at the
//! moment vanilla draws them. The item entities they make are added to the
//! world as soon as the behaviour returns, within the same tick.
use super::GameLevel;
use mod_api::EntityRef;
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::spill_common::{
    BlockDrops, BlockItemDrop, Pending, Spawn, UsedDrops, live_full_metadata, saved_entity,
    world_source,
};
use sunrise_items::items::{BlockLootRoll, ItemStack};
use sunrise_level::containers::BlockEntity;
use sunrise_world::registry::BlockStateId;

/// Lets `say` change each use's run of `spawns`, newest first so the earlier
/// ranges hold. A run's stacks keep their spots, and a stack `say` adds takes
/// the run's last spot, else the block's top.
fn retell_runs(
    spawns: &mut Vec<Spawn>,
    used: &[UsedDrops],
    mut say: impl FnMut(&UsedDrops, [f64; 3], &mut Vec<ItemStack>),
) {
    for used in used.iter().rev() {
        let run: Vec<Spawn> = spawns.drain(used.start..used.end).collect();
        let mut drops: Vec<ItemStack> = run.iter().map(|s| s.stack.clone()).collect();
        let origin = used.pos.map(|c| f64::from(c) + 0.5);
        say(used, origin, &mut drops);
        let top = [origin[0], origin[1] + 0.5, origin[2]];
        let kept: Vec<Spawn> = drops
            .into_iter()
            .enumerate()
            .map(|(i, stack)| Spawn {
                random: run.get(i).and_then(|s| s.random),
                stack,
                pos: run.get(i).or(run.last()).map_or(top, |s| s.pos),
                motion: run.get(i).and_then(|s| s.motion),
                delay: run.get(i).map_or(10, |s| s.delay),
            })
            .collect();
        spawns.splice(used.start..used.start, kept);
    }
}

/// `Block.getDrops(state, level, pos, blockEntity, entity, tool)` for the block still
/// standing: `drop_resources_now`'s roll, with what the block entity copies into the drops,
/// drawn from the same random, but nothing popped, no `block_drops` event and no
/// experience. A scratch record keeps a break in progress at `pos` as it was.
pub(in crate::game::server_side) fn block_drops_of(
    level: &mut GameLevel<'_>,
    pos: BlockPos,
    state: BlockStateId,
    tool: Option<&ItemStack>,
    entity: Option<EntityRef>,
) -> Vec<ItemStack> {
    let key = (pos.x, pos.y, pos.z);
    let at = [pos.x, pos.y, pos.z];
    let mut copied = Pending::default();
    if let Some(block_entity) = level.world.block_entities.get(&key) {
        let name = &level.facts.get(state).name;
        let current = level.encode_live_block_entity(key, block_entity, name);
        let saved = live_full_metadata(level.world, key, block_entity, name, level.items, current);
        copied.record_from(level.world, at, saved);
    } else if let Some(tag) = saved_entity(level.world, at) {
        copied.record_saved(at, &tag);
    }
    let dynamic = copied.dynamic(at);
    let roll = sunrise_items::items::BlockLootRoll {
        held: tool,
        dynamic: &dynamic,
        breaker: entity.is_some(),
        ..Default::default()
    };
    let source = copied.source(at);
    let mut drops = level.roll_block_loot(pos, state, entity, roll, source.as_deref());
    drops.retain(|stack| &*stack.item != "minecraft:air" && stack.count > 0);
    drops
}

impl crate::game::ServerGame<'_> {
    /// [`GameLevel::roll_block_loot`] for a roll outside a level write:
    /// `entity` is `THIS_ENTITY`'s id. With no listener it builds no level.
    pub(in crate::game) fn roll_block_loot(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        entity: Option<u32>,
        roll: BlockLootRoll<'_>,
    ) -> Vec<ItemStack> {
        let source = self.common.pending.source(pos).or_else(|| {
            self.common
                .pack
                .assets
                .registry
                .state(state)
                .and_then(|block| {
                    world_source(
                        &self.common.world,
                        pos,
                        &block.name,
                        world_items!(self),
                        |key, entity, block| {
                            self.encode_live_block_entity(key, entity, block, world_items!(self))
                        },
                    )
                })
        });
        let storage = self.common.score.storage.snapshot();
        sunrise_world::loot::copy::with_storage(storage, || {
            sunrise_world::loot::copy::with_block_source(source.as_deref(), || {
                if mod_api::__private::listened(mod_api::HookId::EnchantedBlockLoot) {
                    let at = BlockPos::new(pos[0], pos[1], pos[2]);
                    let entity = self.loot_entity_ref(entity);
                    return self.with_level(|_, level, _| {
                        level.roll_block_loot(at, state, entity, roll, source.as_deref())
                    });
                }
                // ORIGIN is the block's centre; its location checks read the level, so
                // the roll draws from copies of the randoms put back after it.
                let mut random = self.common.world.random.clone();
                let mut sequences = std::mem::take(&mut self.server.rand_sequences);
                let drops = {
                    let level = crate::game::advancements::View(&*self);
                    let read = |predicate: &serde_json::Value, at: [f64; 3]| {
                        sunrise_sim::advancements::location(predicate, at, &level)
                    };
                    let origin = pos.map(|c| f64::from(c) + 0.5);
                    let roll = BlockLootRoll {
                        location: Some(sunrise_sim::advancements::loot_location(origin, &read)),
                        ..roll
                    };
                    server_world_items!(self).block_drops(
                        self.common.pack.assets.registry,
                        state,
                        roll,
                        &mut random,
                        Some((&mut sequences, self.common.level.seed)),
                    )
                };
                self.common.world.random = random;
                self.server.rand_sequences = sequences;
                drops
            })
        })
    }
    /// `Block.dropFromBlockInteractLootTable` for interactions run directly on
    /// `Game` (`PumpkinBlock`, `BeehiveBlock`, `ShearsDispenseItemBehavior`):
    /// supplies `BLOCK_STATE`, `ORIGIN` (block centre), optional `BLOCK_ENTITY`,
    /// optional `INTERACTING_ENTITY`, optional `TOOL`, and the level.
    pub(in crate::game) fn interact_loot_at(
        &mut self,
        table: &str,
        pos: [i32; 3],
        state: BlockStateId,
        interacting: Option<sunrise_world::loot::LootEntity>,
        held: Option<&ItemStack>,
    ) -> Vec<ItemStack> {
        let centre = pos.map(|c| f64::from(c) + 0.5);
        let source = self
            .common
            .pack
            .assets
            .registry
            .state(state)
            .and_then(|block| {
                world_source(
                    &self.common.world,
                    pos,
                    &block.name,
                    world_items!(self),
                    |key, entity, block| {
                        self.encode_live_block_entity(key, entity, block, world_items!(self))
                    },
                )
            });
        let mut place = self.common.loot_place_at(centre, None);
        if let Some(source) = source {
            place.block_entity = Some(source.nbt.clone());
            place.block_components = Some(source.components.clone());
        }
        let loot = self.server.server_loot.0.get();
        let matcher = |predicate: &serde_json::Value, entity: &sunrise_world::loot::LootEntity| {
            loot.as_ref()
                .is_some_and(|data| data.entity_matches(predicate, Some(entity)))
        };
        let interacting_target = interacting.as_ref().map(|entity| {
            (
                entity,
                &matcher as &dyn Fn(&serde_json::Value, &sunrise_world::loot::LootEntity) -> bool,
            )
        });
        sunrise_world::loot::with_native_loot_place(place, || {
            server_world_items!(self).interact_loot(
                table,
                self.common.pack.assets.registry,
                state,
                interacting_target,
                held,
                None,
                &mut self.common.world.random,
                &mut self.server.rand_sequences,
                self.common.level.seed,
            )
        })
    }
    /// The handle of `LootContextParams.THIS_ENTITY`, by its id.
    pub(in crate::game) fn loot_entity_ref(&self, id: Option<u32>) -> Option<EntityRef> {
        let epoch = self.common.native_lifecycle.epoch()?;
        Some(EntityRef::new(epoch, id.filter(|id| *id != 0)?))
    }
    /// Apply the shared block-drop event's item entities and experience unchanged.
    pub(in crate::game::server_side) fn spawn_block_drops(
        &mut self,
        pos: [i32; 3],
        dropped: BlockDrops,
    ) {
        for item in dropped.items {
            self.add_item_drop(item);
        }
        if dropped.experience > 0 {
            self.spawn_orbs(pos.map(|v| f64::from(v) + 0.5), dropped.experience);
        }
    }
    /// Adds a drop's item entity, with the state a drop hook set.
    pub(in crate::game) fn add_item_drop(&mut self, drop: BlockItemDrop) {
        let random = drop.random_with_state();
        let velocity = Some(drop.velocity);
        self.add_item_entity_prepared(
            drop.stack,
            drop.position,
            velocity,
            drop.pickup_delay,
            random,
        );
    }
    pub(in crate::game) fn publish_removal_spawns(
        &mut self,
        spawns: Vec<Spawn>,
        orbs: Vec<([f64; 3], u32, i32)>,
    ) {
        let _save_lease = sunrise_sim::join_seq::visibility::PassLease::enter();
        for spawn in spawns {
            self.push_item_entity(
                spawn.stack,
                spawn.pos,
                spawn.motion,
                spawn.delay,
                spawn.random,
            );
            self.join_removal_item(self.server.next_entity, spawn.pos);
        }
        for (at, value, merge_id) in orbs {
            self.award_orb(at, value, merge_id);
        }
    }

    /// Publish item entities and orbs already prepared by a block operation.
    pub(in crate::game) fn publish_block_spawns(
        &mut self,
        spawns: Vec<Spawn>,
        orbs: Vec<([f64; 3], u32, i32)>,
    ) {
        let _save_lease = sunrise_sim::join_seq::visibility::PassLease::enter();
        for s in spawns {
            self.add_item_entity_prepared(s.stack, s.pos, s.motion, s.delay, s.random);
        }
        for (at, value, merge_id) in orbs {
            self.award_orb(at, value, merge_id);
        }
    }

    /// Add the item entities the block behaviour spawned and follow up on the
    /// block entities it removed.
    pub fn flush_spill(&mut self) {
        if self.server.spill.0.is_empty() {
            return;
        }
        let _save_lease = sunrise_sim::join_seq::visibility::PassLease::enter();
        let mut spill = std::mem::take(&mut self.server.spill.0);
        self.finish_piston_position_changes(std::mem::take(&mut spill.piston_positions));
        let used = std::mem::take(&mut spill.used);
        let tool = self.common.held().cloned();
        let player = Some(self.native_acting_id());
        retell_runs(&mut spill.spawns, &used, |used, origin, drops| {
            let block = Some((used.pos, used.state));
            self.native_use_drops(
                used.table,
                origin,
                block,
                None,
                player,
                tool.as_ref(),
                drops,
            );
        });
        self.publish_block_spawns(spill.spawns, spill.orbs);
        for (pos, entity) in spill.removed {
            let key = (pos[0], pos[1], pos[2]);
            if matches!(entity, BlockEntity::Jukebox(_)) {
                self.jukebox_removed(pos);
            }
            if self.common.container == Some(key) {
                self.container_opener_event(key, false);
                self.common.container = None;
            }
        }
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::block_entity_save_additional::BlockEntitySaveAdditionalGameExt1 as _;

#[cfg(test)]
#[path = "loot_context_tests.rs"]
mod loot_context_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_core::random::RandomSource;
    use sunrise_gameplay_common::game::spill_common::BlockDropEntity;

    #[test]
    #[ignore = "daily"]
    fn disabled_block_drops_draw_offsets_without_constructing_items() {
        use crate::game::native_entity_refs::tests::with_game;
        use std::sync::{Arc, Mutex};
        use sunrise_level::random_source::record_seeds;

        struct Observe(Arc<Mutex<Vec<usize>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Observe {
            fn block_drops(
                &self,
                _: &dyn mod_api::LevelHost,
                event: &mut mod_api::BlockDropsEvent<'_>,
            ) -> mod_api::Flow {
                self.0.lock().unwrap().push(event.drops.len());
                mod_api::Flow::Continue
            }
        }

        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "observe",
            Box::new(Observe(seen.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                assert!(game.ui.rules.set(
                    "block_drops",
                    sunrise_game_data::gamerules::Value::Bool(false),
                ));
                let dirt = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:dirt")
                    .unwrap();
                game.world.random.set_seed(7);
                let (dropped, seeds) = record_seeds(|| {
                    game.with_level(|_, level, _| {
                        level.block_dropped(
                            BlockPos::new(2, 64, 1),
                            dirt,
                            None,
                            None,
                            BlockDropEntity::None,
                            vec![ItemStack::new("minecraft:dirt", 1)],
                            Some(5),
                        )
                    })
                });
                assert!(dropped.items.is_empty());
                assert_eq!(dropped.experience, 0);
                assert!(seeds.complete(), "no entity RNG seed is drawn");
                // Java Random(7) after three nextDouble calls for the offsets.
                assert_eq!(game.world.random.next_int(), -441_191_359);
            });
        });
        assert_eq!(*seen.lock().unwrap(), [0]);
    }

    use sunrise_core::nbt::Tag;
    use sunrise_items::item_data;

    #[test]
    fn a_used_run_is_retold_in_place() {
        let spawn = |item: &str, x: f64| Spawn {
            random: None,
            stack: ItemStack::new(item, 1),
            pos: [x, 0.0, 0.0],
            motion: None,
            delay: 10,
        };
        let mut spawns = vec![
            spawn("minecraft:stone", 0.0),
            spawn("minecraft:sweet_berries", 1.0),
            spawn("minecraft:sweet_berries", 2.0),
            spawn("minecraft:dirt", 3.0),
        ];
        let used = [UsedDrops {
            start: 1,
            end: 3,
            pos: [4, 64, 4],
            state: BlockStateId(0),
            table: "minecraft:harvest/sweet_berry_bush",
        }];
        retell_runs(&mut spawns, &used, |used, origin, drops| {
            assert_eq!(used.table, "minecraft:harvest/sweet_berry_bush");
            assert_eq!(origin, [4.5, 64.5, 4.5]);
            assert_eq!(drops.len(), 2);
            drops.remove(0);
            drops.push(ItemStack::new("minecraft:diamond", 1));
        });
        let items: Vec<(&str, f64)> = spawns.iter().map(|s| (&*s.stack.item, s.pos[0])).collect();
        assert_eq!(
            items,
            [
                ("minecraft:stone", 0.0),
                ("minecraft:sweet_berries", 1.0),
                ("minecraft:diamond", 2.0),
                ("minecraft:dirt", 3.0),
            ]
        );
    }

    #[test]
    fn tool_source_keeps_the_full_patch_and_distinguishes_stored_enchantments() {
        let potion = Tag::compound([("potion", Tag::String("minecraft:healing".into()))]);
        let stored = Tag::compound([("minecraft:sharpness", Tag::Int(13))]);
        let patch = Tag::compound([
            ("minecraft:potion_contents", potion.clone()),
            ("minecraft:stored_enchantments", stored.clone()),
            ("!minecraft:unbreakable", Tag::Compound(Default::default())),
        ]);
        let (damage, extra) = item_data::decode_components(Some(&patch));
        let tool = ItemStack {
            item: std::sync::Arc::from("minecraft:enchanted_book"),
            count: 1,
            damage,
            extra,
        };
        let snapshot = sunrise_gameplay_common::game::creatures::to_loot(&tool);
        assert!(snapshot.enchantments.is_empty());
        let prototype = [
            (
                "minecraft:unbreakable".into(),
                Tag::Compound(Default::default()),
            ),
            (
                "minecraft:enchantments".into(),
                Tag::Compound(Default::default()),
            ),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            sunrise_world::loot::copy::stack_components(&snapshot, Some(&prototype)),
            [
                ("minecraft:potion_contents".into(), potion),
                ("minecraft:stored_enchantments".into(), stored),
                (
                    "minecraft:enchantments".into(),
                    Tag::Compound(Default::default())
                )
            ]
            .into_iter()
            .collect()
        );
    }

    #[test]
    #[cfg(any())]
    fn ominous_banner_survives_placement_save_and_drop() {
        use sunrise_sim::{block_entity_item, mobs::raider, save};

        let original = raider::ominous_banner();
        let Tag::Compound(components) = item_data::component_snapshot(&original).unwrap() else {
            panic!("banner components");
        };
        for block in ["minecraft:white_banner", "minecraft:white_wall_banner"] {
            let pos = (1, 64, 2);
            let fresh = BlockEntity::for_block(block).unwrap();
            let fresh = save::block_entity_encode::encode(pos, &fresh, block, &Default::default());
            let placed = block_entity_item::place(&fresh, &components, false);
            let (_, placed) = save::decode_block_entity(&placed).unwrap();
            let saved = save::block_entity_encode::encode(pos, &placed, block, &Default::default());
            let (_, loaded) = save::decode_block_entity(&saved).unwrap();
            let BlockEntity::Banner(banner) = &loaded else {
                panic!("placed banner");
            };
            assert_eq!(banner.layers.len(), item_data::OMINOUS_LAYERS.len());
            let mut pending = Pending::default();
            pending.record_saved([pos.0, pos.1, pos.2], &saved);
            let mut items = sunrise_items::items::Items::for_tests(Default::default());
            items.set_loot_tables(std::sync::Arc::new([(
                "test:banner".to_owned(),
                serde_json::json!({"type":"minecraft:empty","pools":[{"rolls":1,"entries":[{
                    "type":"minecraft:item","name":"minecraft:white_banner",
                    "modifier":{"type":"minecraft:copy_components","source":"block_entity",
                        "include":["minecraft:banner_patterns","minecraft:custom_name",
                            "minecraft:item_name","minecraft:rarity","minecraft:tooltip_display"]}
                }]}]}),
            )].into_iter().collect()));
            let source = pending.source([pos.0, pos.1, pos.2]);
            let dropped = sunrise_world::loot::copy::with_block_source(source.as_deref(), || {
                items
                    .loot_table(
                        "test:banner",
                        false,
                        None,
                        &mut sunrise_core::random::LegacyRandom::new(0),
                    )
                    .remove(0)
            });
            assert!(dropped.data().unwrap().ominous_banner);
            assert!(sunrise_sim::banner::tooltip_layers(&dropped).is_empty());
            assert_eq!(
                sunrise_client::item_tooltip::name_line(&dropped, &|_| None),
                "Ominous Banner"
            );
            assert_eq!(
                sunrise_sim::stations::rarity(&dropped, "common"),
                "uncommon"
            );
            assert_eq!(
                item_data::component_snapshot(&dropped),
                Some(Tag::Compound(components.clone()))
            );
        }
    }
}
