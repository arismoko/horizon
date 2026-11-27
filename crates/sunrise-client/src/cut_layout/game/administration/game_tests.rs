#![cfg(test)]
use sunrise_game_data::text::Text;
use sunrise_gameplay::game::game_tests::Out;
use sunrise_gameplay_common::game::Dim;
use sunrise_world::worldgen::structure::Rotation;
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use sunrise_gameplay::game::game_tests::{
        Action, DimGrid, GameTests, Grid, Info, report_of, retry_line, verify_batches,
    };
    use sunrise_gameplay::game::server_side::test_instances::TestDef;
    use sunrise_gameplay::game::server_side::test_instances::TestDefSample as _;
    use sunrise_gameplay::game_tests::{Retry, TestError};

    pub use sunrise_gameplay::game::game_tests::tests::server_game;

    /// Puts the process-wide block namespaces back as it found them.
    pub(crate) struct Published(Option<Vec<String>>);
    impl Drop for Published {
        fn drop(&mut self) {
            sunrise_sim::save::level_mods::set_block_namespaces(self.0.take());
        }
    }

    /// [`server_game`] over the loading screen's path, which leaves the
    /// process-wide language and UI metadata alone (its rest tables load on a
    /// worker joined here), with the block namespaces it publishes put back
    /// when the returned guard drops. Bind the guard before the game.
    pub(crate) fn isolated_server_game<'a>(
        resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
    ) -> (Published, sunrise_gameplay::game::ServerGame<'a>) {
        let published = Published(sunrise_sim::save::level_mods::block_namespaces());
        (published, build_server_game(resources, true))
    }

    fn build_server_game<'a>(
        resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
        menu: bool,
    ) -> sunrise_gameplay::game::ServerGame<'a> {
        use {crate::game::*, sunrise_gameplay_common::game::*};
        let root = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES")
                .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
        );
        let settings = Settings::default();
        let loaded = if menu {
            load_resources_skip_lang(
                sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
                Vec::new(),
                None,
                &root,
                &settings,
                std::time::Instant::now(),
                execution_role::ExecutionRole::IntegratedServer,
                &Progress::new(),
                &mut |_, _| {},
            )
            .unwrap()
        } else {
            load_resources(
                sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
                Vec::new(),
                None,
                &root,
                &settings,
                std::time::Instant::now(),
                false,
                execution_role::ExecutionRole::IntegratedServer,
                &Progress::new(),
                &mut |_, _| {},
            )
            .unwrap()
        };
        #[cfg(feature = "client")]
        if let Some(rest) = loaded.rest_loader {
            rest.finish().unwrap();
        }
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = execution_role::ExecutionRole::IntegratedServer;
        let res = resources.insert(loaded.resources);
        let mut game = sunrise_gameplay::game::ServerGame::new(
            res,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.reset_entity_residency();
        game.install_root_block_transformers();
        game.native_lifecycle
            .begin_open(Some(mod_api::Authority::IntegratedServer));
        // Match the server fixture this replaces: commands can force this
        // resident column without starting terrain generation.
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
        );
        game
    }

    fn info(block_based: bool, max_ticks: i32, setup_ticks: i32) -> Info {
        Info::new(
            1,
            "minecraft:t".into(),
            TestDef::sample(block_based, max_ticks, setup_ticks),
            Some((0, 64, 0)),
            Dim::Overworld,
            Rotation::None,
        )
    }

    #[test]
    fn a_test_keeps_its_level_awake_until_it_is_done() {
        let mut tests = GameTests::default();
        assert!(!tests.has_tests_in(Dim::Nether));
        let mut nether = info(false, 10, 0);
        nether.dim = Dim::Nether;
        tests.ticker.push(nether);
        assert!(tests.has_tests_in(Dim::Nether));
        assert!(!tests.has_tests_in(Dim::Overworld));
        // A rerun keeps the level of the test it copies.
        assert_eq!(tests.ticker[0].copy_reset(2).dim, Dim::Nether);
    }

    #[test]
    fn a_grid_has_a_state_per_dimension_starting_at_its_own_corner() {
        let mut grid = Grid::new((4, 9), 10).clearing();
        assert!(grid.clear_on_batch);
        grid.states
            .push((Dim::Overworld, DimGrid::new((4, 70, 12))));
        grid.states.push((Dim::Nether, DimGrid::new((4, 32, 12))));
        let nether = &grid.states[1].1;
        assert_eq!(nether.next, (4, 32, 12));
        assert_eq!(nether.row_z, (12, 13));
    }

    #[test]
    fn verify_runs_each_rotation_a_hundred_times_in_its_own_batch() {
        let mut id = 0;
        let batches = verify_batches(&[info(true, 100, 0)], || {
            id += 1;
            id
        });
        assert_eq!(batches.len(), 4);
        for (i, batch) in batches.iter().enumerate() {
            assert_eq!(batch.index, i as i32);
            assert_eq!(batch.infos.len(), 100);
            assert!(
                batch
                    .infos
                    .iter()
                    .all(|t| t.extra_rotation == Rotation::ALL[i])
            );
        }
        assert_eq!(id, 400);
    }

    #[test]
    fn start_execution_waits_for_the_setup_ticks_and_the_delay() {
        // always_pass: setup_ticks 1, delay 1: ticks -2, -1 and 0 follow.
        let mut i = info(false, 1, 1);
        i.start_execution(1);
        assert_eq!(i.tick_count, -3);
    }

    #[test]
    fn on_each_tick_runs_every_tick_up_to_the_timeout() {
        let mut i = info(true, 5, 0);
        i.tick_count = 2;
        i.on_each_tick(Action::BlockBasedTick);
        let times: Vec<i64> = i.run_at.iter().map(|(t, _)| *t).collect();
        assert_eq!(times, vec![2, 3, 4]);
    }

    #[test]
    fn a_failure_is_not_overwritten_by_success_and_stamps_the_tick() {
        let mut i = info(true, 5, 0);
        i.tick_count = 3;
        i.fail(Text::literal("boom"));
        i.succeed();
        assert!(!i.done);
        assert!(matches!(i.error, Some(TestError::Assert { tick: 3, .. })));
        let mut ok = info(true, 5, 0);
        ok.succeed();
        assert!(ok.done && ok.error.is_none());
    }

    #[test]
    fn a_retry_line_counts_runs_and_pads_to_53() {
        let info = info(true, 10, 0);
        let line = retry_line(&info, Retry::new(5, false), 2, 1, false);
        assert!(line.starts_with("[Run:    2, Ok:    1, Fail:    1, Left:    3]"));
        assert_eq!(line.find("minecraft:t"), Some(53));
        assert!(line.ends_with("minecraft:t failed! 0ms"));
        let unlimited = retry_line(&info, Retry::new(0, true), 1, 1, true);
        assert!(unlimited.starts_with("[Run:    1, Ok:    1, Fail:    0]"));
    }

    #[test]
    fn a_rerun_is_a_copy_that_shares_the_originals_counters() {
        let original = info(true, 10, 0).with_retry(Retry::new(2, false));
        let copy = original.copy_reset(9);
        assert_eq!((copy.id, copy.origin, copy.rerun), (9, 1, true));
        assert_eq!(copy.retry, original.retry);
        assert!(copy.has_pos && copy.pos == original.pos);
        let mut reports = Vec::new();
        *report_of(&mut reports, copy.origin).0 += 1;
        *report_of(&mut reports, original.origin).1 += 1;
        assert_eq!(reports, vec![(1, 1, 1)]);
    }
}

#[cfg(test)]
mod completed_test_coordinate_feedback_tests {
    use super::*;
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::motion::PlayerMotion;

    #[test]
    #[ignore = "daily"]
    fn actual_completed_test_retains_player_source_for_coordinate_feedback() {
        let mut resources = None;
        let mut game = tests::server_game(&mut resources);
        game.build_registries();
        game.set_allow_commands(true);
        game.level.player_uuid = Some([0, 0, 0, 77]);
        game.level.health = 20.0;
        let uuid = game.acting_player_uuid();
        assert_eq!(uuid, [0, 0, 0, 77]);
        assert_eq!(game.level.health, 20.0);
        for dimension in [Dim::Overworld, Dim::Nether] {
            game.enter_command_level(dimension);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(sunrise_core::position::BlockPos::new(0, 0, 0).chunk())
                .unwrap();
            assert!(game.run_command("forceload add 0 0"));
            assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
            assert_eq!(
                &*game.res.facts().get(game.world.block(8, 199, 8)).name,
                "minecraft:stone"
            );
        }
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 200.0, 8.5)).unwrap());
        assert_eq!(game.dim, Dim::Nether);
        let definition = game.test_def("minecraft:always_pass").unwrap();
        assert_eq!(definition.dimension, "minecraft:overworld");
        assert!(!definition.block_based);
        game.server.chat_capture = Some(Vec::new());
        assert!(game.run_command("scoreboard objectives add coords dummy"));
        assert!(game.run_command("scoreboard players set result coords 99"));
        assert!(game.run_command("scoreboard players set success coords 99"));
        assert!(game.run_command("execute store result score result coords store success score success coords run test run minecraft:always_pass"));
        assert!(game.run_command("execute if score result coords matches 1"));
        assert!(game.run_command("execute if score success coords matches 1"));
        assert!(
            game.server
                .game_tests
                .0
                .ticker
                .iter()
                .any(|i| i.key == "minecraft:always_pass" && i.dim == Dim::Overworld)
        );
        let mut messages = Vec::new();
        // Real registered run, actual test ticker and completion; no injected finish.
        for _ in 0..8 {
            game.enter_command_level(Dim::Overworld);
            game.tick_game_tests();
            for out in &game.server.game_tests.0.outbox {
                if let Out::Console { text, .. } = out {
                    messages.push(text.component());
                }
            }
            game.enter_command_level(Dim::Nether);
        }
        let runner = game.server.game_tests.0.runner.as_ref().unwrap();
        assert_eq!(runner.tracked.len(), 1);
        assert!(runner.tracked.iter().all(|t| t.done && !t.failed));
        messages.extend(
            game.server
                .chat_capture
                .take()
                .unwrap()
                .iter()
                .map(Text::component),
        );
        let debug: Vec<_> = messages.iter().map(|m| format!("{m:?}")).collect();
        assert!(
            debug.iter().any(|m| m.contains("commands.test.summary\"")),
            "actual summary prerequisite: {debug:?}"
        );
        assert!(
            debug
                .iter()
                .any(|m| m.contains("commands.test.summary.all_required_passed")),
            "actual success prerequisite: {debug:?}"
        );
        assert_eq!(game.acting_player_uuid(), uuid);
        assert_eq!(
            game.player.as_ref().unwrap().position,
            Vec3::new(8.5, 200.0, 8.5)
        );
        assert_eq!(game.dim, Dim::Nether);
        assert!(
            debug.iter().any(|m| m.contains("test.player.coordinates"))
                && debug.iter().any(|m| m.contains("test.run.coordinates")),
            "completed real test must retain its player's coordinate feedback: {debug:?}"
        );
    }
}

#[cfg(test)]
#[path = "commands/test_summary_context_tests.rs"]
mod added_test_summary_context_tests;
#[cfg(test)]
mod registered_game_test_tests {
    use super::*;
    use mod_api::{GameTestContext, GameTestFunction, GameTestResult, GameTests};
    use std::sync::Arc;
    use sunrise_gameplay::game::game_tests::{Info, Runner, Tracked};
    use sunrise_gameplay::game::server_side::test_instances::TestDef;
    use sunrise_gameplay::game::server_side::test_instances::TestDefSample as _;
    use sunrise_gameplay::game_tests::TestError;

    struct ChangeBlock(mod_api::BlockState);
    impl GameTestFunction for ChangeBlock {
        fn run(
            &self,
            host: &mut dyn mod_api::ServerHost,
            context: &GameTestContext,
        ) -> GameTestResult {
            let pos = context.absolute_pos([1, 64, 1]);
            assert!(
                context
                    .level
                    .set_block(
                        host,
                        pos,
                        self.0,
                        mod_api::BlockUpdateFlags::CLIENTS | mod_api::BlockUpdateFlags::NEIGHBORS
                    )
                    .unwrap()
            );
            assert_eq!(context.level.block(host, pos), Some(self.0.block()));
            GameTestResult::Pass
        }
    }
    struct Fail;
    impl GameTestFunction for Fail {
        fn run(&self, _: &mut dyn mod_api::ServerHost, _: &GameTestContext) -> GameTestResult {
            GameTestResult::Fail("registered failure".into())
        }
    }
    struct Panics;
    impl GameTestFunction for Panics {
        fn run(&self, _: &mut dyn mod_api::ServerHost, _: &GameTestContext) -> GameTestResult {
            let attempt = 1;
            panic!("registered panic in attempt {attempt}");
        }
    }
    struct TestMod(mod_api::BlockState);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for TestMod {
        fn register_game_tests(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::RegisterGameTestsEvent<'_>,
        ) {
            event
                .functions
                .register("example:pass", Arc::new(ChangeBlock(self.0)))
                .unwrap();
            event
                .functions
                .register("example:fail", Arc::new(Fail))
                .unwrap();
            event
                .functions
                .register("example:panic", Arc::new(Panics))
                .unwrap();
            event
                .functions
                .register_test(
                    "example:passing",
                    mod_api::GameTestDefinition::new(
                        "minecraft:empty".into(),
                        "example:pass".into(),
                    ),
                )
                .unwrap();
        }
    }
    #[test]
    #[ignore = "daily"]
    fn registered_game_tests_report_pass_failure_and_unknown_in_real_level() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            let mut functions = mod_api::GameTestFunctions::default();
            game.native_register_game_tests(&mut functions);
            assert!(functions.resolve("example:pass").is_none());
            let hooks = Box::new(TestMod(
                sunrise_gameplay_common::game::api_values::TryIntoApi::try_into_api(&stone)
                    .unwrap(),
            ));
            let mods = vec![mod_api::ModEntry::new("example", hooks)];
            mod_platform::testing::with_mods(mods.into(), || {
                game.native_register_game_tests(&mut functions);
                game.reload_datapacks(false, &[]).unwrap();
                let registered = game.test_def("example:passing").unwrap();
                assert!(registered.callback.is_some());
                assert_eq!(registered.max_ticks, 100);
            });
            assert!(functions.register("example:pass", Arc::new(Fail)).is_err());
            assert!(
                functions
                    .register("minecraft:always_pass", Arc::new(Fail))
                    .is_err()
            );
            game.server.game_tests.0.runner = Some(Runner {
                batches: Vec::new(),
                current: 0,
                batch_pending: 5,
                tracked: (1..=5)
                    .map(|id| Tracked {
                        id,
                        required: true,
                        done: false,
                        failed: false,
                    })
                    .collect(),
                halt_on_error: false,
                batch_size: 50,
                clear_between_batches: false,
                grid_for_existing: false,
                last_batch: Vec::new(),
                grid: None,
                reports: Vec::new(),
                scheduled: Vec::new(),
                stopped: false,
                summary_source: None,
                summary_dimensions: Vec::new(),
            });
            let mut outcomes = Vec::new();
            for (index, id) in [
                "example:panic",
                "example:pass",
                "example:fail",
                "example:missing",
                "minecraft:always_pass",
            ]
            .into_iter()
            .enumerate()
            {
                let mut def = TestDef::sample(false, 10, 0);
                def.function = Some(id.into());
                def.callback = functions.resolve(id);
                let mut info = Info::new(
                    index as u32 + 1,
                    id.into(),
                    def,
                    Some((0, 64, 0)),
                    Dim::Overworld,
                    Rotation::None,
                );
                game.start_test(&mut info);
                if info.error.is_some() {
                    info.finish();
                }
                if id == "example:panic" {
                    let Some(TestError::Unknown(message)) = &info.error else {
                        panic!("expected panic to become an unknown test failure");
                    };
                    assert!(
                        message.contains("registered panic in attempt 1"),
                        "{message}"
                    );
                }
                if id == "example:fail" {
                    let Some(TestError::Assert { message, .. }) = &info.error else {
                        panic!("expected assertion");
                    };
                    assert_eq!(message.plain(), "registered failure");
                }
                game.test_finished(&info);
                outcomes.push((info.done, info.error.is_none()));
            }
            let runner = game.server.game_tests.0.runner.as_ref().unwrap();
            assert_eq!(runner.tracked.iter().filter(|t| t.done).count(), 5);
            assert_eq!(runner.tracked.iter().filter(|t| t.failed).count(), 3);
            assert_eq!(
                game.server.game_tests.0.failed,
                ["example:panic", "example:fail", "example:missing"]
            );
            assert_eq!(
                outcomes,
                [
                    (true, false),
                    (true, true),
                    (true, false),
                    (true, false),
                    (true, true)
                ]
            );
        });
    }
}
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
