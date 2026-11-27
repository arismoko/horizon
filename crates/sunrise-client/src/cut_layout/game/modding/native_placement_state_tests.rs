//! The server placement-state query through the public handle on prepared resources.
use super::native_entity_refs::tests::{run_fresh_cases, with_game_at};
use mod_api::{
    Block, BlockPart, BlockState, BlocksEvent, CanSurviveEvent, FenceGateInWallEvent, Hooks,
    LevelReadError, LevelRef, LevelSide, ModEntry, Mods, PlacementContext, PlacementSource,
    PlacementStateError, Registrar, RegistrationContext, StateForPlacementEvent, WorldEpoch,
};
use std::sync::Arc;
use sunrise_core::{
    geometry::Direction,
    position::{BlockPos, ChunkPos},
};
use sunrise_gameplay_common::game::Dim;
use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
use sunrise_level::chunk::Chunk;

const BLOCK: &str = "placement_read:camouflage";
const POS: BlockPos = BlockPos::new(8, 70, 8);
thread_local! {
    static CAN_SURVIVE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn can_survive_calls() -> usize {
    CAN_SURVIVE_CALLS.with(std::cell::Cell::get)
}

struct PlacementPart;
#[mod_api::sunrise_part]
impl BlockPart for PlacementPart {
    fn can_survive(&self, host: LevelSide<'_>, _: &mut CanSurviveEvent) -> Option<bool> {
        CAN_SURVIVE_CALLS.with(|calls| calls.set(calls.get() + 1));
        let level = match host {
            LevelSide::Client(level) | LevelSide::Server(level) => level,
        };
        let dirt = level.block_state_id("minecraft:dirt")?;
        Some(level.block_state([POS.x + 1, POS.y, POS.z]) == Some(dirt))
    }

    fn state_for_placement(
        &self,
        level: LevelSide<'_>,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockState>> {
        let LevelSide::Server(level) = level else {
            panic!("the query should ask the server part");
        };
        let default = level.block_state_default(event.state.block()).unwrap();
        assert_eq!(event.state, default);
        if matches!(
            event.context.source,
            PlacementSource::Player { sneaking: true, .. }
        ) {
            return Some(None);
        }
        if event.context.inside {
            return Some(Some(mod_api::__private::block_state_from_ordinal(
                default.block(),
                u16::MAX,
            )));
        }
        if let PlacementSource::Directional { direction } = event.context.source {
            assert_eq!(direction, Direction::South);
            return Some(level.block_state_id("minecraft:oak_log[axis=x]"));
        }
        // A real non-replaceable block is clicked, but the query must use the
        // independent destination and replacement flag supplied by the caller.
        assert_eq!(event.context.pos, POS);
        assert_eq!(event.context.hit_pos, POS.offset(2, 0, 0));
        assert!(event.context.replace_clicked);
        assert_eq!(event.context.face, Direction::East);
        assert_eq!(event.context.hit, [10.0, 70.75, 8.0]);
        assert_eq!(
            event.context.source,
            PlacementSource::Player {
                yaw: -90.0,
                pitch: 35.0,
                sneaking: false,
            }
        );
        assert_eq!(
            event.context.item,
            mod_api::__private::item_key("minecraft:oak_log")
        );
        assert_eq!(level.has_neighbor_signal([POS.x, POS.y, POS.z]), Some(true));
        Some(level.block_state_id("minecraft:oak_log[axis=z]"))
    }

    fn fence_gate_in_wall(&self, _: LevelSide<'_>, _: &mut FenceGateInWallEvent) -> Option<bool> {
        Some(true)
    }
}

#[test]
#[ignore = "daily"]
fn can_survive_handle_refuses_reentry_and_full_guard_without_dispatching_parts() {
    with_game(
        "can_survive_handle_refuses_reentry_and_full_guard_without_dispatching_parts",
        |game| {
            use sunrise_gameplay_common::game::TryIntoApi;
            use sunrise_gameplay_common::game::native_hooks::enter_survival_query;

            let level = game.level(game.dim.id()).unwrap();
            let candidate = game
                .block_state_default(game.block_key(BLOCK).unwrap())
                .unwrap();
            let candidate_api = candidate.try_into_api().unwrap();
            let before = can_survive_calls();

            let same_query =
                enter_survival_query(&game.world, POS, candidate).expect("outer query guard");
            assert_eq!(
                level.can_survive(game, [POS.x, POS.y, POS.z], candidate_api),
                Err(LevelReadError::Unavailable),
                "same handle query cannot re-enter its active candidate"
            );
            assert_eq!(can_survive_calls(), before);
            drop(same_query);

            let mut nested = Vec::new();
            for x in 1_000..1_100 {
                let Some(guard) =
                    enter_survival_query(&game.world, BlockPos::new(x, 64, 0), candidate)
                else {
                    break;
                };
                nested.push(guard);
            }
            assert!(!nested.is_empty());
            assert_eq!(
                level.can_survive(game, [POS.x, POS.y, POS.z], candidate_api),
                Err(LevelReadError::Unavailable),
                "the API refuses a query when no callback frame remains"
            );
            assert_eq!(can_survive_calls(), before);
            while nested.pop().is_some() {}
        },
    );
}

struct PlacementReadMod;
#[mod_api::__subscriptions]
impl Hooks for PlacementReadMod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let block = r.block_key(BLOCK);
        r.block_part(block, PlacementPart);
    }

    fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
        event
            .register(Block::new(BLOCK, "minecraft:stone"))
            .unwrap();
    }
}

fn with_game(name: &str, test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    if run_fresh_cases(
        &format!("cut_layout::game::server_side::native_placement_state_tests::{name}"),
        "SUNRISE_PLACEMENT_STATE_CHILD",
        &["run"],
    ) {
        return;
    }
    let mods: Mods = vec![ModEntry::new("placement_read", Box::new(PlacementReadMod))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_api::__private::mark_game_thread();
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root =
            sunrise_gameplay_common::game::mods::content_root(&base, &Default::default()).unwrap();
        with_game_at(root.prepared().to_path_buf(), |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            test(game);
        });
    });
}

#[test]
#[ignore = "daily"]
fn vanilla_candidates_use_the_resolved_context_without_placing() {
    with_game(
        "vanilla_candidates_use_the_resolved_context_without_placing",
        |game| {
            use sunrise_gameplay_common::game::api_values::TryIntoEngine;
            let level = game.level(game.dim.id()).unwrap();
            let mut context = PlacementContext::new(POS, Direction::East, None);
            context.source = PlacementSource::Player {
                yaw: -90.0,
                pitch: 0.0,
                sneaking: false,
            };
            context.hit[1] += 0.75;
            let registry = game.pack.assets.registry;
            // A read must not change the destination or its neighbours, or enqueue the dry coral's
            // death tick and spend the random draw its placement rule would make.
            let occupied = POS.offset(1, 0, 0);
            game.world
                .set(
                    occupied.x,
                    occupied.y,
                    occupied.z,
                    registry.default_state("minecraft:stone").unwrap(),
                )
                .unwrap();
            let blocks = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                std::iter::once(POS)
                    .chain(Direction::ALL.map(|face| {
                        let [x, y, z] = face.normal();
                        POS.offset(x, y, z)
                    }))
                    .map(|p| game.world.block(p.x, p.y, p.z))
                    .collect::<Vec<_>>()
            };
            let before = blocks(game);
            let rng = game.world.random.state();
            let ticks = game.block_ticks.blocks.count();
            for (block, key, expected) in [
                ("minecraft:oak_stairs", "facing", "east"),
                ("minecraft:oak_log", "axis", "x"),
                ("minecraft:oak_slab", "type", "top"),
                // `BlockItem.place` would refuse this unsupported door: this read does not.
                ("minecraft:oak_door", "half", "lower"),
            ] {
                let block_key = game.block_key(block).unwrap();
                let state = level
                    .state_for_placement(game, block_key, &context)
                    .unwrap()
                    .unwrap();
                let state = registry.state(state.try_into_engine(&()).unwrap()).unwrap();
                assert_eq!(state.name, block);
                assert_eq!(state.properties[key], expected, "{block}");
            }
            let coral = game.block_key("minecraft:tube_coral").unwrap();
            assert!(matches!(
                level.state_for_placement(game, coral, &context),
                Ok(Some(_))
            ));
            assert_eq!(blocks(game), before);
            assert_eq!(game.world.random.state(), rng);
            assert_eq!(game.block_ticks.blocks.count(), ticks);
        },
    );
}

#[test]
#[ignore = "daily"]
fn placement_read_leaves_queued_spill_untouched() {
    with_game("placement_read_leaves_queued_spill_untouched", |game| {
        let level = game.level(game.dim.id()).unwrap();
        let block = game.block_key("minecraft:oak_log").unwrap();
        let context = PlacementContext::new(POS, Direction::Up, None);
        let stack = sunrise_items::items::ItemStack::new("minecraft:stick", 3);
        let pos = [8.5, 70.5, 8.5];
        let motion = Some([0.0, 0.25, 0.0]);
        let delay = 10;
        game.server
            .spill
            .0
            .spawns
            .push(sunrise_gameplay_common::game::spill_common::Spawn {
                random: None,
                stack: stack.clone(),
                pos,
                motion,
                delay,
            });
        let dropped = game.dropped.len();
        assert!(matches!(
            level.state_for_placement(game, block, &context),
            Ok(Some(_))
        ));
        assert_eq!(game.dropped.len(), dropped);
        let [queued] = game.server.spill.0.spawns.as_slice() else {
            panic!("the spill must still contain the original item");
        };
        assert_eq!(queued.stack, stack);
        assert_eq!(queued.pos, pos);
        assert_eq!(queued.motion, motion);
        assert_eq!(queued.delay, delay);
        assert!(queued.random.is_none());
    });
}

#[test]
#[ignore = "daily"]
fn placement_refusals_are_typed() {
    with_game("placement_refusals_are_typed", |game| {
        let level = game.level(game.dim.id()).unwrap();
        let mut context = PlacementContext::new(POS, Direction::Up, None);
        // No surface supports a vine in this empty column.
        let vine = game.block_key("minecraft:vine").unwrap();
        assert_eq!(level.state_for_placement(game, vine, &context), Ok(None));
        let mod_block = game.block_key(BLOCK).unwrap();
        context.source = PlacementSource::Player {
            yaw: 0.0,
            pitch: 0.0,
            sneaking: true,
        };
        assert_eq!(
            level.state_for_placement(game, mod_block, &context),
            Ok(None)
        );
        context.source = PlacementSource::Player {
            yaw: 0.0,
            pitch: 0.0,
            sneaking: false,
        };
        context.inside = true;
        assert_eq!(
            level.state_for_placement(game, mod_block, &context),
            Ok(game.block_state_default(mod_block).map(|state| {
                sunrise_gameplay_common::game::api_values::TryIntoApi::try_into_api(&state).unwrap()
            }))
        );

        for (hit_y, yaw, pitch) in [
            (f64::NAN, 0.0, 0.0),
            (70.0, f32::INFINITY, 0.0),
            (70.0, 0.0, f32::NEG_INFINITY),
        ] {
            let mut invalid = context.clone();
            invalid.hit[1] = hit_y;
            invalid.source = PlacementSource::Player {
                yaw,
                pitch,
                sneaking: false,
            };
            assert_eq!(
                level.state_for_placement(game, mod_block, &invalid),
                Ok(None)
            );
        }

        let stone = game.block_key("minecraft:stone").unwrap();
        let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
        let closed = LevelRef::new(level.epoch(), mod_api::DimensionKey::NETHER);
        for invalid in [stale, closed] {
            assert_eq!(
                invalid.state_for_placement(game, stone, &context),
                Err(PlacementStateError::Stale)
            );
        }
        let unknown = mod_api::__private::parts_installation::block_key(u32::MAX);
        assert_eq!(level.state_for_placement(game, unknown, &context), Ok(None));
        context.item = Some(mod_api::__private::parts_installation::item_key(u32::MAX));
        assert_eq!(level.state_for_placement(game, stone, &context), Ok(None));
        context.item = None;
        for pos in [POS.offset(0, 1_000, 0), POS.offset(16, 0, 0)] {
            context.pos = pos;
            assert_eq!(level.state_for_placement(game, stone, &context), Ok(None));
        }
        assert_eq!(game.world.chunks.len(), 1);
        context.pos = POS;
        assert!(matches!(
            level.state_for_placement(game, stone, &context),
            Ok(Some(_))
        ));
    });
}

#[test]
#[ignore = "daily"]
fn can_survive_reads_unplaced_states_and_dispatches_parts_without_writing() {
    with_game(
        "can_survive_reads_unplaced_states_and_dispatches_parts_without_writing",
        |game| {
            use sunrise_gameplay_common::game::api_values::TryIntoApi;

            let level = game.level(game.dim.id()).unwrap();
            let registry = game.pack.assets.registry;
            let sapling = game
                .block_state_default(game.block_key("minecraft:oak_sapling").unwrap())
                .unwrap()
                .try_into_api()
                .unwrap();
            let camouflage = game
                .block_state_default(game.block_key(BLOCK).unwrap())
                .unwrap()
                .try_into_api()
                .unwrap();
            let support = POS.offset(0, -1, 0);
            let snapshot = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                (
                    game.world.block(POS.x, POS.y, POS.z),
                    game.world.block(support.x, support.y, support.z),
                    game.world.random.state(),
                    game.block_ticks.blocks.count(),
                )
            };

            for (support_block, expected) in [("minecraft:dirt", true), ("minecraft:stone", false)]
            {
                game.world
                    .set(
                        support.x,
                        support.y,
                        support.z,
                        registry.default_state(support_block).unwrap(),
                    )
                    .unwrap();
                let before = snapshot(game);
                assert_eq!(
                    level.can_survive(game, [POS.x, POS.y, POS.z], sapling),
                    Ok(expected),
                    "support: {support_block}"
                );
                assert_eq!(snapshot(game), before, "support: {support_block}");
            }

            // This block inherits stone's vanilla rule, but its Part can replace that answer.
            assert_eq!(
                level.can_survive(game, [POS.x, POS.y, POS.z], camouflage),
                Ok(false)
            );
            assert!(
                game.res
                    .behaviours()
                    .is_air(game.world.block(POS.x, POS.y, POS.z))
            );
        },
    );
}

#[test]
#[ignore = "daily"]
fn can_survive_uses_the_parked_level_for_vanilla_and_part_reads() {
    with_game(
        "can_survive_uses_the_parked_level_for_vanilla_and_part_reads",
        |game| {
            use sunrise_gameplay_common::game::api_values::TryIntoApi;
            use sunrise_items::items::ItemStack;
            use sunrise_level::containers::BlockEntity;

            let overworld = game.level("minecraft:overworld").unwrap();
            let registry = game.pack.assets.registry;
            let dirt = registry.default_state("minecraft:dirt").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            let chest = registry.default_state("minecraft:chest").unwrap();
            let sapling = game
                .block_state_default(game.block_key("minecraft:oak_sapling").unwrap())
                .unwrap()
                .try_into_api()
                .unwrap();
            let camouflage = game
                .block_state_default(game.block_key(BLOCK).unwrap())
                .unwrap()
                .try_into_api()
                .unwrap();

            game.world.set(POS.x, POS.y - 1, POS.z, dirt).unwrap();
            game.world.set(POS.x + 1, POS.y, POS.z, stone).unwrap();
            game.enter_level(Dim::Nether).unwrap();
            let nether = game.level("minecraft:the_nether").unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            game.world.set(POS.x, POS.y - 1, POS.z, stone).unwrap();
            game.world.set(POS.x + 1, POS.y, POS.z, dirt).unwrap();
            let chest_pos = (POS.x + 2, POS.y, POS.z);
            game.world
                .set(chest_pos.0, chest_pos.1, chest_pos.2, chest)
                .unwrap();
            let mut chest_slots = vec![None; 27];
            chest_slots[0] = Some(ItemStack::new("minecraft:stone", 3));
            game.world
                .block_entities
                .insert(chest_pos, BlockEntity::Chest(chest_slots));
            game.enter_level(Dim::Overworld).unwrap();

            assert_eq!(
                nether.can_survive(game, [POS.x, POS.y, POS.z], sapling),
                Ok(false),
                "vanilla support comes from the parked Nether"
            );
            assert_eq!(
                nether.can_survive(game, [POS.x, POS.y, POS.z], camouflage),
                Ok(true),
                "the Part reads its neighbour from the parked Nether"
            );
            assert_eq!(game.dim, Dim::Overworld, "the query does not switch levels");
            assert_eq!(
                game.world.block(POS.x, POS.y - 1, POS.z),
                dirt,
                "the active Overworld remains unchanged"
            );
            assert_eq!(overworld.dimension_id(), "minecraft:overworld");
            assert_eq!(nether.dimension_id(), "minecraft:the_nether");
            let parked = game.server.levels.0.parked_world(Dim::Nether).unwrap();
            let read = sunrise_gameplay_common::game::native_level_signals::ReadLevel::new(
                game, nether, parked,
            );
            let side = mod_api::BlockSide {
                level: nether,
                pos: [chest_pos.0, chest_pos.1, chest_pos.2],
                side: None,
            };
            assert_eq!(
                mod_api::LevelHost::container_size(&read, side),
                Some(27),
                "container size follows the parked level handle"
            );
            let mut stack_available = false;
            mod_api::LevelHost::lend_container_stack(&read, side, 0, &mut |_| {
                stack_available = true;
            });
            assert!(
                stack_available,
                "container stacks follow the parked level handle"
            );
        },
    );
}

#[test]
#[ignore = "daily"]
fn crop_survival_uses_block_light_when_sky_light_is_zero() {
    with_game(
        "crop_survival_uses_block_light_when_sky_light_is_zero",
        |game| {
            use sunrise_gameplay_common::game::api_values::TryIntoApi;
            use sunrise_level::chunk::Light;

            let level = game.level(game.dim.id()).unwrap();
            let wheat = game
                .block_state_default(game.block_key("minecraft:wheat").unwrap())
                .unwrap()
                .try_into_api()
                .unwrap();
            let farmland = game
                .pack
                .assets
                .registry
                .default_state("minecraft:farmland")
                .unwrap();
            game.world.set(POS.x, POS.y - 1, POS.z, farmland).unwrap();
            let section = ((POS.y - game.world.shape.min_y) >> 4) as usize;
            let chunk = Arc::make_mut(game.world.chunks.get_mut(&(0, 0)).unwrap());
            Arc::make_mut(&mut chunk.sections[section]).light = Light::Uniform(0x08);

            assert_eq!(game.world.light(POS.x, POS.y, POS.z) >> 4, 0);
            assert_eq!(game.world.light(POS.x, POS.y, POS.z) & 15, 8);
            assert_eq!(
                level.can_survive(game, [POS.x, POS.y, POS.z], wheat),
                Ok(true),
                "crop survival uses max(sky light, block light)"
            );
            assert_eq!(level.dimension_id(), game.dim.id());
        },
    );
}

#[test]
#[ignore = "daily"]
fn can_survive_refuses_invalid_reads_and_preserves_transaction_writes() {
    with_game(
        "can_survive_refuses_invalid_reads_and_preserves_transaction_writes",
        |game| {
            use sunrise_gameplay_common::game::api_values::TryIntoApi;

            let level = game.level(game.dim.id()).unwrap();
            let registry = game.pack.assets.registry;
            let sapling_block = game.block_key("minecraft:oak_sapling").unwrap();
            let sapling = game
                .block_state_default(sapling_block)
                .unwrap()
                .try_into_api()
                .unwrap();
            let invalid = mod_api::__private::block_state_from_ordinal(sapling.block(), u16::MAX);
            assert_eq!(
                level.can_survive(game, [POS.x, POS.y, POS.z], invalid),
                Err(LevelReadError::Unavailable)
            );
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            assert_eq!(
                stale.can_survive(game, [POS.x, POS.y, POS.z], sapling),
                Err(LevelReadError::Stale)
            );
            for pos in [
                [16, POS.y, POS.z],
                [30_000_000, POS.y, POS.z],
                [POS.x, i32::MAX, POS.z],
            ] {
                assert_eq!(
                    level.can_survive(game, pos, sapling),
                    Err(LevelReadError::Unavailable),
                    "position {pos:?}"
                );
            }
            let support = POS.offset(0, -1, 0);
            let old_support = game.world.block(support.x, support.y, support.z);
            let dirt = registry.default_state("minecraft:dirt").unwrap();
            let dirt = dirt.try_into_api().unwrap();
            let writes = game.storage_scope(|game, _| {
                assert_eq!(
                    level.set_block(
                        game,
                        [support.x, support.y, support.z],
                        dirt,
                        mod_api::BlockUpdateFlags::NONE,
                    ),
                    Ok(true)
                );
                let (writes, tick_count, random) = (
                    game.transactions.writes(),
                    game.block_ticks.blocks.count(),
                    game.world.random.state(),
                );
                let candidate_before = game.world.block(POS.x, POS.y, POS.z);
                assert_eq!(
                    level.can_survive(game, [POS.x, POS.y, POS.z], sapling),
                    Ok(true),
                    "the query sees the caller's tentative support block"
                );
                assert_eq!(game.transactions.writes(), writes);
                assert_eq!(game.block_ticks.blocks.count(), tick_count);
                assert_eq!(game.world.random.state(), random);
                assert_eq!(game.world.block(POS.x, POS.y, POS.z), candidate_before);
                (writes, false)
            });
            assert_eq!(
                game.world.block(support.x, support.y, support.z),
                old_support
            );
            assert_eq!(game.transactions.writes(), writes);
        },
    );
}

#[test]
#[ignore = "daily"]
fn another_blocks_state_is_returned_only_from_the_level_in_play() {
    with_game(
        "another_blocks_state_is_returned_only_from_the_level_in_play",
        |game| {
            use sunrise_gameplay_common::game::api_values::TryIntoApi;
            let level = game.level(game.dim.id()).unwrap();
            let registry = game.pack.assets.registry;
            let log = game
                .block_state_id("minecraft:oak_log[axis=z]")
                .unwrap()
                .try_into_api()
                .unwrap();
            let mod_block = game.block_key(BLOCK).unwrap();
            let mut context = PlacementContext::new(
                POS,
                Direction::East,
                mod_api::__private::item_key("minecraft:oak_log"),
            );
            context.hit_pos = POS.offset(2, 0, 0);
            context.replace_clicked = true;
            context.hit = [10.0, 70.75, 8.0];
            context.source = PlacementSource::Player {
                yaw: -90.0,
                pitch: 35.0,
                sneaking: false,
            };
            game.world
                .set(
                    10,
                    70,
                    8,
                    registry.default_state("minecraft:stone").unwrap(),
                )
                .unwrap();
            let comparator = game
                .block_state_id("minecraft:comparator[facing=west,powered=true]")
                .unwrap();
            game.world.set(7, 70, 8, comparator).unwrap();
            game.block_ticks
                .comparator_outputs
                .insert(POS.offset(-1, 0, 0), 8);
            assert_eq!(
                level.state_for_placement(game, mod_block, &context),
                Ok(Some(log))
            );
            // A directional query reaches the part with `PlacementSource::Directional`.
            let directional =
                PlacementContext::directional(POS, Direction::South, Direction::Up, None);
            let log_x = game
                .block_state_id("minecraft:oak_log[axis=x]")
                .unwrap()
                .try_into_api()
                .unwrap();
            assert_eq!(
                level.state_for_placement(game, mod_block, &directional),
                Ok(Some(log_x))
            );
            let door = game.block_key("minecraft:oak_door").unwrap();
            let state = level
                .state_for_placement(game, door, &context)
                .unwrap()
                .unwrap();
            use sunrise_gameplay_common::game::api_values::TryIntoEngine;
            let state = registry.state(state.try_into_engine(&()).unwrap()).unwrap();
            assert_eq!(state.properties["powered"], "true");

            // The vanilla candidate can also depend on a neighbour's Part.
            let neighbour = game.block_state_default(mod_block).unwrap();
            game.world.set(8, 70, 9, neighbour).unwrap();
            let gate = game.block_key("minecraft:oak_fence_gate").unwrap();
            let state = level
                .state_for_placement(game, gate, &context)
                .unwrap()
                .unwrap();
            let state = registry.state(state.try_into_engine(&()).unwrap()).unwrap();
            assert_eq!(state.properties["in_wall"], "true");

            assert!(game.enter_command_level(Dim::Nether).is_some());
            assert_eq!(
                level.block(game, [10, 70, 8]),
                game.block_key("minecraft:stone")
            );
            assert_eq!(
                level.state_for_placement(game, mod_block, &context),
                Err(PlacementStateError::Unavailable)
            );
            assert_eq!(game.dim, Dim::Nether, "the query never swaps the level in");

            // The column layout has padding above the Nether's last buildable Y = 255.
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let nether = game.level(Dim::Nether.id()).unwrap();
            let log = game.block_key("minecraft:oak_log").unwrap();
            let almost_top = PlacementContext::new(BlockPos::new(8, 254, 8), Direction::Up, None);
            let top = PlacementContext::new(BlockPos::new(8, 255, 8), Direction::Up, None);
            let above_top = PlacementContext::new(BlockPos::new(8, 256, 8), Direction::Up, None);
            assert!(matches!(
                nether.state_for_placement(game, door, &almost_top),
                Ok(Some(_))
            ));
            assert_eq!(nether.state_for_placement(game, door, &top), Ok(None));
            assert!(matches!(
                nether.state_for_placement(game, log, &top),
                Ok(Some(_))
            ));
            assert_eq!(nether.state_for_placement(game, log, &above_top), Ok(None));
        },
    );
}

#[test]
#[ignore = "daily"]
fn directional_queries_read_the_clicked_block_and_refuse_what_vanilla_throws_on() {
    with_game(
        "directional_queries_read_the_clicked_block_and_refuse_what_vanilla_throws_on",
        |game| {
            use sunrise_gameplay_common::game::api_values::TryIntoEngine;
            let level = game.level(game.dim.id()).unwrap();
            let registry = game.pack.assets.registry;
            let air = registry.default_state("minecraft:air").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            let props = |state: BlockState, keys: [&str; 2]| {
                let state = registry.state(state.try_into_engine(&()).unwrap()).unwrap();
                keys.map(|key| state.properties[key].clone())
            };

            // `DirectionalPlaceContext.replacingClickedOnBlock()` is `canPlace()`, asked of the
            // clicked block, not the `true` that `PlacementContext::directional` writes. Into
            // stone the trapdoor hangs on the clicked face, on the click's half; into air it
            // faces away from the direction, on the top half.
            let trapdoor = game.block_key("minecraft:oak_trapdoor").unwrap();
            let east = PlacementContext::directional(
                POS,
                Direction::East,
                Direction::East,
                mod_api::__private::item_key("minecraft:oak_trapdoor"),
            );
            game.world.set(POS.x, POS.y, POS.z, stone).unwrap();
            let into_stone = level
                .state_for_placement(game, trapdoor, &east)
                .unwrap()
                .unwrap();
            assert_eq!(props(into_stone, ["facing", "half"]), ["east", "bottom"]);
            game.world.set(POS.x, POS.y, POS.z, air).unwrap();
            let mut claims_kept = east.clone();
            claims_kept.replace_clicked = false;
            for context in [&east, &claims_kept] {
                let into_air = level
                    .state_for_placement(game, trapdoor, context)
                    .unwrap()
                    .unwrap();
                assert_eq!(props(into_air, ["facing", "half"]), ["west", "top"]);
            }

            // Pointed dripstone reads the placer's vertical look, which a directional context
            // has no player for: refused for every direction, though both supports stand. A
            // player's pitch still picks the tip.
            let dripstone = game.block_key("minecraft:pointed_dripstone").unwrap();
            let item = mod_api::__private::item_key("minecraft:pointed_dripstone");
            game.world.set(POS.x, POS.y + 1, POS.z, stone).unwrap();
            game.world.set(POS.x, POS.y - 1, POS.z, stone).unwrap();
            for direction in Direction::ALL {
                let context = PlacementContext::directional(POS, direction, Direction::Up, item);
                assert_eq!(
                    level.state_for_placement(game, dripstone, &context),
                    Err(PlacementStateError::NoVerticalLook),
                    "{direction:?}"
                );
            }
            let mut player = PlacementContext::new(POS, Direction::Up, item);
            for (pitch, tip) in [(0.0, "up"), (-30.0, "down")] {
                player.source = PlacementSource::Player {
                    yaw: 0.0,
                    pitch,
                    sneaking: false,
                };
                let state = level
                    .state_for_placement(game, dripstone, &player)
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    props(state, ["vertical_direction", "thickness"]),
                    [tip, "tip"]
                );
            }

            // A slab clicked with its own item asks `replacingClickedOnBlock` from its own
            // `canBeReplaced`, which for a directional context is that question again.
            let slab = game.block_key("minecraft:oak_slab").unwrap();
            let bottom = registry.default_state("minecraft:oak_slab").unwrap();
            game.world.set(POS.x, POS.y, POS.z, bottom).unwrap();
            let into_slab = PlacementContext::directional(
                POS,
                Direction::Down,
                Direction::Up,
                mod_api::__private::item_key("minecraft:oak_slab"),
            );
            assert_eq!(
                level.state_for_placement(game, slab, &into_slab),
                Err(PlacementStateError::ReplacingClickedCycle)
            );
        },
    );
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
