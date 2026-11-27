#![cfg(test)]
#[cfg(test)]
pub(crate) mod player_status_tests {

    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;
    use sunrise_sim::motion::PlayerMotion;

    pub(crate) fn stand_player(game: &mut sunrise_gameplay_common::game::CommonGame<'_>) {
        for chunk in [(-1, -1), (-1, 0), (0, -1), (0, 0)] {
            game.world
                .chunks
                .insert(chunk, Arc::new(Chunk::empty(chunk.0, chunk.1)));
        }
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    }

    pub(crate) fn rain_on_the_player(
        game: &mut sunrise_gameplay_common::game::CommonGame<'_>,
        raining: bool,
    ) {
        let plains = game.res.biomes().id("minecraft:plains");
        let bottom = game.world.bottom_y() >> 2;
        for x in [-1, 0] {
            for y in [15, 16] {
                for z in [-1, 0] {
                    let chunk = game.world.chunks.get_mut(&(x >> 2, z >> 2)).unwrap();
                    Arc::make_mut(chunk).set_biome_cell(x, y - bottom, z, plains);
                }
            }
        }
        game.world.weather.raining = raining;
        game.world.weather.settle_levels();
    }

    use sunrise_level::{chunk::Chunk, effects::EffectInstance};

    /// On a server's client the player booleans follow what the server sent:
    /// `Entity.isCurrentlyGlowing` is shared flag 6 alone (a client never asks the Glowing
    /// effect), `Entity.isVehicle` the passenger list of `SetPassengers`, and the rain the
    /// chunks and weather it was sent.
    #[test]
    #[ignore = "daily"]
    fn pack_status_on_a_server_follows_what_it_sent() {
        use sunrise_protocol::{
            entity_data::{Value, serializer},
            packets::play::{self, Clientbound as C},
        };
        use {
            crate::game::server_side::native_entity_refs::tests::with_client_game,
            sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client,
        };
        with_client_game(|game| {
            let _server = logged_client(game);
            let own = game.client.remote.as_ref().unwrap().entity_id;
            stand_player(game);
            let flags = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>, flags: i8| {
                game.handle_packet(C::SetEntityData(play::SetEntityData {
                    id: own,
                    values: vec![(0, serializer::BYTE, Value::Byte(flags))],
                }))
                .unwrap();
            };
            assert!(!game.pack_player_status().glowing);
            flags(game, 0x40);
            assert!(game.pack_player_status().glowing, "flag 6");
            flags(game, 0);
            game.potions
                .effects
                .add(EffectInstance::new("glowing", 200, 0));
            assert!(
                !game.pack_player_status().glowing,
                "the effect without flag 6"
            );
            flags(game, 0x40);
            assert!(game.pack_player_status().glowing);
            flags(game, 0);
            assert!(!game.pack_player_status().glowing, "flag 6 cleared");

            let kind = |game: &sunrise_client_gameplay::game::ClientGame<'_>, name: &str| {
                let remote = game.client.remote.as_ref().unwrap();
                let types = remote.builtin.get("minecraft:entity_type").unwrap();
                types.id(name).unwrap()
            };
            // A pig, which the client seats as well, and a text display, which servers hang
            // over a player's head and only the server's list holds.
            for (id, name) in [(77, "minecraft:pig"), (78, "minecraft:text_display")] {
                let kind = kind(game, name);
                game.handle_packet(C::AddEntity(play::AddEntity {
                    id,
                    uuid: id as u128,
                    kind,
                    position: [0.5, 65.8, 0.5],
                    velocity: [0.0; 3],
                    pitch: 0,
                    yaw: 0,
                    head_yaw: 0,
                    data: 0,
                }))
                .unwrap();
                assert!(!game.pack_player_status().ridden);
                game.handle_packet(C::SetPassengers(play::SetPassengers {
                    vehicle: own,
                    passengers: vec![id],
                }))
                .unwrap();
                assert!(game.pack_player_status().ridden, "{name} listed");
                game.handle_packet(C::SetPassengers(play::SetPassengers {
                    vehicle: own,
                    passengers: Vec::new(),
                }))
                .unwrap();
                assert!(!game.pack_player_status().ridden, "an empty list");
            }

            assert!(!game.pack_player_status().wet);
            rain_on_the_player(game, true);
            assert!(game.pack_player_status().wet, "rain");
            rain_on_the_player(game, false);
            assert!(!game.pack_player_status().wet);
        });
    }
    /// A new `LocalPlayer` starts with default shared flags: after a login, and after a respawn
    /// without `KEEP_ENTITY_DATA`, `Entity.isCurrentlyGlowing` is false until the server sets
    /// flag 6 again; a respawn that keeps the data keeps it. `LivingEntity.isAlive` follows the
    /// health the server sent, and the new player is alive.
    #[test]
    #[ignore = "daily"]
    fn pack_status_starts_over_with_a_new_local_player() {
        use crate::game::server_side::native_entity_refs::tests::with_client_game;
        use sunrise_protocol::{
            entity_data::{Value, serializer},
            packets::play::{self, Clientbound as C},
        };
        with_client_game(|game| {
            let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(pipe, "Steve").unwrap();
            let spawn = || play::SpawnInfo {
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
            let login = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>| {
                game.handle_packet(C::Login(play::Login {
                    player_id: 7,
                    hardcore: false,
                    levels: vec!["minecraft:overworld".into()],
                    max_players: 20,
                    chunk_radius: 8,
                    simulation_distance: 8,
                    reduced_debug_info: false,
                    show_death_screen: true,
                    limited_crafting: false,
                    spawn: spawn(),
                    online_mode: false,
                    enforces_secure_chat: false,
                }))
                .unwrap();
            };
            let mut teleport = 0;
            let mut place = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>| {
                teleport += 1;
                game.handle_packet(C::PlayerPosition(play::PlayerPosition {
                    teleport,
                    position: [0.5, 65.0, 0.5],
                    delta: [0.0; 3],
                    yaw: 0.0,
                    pitch: 0.0,
                    relatives: 0,
                }))
                .unwrap();
            };
            let glow = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>| {
                game.handle_packet(C::SetEntityData(play::SetEntityData {
                    id: 7,
                    values: vec![(0, serializer::BYTE, Value::Byte(0x40))],
                }))
                .unwrap();
            };
            let respawn = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>, keep| {
                game.handle_packet(C::Respawn(play::Respawn {
                    spawn: spawn(),
                    keep,
                }))
                .unwrap();
            };
            login(game);
            place(game);
            let status = game.pack_player_status();
            assert!(status.alive && !status.glowing, "{status:?}");
            glow(game);
            assert!(game.pack_player_status().glowing, "flag 6");

            respawn(game, 2);
            place(game);
            assert!(
                game.pack_player_status().glowing,
                "KEEP_ENTITY_DATA keeps flag 6"
            );

            game.handle_packet(C::SetHealth(play::SetHealth {
                health: 0.0,
                food: 20,
                saturation: 5.0,
            }))
            .unwrap();
            assert!(!game.pack_player_status().alive, "dead");
            respawn(game, 0);
            place(game);
            let status = game.pack_player_status();
            assert!(!status.glowing, "a respawn without the data resets flag 6");
            assert!(status.alive, "the new player is alive");

            glow(game);
            login(game);
            place(game);
            assert!(!game.pack_player_status().glowing, "a login resets flag 6");
        });
    }
}
