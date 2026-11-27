#![cfg(all(test, feature = "client"))]
//! `EntityRef::previous_position`, `previous_rotation` and `is_alive` on a client:
//! the values kept when the tracked entity's tick began and its synced health.

use super::native_client_session::tests::logged_client;
use super::native_entity_refs::tests::with_client_game;
use crate::game::ClientGame;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
use sunrise_level::chunk::Chunk;
use sunrise_protocol::entity_data::Value as EntityDataValue;
use sunrise_protocol::packets::play::{self, Clientbound as C};
use sunrise_sim::mobs::Mob;

/// A client logged in to a server, with the column at (0, 0) loaded, that the server told of
/// entity `id`, a `kind`, at (2.5, 64, 2.5).
fn client_with(
    game: &mut ClientGame<'_>,
    id: i32,
    kind: &str,
) -> sunrise_protocol::connection::Pipe {
    let server = logged_client(game);
    game.client
        .streaming
        .insert_remote(&mut game.common.world, Chunk::empty(0, 0));
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
        id,
        uuid: 0x4984_0000 + id as u128,
        kind,
        position: [2.5, 64.0, 2.5],
        velocity: [0.0; 3],
        pitch: 0,
        yaw: 0,
        head_yaw: 0,
        data: 0,
    }))
    .unwrap();
    server
}

/// The mob `id` of the game's list.
fn mob<'a>(game: &'a mut ClientGame<'_>, id: u32) -> &'a mut Mob {
    game.mobs.iter_mut().find(|m| m.id == id).expect("the mob")
}

/// A client's copy of a walking villager: the client's entity pass keeps where its tick began.
#[test]
#[ignore = "daily"]
fn a_walking_villagers_previous_tick_on_a_client() {
    mod_platform::testing::with_mods(Default::default(), || {
        with_client_game(|game| {
            let _server = client_with(game, 401, "minecraft:villager");
            let entity = game.entity(401).expect("the tracked villager");
            game.tick_remote_entities(true);
            let m = mob(game, 401);
            m.position = Vec3::new(3.0, 64.0, 2.25);
            m.yaw = 45.0;
            m.head_pitch = 20.0;
            let host = &*game as &dyn mod_api::ClientHost;
            assert_eq!(entity.previous_position(host), Some([2.5, 64.0, 2.5]));
            assert_eq!(entity.previous_rotation(host), Some([0.0, 0.0]));
            assert_eq!(entity.position(host), Some([3.0, 64.0, 2.25]));
            game.tick_remote_entities(true);
            let host = &*game as &dyn mod_api::ClientHost;
            assert_eq!(entity.previous_position(host), Some([3.0, 64.0, 2.25]));
            assert_eq!(entity.previous_rotation(host), Some([45.0, 20.0]));
            let other = mod_api::EntityRef::new(entity.epoch(), 9_999);
            assert_eq!(other.previous_position(host), None);
            assert_eq!(other.previous_rotation(host), None);
        });
    });
}

/// A zombie's client copy is alive until its synced health (`DATA_HEALTH_ID`) reaches 0.
#[test]
#[ignore = "daily"]
fn a_zombies_client_copy_dies_with_its_synced_health() {
    mod_platform::testing::with_mods(Default::default(), || {
        with_client_game(|game| {
            let _server = client_with(game, 402, "minecraft:zombie");
            let zombie = game.entity(402).expect("the tracked zombie");
            let health = |game: &mut ClientGame<'_>, health: f32| {
                game.handle_packet(C::SetEntityData(play::SetEntityData {
                    id: 402,
                    values: vec![(9, 3, EntityDataValue::Float(health))],
                }))
                .unwrap();
            };
            health(game, 4.0);
            game.tick_remote_entities(true);
            let host = &*game as &dyn mod_api::ClientHost;
            assert_eq!(zombie.is_alive(host), Some(true));
            health(game, 0.0);
            game.tick_remote_entities(true);
            let host = &*game as &dyn mod_api::ClientHost;
            assert_eq!(zombie.is_alive(host), Some(false));
            let other = mod_api::EntityRef::new(zombie.epoch(), 9_999);
            assert_eq!(other.is_alive(host), None);
        });
    });
}
