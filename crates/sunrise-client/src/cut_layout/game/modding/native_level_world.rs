#![cfg(test)]
use sunrise_gameplay_common::game::Dim;
use sunrise_gameplay_common::game::native_rain_read::weather_flags;

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use core::ops::ControlFlow;
    use mod_api::{GameRuleChangedEvent, GameRuleValue, Hooks, LevelHost, ModEntry, ServerHost};
    use mod_platform::testing::with_mods;
    use std::sync::{Arc, Mutex};
    use sunrise_game_data::gamerules;
    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game_side;
    #[cfg(feature = "client")]
    use sunrise_level::chunk::Chunk;

    /// Read the recorded generator, including absence and stale-handle cases.
    #[test]
    #[ignore = "daily"]
    fn a_level_names_the_chunk_generator_it_was_built_with() {
        with_game(|game| {
            let overworld = game.level("minecraft:overworld").unwrap();
            // The harness skips terrain construction, so seed the value make_streaming records.
            let debug = mod_api::Identifier::parse("minecraft:debug").unwrap();
            game.world.generator_type = Some(debug.clone());
            assert_eq!(overworld.generator_type(game), Some(debug));
            game.world.generator_type = None;
            assert_eq!(overworld.generator_type(game), None);
            let stale = mod_api::LevelRef::new(
                mod_api::WorldEpoch::new(overworld.epoch().get() + 1),
                overworld.dimension(),
            );
            game.world.generator_type =
                Some(mod_api::Identifier::parse("minecraft:noise").unwrap());
            assert_eq!(stale.generator_type(game), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn dimension_shape_reads_current_parked_and_stale_levels() {
        use mod_api::{Authority, Host, LevelDimensionShape};
        use sunrise_gameplay_common::game::Dim;
        use sunrise_level::chunk::LevelShape;

        with_game(|game| {
            game.world.shape = LevelShape::new(-32, 320, 256, true, false).unwrap();
            let overworld = game.level("minecraft:overworld").unwrap();
            let expected = LevelDimensionShape::new(-32, 320, 256, true, false);
            let host: &dyn Host = game;
            assert_eq!(overworld.dimension_shape(host), Some(expected));
            game.enter_level(Dim::Nether).unwrap();
            game.world.shape = LevelShape::new(0, 256, 128, false, true).unwrap();
            let nether = game.level("minecraft:the_nether").unwrap();
            let host: &dyn Host = game;
            assert_eq!(overworld.dimension_shape(host), Some(expected));
            assert_eq!(
                nether.dimension_shape(host),
                Some(LevelDimensionShape::new(0, 256, 128, false, true))
            );
            game.native_lifecycle
                .begin_open(Some(Authority::IntegratedServer));
            let host: &dyn Host = game;
            assert_eq!(overworld.dimension_shape(host), None);
            assert_eq!(nether.dimension_shape(host), None);
        });
    }

    /// A chunk of air with `biome` in every cell: full sky light everywhere.
    #[cfg(feature = "client")]
    fn chunk_in(game: &sunrise_gameplay::game::ServerGame<'_>, x: i32, biome: &str) -> Arc<Chunk> {
        let id = game.res.biomes().id(biome);
        let mut chunk = Chunk::empty(x, 0);
        for qy in 0..chunk.sections.len() as i32 * 4 {
            for (qx, qz) in (0..4).flat_map(|qx| (0..4).map(move |qz| (qx, qz))) {
                chunk.set_biome_cell(qx, qy, qz, id);
            }
        }
        Arc::new(chunk)
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn rain_at_a_position_checks_sky_roof_and_biome() {
        with_game(|game| {
            game.install_live_features();
            for (x, biome) in [
                (0, "minecraft:plains"),
                (1, "minecraft:desert"),
                (2, "minecraft:snowy_plains"),
            ] {
                let chunk = chunk_in(game, x, biome);
                game.world.chunks.insert((x, 0), chunk);
            }
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(3, 100, 3, stone).unwrap();
            let level = game.level(game.dim.id()).unwrap();
            let chunks = game.world.chunks.len();
            let plains = [1, 70, 1];
            let roofed = [3, 70, 3];
            let desert = [17, 70, 1];
            let snowy = [33, 70, 1];
            let unloaded = [16 * 9, 70, 1];

            game.world.weather.rain_level = (1.0, 1.0);
            for (pos, rains) in [
                (plains, true),
                (roofed, false),
                (desert, false),
                (snowy, false),
            ] {
                assert_eq!(game.level_is_raining_at(level, pos), Some(rains), "{pos:?}");
            }
            assert_eq!(game.level_is_raining_at(level, unloaded), None);
            assert_eq!(game.level_is_raining_at(level, [1, 4000, 1]), None);
            game.with_level(|_, host, _| {
                // The level's own weather, wherever a position is (`Level.isRaining`).
                assert_eq!(mod_api::LevelHost::is_raining(&*host), Some(true));
                assert!(host.is_raining_at(plains));
                assert!(!host.is_raining_at(roofed));
                assert!(!host.is_raining_at(desert));
                assert!(!host.is_raining_at(snowy));
                assert!(!host.is_raining_at(unloaded));
            });
            // The side a motion tick is lent answers the same, so a friction part sees it.
            let side = game_side!(game);
            for (pos, rains) in [
                (plains, true),
                (roofed, false),
                (desert, false),
                (snowy, false),
                (unloaded, false),
            ] {
                let at = sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
                assert_eq!(
                    sunrise_sim::motion::SideLevel::is_raining_at(&side, &game.world, at),
                    rains,
                    "{pos:?}"
                );
            }

            let server_view = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                let view = sunrise_gameplay_common::game::native_level_signals::ReadLevel::new(
                    game,
                    level,
                    &game.world,
                );
                mod_api::LevelHost::is_raining(&view)
            };
            // The level view a block part's callback gets (`ReadLevel`) answers too.
            assert_eq!(server_view(game), Some(true));

            game.world.weather.rain_level = (0.0, 0.0);
            assert_eq!(game.level_is_raining_at(level, plains), Some(false));
            game.with_level(|_, host, _| {
                assert_eq!(mod_api::LevelHost::is_raining(&*host), Some(false));
                assert!(!host.is_raining_at(plains));
            });
            assert_eq!(server_view(game), Some(false));
            assert_eq!(game.world.chunks.len(), chunks, "nothing was loaded");
        });
        // A logged-in client reads the weather its server synced through the same view.
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(pipe, "WeatherReadTest").unwrap();
            game.client.remote.as_mut().unwrap().logged_in = true;
            let client = game.level("minecraft:overworld").unwrap();
            let client_view = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
                let view = sunrise_gameplay_common::game::native_level_signals::ReadLevel::new(
                    game,
                    client,
                    &game.world,
                );
                mod_api::LevelHost::is_raining(&view)
            };
            game.world.weather.rain_level = (1.0, 1.0);
            assert_eq!(client_view(game), Some(true));
            game.world.weather.rain_level = (0.0, 0.0);
            assert_eq!(client_view(game), Some(false));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn enabled_features_default_to_vanilla_and_follow_the_world() {
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let flags = game.level_enabled_features(level).unwrap();
            assert!(flags.contains("minecraft:vanilla"));
            assert!(flags.contains("vanilla"));
            assert!(!flags.contains("minecraft:redstone_experiments"));
            game.level.enabled_features = [
                "minecraft:vanilla".to_string(),
                "minecraft:redstone_experiments".to_string(),
            ]
            .into();
            let flags = game.level_enabled_features(level).unwrap();
            assert!(flags.contains("minecraft:redstone_experiments"));
            assert_eq!(flags.iter().count(), 2);
            let ids: Vec<String> = flags.iter().map(str::to_owned).collect();
            game.with_level(|_, host, _| {
                assert_eq!(host.enabled_features().iter().collect::<Vec<_>>(), ids);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn native_mod_reads_registered_bool_rule_by_key() {
        use std::sync::OnceLock;
        static KEY: OnceLock<mod_api::BoolGameRuleKey> = OnceLock::new();
        struct RuleMod;
        #[mod_api::__subscriptions]
        impl Hooks for RuleMod {
            fn register(
                &self,
                registrar: &mut dyn mod_api::Registrar,
                _: &mod_api::RegistrationContext,
            ) {
                registrar.game_rule_category("machines").unwrap();
                let key = registrar
                    .game_rule_bool("enabled", "example:machines", true)
                    .unwrap();
                let _ = KEY.set(key);
            }
        }
        let mods: mod_api::Mods = vec![ModEntry::new("example", Box::new(RuleMod))].into();
        with_mods(mods, || {
            with_game(|game| {
                let registry =
                    Arc::new(sunrise_gameplay_common::game::api_values::game_rule_registry());
                game.level.rule_registry = registry.clone();
                game.ui.rules = gamerules::GameRules::with_registry(registry, &[]);
                let level = game.level("minecraft:overworld").unwrap();
                assert_eq!(level.game_rule_bool(game, *KEY.get().unwrap()), Some(true));
            })
        });
    }

    /// #3316's case: `example:cap` (8, 0..=64) refuses `/gamerule example:cap 70`, takes 12, and
    /// keeps 12 through a save and load of the rules.
    #[test]
    #[ignore = "daily"]
    fn a_mod_int_rule_is_range_checked_by_gamerule_and_saved() {
        use std::sync::OnceLock;
        static KEY: OnceLock<mod_api::IntGameRuleKey> = OnceLock::new();
        struct RuleMod;
        #[mod_api::__subscriptions]
        impl Hooks for RuleMod {
            fn register(
                &self,
                registrar: &mut dyn mod_api::Registrar,
                _: &mod_api::RegistrationContext,
            ) {
                registrar.game_rule_category("machines").unwrap();
                let key = registrar
                    .game_rule_int("cap", "example:machines", 8, 0, 64)
                    .unwrap();
                let _ = KEY.set(key);
            }
        }
        let mods: mod_api::Mods = vec![ModEntry::new("example", Box::new(RuleMod))].into();
        with_mods(mods, || {
            with_game(|game| {
                let registry =
                    Arc::new(sunrise_gameplay_common::game::api_values::game_rule_registry());
                game.level.rule_registry = registry.clone();
                game.ui.rules = gamerules::GameRules::with_registry(registry.clone(), &[]);
                game.load_world_state().unwrap();
                game.set_allow_commands(true);
                game.build_registries();
                let key = *KEY.get().unwrap();
                let level = game.level("minecraft:overworld").unwrap();
                assert_eq!(level.game_rule_int(game, key), Some(8));
                assert!(!game.run_command("gamerule example:cap 70"));
                assert_eq!(level.game_rule_int(game, key), Some(8));
                assert!(game.run_command("gamerule example:cap 12"));
                assert_eq!(level.game_rule_int(game, key), Some(12));
                let saved = game.ui.rules.to_nbt().tag;
                game.ui.rules = gamerules::GameRules::from_nbt_with_registry(&saved, &[], registry);
                assert_eq!(level.game_rule_int(game, key), Some(12));
            })
        });
    }

    /// What a `game_rule_changed` listener read of the whole rule set: the changed rule, whether a
    /// player caused it, and `keep_inventory`, `random_tick_speed` and the rule count it saw.
    type RuleRead = (
        String,
        bool,
        Option<GameRuleValue>,
        Option<GameRuleValue>,
        usize,
    );
    struct RuleReader(Arc<Mutex<Vec<RuleRead>>>);
    #[mod_api::__subscriptions]
    impl Hooks for RuleReader {
        fn game_rule_changed(
            &self,
            host: &mut dyn ServerHost,
            event: &mut GameRuleChangedEvent<'_>,
        ) {
            let (mut inventory, mut tick_speed, mut count) = (None, None, 0);
            let level = host.level("minecraft:overworld").unwrap();
            assert_eq!(
                level.visit_game_rules(host, &mut |id, value| {
                    count += 1;
                    match id {
                        "minecraft:keep_inventory" => inventory = Some(value),
                        "minecraft:random_tick_speed" => tick_speed = Some(value),
                        _ => {}
                    }
                    ControlFlow::Continue(())
                }),
                Ok(ControlFlow::Continue(()))
            );
            self.0.lock().unwrap().push((
                event.rule.to_owned(),
                event.actor.is_some(),
                inventory,
                tick_speed,
                count,
            ));
        }
    }

    /// A listener of a change no player made reads every server rule through its host: the new
    /// value and the others as they stand.
    #[test]
    #[ignore = "daily"]
    fn a_game_rule_listener_reads_the_whole_rule_set() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "rule-reader",
            Box::new(RuleReader(seen.clone())),
        )]
        .into();
        with_mods(mods, || {
            with_game(|game| {
                assert!(
                    game.ui
                        .rules
                        .set("random_tick_speed", gamerules::Value::Int(7))
                );
                let keep_inventory = gamerules::rule_registry()
                    .rule("keep_inventory")
                    .unwrap()
                    .clone();
                game.set_game_rule(&keep_inventory, gamerules::Value::Bool(true), None);
            });
        });
        assert_eq!(
            *seen.lock().unwrap(),
            [(
                "minecraft:keep_inventory".to_owned(),
                false,
                Some(GameRuleValue::Bool(true)),
                Some(GameRuleValue::Int(7)),
                // Vanilla 26.3 `GameRules` has 59, `max_minecart_speed` behind minecart improvements.
                58,
            )]
        );
    }

    /// `level_set_game_rule` writes what `/gamerule` accepts, refuses what it refuses, and
    /// `level_set_clock_time` moves the level's clock as `/time set` does.
    #[test]
    #[ignore = "daily"]
    fn a_mod_sets_game_rules_and_the_clock() {
        use mod_api::WorldEpoch;
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            assert!(level.set_game_rule(game, "keep_inventory", GameRuleValue::Bool(true)));
            assert_eq!(
                game.ui.rules.get("keep_inventory"),
                Some(gamerules::Value::Bool(true))
            );
            assert!(game.level_set_game_rule(
                level,
                "minecraft:random_tick_speed",
                gamerules::Value::Int(9)
            ));
            assert_eq!(
                game.ui.rules.get("random_tick_speed"),
                Some(gamerules::Value::Int(9))
            );
            assert!(game.level_set_game_rule(level, "random_tick_speed", gamerules::Value::Int(9)));
            assert!(!game.level_set_game_rule(level, "missing:rule", gamerules::Value::Bool(true)));
            assert!(!game.level_set_game_rule(level, "keep_inventory", gamerules::Value::Int(1)));
            assert!(!game.level_set_game_rule(
                level,
                "random_tick_speed",
                gamerules::Value::Int(-1)
            ));
            assert_eq!(
                game.ui.rules.get("random_tick_speed"),
                Some(gamerules::Value::Int(9))
            );

            assert!(game.level_set_clock_time(level, 13_000));
            assert_eq!(game.level_clock_time(level), Some(13_000));
            assert!(game.level_set_clock_time(level, 13_000));
            assert!(!game.level_set_clock_time(level, -1));
            assert_eq!(game.level_clock_time(level), Some(13_000));
            let stale =
                mod_api::LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            assert!(!game.level_set_clock_time(stale, 1));
            assert!(!game.level_set_game_rule(
                stale,
                "keep_inventory",
                gamerules::Value::Bool(false)
            ));
        });
    }

    /// Two levels that follow clocks of their own keep their own times through the clock
    /// handles' writes (`ServerClockManager`'s `setTotalTicks`, `setRate` and `setPaused`): the
    /// Overworld's clock paused at 6000 is a fixed time, and the End's set to 100 at a rate of 2.5
    /// counts 10 in 4 ticks. A clock a mod registered and nothing touched counts too, from 0, as
    /// `ServerClockManager.tick` advances every clock of the registry.
    #[test]
    #[ignore = "daily"]
    fn two_levels_with_clocks_of_their_own_keep_their_own_times() {
        use mod_api::Registrar;
        struct Clocks;
        #[mod_api::__subscriptions]
        impl Hooks for Clocks {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.register_world_clock("dungeon");
            }
        }
        let mods: mod_api::Mods = vec![ModEntry::new("clocks", Box::new(Clocks))].into();
        with_mods(mods.clone(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            with_game(|game| {
                let back = game.dim;
                game.enter_level(super::Dim::End).unwrap();
                game.enter_level(back).unwrap();
                let overworld = game.level(super::Dim::Overworld.id()).unwrap();
                let end = game.level(super::Dim::End.id()).unwrap();
                let host: &mut dyn ServerHost = game;
                let fixed = overworld.clock(&*host).unwrap();
                let running = end.clock(&*host).unwrap();
                assert_ne!(fixed, running);
                assert!(fixed.set_total_ticks(host, 6_000));
                assert!(fixed.set_paused(host, true));
                assert!(running.set_total_ticks(host, 100));
                assert!(running.set_rate(host, 2.5));
                assert!(!running.set_rate(host, f32::NAN));
                assert!(!running.set_rate(host, -1.0));
                assert!(!fixed.set_total_ticks(host, -1));
                for _ in 0..4 {
                    game.advance_clock();
                }
                assert_eq!(
                    game.level_clock_time(overworld),
                    Some(6_000),
                    "a fixed time"
                );
                assert_eq!(
                    game.level_clock_time(end),
                    Some(110),
                    "100 and 4 ticks at 2.5"
                );
                assert_eq!(
                    game.command_clock_time("clocks:dungeon"),
                    4,
                    "an untouched registered clock counts"
                );
            });
        });
    }

    /// What a mod's `game_rule_changed` and `time_skip` listeners were told.
    #[derive(Default)]
    struct Told {
        rules: Mutex<Vec<(String, bool)>>,
        skips: Mutex<Vec<(String, i64, i64, String)>>,
    }
    struct Teller(Arc<Told>, bool);
    #[mod_api::__subscriptions]
    impl Hooks for Teller {
        fn game_rule_changed(&self, _: &mut dyn ServerHost, event: &mut GameRuleChangedEvent<'_>) {
            self.0
                .rules
                .lock()
                .unwrap()
                .push((event.rule.to_owned(), event.actor.is_some()));
        }
        fn time_skip(
            &self,
            _: &mut dyn ServerHost,
            event: &mut mod_api::TimeSkipEvent<'_>,
        ) -> mod_api::Flow {
            self.0.skips.lock().unwrap().push((
                event.dimension.to_owned(),
                event.from,
                event.to,
                event.cause.to_owned(),
            ));
            if self.1 {
                mod_api::Flow::Cancel
            } else {
                mod_api::Flow::Continue
            }
        }
    }

    /// A rule a mod sets reaches `game_rule_changed` once, with no actor; the same value again
    /// reaches nothing.
    #[test]
    #[ignore = "daily"]
    fn a_mod_set_game_rule_has_no_actor() {
        let told = Arc::new(Told::default());
        let mods: mod_api::Mods = vec![ModEntry::new(
            "teller",
            Box::new(Teller(told.clone(), false)),
        )]
        .into();
        with_mods(mods, || {
            with_game(|game| {
                let level = game.level(game.dim.id()).unwrap();
                assert!(level.set_game_rule(game, "keep_inventory", GameRuleValue::Bool(true)));
                assert!(level.set_game_rule(game, "keep_inventory", GameRuleValue::Bool(true)));
            });
        });
        assert_eq!(
            *told.rules.lock().unwrap(),
            [("minecraft:keep_inventory".to_owned(), false)]
        );
    }

    /// A `time_skip` listener that cancels keeps the old time: the call says `false`, the
    /// listener heard the jump with cause `mod`, and the clock has not moved.
    #[test]
    #[ignore = "daily"]
    fn a_canceled_time_skip_keeps_the_clock() {
        let told = Arc::new(Told::default());
        let mods: mod_api::Mods = vec![ModEntry::new(
            "teller",
            Box::new(Teller(told.clone(), true)),
        )]
        .into();
        with_mods(mods, || {
            with_game(|game| {
                let level = game.level(game.dim.id()).unwrap();
                let start = game.level_clock_time(level).unwrap();
                assert_ne!(start, 13_000);
                assert!(!game.level_set_clock_time(level, 13_000));
                assert_eq!(game.level_clock_time(level), Some(start));
                assert_eq!(
                    *told.skips.lock().unwrap(),
                    [(game.dim.id().to_owned(), start, 13_000, "mod".to_owned())]
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn rule_reads_distinguish_unknown_stale_and_unavailable_and_snapshot_values() {
        use mod_api::{LevelReadError, WorldEpoch};
        use sunrise_gameplay_common::game::execution_role::ExecutionRole;
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            assert!(
                game.ui
                    .rules
                    .set("keep_inventory", gamerules::Value::Bool(true))
            );
            assert!(
                game.ui
                    .rules
                    .set("random_tick_speed", gamerules::Value::Int(7))
            );
            assert_eq!(level.game_rule(game, "missing:rule"), Ok(None));
            assert_eq!(
                level.game_rule(game, "keep_inventory"),
                Ok(Some(GameRuleValue::Bool(true)))
            );
            assert_eq!(
                level.game_rule(game, "random_tick_speed"),
                Ok(Some(GameRuleValue::Int(7)))
            );
            let mut tick_speed = None;
            assert_eq!(
                level.visit_game_rules(game, &mut |id, value| {
                    assert_ne!(id, "minecraft:max_minecart_speed");
                    if id == "minecraft:random_tick_speed" {
                        tick_speed = Some(value);
                    }
                    ControlFlow::Continue(())
                }),
                Ok(ControlFlow::Continue(()))
            );
            assert_eq!(tick_speed, Some(GameRuleValue::Int(7)));

            game.ui.rules = game
                .ui
                .rules
                .copy_with_features(&[gamerules::MINECART_IMPROVEMENTS.to_owned()]);
            let mut first = Vec::new();
            assert_eq!(
                level.visit_game_rules(game, &mut |id, value| {
                    first.push((id.to_owned(), value));
                    ControlFlow::Break(())
                }),
                Ok(ControlFlow::Break(()))
            );
            assert_eq!(
                first,
                [(
                    "minecraft:advance_time".to_owned(),
                    GameRuleValue::Bool(true)
                )]
            );

            let server: &dyn ServerHost = game;
            let mut minecart_speed = None;
            assert_eq!(
                server.visit_game_rules(&mut |id, value| {
                    if id == "minecraft:max_minecart_speed" {
                        minecart_speed = Some(value);
                        ControlFlow::Break(())
                    } else {
                        ControlFlow::Continue(())
                    }
                }),
                Ok(ControlFlow::Break(()))
            );
            assert_eq!(minecart_speed, Some(GameRuleValue::Int(8)));
            let stale =
                mod_api::LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            let gone = mod_api::LevelRef::new(
                level.epoch(),
                mod_api::__private::dimension_key("missing:level"),
            );
            for handle in [stale, gone] {
                assert_eq!(
                    handle.game_rule(game, "keep_inventory"),
                    Err(LevelReadError::Stale)
                );
                assert_eq!(
                    handle.visit_game_rules(game, &mut |_, _| panic!("refused rule visit")),
                    Err(LevelReadError::Stale)
                );
                assert!(game.native_level_world(handle).is_none());
            }
            game.options.execution_role = ExecutionRole::Client;
            assert_eq!(
                level.game_rule(game, "keep_inventory"),
                Err(LevelReadError::Unavailable)
            );
            assert_eq!(
                level.visit_game_rules(game, &mut |_, _| panic!("refused rule visit")),
                Err(LevelReadError::Unavailable)
            );
            assert!(game.native_level_world(level).is_none());
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn weather_read_resolves_levels_and_refuses_stale_or_unavailable_handles() {
        use mod_api::{LevelRef, WorldEpoch};

        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            game.world.weather.rain_level = (0.0, 0.5);
            game.world.weather.thunder_level = (0.0, 0.75);
            assert_eq!(level.weather(game), Some((0.5, 0.375)));
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            assert_eq!(stale.weather(game), None);
            let absent = LevelRef::new(
                level.epoch(),
                mod_api::__private::dimension_key("missing:level"),
            );
            assert_eq!(absent.weather(game), None);
            // The current world's dimension type, not its id alone, gates weather.
            game.world.shape.has_ceiling = true;
            assert_eq!(level.weather(game), Some((0.0, 0.0)));
        });
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let epoch = game.native_lifecycle.epoch().unwrap();
            let level = LevelRef::new(epoch, mod_api::DimensionKey::OVERWORLD);
            let stale = LevelRef::new(WorldEpoch::new(epoch.get() + 1), level.dimension());
            assert_eq!(level.weather(game), None, "no logged-in remote level");
            let (client, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(client, "WeatherTest").unwrap();
            assert_eq!(level.weather(game), None, "login has not completed");
            game.client.remote.as_mut().unwrap().logged_in = true;
            game.world.weather.rain_level = (0.0, 0.5);
            game.world.weather.thunder_level = (0.0, 0.75);
            assert_eq!(level.weather(game), Some((0.5, 0.375)));
            let other = LevelRef::new(level.epoch(), mod_api::DimensionKey::NETHER);
            assert_eq!(
                other.weather(game),
                None,
                "client shows only its current level"
            );
            assert_eq!(stale.weather(game), None);
            // Server-only weather queries (`level_is_raining`/`level_is_thundering`) are refused at compile time on `ClientGame`.
        });
    }

    #[test]
    fn weather_requires_skylight_no_ceiling_and_not_end() {
        use sunrise_gameplay_common::game::mod_api_host_common::weather_for_level;
        use sunrise_level::chunk::LevelShape;
        let rain = (1.0, 0.5);
        assert_eq!(weather_for_level(LevelShape::OVERWORLD, false, rain), rain);
        assert_eq!(
            weather_for_level(LevelShape::NETHER, false, rain),
            (0.0, 0.0)
        );
        assert_eq!(weather_for_level(LevelShape::END, true, rain), (0.0, 0.0));
        let mut skyless = LevelShape::OVERWORLD;
        skyless.sky_light = false;
        assert_eq!(weather_for_level(skyless, false, rain), (0.0, 0.0));
        assert_eq!(
            weather_for_level(LevelShape::OVERWORLD.with_ceiling(true), false, rain),
            (0.0, 0.0)
        );
    }

    #[test]
    fn rain_and_thunder_start_above_vanillas_thresholds() {
        assert_eq!(
            super::weather_flags((0.2_f32.next_down(), 0.0)),
            (false, false)
        );
        assert_eq!(super::weather_flags((0.2, 0.9)), (true, false));
        assert_eq!(super::weather_flags((1.0, 0.95)), (true, true));
    }
}
