use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use crate::game::{native_entity_refs::tests::with_game, server_lifecycle::ServerRunOutcome};
use mod_api::{Hooks, ModEntry, ServerHost};
use mod_platform::testing::with_mods;
use sunrise_gameplay_common::{
    execution_role::ExecutionRole,
    game::{
        Dim, native_lifecycle_common::NativeLifecycleCommonGameExt1 as _,
        test_fixtures::run_fresh_cases,
    },
    world_biomes,
};

#[test]
#[ignore = "daily"]
fn a_world_saved_panic_discards_only_its_listeners_journal() {
    if !run_fresh_cases(
        "game::server_side::server_lifecycle_fixtures::a_world_saved_panic_discards_only_its_listeners_journal",
        "SUNRISE_WORLD_SAVED_PANIC_CASE",
        &["shutdown"],
    ) {
        world_saved_panic_during_shutdown();
    }
}

fn world_saved_panic_during_shutdown() {
    use mod_api::__private::{HookPanicReport, take_hook_panics};
    use mod_api::{BlockState, BlockUpdateFlags, HostTransaction, WorldSavedEvent};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::save;

    const POS: [i32; 3] = [1, 64, 1];
    const LEVEL: &str = "minecraft:overworld";

    struct Panics {
        calls: Arc<AtomicUsize>,
        writes: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Panics {
        #[listener(priority = 100)]
        fn world_saved(&self, host: &mut dyn ServerHost, _: &mut WorldSavedEvent) {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let level = host.level(LEVEL).unwrap();
            let gold = host.block_state_id("minecraft:gold_block").unwrap();
            host.transaction(|host, _| {
                assert_eq!(
                    level.set_block(host, POS, gold, BlockUpdateFlags::NONE),
                    Ok(true)
                );
                Ok::<_, std::convert::Infallible>(())
            })
            .unwrap();
            assert_eq!(level.block_state(host, POS), Some(gold));
            self.writes.fetch_add(1, Ordering::Relaxed);
            panic!("after committing a nested write");
        }
    }

    #[derive(Debug, PartialEq)]
    enum Notice {
        Saved(Option<BlockState>),
        Unloaded(String),
        Stopped,
    }
    struct Observer(Arc<Mutex<Vec<Notice>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Observer {
        fn world_saved(&self, host: &mut dyn ServerHost, _: &mut WorldSavedEvent) {
            self.0.lock().unwrap().push(Notice::Saved(
                host.level(LEVEL)
                    .and_then(|level| level.block_state(host, POS)),
            ));
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
                "broken-save",
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
                game.server.save.0 = Arc::new(save::SaveDir::new(
                    game.server.world_root.clone(),
                    save::LevelShape::OVERWORLD,
                    Some(Default::default()),
                ));
                game.server.session_lock.0 = Some(save::SessionLock::acquire(root.path()).unwrap());
                game.server.world_adopted = true;
                game.level.name = "saved despite listener panic".into();
                game.native_level_opened(Dim::Overworld);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let registry = game.pack.assets.registry;
                let stone = registry.default_state("minecraft:stone").unwrap();
                game.world.set(POS[0], POS[1], POS[2], stone).unwrap();
                let stone_api = (&*game as &dyn ServerHost)
                    .block_state_id("minecraft:stone")
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
                        mod_id: "broken-save",
                        hook: "world_saved"
                    }]
                );
                assert_eq!(
                    notices.lock().unwrap().as_slice(),
                    [
                        Notice::Saved(Some(stone_api)),
                        Notice::Unloaded(LEVEL.into()),
                        Notice::Stopped
                    ]
                );
                assert_eq!(
                    save::Level::read(root.path()).unwrap().name,
                    "saved despite listener panic"
                );
                let saved = game
                    .server
                    .save
                    .0
                    .load(0, 0, registry, world_biomes!(game))
                    .unwrap();
                assert_eq!(saved.block(1, 64, 1), stone);
                assert!(game.server.session_lock.0.is_none());

                game.native_world_saved(true);
                assert_eq!(
                    calls.load(Ordering::Relaxed),
                    1,
                    "the panicking callback stays disabled"
                );
                assert_eq!(notices.lock().unwrap().last(), Some(&Notice::Saved(None)));
                assert!(take_hook_panics().is_empty());
            })
        },
    );
}

// The shell hands a dedicated server its profile services; a bare server game has none.
fn with_dedicated_game(test: impl FnOnce(&mut crate::game::ServerGame<'_>)) {
    with_game(|game| {
        game.server.dedicated_services = Some(crate::game::dedicated::Services {
            find_name: |_| None,
            find_id: |_| None,
            profile_keys: Vec::new,
            verify_session: Arc::new(|_, _, _| Ok(None)),
        });
        test(game);
    });
}

#[test]
#[ignore = "daily"]
fn dedicated_port_bind_failure_emits_only_stopped_without_touching_world() {
    use sunrise_java_properties::Properties;
    use sunrise_server_files::properties::{ServerProperties, SystemEntropy};

    struct Log(Arc<Mutex<Vec<&'static str>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Log {
        fn server_about_to_start(
            &self,
            _: &mut dyn ServerHost,
            _: &mut mod_api::ServerAboutToStartEvent,
        ) {
            self.0.lock().unwrap().push("about-to-start");
        }
        fn level_load(&self, _: &mut dyn ServerHost, _: &mut mod_api::LevelLoadEvent<'_>) {
            self.0.lock().unwrap().push("level-load");
        }
        fn server_starting(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStartingEvent) {
            self.0.lock().unwrap().push("starting");
        }
        fn server_started(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStartedEvent) {
            self.0.lock().unwrap().push("started");
        }
        fn server_stopping(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStoppingEvent) {
            self.0.lock().unwrap().push("stopping");
        }
        fn level_unload(&self, _: &mut dyn ServerHost, _: &mut mod_api::LevelUnloadEvent<'_>) {
            self.0.lock().unwrap().push("level-unload");
        }
        fn server_stopped(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStoppedEvent) {
            self.0.lock().unwrap().push("stopped");
        }
    }

    let occupied = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = occupied.local_addr().unwrap().port();
    let universe = tempfile::tempdir().unwrap();
    let world_dir = universe.path().join("untouched_world");
    let mut properties = Properties::default();
    properties.put("online-mode", "false".into());
    properties.put("server-ip", "127.0.0.1".into());
    properties.put("server-port", port.to_string());
    let props = ServerProperties::new(properties, &mut SystemEntropy::default()).unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    with_mods(
        vec![ModEntry::new("log", Box::new(Log(log.clone())))].into(),
        || {
            with_dedicated_game(|game| {
                game.options.execution_role = ExecutionRole::DedicatedServer;
                let setup = crate::game::dedicated::Setup {
                    props,
                    universe: universe.path().to_path_buf(),
                    level_name: "untouched_world".into(),
                    port: Some(port),
                    allow_missing_mods: false,
                };
                let error = game.run_dedicated_with_setup(setup).unwrap_err();
                assert!(
                    error
                        .to_string()
                        .starts_with("Failed to initialize server:")
                );
            });
        },
    );
    assert_eq!(*log.lock().unwrap(), ["stopped"]);
    assert!(
        !world_dir.exists(),
        "failed port bind must not create or write world files"
    );
    drop(occupied);
}

#[test]
#[ignore = "daily"]
fn datapack_dimensions_open_prepare_save_and_close_in_creation_order() {
    if !run_fresh_cases(
        "game::server_side::server_lifecycle_fixtures::datapack_dimensions_open_prepare_save_and_close_in_creation_order",
        "SUNRISE_CONFIGURED_DIMENSION_ORDER_CASE",
        &["open"],
    ) {
        {
            use crate::game::world_loading::WorldRequest;
            use std::fs;
            use sunrise_sim::save::Level;

            struct Log {
                events: Arc<Mutex<Vec<String>>>,
                listed_at_load: Arc<Mutex<Vec<Vec<String>>>>,
            }
            #[mod_api::__subscriptions]
            impl Hooks for Log {
                fn level_load(
                    &self,
                    host: &mut dyn ServerHost,
                    e: &mut mod_api::LevelLoadEvent<'_>,
                ) {
                    self.events
                        .lock()
                        .unwrap()
                        .push(format!("load:{}", e.dimension));
                    let mut levels = Vec::new();
                    let _ = host.levels().visit_levels(&mut |level| {
                        levels.push(level.dimension_id().to_owned());
                        std::ops::ControlFlow::Continue(())
                    });
                    self.listed_at_load.lock().unwrap().push(levels);
                }
                fn create_spawn_position(
                    &self,
                    _: &mut dyn ServerHost,
                    e: &mut mod_api::CreateSpawnPositionEvent<'_>,
                ) -> mod_api::Flow {
                    self.events.lock().unwrap().push("initialize-spawn".into());
                    *e.position = [32, 73, 16];
                    mod_api::Flow::Cancel
                }
                fn level_save(&self, _: &mut dyn ServerHost, e: &mut mod_api::LevelSaveEvent<'_>) {
                    self.events
                        .lock()
                        .unwrap()
                        .push(format!("save:{}", e.dimension));
                }
                fn level_unload(
                    &self,
                    _: &mut dyn ServerHost,
                    e: &mut mod_api::LevelUnloadEvent<'_>,
                ) {
                    self.events
                        .lock()
                        .unwrap()
                        .push(format!("unload:{}", e.dimension));
                }
            }

            let root = tempfile::tempdir().unwrap();
            let pack = root.path().join("datapacks/stems");
            let dimensions = pack.join("data/test/dimension");
            fs::create_dir_all(&dimensions).unwrap();
            fs::write(
        pack.join("pack.mcmeta"),
        r#"{"pack":{"description":"Lifecycle fixture","min_format":[121,0],"max_format":[121,0]}}"#,
    )
    .unwrap();
            fs::write(
                root.path().join("datapacks.json"),
                r#"{"enabled":["vanilla","file/stems"],"disabled":[]}"#,
            )
            .unwrap();
            for name in ["last", "first"] {
                fs::write(
            dimensions.join(format!("{name}.json")),
            r#"{"type":"minecraft:overworld","generator":{"type":"minecraft:flat","settings":{
                "biome":"minecraft:plains","features":false,"lakes":false,"structure_overrides":[],
                "layers":[{"height":1,"block":"minecraft:bedrock"}]}}}"#,
        )
        .unwrap();
            }
            let log = Arc::new(Mutex::new(Vec::new()));
            let listed_at_load = Arc::new(Mutex::new(Vec::new()));
            with_mods(
                vec![ModEntry::new(
                    "log",
                    Box::new(Log {
                        events: log.clone(),
                        listed_at_load: listed_at_load.clone(),
                    }),
                )]
                .into(),
                || {
                    with_game(|game| {
                        game.options.execution_role = ExecutionRole::DedicatedServer;
                        game.join_world_rest().unwrap();
                        let mut request = WorldRequest::new(
                            "stems",
                            Some("stems"),
                            Some(1),
                            false,
                            None,
                            Level::default().rule_registry,
                        )
                        .unwrap();
                        request.save_root = root.path().to_path_buf();

                        request.stem_order = Some(&[
                            "test:last",
                            "minecraft:the_nether",
                            "minecraft:overworld",
                            "test:first",
                            "minecraft:the_end",
                        ]);
                        let prepared = game.prepare_world_and_rules(request).unwrap();
                        assert!(prepared.level.world_spawn.is_none());
                        let order = [
                            "minecraft:overworld",
                            "test:last",
                            "minecraft:the_nether",
                            "test:first",
                            "minecraft:the_end",
                        ];
                        game.native_lifecycle_open_started();
                        game.finish_open_world(prepared).unwrap();
                        game.prepare_levels().unwrap();
                        assert_eq!(game.level.world_spawn, Some([32, 73, 16]));

                        game.enter_level(Dim::Nether).unwrap();
                        let mut visited = Vec::new();
                        let _ = game.native_visit_levels(&mut |level| {
                            visited.push(level.dimension_id());
                            std::ops::ControlFlow::Continue(())
                        });
                        assert_eq!(visited, order);
                        let listed = listed_at_load.lock().unwrap();
                        assert_eq!(listed.len(), order.len());
                        for (i, levels) in listed.iter().enumerate() {
                            assert_eq!(
                                levels.iter().map(String::as_str).collect::<Vec<_>>(),
                                &order[..=i]
                            );
                        }

                        for id in order {
                            game.enter_level(Dim::from_id(id)).unwrap();
                            game.server.streaming.forced.insert((0, 0));
                        }
                        assert_eq!(
                            game.levels_to_tick()
                                .into_iter()
                                .map(Dim::id)
                                .collect::<Vec<_>>(),
                            order
                        );
                        game.enter_level(Dim::Nether).unwrap();
                        game.save_game().unwrap();
                        assert_eq!(game.dim, Dim::Nether);
                        game.close_loaded_levels_for_stop();
                        game.native_lifecycle_world_closed();
                        Dim::set_extra(std::iter::empty());

                        let mut expected =
                            vec!["load:minecraft:overworld".into(), "initialize-spawn".into()];
                        expected.extend(order[1..].iter().map(|id| format!("load:{id}")));
                        expected.extend(order.iter().map(|id| format!("save:{id}")));
                        expected.extend(order.iter().map(|id| format!("unload:{id}")));
                        assert_eq!(*log.lock().unwrap(), expected);
                    })
                },
            );
        }
    }
}

#[test]
#[ignore = "daily"]
fn per_level_close_failure_skips_only_that_levels_unload() {
    struct Log(Arc<Mutex<Vec<String>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Log {
        fn level_unload(&self, _: &mut dyn ServerHost, e: &mut mod_api::LevelUnloadEvent<'_>) {
            self.0.lock().unwrap().push(e.dimension.to_owned());
        }
    }

    let log = Arc::new(Mutex::new(Vec::new()));
    with_mods(
        vec![ModEntry::new("log", Box::new(Log(log.clone())))].into(),
        || {
            with_game(|game| {
                game.join_world_rest().unwrap();
                let root = tempfile::tempdir().unwrap();
                game.server.world_root = root.path().to_path_buf();
                game.server.save.0 = Arc::new(sunrise_sim::save::SaveDir::new(
                    game.server.world_root.clone(),
                    sunrise_sim::save::LevelShape::OVERWORLD,
                    Some(Default::default()),
                ));
                game.level.world_spawn = Some([0, 64, 0]);
                game.native_lifecycle_open_started();
                game.open_configured_levels(&[Dim::Overworld, Dim::Nether, Dim::End])
                    .unwrap();

                let bad_save = Arc::new(sunrise_sim::save::SaveDir::new(
                    root.path().join("broken_nether"),
                    sunrise_sim::save::LevelShape::NETHER,
                    Some(Default::default()),
                ));
                bad_save.inject_sync_error_for_test(std::io::Error::other(
                    "simulated storage sync failure",
                ));
                game.server
                    .levels
                    .0
                    .parked_mut(Dim::Nether)
                    .unwrap()
                    .replace_save_for_test(bad_save);

                game.close_loaded_levels_for_stop();
                game.native_lifecycle_world_closed();
            });
        },
    );
    assert_eq!(
        *log.lock().unwrap(),
        ["minecraft:overworld", "minecraft:the_end"]
    );
}

#[test]
#[ignore = "daily"]
fn debug_world_uses_fixed_spawn_without_create_spawn_position() {
    struct ReplaceSpawn(Arc<AtomicBool>);
    #[mod_api::__subscriptions]
    impl Hooks for ReplaceSpawn {
        fn create_spawn_position(
            &self,
            _: &mut dyn ServerHost,
            event: &mut mod_api::CreateSpawnPositionEvent<'_>,
        ) -> mod_api::Flow {
            self.0.store(true, Ordering::Relaxed);
            *event.position = [32, 73, 16];
            mod_api::Flow::Cancel
        }
    }

    let fired = Arc::new(AtomicBool::new(false));
    with_mods(
        vec![ModEntry::new(
            "spawn",
            Box::new(ReplaceSpawn(fired.clone())),
        )]
        .into(),
        || {
            with_game(|game| {
                game.server.world_debug = true;
                assert!(game.level.world_spawn.is_none());
                game.open_configured_levels(&[Dim::Overworld]).unwrap();
                assert_eq!(game.level.world_spawn, Some([0, 80, 0]));
                assert_eq!(game.level.world_spawn_angle, [0.0, 0.0]);
            })
        },
    );
    assert!(!fired.load(Ordering::Relaxed));
}
