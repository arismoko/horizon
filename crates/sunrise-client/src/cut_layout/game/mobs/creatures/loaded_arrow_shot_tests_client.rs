use crate::game::creatures::loaded_arrow_shot_tests::start_events;
use crate::game::creatures::loaded_arrow_shot_tests::stop_events;
use crate::game::creatures::loaded_arrow_shot_tests::take_shots;
use crate::game::creatures::loaded_arrow_shot_tests::with_game;

/// #14318: an angled rocket falling onto stone builds up its fall and posts
/// HIT_GROUND when it lands (`Entity.checkFallDamage`); one falling into water
/// splashes once (`Entity.doWaterSplashEffect`: the sound and SPLASH) and lands
/// with no fall.
#[test]
#[ignore = "daily"]
fn a_falling_rocket_hits_the_ground_and_one_in_water_splashes() {
    with_game(|game| {
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

/// `projectile_shoot` on its first tick, once; one loaded shot posts none.
/// `FireworkRocketEntity.tick`'s launch sound skips a silent one.
#[test]
#[ignore = "daily"]
fn a_rocket_posts_its_shot_once_and_a_silent_one_launches_quietly() {
    with_game(|game| {
        let launch = "minecraft:entity.firework_rocket.launch";
        assert!(game.run_command(
            "summon firework_rocket 6.5 201.5 10.5 {LifeTime:100,Silent:1b,NoGravity:1b}"
        ));
        assert!(
            game.run_command("summon firework_rocket 9.5 201.5 10.5 {LifeTime:100,HasBeenShot:1b}")
        );
        assert!(
            game.rockets
                .iter()
                .all(|r| r.has_been_shot == (r.position.x == 9.5))
        );
        game.start_audio_capture();
        start_events();
        game.tick_rockets();
        let first = take_shots();
        game.tick_rockets();
        let second = take_shots();
        stop_events();
        assert_eq!(
            first,
            vec![("minecraft:projectile_shoot", [6.5, 201.5, 10.5], None)]
        );
        assert!(second.is_empty());
        assert!(game.rockets.iter().all(|r| r.has_been_shot));
        // Only the rocket that isn't silent launches with a sound.
        assert_eq!(
            game.server
                .pending_sounds
                .iter()
                .filter(|s| s.event == launch)
                .count(),
            1
        );
        game.stop_audio_capture();
    });
}
