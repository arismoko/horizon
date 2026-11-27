#![cfg(any())]

use crate::game::client_side::item_entities_client::thrown_items_tests::with_remote_game;
use sunrise_gameplay_common::game::remote::frozen_screen_effects_tests::JAVA_FREEZING;
use sunrise_gameplay_common::game::remote::frozen_screen_effects_tests::JAVA_UNFREEZING;
use sunrise_protocol::packets::play::{self, Clientbound as C};

#[test]
#[ignore = "daily"]
fn a_frozen_level_eases_the_vignette_as_vanillas_does() {
    with_remote_game(|game| {
        let spawn = play::SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            game_type: 1,
            previous_game_type: None,
            debug: false,
            flat: false,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        game.handle_packet(C::Login(play::Login {
            player_id: 7,
            hardcore: false,
            levels: vec![spawn.dimension.clone()],
            max_players: 1,
            chunk_radius: 2,
            simulation_distance: 2,
            reduced_debug_info: false,
            show_death_screen: true,
            limited_crafting: false,
            spawn,
            online_mode: false,
            enforces_secure_chat: false,
        }))
        .unwrap();
        // Java's camera: the eye at 176.62 over an open sky at noon.
        game.handle_packet(C::PlayerPosition(play::PlayerPosition {
            teleport: 1,
            position: [8.5, 175.0, 8.5],
            delta: [0.0; 3],
            yaw: 180.0,
            pitch: -35.0,
            relatives: 0,
        }))
        .unwrap();
        game.world.day_time = 6000;
        let mut chunk = sunrise_level::chunk::Chunk::empty(0, 0);
        chunk.lit = true;
        game.side.streaming.insert_remote(&mut game.world, chunk);
        let ticking = |frozen| C::TickingState(play::TickingState { rate: 20.0, frozen });
        let bits = |game: &crate::game::ClientGame<'_>| {
            game.client.screen_fx.vignette_brightness.to_bits()
        };

        game.side.screen_fx.vignette_brightness = f32::from_bits(JAVA_FREEZING[0]);
        game.handle_packet(ticking(true)).unwrap();
        assert!(game.clock.frozen());
        for (tick, want) in JAVA_FREEZING.iter().enumerate().skip(1) {
            game.tick_remote().unwrap();
            assert_eq!(bits(game), *want, "frozen tick {tick}");
        }

        game.side.screen_fx.vignette_brightness = f32::from_bits(JAVA_UNFREEZING[0]);
        game.tick_remote().unwrap();
        assert_eq!(bits(game), JAVA_UNFREEZING[1], "the last frozen tick");
        game.handle_packet(ticking(false)).unwrap();
        assert!(!game.clock.frozen());
        for (tick, want) in JAVA_UNFREEZING.iter().enumerate().skip(2) {
            game.tick_remote().unwrap();
            assert_eq!(bits(game), *want, "running tick {tick}");
        }
    });
}
