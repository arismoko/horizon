#![cfg(test)]
#[cfg(feature = "client")]
use mod_api::prelude::BlockStatesHost;
#[cfg(feature = "client")]
use std::sync::atomic::Ordering;
#[cfg(feature = "client")]
use std::sync::mpsc::channel;
#[cfg(feature = "client")]
use std::time::Duration;
#[cfg(feature = "client")]
use std::time::Instant;
#[cfg(feature = "client")]
use sunrise_client_gameplay::game::client_side::integrated_client::Integrated;
#[cfg(feature = "client")]
use sunrise_gameplay::game::server_side::integrated::{LoadPhase, Serving, server_game};
#[cfg(feature = "client")]
use sunrise_gameplay::game::world_loading::WorldRequest;
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::execution_role::ExecutionRole;
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::integrated_link::Flags;
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::owner_link::OwnerRequest;
#[cfg(all(test, feature = "client"))]
pub(crate) mod tests {
    use mod_api::prelude::{EntitiesHost, LivingHost, ServicesClientHost};
    use std::sync::{Arc, Mutex};
    use sunrise_gameplay_common::game::Dim;
    use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1;
    use sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycleCommonGameExt1;
    use sunrise_gameplay_common::game::tables::WorldTablesExt;

    #[derive(Debug, PartialEq, Eq)]
    enum CheckpointEvent {
        LevelSaved,
        WorldSaved,
        LevelUnloaded,
        ServerStopped,
    }
    struct CheckpointMod(Arc<Mutex<Vec<CheckpointEvent>>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for CheckpointMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let kind = r
                .block_entity_type("counter")
                .blocks(&["crash_checkpoint:counter"])
                .build();
            r.block_entity_part(&kind, CheckpointCounter).unwrap();
        }

        fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
            event
                .register(
                    mod_api::Block::new("crash_checkpoint:counter", "minecraft:barrel")
                        .with_item(false),
                )
                .unwrap();
        }

        fn level_save(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::LevelSaveEvent<'_>,
        ) {
            if event.dimension == Dim::Overworld.id() {
                self.0.lock().unwrap().push(CheckpointEvent::LevelSaved);
            }
        }

        fn world_saved(&self, _: &mut dyn mod_api::ServerHost, _: &mut mod_api::WorldSavedEvent) {
            self.0.lock().unwrap().push(CheckpointEvent::WorldSaved);
        }

        fn level_unload(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::LevelUnloadEvent<'_>,
        ) {
            if event.dimension == Dim::Overworld.id() {
                self.0.lock().unwrap().push(CheckpointEvent::LevelUnloaded);
            }
        }

        fn server_stopped(
            &self,
            _: &mut dyn mod_api::ServerHost,
            _: &mut mod_api::ServerStoppedEvent,
        ) {
            self.0.lock().unwrap().push(CheckpointEvent::ServerStopped);
        }
    }
    struct CheckpointCounter;
    #[mod_api::sunrise_part]
    impl mod_api::BlockPart for CheckpointCounter {}
    #[mod_api::sunrise_part]
    impl mod_api::BlockEntityPart for CheckpointCounter {
        fn block_entity_save_additional(
            &self,
            host: &dyn mod_api::LevelHost,
            event: &mut mod_api::BlockEntitySaveAdditionalEvent<'_>,
        ) {
            let entity = host
                .block_entity(event.pos)
                .expect("counter is still resident");
            let data = host
                .block_entity_data(entity)
                .expect("counter handle is live");
            event.tag.insert(
                "saved_progress".into(),
                data.get(&"progress".into()).unwrap().clone(),
            );
        }
    }

    use super::*;

    #[test]
    #[ignore = "daily"]
    #[cfg(any())]
    fn posted_native_panic_writes_one_server_report() {
        let scratch = tempfile::tempdir().unwrap();
        let home = scratch.path().to_path_buf();
        let previous = std::panic::take_hook();
        sunrise_level::crash::install(home.clone(), "client", || None);
        let (tasks, receiver) = channel::<OwnerRequest>();
        let mut server = Integrated {
            emergency_save: None,
            flags: Arc::new(Flags::default()),
            thread: None,
            tasks,
            world_name: String::new(),
            save_folder: String::new(),
            rule_registry: Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla()),
        };
        assert!(server.post(|_| {
            let _guard = sunrise_level::crash::enter_native_mod(sunrise_level::crash::ModCall {
                id: "example:posted",
                hook: "server_tick",
                part: None,
            });
            panic!("posted integrated panic 3494");
        }));
        server.thread = Some(
            std::thread::Builder::new()
                .name(sunrise_level::crash::INTEGRATED_SERVER_THREAD.into())
                .stack_size(server_thread_stack())
                .spawn(move || {
                    let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                            game.serve_owner_request(receiver.recv().unwrap());
                        });
                    }))
                    .unwrap_err();
                    sunrise_client::crash::report_server_panic(&*payload);
                })
                .unwrap(),
        );
        server.halt().unwrap();
        assert!(sunrise_client::crash::server_crashed());
        std::panic::set_hook(previous);
        let reports: Vec<_> = std::fs::read_dir(home.join("crash-reports"))
            .unwrap()
            .map(|p| p.unwrap().path())
            .collect();
        assert_eq!(reports.len(), 1);
        // `MinecraftServer.runServer` saves a server report even under a client install.
        let file = reports[0].file_name().unwrap().to_string_lossy();
        assert!(file.ends_with("-server.txt"), "{file}");
        let text = std::fs::read_to_string(&reports[0]).unwrap();
        assert!(text.contains("Description: Exception in server tick loop"));
        assert!(text.contains("Server thread"));
        assert!(text.contains("posted integrated panic 3494"));
        assert!(text.contains("in native mod example:posted, hook server_tick"));
    }
    /// A mod's quit while the world is still loading stops and joins the starting server
    /// before the loop ends, and that server's failure is the game's error.
    #[test]
    #[ignore = "daily"]
    fn a_quit_while_the_world_loads_waits_for_its_server() {
        use std::cell::Cell;
        use winit::event_loop::ControlFlow;
        use {
            crate::game::server_side::native_entity_refs::tests::with_client_game,
            sunrise_gameplay_common::game::test_fixtures::run_fresh_cases,
        };
        use {
            sunrise_client_gameplay::game::EventLoopCtl,
            sunrise_client_gameplay::game::client_side::integrated_client::Starting,
        };
        if run_fresh_cases(
            "cut_layout::game::server_side::integrated::tests::a_quit_while_the_world_loads_waits_for_its_server",
            "SUNRISE_CLIENT_QUIT_STARTING_CHILD",
            &["starting"],
        ) {
            return;
        }
        struct Loop(Cell<usize>);
        impl EventLoopCtl for Loop {
            fn exit(&self) {
                self.0.set(self.0.get() + 1);
            }
            fn set_control_flow(&self, _: ControlFlow) {}
        }
        with_client_game(|game| {
            let flags = Arc::new(Flags::default());
            let (gate, held) = channel::<()>();
            let server_flags = flags.clone();
            let thread = std::thread::spawn(move || {
                let _ = held.recv();
                *server_flags.failure.lock().unwrap() = Some("the world failed to load".into());
            });
            let (tasks, _requests) = channel::<OwnerRequest>();
            let (_ready, ready) = channel();
            game.client.integrated_starting = Some(Starting {
                handle: Integrated {
                    emergency_save: None,
                    flags: flags.clone(),
                    thread: Some(thread),
                    tasks,
                    world_name: String::new(),
                    save_folder: String::new(),
                    rule_registry: Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla()),
                },
                ready,
                player: "Dev".into(),
                folder: "loading".into(),
                new_world: false,
            });
            assert!(game.request_client_quit());
            let event_loop = Loop(Cell::new(0));
            game.idle(&event_loop);
            game.idle(&event_loop);
            assert_eq!(
                event_loop.0.get(),
                0,
                "the loop waits for the starting server"
            );
            assert!(
                flags.stop.load(Ordering::Relaxed),
                "the starting server is told to stop"
            );
            gate.send(()).unwrap();
            while event_loop.0.get() == 0 {
                game.idle(&event_loop);
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(event_loop.0.get(), 1);
            assert_eq!(
                game.error.as_ref().map(ToString::to_string).as_deref(),
                Some("the world failed to load")
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn integrated_rules_screen_shares_server_session_registry() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let registry = Arc::new(sunrise_game_data::gamerules::RuleRegistry::with_categories(
                &["example:machines".into()],
            ));
            let request =
                WorldRequest::new("test", None, None, true, None, registry.clone()).unwrap();
            let (tasks, _receiver) = channel();
            game.client.integrated = Some(Integrated {
                emergency_save: None,
                flags: Arc::new(Flags::default()),
                thread: None,
                tasks,
                world_name: "test".into(),
                save_folder: "test".into(),
                rule_registry: registry.clone(),
            });
            assert!(Arc::ptr_eq(
                &request.rule_registry,
                &game.client.integrated.as_ref().unwrap().rule_registry
            ));
            assert!(game.test_open_remote_rules(Vec::new()));
            assert!(
                request
                    .rule_registry
                    .categories()
                    .contains(&"example:machines".to_owned())
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn block_state_weathering_reverts_after_integrated_session_stops() {
        use mod_api::HostBlockStates;
        use sunrise_world::level::{BlockMapValue, BlockMaps};

        super::super::native_entity_refs::tests::with_client_game(|window| {
            window.join_world_rest().unwrap();
            let state = |name| BlockStatesHost::block_state_id(window, name).unwrap();
            let copper = state("minecraft:copper_block");
            let exposed = state("minecraft:exposed_copper");
            let oxidized = state("minecraft:oxidized_copper");
            let next = || window.block_states().next_weathering_stage(copper);
            assert_eq!(next(), Some(exposed));

            for failed_open in [false, true] {
                let mut options = window.options.clone();
                options.dedicated = Some(Vec::new());
                options.execution_role = ExecutionRole::IntegratedServer;
                let mut server = server_game(
                    &window.res,
                    window.pack.clone(),
                    options,
                    window.client.settings.sim_settings(),
                    crate::game::profile_resolver,
                )
                .unwrap();
                let result = server.run_server_lifecycle(false, |server| {
                    // Model publication at open_world_tables, before later steps can fail.
                    let maps = BlockMaps::from_values(
                        [(
                            "minecraft:copper_block",
                            BlockMapValue::Oxidizable("minecraft:oxidized_copper".into()),
                        )],
                        |_| 0,
                    );
                    server.res.behaviours().set_block_maps(Some(Arc::new(maps)));
                    assert_eq!(next(), Some(oxidized));
                    if failed_open {
                        Err("open failed after map publication".into())
                    } else {
                        server.server.world_adopted = true;
                        Ok(sunrise_gameplay::game::server_side::server_lifecycle::ServerRunOutcome::Orderly)
                    }
                });
                if failed_open {
                    assert_eq!(
                        result.unwrap_err().to_string(),
                        "open failed after map publication"
                    );
                } else {
                    result.unwrap();
                }
                // The window and its resource tables outlive both kinds of session.
                assert_eq!(next(), Some(exposed), "failed_open = {failed_open}");
            }
        });
    }
    #[test]
    #[ignore = "daily"]
    fn forced_disconnect_keeps_wire_cache_until_integrated_server_stops() {
        use sunrise_game_data::text::Text;
        use sunrise_sim::lan::data::{SyncData, WorldTags};
        use {
            crate::game::server_side::native_entity_refs::tests::with_client_game,
            sunrise_gameplay_common::game::test_fixtures::run_fresh_cases,
        };

        if run_fresh_cases(
            "cut_layout::game::server_side::integrated::tests::forced_disconnect_keeps_wire_cache_until_integrated_server_stops",
            "SUNRISE_INTEGRATED_DISCONNECT_SYNC_CHILD",
            &["disconnect"],
        ) {
            return;
        }
        with_client_game(|game| {
            for failure in [None, Some("test shutdown failure")] {
                let sync = Arc::new(SyncData::empty());
                sunrise_gameplay::game::server_side::lan::sync::install_wire_reload(
                    &game.options.resources,
                    &game.res.core.item_data,
                    &game.pack_files(),
                    Some(sync.clone()),
                    game.adv.tree.clone(),
                    WorldTags::new(game.res.shared_items(), &game.health_resources.damage),
                );
                let flags = Arc::new(Flags::default());
                let stopping = flags.clone();
                let (tasks, _waiting_server) = channel();
                let (observed, probe) = channel();
                game.client.integrated = Some(Integrated {
                    emergency_save: None,
                    flags,
                    thread: Some(std::thread::spawn(move || {
                        let deadline = Instant::now() + Duration::from_secs(5);
                        while !stopping.stop.load(Ordering::Relaxed) {
                            assert!(Instant::now() < deadline, "stop was never requested");
                            std::thread::sleep(Duration::from_millis(1));
                        }
                        observed
                            .send(
                                sunrise_gameplay::game::server_side::lan::sync::cached_reload_sync(
                                ),
                            )
                            .unwrap();
                        *stopping.failure.lock().unwrap() = failure.map(str::to_owned);
                    })),
                    tasks,
                    world_name: "fixture".into(),
                    save_folder: "fixture".into(),
                    rule_registry: Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla()),
                });
                let result = game.leave_server(Some(Text::literal("test disconnect")));
                assert_eq!(
                    result.err().map(|error| error.to_string()),
                    failure.map(str::to_owned)
                );
                assert!(
                    probe
                        .try_recv()
                        .unwrap()
                        .is_some_and(|seen| Arc::ptr_eq(&seen, &sync)),
                    "the cache must still hold the server's snapshot during shutdown"
                );
                assert!(
                    sunrise_gameplay::game::server_side::lan::sync::cached_reload_sync().is_none()
                );
            }
        });
    }
    #[test]
    #[ignore = "daily"]
    fn client_session_identifies_only_its_own_integrated_server() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let _pipe = sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client(game);
            let (tasks, _waiting_server) = channel();
            game.client.integrated = Some(Integrated {
                emergency_save: None,
                flags: Arc::new(Flags::default()),
                thread: None,
                tasks,
                world_name: "First world".into(),
                save_folder: "first_world".into(),
                rule_registry: Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla()),
            });
            let local = game.client_session().client_server().unwrap();
            assert!(local.integrated_local);
            assert_eq!(local.name, Some("First world"));
            assert_eq!(local.address, None);
            assert_eq!(local.save_folder, Some("first_world"));
            game.client.integrated.as_mut().unwrap().world_name = "Renamed world".into();
            let renamed = game.client_session().client_server().unwrap();
            assert_eq!(renamed.name, Some("Renamed world"));
            assert_eq!(renamed.save_folder, Some("first_world"));
            game.client.integrated.take();
            let guest = game.client_session().client_server().unwrap();
            assert!(!guest.integrated_local);
            assert_eq!(guest.name, None);
            assert_eq!(guest.save_folder, None);
            assert_eq!(guest.address, None);
        });
    }
    /// Install the mod before building the fixture's frozen block and block entity registries.
    #[test]
    #[ignore = "daily"]
    fn owner_distances_follow_the_server_tick() {
        use mod_api::{Hooks, ModEntry, ServerHost, ServerTickEvent, ServerTickPhase};

        type Read = (i32, i32);
        struct Retune(Arc<Mutex<Vec<Read>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Retune {
            fn server_tick(&self, host: &mut dyn ServerHost, event: &mut ServerTickEvent<'_>) {
                if !matches!(event.phase, ServerTickPhase::ServerTickStart { .. }) {
                    return;
                }
                self.0.lock().unwrap().push((
                    host.server_view_distance().unwrap(),
                    host.server_simulation_distance().unwrap(),
                ));
                assert!(host.server_set_view_distance(6));
                assert!(host.server_set_simulation_distance(5));
            }
        }

        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::server_side::integrated::tests::owner_distances_follow_the_server_tick",
            || {
                let seen = Arc::new(Mutex::new(Vec::new()));
                let mods = vec![ModEntry::new("retune", Box::new(Retune(seen.clone())))];
                mod_platform::testing::with_mods(mods.into(), || {
                    sunrise_gameplay::game::server_side::native_guest_turn_tests::with_guests(
                        |game| {
                            let serving = Serving::fixture(0);
                            let mut window = Integrated::stopped_for_test();
                            window.flags = serving.flags.clone();
                            let mut smoothed = 0.0;
                            game.server.serving = Some(serving);
                            game.native_lifecycle_world_ready();
                            let live = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                                let config = &game.server.lan.as_ref().unwrap().hub.config;
                                (config.view_distance, config.simulation_distance)
                            };

                            window.set_distances(48, 7, 1.0);
                            game.tick_integrated_unpaused(&window.flags, &mut smoothed)
                                .unwrap();
                            assert_eq!(live(game), (48, 7));
                            game.tick_integrated_unpaused(&window.flags, &mut smoothed)
                                .unwrap();
                            assert_eq!(live(game), (48, 5));

                            window.set_distances(50, 9, 1.0);
                            game.tick_integrated_unpaused(&window.flags, &mut smoothed)
                                .unwrap();
                            assert_eq!(*seen.lock().unwrap(), [(2, 2), (48, 7), (48, 5)]);
                            assert_eq!(live(game), (50, 9));
                            assert_eq!(
                                (
                                    game.server.settings.render_distance,
                                    game.server.settings.simulation_distance
                                ),
                                (32, 9)
                            );
                        },
                    );
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn integrated_quit_unloads_the_client_before_requesting_server_stop() {
        use super::super::native_entity_refs::tests::with_client_game;
        use super::{Flags, Integrated, channel};
        use mod_api::{ClientHost, ClientLevelUnloadEvent, Hooks, ModEntry};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use sunrise_client_gameplay::game::client_side::{
            native_client_session::tests::logged_client,
            remote_client::session_state::RemoteDimensionEntry,
        };
        use sunrise_client_render::level_loading::Reason;
        use sunrise_protocol::packets::play::SpawnInfo;

        struct Log(Arc<Flags>, Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl Hooks for Log {
            fn client_level_unload(
                &self,
                _: &mut dyn ClientHost,
                event: &mut ClientLevelUnloadEvent<'_>,
            ) {
                assert_eq!(event.dimension, "minecraft:overworld");
                assert!(
                    !self.0.stop.load(Ordering::Relaxed),
                    "the client unloads before halt"
                );
                self.1.fetch_add(1, Ordering::Relaxed);
            }
        }
        for closing_screen in [true, false] {
            let flags = Arc::new(Flags::default());
            let heard = Arc::new(AtomicUsize::new(0));
            let mods = vec![ModEntry::new(
                "unload",
                Box::new(Log(flags.clone(), heard.clone())),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_client_game(|game| {
                    let _server = logged_client(game);
                    let login = SpawnInfo {
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
                    game.enter_remote_dimension(&login, RemoteDimensionEntry::Login, Reason::Other);
                    game.client.integrated = Some(Integrated {
                        flags: flags.clone(),
                        thread: None,
                        emergency_save: None,
                        tasks: channel().0,
                        world_name: "Test".into(),
                        save_folder: "test".into(),
                        rule_registry: Arc::new(
                            sunrise_game_data::gamerules::RuleRegistry::vanilla(),
                        ),
                    });
                    if closing_screen {
                        game.begin_closing_save(true).unwrap();
                        assert!(game.poll_closing_save().unwrap());
                    }
                    game.save_for_exit().unwrap();
                    assert!(flags.stop.load(Ordering::Relaxed));
                    assert_eq!(heard.load(Ordering::Relaxed), 1);
                });
            });
        }
    }

    #[test]
    #[ignore = "daily"]
    fn new_authenticated_world_saves_profile_without_player_before_login() {
        use sunrise_core::nbt::Tag;
        use sunrise_gameplay::game::server_side::integrated::serve;
        use sunrise_gameplay_common::game::integrated_link::Launch;
        use sunrise_sim::{lan::playerdata, save};

        super::super::native_entity_refs::tests::with_client_game(|window| {
            window.join_world_rest().unwrap();
            let dir = tempfile::tempdir().unwrap();
            // Deliberately not the offline UUID for Tester.
            let owner = 0x1234_5678_90ab_cdef_0123_4567_89ab_cdef;
            let mut options = window.options.clone();
            options.dedicated = Some(Vec::new());
            options.execution_role = ExecutionRole::IntegratedServer;
            options.headless = true;
            options.test_terrain = true;
            let mut settings = window.client.settings.sim_settings();
            settings.render_distance = 2;
            settings.simulation_distance = 2;
            let launch = Launch {
                online_auth: false,
                session_verifier: crate::game::session_verifier(),
                rule_registry: Arc::new(
                    sunrise_gameplay_common::game::api_values::game_rule_registry(),
                ),
                folder: dir.path().to_string_lossy().into_owned(),
                name: Some("Pre-login".into()),
                seed: Some(1),
                creative: true,
                create: None,
                owner,
                player: "Tester".into(),
                owner_profile: None,
            };
            let flags = Arc::new(Flags::default());
            // Stop as soon as startup finishes, without connecting the owner.
            flags.stop.store(true, Ordering::Relaxed);
            let (ready_tx, ready_rx) = channel();
            let (_tasks_tx, tasks_rx) = channel();
            serve(
                &window.res,
                window.pack.clone(),
                options,
                settings,
                launch,
                flags,
                tasks_rx,
                &ready_tx,
                crate::game::profile_resolver,
            )
            .unwrap();
            assert_eq!(ready_rx.try_recv().unwrap().unwrap().1, "Pre-login");
            let profile = Tag::IntArray(playerdata::uuid_ints(owner).to_vec());
            // Shutdown rotates the initial save into level.dat_old. Both name the profile.
            for file in ["level.dat_old", "level.dat"] {
                let root = save::read_dat(&dir.path().join(file)).unwrap();
                let data = root.get("Data").unwrap();
                assert_eq!(data.get("singleplayer_uuid"), Some(&profile));
                assert!(data.get("Player").is_none());
            }
            assert!(!playerdata::path(dir.path(), owner).exists());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn integrated_startup_saves_and_starts_before_handing_out_ready() {
        use mod_api::{Hooks, ModEntry, ServerHost};
        use mod_platform::testing::with_mods;
        use std::sync::{Mutex, mpsc::Receiver};
        use sunrise_core::nbt::Tag;
        use sunrise_gameplay::game::server_side::integrated::serve;
        use sunrise_gameplay_common::game::integrated_link::{Launch, Ready};
        use sunrise_protocol::crypto::offline_uuid;
        use sunrise_sim::{lan::playerdata, save};

        struct Order {
            events: Arc<Mutex<Vec<String>>>,
            ready_rx: Arc<Mutex<Receiver<Ready>>>,
            flags: Arc<Flags>,
            player_file: std::path::PathBuf,
            player_bytes: Vec<u8>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Order {
            fn server_about_to_start(
                &self,
                _: &mut dyn ServerHost,
                _: &mut mod_api::ServerAboutToStartEvent,
            ) {
                assert!(
                    self.ready_rx.lock().unwrap().try_recv().is_err(),
                    "ready must not be sent before server_started"
                );
                self.events.lock().unwrap().push("about-to-start".into());
            }
            fn level_load(&self, _: &mut dyn ServerHost, e: &mut mod_api::LevelLoadEvent<'_>) {
                self.events
                    .lock()
                    .unwrap()
                    .push(format!("level-load:{}", e.dimension));
            }
            fn level_save(&self, _: &mut dyn ServerHost, e: &mut mod_api::LevelSaveEvent<'_>) {
                self.events
                    .lock()
                    .unwrap()
                    .push(format!("level-save:{}", e.dimension));
            }
            fn world_saved(&self, _: &mut dyn ServerHost, _: &mut mod_api::WorldSavedEvent) {
                if self.events.lock().unwrap().iter().all(|e| e != "started") {
                    assert!(
                        self.ready_rx.lock().unwrap().try_recv().is_err(),
                        "ready must not be sent during initial save"
                    );
                }
                assert_eq!(std::fs::read(&self.player_file).unwrap(), self.player_bytes);
                self.events.lock().unwrap().push("saved".into());
            }
            fn server_starting(
                &self,
                _: &mut dyn ServerHost,
                _: &mut mod_api::ServerStartingEvent,
            ) {
                assert!(
                    self.ready_rx.lock().unwrap().try_recv().is_err(),
                    "ready must not be sent before server_started"
                );
                self.events.lock().unwrap().push("starting".into());
            }
            fn server_started(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStartedEvent) {
                assert!(
                    self.ready_rx.lock().unwrap().try_recv().is_err(),
                    "ready is sent only after server_started"
                );
                self.events.lock().unwrap().push("started".into());
                self.flags.stop.store(true, Ordering::Relaxed);
            }
            fn server_stopping(
                &self,
                _: &mut dyn ServerHost,
                _: &mut mod_api::ServerStoppingEvent,
            ) {
                self.events.lock().unwrap().push("stopping".into());
            }
            fn level_unload(&self, _: &mut dyn ServerHost, e: &mut mod_api::LevelUnloadEvent<'_>) {
                self.events
                    .lock()
                    .unwrap()
                    .push(format!("level-unload:{}", e.dimension));
            }
            fn server_stopped(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStoppedEvent) {
                self.events.lock().unwrap().push("stopped".into());
            }
        }

        super::super::native_entity_refs::tests::with_client_game(|window| {
            window.join_world_rest().unwrap();
            let dir = tempfile::tempdir().unwrap();
            let owner = offline_uuid("Tester");
            let position = [8.5, 90.0, -4.5];
            let mut initial = save::Level {
                name: "Integrated Startup Order".into(),
                seed: 1,
                player_uuid: Some(playerdata::uuid_ints(owner)),
                position: Some(position),
                dimension: "minecraft:the_nether".into(),
                world_spawn: Some([0, 80, 0]),
                xp_level: 7,
                inventory: vec![(0, "minecraft:diamond".into(), 2, 0)],
                ..save::Level::default()
            };
            initial.write(dir.path(), window.res.items()).unwrap();
            save::biome_regions::create(dir.path()).unwrap();
            // An existing experimental world whose preset omits the player's saved Nether.
            let settings_path = dir.path().join("data/minecraft/world_gen_settings.dat");
            let mut settings = save::read_dat(&settings_path).unwrap();
            let Some(Tag::Compound(stems)) = settings
                .get_mut("data")
                .and_then(|data| data.get_mut("dimensions"))
            else {
                panic!("saved worldgen contains its stems");
            };
            assert!(stems.remove(&"minecraft:the_nether".into()).is_some());
            save::write_dat(&settings_path, settings).unwrap();
            let player_file = playerdata::path(dir.path(), owner);
            let player_bytes = std::fs::read(&player_file).unwrap();
            let folder = dir.path().to_string_lossy().into_owned();
            let flags = Arc::new(Flags::default());
            let (ready_tx, ready_rx) = channel();
            let ready_rx = Arc::new(Mutex::new(ready_rx));
            let (_tasks_tx, tasks_rx) = channel();
            let events = Arc::new(Mutex::new(Vec::new()));
            let mut options = window.options.clone();
            options.dedicated = Some(Vec::new());
            options.execution_role = ExecutionRole::IntegratedServer;
            options.headless = true;
            let mut settings = window.client.settings.sim_settings();
            settings.render_distance = 2;
            settings.simulation_distance = 2;
            let launch = Launch {
                online_auth: false,
                session_verifier: crate::game::session_verifier(),
                rule_registry: Arc::new(
                    sunrise_gameplay_common::game::api_values::game_rule_registry(),
                ),
                folder,
                name: Some("Integrated Startup Order".into()),
                seed: Some(1),
                creative: true,
                create: None,
                owner,
                player: "Tester".into(),
                owner_profile: None,
            };
            with_mods(
                vec![ModEntry::new(
                    "order",
                    Box::new(Order {
                        events: events.clone(),
                        ready_rx: ready_rx.clone(),
                        flags: flags.clone(),
                        player_file: player_file.clone(),
                        player_bytes: player_bytes.clone(),
                    }),
                )]
                .into(),
                || {
                    serve(
                        &window.res,
                        window.pack.clone(),
                        options,
                        settings,
                        launch,
                        flags.clone(),
                        tasks_rx,
                        &ready_tx,
                        crate::game::profile_resolver,
                    )
                    .unwrap();
                },
            );
            let received = ready_rx.lock().unwrap().try_recv().unwrap();
            assert_eq!(received.unwrap().1, "Integrated Startup Order");
            assert_eq!(std::fs::read(&player_file).unwrap(), player_bytes);
            let reopened = save::Level::read(dir.path()).unwrap();
            assert_eq!(reopened.position, Some(position));
            assert_eq!(reopened.dimension, "minecraft:the_nether");
            assert_eq!(reopened.xp_level, 7);
            assert_eq!(reopened.inventory, initial.inventory);
            assert_eq!(
                *events.lock().unwrap(),
                [
                    "about-to-start",
                    "level-load:minecraft:overworld",
                    "level-load:minecraft:the_end",
                    "level-save:minecraft:overworld",
                    "level-save:minecraft:the_end",
                    "saved",
                    "starting",
                    "started",
                    "stopping",
                    "level-save:minecraft:overworld",
                    "level-save:minecraft:the_end",
                    "saved",
                    "level-unload:minecraft:overworld",
                    "level-unload:minecraft:the_end",
                    "stopped",
                ]
            );
        });
    }

    fn with_checkpoint_mod(
        test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &Mutex<Vec<CheckpointEvent>>),
    ) {
        use {
            sunrise_gameplay::game::native_entity_refs::tests::fixture_root,
            sunrise_gameplay::game::native_entity_refs::tests::with_game_at,
        };

        let events = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "crash_checkpoint",
            Box::new(CheckpointMod(events.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods.clone(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            mod_api::__private::mark_game_thread();
            let resources = sunrise_gameplay_common::game::mods::content_root(
                &fixture_root(),
                &Default::default(),
            )
            .unwrap();
            with_game_at(resources.prepared().to_path_buf(), |game| {
                test(game, &events)
            });
        });
    }
    #[test]
    #[ignore = "daily"]
    fn integrated_tick_panic_saves_changed_world() {
        use sunrise_core::nbt::Tag;
        use sunrise_level::{
            chunk::Chunk,
            containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields},
        };
        use sunrise_sim::save::{LevelShape, SaveDir, SessionLock};
        use {
            sunrise_gameplay::game::server_side::native_living_refs::tests::hurt_pig,
            sunrise_gameplay::game::server_side::native_living_refs::tests::send_to_the_nether,
            sunrise_gameplay::game::server_side::native_living_refs::tests::summon_mob,
            sunrise_gameplay_common::game::server_side::native_living_refs::tests::ZOMBIE_UUID,
            sunrise_gameplay_common::game::test_fixtures::run_fresh_cases,
        };

        const NAME: &str = "cut_layout::game::server_side::integrated::tests::integrated_tick_panic_saves_changed_world";
        // The mod's block and block entity declarations freeze process-wide registries.
        if run_fresh_cases(NAME, "SUNRISE_CRASH_CHECKPOINT_CHILD", &["crash"]) {
            return;
        }
        with_checkpoint_mod(|game, events| {
            let scratch = tempfile::tempdir().unwrap();
            let root = scratch.path();
            let open = || {
                SaveDir::new(
                    root.to_path_buf(),
                    LevelShape::OVERWORLD,
                    Some(Default::default()),
                )
            };
            game.server.world_root = root.to_path_buf();
            game.server.save.0 = Arc::new(open());
            // Crash shutdown only saves an adopted world whose storage is owned.
            game.server.session_lock.0 = Some(SessionLock::acquire(root).unwrap());
            game.server.world_adopted = true;
            game.native_level_opened(Dim::Overworld);
            game.native_lifecycle_world_ready();

            if !game.world.chunks.contains_key(&(0, 0)) {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            }
            let (victim, attacker) = hurt_pig(game);
            send_to_the_nether(game, attacker);
            let bystander = summon_mob(game, "minecraft:pig", "{NoAI:1b,UUID:[I;21,22,23,24]}");
            let victim_uuid = Tag::IntArray(game.persist.uuid_of(victim).unwrap().to_vec());
            let bystander_uuid = Tag::IntArray(game.persist.uuid_of(bystander).unwrap().to_vec());
            let slot = game.mobs.iter().position(|m| m.id == victim).unwrap();
            let ticks_since_hurt = 17;
            assert!(game.mobs[slot].ai.last_hurt_by_ref.is_none());
            game.mobs.update(slot, |m| {
                m.age = (m.ai.last_hurt_by_time + ticks_since_hurt) as u32;
            });
            // The victim joined first, but re-enters the bystander's section last.
            let at = game.mobs[slot].position;
            let outside = sunrise_core::geometry::Vec3::new(at.x, 50.0, at.z);
            for (from, to) in [(at, outside), (outside, at)] {
                game.mobs.update(slot, |m| m.position = to);
                game.entity_section_step(victim, [from.x, from.y, from.z], [to.x, to.y, to.z]);
            }
            let living = game
                .living(EntitiesHost::entity(game, victim).unwrap())
                .unwrap();
            assert_eq!(
                living.last_attacker(game).unwrap().level(game),
                EntitiesHost::level(game, Dim::Nether.id())
            );

            let block = game
                .pack
                .assets
                .registry
                .default_state("crash_checkpoint:counter")
                .unwrap();
            game.world.set(1, 64, 1, block).unwrap();
            game.world.block_entities.insert(
                (1, 64, 1),
                BlockEntity::Mod(Box::new(ModBlockEntity::new(
                    "crash_checkpoint:counter".into(),
                    ModBlockEntityFields {
                        nbt: [("progress".into(), Tag::Int(41))].into(),
                        ..Default::default()
                    },
                ))),
            );
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::new(8.5, 64.0, 8.5),
                )
                .unwrap(),
            );
            // The owner's file as last saved, with an experience total the panicking tick's
            // state (37) differs from.
            game.level.xp_total = 11;
            game.common
                .level
                .write_player(root, game.common.res.items())
                .unwrap();
            let owner_file = std::fs::read_dir(root.join("players").join("data"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            let owner_xp = || {
                let player = sunrise_sim::save::read_dat(&owner_file)?;
                match player.get("XpTotal") {
                    Some(sunrise_core::nbt::Tag::Int(xp)) => Some(*xp),
                    _ => None,
                }
            };
            assert_eq!(owner_xp(), Some(11));
            let flags = Arc::new(Flags::default());
            flags.fail_tick.store(true, Ordering::Relaxed);
            flags.published.store(1234, Ordering::Relaxed);
            let (_sender, tasks) = channel();
            game.server.serving = Some(Serving {
                flags: flags.clone(),
                tasks,
                owner: 1,
                pending: None,
                previous_simulation_distance: 0,
                load: LoadPhase::default(),
            });
            events.lock().unwrap().clear();
            let error = game
                .run_server_lifecycle(false, |game| {
                    game.integrated_tick_loop(&flags)?;
                    Ok(sunrise_gameplay::game::server_side::server_lifecycle::ServerRunOutcome::Orderly)
                })
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                "the server thread panicked: integrated tick fixture"
            );
            let saved = sunrise_sim::save::Level::read(root).unwrap();
            assert_eq!(saved.name, "saved after integrated tick panic");
            // A fresh reader has no pending writes or captured payload to fall back to.
            let disk = open();
            let chunk = disk
                .raw_chunk(0, 0)
                .expect("crash checkpoint wrote the chunk");
            let Some(Tag::List(entities)) = chunk.tag.get("block_entities") else {
                panic!("crash checkpoint lost the block entities");
            };
            let counter = entities
                .iter()
                .find(|tag| tag.get("id") == Some(&Tag::String("crash_checkpoint:counter".into())))
                .expect("crash checkpoint lost the counter");
            assert_eq!(counter.get("saved_progress"), Some(&Tag::Int(41)));
            let entities = disk.load_entities(0, 0).unwrap();
            let ids: Vec<_> = entities.iter().map(|tag| tag.get("UUID")).collect();
            assert_eq!(ids, [Some(&bystander_uuid), Some(&victim_uuid)]);
            let victim = &entities[1];
            assert_eq!(
                victim.get("last_hurt_by_mob"),
                Some(&Tag::IntArray(ZOMBIE_UUID.to_vec()))
            );
            assert_eq!(
                victim.get("ticks_since_last_hurt_by_mob"),
                Some(&Tag::Int(ticks_since_hurt))
            );
            assert_eq!(
                *events.lock().unwrap(),
                [
                    CheckpointEvent::LevelSaved,
                    CheckpointEvent::WorldSaved,
                    CheckpointEvent::LevelUnloaded,
                    CheckpointEvent::ServerStopped,
                ]
            );
            // No hub: players are not saved, so the owner's file keeps what it had.
            assert_eq!(owner_xp(), Some(11));
            assert_eq!(flags.published.load(Ordering::Relaxed), 0);
            assert!(game.server.session_lock.0.is_none());
        });
    }
}
