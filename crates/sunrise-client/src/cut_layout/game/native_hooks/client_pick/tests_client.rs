use crate::game::native_hooks::client_pick::tests::First;
use crate::game::native_hooks::client_pick::tests::Last;
use crate::game::native_hooks::client_pick::tests::Policy;
use crate::game::native_hooks::client_pick::tests::RegisterRiderPart;
use crate::game::native_hooks::client_pick::tests::finish_login;
use crate::game::native_hooks::client_pick::tests::ranged_stack;
use mod_api::CrosshairTarget;
use std::sync::{Arc, Mutex};
use sunrise_gameplay_common::game::GameGameExt1;

#[test]
#[ignore = "daily"]
fn frame_pick_is_shared_ordered_and_refuses_stale_or_invalid_output() {
    use sunrise_core::geometry::Vec3;
    use sunrise_level::chunk::Chunk;
    use sunrise_protocol::packets::play::{self, Clientbound as C};
    use sunrise_sim::motion::PlayerMotion;
    use {
        crate::game::server_side::native_entity_refs::tests::with_client_game,
        sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client,
    };
    let policy = Arc::new(Mutex::new(Policy::default()));
    let mods = vec![
        mod_api::ModEntry::new("a_pick", Box::new(First)),
        mod_api::ModEntry::new("b_pick", Box::new(Last(policy.clone()))),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let mut server = sunrise_protocol::connection::Connection::local(logged_client(game));
            server
                .reader
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            finish_login(game, &mut server);
            game.native_lifecycle.begin_open(None);
            game.client.title.0 = None;
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 1.0, 0.5)).unwrap());
            game.yaw = -90.0;
            game.pitch = 0.0;
            game.block_reach = 4.5;
            game.entity_reach = 3.0;
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let kind = game
                .client
                .remote
                .as_ref()
                .unwrap()
                .builtin
                .get("minecraft:entity_type")
                .unwrap()
                .id("minecraft:pig")
                .unwrap();
            game.handle_packet(C::AddEntity(play::AddEntity {
                id: 31,
                uuid: 31,
                kind,
                position: [1.5, 2.0, 0.5],
                velocity: [0.0; 3],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 0,
            }))
            .unwrap();
            let entity = game.entity(31).unwrap();
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(2, 2, 0, stone).unwrap();
            game.refresh_client_pick(1.0);
            let block = game.crosshair_target().unwrap();
            assert!(matches!(block, CrosshairTarget::Block { .. }));
            assert_eq!(
                game.pick().unwrap().pos,
                [2, 2, 0],
                "outline and interaction share this read"
            );
            assert!(!game.entity_targeted());
            assert!(
                game.mob_hit().is_none(),
                "the entity cannot steal the block interaction"
            );
            game.pick_tick_targets();
            assert_eq!(game.crosshair_target(), Some(block));
            game.release_tick_targets();
            assert_eq!(
                policy.lock().unwrap().calls,
                1,
                "input never refires the hook"
            );

            policy.lock().unwrap().override_result = Some(None);
            game.refresh_client_pick(1.0);
            assert!(game.crosshair_target().is_none());
            assert!(game.pick().is_none());
            assert!(
                game.bucket_hit().is_some(),
                "the item-specific ray ignores the pick miss"
            );
            policy.lock().unwrap().override_result = Some(Some(block));
            game.refresh_client_pick(1.0);
            let air = game
                .pack
                .assets
                .registry
                .default_state("minecraft:air")
                .unwrap();
            game.world.set(2, 2, 0, air).unwrap();
            assert_eq!(
                game.pick().unwrap().pos,
                [2, 2, 0],
                "consumption keeps the tick hit after a block changes"
            );
            game.refresh_client_pick(1.0);
            assert!(
                game.crosshair_target().is_none(),
                "stale output refuses at writeback too"
            );
            game.world.set(2, 2, 0, stone).unwrap();
            for point in [[f64::NAN, 2.0, 0.5], [f64::INFINITY; 3], [100.0; 3]] {
                policy.lock().unwrap().override_result =
                    Some(Some(CrosshairTarget::Entity { entity, point }));
                game.refresh_client_pick(1.0);
                assert!(game.crosshair_target().is_none());
            }
            let entity_hit = CrosshairTarget::Entity {
                entity,
                point: [1.5, 2.6, 0.5],
            };
            policy.lock().unwrap().override_result = Some(Some(entity_hit));
            game.refresh_client_pick(1.0);
            assert_eq!(game.crosshair_target(), Some(entity_hit));
            assert!(game.entity_targeted());
            assert_eq!(game.mob_hit().map(|(i, _, _)| game.mobs[i].id), Some(31));
            game.native_lifecycle.begin_open(None);
            game.refresh_client_pick(1.0);
            assert!(
                game.crosshair_target().is_none(),
                "old level entity is rejected while its id is still present"
            );
            let entity = game.entity(31).unwrap();
            policy.lock().unwrap().override_result = Some(Some(CrosshairTarget::Entity {
                entity,
                point: [1.5, 2.6, 0.5],
            }));
            game.refresh_client_pick(1.0);
            assert!(game.entity_targeted());
            game.handle_packet(C::RemoveEntities(play::RemoveEntities { ids: vec![31] }))
                .unwrap();
            assert!(
                matches!(
                    game.crosshair_target(),
                    Some(CrosshairTarget::Entity { .. })
                ),
                "consumption retains the hit until the next pick"
            );
            game.refresh_client_pick(1.0);
            assert!(
                game.crosshair_target().is_none(),
                "removed entity output is refused at writeback"
            );
        })
    });
}

#[test]
#[ignore = "daily"]
fn frame_pick_without_listeners_keeps_the_vanilla_result() {
    use sunrise_core::geometry::Vec3;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::motion::PlayerMotion;
    use {
        crate::game::server_side::native_entity_refs::tests::with_client_game,
        sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client,
    };
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_client_game(|game| {
            let mut server = sunrise_protocol::connection::Connection::local(logged_client(game));
            finish_login(game, &mut server);
            game.native_lifecycle.begin_open(None);
            game.client.title.0 = None;
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 1.0, 0.5)).unwrap());
            game.yaw = -90.0;
            game.pitch = 0.0;
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(2, 2, 0, stone).unwrap();
            game.refresh_client_pick(1.0);
            assert_eq!(game.pick().unwrap().pos, [2, 2, 0]);
            game.yaw = 90.0;
            assert_eq!(
                game.pick().unwrap().pos,
                [2, 2, 0],
                "look changes wait for the next frame"
            );
            game.refresh_client_pick(1.0);
            assert!(game.pick().is_none());
        })
    });
}

#[test]
#[ignore = "daily"]
fn rider_pick_rejects_shared_passengers_unless_their_part_allows_them() {
    use sunrise_protocol::packets::play::{self, Clientbound as C};
    for subscribed in [false, true] {
        let calls = Arc::new(Mutex::new(0));
        let mods = if subscribed {
            vec![mod_api::ModEntry::new(
                "rider",
                Box::new(RegisterRiderPart(calls.clone(), false)),
            )]
        } else {
            Vec::new()
        };
        mod_platform::testing::with_mods(mods.into(), || {
            with_pick_game("minecraft:pig", 1.5, |game, _| {
                let kind = game
                    .client
                    .remote
                    .as_ref()
                    .unwrap()
                    .builtin
                    .get("minecraft:entity_type")
                    .unwrap()
                    .id("minecraft:oak_boat")
                    .unwrap();
                game.handle_packet(C::AddEntity(play::AddEntity {
                    id: 32,
                    uuid: 32,
                    kind,
                    position: [0.5, 1.0, 0.5],
                    velocity: [0.0; 3],
                    pitch: 0,
                    yaw: 0,
                    head_yaw: 0,
                    data: 0,
                }))
                .unwrap();
                game.remote_set_passengers(32, vec![7, 31]);
                game.refresh_client_pick(1.0);
                assert_eq!(
                    matches!(game.crosshair_target(),
                    Some(CrosshairTarget::Entity { entity, .. }) if entity.id() == 31),
                    subscribed
                );
                assert_eq!(*calls.lock().unwrap(), usize::from(subscribed));
                // The brush uses the margin-overload ray, not the mounted
                // crosshair's root-vehicle exclusion or rider part.
                let (mob, _, _) = game.mob_hit_within(game.block_reach).unwrap();
                assert_eq!(game.mobs[mob].id, 31);
                assert_eq!(*calls.lock().unwrap(), usize::from(subscribed));
                if !subscribed {
                    assert_eq!(game.client.client_pick.rider_candidates.capacity(), 0);
                }
                let capacity = game.client.client_pick.rider_candidates.capacity();
                game.refresh_client_pick(1.0);
                assert_eq!(
                    game.client.client_pick.rider_candidates.capacity(),
                    capacity
                );

                game.remote_set_passengers(32, vec![]);
                game.refresh_client_pick(1.0);
                assert!(matches!(game.crosshair_target(),
                    Some(CrosshairTarget::Entity { entity, .. }) if entity.id() == 31));
                assert_eq!(*calls.lock().unwrap(), 2 * usize::from(subscribed));
                game.remote_set_passengers(32, vec![7, 31]);

                // AttackRange uses its own geometry, with no root-vehicle filter.
                let selected = game.selected;
                game.inventory[selected] = Some(ranged_stack());
                game.refresh_client_pick(1.0);
                assert!(matches!(game.crosshair_target(),
                    Some(CrosshairTarget::Entity { entity, .. }) if entity.id() == 31));
                assert_eq!(*calls.lock().unwrap(), 2 * usize::from(subscribed));
                let selected = game.selected;
                game.inventory[selected] = None;

                // Put the root vehicle's box around the camera. Inside hits are
                // pickable without asking any part; remove the later passenger.
                game.mobs.update(0, |mob| mob.removed = true);
                game.boats.update(0, |boat| {
                    boat.position = sunrise_core::geometry::Vec3::new(0.5, 2.5, 0.5);
                });
                game.refresh_client_pick(1.0);
                assert!(matches!(game.crosshair_target(),
                    Some(CrosshairTarget::Entity { entity, .. }) if entity.id() == 32));
                assert_eq!(*calls.lock().unwrap(), 2 * usize::from(subscribed));
            });
        });
    }
}

pub(crate) fn with_pick_game(
    kind: &str,
    x: f64,
    run: impl FnOnce(
        &mut sunrise_client_gameplay::game::ClientGame<'_>,
        &mut sunrise_protocol::connection::Connection,
    ),
) {
    use sunrise_core::geometry::Vec3;
    use sunrise_level::chunk::Chunk;
    use sunrise_protocol::packets::play::{self, Clientbound as C};
    use sunrise_sim::motion::PlayerMotion;
    use {
        crate::game::server_side::native_entity_refs::tests::with_client_game,
        sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client,
    };
    with_client_game(|game| {
        let mut server = sunrise_protocol::connection::Connection::local(logged_client(game));
        server
            .reader
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        finish_login(game, &mut server);
        game.native_lifecycle.begin_open(None);
        game.client.title.0 = None;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 1.0, 0.5)).unwrap());
        game.previous_position = Vec3::new(0.5, 1.0, 0.5);
        game.yaw = -90.0;
        game.pitch = 0.0;
        game.entity_reach = 3.0;
        game.block_reach = 4.5;
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let kind = game
            .client
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:entity_type")
            .unwrap()
            .id(kind)
            .unwrap();
        game.handle_packet(C::AddEntity(play::AddEntity {
            id: 31,
            uuid: 31,
            kind,
            position: [x, 2.0, 0.5],
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 0,
            head_yaw: 0,
            data: 0,
        }))
        .unwrap();
        run(game, &mut server);
    });
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
