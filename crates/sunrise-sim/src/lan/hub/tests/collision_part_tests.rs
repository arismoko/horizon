//! A block part that gives a block another collision shape for the entity that asks, on both
//! sides of a guest's moves: the guest's client reads its own mod state, and the server's check
//! asks the same part with that guest's entity. When the two states differ, the server refuses
//! the move and sends the guest back (a position correction), as for any move it finds wrong.
use super::*;
use crate::motion::{MotionWorld, PlayerMotion};
use crate::native_client_hooks::test_hooks::{COLLISION_ANSWER, install, loop_context};
use crate::sprint::Keys;
use std::rc::Rc;
use sunrise_core::geometry::Vec3;
use sunrise_world::part_collision::InParts;

/// What a part can tell of the entity apart from where it is: the same on both sides when the
/// two ask about the same player.
fn signature(
    query: &mod_api::CollisionQuery,
) -> (
    Option<u32>,
    mod_api::CollisionEntityKind,
    bool,
    bool,
    bool,
    bool,
) {
    (
        query.entity(),
        query.kind(),
        query.is_descending(),
        query.is_placement(),
        query.always_collide_with_fluid(),
        query.is_holding_item("minecraft:stick"),
    )
}

/// The entity id both sides know the guest by.
const GUEST_ENTITY: i32 = 41;

fn scene(fixture: &crate::test_fixtures::Vanilla) -> World {
    let assets = &fixture.assets;
    let mut scene = World::test_scene_with_placements(assets, &[], 2).unwrap();
    let stone = assets.registry.default_state("minecraft:stone").unwrap();
    let glass = assets.registry.default_state("minecraft:glass").unwrap();
    let air = assets.registry.default_state("minecraft:air").unwrap();
    for x in 10..=30 {
        for z in 10..=30 {
            scene.set(x, 4, z, stone).unwrap();
            for y in 5..=12 {
                let block = if x == 22 && y < 8 { glass } else { air };
                scene.set(x, y, z, block).unwrap();
            }
        }
    }
    scene
}

#[test]
#[ignore = "daily"]
fn a_client_and_a_server_that_disagree_about_a_parts_state_get_a_correction() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    install();
    let glass = fixture
        .assets
        .registry
        .default_state("minecraft:glass")
        .unwrap();
    let collision = fixture.own_collision();
    collision.set_part_states([glass]);
    let mut walled = scene(fixture);
    // The mod's state on the client says the wall is open to this guest: its part answers empty.
    let client_asked = Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = Rc::clone(&client_asked);
    COLLISION_ANSWER.with_borrow_mut(|answer| {
        *answer = Some(Box::new(move |query| {
            seen.borrow_mut().push(signature(query));
            Some(Vec::new())
        }));
    });
    let resources = &fixture.motion;
    let mut client = PlayerMotion::new(resources, Vec3::new(20.5, 5.0, 20.5)).unwrap();
    client.client_side = true;
    client.entity_id = GUEST_ENTITY;
    client.main_hand = Some("minecraft:stick".into());
    client.yaw = -90.0;
    let forward = Keys {
        forward: true,
        ..Keys::default()
    };
    let walk: Vec<[f64; 3]> = {
        let _parts = InParts::enter(Rc::new(crate::collision_parts::ModParts::new(Arc::new(
            loop_context(),
        ))));
        (0..20)
            .map(|_| {
                MotionWorld {
                    side: crate::motion::dry_side(),
                    entities: &[],
                    player: &mut client,
                    scene: &mut walled,
                    assets: &fixture.assets,
                    collision_binding: &collision,
                    resources,
                    health: &resources.health,
                    tag_items: None,
                    block_registries: None,
                    loop_context: crate::test_resources::server_loop_context(),
                    damage_hooks: &crate::health::DamageHooks::NONE,
                }
                .controlled_simulation_tick(forward)
                .unwrap();
                let p = client.position;
                [p.x, p.y, p.z]
            })
            .collect()
    };
    assert!(
        walk.last().unwrap()[0] > 23.3,
        "the client walked through the wall"
    );

    // The server's own state says the guest is not let through: its part keeps the glass.
    let server_asked = Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = Rc::clone(&server_asked);
    COLLISION_ANSWER.with_borrow_mut(|answer| {
        *answer = Some(Box::new(move |query| {
            seen.borrow_mut().push(signature(query));
            None
        }));
    });
    let (mut hub, _rx) = hub_with_a_guest();
    let mut next_entity = 2;
    let border = crate::worldborder::WorldBorder::default();
    let ctx = TickCtx {
        sea_level: 63,
        world: &mut walled,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [20.5, 5.0, 20.5],
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: Some(&collision),
        border: &border,
        streaming: None,
    };
    let into = walk.iter().position(|at| at[0] + 0.3 > 22.0).unwrap();
    let before = if into == 0 {
        [20.5, 5.0, 20.5]
    } else {
        walk[into - 1]
    };
    hub.guests.update(0, |g| {
        (g.position, g.on_ground) = (before, true);
        g.entity_id = GUEST_ENTITY;
        let selected = g.selected;
        g.inventory[selected] = Some(crate::items::ItemStack::new("minecraft:stick", 1));
    });
    let _parts = InParts::enter(Rc::new(crate::collision_parts::ModParts::new(Arc::new(
        loop_context(),
    ))));
    let (_, refused, _) = hub.collide(0, walk[into], &ctx);
    assert!(refused, "the server sends the guest back");
    // Both sides asked the same query about the same player: a player holding a stick.
    {
        let (client, server) = (client_asked.borrow(), server_asked.borrow());
        assert!(!client.is_empty() && !server.is_empty());
        assert_eq!(client.last(), server.last());
        assert_eq!(
            server.last(),
            Some(&(
                Some(GUEST_ENTITY as u32),
                mod_api::CollisionEntityKind::Player,
                false,
                false,
                false,
                true
            ))
        );
    }
    assert_eq!(hub.guests.len(), 1, "a correction, not a kick");
    // With the same state on both sides the same move is accepted.
    COLLISION_ANSWER.with_borrow_mut(|answer| *answer = Some(Box::new(|_| Some(Vec::new()))));
    let (_parts_again, accepted) = (
        InParts::enter(Rc::new(crate::collision_parts::ModParts::new(Arc::new(
            loop_context(),
        )))),
        hub.collide(0, walk[into], &ctx),
    );
    assert_eq!((accepted.0, accepted.1), (false, false));
}
