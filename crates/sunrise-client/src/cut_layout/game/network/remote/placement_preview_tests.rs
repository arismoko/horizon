#[test]
#[ignore = "daily"]
fn placement_preview_checks_obstruction_without_changing_the_level() {
    run_placement_preview_checks_obstruction_without_changing_the_level();
}

pub(in crate::game) fn run_placement_preview_checks_obstruction_without_changing_the_level() {
    use sunrise_core::{position::BlockPos, random::LegacyRandom};
    use sunrise_gameplay_common::game::remote::dry_level::DryLevel;
    use sunrise_world::{
        level::{Behaviours, LevelAccess, PlaceContext, PlaceOutcome, Ticks, data::load_registry},
        registry::BlockStateId,
    };

    struct Level {
        air: BlockStateId,
        stone: BlockStateId,
        power: BlockStateId,
        client_side: bool,
        obstructed: bool,
        loaded: bool,
        ceiling: i32,
    }
    impl sunrise_world::level::BlockEntityNeighbors for Level {}
    impl LevelAccess for Level {
        fn block_state(&self, pos: BlockPos) -> BlockStateId {
            if pos == BlockPos::new(0, 0, 0) {
                self.stone
            } else if pos == BlockPos::new(-1, 1, 0) {
                self.power
            } else {
                self.air
            }
        }
        fn set_raw(&mut self, _: BlockPos, _: BlockStateId) -> Option<BlockStateId> {
            panic!("preview must not write to the live level");
        }
        fn is_loaded(&self, _: BlockPos) -> bool {
            self.loaded
        }
        fn is_client_side(&self) -> bool {
            self.client_side
        }
        fn max_y(&self) -> i32 {
            self.ceiling
        }
        fn is_unobstructed(&self, pos: BlockPos, _: BlockStateId) -> bool {
            assert_eq!(pos, BlockPos::new(0, 1, 0));
            !self.obstructed
        }
        fn random(&mut self) -> &mut LegacyRandom {
            panic!("preview must not advance the live level's random source");
        }
        fn game_time(&self) -> i64 {
            0
        }
    }

    let resources = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES")
            .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
    );
    let registry = load_registry(&resources).unwrap();
    let behaviours = Behaviours::load(&registry, &resources).unwrap();
    impl sunrise_gameplay_common::game::remote::dry_level::PredictionBase for Level {}

    let mut base = Level {
        air: registry.default_state("minecraft:air").unwrap(),
        stone: registry.default_state("minecraft:stone").unwrap(),
        power: registry.default_state("minecraft:redstone_block").unwrap(),
        client_side: true,
        obstructed: false,
        loaded: true,
        ceiling: 320,
    };
    for item in [
        "minecraft:stone",
        "minecraft:oak_slab",
        "minecraft:oak_door",
        "minecraft:tnt",
        "minecraft:piston",
        "minecraft:sticky_piston",
        // GrowingPlantHeadBlock.getStateForPlacement draws a random age.
        "minecraft:twisting_vines",
    ] {
        let ctx = PlaceContext {
            clicked_pos: BlockPos::new(0, 0, 0),
            face: sunrise_core::geometry::Direction::Up,
            hit: sunrise_core::geometry::Vec3::new(0.5, 1.0, 0.5),
            inside: false,
            source: sunrise_world::level::PlacementSource::Player {
                yaw: 0.0,
                pitch: 45.0,
                sneaking: false,
            },
            item: item.into(),
            can_use_game_master_blocks: false,
        };
        for (client_side, obstructed, loaded, ceiling) in [
            (true, false, true, 320),
            (true, true, true, 320),
            (true, false, false, 320),
            (true, false, true, 1),
            (false, false, true, 320),
        ] {
            base.client_side = client_side;
            base.obstructed = obstructed;
            base.loaded = loaded;
            base.ceiling = ceiling;
            let valid = !obstructed && loaded && ceiling > 1;
            let mut preview = DryLevel::new(&base);
            let mut ticks = Ticks::default();
            let outcome = behaviours.place_block(&mut preview, &mut ticks, &ctx);
            assert_eq!(
                matches!(outcome, PlaceOutcome::Placed { .. }),
                valid,
                "{item}, obstructed={obstructed}, loaded={loaded}, ceiling={ceiling}"
            );
            assert_eq!(preview.changed.is_empty(), !valid);
            if valid
                && matches!(
                    item,
                    "minecraft:tnt" | "minecraft:piston" | "minecraft:sticky_piston"
                )
            {
                assert_eq!(
                    registry
                        .state(preview.block_state(BlockPos::new(0, 1, 0)))
                        .unwrap()
                        .name,
                    if item == "minecraft:tnt" && !client_side {
                        "minecraft:air"
                    } else {
                        item
                    },
                    "powered {item}, client_side={client_side}"
                );
                assert_eq!(
                    ticks.block_events.is_empty(),
                    client_side || item == "minecraft:tnt",
                    "only the server starts a powered piston"
                );
            }
            assert_eq!(base.block_state(BlockPos::new(0, 1, 0)), base.air);
        }
    }
}

#[test]
#[ignore = "daily"]
fn redstone_updates_respect_the_preview_level_side() {
    run_redstone_updates_respect_the_preview_level_side();
}

pub(in crate::game) fn run_redstone_updates_respect_the_preview_level_side() {
    use std::collections::HashMap;
    use sunrise_core::{geometry::Direction, position::BlockPos, random::LegacyRandom};
    use sunrise_world::{
        level::{Behaviours, BlockEntityNeighbors, LevelAccess, Ticks, data::load_registry, flags},
        registry::BlockStateId,
    };
    use {
        sunrise_gameplay_common::game::remote::dry_level::DryLevel,
        sunrise_gameplay_common::game::remote::dry_level::PredictionBase,
    };

    struct Level {
        cells: HashMap<BlockPos, BlockStateId>,
        air: BlockStateId,
        client_side: bool,
    }
    impl BlockEntityNeighbors for Level {}
    impl PredictionBase for Level {}
    impl LevelAccess for Level {
        fn block_state(&self, pos: BlockPos) -> BlockStateId {
            self.cells.get(&pos).copied().unwrap_or(self.air)
        }
        fn set_raw(&mut self, _: BlockPos, _: BlockStateId) -> Option<BlockStateId> {
            panic!("preview must not write to the live level");
        }
        fn is_loaded(&self, _: BlockPos) -> bool {
            true
        }
        fn is_client_side(&self) -> bool {
            self.client_side
        }
        fn random(&mut self) -> &mut LegacyRandom {
            panic!("preview must not advance the live level's random source");
        }
        fn game_time(&self) -> i64 {
            0
        }
    }

    let resources = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES")
            .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
    );
    let registry = load_registry(&resources).unwrap();
    let behaviours = Behaviours::load(&registry, &resources).unwrap();
    let default = |block| registry.default_state(block).unwrap();
    let state = |block, props: &[(&str, &str)]| {
        let mut descriptor = registry.state(default(block)).unwrap().clone();
        for &(key, value) in props {
            descriptor.properties.insert(key.into(), value.into());
        }
        registry.id(&descriptor).unwrap()
    };
    let observer = state("minecraft:observer", &[("powered", "true")]);
    let repeater = state(
        "minecraft:repeater",
        &[("facing", "north"), ("locked", "true")],
    );
    let air = default("minecraft:air");
    let sponge = default("minecraft:sponge");
    let wet_sponge = default("minecraft:wet_sponge");
    let water = default("minecraft:water");
    let stone = default("minecraft:stone");
    let comparator = default("minecraft:comparator");
    let pos = BlockPos::new(4, 1, 4);
    let east = BlockPos::new(5, 1, 4);
    let west = BlockPos::new(3, 1, 4);

    for client_side in [true, false] {
        let base = Level {
            cells: HashMap::new(),
            air,
            client_side,
        };
        let mut dry = DryLevel::new(&base);
        let mut ticks = Ticks::default();
        assert!(behaviours.set_block(
            &mut dry,
            &mut ticks,
            pos,
            observer,
            flags::UPDATE_MOVE_BY_PISTON | flags::UPDATE_KNOWN_SHAPE,
        ));
        let observer_powered = dry.block_state(pos) == observer;

        let repeater_locked = behaviours.update_shape(
            &mut DryLevel::new(&base),
            &mut Ticks::default(),
            repeater,
            pos,
            Direction::West,
            west,
            air,
        ) == repeater;

        let base = Level {
            cells: HashMap::from([(east, water)]),
            ..base
        };
        let mut dry = DryLevel::new(&base);
        assert!(behaviours.set_block(
            &mut dry,
            &mut Ticks::default(),
            pos,
            sponge,
            flags::UPDATE_KNOWN_SHAPE,
        ));
        let after_sponge = (dry.block_state(pos), dry.block_state(east));
        let expected_sponge = if client_side {
            (sponge, water)
        } else {
            (wet_sponge, air)
        };

        let comparator_retained = |old, next, through_conductor| {
            let comparator_pos = if through_conductor {
                BlockPos::new(6, 1, 4)
            } else {
                east
            };
            // With no support, a notified comparator is removed. Put it behind
            // a conductor for the two output scans so the six-way update cannot
            // reach it. Suppress shape updates to isolate notification routes.
            let mut cells = HashMap::from([(pos, old), (comparator_pos, comparator)]);
            if through_conductor {
                cells.insert(east, stone);
            }
            let base = Level {
                cells,
                air,
                client_side,
            };
            let mut dry = DryLevel::new(&base);
            assert!(behaviours.set_block(
                &mut dry,
                &mut Ticks::default(),
                pos,
                next,
                flags::UPDATE_NEIGHBORS | flags::UPDATE_KNOWN_SHAPE,
            ));
            assert_eq!(dry.block_state(pos), next);
            dry.block_state(comparator_pos) == comparator
        };
        let mismatches = [
            ("powered observer", observer_powered == client_side),
            ("sponge placement callback", after_sponge == expected_sponge),
            ("repeater lock", repeater_locked == client_side),
            (
                "plain adjacent notification",
                comparator_retained(air, stone, false) == client_side,
            ),
            (
                "old source removal",
                comparator_retained(default("minecraft:chest"), stone, true) == client_side,
            ),
            (
                "new analog source",
                comparator_retained(air, default("minecraft:cake"), true) == client_side,
            ),
        ]
        .into_iter()
        .filter_map(|(label, matches)| (!matches).then_some(label))
        .collect::<Vec<_>>();
        assert!(
            mismatches.is_empty(),
            "client_side={client_side}: {mismatches:?}"
        );
    }
}
