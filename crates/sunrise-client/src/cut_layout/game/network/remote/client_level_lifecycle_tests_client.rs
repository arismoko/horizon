#[test]
#[ignore = "daily"]
fn client_level_notices_pair_across_replacement_reconfiguration_leave_and_quit() {
    use crate::game::{
        remote::tests::connect_play_test, server_side::native_entity_refs::tests::with_client_game,
    };
    use mod_api::__private::{HookPanicReport, take_hook_panics};
    use mod_api::{
        BlockState, ClientHost, ClientLevelLoadEvent, ClientLevelUnloadEvent, DatapackCandidate,
        DatapackField, DatapackFieldKind, DatapackReadError, DatapackRegistrySet,
        DatapackRegistrySync, DatapackValue, Hooks, LevelRef, ModEntry, Registrar,
        RegistrationContext,
    };
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use sunrise_client::{
        level_loading::Reason,
        net::{Event, PhaseChange, ingress},
    };
    use sunrise_client_gameplay::{
        game::client_side::{
            native_client_session::tests::logged_client,
            remote_client::session_state::RemoteDimensionEntry,
        },
        remote::chunks::Received,
    };
    use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
    use sunrise_level::chunk::Chunk;
    use sunrise_protocol::packets::{
        configuration::CustomPayload,
        play::{Clientbound, Login, SpawnInfo},
    };
    use sunrise_sim::datapack_net;

    if run_fresh_cases(
        "cut_layout::game::client_side::remote_client::tests::client_level_lifecycle_tests::client_level_notices_pair_across_replacement_reconfiguration_leave_and_quit",
        "SUNRISE_CLIENT_LEVEL_LIFECYCLE_CASE",
        &["reconfiguration"],
    ) {
        return;
    }

    struct Notice {
        kind: &'static str,
        dimension: String,
        level: LevelRef,
        block: Option<BlockState>,
        power: Option<i64>,
    }
    struct Log(Arc<Mutex<Vec<Notice>>>);
    impl Log {
        fn record(&self, kind: &'static str, host: &dyn ClientHost, dimension: &str) {
            let level = host
                .level(dimension)
                .expect("the event's level must resolve");
            let block = level.block_state(host, [3, 70, 5]);
            let mut power = None;
            let _ = level.datapack_read(host, "levels:spell", "addon:fire", &mut |entry| {
                if let Some(DatapackValue::Int(value)) = entry.get("power") {
                    power = Some(*value);
                }
            });
            self.0.lock().unwrap().push(Notice {
                kind,
                dimension: dimension.to_owned(),
                level,
                block,
                power,
            });
        }
    }
    #[mod_api::__subscriptions]
    impl Hooks for Log {
        fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
            registrar
                .declare_datapack_registry(
                    "spell",
                    DatapackRegistrySync::Required,
                    vec![DatapackField::required(
                        "power",
                        DatapackFieldKind::Int { min: 0, max: 100 },
                    )],
                )
                .unwrap();
        }
        fn client_level_load(
            &self,
            host: &mut dyn ClientHost,
            event: &mut ClientLevelLoadEvent<'_>,
        ) {
            self.record("load", host, event.dimension);
        }
        fn client_level_unload(
            &self,
            host: &mut dyn ClientHost,
            event: &mut ClientLevelUnloadEvent<'_>,
        ) {
            self.record("unload", host, event.dimension);
        }
    }
    #[derive(Default)]
    struct Panics {
        loads: AtomicUsize,
        unloads: AtomicUsize,
    }
    struct Broken(Arc<Panics>);
    #[mod_api::__subscriptions]
    impl Hooks for Broken {
        #[listener(priority = 100)]
        fn client_level_load(&self, _: &mut dyn ClientHost, _: &mut ClientLevelLoadEvent<'_>) {
            self.0.loads.fetch_add(1, Ordering::Relaxed);
            panic!("client Load listener");
        }

        #[listener(priority = 100)]
        fn client_level_unload(&self, _: &mut dyn ClientHost, _: &mut ClientLevelUnloadEvent<'_>) {
            self.0.unloads.fetch_add(1, Ordering::Relaxed);
            panic!("client Unload listener");
        }
    }

    let log = Arc::new(Mutex::new(Vec::new()));
    let panics = Arc::new(Panics::default());
    mod_platform::testing::with_mods(
        vec![
            ModEntry::new("broken-client-lifecycle", Box::new(Broken(panics.clone()))),
            ModEntry::new("levels", Box::new(Log(log.clone()))),
        ]
        .into(),
        || {
            with_client_game(|game| {
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
                let _server = connect_play_test(game, |_, _| {});
                game.wait_for_test_remote_event(|event| matches!(event, Event::Joined { .. }));
                game.poll_remote_events(true).unwrap();
                {
                    let remote = game.client.remote.as_mut().unwrap();
                    remote.logged_in = true;
                    remote.entity_id = 7;
                    remote.profile = 42;
                }
                let declarations = mod_api::__private::datapack_registry_declarations();
                let candidate = |power| {
                    DatapackCandidate::build(
                        declarations,
                        &[(
                            "data/addon/levels/spell/fire.json".into(),
                            vec![(
                                "fixture".into(),
                                format!(r#"{{"power":{power}}}"#).into_bytes(),
                            )],
                        )],
                        &mut mod_api::__private::NoVanillaValueDecoder,
                    )
                    .unwrap()
                };
                let encode = |set: &DatapackRegistrySet| {
                    set.snapshot(declarations, |_| true).encode().unwrap()
                };
                let receiver = |set: &DatapackRegistrySet| {
                    let mut receiver = datapack_net::Receiver::default();
                    for chunk in datapack_net::chunks(&encode(set)).unwrap() {
                        receiver.accept_chunk(&chunk).unwrap();
                    }
                    receiver
                };
                game.client.remote.as_mut().unwrap().datapacks =
                    receiver(&DatapackRegistrySet::from_candidate(candidate(17)));
                game.enter_remote_dimension(&login, RemoteDimensionEntry::Login, Reason::Other);
                let overworld = game.level(&login.dimension).unwrap();
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.enter_remote_dimension(&login, RemoteDimensionEntry::Respawn, Reason::Other);
                assert_eq!(game.level(&login.dimension), Some(overworld));
                let mut nether = login.clone();
                nether.dimension = "minecraft:the_nether".into();
                game.enter_remote_dimension(&nether, RemoteDimensionEntry::Respawn, Reason::Other);
                let nether = game.level(&nether.dimension).unwrap();
                assert_eq!(
                    log.lock().unwrap()[1].level,
                    nether,
                    "constructor handle survives installation"
                );
                assert_eq!(
                    game.level_block_state(overworld, [3, 70, 5]),
                    None,
                    "old handle retires after Unload"
                );
                assert_eq!(
                    overworld.visit_datapack_entries(game, "levels:spell", &mut |_| unreachable!()),
                    Err(DatapackReadError::UnknownRegistry),
                    "a synchronized registry does not revive a stale level handle"
                );
                let next_set = DatapackRegistrySet::from_candidate(candidate(29));
                let (tx, rx) = ingress::channel(1024, |_| 1).unwrap();
                tx.send(Event::Reconfiguring(
                    Box::new(PhaseChange::dummy_for_test()),
                ))
                .unwrap();
                // The next configuration has already accepted all its registry fragments
                // and queued the handoff before the game can unload the retiring level.
                tx.send(Event::Reconfigured {
                    registries: game.client.remote.as_ref().unwrap().registries.clone(),
                    datapacks: receiver(&next_set),
                })
                .unwrap();
                game.client.remote.as_mut().unwrap().backlog.extend(
                    rx.take_turn().into_iter().map(|receipt| {
                        let (event, credit) = receipt.into_parts();
                        Received::Event(event, credit)
                    }),
                );
                game.poll_remote_events(true).unwrap();
                let remote = game.client.remote.as_ref().unwrap();
                assert!(remote.reconfiguring.is_some());
                assert!(!remote.logged_in);
                game.apply_packet(Clientbound::Login(Login {
                    player_id: 8,
                    hardcore: false,
                    levels: vec![login.dimension.clone()],
                    max_players: 1,
                    chunk_radius: 2,
                    simulation_distance: 2,
                    reduced_debug_info: false,
                    show_death_screen: true,
                    limited_crafting: false,
                    spawn: login.clone(),
                    online_mode: false,
                    enforces_secure_chat: false,
                }))
                .unwrap();
                let remote = game.client.remote.as_ref().unwrap();
                assert!(remote.reconfiguring.is_none());
                assert!(remote.logged_in);
                assert_eq!(remote.entity_id, 8);
                let next = game.level(&login.dimension).unwrap();
                assert_eq!(
                    nether.visit_datapack_entries(game, "levels:spell", &mut |_| unreachable!()),
                    Err(DatapackReadError::UnknownRegistry),
                );
                assert_eq!(
                    nether.datapack_read(
                        game,
                        "levels:spell",
                        "addon:fire",
                        &mut |_| unreachable!()
                    ),
                    Err(DatapackReadError::UnknownRegistry),
                );
                let reloaded = DatapackRegistrySet::from_candidate(candidate(31));
                for data in datapack_net::chunks(&encode(&reloaded)).unwrap() {
                    game.apply_packet(Clientbound::CustomPayload(CustomPayload {
                        channel: datapack_net::CHANNEL.into(),
                        data,
                    }))
                    .unwrap();
                }
                let mut reloaded_power = None;
                next.datapack_read(game, "levels:spell", "addon:fire", &mut |entry| {
                    if let Some(DatapackValue::Int(value)) = entry.get("power") {
                        reloaded_power = Some(*value);
                    }
                })
                .unwrap();
                assert_eq!(reloaded_power, Some(31));
                tx.send(Event::Reconfiguring(
                    Box::new(PhaseChange::dummy_for_test()),
                ))
                .unwrap();
                let (event, credit) = rx.try_recv_owned().unwrap().into_parts();
                game.client
                    .remote
                    .as_mut()
                    .unwrap()
                    .backlog
                    .push_back(Received::Event(event, credit));
                game.poll_remote_events(true).unwrap();
                // Disconnecting from the reconfiguration screen must not unload twice.
                game.leave_server(None).unwrap();
                let _server = logged_client(game);
                game.enter_remote_dimension(&login, RemoteDimensionEntry::Login, Reason::Other);
                game.leave_server(None).unwrap();
                let _server = logged_client(game);
                game.enter_remote_dimension(&login, RemoteDimensionEntry::Login, Reason::Other);
                game.begin_closing_save(true).unwrap();
                assert!(game.poll_closing_save().unwrap());
                game.save_for_exit().unwrap();
                assert_eq!(panics.loads.load(Ordering::Relaxed), 1);
                assert_eq!(panics.unloads.load(Ordering::Relaxed), 1);
                assert_eq!(
                    take_hook_panics(),
                    [
                        HookPanicReport {
                            mod_id: "broken-client-lifecycle",
                            hook: "client_level_load"
                        },
                        HookPanicReport {
                            mod_id: "broken-client-lifecycle",
                            hook: "client_level_unload"
                        },
                    ]
                );
            });
        },
    );
    let log = log.lock().unwrap();
    assert_eq!(
        log.iter()
            .map(|notice| (notice.kind, notice.dimension.as_str()))
            .collect::<Vec<_>>(),
        [
            ("load", "minecraft:overworld"),
            ("load", "minecraft:the_nether"),
            ("unload", "minecraft:overworld"),
            ("unload", "minecraft:the_nether"),
            ("load", "minecraft:overworld"),
            ("unload", "minecraft:overworld"),
            ("load", "minecraft:overworld"),
            ("unload", "minecraft:overworld"),
            ("load", "minecraft:overworld"),
            ("unload", "minecraft:overworld"),
        ]
    );
    assert_eq!(log[0].level, log[2].level);
    assert_eq!(log[1].level, log[3].level);
    assert_ne!(log[0].level.epoch(), log[1].level.epoch());
    assert!(log[0].block.is_none() && log[1].block.is_none());
    assert_eq!(
        log.iter().map(|notice| notice.power).collect::<Vec<_>>(),
        [
            Some(17),
            Some(17),
            Some(17),
            Some(17),
            Some(29),
            Some(31),
            None,
            None,
            None,
            None,
        ],
        "retiring levels see their own configuration and the new level sees its handoff and reload"
    );
    assert!(
        log[2].block.is_some(),
        "the retiring Overworld still holds the old chunk"
    );
}

/// #12148: a proxy moves the player to its next server. The last server's
/// columns stay until the next Login, but Reconfigured already put in the
/// next server's biome table, shorter when only the last server had a
/// biome the game lacks: the columns hold an id past it ("the len is 67
/// but the index is 67"). Vanilla's clearClientLevel leaves no player, so
/// getSituationalMusic is MENU, and ServerReconfigScreen (no
/// LevelLoadingScreen) lets MusicManager.tick start it.
#[test]
#[ignore = "daily"]
fn a_reconfiguring_client_plays_menu_music_and_never_indexes_past_the_biomes() {
    use crate::game::load_resources;
    use crate::settings::Settings;
    use sunrise_client::music::Music;
    use sunrise_client_lib::audio::Audio;
    use sunrise_core::geometry::Vec3;
    use sunrise_level::chunk::Chunk;
    use {
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
        sunrise_gameplay_common::world_biomes,
    };
    let root = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES").expect("prepared resources required"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    let mut game = sunrise_client_gameplay::game::ClientGame::new(
        &loaded.resources,
        loaded.client_res.unwrap(),
        loaded.pack,
        loaded.client_pack.unwrap(),
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
        sunrise_gameplay::game::server_side::integrated::serve,
        sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
        crate::game::session_verifier(),
        crate::game::profile_resolver,
    )
    .unwrap();
    game.init_remote_for_replay().unwrap();
    // The last server's column at the origin: its extra biome came after
    // the built-in ones, one past the table the next server leaves.
    let stale = world_biomes!(game).list.len() as u16;
    let mut column = Chunk::empty(0, 0);
    // Quart 16 above the column's bottom at -64: block y 0.
    column.set_biome_cell(0, 16, 0, stale);
    game.world
        .chunks
        .insert((0, 0), std::sync::Arc::new(column));
    assert_eq!(game.biome_at([0.0; 3]), "minecraft:plains");
    assert_eq!(
        game.biome_name_at(Vec3::new(0.5, 0.5, 0.5)).as_deref(),
        Some("minecraft:plains")
    );

    // StartConfiguration after a Login: no player, ServerReconfigScreen.
    let ticks = game.ticks;
    let r = game.client.remote.as_mut().unwrap();
    r.logged_in = true;
    r.reconfiguring = Some(ticks);
    game.player = None;
    assert_eq!(game.situational_music().0, Some(Music::menu()));
    // A new manager waits its first 100 ticks, then starts the menu track.
    let menu = (0..=100).any(|_| {
        game.tick_music();
        game.client.music.current(&game.client.audio.0.engine) == Some("minecraft:music.menu")
    });
    assert!(menu, "the menu music starts under ServerReconfigScreen");

    // With a level again the camera probe reads that column: plains' music.
    game.client.remote.as_mut().unwrap().reconfiguring = None;
    let (music, _) = game.situational_music();
    assert!(music.is_some_and(|m| m != Music::menu()));
}

/// Vanilla ClientPacketListener.handleConfigurationStart -> Minecraft.clearClientLevel ->
/// updateLevelInEngines(null): entering configuration stops sound immediately, so a flat mod
/// audio stream is Stopped at configuration entry without waiting for a new login.
#[test]
#[ignore = "daily"]
fn a_reconfiguring_client_stops_flat_audio_streams_before_configuration() {
    use mod_api::{AudioStream, AudioStreamState, SoundSource, StreamSpec};
    use sunrise_client::net::Event;
    use sunrise_protocol::packets::{Empty, play::Clientbound};
    use {
        crate::game::remote::tests::connect_play_test, crate::game::remote::tests::spawn_menu_test,
        crate::game::server_side::native_entity_refs::tests::with_client_game,
    };

    struct Silent;
    impl AudioStream for Silent {
        fn read(&mut self, _pcm: &mut [i16]) -> usize {
            1
        }
    }

    with_client_game(|game| {
        let (_reader, mut writer) = connect_play_test(game, |_, _| {});
        spawn_menu_test(game);
        game.client.audio.0.backend_live_for_bench();
        let stream = game
            .client
            .audio
            .0
            .open_stream(
                "test_mod",
                &StreamSpec::new(SoundSource::Music, 1, 48_000),
                None,
                Box::new(Silent),
            )
            .unwrap();
        assert_eq!(stream.state(), AudioStreamState::Playing);

        writer
            .send(&Clientbound::StartConfiguration(Empty).encode())
            .unwrap();
        game.wait_for_test_remote_event(|event| matches!(event, Event::Reconfiguring(_)));
        game.poll_remote_events(true).unwrap();

        assert!(game.client.remote.as_ref().unwrap().reconfiguring.is_some());
        assert_eq!(stream.state(), AudioStreamState::Stopped);
    });
}

#[test]
#[ignore = "daily"]
fn client_level_transitions_stop_streams_opened_during_unload() {
    use mod_api::{
        AudioStream, AudioStreamRef, AudioStreamState, ClientHost, ClientLevelUnloadEvent,
        ClientPlayerLoggingOutEvent, Hooks, ModEntry, SoundSource, StreamSpec,
    };
    use std::sync::{Arc, Mutex};
    use sunrise_client::level_loading::Reason;
    use sunrise_protocol::packets::play::SpawnInfo;
    use {
        crate::game::server_side::native_entity_refs::tests::with_client_game,
        sunrise_client_gameplay::game::client_side::integrated_client::Integrated,
        sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client,
        sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry,
    };

    struct Silent;
    impl AudioStream for Silent {
        fn read(&mut self, _pcm: &mut [i16]) -> usize {
            1
        }
    }

    struct Listener(Arc<Mutex<Vec<AudioStreamRef>>>);
    impl Listener {
        fn open(&self, host: &dyn ClientHost) {
            let stream = host
                .open_audio_stream(
                    &StreamSpec::new(SoundSource::Music, 1, 48_000),
                    Box::new(Silent),
                )
                .unwrap();
            assert_eq!(stream.state(), AudioStreamState::Playing);
            self.0.lock().unwrap().push(stream);
        }
    }

    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn client_level_unload(
            &self,
            host: &mut dyn ClientHost,
            _: &mut ClientLevelUnloadEvent<'_>,
        ) {
            self.open(host);
        }

        fn client_player_logging_out(
            &self,
            host: &mut dyn ClientHost,
            _: &mut ClientPlayerLoggingOutEvent,
        ) {
            self.open(host);
        }
    }

    let streams = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new(
        "unload_audio",
        Box::new(Listener(streams.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            game.client.audio.0.backend_live_for_bench();
            let spawn = SpawnInfo {
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
            for (replace, integrated) in [(true, false), (false, false), (false, true)] {
                let _server = logged_client(game);
                game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
                let existing = game
                    .client
                    .audio
                    .0
                    .open_stream(
                        "test_mod",
                        &StreamSpec::new(SoundSource::Music, 1, 48_000),
                        None,
                        Box::new(Silent),
                    )
                    .unwrap();
                game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Respawn, Reason::Other);
                assert!(
                    streams.lock().unwrap().is_empty(),
                    "same-level respawn does not unload"
                );
                assert_eq!(existing.state(), AudioStreamState::Playing);
                if replace {
                    let mut other = spawn.clone();
                    other.dimension = "minecraft:the_nether".into();
                    game.enter_remote_dimension(
                        &other,
                        RemoteDimensionEntry::Respawn,
                        Reason::Other,
                    );
                } else {
                    if integrated {
                        game.client.integrated = Some(Integrated::stopped_for_test());
                    }
                    game.leave_server(None).unwrap();
                }
                {
                    let mut heard = streams.lock().unwrap();
                    assert_eq!(heard.len(), if replace { 1 } else { 2 });
                    for stream in heard.drain(..) {
                        assert_eq!(stream.state(), AudioStreamState::Stopped);
                    }
                }
                assert_eq!(existing.state(), AudioStreamState::Stopped);
                if replace {
                    game.leave_server(None).unwrap();
                    streams.lock().unwrap().clear();
                }
            }
        });
    });
}
