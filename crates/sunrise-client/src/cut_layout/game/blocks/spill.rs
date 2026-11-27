#![cfg(test)]
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::GameLevel;
use sunrise_items::items::ItemStack;
use sunrise_level::containers::BlockEntity;
#[cfg(test)]
pub(crate) mod tests {

    const MANY_REMOVAL_EFFECTS: u32 = 1025;
    static REMOVAL_PANICKED: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    struct RemovalMod(u8);
    struct RemovalPart(u8, mod_api::ItemKey);
    #[mod_api::sunrise_part]
    impl mod_api::BlockPart for RemovalPart {}
    #[mod_api::sunrise_part]
    impl mod_api::BlockEntityPart for RemovalPart {
        fn pre_remove_side_effects(
            &self,
            _: &mut dyn mod_api::LevelRandomHost,
            event: &mut mod_api::PreRemoveSideEffectsEvent<'_>,
        ) {
            let stack = || mod_api::ItemStack::of(self.1);
            let at = [1.25, 65.75, -2.5];
            let effects = &mut *event.effects;
            match self.0 {
                1 => effects.set_keep_contents(true),
                2 => effects.spawn_item(stack(), at).unwrap(),
                3 => effects.award_experience(10, at).unwrap(),
                4 => {
                    effects.drop_slot(None).unwrap();
                    effects.drop_slot(Some(stack())).unwrap();
                }
                5 => {
                    use mod_api::{BlockRemovalEffectError, StackWriteError};
                    assert_eq!(event.entity.data_slots(), [5]);
                    for index in 0..MANY_REMOVAL_EFFECTS {
                        // Supply them in reverse phase order. Publication uses base slots,
                        // extra scatter slots, positioned items, then XP.
                        let pos = [at[0] + f64::from(index), at[1], at[2]];
                        effects.award_experience(1, pos).unwrap();
                        effects.spawn_item(stack(), pos).unwrap();
                        let count = 1 + index % 2;
                        effects
                            .drop_slot(Some(mod_api::ItemStack::new(self.1, count).unwrap()))
                            .unwrap();
                        if index == 1 {
                            let air = mod_api::ItemStack::of(
                                mod_api::__private::item_key("minecraft:air").unwrap(),
                            );
                            assert_eq!(
                                effects.drop_slot(Some(air.clone())),
                                Err(BlockRemovalEffectError::Stack(
                                    StackWriteError::EmptyEngineStack
                                ))
                            );
                            assert_eq!(
                                effects.spawn_item(air, pos),
                                Err(BlockRemovalEffectError::Stack(
                                    StackWriteError::EmptyEngineStack
                                ))
                            );
                            let invalid = mod_api::ItemStack::new(self.1, u32::MAX).unwrap();
                            assert!(matches!(
                                effects.drop_slot(Some(invalid.clone())),
                                Err(BlockRemovalEffectError::Stack(
                                    StackWriteError::CountExceeded { .. }
                                ))
                            ));
                            assert!(matches!(
                                effects.spawn_item(invalid, pos),
                                Err(BlockRemovalEffectError::Stack(
                                    StackWriteError::CountExceeded { .. }
                                ))
                            ));
                            assert_eq!(
                                effects.spawn_item(stack(), [f64::NAN, 0.0, 0.0]),
                                Err(BlockRemovalEffectError::NonFinitePosition)
                            );
                            assert_eq!(
                                effects.award_experience(1, [0.0, f64::INFINITY, 0.0]),
                                Err(BlockRemovalEffectError::NonFinitePosition)
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }
    struct PanickingRemovalPart(mod_api::ItemKey);
    #[mod_api::sunrise_part]
    impl mod_api::BlockPart for PanickingRemovalPart {}
    #[mod_api::sunrise_part]
    impl mod_api::BlockEntityPart for PanickingRemovalPart {
        fn pre_remove_side_effects(
            &self,
            _: &mut dyn mod_api::LevelRandomHost,
            event: &mut mod_api::PreRemoveSideEffectsEvent<'_>,
        ) {
            event.effects.set_keep_contents(true);
            event.effects.drop_slot(None).unwrap();
            event
                .effects
                .spawn_item(mod_api::ItemStack::of(self.0), [0.0; 3])
                .unwrap();
            event.effects.award_experience(1, [0.0; 3]).unwrap();
            REMOVAL_PANICKED.store(true, std::sync::atomic::Ordering::Relaxed);
            panic!("refuse this Part's removal effects");
        }
    }
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for RemovalMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let kind = r.block_entity_type("machine").blocks(&[]).build();
            let diamond = r.item_key("minecraft:diamond");
            r.block_entity_part(&kind, RemovalPart(self.0, diamond))
                .unwrap();
            if self.0 == 5 {
                r.block_entity_part(&kind, PanickingRemovalPart(diamond))
                    .unwrap();
            }
        }
    }
    /// Listens to `block_drops`: takes any break of dirt, adds 2 points to the rest.
    struct DropsMod;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for DropsMod {
        fn block_drops(
            &self,
            _: &dyn mod_api::LevelHost,
            event: &mut mod_api::BlockDropsEvent<'_>,
        ) -> mod_api::Flow {
            if event.drops.iter().any(|d| {
                d.stack().is_some_and(|s| {
                    mod_api::__private::item_key_id(s.item()) == Some("minecraft:dirt")
                })
            }) {
                return mod_api::Flow::Cancel;
            }
            event.experience += 2;
            mod_api::Flow::Continue
        }
    }

    use super::*;
    use sunrise_core::random::RandomSource;
    use sunrise_gameplay_common::game::GameGameExt1;

    use crate::game::spill_common::BlockDropEntity;

    use sunrise_world::level::removal;

    fn removal_outputs(
        mode: u8,
        check: impl FnOnce(&mut GameLevel<'_>, &mut sunrise_core::random::LevelRandom),
    ) {
        use std::sync::Arc;
        use sunrise_level::containers::{ModBlockEntity, ModBlockEntityFields, ModContainer};
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "removal",
            Box::new(RemovalMod(mode)),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                let state = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                let mut entity = ModBlockEntity::new(
                    "removal:machine".into(),
                    ModBlockEntityFields {
                        data: vec![i32::from(mode)],
                        ..Default::default()
                    },
                );
                if matches!(mode, 1 | 5) {
                    let item = if mode == 5 {
                        "minecraft:gold_ingot"
                    } else {
                        "minecraft:diamond"
                    };
                    entity.items = vec![Some(ItemStack::new(item, 1))];
                    game.world.block_entities.set_mod_containers(
                        [(
                            state.0,
                            Arc::new(ModContainer {
                                slots: 1,
                                ..Default::default()
                            }),
                        )]
                        .into_iter()
                        .collect(),
                    );
                }
                game.world
                    .block_entities
                    .insert((1, 64, 2), BlockEntity::Mod(Box::new(entity)));
                game.with_level(|_, level, _| {
                    let mut reference = level.world.random.clone();
                    level.remove_entity(BlockPos::new(1, 64, 2), state, true);
                    check(level, &mut reference);
                });
            });
        });
    }
    #[test]
    #[ignore = "daily"]
    fn removal_keep_contents_suppresses_slot_scatter() {
        removal_outputs(1, |level, reference| {
            assert!(level.spill.spawns.is_empty());
            assert_eq!(level.world.random.next_int(), reference.next_int());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn removal_uses_the_same_random_state_for_lazy_and_padded_slots() {
        use std::sync::Arc;
        use sunrise_core::random::LegacyRandom;
        use sunrise_level::containers::{ModBlockEntity, ModBlockEntityFields, ModContainer};

        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let state = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.block_entities.set_mod_containers(
                [(
                    state.0,
                    Arc::new(ModContainer {
                        slots: 3,
                        ..Default::default()
                    }),
                )]
                .into_iter()
                .collect(),
            );
            let mut reference = LegacyRandom::new(7);
            for _ in 0..9 {
                reference.next_double();
            }
            let pos = BlockPos::new(1, 64, 2);
            for len in [3, 0, 1] {
                game.world.random.set_seed(7);
                game.world.block_entities.insert(
                    (pos.x, pos.y, pos.z),
                    BlockEntity::Mod(Box::new(ModBlockEntity::new(
                        "removal:machine".into(),
                        ModBlockEntityFields {
                            items: vec![None; len],
                            ..Default::default()
                        },
                    ))),
                );
                game.with_level(|_, level, _| {
                    level.remove_entity(pos, state, true);
                    assert!(level.spill.spawns.is_empty());
                    assert_eq!(
                        level.world.random.state(),
                        reference.state(),
                        "stored slots: {len}"
                    );
                });
            }
        });
    }
    #[test]
    #[ignore = "daily"]
    fn removal_spawn_has_exact_position_and_default_delay() {
        removal_outputs(2, |level, _| {
            assert_eq!(level.spill.spawns.len(), 1);
            let spawn = &level.spill.spawns[0];
            assert_eq!(spawn.pos, [1.25, 65.75, -2.5]);
            assert_eq!(spawn.delay, 10);
            assert_eq!(&*spawn.stack.item, "minecraft:diamond");
            assert_eq!(spawn.stack.count, 1);
            assert!(spawn.motion.is_none());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn removal_experience_awards_positioned_orbs() {
        removal_outputs(3, |level, reference| {
            assert_eq!(
                level
                    .spill
                    .orbs
                    .iter()
                    .map(|(_, value, _)| *value)
                    .collect::<Vec<_>>(),
                [7, 3]
            );
            for (at, _, merge) in &level.spill.orbs {
                assert_eq!(*at, [1.25, 65.75, -2.5]);
                assert_eq!(*merge, reference.next_int_bound(40).unwrap());
            }
            assert_eq!(level.world.random.next_int(), reference.next_int());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn removal_effects_are_uncapped_and_refusals_are_local() {
        if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::server_side::spill::tests::removal_effects_are_uncapped_and_refusals_are_local",
            "SUNRISE_REMOVAL_EFFECTS_CHILD",
            &["removal"],
        ) {
            return;
        }
        removal_outputs(5, |level, reference| {
            assert!(REMOVAL_PANICKED.load(std::sync::atomic::Ordering::Relaxed));
            let n = MANY_REMOVAL_EFFECTS as usize;
            let spawns = &level.spill.spawns;
            assert_eq!(spawns.len(), 1 + 2 * n);
            assert_eq!(&*spawns[0].stack.item, "minecraft:gold_ingot");
            let counts = std::iter::once(1).chain((0..MANY_REMOVAL_EFFECTS).map(|i| 1 + i % 2));
            for (spawn, count) in spawns[..1 + n].iter().zip(counts) {
                assert_eq!(spawn.stack.count, count);
                assert_eq!(spawn.delay, 0);
                let scattered = removal::drop_item_stack(reference, BlockPos::new(1, 64, 2), count);
                assert_eq!(spawn.pos, scattered[0].pos);
            }
            for (index, spawn) in (0..MANY_REMOVAL_EFFECTS).zip(&spawns[1 + n..]) {
                assert_eq!(&*spawn.stack.item, "minecraft:diamond");
                assert_eq!(spawn.delay, 10);
                assert_eq!(spawn.pos, [1.25 + f64::from(index), 65.75, -2.5]);
            }
            assert_eq!(level.spill.orbs.len(), n);
            for (index, (at, value, merge)) in (0..MANY_REMOVAL_EFFECTS).zip(&level.spill.orbs) {
                assert_eq!(*at, [1.25 + f64::from(index), 65.75, -2.5]);
                assert_eq!(*value, 1);
                assert_eq!(*merge, reference.next_int_bound(40).unwrap());
            }
            assert_eq!(level.world.random.next_int(), reference.next_int());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn removal_empty_slot_draws_three_doubles_without_spawning() {
        removal_outputs(4, |level, reference| {
            // Containers.dropItemStack draws x, y, z before its isEmpty loop.
            for _ in 0..3 {
                reference.next_double();
            }
            // The filled slot follows the empty one, using its own three draws.
            let at = [
                1.125 + reference.next_double() * 0.75,
                64.0 + reference.next_double() * 0.75,
                2.125 + reference.next_double() * 0.75,
            ];
            reference.next_int_bound(21).unwrap();
            for _ in 0..6 {
                reference.next_double();
            }
            assert_eq!(level.spill.spawns.len(), 1);
            assert_eq!(level.spill.spawns[0].pos, at);
            assert_eq!(level.spill.spawns[0].stack.count, 1);
            assert_eq!(level.world.random.next_int(), reference.next_int());

            // Engine-owned contents can contain air; Containers treats it as an empty slot.
            level.drop_item_stack(
                BlockPos::new(1, 64, 2),
                Some(ItemStack::new("minecraft:air", 1)),
            );
            for _ in 0..3 {
                reference.next_double();
            }
            assert_eq!(level.spill.spawns.len(), 1);
            assert_eq!(level.world.random.next_int(), reference.next_int());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn removal_unanswered_part_leaves_random_untouched() {
        removal_outputs(0, |level, reference| {
            assert!(level.spill.spawns.is_empty());
            assert!(level.spill.orbs.is_empty());
            assert_eq!(level.world.random.next_int(), reference.next_int());
        });
    }
    /// `Block.dropResources` builds its item entities (and samples the experience) before
    /// `BlockDropsEvent`, so the level random ends where it does without a listener; a mod that
    /// cancels gets no entities and no experience but the draws still happened; a mod that
    /// edits the amount changes only the amount.
    #[test]
    #[ignore = "daily"]
    fn block_drops_draws_before_the_event_and_cancel_empties_the_break() {
        use std::sync::Arc;
        use sunrise_core::random::LegacyRandom;
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let dirt = registry.default_state("minecraft:dirt").unwrap();
            let ore = registry.default_state("minecraft:diamond_ore").unwrap();
            let pos = BlockPos::new(1, 64, 2);
            let break_of =
                |game: &mut sunrise_gameplay::game::ServerGame<'_>, state, item, exp_to_drop| {
                    game.world.random =
                        sunrise_core::random::LevelRandom::for_level(LegacyRandom::new(7));
                    let dropped = game.with_level(|_, level, _| {
                        level.block_dropped(
                            pos,
                            state,
                            None,
                            None,
                            BlockDropEntity::None,
                            vec![ItemStack::new(item, 1)],
                            exp_to_drop,
                        )
                    });
                    (dropped, game.world.random.next_int())
                };
            let (plain_ore, ore_after) = break_of(game, ore, "minecraft:diamond", None);
            let (plain_dirt, dirt_after) = break_of(game, dirt, "minecraft:dirt", None);
            let (overridden, _) = break_of(game, ore, "minecraft:diamond", Some(17));
            assert_eq!(overridden.experience, 17);
            assert_eq!(plain_dirt.items.len(), 1);
            let mods: mod_api::Mods =
                vec![mod_api::ModEntry::new("block-drops", Box::new(DropsMod))].into();
            mod_platform::testing::with_mods(mods, || {
                let (heard, after) = break_of(game, ore, "minecraft:diamond", None);
                assert_eq!(after, ore_after);
                assert_eq!(heard.items.len(), 1);
                assert_eq!(heard.items[0].position, plain_ore.items[0].position);
                assert_eq!(heard.experience, plain_ore.experience + 2);
                assert!(!heard.canceled);
                let (overridden, _) = break_of(game, ore, "minecraft:diamond", Some(17));
                assert_eq!(overridden.experience, 19);
                let (taken, after) = break_of(game, dirt, "minecraft:dirt", None);
                assert_eq!(after, dirt_after);
                assert!(taken.canceled && taken.items.is_empty() && taken.experience == 0);
            });
        });
    }
    /// Removing drop 0 shifts drop 1 without stealing drop 0's `ItemRandom`, and a later
    /// listener's invalid stack edit or out-of-bounds edit is rejected per call without rolling
    /// back earlier listeners' valid drop mutations or experience.
    #[test]
    #[ignore = "daily"]
    fn block_drops_per_edit_rejection_and_random_preservation() {
        use mod_api::{BlockDropsEvent, Flow, Hooks, ItemDrop, ModEntry, Mods, StackWriteError};
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        use sunrise_core::random::LegacyRandom;

        struct MutatingListener;
        #[mod_api::__subscriptions]
        impl Hooks for MutatingListener {
            fn block_drops(
                &self,
                _: &dyn mod_api::LevelHost,
                event: &mut BlockDropsEvent<'_>,
            ) -> Flow {
                assert_eq!(event.drops.len(), 2);
                assert!(event.drops.remove(0));
                assert_eq!(event.drops.len(), 1);
                event.drops.get_mut(0).unwrap().set_pickup_delay(25);
                let diamond = mod_api::ItemStack::new(
                    mod_api::__private::item_key("minecraft:diamond").unwrap(),
                    1,
                )
                .unwrap();
                event
                    .drops
                    .push(ItemDrop::new(
                        diamond,
                        [1.5, 65.0, 2.5],
                        [0.0, 0.2, 0.0],
                        10,
                    ))
                    .unwrap();
                event.experience = 9;
                Flow::Continue
            }
        }

        struct InvalidEditListener(Arc<AtomicBool>);
        #[mod_api::__subscriptions]
        impl Hooks for InvalidEditListener {
            fn block_drops(
                &self,
                _: &dyn mod_api::LevelHost,
                event: &mut BlockDropsEvent<'_>,
            ) -> Flow {
                assert_eq!(event.drops.len(), 2);
                let stone = mod_api::__private::item_key("minecraft:stone").unwrap();
                let invalid = mod_api::ItemStack::new(stone, 65).unwrap();
                let invalid_drop = ItemDrop::new(invalid.clone(), [1.5, 65.0, 2.5], [0.0; 3], 10);
                assert_eq!(
                    event.drops.push(invalid_drop),
                    Err(StackWriteError::CountExceeded { maximum: 64 })
                );
                assert_eq!(
                    event.drops.get_mut(0).unwrap().set_stack(invalid),
                    Err(StackWriteError::CountExceeded { maximum: 64 })
                );
                let valid = ItemDrop::new(
                    mod_api::ItemStack::new(stone, 1).unwrap(),
                    [1.5, 65.0, 2.5],
                    [0.0; 3],
                    10,
                );
                assert_eq!(event.drops.insert(99, valid), Ok(false));
                assert!(!event.drops.remove(99));
                assert_eq!(event.drops.len(), 2);
                let surviving = event.drops.get(0).unwrap();
                assert_eq!(
                    mod_api::__private::item_key_id(surviving.stack().unwrap().item()),
                    Some("minecraft:cobblestone")
                );
                assert_eq!(surviving.pickup_delay(), 25);
                self.0.store(true, Ordering::Relaxed);
                Flow::Continue
            }
        }

        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            let pos = BlockPos::new(1, 64, 2);
            let break_of = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                game.world.random =
                    sunrise_core::random::LevelRandom::for_level(LegacyRandom::new(7));
                let dropped = game.with_level(|_, level, _| {
                    level.block_dropped(
                        pos,
                        stone,
                        None,
                        None,
                        BlockDropEntity::None,
                        vec![
                            ItemStack::new("minecraft:stone", 1),
                            ItemStack::new("minecraft:cobblestone", 1),
                        ],
                        None,
                    )
                });
                (dropped, game.world.random.next_int())
            };

            let ((baseline, baseline_after), entity_seeds) =
                sunrise_level::random_source::record_seeds(|| break_of(game));
            assert_eq!(baseline.items.len(), 2);
            assert!(baseline.items[0].random.is_some());
            assert!(baseline.items[1].random.is_some());
            assert_ne!(baseline.items[0].random, baseline.items[1].random);

            let ran_second = Arc::new(AtomicBool::new(false));
            let mods: Mods = vec![
                ModEntry::new("first", Box::new(MutatingListener)),
                ModEntry::new("second", Box::new(InvalidEditListener(ran_second.clone()))),
            ]
            .into();
            mod_platform::testing::with_mods(mods, || {
                let (heard, after) =
                    sunrise_level::random_source::replay_seeds(Arc::clone(&entity_seeds), || {
                        break_of(game)
                    });
                assert!(entity_seeds.complete());
                assert!(ran_second.load(Ordering::Relaxed));
                assert_eq!(after, baseline_after);
                assert_eq!(heard.experience, 9);
                assert!(!heard.canceled);
                assert_eq!(heard.items.len(), 2);
                assert_eq!(&*heard.items[0].stack.item, "minecraft:cobblestone");
                assert_eq!(heard.items[0].stack.count, 1);
                assert_eq!(heard.items[0].pickup_delay, 25);
                assert_eq!(heard.items[0].position, baseline.items[1].position);
                assert_eq!(heard.items[0].velocity, baseline.items[1].velocity);
                assert_eq!(heard.items[0].random, baseline.items[1].random);
                assert_ne!(heard.items[0].random, baseline.items[0].random);
                assert_eq!(&*heard.items[1].stack.item, "minecraft:diamond");
                assert_eq!(heard.items[1].stack.count, 1);
                assert_eq!(heard.items[1].position, [1.5, 65.0, 2.5]);
                assert_eq!(heard.items[1].velocity, [0.0, 0.2, 0.0]);
                assert_eq!(heard.items[1].pickup_delay, 10);
                assert_eq!(heard.items[1].random, None);
            });
        });
    }
    /// An empty or unrepresentable optional tool (such as an unregistered item from an older
    /// save) omits only `event.tool` and still dispatches `block_drops` so listeners can edit
    /// drops and experience or cancel the break.
    #[test]
    #[ignore = "daily"]
    fn block_drops_dispatches_with_empty_or_unrepresentable_tool() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use sunrise_core::random::LegacyRandom;

        struct ToolOmitListener(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for ToolOmitListener {
            fn block_drops(
                &self,
                _: &dyn mod_api::LevelHost,
                event: &mut mod_api::BlockDropsEvent<'_>,
            ) -> mod_api::Flow {
                self.0.fetch_add(1, Ordering::Relaxed);
                assert!(
                    event.tool.is_none(),
                    "empty or unrepresentable tools omit only event.tool"
                );
                if event.drops.iter().any(|drop| {
                    drop.stack().is_some_and(|stack| {
                        mod_api::__private::item_key_id(stack.item()) == Some("minecraft:dirt")
                    })
                }) {
                    return mod_api::Flow::Cancel;
                }
                event.drops.get_mut(0).unwrap().set_pickup_delay(44);
                event.experience += 5;
                mod_api::Flow::Continue
            }
        }

        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let ore = registry.default_state("minecraft:diamond_ore").unwrap();
            let dirt = registry.default_state("minecraft:dirt").unwrap();
            let pos = BlockPos::new(1, 64, 2);
            let calls = Arc::new(AtomicUsize::new(0));
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "tool-omit",
                Box::new(ToolOmitListener(calls.clone())),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                for tool in [
                    ItemStack::new("missingmod:ghost_pick", 1),
                    ItemStack::new("minecraft:air", 1),
                    ItemStack::new("minecraft:diamond_pickaxe", 0),
                ] {
                    game.world.random =
                        sunrise_core::random::LevelRandom::for_level(LegacyRandom::new(7));
                    let edited = game.with_level(|_, level, _| {
                        level.block_dropped(
                            pos,
                            ore,
                            None,
                            Some(&tool),
                            BlockDropEntity::None,
                            vec![ItemStack::new("minecraft:diamond", 1)],
                            Some(3),
                        )
                    });
                    assert!(!edited.canceled);
                    assert_eq!(edited.experience, 8);
                    assert_eq!(edited.items.len(), 1);
                    assert_eq!(edited.items[0].pickup_delay, 44);

                    let canceled = game.with_level(|_, level, _| {
                        level.block_dropped(
                            pos,
                            dirt,
                            None,
                            Some(&tool),
                            BlockDropEntity::None,
                            vec![ItemStack::new("minecraft:dirt", 1)],
                            Some(3),
                        )
                    });
                    assert!(canceled.canceled);
                    assert!(canceled.items.is_empty());
                    assert_eq!(canceled.experience, 0);
                }
            });
            assert_eq!(calls.load(Ordering::Relaxed), 6);
        });
    }
    /// A mod block entity's part answers its removal as vanilla's `preRemoveSideEffects`
    /// override does: found by the entity's type, not its block, reading its attachments in
    /// place, its stacks scattered after the block entity's own; skipped under flag 256, and
    /// drawing nothing when it gives nothing back.
    #[test]
    #[ignore = "daily"]
    fn a_block_entity_part_gives_back_what_it_held_when_removed() {
        use mod_api::Holder;
        use mod_platform::__private::part_tests::{HOARD_ASKED, game_thread, hoard_key};
        use std::sync::Arc;
        use std::sync::atomic::Ordering;
        use sunrise_level::chunk::Chunk;
        use sunrise_level::containers::{ModBlockEntity, ModBlockEntityFields};
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let _mods = game_thread();
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            // Each case: the entity's type, its hoard, side effects; the gold that comes out.
            let asked = HOARD_ASKED.load(Ordering::Relaxed);
            for (id, hoard, side_effects, gold) in [
                ("doubler:hoard", 5, true, 5),
                ("doubler:hoard", 5, false, 0),
                ("doubler:hoard", 0, true, 0),
                // A different declared type's removal doesn't reach `Hoard`.
                ("doubler:press_entity", 5, true, 0),
            ] {
                let pos = BlockPos::new(1, 64, 2);
                let key = (pos.x, pos.y, pos.z);
                let entity = BlockEntity::Mod(Box::new(ModBlockEntity::new(
                    id.into(),
                    ModBlockEntityFields::default(),
                )));
                game.world.block_entities.insert(key, entity);
                let holder = Holder::BlockEntity([pos.x, pos.y, pos.z]);
                let host: &mut dyn mod_api::Host = game;
                hoard_key().set(host, holder, hoard).unwrap();
                let (spawns, random) = game.with_level(|_, level, _| {
                    let mut scatter = level.world.random.clone();
                    level.remove_entity(pos, stone, side_effects);
                    // The game's own draws for what came out, from the random as it was.
                    let count = level.spill.spawns.iter().map(|s| s.stack.count).sum();
                    if count > 0 {
                        removal::drop_item_stack(&mut scatter, pos, count);
                    }
                    let spawns: Vec<_> = level
                        .spill
                        .spawns
                        .iter()
                        .map(|s| (s.stack.item.to_string(), s.stack.count))
                        .collect();
                    (spawns, scatter.next_int() == level.world.random.next_int())
                });
                let total: u32 = spawns.iter().map(|(_, count)| count).sum();
                assert!(
                    spawns
                        .iter()
                        .all(|(item, _)| item == "minecraft:gold_ingot")
                );
                assert_eq!(
                    total, gold,
                    "{id} holding {hoard}, side effects {side_effects}"
                );
                assert!(random, "{id}: the random drew only the scatter");
                assert!(game.world.block_entities.get(&key).is_none());
            }
            // Only the two `doubler:hoard` removals with side effects asked it.
            assert_eq!(HOARD_ASKED.load(Ordering::Relaxed) - asked, 2);
        });
    }
    /// LevelChunk ticks the resident row and calls its pre-removal effects before retiring it.
    /// Replacing its own block must spill both the container contents and the Part's gold.
    #[test]
    #[ignore = "daily"]
    fn a_block_entity_removing_itself_in_its_tick_runs_resident_removal() {
        use mod_api::Holder;
        use mod_platform::__private::part_tests::{HOARD_ASKED, hoard_key};
        use std::sync::Arc;
        use std::sync::atomic::Ordering;
        use sunrise_level::chunk::Chunk;
        use sunrise_level::containers::{ModBlockEntity, ModBlockEntityFields, ModContainer};
        let Some(root) = crate::game::native_entity_refs::tests::declared_part_root(
            "cut_layout::game::server_side::spill::tests::a_block_entity_removing_itself_in_its_tick_runs_resident_removal",
        ) else {
            return;
        };
        sunrise_gameplay::game::native_entity_refs::tests::with_game_at(root, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = &game.pack.assets.registry;
            let hoard = registry.default_state("test:hoard").unwrap();
            let granite = registry.default_state("minecraft:granite").unwrap();
            let pos = BlockPos::new(1, 64, 2);
            let key = (pos.x, pos.y, pos.z);
            game.world.set(pos.x, pos.y, pos.z, hoard).unwrap();
            game.world.block_entities.set_mod_containers(
                [(
                    hoard.0,
                    Arc::new(ModContainer {
                        slots: 1,
                        ..Default::default()
                    }),
                )]
                .into_iter()
                .collect(),
            );
            // Data `[1]` makes the hoard's tick set its own block to granite.
            let fields = ModBlockEntityFields {
                items: vec![Some(ItemStack::new("minecraft:diamond", 3))],
                data: vec![1],
                ..Default::default()
            };
            game.world.block_entities.insert(
                key,
                BlockEntity::Mod(Box::new(ModBlockEntity::new(
                    "doubler:hoard".into(),
                    fields,
                ))),
            );
            let holder = Holder::BlockEntity([pos.x, pos.y, pos.z]);
            let host: &mut dyn mod_api::Host = game;
            hoard_key().set(host, holder, 5).unwrap();
            let asked = HOARD_ASKED.load(Ordering::Relaxed);
            let dropped = game.dropped.len();
            assert!(game.tick_mod_block_entity(key));
            assert_eq!(game.world.block(pos.x, pos.y, pos.z), granite);
            assert_eq!(HOARD_ASKED.load(Ordering::Relaxed), asked + 1);
            assert!(game.world.block_entities.get(&key).is_none());
            assert!(game.world.block_entities.attachments(key).is_none());
            let out: Vec<_> = game.dropped[dropped..]
                .iter()
                .map(|d| (d.stack.item.to_string(), d.stack.count))
                .collect();
            assert_eq!(
                out,
                [
                    ("minecraft:diamond".to_owned(), 3),
                    ("minecraft:gold_ingot".to_owned(), 5),
                ]
            );
        });
    }
}

#[cfg(test)]
mod crafter_drop_name_tests {
    use super::*;
    use sunrise_core::nbt::Tag;

    #[test]
    #[ignore = "daily"]
    fn actual_crafter_destruction_keeps_source_name_off_the_dropped_item() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            game.block_ticks
                .add_empty_chunk(BlockPos::new(0, 0, 0).chunk())
                .unwrap();
            assert!(game.run_command("forceload add 0 0"));
            assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
            assert!(
                game.ui
                    .rules
                    .bool_at(sunrise_game_data::gamerules::vanilla::BLOCK_DROPS)
            );
            let clock = game.world.game_time;
            let mut unexpected = Vec::new();
            // Positive name-copy and unnamed controls precede the assigned case.
            for (x, z, kind, name) in [
                (3, 11, "chest", Some("Box")),
                (11, 11, "shulker_box", Some("Box")),
                (11, 3, "crafter", None),
                (3, 3, "crafter", Some("Rig")),
            ] {
                let extra = name.map_or(String::new(), |n| format!(",CustomName:\"{n}\""));
                assert!(game.run_command(&format!(
                "setblock {x} 200 {z} minecraft:{kind}{{Items:[{{Slot:0b,id:\"minecraft:diamond\",count:3,components:{{\"minecraft:custom_data\":{{h212:1}}}}}}]{extra}}} strict"
            )));
                let key = (x, 200, z);
                let item_id = format!("minecraft:{kind}");
                assert_eq!(
                    &*game.res.facts().get(game.world.block(x, 200, z)).name,
                    item_id
                );
                assert_eq!(
                    &*game.res.facts().get(game.world.block(x, 199, z)).name,
                    "minecraft:stone"
                );
                assert_eq!(
                    game.world.block_names.get(&key),
                    name.map(|n| Tag::String(n.into())).as_ref()
                );
                let entity = game
                    .world
                    .block_entities
                    .get(&key)
                    .expect("actual source block entity");
                assert_eq!(matches!(entity, BlockEntity::Crafter(_)), kind == "crafter");
                let diamond = entity.slots()[0].as_ref().expect("actual loaded inventory");
                assert_eq!((&*diamond.item, diamond.count), ("minecraft:diamond", 3));
                assert_eq!(
                    diamond
                        .data()
                        .and_then(|d| d.other("minecraft:custom_data"))
                        .and_then(|t| t.get("h212")),
                    Some(&Tag::Int(1))
                );
                assert!(game.world.loaded(x, z), "actual destruction-level chunk");
                assert!(game.run_command(&format!(
                "execute if data block {x} 200 {z} {{id:\"minecraft:{kind}\",x:{x},y:200,z:{z}}}"
            )));
                if let Some(n) = name {
                    assert!(game.run_command(&format!(
                        "execute if data block {x} 200 {z} {{CustomName:\"{n}\"}}"
                    )));
                } else {
                    assert!(
                        game.run_command(&format!(
                            "execute unless data block {x} 200 {z} CustomName"
                        ))
                    );
                }
                let start = game.dropped.len();
                assert!(game.run_command(&format!("setblock {x} 200 {z} air destroy")));
                game.flush_spill();
                assert!(game.res.facts().is_air(game.world.block(x, 200, z)));
                assert!(!game.world.block_entities.contains_key(&key));
                assert!(!game.world.block_names.contains_key(&key));
                let emitted = &game.dropped[start..];
                let matching: Vec<_> = emitted
                    .iter()
                    .filter(|d| *d.stack.item == item_id)
                    .collect();
                assert_eq!(matching.len(), 1, "one actual block item: {kind}");
                let drop = matching[0];
                assert_eq!(drop.stack.count, 1);
                assert_eq!((drop.age, drop.pickup_delay), (0, 10));
                let actual = drop.stack.data().and_then(|d| d.custom_name.as_ref());
                if kind == "crafter" {
                    if actual.is_some() {
                        unexpected.push((name, actual.cloned()));
                    }
                } else {
                    assert_eq!(
                        actual,
                        Some(&Tag::String("Box".into())),
                        "positive ordinary name-copy control"
                    );
                }
                if kind == "shulker_box" {
                    let contents = &drop.stack.data().unwrap().container;
                    assert_eq!(contents.len(), 1);
                    let kept = &contents[0].1;
                    assert_eq!((&*kept.item, kept.count), ("minecraft:diamond", 3));
                    assert_eq!(
                        kept.data()
                            .and_then(|d| d.other("minecraft:custom_data"))
                            .and_then(|t| t.get("h212")),
                        Some(&Tag::Int(1))
                    );
                    assert_eq!(emitted.len(), 1);
                } else {
                    let contents: Vec<_> = emitted
                        .iter()
                        .filter(|d| &*d.stack.item == "minecraft:diamond")
                        .collect();
                    assert_eq!(contents.len(), 1);
                    assert_eq!(contents[0].stack.count, 3);
                    assert_eq!(
                        contents[0]
                            .stack
                            .data()
                            .and_then(|d| d.other("minecraft:custom_data"))
                            .and_then(|t| t.get("h212")),
                        Some(&Tag::Int(1))
                    );
                    assert_eq!(emitted.len(), 2);
                }
                assert_eq!(game.world.game_time, clock, "zero observation ticks");
            }
            assert!(
                unexpected.is_empty(),
                "actual vanilla crafter drop retained source name: {unexpected:?}"
            );
        });
    }
}
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
