#![cfg(test)]

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::{run_fresh_cases, with_game};
    use mod_api::{LevelRef, WorldEpoch};
    use std::sync::{Arc, Mutex};
    use sunrise_gameplay_common::game::{
        Dim, native_entity_refs_common::NativeEntityRefsCommonGameExt1,
        weather::WeatherGameExt as _,
    };
    use sunrise_level::world::Weather;

    fn parameters(w: &Weather) -> (i32, i32, i32, bool, bool) {
        (
            w.clear_time,
            w.rain_time,
            w.thunder_time,
            w.raining,
            w.thundering,
        )
    }

    #[test]
    #[ignore = "daily"]
    fn weather_parameters_are_shared_and_strengths_are_local() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::server_side::native_level_weather::tests::weather_parameters_are_shared_and_strengths_are_local",
            || {
                with_game(|game| {
                    game.server.world_root = game.options.game_dir.clone().unwrap();
                    let level = game.level(game.dim.id()).unwrap();
                    let stale =
                        LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
                    let nether = LevelRef::new(level.epoch(), mod_api::DimensionKey::NETHER);

                    assert!(
                        !nether.set_weather(game, 0, 10, true, false),
                        "not yet open"
                    );
                    assert!(level.set_weather(game, -9, -37, true, true));
                    assert_eq!(parameters(&game.world.weather), (-9, -37, -37, true, true));
                    assert!(!stale.set_weather(game, 0, 10, false, false));
                    assert_eq!(parameters(&game.world.weather), (-9, -37, -37, true, true));

                    game.enter_level(Dim::Nether).unwrap();
                    game.enter_level(Dim::Overworld).unwrap();
                    assert_eq!(parameters(&game.world.weather), (-9, -37, -37, true, true));
                    assert!(nether.set_weather(game, 1200, 0, false, false));
                    assert_eq!(parameters(&game.world.weather), (1200, 0, 0, false, false));
                    assert_eq!(
                        game.dim,
                        Dim::Overworld,
                        "a parked origin does not switch levels"
                    );

                    assert!(level.set_weather_levels(game, f32::NEG_INFINITY, f32::INFINITY));
                    let w = &game.world.weather;
                    assert_eq!((w.rain_level, w.thunder_level), ((0.0, 0.0), (1.0, 1.0)));
                    assert!(level.set_weather_levels(game, 0.5, 2.0));
                    assert_eq!(
                        level.weather(game),
                        Some((0.5, 0.5)),
                        "thunder scales with rain"
                    );
                    assert!(!level.set_weather_levels(game, f32::NAN, 0.0));
                    assert!(!level.set_weather_levels(game, 0.0, f32::NAN));
                    assert!(!stale.set_weather_levels(game, 0.0, 0.0));
                    let w = &game.world.weather;
                    assert_eq!(
                        (w.rain_level, w.thunder_level),
                        ((0.5, 0.5), (1.0, 1.0)),
                        "refusals change neither interpolant",
                    );

                    let wet_id = "weather_test:wet";
                    let wet = {
                        let _caller = mod_api::content::crash::enter_native_mod(
                            mod_api::content::crash::ModCall {
                                id: "weather_test",
                                hook: "level_tick_post",
                                part: None,
                            },
                        );
                        game.runtime_level_open(
                            wet_id,
                            mod_api::RuntimeLevel::new(
                                "minecraft:overworld",
                                mod_api::RuntimeGenerator::Flat,
                                1,
                            ),
                        )
                        .unwrap()
                    };
                    game.apply_runtime_levels();
                    assert!(wet.set_weather_levels(game, 0.75, 0.25));
                    assert_eq!(game.dim, Dim::Overworld);
                    assert_eq!(wet.weather(game), Some((0.75, 0.1875)));
                    assert_eq!(level.weather(game), Some((0.5, 0.5)));

                    let wet_dim = Dim::extra(wet_id).unwrap();
                    game.enter_level(wet_dim).unwrap();
                    assert!(game.can_have_weather());
                    let w = &game.world.weather;
                    assert_eq!(
                        (w.rain_level, w.thunder_level),
                        ((0.75, 0.75), (0.25, 0.25))
                    );
                    assert_eq!(parameters(w), (1200, 0, 0, false, false));
                    assert!(level.set_weather_levels(game, 0.25, 0.5));
                    assert_eq!(wet.weather(game), Some((0.75, 0.1875)));

                    game.enter_level(Dim::Overworld).unwrap();
                    let w = &game.world.weather;
                    assert_eq!((w.rain_level, w.thunder_level), ((0.25, 0.25), (0.5, 0.5)));
                    assert_eq!(wet.weather(game), Some((0.75, 0.1875)));
                })
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn local_difficulty_reaches_the_game() {
        if run_fresh_cases(
            "cut_layout::game::server_side::native_level_weather::tests::local_difficulty_reaches_the_game",
            "SUNRISE_LOCAL_DIFFICULTY_CHILD",
            &["local-difficulty"],
        ) {
            return;
        }
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            let here = game.local_difficulty([3.0, 64.0, -7.0]);
            let read = level.local_difficulty(game, [3, 64, -7]).unwrap();
            assert_eq!((read.base, read.effective), (here.base, here.effective));
            assert_eq!(read.special_multiplier(), here.special());
            assert_eq!(stale.local_difficulty(game, [3, 64, -7]), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn weather_veto_does_not_undo_listener_work() {
        type Proposal = (String, i32, i32, bool, bool);
        struct Veto(Arc<Mutex<Vec<Proposal>>>);

        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Veto {
            fn weather_change(
                &self,
                host: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::WeatherChangeEvent<'_>,
            ) -> mod_api::Flow {
                self.0.lock().unwrap().push((
                    event.dimension.to_owned(),
                    event.clear_ticks,
                    event.weather_ticks,
                    event.raining,
                    event.thundering,
                ));
                let level = host.level("minecraft:overworld").unwrap();
                if event.weather_ticks == 23 {
                    assert!(level.set_weather(host, 31, 37, true, false));
                    assert!(level.set_weather_levels(host, 0.5, 0.25));
                } else {
                    assert!(level.set_weather_levels(host, 0.25, 0.75));
                }
                mod_api::Flow::Cancel
            }
        }

        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new("veto", Box::new(Veto(seen.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.enter_level(Dim::Nether).unwrap();
                game.enter_level(Dim::Overworld).unwrap();
                let nether = game.level("minecraft:the_nether").unwrap();
                let weather = &mut game.world.weather;
                weather.clear_time = 7;
                weather.rain_time = 11;
                weather.thunder_time = 13;
                weather.raining = false;
                weather.thundering = true;
                assert!(!nether.set_weather(game, -7, 19, true, false));
                let weather = &game.world.weather;
                assert_eq!(parameters(weather), (7, 11, 13, false, true));
                assert_eq!(
                    (weather.rain_level, weather.thunder_level),
                    ((0.25, 0.25), (0.75, 0.75)),
                    "the vetoed write did not run, and the listener's own write stands",
                );
                assert_eq!(
                    std::mem::take(&mut *seen.lock().unwrap()),
                    [(String::from("minecraft:the_nether"), -7, 19, true, false)]
                );

                let failure = game
                    .as_console(|game| {
                        game.execute_line("weather thunder 23t", game.player_source())
                    })
                    .unwrap_err();
                assert_eq!(failure.message.translation_key(), Some("command.failed"));
                let weather = &game.world.weather;
                assert_eq!(
                    parameters(weather),
                    (31, 37, 37, true, false),
                    "the accepted nested write is not rolled back",
                );
                assert_eq!(
                    (weather.rain_level, weather.thunder_level),
                    ((0.5, 0.5), (0.25, 0.25))
                );
                assert_eq!(
                    std::mem::take(&mut *seen.lock().unwrap()),
                    [(String::from("minecraft:overworld"), 0, 23, true, true)]
                );
            });
        });
    }

    /// `DifficultyInstance`'s formula worked by hand, not through the shared code, for a chunk
    /// that isn't loaded (no inhabited time, no moon) on Hard: 3 x 0.75 = 2.25 (special
    /// multiplier 0.125) before day time 72 000, and 3 x (0.75 + 0.25) = 3.0 (special 0.5)
    /// once the time factor is full at 1 512 000.
    #[test]
    #[ignore = "daily"]
    fn local_difficulty_matches_vanilla_values_for_an_unloaded_chunk() {
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let far = (100_000, -100_000);
            assert!(!game.world.chunks.contains_key(&far));
            let at = [far.0 * 16 + 8, 64, far.1 * 16 + 8];
            game.world.health_rules.difficulty = sunrise_world::food::Difficulty::Hard;
            for (day_time, effective, special) in [(0, 2.25, 0.125), (1_512_000, 3.0, 0.5)] {
                game.world.day_time = day_time;
                let read = level.local_difficulty(game, at).unwrap();
                assert_eq!((read.base, read.effective), (3, effective), "{day_time}");
                assert_eq!(read.special_multiplier(), special, "{day_time}");
            }
        });
    }
}
