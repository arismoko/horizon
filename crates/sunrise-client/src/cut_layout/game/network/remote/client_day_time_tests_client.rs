#[test]
#[ignore = "daily"]
fn client_day_time_reads_overworld_clock_across_packets_and_levels() {
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use mod_api::LevelRef;
    use std::sync::Arc;
    use sunrise_protocol::{
        connection::Pipe,
        packets::play::{self, Clientbound as C},
    };

    with_client_game(|game| {
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "ClockReadTest").unwrap();
        let mut registries = (*game.client.remote.as_ref().unwrap().registries).clone();
        let dimensions = vec![
            ("minecraft:overworld".into(), None),
            ("minecraft:the_end".into(), None),
            ("minecraft:the_nether".into(), None),
        ];
        registries
            .registries
            .insert("minecraft:world_clock".into(), dimensions[..2].to_vec());
        registries
            .registries
            .insert("minecraft:dimension_type".into(), dimensions);
        let registries = Arc::new(registries);
        game.set_registries(registries.clone()).unwrap();
        let mut spawn = play::SpawnInfo {
            dimension_type: 1,
            dimension: "minecraft:the_end".into(),
            seed: 0,
            game_type: 1,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        let login = |spawn| {
            C::Login(play::Login {
                player_id: 7,
                hardcore: false,
                levels: vec!["minecraft:overworld".into(), "minecraft:the_end".into()],
                max_players: 1,
                chunk_radius: 2,
                simulation_distance: 2,
                reduced_debug_info: false,
                show_death_screen: true,
                limited_crafting: false,
                spawn,
                online_mode: false,
                enforces_secure_chat: false,
            })
        };
        let state = |clock, total_ticks, partial_tick, rate| play::ClockState {
            clock,
            total_ticks,
            partial_tick,
            rate,
        };
        game.handle_packet(login(spawn.clone())).unwrap();
        let end = game.level("minecraft:the_end").unwrap();
        let wrong_level = LevelRef::new(end.epoch(), mod_api::DimensionKey::OVERWORLD);
        assert_eq!(wrong_level.day_time(game), None);
        game.handle_packet(C::SetTime(play::SetTime {
            game_time: 100,
            clocks: vec![state(0, 30_000, 0.0, 1.0), state(1, 100, 0.0, 1.0)],
        }))
        .unwrap();
        assert_eq!(end.day_time(game), Some(30_000));
        assert_eq!(
            game.world.day_time, 100,
            "the sky follows the End default clock"
        );

        // An Overworld-only update must not select it as the End sky clock.
        game.handle_packet(C::SetTime(play::SetTime {
            game_time: 104,
            clocks: vec![state(0, 40_000, 0.75, 0.5)],
        }))
        .unwrap();
        assert_eq!(end.day_time(game), Some(40_000));
        assert_eq!(game.world.day_time, 104);
        game.tick_remote_level().unwrap();
        assert_eq!(end.day_time(game), Some(40_001));
        assert_eq!(game.world.day_time, 105);
        game.tick_remote_level().unwrap();
        assert_eq!(end.day_time(game), Some(40_001));
        assert_eq!(game.world.day_time, 106);

        // Empty time syncs advance every retained clock by the game-time delta.
        game.handle_packet(C::SetTime(play::SetTime {
            game_time: 110,
            clocks: vec![],
        }))
        .unwrap();
        assert_eq!(end.day_time(game), Some(40_003));
        assert_eq!(game.world.day_time, 110);
        // A paused clock stays put even while the client's game time runs.
        game.handle_packet(C::SetTime(play::SetTime {
            game_time: 110,
            clocks: vec![state(0, 50_000, 0.25, 0.0)],
        }))
        .unwrap();
        game.tick_remote_level().unwrap();
        assert_eq!(end.day_time(game), Some(50_000));
        assert_eq!(game.world.day_time, 111);

        // A frozen world ticks neither clock; packets still catch both up.
        game.handle_packet(C::SetTime(play::SetTime {
            game_time: 111,
            clocks: vec![state(0, 60_000, 0.0, 2.0)],
        }))
        .unwrap();
        game.handle_packet(C::TickingState(play::TickingState {
            rate: 20.0,
            frozen: true,
        }))
        .unwrap();
        game.tick_remote_level().unwrap();
        assert_eq!(game.world.game_time, 111);
        assert_eq!(end.day_time(game), Some(60_000));
        assert_eq!(game.world.day_time, 111);
        game.handle_packet(C::SetTime(play::SetTime {
            game_time: 114,
            clocks: vec![],
        }))
        .unwrap();
        assert_eq!(end.day_time(game), Some(60_006));
        assert_eq!(game.world.day_time, 114);
        // Backward corrections use the same signed delta and fractional state.
        game.handle_packet(C::SetTime(play::SetTime {
            game_time: 113,
            clocks: vec![],
        }))
        .unwrap();
        assert_eq!(end.day_time(game), Some(60_004));
        assert_eq!(game.world.day_time, 113);
        game.handle_packet(C::TickingState(play::TickingState {
            rate: 20.0,
            frozen: false,
        }))
        .unwrap();

        // Same-level respawn keeps the epoch and clocks, but replaces the
        // local player and its mood handler, with or without entity data.
        for keep in [0, 2] {
            game.client.ambience.moodiness = 0.375;
            game.handle_packet(C::Respawn(play::Respawn {
                spawn: spawn.clone(),
                keep,
            }))
            .unwrap();
            assert_eq!(game.client.ambience.moodiness, 0.0);
            assert_eq!(end.day_time(game), Some(60_004));
        }
        spawn.dimension_type = 0;
        spawn.dimension = "minecraft:overworld".into();
        game.handle_packet(C::Respawn(play::Respawn {
            spawn: spawn.clone(),
            keep: 0,
        }))
        .unwrap();
        assert_eq!(end.day_time(game), None);
        let overworld = game.level("minecraft:overworld").unwrap();
        assert_eq!(overworld.day_time(game), Some(60_004));
        assert_eq!(
            game.world.day_time, 60_004,
            "select the new default from retained clocks"
        );
        game.tick_remote_level().unwrap();
        assert_eq!(overworld.day_time(game), Some(60_006));
        assert_eq!(game.world.day_time, 60_006);

        // A dimension without a default clock must not select the prior one.
        spawn.dimension_type = 2;
        spawn.dimension = "minecraft:the_nether".into();
        game.handle_packet(C::Respawn(play::Respawn {
            spawn: spawn.clone(),
            keep: 0,
        }))
        .unwrap();
        let nether = game.level("minecraft:the_nether").unwrap();
        assert_eq!(overworld.day_time(game), None);
        assert_eq!(nether.day_time(game), Some(60_006));
        assert_eq!(game.world.day_time, 0);

        // Login starts a new listener epoch, even on the same connection.
        game.handle_packet(login(spawn.clone())).unwrap();
        assert_eq!(nether.day_time(game), None);
        let fresh = game.level("minecraft:the_nether").unwrap();
        assert_eq!(fresh.day_time(game), Some(0));
        game.handle_packet(C::SetTime(play::SetTime {
            game_time: 1,
            clocks: vec![],
        }))
        .unwrap();
        assert_eq!(
            fresh.day_time(game),
            Some(1),
            "the new clock anchor and rate are fresh too"
        );
        game.handle_packet(C::SetTime(play::SetTime {
            game_time: 1,
            clocks: vec![state(0, 7_000, 0.0, 1.0)],
        }))
        .unwrap();
        assert_eq!(fresh.day_time(game), Some(7_000));
        assert_eq!(game.world.day_time, 0);
        game.client_logging_out();
        assert_eq!(fresh.day_time(game), None);
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "ClockReadTest").unwrap();
        assert_eq!(fresh.day_time(game), None);
        game.set_registries(registries).unwrap();
        game.handle_packet(login(spawn)).unwrap();
        let reconnected = game.level("minecraft:the_nether").unwrap();
        assert_eq!(fresh.day_time(game), None);
        assert_eq!(reconnected.day_time(game), Some(0));
    });
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
