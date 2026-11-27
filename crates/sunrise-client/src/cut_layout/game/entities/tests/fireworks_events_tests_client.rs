use crate::game::fireworks::events_tests::start_events;
use crate::game::fireworks::events_tests::stop_events;
#[cfg(feature = "client")]
use crate::game::fireworks::events_tests::twinkling;
use crate::game::fireworks::events_tests::with_game;
use sunrise_core::position::ChunkPos;
#[cfg(feature = "client")]
use sunrise_core::{geometry::Vec3, random::LegacyRandom};
use sunrise_gameplay_common::game::GameGameExt1;
#[cfg(feature = "client")]
use sunrise_items::items::ItemStack;

#[test]
#[ignore = "daily"]
fn a_falling_rocket_hits_the_ground_and_one_in_water_splashes() {
    with_game(|game| {
        // Commands and collisions need resident terrain for both rocket columns.
        // Use the shared chunk fixture and the player's real ticket sources.
        sunrise_gameplay::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood(
            game,
            ChunkPos::new(0, 0),
        );
        game.build_registries();
        game.set_allow_commands(true);
        game.update_chunk_tickets();
        assert!(game.run_command("setblock 6 199 10 stone"));
        // Keep the falling rocket submerged through the sixth move: one
        // floating source block ends at y = 200, above its final position.
        assert!(game.run_command("setblock 20 199 10 water"));
        assert!(game.run_command("setblock 20 200 10 water"));
        for x in [6, 20] {
            assert!(game.run_command(&format!(
                "summon firework_rocket {x}.5 202 10.5 {{LifeTime:100,HasBeenShot:1b,ShotAtAngle:1b,Motion:[0d,-0.5d,0d]}}"
            )));
        }
        game.start_audio_capture();
        start_events();
        let mut falls = Vec::new();
        for _ in 0..6 {
            game.tick_rockets();
            falls.push(game.rockets[0].fall_distance);
        }
        let events: Vec<_> = sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
            .with(|v| {
                std::mem::take(v.borrow_mut().as_mut().unwrap())
                    .into_iter()
                    .map(|e| (e.0, e.1[0].floor() as i32))
                    .filter(|e| matches!(e.0, "minecraft:hit_ground" | "minecraft:splash"))
                    .collect()
            });
        stop_events();
        // The fourth move ends on the stone without a collision; the fifth lands.
        assert_eq!(falls, [0.5, 1.0, 1.5, 2.0, 0.0, 0.0]);
        assert_eq!(game.rockets[1].fall_distance, 0.0);
        assert_eq!(
            events,
            [("minecraft:splash", 20), ("minecraft:hit_ground", 6)]
        );
        assert_eq!(
            game.server
                .pending_sounds
                .iter()
                .filter(|s| s.event == "minecraft:entity.generic.splash")
                .count(),
            1
        );
        game.stop_audio_capture();
    });
}

/// `FireworkParticles.Starter.playSound`: a silent rocket launches quietly (as
/// it already did) and bursts quietly too, drawing its sparks and flash only.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_silent_rocket_bursts_without_blast_or_twinkle() {
    with_game(|game| {
        let stack = ItemStack::new("minecraft:firework_rocket", 1);
        let mut launching = game.new_rocket(
            Vec3::new(6.5, 201.5, 10.5),
            &stack,
            false,
            &mut LegacyRandom::new(0),
        );
        launching.silent = true;
        launching.lifetime = 100;
        game.rockets.push(launching);
        game.start_audio_capture();
        game.tick_rockets();
        let heard = std::mem::take(&mut game.server.pending_sounds);
        game.stop_audio_capture();
        assert!(
            heard
                .iter()
                .all(|s| s.event != "minecraft:entity.firework_rocket.launch"),
            "a silent rocket launches quietly: {heard:?}"
        );
    });
    crate::game::client_side::fireworks_client::tests::assert_client_silent_rocket_burst_without_blast_or_twinkle(
        twinkling(),
    );
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
