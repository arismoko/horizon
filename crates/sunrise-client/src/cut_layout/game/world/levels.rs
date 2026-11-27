#![cfg(test)]
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use sunrise_gameplay_common::game::Dim;
use sunrise_sim::save::SaveDir;
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_gameplay::game::ServerRunOutcome;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycleCommonGameExt1;
    use sunrise_gameplay_common::game::server_side::levels::tests::LOGGED;
    use sunrise_gameplay_common::game::tables::WorldTablesExt;

    #[cfg(test)]
    struct LogCapture;
    #[cfg(test)]
    impl log::Log for LogCapture {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }
        fn log(&self, record: &log::Record) {
            LOGGED.lock().unwrap().push(record.args().to_string());
        }
        fn flush(&self) {}
    }

    use super::*;

    /// A game on the prepared resources in `SUNRISE_TEST_RESOURCES`, with no
    /// world opened and commands allowed.
    fn prepared_game<'a>(
        resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
    ) -> sunrise_gameplay::game::ServerGame<'a> {
        use {
            crate::game::load_resources, crate::settings::Settings,
            sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
        };
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        // The game keeps its resources for the whole test.
        let resources = &*resources.insert(loaded.resources);
        let mut game = sunrise_gameplay::game::ServerGame::new(
            resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        game
    }
    /// `WorldBorderCommand.setSize` from `/worldborder add` with a time: the
    /// new change lasts the ticks still running plus the argument
    /// (`getLerpTime() + time`), where `/worldborder set` replaces it.
    #[test]
    #[ignore = "daily"]
    fn worldborder_add_extends_a_change_in_flight() {
        let mut resources = None;
        let mut game = prepared_game(&mut resources);
        assert!(game.run_command("worldborder set 100 1000"));
        assert_eq!(game.border.remaining_ticks(), 1000);
        // Part way through the change, where the size is between its ends.
        for _ in 0..100 {
            game.border.tick().unwrap();
        }
        assert_eq!(game.border.remaining_ticks(), 900);
        let from = game.border.size();
        assert!(from < 59_999_968.0 && from > 100.0);
        // The new change starts from that size and lasts the ticks still
        // running plus the argument, not just the argument.
        assert!(game.run_command("worldborder add -50 5s"));
        assert_eq!(game.border.target_size(), from - 50.0);
        assert_eq!(game.border.remaining_ticks(), 1000);
        // `set` starts over from the size the border has now.
        assert!(game.run_command("worldborder set 200 40"));
        assert_eq!(game.border.target_size(), 200.0);
        assert_eq!(game.border.remaining_ticks(), 40);
    }
    /// `ServerLevel.getWorldBorder`: `/worldborder` in one level leaves the others' alone.
    #[test]
    #[ignore = "daily"]
    fn each_level_keeps_its_own_world_border() {
        let mut resources = None;
        let mut game = prepared_game(&mut resources);
        assert!(game.run_command("worldborder set 1000"));
        assert!(game.run_command("execute in minecraft:the_nether run worldborder set 100"));
        assert_eq!(game.dim, Dim::Overworld);
        assert_eq!(game.border.size(), 1000.0);
        let nether = game.server.levels.0.parked_border(Dim::Nether).unwrap();
        assert_eq!(nether.size(), 100.0);
    }
    /// 300 slow ticks in ten seconds log no Sunrise slow-tick line, and the ring holds the last 256.
    #[test]
    #[ignore = "daily"]
    fn slow_ticks_enter_the_ring_and_not_the_log() {
        use sunrise_sim::server_perf::{ServerPerf, TickDetail, WorldCounts};
        use {
            crate::game::load_resources, crate::settings::Settings,
            sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
        };
        log::set_logger(&LogCapture).expect("no other logger in this test run");
        log::set_max_level(log::LevelFilter::Info);
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        options.spikes = Some(0.0);
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.build_registries();
        let mut perf = ServerPerf::new(0.0, 0.0);
        for i in 0..300 {
            let mut detail = None;
            let started = Instant::now();
            game.tick_server_spans(false, |timing| {
                detail = Some(TickDetail {
                    cpu_ms: timing.cpu_ms,
                    spans: timing.raw,
                });
            })
            .unwrap();
            let wall_ms = started.elapsed().as_secs_f32() * 1000.0;
            perf.tick(
                f64::from(i) / 30.0,
                wall_ms,
                detail,
                WorldCounts::default,
                Vec::new,
            );
        }
        let kept: Vec<u64> = perf.slow_records().map(|r| r.counter).collect();
        assert_eq!(kept, (45..=300).collect::<Vec<u64>>());
        let logged = LOGGED.lock().unwrap();
        assert!(
            logged.iter().all(|line| !line.contains("slow tick")),
            "{logged:?}"
        );
    }
    /// A game on a scratch world folder, the way `dedicated.rs` opens one
    /// (`load_world` with an absolute folder).
    fn open_game<'a>(
        resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
        root: &std::path::Path,
        home: &std::path::Path,
        world: &std::path::Path,
    ) -> sunrise_gameplay::game::ServerGame<'a> {
        use {
            crate::game::load_resources, crate::settings::Settings,
            sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
        };
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            Vec::new(),
            None,
            root,
            &settings,
            Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        // The game keeps a reference to its resources for the whole test.
        let resources = resources.insert(loaded.resources);
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.to_path_buf();
        options.game_dir = Some(home.to_path_buf());
        // A fixed name: no generated sunrise-player.txt in the working directory.
        options.player_name = Some("H260".into());
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        game.native_lifecycle_open_started();
        let folder = world.to_string_lossy().into_owned();
        game.load_world(
            sunrise_gameplay::game::world_loading::WorldRequest::new(
                &folder,
                Some("h260"),
                Some(260),
                true,
                None,
                std::sync::Arc::new(
                    sunrise_gameplay_common::game::api_values::game_rule_registry(),
                ),
            )
            .unwrap(),
        )
        .unwrap();
        game
    }

    #[test]
    #[ignore = "daily"]
    fn late_chunk_saves_use_their_opened_levels_disk_context() {
        use mod_api::{
            ChunkDataSaveEvent, ChunkDiskContext, ChunkDiskHost, ChunkDiskKind, ChunkDiskPhase,
            Hooks, Identifier, ModEntry,
        };
        use std::path::PathBuf;
        use std::sync::Mutex;
        use sunrise_client::chunk::Chunk;

        struct ObserveSaves(Arc<Mutex<Vec<ChunkDiskContext>>>);
        #[mod_api::__subscriptions]
        impl Hooks for ObserveSaves {
            fn chunk_data_save(&self, host: &dyn ChunkDiskHost, _: &mut ChunkDataSaveEvent<'_>) {
                self.0.lock().unwrap().push(host.job().clone());
            }
        }

        let root = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        // Opening a world publishes the server's worldgen tables. Isolate that
        // publication from the other tests, even though the mod list is scoped.
        if crate::game::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::server_side::levels::tests::late_chunk_saves_use_their_opened_levels_disk_context",
            "SUNRISE_CHUNK_DISK_CONTEXT_TEST_CHILD",
            &["1"],
        ) {
            return;
        }
        // The fresh child runs in its own scratch home.
        let home = std::env::current_dir().unwrap();
        let world = home.join("world");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new(
            "level_disk_context",
            Box::new(ObserveSaves(seen.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            let mut resources = None;
            let game = open_game(&mut resources, &root, &home, &world);
            let epoch = game.native_lifecycle.epoch().unwrap();

            // The streamer can send a late chunk straight to its writer, even
            // while its level is parked and the Overworld is in place.
            game.server
                .streaming
                .save_finished(Chunk::with_layout(2, -3, game.world.layout()));
            game.server.save.0.flush().unwrap();
            let nether = &game.server.levels.0.parked[&Dim::Nether];
            nether
                .streaming
                .save_finished(Chunk::with_layout(-4, 5, nether.world.layout()));
            nether.save.flush().unwrap();

            assert_eq!(
                seen.lock().unwrap().as_slice(),
                [
                    ChunkDiskContext::new(
                        epoch,
                        Identifier::parse("minecraft:overworld").unwrap(),
                        [2, -3],
                        ChunkDiskKind::Full,
                        ChunkDiskPhase::Save,
                    ),
                    ChunkDiskContext::new(
                        epoch,
                        Identifier::parse("minecraft:the_nether").unwrap(),
                        [-4, 5],
                        ChunkDiskKind::Full,
                        ChunkDiskPhase::Save,
                    ),
                ],
            );
        });
    }
    /// `MinecraftServer.prepareLevels` activates each level's saved tickets and
    /// waits for their ticking chunks without a player visiting either level.
    #[test]
    #[ignore = "daily"]
    fn built_in_levels_restore_saved_tickets_before_the_first_tick() {
        use mod_api::{
            Hooks, LevelRef, ModEntry, Registrar, RegistrationContext, ServerHost,
            TicketController, TicketValidation,
        };
        use std::sync::{
            OnceLock,
            atomic::{AtomicBool, Ordering},
        };
        use sunrise_level::mod_chunk_tickets::{ModTicket, ModTickets, TicketOwner};
        use sunrise_sim::save::dimension_dir;

        const OWNER: [i32; 3] = [2, 64, 3];
        struct Quarry {
            saw_end: Arc<AtomicBool>,
            controller: Arc<OnceLock<TicketController>>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Quarry {
            fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
                let saw_end = Arc::clone(&self.saw_end);
                let controller = registrar.register_ticket_controller(
                    "quarry",
                    Some(Arc::new(
                        move |host: &mut dyn ServerHost,
                              level: LevelRef,
                              validation: &mut dyn TicketValidation| {
                            if level.dimension_id() == "minecraft:the_nether"
                                && host.level("minecraft:the_end").is_some()
                            {
                                saw_end.store(true, Ordering::Relaxed);
                            } else {
                                validation.remove_block_tickets(OWNER);
                            }
                        },
                    )),
                );
                let _ = self.controller.set(controller);
            }
        }

        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let home = tempfile::tempdir().unwrap();
        let world = home.path().join("world");
        let saw_end = Arc::new(AtomicBool::new(false));
        let controller_cell = Arc::new(OnceLock::new());
        let mods = vec![ModEntry::new(
            "startup_tickets",
            Box::new(Quarry {
                saw_end: saw_end.clone(),
                controller: controller_cell.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            {
                let mut resources = None;
                let mut game = open_game(&mut resources, &root, home.path(), &world);
                game.save_game_metadata().unwrap();
            }
            let nether = Dim::Nether;
            let end = Dim::End;
            let commanded_chunk = (4, -5);
            let controller = *controller_cell.get().unwrap();
            let controller_slot =
                mod_api::__private::game::ticket_controller_slot(controller).unwrap();
            let ticket = ModTicket {
                controller: controller_slot,
                owner: TicketOwner::Block(OWNER),
                chunk: (12, -13),
                natural_spawning: false,
            };
            SaveDir::new(dimension_dir(&world, end.id()), end.shape(), None)
                .save_chunk_tickets(&[commanded_chunk].into(), &[])
                .unwrap();
            let mut tickets = ModTickets::default();
            tickets.insert(ticket.clone());
            SaveDir::new(dimension_dir(&world, nether.id()), nether.shape(), None)
                .save_mod_tickets(&tickets, mod_api::__private::game::ticket_controller_id)
                .unwrap();

            let mut resources = None;
            let mut game = open_game(&mut resources, &root, home.path(), &world);
            assert!(!saw_end.load(Ordering::Relaxed), "validation has not run");
            assert!(
                game.server.levels.0.parked[&end]
                    .streaming
                    .commanded()
                    .is_empty()
            );
            // An early final save can run after the world is adopted but
            // before ticket activation. The originals on disk must survive.
            game.save_all_levels(false).unwrap();
            game.prepare_levels().unwrap();
            assert!(
                saw_end.load(Ordering::Relaxed),
                "Nether validator sees the End"
            );
            assert_eq!(game.dim, Dim::Overworld);
            let nether_level = &game.server.levels.0.parked[&nether];
            assert_eq!(
                nether_level
                    .streaming
                    .mod_tickets()
                    .of_controller(controller_slot)
                    .next(),
                Some(&ticket)
            );
            let end_level = &game.server.levels.0.parked[&end];
            assert!(end_level.streaming.commanded().contains(&commanded_chunk));
            let ticking = |game: &sunrise_gameplay::game::ServerGame<'_>, dim, (x, z)| {
                let world = if dim == game.dim {
                    &game.world
                } else {
                    &game.server.levels.0.parked[&dim].world
                };
                sunrise_gameplay_common::game::ticking_chunk(world, x, z)
            };
            assert!(ticking(&game, nether, ticket.chunk));
            assert!(ticking(&game, end, commanded_chunk));
            game.tick_server(false).unwrap();
            assert!(ticking(&game, nether, ticket.chunk));
            assert!(ticking(&game, end, commanded_chunk));
        });
    }
    /// `WorldDimensions.BUILTIN_ORDER` is independent of a saved player's dimension.
    #[test]
    #[ignore = "daily"]
    fn built_in_level_load_order_ignores_the_saved_player_dimension() {
        use mod_api::{Hooks, LevelLoadEvent, ModEntry, ServerHost};
        use std::sync::{Arc, Mutex};

        const BUILT_INS: [&str; 3] = [
            "minecraft:overworld",
            "minecraft:the_nether",
            "minecraft:the_end",
        ];
        struct Observer(Arc<Mutex<Vec<(String, [bool; 3])>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn level_load(&self, host: &mut dyn ServerHost, event: &mut LevelLoadEvent<'_>) {
                assert_eq!(host.dimension(), event.dimension);
                let available = BUILT_INS.map(|id| host.level(id).is_some());
                self.0
                    .lock()
                    .unwrap()
                    .push((event.dimension.to_owned(), available));
            }
        }

        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let home = tempfile::tempdir().unwrap();
        let world = home.path().join("world");
        {
            let mut resources = None;
            let mut game = open_game(&mut resources, &root, home.path(), &world);
            game.level.dimension = "minecraft:the_nether".into();
            game.level.position = Some([0.0, 64.0, 0.0]);
            game.common
                .level
                .write(&world, game.common.res.items())
                .unwrap();
        }

        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new(
            "startup_order",
            Box::new(Observer(seen.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            let mut resources = None;
            let game = open_game(&mut resources, &root, home.path(), &world);
            assert_eq!(game.dim, Dim::Nether);
        });
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), BUILT_INS.len());
        assert_eq!(seen[0], (BUILT_INS[0].into(), [true, false, false]));
        let mut loaded = [true, false, false];
        let mut names: Vec<&str> = vec![seen[0].0.as_str()];
        for (dim, available) in &seen[1..] {
            let idx = BUILT_INS
                .iter()
                .position(|&id| id == dim)
                .expect("built-in dimension");
            assert!(!loaded[idx], "duplicate level_load for {dim}");
            loaded[idx] = true;
            assert_eq!(*available, loaded);
            names.push(dim.as_str());
        }
        names.sort_unstable();
        let mut expected = BUILT_INS;
        expected.sort_unstable();
        assert_eq!(names, expected);
    }
    /// Vanilla 26.3 (`MinecraftServer.createLevels` / `MinecraftServer.levels` /
    /// `ServerLevel.noSave` / `ChunkMap.processUnloads`): built-in levels open
    /// once at startup, stay open and addressable across idle ticks while
    /// unloading released chunks (except during server pause or `/save-off`,
    /// but still unloading during `/tick freeze`), keep ticketed chunks loaded,
    /// do not fire a second `level_load` on a later first visit, and fire
    /// `level_unload` once on normal server shutdown.
    #[test]
    #[ignore = "daily"]
    fn built_in_levels_stay_open_across_idle_ticks_until_shutdown() {
        use crate::game::native_entity_refs::tests::run_fresh_cases;
        use mod_api::{Hooks, LevelLoadEvent, LevelUnloadEvent, ModEntry, ServerHost};
        use std::sync::{Arc, Mutex};
        use sunrise_gameplay::game::lan::LanSettings;
        use sunrise_gameplay_common::game::lan_actions::GuestCommand;
        use sunrise_server_files::user_lists::{BanList, IpBanList, OpList, WhiteList};
        use sunrise_sim::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};

        if run_fresh_cases(
            "cut_layout::game::server_side::levels::tests::built_in_levels_stay_open_across_idle_ticks_until_shutdown",
            "SUNRISE_BUILT_IN_IDLE_LIFETIME_CASE",
            &["built-ins"],
        ) {
            return;
        }

        const BUILT_INS: [&str; 3] = [
            "minecraft:overworld",
            "minecraft:the_nether",
            "minecraft:the_end",
        ];
        struct Observer {
            loads: Arc<Mutex<Vec<String>>>,
            unloads: Arc<Mutex<Vec<String>>>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn level_load(&self, _: &mut dyn ServerHost, event: &mut LevelLoadEvent<'_>) {
                self.loads.lock().unwrap().push(event.dimension.to_owned());
            }
            fn level_unload(&self, _: &mut dyn ServerHost, event: &mut LevelUnloadEvent<'_>) {
                self.unloads
                    .lock()
                    .unwrap()
                    .push(event.dimension.to_owned());
            }
        }

        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let home = tempfile::tempdir().unwrap();
        let world = home.path().join("world");
        let loads = Arc::new(Mutex::new(Vec::new()));
        let unloads = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new(
            "builtin_lifetime",
            Box::new(Observer {
                loads: loads.clone(),
                unloads: unloads.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            let mut resources = None;
            let mut game = open_game(&mut resources, &root, home.path(), &world);
            let initial_loads = loads.lock().unwrap().clone();
            assert_eq!(
                initial_loads.first().map(String::as_str),
                Some("minecraft:overworld")
            );
            let mut sorted_loads: Vec<&str> = initial_loads.iter().map(String::as_str).collect();
            sorted_loads.sort_unstable();
            let mut sorted_builtins = BUILT_INS;
            sorted_builtins.sort_unstable();
            assert_eq!(sorted_loads, sorted_builtins);
            assert!(unloads.lock().unwrap().is_empty());

            // Configure as a dedicated server with real commands (`/save-off`,
            // `/save-on`, `/tick freeze`, `/forceload`) and a local placed guest
            // (no network listener) so ticks, visits and shutdown run normally.
            game.options.execution_role =
                sunrise_gameplay_common::game::execution_role::ExecutionRole::DedicatedServer;
            game.options.dedicated = Some(Vec::new());
            game.server.settings.render_distance = 2;
            game.server.settings.simulation_distance = 2;
            game.server.streaming.radius = -1;
            game.server.server_lists.0 = Some(Arc::new(Mutex::new(
                sunrise_gameplay::game::dedicated::Lists {
                    ops: OpList::new(home.path().join("ops.json")),
                    white: WhiteList::new(home.path().join("whitelist.json")),
                    bans: BanList::new(home.path().join("banned-players.json")),
                    ip_bans: IpBanList::new(home.path().join("banned-ips.json")),
                    use_whitelist: false,
                    enforce_whitelist: false,
                    op_level: 4,
                    player_idle_timeout: 0,
                    stop: false,
                    autosave: true,
                },
            )));
            game.ui.dispatcher = sunrise_gameplay_common::game::chat_common::SessionDispatcher(
                Arc::new(sunrise_command::vanilla::build_for(false, true)),
            );
            game.lan_host(
                LanSettings {
                    port: 0,
                    guest_commands: true,
                    force_game_mode: false,
                },
                NetConfig {
                    listen: false,
                    announce: false,
                    ..NetConfig::lan(0, "builtin-lifetime".into(), 2)
                },
                HubConfig {
                    default_game_mode: 0,
                    guest_commands: true,
                    force_game_mode: false,
                    hardcore: false,
                    view_distance: 2,
                    simulation_distance: 2,
                    dimension: Dim::Overworld.id().into(),
                    allow_flight: false,
                    seed: game.level.seed,
                    max_players: 2,
                    integrated: false,
                    levels: game.server.level_shapes.clone(),
                    level_seeds: game.server.levels.0.runtime.seeds(),
                },
            )
            .unwrap();
            let (tx, _rx) = std::sync::mpsc::channel();
            let joined = Joined {
                conn: 1,
                uuid: 1,
                name: "Visitor".into(),
                properties: Vec::new(),
                info: sunrise_protocol::packets::configuration::ClientInformation::new(2),
                brand: "sunrise".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(std::sync::atomic::AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            };
            game.server.next_entity += 1;
            let mut guest = Guest::new(joined, game.server.next_entity as i32, [0.5, 80.0, 0.5], 0);
            guest.placed = true;
            guest.loaded = true;
            guest.dimension = Dim::Overworld.id().into();
            // Creative: no fall or suffocation where the teleports below land.
            guest.game_mode = sunrise_sim::lan::guest::CREATIVE;
            guest.motion = Some(Box::new(
                sunrise_sim::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::new(0.5, 80.0, 0.5),
                )
                .unwrap(),
            ));
            guest.max_unacknowledged = 1000;
            game.server.lan.as_mut().unwrap().hub.guests.push(guest);

            game.prepare_levels().unwrap();
            game.native_lifecycle_world_ready();

            // Force one chunk in the Nether (to release later) and one in the End
            // (to keep ticketed), then drive real server ticks until both load.
            assert!(game.run_command("execute in minecraft:the_nether run forceload add 0 0"));
            assert!(game.run_command("execute in minecraft:the_end run forceload add 0 0"));
            let mut loaded = false;
            for _ in 0..2000 {
                game.tick_server(false).unwrap();
                let nether_ready = game
                    .server
                    .levels
                    .0
                    .parked
                    .get(&Dim::Nether)
                    .is_some_and(|p| {
                        p.world.chunks.contains_key(&(0, 0)) && p.world.entities_pending.is_empty()
                    });
                let end_ready = game.server.levels.0.parked.get(&Dim::End).is_some_and(|p| {
                    p.world.chunks.contains_key(&(0, 0)) && p.world.entities_pending.is_empty()
                });
                if nether_ready && end_ready {
                    loaded = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(loaded, "forced chunks in Nether and End did not load");

            // Release the Nether ticket while keeping the End chunk ticketed.
            assert!(game.run_command("execute in minecraft:the_nether run forceload remove 0 0"));

            // Server pause (`paused = true`) must not drain the parked Nether chunk.
            for _ in 0..16 {
                game.tick_server(true).unwrap();
            }
            assert!(
                game.server.levels.0.parked[&Dim::Nether]
                    .world
                    .chunks
                    .contains_key(&(0, 0)),
                "paused server must keep parked Nether chunk resident"
            );

            // `/save-off` must keep the parked Nether chunk resident until `/save-on`.
            assert!(game.run_command("save-off"));
            for _ in 0..16 {
                game.tick_server(false).unwrap();
            }
            assert!(
                game.server.levels.0.parked[&Dim::Nether]
                    .world
                    .chunks
                    .contains_key(&(0, 0)),
                "save-off must keep parked Nether chunk resident"
            );
            assert!(game.run_command("save-on"));

            assert!(game.run_command("tick freeze"));
            game.tick_server(false).unwrap();
            assert!(game.server.levels.0.parked.contains_key(&Dim::Nether));
            assert!(game.server.levels.0.parked.contains_key(&Dim::End));
            assert!(
                game.server.levels.0.parked[&Dim::Nether]
                    .world
                    .chunks
                    .is_empty(),
                "released Nether chunk unloaded while parked under /tick freeze"
            );
            assert!(
                !game.run_command_quietly("execute in minecraft:the_nether if loaded 0 64 0"),
                "released Nether chunk reports unloaded"
            );
            assert!(
                game.run_command_quietly("execute in minecraft:the_end if loaded 0 64 0"),
                "ticketed End chunk stays loaded"
            );
            assert!(
                game.run_command("execute in minecraft:the_nether run worldborder get"),
                "idle built-in Nether stays addressable"
            );
            assert!(
                unloads.lock().unwrap().is_empty(),
                "built-in levels stay open after their idle chunk sweep"
            );
            assert!(game.run_command("tick unfreeze"));

            // The guest's first visits to the Nether and the End after the idle
            // period cross levels as a player's own `/execute in ... run tp`
            // does (`ServerPlayer.teleport`): no second `level_load`.
            for dim in [Dim::Nether, Dim::End, Dim::Overworld] {
                assert!(game.run_guest_command(GuestCommand {
                    conn: 1,
                    command: format!("execute in {} run tp @s 0.5 100 0.5", dim.id()),
                    permission: 4,
                    integrated: false,
                    quiet: false,
                }));
                let mut arrived = false;
                for _ in 0..2000 {
                    game.tick_server(false).unwrap();
                    let guest = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
                    if guest.dimension == dim.id() && !game.on_server_trip(1) {
                        arrived = true;
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                assert!(arrived, "the guest never arrived in {dim:?}");
            }
            assert_eq!(*loads.lock().unwrap(), initial_loads);
            assert!(unloads.lock().unwrap().is_empty());

            game.run_server_lifecycle(true, |_| Ok(ServerRunOutcome::Orderly))
                .unwrap();
            assert_eq!(*unloads.lock().unwrap(), initial_loads);
        });
    }
    /// Expected values from the official 26.3 dedicated server restarted on the
    /// same world: each level's `world_border` SavedData is read when the level
    /// is made, so the Overworld's saved border is in place for the first
    /// command, and a first `/worldborder set` is not overwritten later.
    #[test]
    #[ignore = "daily"]
    fn a_reopened_world_has_each_levels_saved_border_from_the_start() {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let home = std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap()).join("h260");
        let world = home.join("saves/h260");
        // A new world each run: a border saved by the last one would already be 50.
        let _ = std::fs::remove_dir_all(&home);
        {
            let mut resources = None;
            let mut game = open_game(&mut resources, &root, &home, &world);
            // A server ticks before it saves.
            game.tick_world(true).unwrap();
            assert!(game.run_command("worldborder set 50"));
            assert!(game.run_command("execute in minecraft:the_nether run worldborder set 100"));
            game.save_game().unwrap();
        }
        let mut resources = None;
        let mut game = open_game(&mut resources, &root, &home, &world);
        let mut failures = Vec::new();
        if game.border.size() != 50.0 {
            failures.push(format!("in place after the open: {}", game.border.size()));
        }
        assert!(game.run_command("worldborder set 70"));
        game.tick_world(true).unwrap();
        assert!(game.run_command("execute in minecraft:the_nether run worldborder get"));
        assert!(game.run_command("execute in minecraft:the_end run worldborder get"));
        assert_eq!(game.dim, Dim::Overworld);
        let sizes = (
            game.border.size(),
            game.server
                .levels
                .0
                .parked_border(Dim::Nether)
                .map(|b| b.size()),
            game.server
                .levels
                .0
                .parked_border(Dim::End)
                .map(|b| b.size()),
        );
        if sizes != (70.0, Some(100.0), Some(59_999_968.0)) {
            failures.push(format!("after the first set and a tick: {sizes:?}"));
        }
        assert!(failures.is_empty(), "reopened borders: {failures:#?}");
    }
    /// Added controls (H260, after the baseline): a new world, and a read and a
    /// set before the first tick that survive a tick, a save and another reopen.
    #[test]
    #[ignore = "daily"]
    fn reopened_borders_keep_through_a_tick_and_a_save_and_a_new_world_has_defaults() {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let home = std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
            .join("h260-controls");
        // A new world each run, as the first open expects.
        let _ = std::fs::remove_dir_all(&home);
        let world = home.join("saves/h260");
        let sizes = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
            assert!(game.run_command("execute in minecraft:overworld run worldborder get"));
            assert!(game.run_command("execute in minecraft:the_nether run worldborder get"));
            assert!(game.run_command("execute in minecraft:the_end run worldborder get"));
            let parked = |d| game.server.levels.0.parked_border(d).map(|b| b.size());
            let size = |d: Dim| {
                if game.dim == d {
                    Some(game.border.size())
                } else {
                    parked(d)
                }
            };
            (size(Dim::Overworld), size(Dim::Nether), size(Dim::End))
        };
        const DEFAULT: f64 = 59_999_968.0;
        let mut failures = Vec::new();
        {
            let mut resources = None;
            let mut game = open_game(&mut resources, &root, &home, &world);
            // A new world: every level at the default.
            if game.border.size() != DEFAULT {
                failures.push(format!("new world in place: {}", game.border.size()));
            }
            game.tick_world(true).unwrap();
            let got = sizes(&mut game);
            if got != (Some(DEFAULT), Some(DEFAULT), Some(DEFAULT)) {
                failures.push(format!("new world: {got:?}"));
            }
            assert!(game.run_command("worldborder set 50"));
            assert!(game.run_command("execute in minecraft:the_nether run worldborder set 100"));
            game.save_game().unwrap();
        }
        {
            let mut resources = None;
            let mut game = open_game(&mut resources, &root, &home, &world);
            // Before the first tick: the read, then the set.
            if game.border.size() != 50.0 {
                failures.push(format!("first read before a tick: {}", game.border.size()));
            }
            assert!(game.run_command("worldborder set 70"));
            if game.border.size() != 70.0 {
                failures.push(format!("first set before a tick: {}", game.border.size()));
            }
            game.tick_world(true).unwrap();
            let got = sizes(&mut game);
            if got != (Some(70.0), Some(100.0), Some(DEFAULT)) {
                failures.push(format!("after a tick: {got:?}"));
            }
            game.save_game().unwrap();
        }
        {
            let mut resources = None;
            let mut game = open_game(&mut resources, &root, &home, &world);
            let got = sizes(&mut game);
            if got != (Some(70.0), Some(100.0), Some(DEFAULT)) {
                failures.push(format!("after the save and another reopen: {got:?}"));
            }
        }
        assert!(failures.is_empty(), "reopened borders: {failures:#?}");
    }
    /// `ImposterProtoChunk(…, false)`: a new chunk's features read a full
    /// neighbour but their ordinary `setBlockState` writes do not change it,
    /// while `BulkSectionAccess` (ores) writes its sections. Expected values
    /// from the official 26.3 server on the constructed 1.20.4 world
    /// (`H077_OLD_WORLD`: full old chunks, their partial ring removed), where
    /// chunks 13..14 at z 0 generate next to the full chunk (12, -1).
    #[test]
    #[ignore = "needs SUNRISE_TEST_RESOURCES, a scratch SUNRISE_HOME and the constructed H077_OLD_WORLD"]
    fn features_write_a_full_neighbour_only_through_its_sections() {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let home = std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap()).join("h078");
        let world = home.join("saves/h078");
        let source = std::path::PathBuf::from(std::env::var_os("H077_OLD_WORLD").unwrap());
        fn copy(from: &std::path::Path, to: &std::path::Path) {
            std::fs::create_dir_all(to).unwrap();
            for entry in std::fs::read_dir(from).unwrap().flatten() {
                let target = to.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy(&entry.path(), &target);
                } else if entry.file_name() != "session.lock" {
                    std::fs::copy(entry.path(), target).unwrap();
                }
            }
        }
        copy(&source, &world);
        let mut resources = None;
        let mut game = open_game(&mut resources, &root, &home, &world);
        // The world's level.dat has cheats off: the open read it after open_game's setting.
        game.set_allow_commands(true);
        assert!(game.run_command("forceload add 208 0 239 15"));
        let mut loaded = false;
        // As the dedicated server's loop: a tick, then the chunk streaming.
        for _ in 0..2000 {
            game.tick_world(true).unwrap();
            game.server_update_world();
            std::thread::sleep(std::time::Duration::from_millis(10));
            if game.run_command_quietly("execute if loaded 215 64 8") {
                loaded = true;
                break;
            }
        }
        assert!(loaded, "the new chunk (13, 0) was not generated");
        game.save_game().unwrap();
        let mut failures = Vec::new();
        for (p, want) in [
            // A spruce tree and a mushroom of the new chunks: ordinary writes.
            ([207, 68, -10], "minecraft:air"),
            ([207, 69, -6], "minecraft:air"),
            ([202, 69, -5], "minecraft:air"),
            // A gold ore blob: section writes.
            ([207, -32, -12], "minecraft:deepslate_gold_ore"),
        ] {
            let got = game.block_name(p);
            if &*got != want {
                failures.push(format!("{p:?}: {got}, expected {want}"));
            }
        }
        assert!(
            failures.is_empty(),
            "the full chunk (12, -1): {failures:#?}"
        );
    }
    /// `BlendingData.pack` of an old chunk whose border columns a new chunk's
    /// generation calculated (`getOrUpdateBlendingData`), as the official 26.3
    /// server saves it: a constructed 1.20.4 world (full old chunks, their
    /// partial ring removed, `H077_OLD_WORLD`) where chunk (13, 0) is new.
    #[test]
    #[ignore = "needs SUNRISE_TEST_RESOURCES, a scratch SUNRISE_HOME and the constructed H077_OLD_WORLD"]
    fn an_old_chunk_saves_the_heights_its_new_neighbour_calculated() {
        use sunrise_core::nbt::Tag;
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let home = std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap()).join("h077");
        let world = home.join("saves/h077");
        let source = std::path::PathBuf::from(std::env::var_os("H077_OLD_WORLD").unwrap());
        fn copy(from: &std::path::Path, to: &std::path::Path) {
            std::fs::create_dir_all(to).unwrap();
            for entry in std::fs::read_dir(from).unwrap().flatten() {
                let target = to.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy(&entry.path(), &target);
                } else if entry.file_name() != "session.lock" {
                    std::fs::copy(entry.path(), target).unwrap();
                }
            }
        }
        copy(&source, &world);
        {
            let mut resources = None;
            let mut game = open_game(&mut resources, &root, &home, &world);
            // The world's level.dat has cheats off: the open read it after open_game's setting.
            game.set_allow_commands(true);
            assert!(game.run_command("forceload add 208 0 239 15"));
            let mut loaded = false;
            // As the dedicated server's loop: a tick, then the chunk streaming.
            for _ in 0..2000 {
                game.tick_world(true).unwrap();
                game.server_update_world();
                std::thread::sleep(std::time::Duration::from_millis(10));
                if game.run_command_quietly("execute if loaded 215 64 8") {
                    loaded = true;
                    break;
                }
            }
            assert!(loaded, "the new chunk (13, 0) was not generated");
            game.save_game().unwrap();
        }
        let save = sunrise_sim::save::SaveDir::new(
            sunrise_sim::save::dimension_dir(&world, "minecraft:overworld"),
            sunrise_level::chunk::LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        let blending = |x, z| {
            save.raw_chunk(x, z)
                .and_then(|r| r.tag.get("blending_data").cloned())
        };
        let heights = |tag: &Tag| match tag.get("heights") {
            Some(Tag::List(list)) => Some(
                list.iter()
                    .map(|v| match v {
                        Tag::Float(bits) => *bits,
                        other => panic!("height {other:?}"),
                    })
                    .collect::<Vec<u32>>(),
            ),
            _ => None,
        };
        let sections = |tag: &Tag| {
            (
                tag.get("min_section").cloned(),
                tag.get("max_section").cloned(),
            )
        };
        let mut failures = Vec::new();
        let mut want = vec![f32::MAX.to_bits(); 12];
        want.extend([70.0f32, 70.0, 69.0, 68.0].map(f32::to_bits));
        match blending(12, 0) {
            Some(tag) => {
                if heights(&tag) != Some(want.clone()) {
                    failures.push(format!("(12, 0) heights {:?}", heights(&tag)));
                }
                if sections(&tag) != (Some(Tag::Int(-4)), Some(Tag::Int(20))) {
                    failures.push(format!("(12, 0) sections {:?}", sections(&tag)));
                }
            }
            None => failures.push("(12, 0) has no blending_data".into()),
        }
        // No calculation reached these: the compound stays, without heights.
        for (x, z) in [(11, 0), (0, 0)] {
            match blending(x, z) {
                Some(tag) => {
                    if heights(&tag).is_some()
                        || sections(&tag) != (Some(Tag::Int(-4)), Some(Tag::Int(20)))
                    {
                        failures.push(format!("({x}, {z}) {tag:?}"));
                    }
                }
                None => failures.push(format!("({x}, {z}) has no blending_data")),
            }
        }
        assert!(failures.is_empty(), "saved blending data: {failures:#?}");
    }
    /// Added control (H077, after the baseline): the exact cache state the save
    /// reads. After the generation of (13, 0), the Overworld folder's lookup
    /// holds the calculated data of the edge chunk (12, 0), with the official
    /// server's heights; an old neighbour without new sides holds no height,
    /// and a far chunk no entry.
    #[test]
    #[ignore = "needs SUNRISE_TEST_RESOURCES, a scratch SUNRISE_HOME and the constructed H077_OLD_WORLD"]
    fn the_overworld_cache_holds_the_edge_chunks_calculated_heights() {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let home =
            std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap()).join("h077-cache");
        let world = home.join("saves/h077");
        let source = std::path::PathBuf::from(std::env::var_os("H077_OLD_WORLD").unwrap());
        fn copy(from: &std::path::Path, to: &std::path::Path) {
            std::fs::create_dir_all(to).unwrap();
            for entry in std::fs::read_dir(from).unwrap().flatten() {
                let target = to.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy(&entry.path(), &target);
                } else if entry.file_name() != "session.lock" {
                    std::fs::copy(entry.path(), target).unwrap();
                }
            }
        }
        copy(&source, &world);
        let mut resources = None;
        let mut game = open_game(&mut resources, &root, &home, &world);
        game.set_allow_commands(true);
        assert!(game.run_command("forceload add 208 0 239 15"));
        let mut loaded = false;
        for _ in 0..2000 {
            game.tick_world(true).unwrap();
            game.server_update_world();
            std::thread::sleep(std::time::Duration::from_millis(10));
            if game.run_command_quietly("execute if loaded 215 64 8") {
                loaded = true;
                break;
            }
        }
        assert!(loaded, "the new chunk (13, 0) was not generated");
        assert_eq!(game.dim, Dim::Overworld);
        let mut want = vec![f32::MAX.to_bits(); 12];
        want.extend([70.0f32, 70.0, 69.0, 68.0].map(f32::to_bits));
        let edge = game
            .server
            .save
            .0
            .calculated_blending(12, 0)
            .expect("(12, 0) was calculated");
        let (min, max, heights) = edge.pack();
        assert_eq!((min, max), (-4, 20));
        assert_eq!(heights.map(|h| h.map(f32::to_bits).to_vec()), Some(want));
        // (11, 0) borders no new chunk: the generation's calculation adds no height
        // to it (it may have an entry), so it packs without heights, as vanilla's.
        let inner = game.server.save.0.calculated_blending(11, 0);
        assert!(
            inner.as_ref().is_none_or(|data| data.pack().2.is_none()),
            "{inner:?}"
        );
        // (0, 0) is far from the new chunks: nothing calculated it.
        assert!(game.server.save.0.calculated_blending(0, 0).is_none());
    }
}

#[cfg(test)]
mod request_save_tests {
    use {sunrise_gameplay::game::server_side::levels::Levels, sunrise_gameplay_common::game::Dim};

    #[test]
    #[ignore = "daily"]
    fn requested_save_waits_without_consuming_the_queue_while_lifted() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.server.levels.0.request_save(Dim::Overworld);
            let pass = game.dropped.lift();
            game.run_requested_saves();
            assert_eq!(game.server.levels.0.requested_saves, [Dim::Overworld]);
            drop(pass);
            game.run_requested_saves();
            assert!(game.server.levels.0.requested_saves.is_empty());
        });
    }

    /// A level asked for twice is queued once, and clearing the levels drops the queue.
    #[test]
    fn a_level_asked_for_twice_is_saved_once() {
        let mut levels = Levels::default();
        levels.request_save(Dim::Nether);
        levels.request_save(Dim::Nether);
        levels.request_save(Dim::End);
        assert_eq!(levels.requested_saves, [Dim::Nether, Dim::End]);
        levels.clear();
        assert!(levels.requested_saves.is_empty());
    }
}
