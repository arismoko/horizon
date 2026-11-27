//! #1710: the mob checks that read a block's collision shape (evoker fangs, the panic
//! goal's water search, iron golem spawning) read the shape its block entity publishes,
//! on the #1666 fixture.
use super::published_shape_game_tests::{BUS, FULL, HALF, SMALL, place, shapes, with_bus};
use mod_api::*;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;
use sunrise_sim::mobs::goals::{Cx, Goal, Panic};
use sunrise_sim::mobs::{Mob, illager, villager};

/// Whether this is the parent process, which ran test `name` in a child process
/// (the content root freezes the process's block entity types).
fn fresh(name: &str) -> bool {
    run_fresh_cases(
        &format!("cut_layout::game::published_collision_mob_tests::{name}"),
        "SUNRISE_PUBLISHED_COLLISION_MOB_CHILD",
        &["run"],
    )
}

/// A box below the block, ending at its bottom: not empty, with a `max(Y)` of 0.
const SUNK: [f64; 6] = [0.0, -0.5, 0.0, 1.0, 0.0, 1.0];

fn set(game: &mut sunrise_gameplay::game::ServerGame<'_>, block: &str, pos: [i32; 3]) {
    let level = game.level(game.dim.id()).unwrap();
    let state = game.pack.assets.registry.default_state(block).unwrap();
    assert!(
        game.level_set_block(level, pos, state, BlockUpdateFlags::NONE)
            .unwrap()
    );
}

/// `EvokerAttackSpellGoal.createSpellEntity`: over a sturdy floor, a fang stands on
/// `getCollisionShape(level, pos).max(Y)` of the block in its way (0 for an empty
/// shape). Over a published half shape that is 64.5, over a published empty one 64.0,
/// where the declared full block gives 65.0.
#[test]
#[ignore = "daily"]
fn evoker_fangs_stand_on_the_published_collision_top() {
    if fresh("evoker_fangs_stand_on_the_published_collision_top") {
        return;
    }
    with_bus(|game| {
        for x in [2, 5, 6] {
            set(game, "minecraft:stone", [x, 63, 2]);
        }
        // Unflagged controls: stone and a vanilla bottom slab keep their own tops.
        set(game, "minecraft:stone", [5, 64, 2]);
        set(game, "minecraft:oak_slab", [6, 64, 2]);
        let entity = place(game, BUS, [2, 64, 2]);
        // From 64.5 down: the stone at 63 is the first sturdy floor, so the fang
        // stands in the block at 64.
        let fang = |game: &sunrise_gameplay::game::ServerGame<'_>, x: f64| {
            illager::fang_floor(&game.mob_world_base(&[]), x, 2.5, 64.0, 64.5)
        };
        assert_eq!((fang(game, 5.5), fang(game, 6.5)), (Some(65.0), Some(64.5)));
        assert_eq!(
            fang(game, 2.5),
            Some(65.0),
            "declared before the first publish"
        );
        assert_eq!(
            game.block_entity_set_shapes(entity, Some(shapes(&[HALF], &[SMALL]))),
            Ok(true)
        );
        assert_eq!(fang(game, 2.5), Some(64.5), "on the published half");
        assert_eq!(
            game.block_entity_set_shapes(entity, Some(shapes(&[], &[SMALL]))),
            Ok(true)
        );
        assert_eq!(fang(game, 2.5), Some(64.0), "an empty published shape");
        assert_eq!(game.block_entity_set_shapes(entity, None), Ok(true));
        assert_eq!(fang(game, 2.5), Some(65.0), "declared again once cleared");
    });
}

/// `PanicGoal.lookForWater` (a burning mob, range 5): it searches for water only when
/// `getCollisionShape(level, blockPosition())` is empty. A mob in a block whose entity
/// publishes an empty collision heads for the water two blocks away; in the declared
/// full block, a published half or a published box ending at the block's bottom it doesn't
/// look.
#[test]
#[ignore = "daily"]
fn the_panic_goal_looks_for_water_in_a_published_empty_shape() {
    if fresh("the_panic_goal_looks_for_water_in_a_published_empty_shape") {
        return;
    }
    with_bus(|game| {
        set(game, "minecraft:stone", [2, 63, 2]);
        set(game, "minecraft:water", [4, 64, 2]);
        let entity = place(game, BUS, [2, 64, 2]);
        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        // The goal after `canUse`, which keeps the position it chose.
        let panic = |game: &sunrise_gameplay::game::ServerGame<'_>| {
            let mut pig = Mob::new(60, kind.clone(), Vec3::new(2.5, 64.0, 2.5), 0.0);
            pig.fire_ticks = 20;
            let mut goal = Panic::new(1.0);
            let mut test_level = sunrise_sim::health::TestLevel::new(0);
            let w = game.mob_world_base(&[]);
            let mut cx = Cx {
                w: &w,
                level: &mut test_level.level(),
                events: &mut Vec::new(),
            };
            let used = goal.can_use(&mut pig, &mut cx);
            (used, goal.target())
        };
        // The water two blocks away: the water block's position (its min corner, as
        // `lookForWater` sets it).
        const TO_WATER: [f64; 3] = [4.0, 64.0, 2.0];
        assert_ne!(
            panic(game).1,
            TO_WATER,
            "the declared block doesn't look for water"
        );
        assert_eq!(
            game.block_entity_set_shapes(entity, Some(shapes(&[], &[SMALL]))),
            Ok(true)
        );
        assert_eq!(panic(game), (true, TO_WATER));
        assert_eq!(
            game.block_entity_set_shapes(entity, Some(shapes(&[HALF], &[SMALL]))),
            Ok(true)
        );
        assert_ne!(panic(game).1, TO_WATER, "a published half isn't empty");
        // Not empty either, though its top is the block's bottom (`max(Y)` 0).
        assert_eq!(
            game.block_entity_set_shapes(entity, Some(shapes(&[SUNK], &[SMALL]))),
            Ok(true)
        );
        assert_ne!(
            panic(game).1,
            TO_WATER,
            "a published box ending at y 0 isn't empty"
        );
    });
}

/// `IronGolem.checkSpawnObstruction` → `entityCanStandOn`: `Block.isFaceFull` of the
/// floor's collision shape, up. On a floor of blocks that publish a half shape, no
/// golem fits; once they publish a full block, one does, as on the declared block.
#[test]
#[ignore = "daily"]
fn an_iron_golem_stands_only_on_a_full_published_top() {
    if fresh("an_iron_golem_stands_only_on_a_full_published_top") {
        return;
    }
    with_bus(|game| {
        // Every column `SpawnUtil.trySpawnMob` draws from (8, 65, 8) inside chunk (0, 0).
        let entities: Vec<_> = (0..16)
            .flat_map(|x| (0..16).map(move |z| [x, 64, z]))
            .map(|pos| place(game, BUS, pos))
            .collect();
        let summon = |game: &sunrise_gameplay::game::ServerGame<'_>| {
            let mut random = sunrise_core::random::LegacyRandom::new(1710);
            villager::summon_golem(
                &game.mob_world_base(&[]),
                [8, 65, 8],
                &mut random,
                |_, at, _| at.y,
            )
        };
        assert_eq!(
            summon(game),
            Some(65.0),
            "declared before the first publish"
        );
        for &entity in &entities {
            assert_eq!(
                game.block_entity_set_shapes(entity, Some(shapes(&[HALF], &[SMALL]))),
                Ok(true)
            );
        }
        assert_eq!(summon(game), None, "a half top face isn't full");
        for &entity in &entities {
            assert_eq!(
                game.block_entity_set_shapes(entity, Some(shapes(&[FULL], &[FULL]))),
                Ok(true)
            );
        }
        assert_eq!(summon(game), Some(65.0), "a full published top");
    });
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
