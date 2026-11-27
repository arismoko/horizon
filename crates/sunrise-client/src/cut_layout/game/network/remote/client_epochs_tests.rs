#[test]
#[ignore = "daily"]
fn client_epochs_follow_level_replacement_and_logout() {
    use mod_api::{CrosshairTarget, EntityRef};
    use sunrise_client::level_loading::Reason;
    use sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry;
    use sunrise_core::geometry::Vec3;
    use sunrise_protocol::{
        connection::Pipe,
        packets::play::{Clientbound, Respawn, SpawnInfo},
    };
    use sunrise_sim::motion::PlayerMotion;
    use {
        crate::game::server_side::native_entity_refs::tests::with_client_game,
        sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycle,
    };

    with_client_game(|game| {
        game.native_lifecycle = NativeLifecycle::default();
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "EpochTest").unwrap();
        // A transport alone has no ClientLevel yet.
        assert!(game.native_lifecycle.epoch().is_none());
        let mut spawn = SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            game_type: 1,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
        let joined = game.native_lifecycle.epoch().unwrap();
        let old = EntityRef::new(joined, 12);
        let remote = game.client.remote.as_mut().unwrap();
        remote.menu.set(Some((7, "minecraft:generic_9x3".into())));
        let open_generation = remote.menu.generation;
        game.handle_packet(Clientbound::Respawn(Respawn {
            spawn: spawn.clone(),
            keep: 0,
        }))
        .unwrap();
        assert_eq!(game.native_lifecycle.epoch(), Some(joined));
        let remote = game.client.remote.as_mut().unwrap();
        assert!(remote.menu.as_ref().is_none());
        assert_ne!(remote.menu.generation, open_generation);
        let inventory_generation = remote.menu.generation;
        game.handle_packet(Clientbound::Respawn(Respawn {
            spawn: spawn.clone(),
            keep: 0,
        }))
        .unwrap();
        assert_eq!(game.native_lifecycle.epoch(), Some(joined));
        let remote = game.client.remote.as_mut().unwrap();
        assert!(remote.menu.as_ref().is_none());
        assert_ne!(remote.menu.generation, inventory_generation);
        remote.menu.set(Some((7, "minecraft:generic_9x3".into())));
        assert_ne!(remote.menu.generation, open_generation);
        assert_ne!(remote.menu.generation, inventory_generation);

        // A different level key replaces the world even with the same
        // dimension type; IDs can be reused but old handles cannot.
        spawn.dimension = "test:other_overworld".into();
        game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Respawn, Reason::Other);
        let replaced = game.native_lifecycle.epoch().unwrap();
        assert_ne!(replaced, joined);
        let current = EntityRef::new(replaced, 12);
        game.client.remote.as_mut().unwrap().logged_in = true;
        game.spawn_entity(12, "minecraft:pig", [0.0, 1.0, 2.0], [0.0; 3]);
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.yaw = 0.0;
        game.pitch = 0.0;
        game.refresh_client_pick(1.0);
        let Some(CrosshairTarget::Entity { entity, .. }) = game.crosshair_target() else {
            panic!("the crosshair hits the pig in front of the player");
        };
        assert_eq!(entity, current);
        assert!(game.native_client_entity_target(current).is_some());
        assert!(game.native_client_entity_target(old).is_none());

        game.client_logging_out();
        assert!(game.native_lifecycle.epoch().is_none());
        assert!(game.crosshair_target().is_none());
        game.client_logging_out();
        assert!(game.native_lifecycle.epoch().is_none());
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "EpochTest").unwrap();
        assert!(game.native_lifecycle.epoch().is_none());
        game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
        assert_ne!(game.native_lifecycle.epoch(), Some(replaced));
        game.client_logging_out();
        assert!(game.native_lifecycle.epoch().is_none());
    });
}

#[test]
#[ignore = "daily"]
fn a_dimension_change_replaces_a_signed_world_border() {
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client;
    use sunrise_level::worldborder::WorldBorder;
    use sunrise_protocol::packets::play::{self, Clientbound as C};

    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::remote::tests::client_epochs_tests::a_dimension_change_replaces_a_signed_world_border",
        || {
            with_client_game(|client| {
                let _transport = logged_client(client);
                let mut spawn = super::spawn_menu_test(client);
                let initialize = |center, size, clip| {
                    C::InitializeBorder(play::InitializeBorder {
                        center_x: center,
                        center_z: center,
                        old_size: size,
                        new_size: size,
                        lerp_time: 0,
                        absolute_max_size: clip,
                        warning_blocks: 7,
                        warning_time: 50,
                    })
                };
                client.handle_packet(initialize(0.0, -1.5, 100)).unwrap();
                assert_eq!(client.border.size(), -1.5);
                client
                    .handle_packet(C::Respawn(play::Respawn {
                        spawn: spawn.clone(),
                        keep: 0,
                    }))
                    .unwrap();
                assert_eq!(client.border.size(), -1.5);

                spawn.dimension = "test:other_overworld".into();
                client
                    .handle_packet(C::Respawn(play::Respawn { spawn, keep: 3 }))
                    .unwrap();
                assert_eq!(client.border, WorldBorder::default());
                client.handle_packet(initialize(0.5, 20.0, 5)).unwrap();
                assert_eq!(client.border.center(), [0.5, 0.5]);
                assert_eq!(client.border.size(), 20.0);
                assert_eq!(client.border.bounds(), [-5.0, -5.0, 5.0, 5.0]);
                assert_eq!(
                    (client.border.warning_distance, client.border.warning_time),
                    (7, 50)
                );

                // This extent is valid only with the clip limit supplied by the packet.
                client.handle_packet(initialize(0.5, -2.0, 0)).unwrap();
                assert_eq!(client.border.size(), -2.0);
                assert_eq!(client.border.bounds(), [0.0; 4]);
                client.handle_packet(initialize(0.0, -2.0, 100)).unwrap();
                assert_eq!(client.border.center(), [0.5, 0.5]);
                assert_eq!(client.border.absolute_max_size(), 0);
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_dimension_change_uses_the_destination_weather_strengths() {
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use sunrise_client_gameplay::game::{
        ClientGame, client_side::native_client_session::tests::logged_client,
    };
    use sunrise_gameplay::game::{ServerGame, native_guest_turn_tests::with_guests};
    use sunrise_gameplay_common::game::weather::WeatherGameExt as _;
    use sunrise_protocol::{
        buf::Reader,
        packets::play::{self, Clientbound as C},
    };
    use sunrise_sim::lan::{Out, PlayerSender, hub::Hub};

    const WET: &str = "test:wet_overworld";
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::remote::tests::client_epochs_tests::a_dimension_change_uses_the_destination_weather_strengths",
        || {
            with_guests(|server| {
                let mut host = server.server.lan.take().unwrap();
                let (tx, rx) = std::sync::mpsc::channel();
                host.hub.guests.update(0, |guest| {
                    guest.sender = PlayerSender::new(tx, std::sync::Arc::default());
                });
                // Both levels use the Overworld dimension type and the same clear WeatherData.
                let dry = server.world.weather;
                assert!(!dry.raining && !dry.thundering);
                with_client_game(|client| {
                    let _transport = logged_client(client);
                    let mut spawn = super::spawn_menu_test(client);
                    let transfer = |client: &mut ClientGame<'_>,
                                    hub: &mut Hub,
                                    server: &mut ServerGame<'_>,
                                    dimension: &str| {
                        hub.set_dimension(dimension);
                        let ctx = server.server_ctx();
                        let change =
                            hub.begin_level_change(0, [0.5, 64.0, 0.5], 0.0, None, 0, &ctx);
                        hub.finish_level_change(0, change, false, &ctx);
                        for out in rx.try_iter() {
                            let bodies = match out {
                                Out::Packet(body) => vec![body],
                                Out::Packets(bodies) => bodies,
                                Out::Disconnect(_) => panic!("player disconnected during travel"),
                            };
                            for body in bodies {
                                let mut reader = Reader::new(&body);
                                let id = reader.varint().unwrap();
                                let packet = C::decode(id, reader.rest()).unwrap();
                                if matches!(packet, C::Respawn(_) | C::GameEvent(_)) {
                                    client.handle_packet(packet).unwrap();
                                }
                            }
                        }
                        assert_eq!(client.client.remote.as_ref().unwrap().level, dimension);
                        assert!(client.can_have_weather());
                    };
                    // The 0.2f32 strength exceeds the double threshold in Level.isRaining.
                    server.world.weather.set_levels(0.2, 0.5);
                    transfer(client, &mut host.hub, server, WET);
                    assert_eq!(client.world.weather.rain_level, (0.2, 0.2));
                    // PlayerList.sendLevelInfo uses Level.getThunderLevel(1), which scales by rain.
                    assert_eq!(client.world.weather.thunder_level, (0.1, 0.1));

                    spawn.dimension = WET.into();
                    client
                        .handle_packet(C::Respawn(play::Respawn { spawn, keep: 0 }))
                        .unwrap();
                    assert_eq!(client.world.weather.rain_level, (0.2, 0.2));

                    server.world.weather = dry;
                    transfer(client, &mut host.hub, server, "minecraft:overworld");
                    assert_eq!(client.weather_levels(1.0), (0.0, 0.0));
                });
            });
        },
    );
}
