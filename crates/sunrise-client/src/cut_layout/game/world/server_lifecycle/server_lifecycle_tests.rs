use mod_api::{
    __private::{HookPanicReport, take_hook_panics},
    ConfigEvent, ConfigKind, ConfigSnapshot, ConfigSpec, Hooks, ModEntry, Registrar,
    RegistrationContext, ServerHost,
};
use mod_platform::testing::with_mods;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use sunrise_gameplay::game::ServerRunOutcome;
use sunrise_gameplay_common::game::Dim;
use sunrise_gameplay_common::game::native_configs::NativeConfigsGameExt1;
use sunrise_gameplay_common::world_biomes;
use {
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
    sunrise_gameplay::game::native_guest_turn_tests::with_guests,
    sunrise_gameplay_common::game::execution_role::ExecutionRole,
};

const CHILD: &str = "SUNRISE_SERVER_LIFECYCLE_CHILD";

/// Runs `test`, this module's test of that name, in a child process of its own, and false there.
/// A final save is admitted only while no entity pass is lifted anywhere in the process
/// (`Game::admit_entity_save`), so in a shared runner another test's lifted pass drops the
/// `saved` these tests expect.
fn in_child(test: &str) -> bool {
    let name = format!("cut_layout::game::server_side::server_lifecycle::tests::{test}");
    run_fresh_cases(&name, CHILD, &["stop"])
}

struct PanicAt {
    stopping: bool,
    config_unloading: bool,
    logout: bool,
}

struct StoppedProbe {
    serving_stopped: Box<dyn Fn() -> bool + Send + Sync>,
    lan: std::sync::Weak<sunrise_sim::lan::Shared>,
    root: std::path::PathBuf,
    save: Arc<sunrise_sim::save::SaveDir>,
}

#[test]
#[ignore = "daily"]
fn integrated_stop_finishes_after_an_unencodable_record() {
    if in_child("integrated_stop_finishes_after_an_unencodable_record") {
        return;
    }
    use sunrise_core::nbt::{DISK_WRITE_NESTING_GUARD, Tag};
    use sunrise_sim::save::{LevelShape, SaveDir, SessionLock};

    struct SaveEvents(Arc<Mutex<Vec<&'static str>>>);
    #[mod_api::__subscriptions]
    impl Hooks for SaveEvents {
        fn world_save_start(
            &self,
            _: &mut dyn ServerHost,
            event: &mut mod_api::WorldSaveStartEvent,
        ) {
            self.0.lock().unwrap().push(if event.wait {
                "save start, wait"
            } else {
                "save start"
            });
        }
        fn player_save_to_file(
            &self,
            _: &mut dyn ServerHost,
            _: &mut mod_api::PlayerSaveToFileEvent<'_>,
        ) {
            self.0.lock().unwrap().push("player saved");
        }
        fn level_save(&self, _: &mut dyn ServerHost, _: &mut mod_api::LevelSaveEvent<'_>) {
            self.0.lock().unwrap().push("level saved");
        }
        fn world_saved(&self, _: &mut dyn ServerHost, _: &mut mod_api::WorldSavedEvent) {
            self.0.lock().unwrap().push("world saved");
        }
        fn server_stopped(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStoppedEvent) {
            self.0.lock().unwrap().push("stopped");
        }
    }

    let root = tempfile::tempdir().unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new(
        "save_events",
        Box::new(SaveEvents(events.clone())),
    )]
    .into();
    with_mods(mods, || {
        with_guests(|game| {
            game.join_world_rest().unwrap();
            game.server.world_root = root.path().to_path_buf();
            let open = || {
                SaveDir::new(
                    root.path().to_path_buf(),
                    LevelShape::OVERWORLD,
                    Some(Default::default()),
                )
            };
            game.server.save.0 = Arc::new(open());
            game.server.session_lock.0 = Some(SessionLock::acquire(root.path()).unwrap());
            game.server.world_adopted = true;
            game.native_lifecycle
                .begin_open(Some(mod_api::Authority::IntegratedServer));
            game.native_lifecycle.world_open_ready().unwrap();
            game.save_game().unwrap();
            assert_eq!(
                *events.lock().unwrap(),
                [
                    "save start, wait",
                    "player saved",
                    "player saved",
                    "level saved",
                    "world saved",
                ]
            );
            events.lock().unwrap().clear();
            game.level.name = "saved on exit after record failure".into();

            let slot = (2, -3);
            let neighbour = (3, -3);
            let deleted = (4, -3);
            let entity = |marker| Tag::compound([("marker", Tag::Int(marker))]);
            game.server
                .save
                .0
                .save_entities(vec![(slot, vec![entity(1)]), (deleted, vec![entity(3)])])
                .unwrap();
            let invalid = (0..=DISK_WRITE_NESTING_GUARD)
                .fold(Tag::Int(0), |nested, _| Tag::List(vec![nested]));
            game.server
                .save
                .0
                .save_entities_later(vec![
                    (slot, vec![invalid]),
                    (neighbour, vec![entity(2)]),
                    (deleted, vec![]),
                ])
                .unwrap();

            // An Err here reaches the client through Integrated::stop and makes quitting fatal.
            game.run_server_lifecycle(false, |_| Ok(ServerRunOutcome::Orderly))
                .unwrap();
            assert!(game.server.session_lock.0.is_none());
            let disk = open();
            assert_eq!(disk.load_entities(slot.0, slot.1).unwrap(), [entity(1)]);
            assert_eq!(
                disk.load_entities(neighbour.0, neighbour.1).unwrap(),
                [entity(2)]
            );
            assert!(disk.load_entities(deleted.0, deleted.1).unwrap().is_empty());
            assert_eq!(
                sunrise_sim::save::Level::read(root.path()).unwrap().name,
                "saved on exit after record failure"
            );
            game.server.save.0.flush().unwrap();
            assert_eq!(game.server.save.0.record_save_failures(), 1);
            assert_eq!(
                *events.lock().unwrap(),
                [
                    "save start, wait",
                    "player saved",
                    "player saved",
                    "level saved",
                    "world saved",
                    "stopped",
                ]
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn integrated_stop_propagates_metadata_errors_even_after_a_record_failure() {
    if in_child("integrated_stop_propagates_metadata_errors_even_after_a_record_failure") {
        return;
    }
    use sunrise_core::nbt::{DISK_WRITE_NESTING_GUARD, Tag};
    use sunrise_sim::save::{LevelShape, SaveDir, SessionLock};

    with_mods(Vec::new().into(), || {
        with_game(|game| {
            for mixed in [false, true] {
                let root = tempfile::tempdir().unwrap();
                game.server.world_root = root.path().to_path_buf();
                game.server.save.0 = Arc::new(SaveDir::new(
                    game.server.world_root.clone(),
                    LevelShape::OVERWORLD,
                    Some(Default::default()),
                ));
                game.server.session_lock.0 = Some(SessionLock::acquire(root.path()).unwrap());
                game.server.world_adopted = true;
                if mixed {
                    let invalid = (0..=DISK_WRITE_NESTING_GUARD)
                        .fold(Tag::Int(0), |nested, _| Tag::List(vec![nested]));
                    game.server
                        .save
                        .0
                        .save_entities(vec![((2, 0), vec![invalid])])
                        .unwrap();
                    assert_eq!(game.server.save.0.record_save_failures(), 1);
                }
                let blocked = root.path().join("level.dat_new");
                std::fs::create_dir(&blocked).unwrap();
                let expected = std::fs::write(&blocked, b"test").unwrap_err();
                let error = game
                    .run_server_lifecycle(false, |_| Ok(ServerRunOutcome::Orderly))
                    .unwrap_err();
                assert_eq!(
                    error.downcast_ref::<std::io::Error>().unwrap().kind(),
                    expected.kind()
                );
                assert!(game.server.session_lock.0.is_none());
            }
        });
    });
}

/// Exercise the real stop/save paths, not just direct calls to hook dispatch.
#[test]
#[ignore = "daily"]
fn every_started_server_stops_and_saves_even_when_startup_or_ticks_fail() {
    if in_child("every_started_server_stops_and_saves_even_when_startup_or_ticks_fail") {
        return;
    }
    struct Log(
        Arc<Mutex<Vec<String>>>,
        PanicAt,
        Arc<Mutex<Option<StoppedProbe>>>,
    );
    #[mod_api::__subscriptions]
    impl Hooks for Log {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let log = self.0.clone();
            let panic_on_unload = self.1.config_unloading;
            r.register_config(
                ConfigSpec::new(ConfigKind::Server, ""),
                Some(Arc::new(
                    move |event: ConfigEvent<'_>, _: &ConfigSnapshot| {
                        if matches!(event, ConfigEvent::Unloading(_)) {
                            log.lock().unwrap().push("config-unloading".into());
                            if panic_on_unload {
                                panic!("config unloading listener");
                            }
                        }
                        Ok(())
                    },
                )),
            )
            .unwrap();
        }
        fn server_about_to_start(
            &self,
            _: &mut dyn ServerHost,
            _: &mut mod_api::ServerAboutToStartEvent,
        ) {
            self.0.lock().unwrap().push("about-to-start".into());
        }
        fn level_load(&self, _: &mut dyn ServerHost, e: &mut mod_api::LevelLoadEvent<'_>) {
            self.0
                .lock()
                .unwrap()
                .push(format!("level-load:{}", e.dimension));
        }
        fn server_starting(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStartingEvent) {
            self.0.lock().unwrap().push("starting".into());
        }
        fn server_started(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStartedEvent) {
            self.0.lock().unwrap().push("started".into());
        }
        fn server_stopping(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStoppingEvent) {
            self.0.lock().unwrap().push("stopping".into());
            if self.1.stopping {
                panic!("stopping listener");
            }
        }
        fn level_save(&self, _: &mut dyn ServerHost, e: &mut mod_api::LevelSaveEvent<'_>) {
            self.0
                .lock()
                .unwrap()
                .push(format!("level-save:{}", e.dimension));
        }
        fn world_saved(&self, _: &mut dyn ServerHost, _: &mut mod_api::WorldSavedEvent) {
            self.0.lock().unwrap().push("saved".into());
        }
        fn player_logged_out(
            &self,
            _: &mut dyn ServerHost,
            _: &mut mod_api::PlayerLoggedOutEvent<'_>,
        ) {
            if self.1.logout {
                self.0.lock().unwrap().push("logout".into());
                panic!("logout listener");
            }
        }
        fn level_unload(&self, _: &mut dyn ServerHost, e: &mut mod_api::LevelUnloadEvent<'_>) {
            self.0
                .lock()
                .unwrap()
                .push(format!("level-unload:{}", e.dimension));
        }
        fn server_stopped(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStoppedEvent) {
            if let Some(probe) = self.2.lock().unwrap().as_ref() {
                assert!((probe.serving_stopped)(), "serving retired before Stopped");
                assert!(probe.lan.upgrade().is_none(), "LAN dropped before Stopped");
                let saved = sunrise_sim::save::Level::read(&probe.root).unwrap();
                assert_eq!(saved.name, "lifecycle final save");
                assert!(
                    probe.save.raw_chunk(0, 0).is_some(),
                    "dirty chunk saved before Stopped"
                );
                assert_eq!(
                    self.0.lock().unwrap().last().map(String::as_str),
                    Some("level-unload:minecraft:overworld")
                );

                let _lock = sunrise_sim::save::SessionLock::acquire(&probe.root).unwrap();
            }
            self.0.lock().unwrap().push("stopped".into());
        }
    }
    for dedicated in [false, true] {
        for case in [
            "normal",
            "startup-error",
            "opening-failed-init",
            "bind-failed-init",
            "runtime-extra-level",
            "tick-error",
            "panic",
            "save-error",
            "unopened",
            "locked-unadopted",
            "stopping-panic",
            "config-unloading-panic",
            "config-unloading-logout-panic",
        ] {
            let prepare_error = case == "dedicated-prepare-error";
            if prepare_error && !dedicated {
                continue;
            }
            let log = Arc::new(Mutex::new(Vec::new()));
            let stopped_probe = Arc::new(Mutex::new(None));
            let config_panic = case.starts_with("config-unloading");
            let panic_at = PanicAt {
                stopping: case == "stopping-panic",
                config_unloading: config_panic,
                logout: case == "config-unloading-logout-panic",
            };

            with_mods(
                vec![ModEntry::new(
                    "log",
                    Box::new(Log(log.clone(), panic_at, stopped_probe.clone())),
                )]
                .into(),
                || {
                    let run_case = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                        game.options.execution_role = if dedicated {
                            ExecutionRole::DedicatedServer
                        } else {
                            ExecutionRole::IntegratedServer
                        };
                        let root = std::env::temp_dir().join(format!(
                            "server-lifecycle-{}-{dedicated}-{case}",
                            std::process::id()
                        ));
                        std::fs::create_dir_all(&root).unwrap();
                        let serverconfig = root.join("serverconfig");
                        std::fs::create_dir_all(&serverconfig).unwrap();
                        std::fs::write(serverconfig.join("log"), "").unwrap();
                        game.server.world_root = root.clone();
                        game.server.save.0 = Arc::new(sunrise_sim::save::SaveDir::new(
                            root.clone(),
                            sunrise_sim::save::LevelShape::OVERWORLD,
                            Some(Default::default()),
                        ));
                        game.level.name = "lifecycle final save".into();
                        let adopted =
                            !matches!(case, "unopened" | "bind-failed-init" | "locked-unadopted");
                        if adopted || case == "locked-unadopted" {
                            game.server.session_lock.0 =
                                Some(sunrise_sim::save::SessionLock::acquire(&root).unwrap());
                        }

                        game.server.world_adopted = adopted;
                        if matches!(case, "unopened" | "bind-failed-init" | "locked-unadopted") {
                            std::fs::write(root.join("level.dat"), b"unadopted save").unwrap();
                        }
                        if config_panic || prepare_error {
                            let config = root.join("serverconfig");
                            std::fs::create_dir_all(&config).unwrap();
                            std::fs::write(config.join("log-server.toml"), "fixture = true\n")
                                .unwrap();
                        }
                        if config_panic {
                            game.level.name = "previous save".into();
                            game.level.write(&root, &Default::default()).unwrap();

                            game.level.name = "lifecycle final save".into();
                            game.world
                                .chunks
                                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                            let stone = game
                                .pack
                                .assets
                                .registry
                                .default_state("minecraft:stone")
                                .unwrap();
                            game.world.set(1, 64, 1, stone).unwrap();
                            assert!(game.server.save.0.raw_chunk(0, 0).is_none());
                            let serving_stopped = if dedicated {
                                Box::new(|| true) as Box<dyn Fn() -> bool + Send + Sync>
                            } else {
                                Box::new(sunrise_gameplay::game::server_side::integrated::tests::install_serving(
                                    game,
                                    case == "config-unloading-logout-panic",
                                ))
                            };
                            let lan =
                                Arc::downgrade(&game.server.lan.as_ref().unwrap().hub.lan.shared);
                            *stopped_probe.lock().unwrap() = Some(StoppedProbe {
                                serving_stopped,
                                lan,
                                root: root.clone(),
                                save: game.server.save.0.clone(),
                            });
                        }
                        let result = game.run_server_lifecycle(dedicated, |game| {
                            if case == "bind-failed-init" {
                                return Ok(ServerRunOutcome::FailedInit(std::io::Error::other(
                                    "fixture initialization error",
                                )));
                            }
                            game.native_configs_server_start(&root, dedicated);
                            game.native_server_about_to_start(dedicated);
                            game.native_lifecycle_open_started();
                            if case == "unopened" {
                                return Err("failed init".into());
                            }
                            game.native_level_opened(game.dim);
                            if matches!(case, "startup-error" | "locked-unadopted") {
                                return Err("failed init".into());
                            }
                            if case == "opening-failed-init" {
                                return Ok(ServerRunOutcome::FailedInit(std::io::Error::other(
                                    "fixture initialization error",
                                )));
                            }
                            game.native_server_starting(dedicated);
                            game.native_lifecycle_world_ready();
                            if case == "runtime-extra-level" {
                                game.join_world_rest().unwrap();
                                let level = mod_api::RuntimeLevel::new(
                                    "minecraft:overworld",
                                    mod_api::RuntimeGenerator::Flat,
                                    1,
                                );
                                game.runtime_level_open("test:pocket", level).unwrap();
                                game.apply_runtime_levels();
                                assert!(game.runtime_level_is_open("test:pocket"));
                            }
                            match case {
                                "tick-error" => Err("failed tick".into()),
                                "panic" => panic!("failed tick"),
                                "save-error" => {
                                    let file = root.join("not-a-directory");
                                    std::fs::write(&file, b"occupied").unwrap();
                                    game.server.world_root = file;
                                    Ok(ServerRunOutcome::Orderly)
                                }
                                _ => Ok(ServerRunOutcome::Orderly),
                            }
                        });
                        assert_eq!(
                            result.is_ok(),
                            matches!(case, "normal" | "runtime-extra-level" | "stopping-panic")
                                || config_panic,
                            "{dedicated} {case}: {result:?}"
                        );
                        let expected_panics: &[&str] = match case {
                            "stopping-panic" => &["server_stopping"],
                            "config-unloading-panic" => &["config_unloading"],
                            "config-unloading-logout-panic" => {
                                &["player_logged_out", "config_unloading"]
                            }
                            _ => &[],
                        };
                        assert_eq!(
                            take_hook_panics(),
                            expected_panics
                                .iter()
                                .map(|&hook| HookPanicReport {
                                    mod_id: "log",
                                    hook
                                })
                                .collect::<Vec<_>>(),
                            "{dedicated} {case}"
                        );
                        if prepare_error {
                            let error = result.as_ref().unwrap_err().to_string();
                            assert!(error.contains("its level data does not read"), "{error}");
                        }
                        let expected: &[&str] = match case {
                            "bind-failed-init" => &["stopped"],
                            "unopened" => &["about-to-start", "stopped", "config-unloading"],
                            "locked-unadopted" => &[
                                "about-to-start",
                                "level-load:minecraft:overworld",
                                "level-unload:minecraft:overworld",
                                "stopped",
                                "config-unloading",
                            ],
                            "startup-error" | "opening-failed-init" => &[
                                "about-to-start",
                                "level-load:minecraft:overworld",
                                "level-save:minecraft:overworld",
                                "saved",
                                "level-unload:minecraft:overworld",
                                "stopped",
                                "config-unloading",
                            ],
                            "tick-error" | "panic" => &[
                                "about-to-start",
                                "level-load:minecraft:overworld",
                                "starting",
                                "started",
                                "level-save:minecraft:overworld",
                                "saved",
                                "level-unload:minecraft:overworld",
                                "stopped",
                                "config-unloading",
                            ],
                            "save-error" => &[
                                "about-to-start",
                                "level-load:minecraft:overworld",
                                "starting",
                                "started",
                                "stopping",
                                "level-save:minecraft:overworld",
                                "stopped",
                                "config-unloading",
                            ],
                            "runtime-extra-level" => &[
                                "about-to-start",
                                "level-load:minecraft:overworld",
                                "starting",
                                "started",
                                "level-load:test:pocket",
                                "stopping",
                                "level-save:minecraft:overworld",
                                "level-save:test:pocket",
                                "saved",
                                "level-unload:minecraft:overworld",
                                "level-unload:test:pocket",
                                "stopped",
                                "config-unloading",
                            ],
                            "config-unloading-logout-panic" => &[
                                "about-to-start",
                                "level-load:minecraft:overworld",
                                "starting",
                                "started",
                                "stopping",
                                "logout",
                                "level-save:minecraft:overworld",
                                "saved",
                                "level-unload:minecraft:overworld",
                                "stopped",
                                "config-unloading",
                            ],
                            _ => &[
                                "about-to-start",
                                "level-load:minecraft:overworld",
                                "starting",
                                "started",
                                "stopping",
                                "level-save:minecraft:overworld",
                                "saved",
                                "level-unload:minecraft:overworld",
                                "stopped",
                                "config-unloading",
                            ],
                        };
                        assert_eq!(*log.lock().unwrap(), expected, "{dedicated} {case}");
                        game.native_lifecycle_world_closed();
                        assert_eq!(
                            *log.lock().unwrap(),
                            expected,
                            "second close is a no-op after {case}"
                        );
                        assert!(
                            game.server.session_lock.0.is_none(),
                            "storage released after {case}"
                        );
                        assert!(!game.server.world_adopted, "adoption cleared after {case}");
                        assert!(
                            game.server.serving.is_none(),
                            "serving retired after {case}"
                        );
                        assert!(game.server.lan.is_none(), "LAN dropped after {case}");
                        if config_panic {
                            let registry = &game.pack.assets.registry;
                            let chunk = game
                                .server
                                .save
                                .0
                                .load(0, 0, registry, world_biomes!(game))
                                .unwrap();
                            assert_eq!(
                                chunk.block(1, 64, 1),
                                registry.default_state("minecraft:stone").unwrap()
                            );
                        }
                        if matches!(case, "unopened" | "bind-failed-init" | "locked-unadopted") {
                            assert_eq!(
                                std::fs::read(root.join("level.dat")).unwrap(),
                                b"unadopted save",
                                "{dedicated} {case}"
                            );
                        } else if case != "save-error" {
                            let saved = sunrise_sim::save::Level::read(&root).unwrap();
                            assert_eq!(saved.name, "lifecycle final save");
                        }
                        if case == "runtime-extra-level" {
                            Dim::set_extra(std::iter::empty());
                        }
                        std::fs::remove_dir_all(root).unwrap();
                    };
                    if config_panic {
                        with_guests(run_case);
                    } else {
                        with_game(run_case);
                    }
                },
            );
        }
    }
}

/// A panicking `world_save_start` listener rolls back its committed nested writes before
/// the snapshot is taken, while healthy listeners' writes persist and shutdown finishes.
#[test]
#[ignore = "daily"]
fn a_world_save_start_panic_discards_only_its_listeners_journal() {
    if in_child("a_world_save_start_panic_discards_only_its_listeners_journal") {
        return;
    }
    world_save_start_panic_during_shutdown();
}

fn world_save_start_panic_during_shutdown() {
    use mod_api::__private::{HookPanicReport, take_hook_panics};
    use mod_api::{
        BlockState, BlockUpdateFlags, HostTransaction, WorldSaveStartEvent, WorldSavedEvent,
    };
    use std::sync::atomic::AtomicUsize;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::save;

    const POS_PANIC: [i32; 3] = [1, 64, 1];
    const POS_HEALTHY: [i32; 3] = [2, 64, 1];
    const LEVEL: &str = "minecraft:overworld";

    struct Panics {
        calls: Arc<AtomicUsize>,
        writes: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Panics {
        #[listener(priority = 100)]
        fn world_save_start(&self, host: &mut dyn ServerHost, _: &mut WorldSaveStartEvent) {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let level = host.level(LEVEL).unwrap();
            let gold = host.block_state_id("minecraft:gold_block").unwrap();
            host.transaction(|host, _| {
                assert_eq!(
                    level.set_block(host, POS_PANIC, gold, BlockUpdateFlags::NONE),
                    Ok(true)
                );
                Ok::<_, std::convert::Infallible>(())
            })
            .unwrap();
            assert_eq!(level.block_state(host, POS_PANIC), Some(gold));
            self.writes.fetch_add(1, Ordering::Relaxed);
            panic!("after committing a nested write");
        }
    }

    #[derive(Debug, PartialEq)]
    enum Notice {
        Start(Option<BlockState>),
        Saved(Option<BlockState>, Option<BlockState>),
        Unloaded(String),
        Stopped,
    }
    struct Observer(Arc<Mutex<Vec<Notice>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Observer {
        fn world_save_start(&self, host: &mut dyn ServerHost, _: &mut WorldSaveStartEvent) {
            let at_panic = host
                .level(LEVEL)
                .and_then(|level| level.block_state(host, POS_PANIC));
            if let Some(level) = host.level(LEVEL) {
                let iron = host.block_state_id("minecraft:iron_block").unwrap();
                assert_eq!(
                    level.set_block(host, POS_HEALTHY, iron, BlockUpdateFlags::NONE),
                    Ok(true)
                );
            }
            self.0.lock().unwrap().push(Notice::Start(at_panic));
        }
        fn world_saved(&self, host: &mut dyn ServerHost, _: &mut WorldSavedEvent) {
            let at_panic = host
                .level(LEVEL)
                .and_then(|level| level.block_state(host, POS_PANIC));
            let at_healthy = host
                .level(LEVEL)
                .and_then(|level| level.block_state(host, POS_HEALTHY));
            self.0
                .lock()
                .unwrap()
                .push(Notice::Saved(at_panic, at_healthy));
        }
        fn level_unload(&self, _: &mut dyn ServerHost, event: &mut mod_api::LevelUnloadEvent<'_>) {
            self.0
                .lock()
                .unwrap()
                .push(Notice::Unloaded(event.dimension.into()));
        }
        fn server_stopped(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStoppedEvent) {
            self.0.lock().unwrap().push(Notice::Stopped);
        }
    }

    let calls = Arc::new(AtomicUsize::new(0));
    let writes = Arc::new(AtomicUsize::new(0));
    let notices = Arc::new(Mutex::new(Vec::new()));
    with_mods(
        vec![
            ModEntry::new(
                "broken-save-start",
                Box::new(Panics {
                    calls: calls.clone(),
                    writes: writes.clone(),
                }),
            ),
            ModEntry::new("observer", Box::new(Observer(notices.clone()))),
        ]
        .into(),
        || {
            with_game(|game| {
                let root = tempfile::tempdir().unwrap();
                game.join_world_rest().unwrap();
                game.server.world_root = root.path().to_path_buf();
                let open = || {
                    save::SaveDir::new(
                        root.path().to_path_buf(),
                        save::LevelShape::OVERWORLD,
                        Some(Default::default()),
                    )
                };
                game.server.save.0 = Arc::new(open());
                game.server.session_lock.0 = Some(save::SessionLock::acquire(root.path()).unwrap());
                game.server.world_adopted = true;
                game.level.name = "saved despite start listener panic".into();
                game.native_level_opened(Dim::Overworld);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let registry = game.pack.assets.registry;
                let stone = registry.default_state("minecraft:stone").unwrap();
                let iron = registry.default_state("minecraft:iron_block").unwrap();
                game.world
                    .set(POS_PANIC[0], POS_PANIC[1], POS_PANIC[2], stone)
                    .unwrap();
                game.world
                    .set(POS_HEALTHY[0], POS_HEALTHY[1], POS_HEALTHY[2], stone)
                    .unwrap();
                let stone_api = (&*game as &dyn ServerHost)
                    .block_state_id("minecraft:stone")
                    .unwrap();
                let iron_api = (&*game as &dyn ServerHost)
                    .block_state_id("minecraft:iron_block")
                    .unwrap();

                game.run_server_lifecycle(false, |_| Ok(ServerRunOutcome::Orderly))
                    .unwrap();

                assert_eq!(
                    writes.load(Ordering::Relaxed),
                    1,
                    "the callback reached its deliberate panic"
                );
                assert_eq!(
                    take_hook_panics(),
                    [HookPanicReport {
                        mod_id: "broken-save-start",
                        hook: "world_save_start"
                    }]
                );
                assert_eq!(
                    notices.lock().unwrap().as_slice(),
                    [
                        Notice::Start(Some(stone_api)),
                        Notice::Saved(Some(stone_api), Some(iron_api)),
                        Notice::Unloaded(LEVEL.into()),
                        Notice::Stopped
                    ]
                );
                assert_eq!(
                    save::Level::read(root.path()).unwrap().name,
                    "saved despite start listener panic"
                );
                let reloaded = open().load(0, 0, registry, world_biomes!(game)).unwrap();
                assert_eq!(reloaded.block(1, 64, 1), stone);
                assert_eq!(reloaded.block(2, 64, 1), iron);
                assert!(game.server.session_lock.0.is_none());

                game.test_native_world_save_start(true);
                assert_eq!(
                    calls.load(Ordering::Relaxed),
                    1,
                    "the panicking callback stays disabled"
                );
                assert_eq!(notices.lock().unwrap().last(), Some(&Notice::Start(None)));
                assert!(take_hook_panics().is_empty());
            })
        },
    );
}

/// Final level.dat writes must preserve the authoritative owner.
#[test]
#[ignore = "daily"]
fn integrated_shutdown_reopens_the_authoritative_owner_or_keeps_an_interrupted_save() {
    use sunrise_core::nbt::Tag;
    use sunrise_gameplay::game::lan::Owner;
    use sunrise_sim::{
        items::ItemStack,
        lan::playerdata,
        save::{self, Level, LevelShape, SaveDir},
    };

    for (interrupted, legacy) in [(false, false), (true, false), (true, true)] {
        with_guests(|game| {
            game.join_world_rest().unwrap();
            let root = tempfile::tempdir().unwrap();
            let dir = root.path();
            let items = game.res.items();
            let mut old = Level {
                player_uuid: Some(playerdata::uuid_ints(12)),
                name: "previous".into(),
                position: Some([1.0, 72.0, 3.0]),
                dimension: "minecraft:overworld".into(),
                xp_level: 4,
                xp_total: 14,
                inventory: vec![(0, "minecraft:stone".into(), 2, 0)],
                ..Level::default()
            };
            let old_player = old.player_tag(items);
            let level_dat = dir.join("level.dat");
            if legacy {
                old.write_without_player(dir).unwrap();
                let mut root = save::read_dat(&level_dat).unwrap();
                let Some(Tag::Compound(data)) = root.get_mut("Data") else {
                    panic!("level.dat has a Data compound");
                };
                data.insert("Player".into(), old_player.clone());
                data.remove(&"singleplayer_uuid".into());
                save::write_dat(&level_dat, root).unwrap();
            } else {
                old.write(dir, items).unwrap();
            }
            let player_file = playerdata::path(dir, 12);
            let prior_bytes = std::fs::read(&player_file).ok();

            let host = game.server.lan.as_mut().unwrap();
            host.owner = Some(Owner {
                uuid: 12,
                saved: None,
            });
            host.hub.singleplayer_owner = Some(12);
            host.hub
                .update_guest(12, |guest| {
                    guest.dimension = "minecraft:the_nether".into();
                    guest.position = [8.0, 90.0, -4.0];
                    guest.xp_level = 11;
                    guest.xp_total = 90;
                    guest.inventory[0] = Some(ItemStack {
                        item: "minecraft:diamond".into(),
                        count: 9,
                        damage: 0,
                        extra: None,
                    });
                })
                .unwrap();
            game.server.world_root = dir.to_path_buf();
            game.server.save.0 = Arc::new(SaveDir::new(
                dir.to_path_buf(),
                LevelShape::OVERWORLD,
                Some(Default::default()),
            ));
            game.server.session_lock.0 = Some(save::SessionLock::acquire(dir).unwrap());
            game.server.world_adopted = true;
            game.level = old;
            game.level.name = "stopped".into();
            if interrupted {
                game.server.interrupted_turns.push(12);
            }
            game.run_server_lifecycle(false, |_| Ok(ServerRunOutcome::Orderly))
                .unwrap();

            let saved = Level::read(dir).unwrap();
            assert_eq!(saved.name, "stopped");
            if interrupted {
                assert_eq!(std::fs::read(&player_file).ok(), prior_bytes);
                assert_eq!(saved.dimension, "minecraft:overworld");
                assert_eq!((saved.xp_level, saved.xp_total), (4, 14));
                assert_eq!(saved.inventory, [(0, "minecraft:stone".to_owned(), 2, 0)]);
                if legacy {
                    let root = save::read_dat(&level_dat).unwrap();
                    assert_eq!(
                        root.get("Data").and_then(|data| data.get("Player")),
                        Some(&old_player)
                    );
                }
            } else {
                assert!(saved.singleplayer_file);
                assert_eq!(saved.dimension, "minecraft:the_nether");
                assert_eq!(saved.position, Some([8.0, 90.0, -4.0]));
                assert_eq!((saved.xp_level, saved.xp_total), (11, 90));
                assert_eq!(saved.inventory, [(0, "minecraft:diamond".to_owned(), 9, 0)]);
            }
        });
    }
}

#[test]
#[ignore = "daily"]
fn shutdown_notifies_only_for_replaced_player_files() {
    use sunrise_core::nbt::Tag;
    use sunrise_gameplay::game::lan::Owner;
    use sunrise_sim::{
        lan::playerdata,
        save::{self, Level, LevelShape, SaveDir, SessionLock},
    };

    const CASE: &str = "SUNRISE_SHUTDOWN_PLAYER_FILE_CASE";
    if run_fresh_cases(
        "cut_layout::game::server_side::server_lifecycle::tests::shutdown_notifies_only_for_replaced_player_files",
        CASE,
        &["owner", "guest"],
    ) {
        return;
    }
    let (failed, written) = if std::env::var(CASE).unwrap() == "owner" {
        (12, 13)
    } else {
        (13, 12)
    };
    struct Log(Arc<Mutex<Vec<u128>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Log {
        fn player_save_to_file(
            &self,
            _: &mut dyn ServerHost,
            event: &mut mod_api::PlayerSaveToFileEvent<'_>,
        ) {
            self.0.lock().unwrap().push(event.player);
        }
    }
    let notices = Arc::new(Mutex::new(Vec::new()));
    with_mods(
        vec![ModEntry::new(
            "shutdown_save",
            Box::new(Log(notices.clone())),
        )]
        .into(),
        || {
            with_guests(|game| {
                game.join_world_rest().unwrap();
                game.options.execution_role = ExecutionRole::IntegratedServer;
                let root = tempfile::tempdir().unwrap();
                let dir = root.path();
                let mut level = Level {
                    player_uuid: Some(playerdata::uuid_ints(12)),
                    name: "before shutdown".into(),
                    xp_level: 3,
                    ..Level::default()
                };
                level.write(dir, game.res.items()).unwrap();
                let failed_path = playerdata::path(dir, failed);
                if failed != 12 {
                    save::write_dat(&failed_path, Tag::compound([("XpLevel", Tag::Int(3))]))
                        .unwrap();
                }
                let prior = std::fs::read(&failed_path).unwrap();

                let backup = failed_path.with_extension("dat_old");
                std::fs::create_dir(&backup).unwrap();
                std::fs::write(backup.join("sentinel"), b"keep").unwrap();
                let host = game.server.lan.as_mut().unwrap();
                host.owner = Some(Owner {
                    uuid: 12,
                    saved: None,
                });
                host.hub.singleplayer_owner = Some(12);
                for uuid in [12, 13] {
                    host.hub.update_guest(uuid, |g| g.xp_level = 9).unwrap();
                }
                game.server.world_root = dir.to_owned();
                game.server.save.0 = Arc::new(SaveDir::new(
                    dir.to_owned(),
                    LevelShape::OVERWORLD,
                    Some(Default::default()),
                ));
                game.server.session_lock.0 = Some(SessionLock::acquire(dir).unwrap());
                game.server.world_adopted = true;
                game.level = level;
                game.level.name = "after shutdown".into();
                game.run_server_lifecycle(false, |_| Ok(ServerRunOutcome::Orderly))
                    .unwrap();

                assert_eq!(std::fs::read(&failed_path).unwrap(), prior);
                let player = save::read_dat(&playerdata::path(dir, written)).unwrap();
                assert_eq!(player.get("XpLevel"), Some(&Tag::Int(9)));
                assert_eq!(Level::read(dir).unwrap().name, "after shutdown");
            });
        },
    );
    assert_eq!(*notices.lock().unwrap(), [written]);
}

#[test]
#[ignore = "daily"]
fn full_saves_write_live_players_before_saving_levels() {
    use sunrise_sim::{lan::playerdata, save};

    #[derive(Debug, PartialEq)]
    enum Notice {
        Start(bool),
        Player(u128),
        Level(Option<i32>, Option<i32>),
        End(bool),
    }

    struct Log {
        directory: std::path::PathBuf,
        notices: Arc<Mutex<Vec<Notice>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Log {
        fn world_save_start(
            &self,
            _: &mut dyn ServerHost,
            event: &mut mod_api::WorldSaveStartEvent,
        ) {
            self.notices.lock().unwrap().push(Notice::Start(event.wait));
        }
        fn player_save_to_file(
            &self,
            _: &mut dyn ServerHost,
            event: &mut mod_api::PlayerSaveToFileEvent<'_>,
        ) {
            self.notices
                .lock()
                .unwrap()
                .push(Notice::Player(event.player));
        }
        fn level_save(&self, _: &mut dyn ServerHost, _: &mut mod_api::LevelSaveEvent<'_>) {
            let xp = |uuid| {
                let tag = save::read_dat(&playerdata::path(&self.directory, uuid))?;
                match tag.get("XpLevel") {
                    Some(sunrise_core::nbt::Tag::Int(xp)) => Some(*xp),
                    _ => None,
                }
            };
            self.notices
                .lock()
                .unwrap()
                .push(Notice::Level(xp(12), xp(13)));
        }
        fn world_saved(&self, _: &mut dyn ServerHost, event: &mut mod_api::WorldSavedEvent) {
            self.notices.lock().unwrap().push(Notice::End(event.wait));
        }
    }

    for role in [
        ExecutionRole::DedicatedServer,
        ExecutionRole::IntegratedServer,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let notices = Arc::new(Mutex::new(Vec::new()));
        with_mods(
            vec![ModEntry::new(
                "save",
                Box::new(Log {
                    directory: dir.path().to_owned(),
                    notices: notices.clone(),
                }),
            )]
            .into(),
            || {
                with_guests(|game| {
                    game.join_world_rest().unwrap();
                    game.options.execution_role = role;
                    game.server.world_root = dir.path().to_owned();
                    let host = game.server.lan.as_mut().unwrap();
                    if role == ExecutionRole::IntegratedServer {
                        host.owner = Some(sunrise_gameplay::game::lan::Owner {
                            uuid: 12,
                            saved: None,
                        });
                        game.common.level.player_uuid = Some(playerdata::uuid_ints(12));
                    }
                    host.hub
                        .update_guest(12, |guest| guest.xp_level = 7)
                        .unwrap();
                    host.hub
                        .update_guest(13, |guest| guest.xp_level = 9)
                        .unwrap();
                    game.native_level_opened(Dim::Overworld);
                    game.save_game_with(false).unwrap();
                    assert_eq!(
                        game.server
                            .lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .online_players()
                            .count(),
                        2
                    );
                })
            },
        );
        assert_eq!(
            *notices.lock().unwrap(),
            [
                Notice::Start(false),
                Notice::Player(12),
                Notice::Player(13),
                Notice::Level(Some(7), Some(9)),
                Notice::End(false),
            ]
        );
    }
}

/// A `world_save_start` listener cannot re-enter the save cycle via `/save-all`; the nested
/// command fails before notifying or writing, and the outer cycle finishes once.
#[test]
#[ignore = "daily"]
fn world_save_start_rejects_nested_save_all_command() {
    if in_child("world_save_start_rejects_nested_save_all_command") {
        return;
    }
    world_save_start_nested_save_all();
}

fn world_save_start_nested_save_all() {
    use std::sync::atomic::AtomicUsize;

    struct ReenterSave {
        starts: Arc<AtomicUsize>,
        saved: Arc<AtomicUsize>,
        nested_ok: Arc<AtomicBool>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for ReenterSave {
        fn world_save_start(
            &self,
            host: &mut dyn ServerHost,
            _: &mut mod_api::WorldSaveStartEvent,
        ) {
            if self.starts.fetch_add(1, Ordering::SeqCst) == 0 {
                let ok = host.commands().run_command("save-all");
                self.nested_ok.store(ok, Ordering::SeqCst);
            }
        }
        fn world_saved(&self, _: &mut dyn ServerHost, _: &mut mod_api::WorldSavedEvent) {
            self.saved.fetch_add(1, Ordering::SeqCst);
        }
    }

    let root = tempfile::tempdir().unwrap();
    let starts = Arc::new(AtomicUsize::new(0));
    let saved = Arc::new(AtomicUsize::new(0));
    let nested_ok = Arc::new(AtomicBool::new(true));
    let mods = vec![ModEntry::new(
        "reenter_save",
        Box::new(ReenterSave {
            starts: starts.clone(),
            saved: saved.clone(),
            nested_ok: nested_ok.clone(),
        }),
    )]
    .into();
    with_mods(mods, || {
        crate::game::server_side::visibility_unwind_tests::with_save_game(root.path(), |game| {
            game.join_world_rest().unwrap();
            game.save_game().unwrap();
            assert!(!nested_ok.load(Ordering::SeqCst));
            assert_eq!(starts.load(Ordering::SeqCst), 1);
            assert_eq!(saved.load(Ordering::SeqCst), 1);
            assert!(!game.test_world_save_starting());
            assert!(game.native_run_command("save-all"));
            assert_eq!(starts.load(Ordering::SeqCst), 2);
            assert_eq!(saved.load(Ordering::SeqCst), 2);
        });
    });
}

use sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycleCommonGameExt1 as _;
