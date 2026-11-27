//! #1666: a content block whose block entity publishes its shapes
//! (`shapes_from_block_entity`), on a game over the prepared resources.
use mod_api::*;
use std::ops::ControlFlow;
use std::sync::Arc;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::GameGameExt1;
use {
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game_at,
};

/// The hit a ray found: `None` for a miss, an unloaded column or an unanswerable shape.
fn found(outcome: Result<ClipOutcome, ClipError>) -> Option<ClipHit> {
    match outcome {
        Ok(ClipOutcome::Hit(hit)) => Some(hit),
        _ => None,
    }
}

pub(super) const BUS: &str = "shapes:bus";
/// A flagged block on a template with a dynamic shape and a random outline offset
/// (`minecraft:bamboo`).
const TUFT: &str = "shapes:tuft";

/// A mod whose blocks [`BUS`] and [`TUFT`] take their shapes from their block entity,
/// and the block entity type that owns both.
struct Bus;
#[mod_api::__subscriptions]
impl Hooks for Bus {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.block_entity_type("bus").blocks(&[BUS, TUFT]).build();
    }
    fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
        for (id, template) in [(BUS, "minecraft:stone"), (TUFT, "minecraft:bamboo")] {
            let mut block = mod_api::Block::new(id, template);
            block.shapes_from_block_entity = true;
            event.register(block).unwrap();
        }
    }
}

/// The mod installed, a game on its content root with a player over chunk (0, 0).
pub(super) fn with_bus(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    let mods: Mods = vec![ModEntry::new("shapes", Box::new(Bus))].into();
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
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(8.5, 70.0, 8.5))
                    .unwrap(),
            );
            game.update_chunk_tickets();
            test(game);
        });
    });
}

/// Places `block` at `pos` in the level in place, with its block entity and no shapes
/// published yet.
pub(super) fn place(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    block: &str,
    pos: [i32; 3],
) -> BlockEntityRef {
    let level = game.level(game.dim.id()).unwrap();
    let state = game.pack.assets.registry.default_state(block).unwrap();
    assert!(
        game.level_set_block(level, pos, state, BlockUpdateFlags::NONE)
            .unwrap()
    );
    game.block_entity(level, pos)
        .expect("the block has its block entity")
}

/// Places [`BUS`] at `pos` in the level in place and publishes `shapes` from its block entity.
fn publish(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    pos: [i32; 3],
    shapes: BlockEntityShapes,
) {
    let entity = place(game, BUS, pos);
    assert_eq!(game.block_entity_set_shapes(entity, Some(shapes)), Ok(true));
}

pub(super) fn shapes(collision: &[[f64; 6]], outline: &[[f64; 6]]) -> BlockEntityShapes {
    BlockEntityShapes {
        collision: collision.to_vec(),
        outline: outline.to_vec(),
    }
}

pub(super) const HALF: [f64; 6] = [0.0, 0.0, 0.0, 1.0, 0.5, 1.0];
const LOW: [f64; 6] = [0.0, 0.0, 0.0, 1.0, 0.25, 1.0];
pub(super) const SMALL: [f64; 6] = [0.25, 0.0, 0.25, 0.75, 0.25, 0.75];
pub(super) const FULL: [f64; 6] = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
const TALL: [f64; 6] = [0.0, 0.0, 0.0, 1.0, 0.75, 1.0];
const THIN: [f64; 6] = [0.45, 0.0, 0.45, 0.55, 0.25, 0.55];

/// Whether this is the parent process, which ran test `name` in a child process
/// (the content root freezes the process's block entity types).
fn fresh(name: &str) -> bool {
    run_fresh_cases(
        &format!("cut_layout::game::published_shape_game_tests::{name}"),
        "SUNRISE_PUBLISHED_SHAPE_CHILD",
        &["run"],
    )
}

/// Two levels hold the block at the same position with different published
/// shapes: each level's query answers with its own, the one in place and the
/// parked one alike (`level_block_shape`, the empty-context shape query).
#[test]
#[ignore = "daily"]
fn each_level_answers_with_its_own_published_shape() {
    if fresh("each_level_answers_with_its_own_published_shape") {
        return;
    }
    with_bus(|game| {
        let pos = [2, 64, 2];
        let state = game.pack.assets.registry.default_state(BUS).unwrap();
        let overworld = game.level(game.dim.id()).unwrap();
        publish(game, pos, shapes(&[HALF], &[SMALL]));
        assert!(game.enter_command_level(crate::game::Dim::Nether).is_some());
        game.world
            .chunks
            .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
        let nether = game.level(game.dim.id()).unwrap();
        publish(game, pos, shapes(&[LOW], &[TALL]));
        for (level, collision, outline) in [(overworld, HALF, SMALL), (nether, LOW, TALL)] {
            assert_eq!(
                game.collect_level_block_shape(level, state, pos, BlockShapeKind::Collision),
                Some(vec![collision])
            );
            assert_eq!(
                game.collect_level_block_shape(level, state, pos, BlockShapeKind::Outline),
                Some(vec![outline])
            );
        }
        assert!(
            game.enter_command_level(crate::game::Dim::Overworld)
                .is_some()
        );
        // Back in the overworld, the nether is the parked level.
        assert_eq!(
            game.collect_level_block_shape(nether, state, pos, BlockShapeKind::Collision),
            Some(vec![LOW])
        );
        assert_eq!(
            game.collect_level_block_shape(nether, state, pos, BlockShapeKind::Outline),
            Some(vec![TALL])
        );
        assert_eq!(
            game.collect_level_block_shape(overworld, state, pos, BlockShapeKind::Collision),
            Some(vec![HALF])
        );
        let bounds = [2.25, 64.125, 2.25, 2.75, 64.375, 2.75];
        for (level, top) in [(nether, 64.25), (overworld, 64.5)] {
            let mut boxes = Vec::new();
            assert_eq!(
                level.visit_block_collisions(game, bounds, &mut |shape| {
                    boxes.extend(shape.boxes());
                    ControlFlow::Continue(())
                }),
                Some(ControlFlow::Continue(()))
            );
            assert_eq!(boxes, [[2.0, 64.0, 2.0, 3.0, top, 3.0]]);
            assert_eq!(level.has_collisions(game, bounds), Some(true));
        }
    });
}

/// The published shape, not the declared full block, decides the entity collision
/// query (a box falling onto it lands on its top), the block collision scan, a
/// collider ray, an outline ray (selection) and support (a half shape holds no
/// full face up, a full one does). Before the first publish and once cleared, the
/// declared full block applies again.
#[test]
#[ignore = "daily"]
fn collision_selection_and_support_read_the_published_shape() {
    if fresh("collision_selection_and_support_read_the_published_shape") {
        return;
    }
    with_bus(|game| {
        let pos = [2, 64, 2];
        let level = game.level(game.dim.id()).unwrap();
        let entity = place(game, BUS, pos);
        let outline_hit = |game: &sunrise_gameplay::game::ServerGame<'_>| {
            found(game.level_clip(
                level,
                [2.5, 66.0, 2.5],
                [2.5, 63.0, 2.5],
                ClipBlock::Outline,
                ClipFluid::None,
                CollisionContext::Empty,
            ))
            .map(|hit| hit.location[1])
        };
        assert_eq!(
            outline_hit(game),
            Some(65.0),
            "declared before the first publish"
        );
        assert_eq!(
            game.block_entity_set_shapes(entity, Some(shapes(&[HALF], &[SMALL]))),
            Ok(true)
        );
        // The entity collision query (`Entity.collide`): a 0.6-wide box falling from 66.
        let mut scene = sunrise_level::collisions::SceneCollisions {
            world: &game.world,
            binding: game.collision,
            subject: None,
        };
        let falling =
            sunrise_core::geometry::Aabb::new(Vec3::new(2.2, 66.0, 2.2), Vec3::new(2.8, 67.8, 2.8));
        // Through the move gate (#2414), as every entity's move resolves.
        let fall = Vec3::new(0.0, -3.0, 0.0);
        let mut gate = sunrise_sim::move_gate::armed(
            &game.loop_context,
            99,
            "minecraft:zombie",
            sunrise_world::entity_move::MoverType::SelfMovement,
            fall,
        );
        let moved =
            sunrise_sim::move_gate::scene_resolve(&mut gate, &mut scene, falling, fall, 0.6, false)
                .unwrap();
        assert_eq!(66.0 + moved.y, 64.5, "it lands on the published top");
        let mut boxes = Vec::new();
        assert_eq!(
            level.visit_block_collisions(game, [2.0, 64.0, 2.0, 3.0, 65.0, 3.0], &mut |shape| {
                boxes.extend(shape.boxes());
                ControlFlow::Continue(())
            }),
            Some(ControlFlow::Continue(()))
        );
        assert_eq!(boxes, [[2.0, 64.0, 2.0, 3.0, 64.5, 3.0]]);
        let down = |game: &sunrise_gameplay::game::ServerGame<'_>, blocks| {
            found(game.level_clip(
                level,
                [2.5, 66.0, 2.5],
                [2.5, 63.0, 2.5],
                blocks,
                ClipFluid::None,
                CollisionContext::Empty,
            ))
            .map(|hit| (hit.miss, hit.block, hit.location))
        };
        assert_eq!(
            down(game, ClipBlock::Collider),
            Some((false, pos, [2.5, 64.5, 2.5]))
        );
        assert_eq!(
            down(game, ClipBlock::Outline),
            Some((false, pos, [2.5, 64.25, 2.5])),
            "selection stops on the published outline"
        );
        assert_eq!(
            game.level_block_face_sturdy(level, pos, BlockFace::Up, SupportType::Full),
            Some(false)
        );
        let entity = game.block_entity(level, pos).unwrap();
        assert_eq!(
            game.block_entity_set_shapes(entity, Some(shapes(&[FULL], &[FULL]))),
            Ok(true)
        );
        assert_eq!(
            game.level_block_face_sturdy(level, pos, BlockFace::Up, SupportType::Full),
            Some(true)
        );
        assert_eq!(game.block_entity_set_shapes(entity, None), Ok(true));
        assert_eq!(outline_hit(game), Some(65.0), "declared again once cleared");
    });
}

/// Pathfinding stands where collision does: `WalkNodeEvaluator.getFloorLevel`
/// is the block below's collision top, so over a published half shape a node
/// stands at 64.5, as a falling entity lands, not on the declared full block.
#[test]
#[ignore = "daily"]
fn pathfinding_stands_on_the_published_collision_top() {
    if fresh("pathfinding_stands_on_the_published_collision_top") {
        return;
    }
    with_bus(|game| {
        let pos = [2, 64, 2];
        let entity = place(game, BUS, pos);
        // Unflagged controls: stone and a vanilla bottom slab keep their own tops.
        let level = game.level(game.dim.id()).unwrap();
        for (block, x) in [("minecraft:stone", 5), ("minecraft:oak_slab", 6)] {
            let state = game.pack.assets.registry.default_state(block).unwrap();
            assert!(
                game.level_set_block(level, [x, 64, 2], state, BlockUpdateFlags::NONE)
                    .unwrap()
            );
        }
        let floor = |game: &sunrise_gameplay::game::ServerGame<'_>, x: i32| {
            game.mob_world_base(&[]).floor_level(x, 65, 2)
        };
        assert_eq!((floor(game, 5), floor(game, 6)), (65.0, 64.5));
        assert_eq!(floor(game, 2), 65.0, "declared before the first publish");
        assert_eq!(
            game.block_entity_set_shapes(entity, Some(shapes(&[HALF], &[SMALL]))),
            Ok(true)
        );
        {
            let world = game.mob_world_base(&[]);
            assert_eq!(world.floor_level(2, 65, 2), 64.5);
            // `getBlockFloorHeight`: the block's own collision top.
            assert_eq!(world.block_floor_height(2, 64, 2), 0.5);
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let mut zombie = sunrise_sim::mobs::Mob::new(50, kind, Vec3::new(2.5, 64.5, 2.5), 0.0);
            let path_mob = zombie.path_mob();
            let finder = sunrise_sim::mobs::path::Finder::new(
                &world,
                &path_mob,
                sunrise_sim::mobs::path::Evaluator::Walk,
            );
            assert_eq!(finder.static_floor(2, 65, 2), 64.5);
            drop(finder);
            // `MoveControl.tick`: standing on the published top, walking on, it doesn't jump.
            zombie.ground.on_ground = true;
            zombie.ai.mov.wanted = [5.5, 64.5, 2.5];
            zombie.ai.mov.speed_modifier = 1.0;
            zombie.ai.mov.operation = sunrise_sim::mobs::control::Operation::MoveTo;
            zombie.move_control_tick(&world, &mut Vec::new());
            assert!(!zombie.ai.jump, "no jump on the published half");
        }
        assert_eq!(game.block_entity_set_shapes(entity, None), Ok(true));
        assert_eq!(floor(game, 2), 65.0, "declared again once cleared");
    });
}

/// A flagged block on `minecraft:bamboo`, whose shape is dynamic and whose outline
/// takes a random offset: once its entity publishes, an outline ray (selection)
/// stops on the published outline moved by that offset, where the game's own
/// outline boxes put it, and a collider ray on the published collision instead of
/// refusing the dynamic shape.
#[test]
#[ignore = "daily"]
fn a_published_shape_stands_over_a_dynamic_template_and_takes_its_offset() {
    if fresh("a_published_shape_stands_over_a_dynamic_template_and_takes_its_offset") {
        return;
    }
    with_bus(|game| {
        let level = game.level(game.dim.id()).unwrap();
        // Before the first publish and once cleared, the dynamic shape is refused again.
        let down = |game: &sunrise_gameplay::game::ServerGame<'_>| {
            game.level_clip(
                level,
                [0.5, 66.0, 9.5],
                [0.5, 63.0, 9.5],
                ClipBlock::Collider,
                ClipFluid::None,
                CollisionContext::Empty,
            )
        };
        let entity = place(game, TUFT, [0, 64, 9]);
        assert!(down(game).is_err(), "refused before the first publish");
        assert_eq!(
            game.block_entity_set_shapes(entity, Some(shapes(&[HALF], &[THIN]))),
            Ok(true)
        );
        assert_eq!(found(down(game)).map(|hit| hit.location[1]), Some(64.5));
        assert_eq!(game.block_entity_set_shapes(entity, None), Ok(true));
        assert!(down(game).is_err(), "refused again once cleared");
        let mut offset_seen = false;
        for x in 0..8 {
            let pos = [x, 64, 6];
            let entity = place(game, TUFT, pos);
            assert_eq!(
                game.block_entity_set_shapes(entity, Some(shapes(&[HALF], &[THIN]))),
                Ok(true)
            );
            let boxes = game.outline_boxes(pos);
            assert_eq!(boxes.len(), 1, "{pos:?}");
            let [x0, _, z0, x1, y1, z1] = boxes[0];
            assert_eq!(y1, 64.25, "{pos:?}: the offset is horizontal only");
            let (cx, cz) = ((x0 + x1) / 2.0, (z0 + z1) / 2.0);
            offset_seen |= (cx - f64::from(x) - 0.5).abs() > 0.06 || (cz - 6.5).abs() > 0.06;
            let hit = found(game.level_clip(
                level,
                [cx, 66.0, cz],
                [cx, 63.0, cz],
                ClipBlock::Outline,
                ClipFluid::None,
                CollisionContext::Empty,
            ))
            .map(|hit| (hit.miss, hit.block, hit.location));
            assert_eq!(
                hit,
                Some((false, pos, [cx, y1, cz])),
                "{pos:?}: the offset outline"
            );
            let collider = found(game.level_clip(
                level,
                [f64::from(x) + 0.5, 66.0, 6.5],
                [f64::from(x) + 0.5, 63.0, 6.5],
                ClipBlock::Collider,
                ClipFluid::None,
                CollisionContext::Empty,
            ))
            .map(|hit| hit.location[1]);
            assert_eq!(collider, Some(64.5), "{pos:?}: the published collision");
        }
        assert!(offset_seen, "some column's outline is offset");
    });
}

/// With no declared offset keys, a bamboo template moves its published outline to the
/// vanilla column, not its published collision. The outline ray only crosses the moved box.
#[test]
#[ignore = "daily"]
fn inherited_published_outline_hits_at_the_template_column() {
    if fresh("inherited_published_outline_hits_at_the_template_column") {
        return;
    }
    with_bus(|game| {
        let pos = [1, 64, 2];
        let entity = place(game, TUFT, pos);
        assert_eq!(
            game.block_entity_set_shapes(entity, Some(shapes(&[HALF], &[THIN]))),
            Ok(true)
        );
        // Vanilla's XZ column offset at (1, 2) is (-0.1166666597, -0.1833333299).
        let x = 1.3833333402872086;
        let z = 2.3166666701436043;
        let level = game.level(game.dim.id()).unwrap();
        let ray = |mode| {
            found(game.level_clip(
                level,
                [x, 66.0, z],
                [x, 63.0, z],
                mode,
                ClipFluid::None,
                CollisionContext::Empty,
            ))
            .map(|hit| (hit.block, hit.location))
        };
        assert_eq!(ray(ClipBlock::Outline), Some((pos, [x, 64.25, z])));
        assert_eq!(ray(ClipBlock::Collider), Some((pos, [x, 64.5, z])));
        let [bounds]: [[f64; 6]; 1] = game.outline_boxes(pos).try_into().unwrap();
        assert!((bounds[0] - 1.3333333402872086).abs() < 1.0e-12);
        assert!((bounds[2] - 2.2666666701436043).abs() < 1.0e-12);
    });
}

/// A publish that a transaction aborts puts the old shape back and tells the neighbours, as the
/// publish did: the water beside the block is scheduled again to read the restored shape.
#[test]
#[ignore = "daily"]
fn an_aborted_publish_restores_the_shape_and_notifies_the_neighbours() {
    if fresh("an_aborted_publish_restores_the_shape_and_notifies_the_neighbours") {
        return;
    }
    with_bus(|game| {
        let pos = [2, 64, 2];
        let state = game.pack.assets.registry.default_state(BUS).unwrap();
        let level = game.level(game.dim.id()).unwrap();
        publish(game, pos, shapes(&[HALF], &[SMALL]));
        let water = game
            .pack
            .assets
            .registry
            .default_state("minecraft:water")
            .unwrap();
        assert!(
            game.level_set_block(level, [3, 64, 2], water, BlockUpdateFlags::NONE)
                .unwrap()
        );
        let entity = game.block_entity(level, pos).unwrap();
        game.storage_scope(|game, _| {
            assert_eq!(
                game.block_entity_set_shapes(entity, Some(shapes(&[LOW], &[TALL]))),
                Ok(true)
            );
            // What the publish scheduled is dropped, so only the abort's notice can schedule again.
            let chunk = sunrise_core::position::ChunkPos::new(0, 0);
            game.block_ticks.remove_chunk(chunk);
            game.block_ticks.add_empty_chunk(chunk).unwrap();
            assert_eq!(game.block_ticks.fluids.count(), 0);
            ((), false)
        });
        assert_eq!(
            game.collect_level_block_shape(level, state, pos, BlockShapeKind::Collision),
            Some(vec![HALF])
        );
        assert!(
            game.block_ticks.fluids.count() > 0,
            "the aborted publish told the water beside it"
        );
    });
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_block_entity_read::NativeBlockEntityReadGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
