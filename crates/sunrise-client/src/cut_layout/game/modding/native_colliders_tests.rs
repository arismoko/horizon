//! Moving colliders on a real game: set, read, refused, saved and loaded back, and solid to a
//! falling item.
use mod_api::{Collider, ColliderBox, ColliderError, ColliderPose, EntityRef};
use std::sync::Arc;
use sunrise_core::{
    geometry::Vec3,
    nbt::{Compound, Tag},
};
use sunrise_gameplay_common::game::Dim;
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::GameGameExt1 as _;
use sunrise_gameplay_common::game::native_collider_state::SAVE_KEY;
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
use sunrise_sim::entities::ItemEntity;

fn deck() -> Collider {
    Collider::new(vec![ColliderBox {
        min: [-1.5, -1.0, -1.5],
        max: [1.5, 0.0, 1.5],
    }])
}

/// A pig with no AI and no gravity at `at`: it stays where it is put.
fn stand(game: &mut sunrise_gameplay::game::ServerGame<'_>, at: [f64; 3]) -> EntityRef {
    stand_with_id(game, at).1
}

fn stand_with_id(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    at: [f64; 3],
) -> (u32, EntityRef) {
    let nbt: Compound = [
        ("NoGravity".into(), Tag::Byte(1)),
        ("NoAI".into(), Tag::Byte(1)),
        ("Silent".into(), Tag::Byte(1)),
    ]
    .into_iter()
    .collect();
    let level = game.level(game.dim.id()).expect("level");
    let entity = game
        .level_summon(level, "minecraft:pig", at, nbt)
        .expect("summon write")
        .expect("a pig");
    (entity.id(), entity)
}

#[test]
#[ignore = "daily"]
fn a_collider_is_set_read_refused_saved_and_loaded() {
    with_world(|game| {
        let entity = stand(game, [1.5, 70.0, 1.5]);
        assert_eq!(entity.collider(game), None);
        assert_eq!(entity.set_collider(game, Some(deck())), Ok(()));
        assert_eq!(entity.collider(game), Some(deck()));

        // Every refusal is by name, and the collider that was set stays.
        let mut bad = deck();
        bad.boxes[0].max[1] = f64::NAN;
        assert_eq!(
            entity.set_collider(game, Some(bad)),
            Err(ColliderError::NonFinite { index: Some(0) })
        );
        let over_boxes = sunrise_protocol::collider_sync::MAX_BOXES_IN_PAYLOAD + 1;
        let mut many = deck();
        many.boxes = vec![many.boxes[0]; over_boxes];
        assert_eq!(
            entity.set_collider(game, Some(many)),
            Err(ColliderError::TooLarge {
                bytes: sunrise_protocol::collider_sync::encoded_bytes(over_boxes) as u64,
                limit: sunrise_protocol::collider_sync::MAX_PAYLOAD_BYTES as u64,
            })
        );
        let zero_q = deck().with_rotation([0.0, 0.0, 0.0, 0.0]);
        assert_eq!(
            entity.set_collider(game, Some(zero_q)),
            Err(ColliderError::NonFinite { index: None })
        );
        let nan_pivot = deck().with_pivot([f64::NAN, 0.0, 0.0]);
        assert_eq!(
            entity.set_collider(game, Some(nan_pivot)),
            Err(ColliderError::NonFinite { index: None })
        );
        assert_eq!(entity.collider(game), Some(deck()));
        let epoch = entity.epoch();
        assert_eq!(
            EntityRef::new(epoch, 999_999).set_collider(game, Some(deck())),
            Err(ColliderError::StaleEntity)
        );

        // The pose alone (rotation normalized when set, plus pivot).
        let half = std::f32::consts::FRAC_1_SQRT_2;
        let pose = ColliderPose::new([0.0, 2.0 * half, 0.0, 2.0 * half], [1.5, 0.0, -0.5]);
        assert_eq!(entity.set_collider_pose(game, pose), Ok(()));
        let got_pose = entity.collider(game).unwrap().pose;
        assert!((got_pose.rotation[1] - half).abs() < 1.0e-6);
        assert!((got_pose.rotation[3] - half).abs() < 1.0e-6);
        assert_eq!(got_pose.pivot, [1.5, 0.0, -0.5]);
        assert_eq!(
            entity.set_collider_pose(
                game,
                ColliderPose::default().with_rotation([f32::NAN, 0.0, 0.0, 1.0]),
            ),
            Err(ColliderError::NonFinite { index: None })
        );
        assert_eq!(
            entity.set_collider_pose(
                game,
                ColliderPose::default().with_rotation([0.0, 0.0, 0.0, 0.0]),
            ),
            Err(ColliderError::NonFinite { index: None })
        );
        let other = stand(game, [10.5, 70.0, 10.5]);
        assert_eq!(
            other.set_collider_pose(game, ColliderPose::default()),
            Err(ColliderError::NoCollider)
        );

        // Saved with the entity, and read back by the entity that loads it.
        let id = entity.id();
        let mut tag = Tag::Compound(Compound::new());
        sunrise_gameplay::game::server_side::persist::write_attachments(
            &mut tag,
            id,
            &game.entity_attachments,
        )
        .unwrap();
        assert!(tag.get(SAVE_KEY).is_some(), "the save holds the collider");
        let want = entity.collider(game);
        assert_eq!(entity.set_collider(game, None), Ok(()));
        assert_eq!(entity.collider(game), None);
        let mut cleared = Tag::Compound(Compound::new());
        sunrise_gameplay::game::server_side::persist::write_attachments(
            &mut cleared,
            id,
            &game.entity_attachments,
        )
        .unwrap();
        assert!(cleared.get(SAVE_KEY).is_none(), "none is not saved");
        game.load_saved_collider(id, &tag);
        assert_eq!(entity.collider(game), want);
        // A malformed save is dropped, and leaves none.
        assert_eq!(entity.set_collider(game, None), Ok(()));
        let broken = Tag::compound([(SAVE_KEY, Tag::compound([("rotation", Tag::Int(1))]))]);
        game.load_saved_collider(id, &broken);
        assert_eq!(entity.collider(game), None);
    });
}

/// A stone dropped at `at`, two blocks above it, in the level in place: its id.
fn drop_item(game: &mut sunrise_gameplay::game::ServerGame<'_>, at: [f64; 3]) -> u32 {
    game.server.next_entity += 1;
    let id = game.server.next_entity;
    game.dropped.push(ItemEntity::new(
        id,
        ItemStack::new("minecraft:stone", 1),
        Vec3::new(at[0], at[1] + 2.0, at[2]),
        Vec3::ZERO,
        0.0,
    ));
    id
}

fn item_y(game: &sunrise_gameplay::game::ServerGame<'_>, id: u32) -> Option<f64> {
    game.dropped
        .iter()
        .find(|i| i.id == id)
        .map(|i| i.position.y)
}

/// An item dropped over a collider rests on its top: the one collision path every mover takes
/// finds the collider, placed at the start of the tick.
#[test]
#[ignore = "daily"]
fn a_dropped_item_rests_on_a_collider() {
    with_world(|game| {
        let entity = stand(game, [1.5, 70.0, 1.5]);
        entity.set_collider(game, Some(deck())).unwrap();
        let held = drop_item(game, [1.5, 70.0, 1.5]);
        for _ in 0..60 {
            tick_level(game);
        }
        let y = item_y(game, held).unwrap();
        assert!(
            (70.0..70.2).contains(&y),
            "the item rests on the top: y = {y}"
        );
        // Without the collider the same drop falls past that height.
        entity.set_collider(game, None).unwrap();
        let falling = drop_item(game, [1.5, 70.0, 1.5]);
        for _ in 0..60 {
            tick_level(game);
        }
        let y = item_y(game, falling).unwrap();
        assert!(y < 69.9, "nothing holds it up: y = {y}");
    });
}

/// Each level ticks with its own colliders: an item over the overworld's collider rests on it,
/// and an item at the same place in the nether, which has none there, falls past it.
#[test]
#[ignore = "daily"]
fn each_level_collides_with_its_own_colliders() {
    with_world(|game| {
        let spot = [1.5, 70.0, 1.5];
        let other = [10.5, 70.0, 10.5];
        let overworld = stand(game, spot);
        overworld.set_collider(game, Some(deck())).unwrap();
        game.enter_level(Dim::Nether).unwrap();
        game.native_level_opened(Dim::Nether);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.server.streaming.forced.insert((0, 0));
        let nether = stand(game, other);
        nether.set_collider(game, Some(deck())).unwrap();
        // The nether's items: one over its own collider, one over the overworld's place.
        let (n_own, n_foreign) = (drop_item(game, other), drop_item(game, spot));
        for _ in 0..60 {
            tick_level(game);
        }
        game.enter_level(Dim::Overworld).unwrap();
        // The overworld's: one over its own collider, one over the nether's place.
        let (o_own, o_foreign) = (drop_item(game, spot), drop_item(game, other));
        for _ in 0..60 {
            tick_level(game);
        }
        let rests = |y: Option<f64>| y.is_some_and(|y| (70.0..70.2).contains(&y));
        assert!(rests(item_y(game, o_own)), "{:?}", item_y(game, o_own));
        assert!(
            item_y(game, o_foreign).is_none_or(|y| y < 69.9),
            "the nether's collider holds no overworld item: {:?}",
            item_y(game, o_foreign)
        );
        game.enter_level(Dim::Nether).unwrap();
        assert!(rests(item_y(game, n_own)), "{:?}", item_y(game, n_own));
        assert!(
            item_y(game, n_foreign).is_none_or(|y| y < 69.9),
            "the overworld's collider holds no nether item: {:?}",
            item_y(game, n_foreign)
        );
        game.enter_level(Dim::Overworld).unwrap();
    });
}

/// A call into the level in place, made while another level's colliders are in scope (a hook of
/// one level reaching into another), reads the level in place's colliders, and the caller's
/// are back afterwards.
#[test]
#[ignore = "daily"]
fn a_call_into_another_level_reads_that_levels_colliders_and_restores_the_callers() {
    use sunrise_core::geometry::Aabb;
    use sunrise_world::entity_obstruction::EntityIdentity;
    with_world(|game| {
        let (spot, other) = ([1.5, 70.0, 1.5], [10.5, 70.0, 10.5]);
        stand(game, spot).set_collider(game, Some(deck())).unwrap();
        game.enter_level(Dim::Nether).unwrap();
        game.native_level_opened(Dim::Nether);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.server.streaming.forced.insert((0, 0));
        stand(game, other).set_collider(game, Some(deck())).unwrap();
        // Each level ticks once, so each has recorded its colliders.
        tick_level(game);
        game.enter_level(Dim::Overworld).unwrap();
        tick_level(game);
        let meets = |at: [f64; 3]| {
            let mut found = Vec::new();
            let bounds = Aabb::new(
                Vec3::new(at[0] - 0.5, 69.2, at[2] - 0.5),
                Vec3::new(at[0] + 0.5, 69.8, at[2] + 0.5),
            );
            sunrise_world::moving_colliders::near(EntityIdentity(0), bounds, &mut found);
            found.len()
        };
        // The overworld's colliders are the caller's scope.
        let placed = game.placed_moving_colliders();
        let _scope =
            sunrise_world::moving_colliders::Scope::enter_level(game.collider_level(), placed);
        assert_eq!((meets(spot), meets(other)), (1, 0));
        game.enter_level(Dim::Nether).unwrap();
        game.in_this_levels_colliders(|_| {
            assert_eq!(
                (meets(spot), meets(other)),
                (0, 1),
                "the level in place's colliders, not the caller's"
            );
        });
        assert_eq!(
            (meets(spot), meets(other)),
            (1, 0),
            "the caller's colliders are back"
        );
        game.enter_level(Dim::Overworld).unwrap();
    });
}

/// A game that simulates its overworld, with the chunk the tests build on loaded.
fn with_world(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    super::native_entity_refs::tests::with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.server.streaming.forced.insert((0, 0));
        game.native_joins_skip();
        test(game);
    });
}

/// One tick of the level in place, as `tick_server` runs it: with the level's colliders placed.
fn tick_level(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    tick_level_moving(game, |_| {});
}

/// [`tick_level`] with a mod moving its colliders during the tick, after the colliders were
/// placed and the movers ran, which is when a mod's own tick moves them.
fn tick_level_moving(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    moving: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
) {
    let placed = game.placed_moving_colliders();
    sunrise_world::moving_colliders::in_level(placed.clone(), || game.tick_world(true).unwrap());
    moving(game);
    // Then the bodies on a collider that moved are carried with it, as the level's tick does.
    game.carry_pass(&placed);
}

/// A collider goes with its entity through a dimension change: the same collider reads back
/// from the arrived entity, and it still holds an item up in the destination.
#[test]
#[ignore = "daily"]
fn a_collider_travels_with_its_entity() {
    with_world(|game| {
        let source = game.dim;
        let (id, entity) = stand_with_id(game, [1.5, 70.0, 1.5]);
        entity.set_collider(game, Some(deck())).unwrap();
        let want = entity.collider(game);
        game.enter_level(Dim::Nether).unwrap();
        game.native_level_opened(Dim::Nether);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.server.streaming.forced.insert((0, 0));
        game.arrive_vehicle(source, id, Vec3::new(1.5, 70.0, 1.5), 0.0);
        for _ in 0..3 {
            tick_level(game);
        }
        let arrived = game.entity(id).expect("it arrived");
        assert_eq!(arrived.collider(game), want, "the collider came along");
        let held = drop_item(game, [1.5, 70.0, 1.5]);
        for _ in 0..60 {
            tick_level(game);
        }
        let y = item_y(game, held).unwrap();
        assert!(
            (70.0..70.2).contains(&y),
            "it still holds an item up: y = {y}"
        );
    });
}

/// The collider's own entity is never stopped by it, and neither is anything in its vehicle
/// stack, ancestors and siblings included; a stranger over the same boxes is.
#[test]
#[ignore = "daily"]
fn a_collider_is_not_solid_to_its_own_vehicle_stack() {
    with_world(|game| {
        let (a, a_ref) = stand_with_id(game, [1.5, 70.0, 1.5]);
        let (b, b_ref) = stand_with_id(game, [1.5, 70.0, 1.5]);
        let (c, c_ref) = stand_with_id(game, [1.5, 70.0, 1.5]);
        let (stranger, _) = stand_with_id(game, [10.5, 70.0, 10.5]);
        // a rides b rides c.
        game.entity_start_riding(a_ref, b_ref, true).unwrap();
        game.entity_start_riding(b_ref, c_ref, true).unwrap();
        // The collider is on the middle of the stack.
        b_ref.set_collider(game, Some(deck())).unwrap();
        for held in [a, b, c] {
            let mut stack = game.vehicle_stack(held);
            stack.sort_unstable();
            assert_eq!(stack, vec![a, b, c], "the stack seen from {held}");
        }
        assert_eq!(game.vehicle_stack(stranger), vec![stranger]);
        // What a mover of each kind meets where the collider stands.
        let placed = game.placed_moving_colliders();
        assert_eq!(placed.len(), 1);
        let mut exempt = placed.exempt(0).unwrap().to_vec();
        exempt.sort_unstable();
        assert_eq!(exempt, vec![a, b, c]);
        let mut stands_on = |id: u32| {
            use sunrise_world::moving_colliders::{in_level, near};
            let mut found = Vec::new();
            in_level(game.placed_moving_colliders(), || {
                near(
                    sunrise_world::entity_obstruction::EntityIdentity(u64::from(id)),
                    sunrise_core::geometry::Aabb::new(
                        Vec3::new(1.0, 69.5, 1.0),
                        Vec3::new(2.0, 71.0, 2.0),
                    ),
                    &mut found,
                );
            });
            !found.is_empty()
        };
        assert!(!stands_on(a) && !stands_on(b) && !stands_on(c));
        assert!(stands_on(stranger), "a stranger meets it");
    });
}

/// An item is a mover that carries its own collider: it moves through the gated collision as
/// a self move, so the boxes under its own feet are not solid to it and it falls through them,
/// where a stranger over the same boxes lands.
#[test]
#[ignore = "daily"]
fn an_item_falls_through_its_own_collider_and_a_stranger_lands_on_it() {
    let deck_under = || {
        Collider::new(vec![ColliderBox {
            min: [-1.0, -2.5, -1.0],
            max: [1.0, -2.0, 1.0],
        }])
    };
    with_world(|game| {
        // A holder that stays where it is, with the deck's top at y = 70, and a stranger over it.
        let fixed = drop_item(game, [1.5, 70.0, 1.5]);
        game.dropped
            .iter_mut()
            .find(|i| i.id == fixed)
            .unwrap()
            .no_gravity = true;
        game.entity(fixed)
            .unwrap()
            .set_collider(game, Some(deck_under()))
            .unwrap();
        let stranger = drop_item(game, [1.2, 71.0, 1.2]);
        // A holder that falls, with the same deck under its own feet.
        let own = drop_item(game, [10.5, 70.0, 10.5]);
        game.entity(own)
            .unwrap()
            .set_collider(game, Some(deck_under()))
            .unwrap();
        for _ in 0..60 {
            tick_level(game);
        }
        let other = item_y(game, stranger).unwrap();
        assert!(
            (70.0..70.3).contains(&other),
            "the stranger stands on the fixed holder's deck: y = {other}"
        );
        // The falling holder took its deck with it and was never stopped by it: with the deck
        // a block and a half under its feet, a stopped holder would rest at y = 72 - 2 + ... 70.
        let fell = item_y(game, own).unwrap_or(f64::NEG_INFINITY);
        assert!(
            fell < 60.0,
            "the holder fell through its own deck: y = {fell}"
        );
    });
}

/// Siblings in one vehicle stack are exempt from each other's collider, and a mover outside
/// the stack is not.
#[test]
#[ignore = "daily"]
fn siblings_riding_one_vehicle_share_a_stack() {
    with_world(|game| {
        let (x, x_ref) = stand_with_id(game, [1.5, 70.0, 1.5]);
        let (y, y_ref) = stand_with_id(game, [1.5, 70.0, 1.5]);
        let (z, z_ref) = stand_with_id(game, [1.5, 70.0, 1.5]);
        let (other, _) = stand_with_id(game, [10.5, 70.0, 10.5]);
        game.entity_start_riding(x_ref, z_ref, true).unwrap();
        game.entity_start_riding(y_ref, z_ref, true).unwrap();
        x_ref.set_collider(game, Some(deck())).unwrap();
        for member in [x, y, z] {
            let mut stack = game.vehicle_stack(member);
            stack.sort_unstable();
            let mut want = vec![x, y, z];
            want.sort_unstable();
            assert_eq!(stack, want, "seen from {member}");
        }
        assert_eq!(game.vehicle_stack(other), vec![other]);
        // What the exemption does: the collision query leaves the siblings and the vehicle
        // out and meets a stranger.
        let placed = game.placed_moving_colliders();
        let meets = |id: u32| {
            let mut found = Vec::new();
            sunrise_world::moving_colliders::in_level(placed.clone(), || {
                sunrise_world::moving_colliders::near(
                    sunrise_world::entity_obstruction::EntityIdentity(u64::from(id)),
                    sunrise_core::geometry::Aabb::new(
                        Vec3::new(1.0, 69.5, 1.0),
                        Vec3::new(2.0, 71.0, 2.0),
                    ),
                    &mut found,
                );
            });
            !found.is_empty()
        };
        assert!(!meets(x) && !meets(y) && !meets(z), "the stack is exempt");
        assert!(meets(other), "a stranger meets it");
    });
}

/// What a level's tick costs with 200 items over a collider at the box cap: with no collider,
/// with one that stays, with one rebuilt every tick but moved before it begins, and with one
/// that moves during the tick and carries all 200 (Create's normal case), each timed over the
/// same ticks. The numbers go to stderr (`--nocapture`); the test holds the outcome: the items
/// rest on the capped collider, moving or not, and go with it when it moves.
#[test]
#[ignore = "daily"]
fn two_hundred_items_over_a_capped_collider_rest_and_the_tick_is_timed() {
    use std::time::Instant;
    const SIDE: usize = 32;
    with_world(|game| {
        let (holder, holder_ref) = stand_with_id(game, [8.0, 70.0, 8.0]);
        let items = |game: &mut sunrise_gameplay::game::ServerGame<'_>| -> Vec<u32> {
            (0..200)
                .map(|i| {
                    let at = [0.5 + (i % 15) as f64, 70.0, 0.5 + (i / 15) as f64];
                    drop_item(game, at)
                })
                .collect()
        };
        let time = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                    ticks: usize,
                    mut each: Option<
            &mut dyn FnMut(&mut sunrise_gameplay::game::ServerGame<'_>),
        >| {
            let start = Instant::now();
            for _ in 0..ticks {
                if let Some(each) = each.as_mut() {
                    each(game);
                }
                tick_level(game);
            }
            start.elapsed().as_secs_f64() * 1000.0 / ticks as f64
        };
        // No collider: the items fall in the open.
        let before = items(game);
        let open = time(game, 40, None);
        for id in &before {
            game.dropped.retain(|i| i.id != *id);
        }
        // A large collider: a 32 x 32 x 32 lump, its top at the holder's feet.
        let boxes = (0..SIDE * SIDE * SIDE)
            .map(|i| {
                let (x, y, z) = (i % SIDE, i / SIDE % SIDE, i / SIDE / SIDE);
                ColliderBox {
                    min: [x as f64 - 16.0, y as f64 - 32.0, z as f64 - 16.0],
                    max: [x as f64 - 15.0, y as f64 - 31.0, z as f64 - 15.0],
                }
            })
            .collect::<Vec<_>>();
        holder_ref
            .set_collider(game, Some(Collider::new(boxes)))
            .unwrap();
        let on = items(game);
        for _ in 0..20 {
            tick_level(game);
        }
        let rests = |game: &sunrise_gameplay::game::ServerGame<'_>| {
            on.iter()
                .filter(|id| item_y(game, **id).is_some_and(|y| (70.0..70.3).contains(&y)))
                .count()
        };
        assert_eq!(rests(game), 200, "every item rests on the capped collider");
        let still = time(game, 40, None);
        assert_eq!(rests(game), 200);
        // The collider moves every tick: its placement is rebuilt each time.
        let index = game.mobs.iter().position(|m| m.id == holder).unwrap();
        let mut step = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
            game.mobs.update(index, |m| m.position.x += 0.001);
        };
        let moving = time(game, 40, Some(&mut step));
        assert_eq!(rests(game), 200, "and they still rest on it while it moves");
        // The collider moves during the tick, after the bodies were placed, so the carry pass
        // carries all 200 items (the phase above moves it before the tick: its pass sees none).
        let from: Vec<f64> = on.iter().map(|id| item_at(game, *id).x).collect();
        let began = Instant::now();
        for _ in 0..40 {
            tick_level_moving(game, |g| {
                g.mobs.update(index, |m| m.position.x += 0.05);
            });
        }
        let carried = began.elapsed().as_secs_f64() * 1000.0 / 40.0;
        assert_eq!(rests(game), 200, "and they are carried with it");
        for (id, x) in on.iter().zip(&from) {
            let carried_by = item_at(game, *id).x - x;
            assert!(
                (carried_by - 2.0).abs() < 0.05,
                "item {id} moved {carried_by} with a collider that moved 2.0"
            );
        }
        eprintln!(
            "tick of the level, 200 items, ms per tick: no collider {open:.3}, capped collider still {still:.3}, capped collider moved before the tick {moving:.3}, carried {carried:.3}"
        );
    });
}

// Carrying: the bodies standing on a collider move with it.

/// Where an item is.
fn item_at(game: &sunrise_gameplay::game::ServerGame<'_>, id: u32) -> Vec3 {
    game.dropped
        .iter()
        .find(|i| i.id == id)
        .map(|i| i.position)
        .expect("an item")
}

/// A different stack from `drop_item`'s, so two items never merge.
fn drop_dirt(game: &mut sunrise_gameplay::game::ServerGame<'_>, at: [f64; 3]) -> u32 {
    game.server.next_entity += 1;
    let id = game.server.next_entity;
    game.dropped.push(ItemEntity::new(
        id,
        ItemStack::new("minecraft:dirt", 1),
        Vec3::new(at[0], at[1] + 2.0, at[2]),
        Vec3::ZERO,
        0.0,
    ));
    id
}

/// Moves the pig that holds a collider, as a mod's own tick would.
fn shift_stand(game: &mut sunrise_gameplay::game::ServerGame<'_>, id: u32, by: Vec3) {
    let index = game.mobs.iter().position(|m| m.id == id).expect("a pig");
    game.mobs.update(index, |m| {
        m.position = Vec3::new(
            m.position.x + by.x,
            m.position.y + by.y,
            m.position.z + by.z,
        );
    });
}

fn settle(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    for _ in 0..25 {
        tick_level(game);
    }
}

fn near(a: Vec3, b: Vec3, within: f64) -> bool {
    (a.x - b.x).abs() <= within && (a.y - b.y).abs() <= within && (a.z - b.z).abs() <= within
}

/// A body resting on a collider that moves is carried by what the contact point moves; one off
/// the deck is not, and the collider is never stopped by the body.
#[test]
#[ignore = "daily"]
fn a_body_on_a_moving_collider_is_carried() {
    with_world(|game| {
        let (holder, holder_ref) = stand_with_id(game, [8.0, 70.0, 8.0]);
        holder_ref.set_collider(game, Some(deck())).unwrap();
        let rider = drop_item(game, [8.5, 70.0, 8.0]);
        // Beside the deck, on nothing: it keeps falling and is not carried with the deck.
        let beside = drop_dirt(game, [12.0, 70.0, 8.0]);
        settle(game);
        let from = item_at(game, rider);
        assert!(
            near(from, Vec3::new(8.5, 70.0, 8.0), 0.2),
            "rests on the deck: {from:?}"
        );
        let beside_x = item_at(game, beside).x;
        for _ in 0..20 {
            tick_level_moving(game, |g| shift_stand(g, holder, Vec3::new(0.1, 0.0, 0.05)));
        }
        let to = item_at(game, rider);
        assert!(
            near(to, Vec3::new(from.x + 2.0, from.y, from.z + 1.0), 0.05),
            "carried by the deck's 2 x 1: {from:?} -> {to:?}"
        );
        assert!(
            (item_at(game, beside).x - beside_x).abs() < 0.05,
            "an item off the deck stays"
        );
        // The deck stops: so does the rider.
        for _ in 0..5 {
            tick_level(game);
        }
        assert!(
            near(item_at(game, rider), to, 0.02),
            "a still deck carries nothing"
        );
    });
}

/// A collider that turns carries the body at a distance from its origin along the arc, the
/// contact point's own displacement: yaw, and roll (a deck that tilts under the body).
#[test]
#[ignore = "daily"]
fn a_turning_collider_carries_the_body_along_the_arc() {
    with_world(|game| {
        let (_, holder_ref) = stand_with_id(game, [8.0, 70.0, 8.0]);
        holder_ref.set_collider(game, Some(deck())).unwrap();
        let rider = drop_item(game, [9.0, 70.0, 8.0]);
        settle(game);
        let yaw = |degrees: f32| {
            let half = degrees.to_radians() / 2.0;
            [0.0, half.sin(), 0.0, half.cos()]
        };
        // Nine turns of 10 degrees: a quarter turn. Local +x (1 block out) goes to -z.
        for step in 1..=9 {
            tick_level_moving(game, |g| {
                holder_ref
                    .set_collider_pose(
                        g,
                        ColliderPose::default().with_rotation(yaw(10.0 * step as f32)),
                    )
                    .unwrap()
            });
        }
        let at = item_at(game, rider);
        assert!(
            near(at, Vec3::new(8.0, 70.0, 7.0), 0.06),
            "a quarter turn about the deck's centre: {at:?}"
        );
    });
}

/// A deck that rises carries the body up with it, at 0.3 blocks a tick.
#[test]
#[ignore = "daily"]
fn a_rising_collider_carries_the_body_up() {
    with_world(|game| {
        let (holder, holder_ref) = stand_with_id(game, [8.0, 70.0, 8.0]);
        holder_ref.set_collider(game, Some(deck())).unwrap();
        let rider = drop_item(game, [8.5, 70.0, 8.0]);
        settle(game);
        let from = item_at(game, rider);
        for _ in 0..10 {
            tick_level_moving(game, |g| shift_stand(g, holder, Vec3::new(0.0, 0.3, 0.0)));
        }
        let to = item_at(game, rider);
        assert!(
            near(to, Vec3::new(from.x, from.y + 3.0, from.z), 0.05),
            "up 3 blocks: {from:?} -> {to:?}"
        );
    });
}

/// A deck tilting under a body (roll about z) lifts it by the contact point's rise.
#[test]
#[ignore = "daily"]
fn a_tilting_collider_lifts_the_body_by_its_contact_point() {
    with_world(|game| {
        let (_, holder_ref) = stand_with_id(game, [8.0, 70.0, 8.0]);
        holder_ref.set_collider(game, Some(deck())).unwrap();
        let rider = drop_item(game, [9.0, 70.0, 8.0]);
        settle(game);
        let roll = 15.0_f32.to_radians() / 2.0;
        tick_level_moving(game, |g| {
            holder_ref
                .set_collider_pose(
                    g,
                    ColliderPose::default().with_rotation([0.0, 0.0, roll.sin(), roll.cos()]),
                )
                .unwrap()
        });
        // The contact point 1 block out along local x rises by sin 15 and moves out by cos 15.
        let want = Vec3::new(
            8.0 + 15.0_f64.to_radians().cos(),
            70.0 + 15.0_f64.to_radians().sin(),
            8.0,
        );
        let at = item_at(game, rider);
        assert!(near(at, want, 0.1), "{at:?} vs {want:?}");
    });
}

/// A body is carried into a wall never through it, and the collider is not stopped.
#[test]
#[ignore = "daily"]
fn a_carried_body_stops_at_a_wall_and_the_collider_goes_on() {
    with_world(|game| {
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        for y in 70..=72 {
            for z in 6..=10 {
                game.world.set(10, y, z, stone).unwrap();
            }
        }
        let (holder, holder_ref) = stand_with_id(game, [8.0, 70.0, 8.0]);
        holder_ref.set_collider(game, Some(deck())).unwrap();
        let rider = drop_item(game, [8.5, 70.0, 8.0]);
        settle(game);
        for _ in 0..15 {
            tick_level_moving(game, |g| shift_stand(g, holder, Vec3::new(0.1, 0.0, 0.0)));
        }
        let at = item_at(game, rider);
        assert!(at.x < 10.0, "never through the wall: {at:?}");
        assert!(at.x > 9.8, "carried up to it: {at:?}");
        let moved = game
            .mobs
            .iter()
            .find(|m| m.id == holder)
            .unwrap()
            .position
            .x;
        assert!(
            (moved - 9.5).abs() < 1.0e-6,
            "the collider is not stopped: {moved}"
        );
    });
}

/// A collider standing on a collider is carried first, and its riders take its whole
/// displacement.
#[test]
#[ignore = "daily"]
fn a_collider_on_a_collider_passes_the_displacement_on() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::native_colliders_tests::a_collider_on_a_collider_passes_the_displacement_on",
        || {
            with_world(|game| {
                let (c1, c1_ref) = stand_with_id(game, [8.0, 70.0, 8.0]);
                c1_ref.set_collider(game, Some(deck())).unwrap();
                // The second collider's holder rests on the first deck, with a small deck above it.
                // Its descent clears the pig that holds the first deck, so this tests carry only.
                let c2 = drop_item(game, [9.0, 70.0, 8.0]);
                game.entity(c2)
                    .unwrap()
                    .set_collider(
                        game,
                        Some(Collider::new(vec![ColliderBox {
                            min: [-0.5, 0.0, -0.5],
                            max: [0.5, 0.2, 0.5],
                        }])),
                    )
                    .unwrap();
                // And a body on that.
                let rider = drop_dirt(game, [9.0, 70.2, 8.0]);
                settle(game);
                let (from_c2, from) = (item_at(game, c2), item_at(game, rider));
                assert!(
                    from.y > 70.1,
                    "the rider stands on the small deck: {from:?}"
                );
                for _ in 0..10 {
                    tick_level_moving(game, |g| shift_stand(g, c1, Vec3::new(0.1, 0.0, 0.0)));
                }
                let (to_c2, to) = (item_at(game, c2), item_at(game, rider));
                assert!(
                    near(
                        to_c2,
                        Vec3::new(from_c2.x + 1.0, from_c2.y, from_c2.z),
                        0.05
                    ),
                    "{to_c2:?}"
                );
                assert!(
                    near(to, Vec3::new(from.x + 1.0, from.y, from.z), 0.05),
                    "{from:?} -> {to:?}"
                );
            })
        },
    );
}

/// A carry is an `entity_move` with the cause `Carried`: a listener hears it with the contact
/// point's displacement, and cancelling it leaves the body where it stood.
#[test]
#[ignore = "daily"]
fn a_listener_hears_a_carry_and_may_cancel_it() {
    use mod_api::{EntityMoveEvent, Flow, Hooks, LoopContext, MoveCause};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    struct Listener(Arc<AtomicBool>, Arc<Mutex<Vec<(u32, MoveCause, [f64; 3])>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn entity_move(&self, _: &dyn LoopContext, event: &mut EntityMoveEvent<'_>) -> Flow {
            if event.cause == MoveCause::Carried {
                self.1
                    .lock()
                    .unwrap()
                    .push((event.entity, event.cause, *event.delta));
                if self.0.load(Ordering::Relaxed) {
                    return Flow::Cancel;
                }
            }
            Flow::Continue
        }
    }
    let cancel = Arc::new(AtomicBool::new(true));
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "carry_listener",
        Box::new(Listener(cancel.clone(), heard.clone())),
    )]
    .into();
    with_world(|game| {
        let (holder, holder_ref) = stand_with_id(game, [8.0, 70.0, 8.0]);
        holder_ref.set_collider(game, Some(deck())).unwrap();
        let rider = drop_item(game, [8.5, 70.0, 8.0]);
        settle(game);
        let from = item_at(game, rider);
        mod_platform::testing::with_mods(mods, || {
            // Cancelled: heard, and the body does not move.
            tick_level_moving(game, |g| shift_stand(g, holder, Vec3::new(0.5, 0.0, 0.0)));
            assert!(
                near(item_at(game, rider), from, 0.01),
                "a cancelled carry moves nothing"
            );
            let first = heard.lock().unwrap().clone();
            assert_eq!(first.len(), 1, "{first:?}");
            assert_eq!(first[0].0, rider);
            assert!(
                (first[0].2[0] - 0.5).abs() < 1.0e-6,
                "the contact point's 0.5: {first:?}"
            );
            // Allowed: the same carry goes through.
            cancel.store(false, Ordering::Relaxed);
            tick_level_moving(game, |g| shift_stand(g, holder, Vec3::new(0.5, 0.0, 0.0)));
            assert!(
                near(
                    item_at(game, rider),
                    Vec3::new(from.x + 0.5, from.y, from.z),
                    0.02
                ),
                "{:?}",
                item_at(game, rider)
            );
        });
    });
}

/// A carried vehicle takes its passenger and that passenger's own passenger with it: all three
/// move by the same displacement, the riders seated by the one rider positioning the tick uses.
#[test]
#[ignore = "daily"]
fn a_carried_vehicle_takes_its_passengers_and_theirs_with_it() {
    with_world(|game| {
        let (holder, holder_ref) = stand_with_id(game, [8.0, 70.0, 8.0]);
        holder_ref.set_collider(game, Some(deck())).unwrap();
        let (vehicle, _) = stand_with_id(game, [8.5, 70.0, 8.0]);
        let (middle, _) = stand_with_id(game, [8.5, 71.0, 8.0]);
        let (top, _) = stand_with_id(game, [8.5, 72.0, 8.0]);
        let update = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                      id: u32,
                      f: &dyn Fn(&mut sunrise_sim::mobs::Mob)| {
            let at = game.mobs.iter().position(|m| m.id == id).expect("a pig");
            game.mobs.update(at, |m| f(m));
        };
        update(game, vehicle, &|m| m.ground.on_ground = true);
        update(game, middle, &|m| m.extra.mount = Some(vehicle));
        update(game, top, &|m| m.extra.mount = Some(middle));
        game.position_riders();
        let position = |game: &sunrise_gameplay::game::ServerGame<'_>, id: u32| {
            game.mobs
                .iter()
                .find(|m| m.id == id)
                .map(|m| m.position)
                .expect("a pig")
        };
        let before = [vehicle, middle, top].map(|id| position(game, id));
        assert!(
            before[1].y > before[0].y && before[2].y > before[1].y,
            "{before:?}"
        );
        tick_level_moving(game, |g| shift_stand(g, holder, Vec3::new(0.5, 0.0, 0.0)));
        for (id, was) in [vehicle, middle, top].into_iter().zip(before) {
            let now = position(game, id);
            assert!(
                near(now, Vec3::new(was.x + 0.5, was.y, was.z), 0.02),
                "{id}: {was:?} -> {now:?}"
            );
        }
    });
}

/// The level records its colliders as each of its ticks begins, so a report that arrives in a
/// tick is judged against the transforms of the ticks before it: age 0 is where the collider
/// was when this tick began, age n where it was n ticks earlier; and a level with no collider
/// keeps nothing.
#[test]
#[ignore = "daily"]
fn the_level_records_its_colliders_as_each_tick_begins() {
    with_world(|game| {
        let (holder, holder_ref) = stand_with_id(game, [8.0, 70.0, 8.0]);
        holder_ref.set_collider(game, Some(deck())).unwrap();
        let x_of = |game: &sunrise_gameplay::game::ServerGame<'_>| {
            game.mobs
                .iter()
                .find(|m| m.id == holder)
                .map(|m| m.position.x)
                .unwrap()
        };
        let mut began = Vec::new();
        for _ in 0..8 {
            began.push(x_of(game));
            // The level's own tick records the colliders as it begins.
            game.tick_server(false).unwrap();
            shift_stand(game, holder, Vec3::new(0.5, 0.0, 0.0));
        }
        let history = game
            .level_collider_history()
            .expect("a server records a level that has a collider");
        let history = history.borrow();
        let (log, _) = history.get(holder).expect("the collider's frames");
        for age in 0..8usize {
            let frame = log.frame_at(history.now(), age as f64).unwrap();
            assert!(
                (frame.origin.x - began[7 - age]).abs() < 1.0e-9,
                "age {age}: {} vs {}",
                frame.origin.x,
                began[7 - age]
            );
        }
        // Between two ticks it is the line between them.
        let mid = log.frame_at(history.now(), 1.5).unwrap().origin.x;
        assert!((mid - (began[6] + began[5]) / 2.0).abs() < 1.0e-9, "{mid}");
        drop(history);
        // The collider goes: the level keeps no history.
        holder_ref.set_collider(game, None).unwrap();
        game.tick_server(false).unwrap();
        assert!(game.level_collider_history().is_none());
    });
}

/// A body standing on a collider turning about a far pivot is carried along the arc around
/// that pivot.
#[test]
#[ignore = "daily"]
fn a_body_standing_on_a_collider_turning_about_a_far_pivot_is_carried_along_the_arc() {
    with_world(|game| {
        let pivot = [6.0, 0.0, 0.0];
        let (holder, holder_ref) = stand_with_id(game, [2.0, 70.0, 2.0]);
        holder_ref
            .set_collider(game, Some(deck().with_pivot(pivot)))
            .unwrap();
        let rider = drop_item(game, [2.5, 70.0, 2.0]);
        settle(game);
        let yaw = |degrees: f32| {
            let half = degrees.to_radians() / 2.0;
            [0.0, half.sin(), 0.0, half.cos()]
        };
        // Nine 10-degree yaw steps about world pivot (8, 70, 2): local (0.5, 0, 0) is (-5.5, 0, 0)
        // from the pivot, so 90 degrees (+x -> -z) sends it to (0, 0, +5.5) -> world (8, 70, 7.5).
        for step in 1..=9 {
            tick_level_moving(game, |g| {
                holder_ref
                    .set_collider_pose(g, ColliderPose::new(yaw(10.0 * step as f32), pivot))
                    .unwrap();
            });
        }
        let item = game.dropped.iter().find(|i| i.id == rider).unwrap();
        let item_bounds = item.bounds();
        assert!(
            near(item.position, Vec3::new(8.0, 70.0, 7.5), 0.1),
            "carried along the arc about the far pivot: {:?}",
            item.position
        );
        let placed = game.placed_moving_colliders();
        let collider = &placed.placed().find(|p| p.id == holder).unwrap().collider;
        assert!(
            !collider.intersects(item_bounds),
            "rider must not be embedded after turning about a far pivot"
        );
    });
}

/// A tilting collider carrying a body with step height 0 (an item entity) never embeds it.
#[test]
#[ignore = "daily"]
fn a_tilting_collider_with_a_step_height_zero_body_does_not_embed_it() {
    with_world(|game| {
        let (holder, holder_ref) = stand_with_id(game, [8.0, 70.0, 8.0]);
        holder_ref.set_collider(game, Some(deck())).unwrap();
        let on_edge = drop_item(game, [9.0, 70.0, 8.0]);
        let at_pivot = drop_dirt(game, [8.0, 70.0, 8.0]);
        settle(game);
        let roll = 15.0_f32.to_radians() / 2.0;
        tick_level_moving(game, |g| {
            holder_ref
                .set_collider_pose(
                    g,
                    ColliderPose::default().with_rotation([0.0, 0.0, roll.sin(), roll.cos()]),
                )
                .unwrap();
        });
        let placed = game.placed_moving_colliders();
        let collider = &placed.placed().find(|p| p.id == holder).unwrap().collider;
        for id in [on_edge, at_pivot] {
            let item = game.dropped.iter().find(|i| i.id == id).unwrap();
            assert!(
                !collider.intersects(item.bounds()),
                "step-height-0 item {id} must not be embedded: {:?}",
                item.position
            );
            assert!(
                item.ground.on_ground,
                "step-height-0 item {id} stays grounded on the tilted deck"
            );
        }
    });
}

/// `teleport_to` and `set_collider_pose` on the same entity in one tick, in either order,
/// sweep and carry standing bodies identically at high speed (76.8 deg/tick and >1.4 blocks/tick)
/// with no body left inside a box.
#[test]
#[ignore = "daily"]
fn teleport_to_and_set_collider_pose_in_one_tick_sweep_and_carry_in_either_order_at_high_speed() {
    let run = |teleport_first: bool| -> (Vec3, Vec3) {
        let mut out = (Vec3::ZERO, Vec3::ZERO);
        with_world(|game| {
            let pivot = [1.0, 0.0, 0.5];
            let start_pos = [4.0, 70.0, 4.0];
            // > 1.4 blocks/tick linear shift (sqrt(1.6^2 + 0.8^2) ~= 1.79 blocks/tick)
            let end_pos = [5.6, 70.0, 4.8];
            // 76.8 deg/tick (a 256 RPM bearing)
            let half = 76.8_f32.to_radians() / 2.0;
            let end_pose = ColliderPose::new([0.0, half.sin(), 0.0, half.cos()], pivot);

            let (holder, holder_ref) = stand_with_id(game, start_pos);
            holder_ref
                .set_collider(game, Some(deck().with_pivot(pivot)))
                .unwrap();
            // Standing rider on top of the deck.
            let rider = drop_item(game, [4.5, 70.0, 4.0]);
            settle(game);
            let before_rider = item_at(game, rider);

            // Bystander sitting in the swept volume of the shifting + turning deck (not on top).
            let bystander = game.server.next_entity + 1;
            game.server.next_entity = bystander;
            game.dropped.push(ItemEntity::new(
                bystander,
                ItemStack::new("minecraft:dirt", 1),
                Vec3::new(5.6, 69.5, 4.8),
                Vec3::ZERO,
                0.0,
            ));
            game.dropped
                .iter_mut()
                .find(|i| i.id == bystander)
                .unwrap()
                .no_gravity = true;

            tick_level_moving(game, |g| {
                if teleport_first {
                    assert!(holder_ref.teleport_to(g, end_pos, None));
                    holder_ref.set_collider_pose(g, end_pose).unwrap();
                } else {
                    holder_ref.set_collider_pose(g, end_pose).unwrap();
                    assert!(holder_ref.teleport_to(g, end_pos, None));
                }
            });

            let placed = game.placed_moving_colliders();
            let collider = &placed.placed().find(|p| p.id == holder).unwrap().collider;
            let after_rider = game.dropped.iter().find(|i| i.id == rider).unwrap();
            let after_bystander = game.dropped.iter().find(|i| i.id == bystander).unwrap();

            // Expected rider contact position from the exact combined frame transform.
            let before_frame = sunrise_core::geometry::oriented::Frame::from_pose(
                Vec3::new(start_pos[0], start_pos[1], start_pos[2]),
                Vec3::new(pivot[0], pivot[1], pivot[2]),
                [0.0, 0.0, 0.0, 1.0],
            );
            let expected_rider = collider.frame.to_world(before_frame.to_local(before_rider));
            assert!(
                near(after_rider.position, expected_rider, 0.05),
                "teleport_first={teleport_first}: {:?} vs {:?}",
                after_rider.position,
                expected_rider
            );
            assert!(
                !collider.intersects(after_rider.bounds()),
                "teleport_first={teleport_first}: rider left inside a box"
            );
            assert!(
                !collider.intersects(after_bystander.bounds()),
                "teleport_first={teleport_first}: bystander left inside a box"
            );
            out = (after_rider.position, after_bystander.position);
        });
        out
    };

    let (rider_a, bystander_a) = run(true);
    let (rider_b, bystander_b) = run(false);
    assert!(
        rider_a.distance(rider_b) < 1.0e-9,
        "rider differs by call order: {rider_a:?} vs {rider_b:?}"
    );
    assert!(
        bystander_a.distance(bystander_b) < 1.0e-9,
        "bystander differs by call order: {bystander_a:?} vs {bystander_b:?}"
    );
}

/// Steps one client tick and drains its packets up to `ClientTickEnd`.
#[cfg(feature = "client")]
fn tick_client(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    wire: &mut sunrise_protocol::connection::Connection,
) {
    use sunrise_protocol::packets::play::Serverbound;
    game.tick().unwrap();
    loop {
        let (id, bytes) = wire.recv().unwrap();
        if matches!(
            Serverbound::decode(id, &bytes).unwrap(),
            Serverbound::ClientTickEnd(_)
        ) {
            break;
        }
    }
}

/// A collider that turns in place via `collider_sync::Message::Pose` arriving before each
/// client tick carries the local player standing 1.2 blocks off its axis around it (#5584).
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_client_collider_turning_before_the_tick_carries_the_local_player() {
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use sunrise_client_gameplay::game::client_side::native_client_session::tests::{
        finish_login, logged_client,
    };
    use sunrise_gameplay_common::game::Screen;
    use sunrise_protocol::collider_sync::{self, Message as ColliderMessage, WireBox};
    use sunrise_protocol::connection::Connection;
    use sunrise_protocol::packets::{configuration::CustomPayload, play::Clientbound};
    use sunrise_sim::motion::PlayerMotion;

    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::native_colliders_tests::a_client_collider_turning_before_the_tick_carries_the_local_player",
        || {
            const PLAYER: u32 = 42;
            const SOURCE: i32 = 80;
            // Off the axis far enough that the player's box (half width 0.3) never meets the
            // pig's (0.45) on the turn: inside 0.75 on both axes `Entity.push` would shove
            // the player outward and the radius would no longer be the carry's alone.
            const RADIUS: f64 = 1.2;
            let yaw = |degrees: f32| {
                let half = degrees.to_radians() / 2.0;
                [0.0, half.sin(), 0.0, half.cos()]
            };

            for deg_per_tick in [2.4_f32, 4.8_f32] {
                with_client_game(|game| {
                    let mut wire = Connection::local(logged_client(game));
                    wire.reader
                        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                        .unwrap();
                    finish_login(game, &mut wire);
                    game.client.title.0 = None;
                    game.client.screen = Screen::Playing;
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));

                    game.spawn_entity(SOURCE, "minecraft:pig", [8.0, 70.0, 8.0], [0.0; 3]);
                    game.handle_packet(Clientbound::CustomPayload(CustomPayload {
                        channel: collider_sync::CHANNEL.into(),
                        data: collider_sync::encode(&ColliderMessage::Boxes {
                            entity: SOURCE as u32,
                            rotation: [0.0, 0.0, 0.0, 1.0],
                            pivot: [0.0; 3],
                            total: 1,
                            first: 0,
                            boxes: vec![WireBox {
                                min: [-1.5, -1.0, -1.5],
                                max: [1.5, 0.0, 1.5],
                            }],
                        }),
                    }))
                    .unwrap();

                    let mut player =
                        PlayerMotion::new(game.motion, Vec3::new(8.0 + RADIUS, 70.0, 8.0)).unwrap();
                    player.entity_id = PLAYER as i32;
                    player.client_side = true;
                    game.player = Some(player);
                    let remote = game.client.remote.as_mut().unwrap();
                    remote.entity_id = PLAYER as i32;
                    remote.loaded_sent = true;
                    remote.level_load.finish();

                    // Initial tick settles on_ground and snapshots the unturned collider pose.
                    tick_client(game, &mut wire);

                    const STEPS: usize = 15;
                    for step in 1..=STEPS {
                        game.handle_packet(Clientbound::CustomPayload(CustomPayload {
                            channel: collider_sync::CHANNEL.into(),
                            data: collider_sync::encode(&ColliderMessage::Pose {
                                entity: SOURCE as u32,
                                rotation: yaw(deg_per_tick * step as f32),
                                pivot: [0.0; 3],
                            }),
                        }))
                        .unwrap();
                        tick_client(game, &mut wire);
                    }

                    let total = f64::from(deg_per_tick * STEPS as f32).to_radians();
                    let want =
                        Vec3::new(8.0 + RADIUS * total.cos(), 70.0, 8.0 - RADIUS * total.sin());
                    let at = game.player.as_ref().unwrap().position;
                    assert!(
                        near(at, want, 0.06),
                        "rate {deg_per_tick} deg/tick: got {at:?}, want {want:?}"
                    );
                });
            }
        },
    );
}

/// A collider descending 0.03125/tick (moved by `teleport_to` / `EntityPositionSync`, and again
/// by the entity's own move / `MoveEntityPos`) carries the local player down with no x/z drift
/// over 40 ticks, and `closest_space_nudge` ignores mod collider boxes (#5585).
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_client_descending_collider_carries_the_local_player_without_sideways_drift() {
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use sunrise_client_gameplay::game::client_side::native_client_session::tests::{
        finish_login, logged_client,
    };
    use sunrise_gameplay_common::game::Screen;
    use sunrise_protocol::collider_sync::{self, Message as ColliderMessage, WireBox};
    use sunrise_protocol::connection::Connection;
    use sunrise_protocol::general_entity::{self, Message as GeneralMessage};
    use sunrise_protocol::packets::{
        configuration::CustomPayload,
        play::{self, Clientbound},
    };
    use sunrise_sim::motion::PlayerMotion;

    struct Declare;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Declare {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.declare_general_entity(
                "platform",
                mod_api::GeneralEntityCategory::Misc,
                0.5,
                0.5,
                None,
                mod_api::GeneralSavePolicy::Save,
                true,
                false,
                false,
                Some(mod_api::PushReaction::IgnoreEntity),
                8,
                1,
                1,
                &[],
            )
            .unwrap();
        }
    }

    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::native_colliders_tests::a_client_descending_collider_carries_the_local_player_without_sideways_drift",
        || {
            const PLAYER: u32 = 42;
            const SOURCE: i32 = 80;
            const STEP: f64 = 0.03125;
            const TICKS: usize = 40;

            mod_platform::__private::initialize().unwrap();
            let _phase = mod_platform::__private::RegistrationGuard::open();
            let declare = || vec![mod_api::ModEntry::new("mymod", Box::new(Declare))];
            mod_platform::testing::with_mods(declare().into(), || {
                mod_platform::__private::part_tests::install_parts_fixture(&declare()).unwrap();

                for by_teleport in [true, false] {
                    with_client_game(|game| {
                        let mut wire = Connection::local(logged_client(game));
                        wire.reader
                            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                            .unwrap();
                        finish_login(game, &mut wire);
                        game.client.title.0 = None;
                        game.client.screen = Screen::Playing;
                        game.world
                            .chunks
                            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));

                        game.handle_packet(Clientbound::CustomPayload(CustomPayload {
                            channel: general_entity::CHANNEL.into(),
                            data: general_entity::encode(&GeneralMessage::Spawn {
                                id: SOURCE,
                                uuid: 0x77,
                                type_key: "mymod:platform".into(),
                                position: [8.0, 70.0, 8.0],
                                rotation: [0.0, 0.0],
                                velocity: [0.0; 3],
                            })
                            .unwrap(),
                        }))
                        .unwrap();
                        game.handle_packet(Clientbound::CustomPayload(CustomPayload {
                            channel: collider_sync::CHANNEL.into(),
                            data: collider_sync::encode(&ColliderMessage::Boxes {
                                entity: SOURCE as u32,
                                rotation: [0.0, 0.0, 0.0, 1.0],
                                pivot: [0.0; 3],
                                total: 1,
                                first: 0,
                                boxes: vec![WireBox {
                                    min: [-1.5, -1.0, -1.5],
                                    max: [1.5, 0.0, 1.5],
                                }],
                            }),
                        }))
                        .unwrap();

                        let mut player =
                            PlayerMotion::new(game.motion, Vec3::new(9.0, 70.0, 8.0)).unwrap();
                        player.entity_id = PLAYER as i32;
                        player.client_side = true;
                        game.player = Some(player);
                        let remote = game.client.remote.as_mut().unwrap();
                        remote.entity_id = PLAYER as i32;
                        remote.loaded_sent = true;
                        remote.level_load.finish();

                        // Settle on the platform before descent begins.
                        tick_client(game, &mut wire);

                        let mut y = 70.0;
                        for tick in 0..TICKS {
                            y -= STEP;
                            let packet = if by_teleport {
                                Clientbound::EntityPositionSync(play::EntityPositionSync {
                                    id: SOURCE,
                                    path: play::PositionPath::Linear([8.0, y, 8.0]),
                                    yaw: 0.0,
                                    pitch: 0.0,
                                    on_ground: false,
                                })
                            } else {
                                // -128 / 4096 == -0.03125
                                Clientbound::MoveEntityPos(play::MoveEntityPos(play::MoveEntity {
                                    id: SOURCE,
                                    delta: Some(play::VecDelta::Linear([0, -128, 0])),
                                    rotation: None,
                                    on_ground: false,
                                }))
                            };
                            game.handle_packet(packet).unwrap();
                            tick_client(game, &mut wire);
                            let at = game.player.as_ref().unwrap().position;
                            assert!(
                                (at.x - 9.0).abs() < 1.0e-9 && (at.z - 8.0).abs() < 1.0e-9,
                                "by_teleport={by_teleport} tick {tick}: drifted sideways to {at:?}"
                            );
                        }

                        let at = game.player.as_ref().unwrap().position;
                        let deck_y = game.client_collider_origin(SOURCE as u32).unwrap().y;
                        assert!(
                            (at.y - deck_y).abs() < 0.02,
                            "by_teleport={by_teleport}: player y {} vs deck y {deck_y}",
                            at.y
                        );
                        assert!(
                            at.y < 70.0 - 1.1,
                            "by_teleport={by_teleport}: carried down from 70.0, got {}",
                            at.y
                        );

                        // Even if a body overlaps the collider top by a hair (e.g. after a
                        // server position update), `LocalPlayer.suffocatesAt` only checks
                        // suffocating blocks and never nudges sideways out of collider boxes.
                        let mut dipped =
                            PlayerMotion::new(game.motion, Vec3::new(9.0, deck_y - 0.05, 8.0))
                                .unwrap();
                        dipped.entity_id = PLAYER as i32;
                        let placed = game.client_placed_moving_colliders();
                        let nudge = sunrise_world::moving_colliders::in_level(placed, || {
                            sunrise_gameplay_common::closest_space::closest_space_nudge(
                                &dipped,
                                &game.world,
                                game.collision,
                                game.level_suffocation(),
                            )
                        });
                        assert_eq!(
                            nudge, [None; 2],
                            "collider boxes never trigger suffocation nudge"
                        );
                    });
                }
            });
        },
    );
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::passengers_common::PassengersCommonGameExt1 as _;

use sunrise_gameplay_common::game::piston_push::carrying::CarryingGameExt1 as _;

#[cfg(feature = "client")]
use sunrise_gameplay_common::game::tables::GameItems as _;
