//! A client reads `Entity.DATA_NO_GRAVITY` (5) for a mob and a dropped item: its own tick's
//! `isNoGravity` follows the server's flag (#5028). An item's flag can come before its stack
//! (`DATA_ITEM`, 8), which is what makes a client's item, and it is kept for it.
#![cfg(feature = "client")]
use crate::game::server_side::item_entities::thrown_items_tests::with_remote_game;
use sunrise_protocol::{
    entity_data::{Value, serializer},
    item::Stack,
    packets::play::{self, Clientbound as C},
};

#[test]
#[ignore = "daily"]
fn a_client_follows_an_items_and_a_mobs_no_gravity() {
    with_remote_game(|game| {
        let builtin = game.client.remote.as_ref().unwrap().builtin.clone();
        let types = builtin.get("minecraft:entity_type").unwrap();
        let stone = builtin
            .get("minecraft:item")
            .unwrap()
            .id("minecraft:stone")
            .unwrap();
        let add = |id: i32, kind: &str| {
            C::AddEntity(play::AddEntity {
                id,
                uuid: id as u128,
                kind: types.id(kind).unwrap(),
                position: [2.5, 64.0, 2.5],
                velocity: [0.0; 3],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 0,
            })
        };
        let data = |id: i32, values: Vec<(u8, i32, Value)>| {
            C::SetEntityData(play::SetEntityData { id, values })
        };
        let no_gravity = |on: bool| (5, serializer::BOOLEAN, Value::Bool(on));
        // A client level to track the entities in.
        let spawn = play::SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 1,
            game_type: 0,
            previous_game_type: None,
            debug: false,
            flat: false,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        game.handle_packet(C::Login(play::Login {
            player_id: 1,
            hardcore: false,
            levels: vec![spawn.dimension.clone()],
            max_players: 20,
            chunk_radius: 8,
            simulation_distance: 8,
            reduced_debug_info: false,
            show_death_screen: true,
            limited_crafting: false,
            spawn,
            online_mode: false,
            enforces_secure_chat: false,
        }))
        .unwrap();
        game.handle_packet(add(30, "minecraft:item")).unwrap();
        game.handle_packet(add(31, "minecraft:pig")).unwrap();

        // The flag first, then the stack that makes the item.
        let stack = (
            8,
            serializer::ITEM_STACK,
            Value::Item(Some(Stack::new(stone, 1))),
        );
        game.handle_packet(data(30, vec![no_gravity(true), stack]))
            .unwrap();
        game.handle_packet(data(31, vec![no_gravity(true)]))
            .unwrap();
        let item = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
            game.dropped
                .iter()
                .find(|d| d.id == 30)
                .expect("the item was made")
                .no_gravity
        };
        let pig = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
            game.mobs
                .iter()
                .find(|m| m.id == 31)
                .expect("the pig")
                .no_gravity
        };
        assert!(item(game) && pig(game));

        // The flag taken away.
        game.handle_packet(data(30, vec![no_gravity(false)]))
            .unwrap();
        game.handle_packet(data(31, vec![no_gravity(false)]))
            .unwrap();
        assert!(!item(game) && !pig(game));
    });
}
