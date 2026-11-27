#[cfg(feature = "client")]
#[cfg(test)]
pub(crate) mod tests {
    use sunrise_client_gameplay::game::client_side::viewed_worlds::routing::RouteError;
    use {
        sunrise_client_gameplay::game::client_side::viewed_worlds::sounds::ViewedPlay,
        sunrise_client_gameplay::game::client_side::viewed_worlds::sounds::ViewedSound,
    };

    use mod_api::SoundSource;
    use std::sync::{Arc, Mutex};
    use sunrise_protocol::{
        buf::Reader,
        chunk::{ChunkData, LightData, Paletted, Section, Strategy},
        connection::Pipe,
        packets::play::{self, Clientbound as C},
        viewed_levels,
    };
    use sunrise_world::registry::BlockStateId;

    pub(super) const STONE: BlockStateId = BlockStateId(1);
    pub(super) const AIR: BlockStateId = BlockStateId(0);

    pub(crate) fn spawn(dimension: &str) -> play::SpawnInfo {
        play::SpawnInfo {
            dimension_type: 0,
            dimension: dimension.into(),
            seed: 1,
            game_type: 0,
            previous_game_type: None,
            debug: false,
            flat: false,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        }
    }

    /// A client of a server whose Login names four levels, the player in the Overworld.
    pub(crate) fn client(
        resources: &mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
    ) -> sunrise_client_gameplay::game::ClientGame<'_> {
        client_with_pipe(resources).0
    }

    /// As `client`, retaining the server half so the executor stays connected for negotiation.
    pub(crate) fn client_with_pipe(
        resources: &mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
    ) -> (sunrise_client_gameplay::game::ClientGame<'_>, Pipe) {
        use {
            crate::game::load_resources, crate::settings::Settings,
            sunrise_client_lib::audio::Audio, sunrise_gameplay_common::game::Options,
            sunrise_gameplay_common::game::Progress, sunrise_gameplay_common::game::execution_role,
        };
        let root = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES")
                .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
        );
        let settings = Settings::default();
        let mut gui = None;
        let mut loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            execution_role::ExecutionRole::Client,
            &Progress::new(),
            &mut |ui, _| gui = Some(ui),
        )
        .unwrap();
        if let Some(rest) = loaded.rest_loader.take() {
            rest.finish().unwrap();
        }
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = execution_role::ExecutionRole::Client;
        let res = resources.insert(loaded.resources);
        let mut game = sunrise_client_gameplay::game::ClientGame::new(
            res,
            loaded.client_res.unwrap(),
            loaded.pack,
            loaded.client_pack.unwrap(),
            gui.or(loaded.gui).unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root,
            )),
            None,
            sunrise_gameplay::game::server_side::integrated::serve,
            sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
            crate::game::session_verifier(),
            crate::game::profile_resolver,
        )
        .unwrap();
        let (pipe, server) = Pipe::pair();
        game.connect_local(pipe, "Steve").unwrap();
        game.handle_packet(C::Login(play::Login {
            player_id: 7,
            hardcore: false,
            levels: [
                "minecraft:overworld",
                "minecraft:the_nether",
                "minecraft:the_end",
                "test:void",
            ]
            .map(String::from)
            .to_vec(),
            max_players: 20,
            chunk_radius: 8,
            simulation_distance: 8,
            reduced_debug_info: false,
            show_death_screen: true,
            limited_crafting: false,
            spawn: spawn("minecraft:overworld"),
            online_mode: false,
            enforces_secure_chat: false,
        }))
        .unwrap();
        // The dimension types the server synced: 0 the Overworld's (what `spawn` names), 1 the
        // Nether's, of another shape.
        let mut registries = sunrise_protocol::registry::Dynamic::default();
        registries.registries.insert(
            "minecraft:dimension_type".into(),
            ["minecraft:overworld", "minecraft:the_nether"]
                .map(|name| (name.to_owned(), None))
                .to_vec(),
        );
        game.client.remote.as_mut().unwrap().registries = Arc::new(registries);
        EPOCHS.with(|epochs| *epochs.borrow_mut() = Default::default());
        (game, server)
    }

    thread_local! {
        /// This test's connection as a server keeps it: the next epoch, and the one each slot was
        /// last bound with, so that `run` and `close` follow `open` and `swap`.
        static EPOCHS: std::cell::RefCell<(i32, std::collections::HashMap<i32, i32>)> =
            Default::default();
    }

    /// Binds `slot` with the next epoch, as a server's `open` or `swap` does.
    pub(crate) fn bind(slot: i32) -> viewed_levels::Level {
        EPOCHS.with(|epochs| {
            let (next, slots) = &mut *epochs.borrow_mut();
            let epoch = *next;
            *next += 1;
            slots.insert(slot, epoch);
            viewed_levels::Level::Bound { slot, epoch }
        })
    }

    /// `slot` with the epoch it was last bound with.
    pub(crate) fn bound_slot(slot: i32) -> viewed_levels::Level {
        let epoch = EPOCHS.with(|epochs| epochs.borrow().1.get(&slot).copied().unwrap_or(0));
        viewed_levels::Level::Bound { slot, epoch }
    }

    /// A column of stone at `x, z`, unlit.
    pub(crate) fn column(x: i32, z: i32) -> C {
        layered(x, z, 24)
    }

    /// A column at `x, z` whose lowest `stone` of its 24 sections are stone and the rest air,
    /// unlit.
    pub(crate) fn layered(x: i32, z: i32, stone: usize) -> C {
        let section = |state: BlockStateId| Section {
            non_empty: if state == AIR { 0 } else { 4096 },

            fluids: 0,
            states: Paletted::from_values(&[state.0 as i32; 4096], Strategy::Blocks, 16),
            biomes: Paletted::from_values(&[1; 64], Strategy::Biomes, 2),
        };
        let sections: Vec<_> = (0..24)
            .map(|i| section(if i < stone { STONE } else { AIR }))
            .collect();
        C::LevelChunkWithLight(play::LevelChunkWithLight {
            x,
            z,
            chunk: ChunkData {
                heightmaps: Vec::new(),
                buffer: ChunkData::from_sections(&sections),
                block_entities: Vec::new(),
            },
            light: LightData {
                sky_mask: Vec::new(),
                block_mask: Vec::new(),
                empty_sky_mask: Vec::new(),
                empty_block_mask: Vec::new(),
                sky: Vec::new(),
                block: Vec::new(),
            },
        })
    }

    /// The custom payload the client's net thread hands the game for a whole packet body.
    pub(crate) fn payload(body: &[u8]) -> C {
        let mut r = Reader::new(body);
        let id = r.varint().unwrap();
        C::decode(id, r.rest()).unwrap()
    }

    /// An `open` of the level `dimension` in `slot`, with the next epoch and a cap of 2048
    /// columns.
    pub(crate) fn open(slot: i32, dimension: &str) -> C {
        payload(&viewed_levels::open(bind(slot), 2048, &spawn(dimension)))
    }

    /// A `run` of `slot` at the epoch it was last bound with.
    pub(crate) fn run(slot: i32, packets: &[C]) -> C {
        run_at(bound_slot(slot), packets)
    }

    /// A `run` of `level`, its slot and epoch as given.
    pub(crate) fn run_at(level: viewed_levels::Level, packets: &[C]) -> C {
        let mut run = viewed_levels::Run::new(level, &packets[0].encode()).unwrap();
        for packet in &packets[1..] {
            run.push(&packet.encode()).unwrap();
        }
        payload(&run.finish())
    }

    /// A `close` of `slot` at the epoch it was last bound with.
    pub(crate) fn close(slot: i32) -> C {
        payload(&viewed_levels::close(bound_slot(slot)))
    }

    /// The server's `swap` after a travel's Respawn: `slot` with the next epoch.
    pub(crate) fn swap(slot: i32) -> C {
        payload(&viewed_levels::swap(bind(slot)))
    }

    pub(crate) fn data(packet: &C) -> &[u8] {
        let C::CustomPayload(p) = packet else {
            panic!("not a payload")
        };
        &p.data
    }

    pub(super) fn viewed<'a>(
        game: &'a sunrise_client_gameplay::game::ClientGame<'_>,
        level: i32,
    ) -> Option<&'a sunrise_level::world::World> {
        Some(&game.client.remote.as_ref()?.viewed.get(level)?.world)
    }

    /// A wrapped `BlockUpdate` changes the viewed level's block, and the player's level keeps
    /// its own at the same position.
    #[test]
    #[ignore = "daily"]
    fn a_wrapped_block_update_changes_the_viewed_world_not_the_players() {
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(column(0, 0)).unwrap();
        game.handle_packet(open(1, "minecraft:the_nether")).unwrap();
        game.handle_packet(run(1, &[column(0, 0)])).unwrap();
        let block = C::BlockUpdate(play::BlockUpdate {
            pos: [3, 70, 5],
            state: AIR.0 as i32,
        });
        game.handle_packet(run(1, &[block])).unwrap();
        assert_eq!(viewed(&game, 1).unwrap().block(3, 70, 5), AIR);
        assert_eq!(viewed(&game, 1).unwrap().block(3, 71, 5), STONE);
        assert_eq!(game.world.block(3, 70, 5), STONE);
    }

    /// An `open` makes a world for the level, a `run` fills it, and a `close` drops it and
    /// releases the slot: a late `run` of that binding is dropped, and makes no world.
    #[test]
    #[ignore = "daily"]
    fn open_makes_a_world_and_close_drops_it() {
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(open(0, "minecraft:the_end")).unwrap();
        let world = game.client.remote.as_ref().unwrap().viewed.get(0).unwrap();
        assert_eq!(world.level, "minecraft:the_end");
        assert_eq!(world.world.chunks.len(), 0);
        game.handle_packet(run(0, &[column(4, -2), column(5, -2)]))
            .unwrap();
        assert_eq!(viewed(&game, 0).unwrap().chunks.len(), 2);
        let forget = C::ForgetLevelChunk(play::ForgetLevelChunk { pos: [4, -2] });
        game.handle_packet(run(0, &[forget])).unwrap();
        assert_eq!(viewed(&game, 0).unwrap().chunks.len(), 1);
        game.handle_packet(close(0)).unwrap();
        assert!(viewed(&game, 0).is_none());
        assert_eq!(
            game.route_viewed_levels(data(&run(0, &[column(4, -2)]))),
            Ok(())
        );
        assert!(viewed(&game, 0).is_none());
    }

    /// The client views two levels besides its own, in slots 0 and 1: a slot past them closes the
    /// connection, and an `open` of a bound slot starts it afresh.
    #[test]
    #[ignore = "daily"]
    fn an_open_past_the_clients_cap_closes_the_connection() {
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(open(0, "minecraft:the_nether")).unwrap();
        game.handle_packet(open(1, "minecraft:the_end")).unwrap();
        assert_eq!(
            game.route_viewed_levels(data(&open(2, "test:void"))),
            Err(RouteError::OverCap { level: 2, cap: 2 })
        );
        game.handle_packet(run(1, &[column(0, 0)])).unwrap();
        game.handle_packet(open(1, "minecraft:the_end")).unwrap();
        assert_eq!(viewed(&game, 1).unwrap().chunks.len(), 0, "afresh");
        assert!(game.client.remote.is_some());
        game.handle_packet(open(2, "test:void")).unwrap();
        assert!(game.client.remote.is_none());
    }

    /// Each `open` refusal has a fixed reason that shows none of the server's text: a dimension
    /// that is no level id (the codec's), a dimension type the server did not sync (no Overworld
    /// fallback), a level viewed already in another slot, the player's own level, a cap past the
    /// client's ceiling, an epoch not newer than the last and a negative slot (the codec's).
    #[test]
    #[ignore = "daily"]
    fn each_open_refusal_has_a_fixed_reason() {
        use viewed_levels::Level;
        let mut resources = None;
        let mut game = client(&mut resources);
        let refused = game
            .route_viewed_levels(data(&open(1, "Evil: Click Here!")))
            .unwrap_err();
        assert_eq!(
            refused,
            RouteError::Codec(viewed_levels::ViewedLevelsError::BadOpen)
        );
        assert_eq!(
            refused.to_string(),
            "sunrise:levels: an open whose level does not read"
        );
        assert!(!refused.to_string().contains("Evil"));
        assert!(viewed(&game, 1).is_none());
        let unsynced = play::SpawnInfo {
            dimension_type: 7,
            ..spawn("evil:elsewhere")
        };
        let refused = game
            .route_viewed_levels(data(&payload(&viewed_levels::open(
                bind(1),
                2048,
                &unsynced,
            ))))
            .unwrap_err();
        assert_eq!(refused, RouteError::UnsyncedType);
        assert!(!refused.to_string().contains("evil"));
        game.handle_packet(open(0, "test:void")).unwrap();
        assert_eq!(
            game.route_viewed_levels(data(&open(1, "test:void"))),
            Err(RouteError::ViewedTwice(1))
        );
        assert_eq!(
            game.route_viewed_levels(data(&open(1, "minecraft:overworld"))),
            Err(RouteError::OwnLevel(1))
        );
        let past = viewed_levels::open(bind(1), 4490, &spawn("minecraft:the_end"));
        assert_eq!(
            game.route_viewed_levels(data(&payload(&past))),
            Err(RouteError::CapPastCeiling(4490))
        );
        let old = Level::Bound { slot: 1, epoch: 0 };
        let old = viewed_levels::open(old, 2048, &spawn("minecraft:the_end"));
        assert_eq!(
            game.route_viewed_levels(data(&payload(&old))),
            Err(RouteError::OldEpoch(0))
        );
        let minus_one = Level::Bound { slot: -1, epoch: 9 };
        let negative = viewed_levels::open(minus_one, 2048, &spawn("minecraft:the_end"));
        assert_eq!(
            game.route_viewed_levels(data(&payload(&negative))),
            Err(RouteError::Codec(
                viewed_levels::ViewedLevelsError::Negative
            ))
        );
    }

    /// A level the Login did not name (a runtime level opened since) is viewed in the slot its
    /// `open` binds: its `run`s fill it.
    #[test]
    #[ignore = "daily"]
    fn a_level_not_named_at_login_is_viewed_in_its_slot() {
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(open(1, "test:instance/1")).unwrap();
        game.handle_packet(run(1, &[column(2, 3)])).unwrap();
        let world = game.client.remote.as_ref().unwrap().viewed.get(1).unwrap();
        assert_eq!(world.level, "test:instance/1");
        assert_eq!(viewed(&game, 1).unwrap().chunks.len(), 1);
    }

    /// A slot closed and bound again to another level: a late chunk or entity of the old binding
    /// is dropped by its epoch, never reaching the new world, and the new binding's packets go to
    /// it. A `run` at an epoch no `open` gave closes the connection.
    #[test]
    #[ignore = "daily"]
    fn a_late_packet_of_a_slot_bound_again_is_dropped() {
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(open(0, "minecraft:the_nether")).unwrap();
        let old = bound_slot(0);
        game.handle_packet(close(0)).unwrap();
        game.handle_packet(open(0, "minecraft:the_end")).unwrap();
        let pig = C::AddEntity(play::AddEntity {
            id: 40,
            uuid: 40,
            kind: 0,
            position: [0.5, 70.0, 0.5],
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 0,
            head_yaw: 0,
            data: 0,
        });
        game.handle_packet(run_at(old, &[column(9, 9), pig]))
            .unwrap();
        let end = game.client.remote.as_ref().unwrap().viewed.get(0).unwrap();
        assert_eq!(end.level, "minecraft:the_end");
        assert!(end.world.chunks.is_empty(), "the Nether's late column");
        assert!(end.entities.get(40).is_none(), "the Nether's late entity");
        game.handle_packet(run(0, &[column(1, 1)])).unwrap();
        assert!(viewed(&game, 0).unwrap().chunks.contains_key(&(1, 1)));
        let ahead = viewed_levels::Level::Bound { slot: 0, epoch: 99 };
        assert_eq!(
            game.route_viewed_levels(data(&run_at(ahead, &[column(2, 2)]))),
            Err(RouteError::UnknownEpoch {
                level: 0,
                epoch: 99
            })
        );
    }

    /// An `open`'s column cap is the world's: a new column past it closes the connection.
    #[test]
    #[ignore = "daily"]
    fn an_opens_column_cap_bounds_its_world() {
        let mut resources = None;
        let mut game = client(&mut resources);
        let small = viewed_levels::open(bind(0), 2, &spawn("minecraft:the_end"));
        game.handle_packet(payload(&small)).unwrap();
        game.handle_packet(run(0, &[column(0, 0), column(1, 0)]))
            .unwrap();
        assert_eq!(
            game.route_viewed_levels(data(&run(0, &[column(2, 0)]))),
            Err(RouteError::TooManyColumns(2))
        );
    }

    /// A viewed level's chunks, time and weather leave the player's level as it was.
    #[test]
    #[ignore = "daily"]
    fn the_players_world_stays_as_it_was() {
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(column(0, 0)).unwrap();
        let (game_time, day_time) = (game.world.game_time, game.world.day_time);
        game.handle_packet(open(1, "minecraft:the_nether")).unwrap();
        let time = C::SetTime(play::SetTime {
            game_time: 24_000,
            clocks: Vec::new(),
        });
        let rain = C::GameEvent(play::GameEvent {
            event: 1,
            param: 0.0,
        });
        game.handle_packet(run(1, &[column(1, 0), time, rain]))
            .unwrap();
        let nether = viewed(&game, 1).unwrap();
        assert_eq!(nether.game_time, 24_000);
        assert!(nether.weather.raining);
        assert!(nether.chunks.contains_key(&(1, 0)));
        assert_eq!(game.world.chunks.len(), 1);
        assert!(!game.world.chunks.contains_key(&(1, 0)));
        assert_eq!(game.world.game_time, game_time);
        assert_eq!(game.world.day_time, day_time);
        assert!(!game.world.weather.raining);
        assert_eq!(
            game.client.remote.as_ref().unwrap().level,
            "minecraft:overworld"
        );
    }

    /// `far` (revision 5): a column of the player's own level 300 chunks from the cache centre
    /// is ignored before it is kept and admitted after; it stays through a move of the centre and
    /// takes a block update; released, it goes. More than `MAX_COLUMNS` kept close the connection.
    #[test]
    #[ignore = "daily"]
    fn a_far_column_is_kept_past_the_view_until_released() {
        let mut resources = None;
        let mut game = client(&mut resources);
        let far = |kept: &[(i32, i32)], released: &[(i32, i32)]| {
            payload(&viewed_levels::far(kept, released))
        };
        game.handle_packet(column(300, 0)).unwrap();
        assert!(!game.world.chunks.contains_key(&(300, 0)), "past the view");
        game.handle_packet(far(&[(300, 0)], &[])).unwrap();
        game.handle_packet(column(300, 0)).unwrap();
        assert!(game.world.chunks.contains_key(&(300, 0)));
        let centre = C::SetChunkCacheCenter(play::SetChunkCacheCenter { x: 20, z: 0 });
        game.handle_packet(centre).unwrap();
        assert!(
            game.world.chunks.contains_key(&(300, 0)),
            "the centre moved"
        );
        let block = C::BlockUpdate(play::BlockUpdate {
            pos: [4803, 70, 5],
            state: AIR.0 as i32,
        });
        game.handle_packet(block).unwrap();
        assert_eq!(game.world.block(4803, 70, 5), AIR);
        assert_eq!(game.world.block(4803, 71, 5), STONE);
        game.handle_packet(far(&[], &[(300, 0)])).unwrap();
        assert!(!game.world.chunks.contains_key(&(300, 0)), "released");

        let many: Vec<_> = (0..viewed_levels::MAX_COLUMNS as i32)
            .map(|x| (1000 + x, 0))
            .collect();
        assert_eq!(game.route_viewed_levels(data(&far(&many, &[]))), Ok(()));
        assert_eq!(
            game.route_viewed_levels(data(&far(&[(-1000, 0)], &[]))),
            Err(RouteError::TooManyFar)
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_dry_view_stays_dry_after_shared_weather_changes_and_promotion() {
        use std::sync::mpsc::{Receiver, channel};
        use sunrise_client_gameplay::game::{
            ClientGame,
            client_side::{
                native_client_session::tests::logged_client,
                native_entity_refs::tests::with_client_game,
                remote_client::session_state::RemoteDimensionEntry,
            },
        };
        use sunrise_client_render::level_loading::Reason;
        use sunrise_gameplay::game::native_guest_turn_tests::with_guests;
        use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
        use sunrise_level::chunk::{ExtraLevel, ExtraLevels, LevelShape};
        use sunrise_sim::lan::{
            Out, PlayerSender,
            hub::game_event::{RAIN_LEVEL, START_RAINING, STOP_RAINING, THUNDER_LEVEL},
            viewed_levels::areas::{ChunkArea, ChunkAreaMode},
        };

        const OVERWORLD: &str = "minecraft:overworld";
        const VIEWED: &str = "test:viewed_overworld";
        const CLEARED: &str = "test:cleared_overworld";
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::client_side::viewed_worlds::tests::a_dry_view_stays_dry_after_shared_weather_changes_and_promotion",
            || {
                with_guests(|server| {
                    let mut host = server.server.lan.take().unwrap();
                    let hub = &mut host.hub;
                    hub.config.levels.extra = ExtraLevels::new(vec![
                        ExtraLevel {
                            id: VIEWED,
                            dimension_type: OVERWORLD,
                            shape: LevelShape::OVERWORLD,
                        },
                        ExtraLevel {
                            id: CLEARED,
                            dimension_type: OVERWORLD,
                            shape: LevelShape::OVERWORLD,
                        },
                    ]);
                    let area = |level: &str| ChunkArea {
                        level: level.into(),
                        center: (0, 0),
                        radius: 0,
                        mode: ChunkAreaMode::Loaded,
                    };
                    let (view_tx, view_rx) = channel();
                    let (plain_tx, plain_rx) = channel();
                    hub.guests.update(0, |guest| {
                        guest.dimension = OVERWORLD.into();
                        guest.sender = PlayerSender::new(view_tx, Arc::default());
                        guest.sender.viewed_levels = viewed_levels::Answer {
                            revision: viewed_levels::REVISION_4,
                            max_levels: 2,
                        };
                    });
                    hub.guests.update(1, |guest| {
                        guest.dimension = VIEWED.into();
                        guest.sender = PlayerSender::new(plain_tx, Arc::default());
                    });
                    hub.set_dimension(VIEWED);
                    hub.set_viewed_areas(12, "test:portal", vec![area(VIEWED)])
                        .unwrap();

                    let packets = |rx: &Receiver<Out>| {
                        rx.try_iter()
                            .flat_map(|out| match out {
                                Out::Packet(body) => vec![body],
                                Out::Packets(bodies) => bodies,
                                Out::Disconnect(_) => panic!("unexpected disconnect"),
                            })
                            .map(|body| payload(&body))
                            .collect::<Vec<_>>()
                    };
                    let plain_weather = || {
                        packets(&plain_rx)
                            .into_iter()
                            .filter_map(|packet| match packet {
                                C::GameEvent(p) => Some((p.event, p.param)),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                    };
                    let deliver_viewer_packets = |game: &mut ClientGame<'_>| {
                        for packet in packets(&view_rx) {
                            if matches!(packet, C::Respawn(_) | C::GameEvent(_))
                                || matches!(&packet, C::CustomPayload(p) if p.channel == viewed_levels::CHANNEL)
                            {
                                game.handle_packet(packet).unwrap();
                            }
                        }
                    };
                    with_client_game(|game| {
                        let _transport = logged_client(game);
                        game.client.remote.as_mut().unwrap().registries =
                            Arc::new(hub.sync_data().dynamic.clone());
                        game.enter_remote_dimension(
                            &play::SpawnInfo {
                                dimension_type: hub
                                    .sync_data()
                                    .id("minecraft:dimension_type", OVERWORLD)
                                    .unwrap(),
                                ..spawn(OVERWORLD)
                            },
                            RemoteDimensionEntry::Login,
                            Reason::Other,
                        );
                        let ticks: [(_, &[_]); 4] = [
                            ((true, 0.0, 0.0), &[]),
                            // Local transitions send both strengths after START/STOP, even
                            // when thunder hasn't changed.
                            (
                                (true, 0.25, 0.5),
                                &[
                                    (RAIN_LEVEL, 0.25),
                                    (THUNDER_LEVEL, 0.5),
                                    (START_RAINING, 0.0),
                                    (RAIN_LEVEL, 0.25),
                                    (THUNDER_LEVEL, 0.5),
                                ],
                            ),
                            (
                                (true, 0.0, 0.5),
                                &[
                                    (RAIN_LEVEL, 0.0),
                                    (STOP_RAINING, 0.0),
                                    (RAIN_LEVEL, 0.0),
                                    (THUNDER_LEVEL, 0.5),
                                ],
                            ),
                            // The shared flag flipped without a local transition.
                            ((false, 0.0, 0.5), &[]),
                        ];
                        for ((shared_rain, rain, thunder), expected) in ticks {
                            server.world.weather.raining = shared_rain;
                            server.world.weather.set_levels(rain, thunder);
                            let _ = hub.tick_level(&mut server.server_ctx(), None);
                            deliver_viewer_packets(game);
                            let view = &viewed(game, 0).unwrap().weather;
                            assert_eq!((view.rain_level.1, view.thunder_level.1), (rain, thunder));
                            assert_eq!(plain_weather().as_slice(), expected);
                        }

                        let ctx = server.server_ctx();
                        let change =
                            hub.begin_level_change(0, [0.5, 64.0, 0.5], 0.0, None, 0, &ctx);
                        hub.finish_level_change(0, change, false, &ctx);
                        deliver_viewer_packets(game);
                        assert_eq!(game.client.remote.as_ref().unwrap().level, VIEWED);
                        assert_eq!(game.world.weather.rain_level, (0.0, 0.0));
                        // A plain Respawn would have dropped the viewed level's raw thunder.
                        assert_eq!(game.world.weather.thunder_level, (0.5, 0.5));

                        // Only the viewer is told it is wet; the ordinary send cache never saw it.
                        hub.set_dimension(CLEARED);
                        assert!(hub.guests.iter().all(|guest| guest.dimension != CLEARED));
                        hub.set_viewed_areas(12, "test:portal", vec![area(CLEARED)])
                            .unwrap();
                        server.world.weather.raining = true;
                        let level = server.level(server.dim.id()).unwrap();
                        assert!(level.set_weather_levels(server, 0.75, 0.5));
                        assert!(hub.viewed_pass(&mut server.server_ctx()));
                        deliver_viewer_packets(game);
                        let slot = game
                            .client
                            .remote
                            .as_ref()
                            .unwrap()
                            .viewed
                            .slot_of(CLEARED)
                            .unwrap();
                        let wet = &viewed(game, slot).unwrap().weather;
                        assert_eq!(wet.rain_level.1, 0.75);
                        assert_eq!(wet.thunder_level.1, 0.5);

                        // Clear the destination with the shared flag still set, then travel
                        // without another viewed pass or an ordinary level sync.
                        assert!(level.set_weather_levels(server, 0.0, 0.0));
                        assert!(server.world.weather.raining);
                        let ctx = server.server_ctx();
                        let change =
                            hub.begin_level_change(0, [0.5, 64.0, 0.5], 0.0, None, 0, &ctx);
                        hub.finish_level_change(0, change, false, &ctx);
                        deliver_viewer_packets(game);
                        let remote = game.client.remote.as_ref().unwrap();
                        assert_eq!(remote.level, CLEARED);
                        assert_eq!(remote.viewed.slot_of(VIEWED), Some(slot));
                        let dry = &game.world.weather;
                        assert!(!dry.raining);
                        assert_eq!(dry.rain_level, (0.0, 0.0));
                        assert_eq!(dry.thunder_level, (0.0, 0.0));
                    });
                });
            },
        );
    }

    /// A wrapped `AddEntity` and its move put the entity in the viewed world, where it moves; the
    /// player's own world holds nothing of it. A wrapped `RemoveEntities` takes it out.
    #[test]
    #[ignore = "daily"]
    fn a_wrapped_entity_lives_in_the_viewed_world_not_the_players() {
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(open(1, "minecraft:the_nether")).unwrap();
        let add = C::AddEntity(play::AddEntity {
            id: 50,
            uuid: 50,
            kind: 0,
            position: [8.5, 70.0, 8.5],
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 64,
            head_yaw: 0,
            data: 0,
        });
        let step = C::MoveEntityPos(play::MoveEntityPos(play::MoveEntity {
            id: 50,
            delta: Some(play::VecDelta::Linear([4096, 0, -2048])),
            rotation: None,
            on_ground: true,
        }));
        game.handle_packet(run(1, &[add, step])).unwrap();
        let nether = &game
            .client
            .remote
            .as_ref()
            .unwrap()
            .viewed
            .get(1)
            .unwrap()
            .entities;
        let pig = nether.get(50).expect("in the viewed world");
        assert_eq!((pig.uuid, pig.kind, pig.data), (50, 0, 0));
        assert_eq!(pig.position, [9.5, 70.0, 8.0]);
        assert_eq!(pig.rotation.0, 90.0);
        assert!(pig.on_ground);
        let r = game.client.remote.as_ref().unwrap();
        assert!(r.tracked_entity(50).is_none(), "not in the player's world");
        assert!(game.mobs.iter().all(|m| m.id != 50));
        let gone = C::RemoveEntities(play::RemoveEntities { ids: vec![50] });
        game.handle_packet(run(1, &[gone])).unwrap();
        let nether = &game
            .client
            .remote
            .as_ref()
            .unwrap()
            .viewed
            .get(1)
            .unwrap()
            .entities;
        assert!(nether.get(50).is_none());
    }

    /// A death's Respawn drops the viewed worlds: the server opens again what it still views.
    #[test]
    #[ignore = "daily"]
    fn a_death_respawn_drops_the_viewed_worlds() {
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(open(1, "minecraft:the_nether")).unwrap();
        game.handle_packet(run(1, &[column(0, 0)])).unwrap();
        game.handle_packet(C::Respawn(play::Respawn {
            spawn: spawn("minecraft:overworld"),
            keep: 0,
        }))
        .unwrap();
        assert_eq!(game.client.remote.as_ref().unwrap().viewed.len(), 0);
    }

    /// A sound of `name` at fixed-point `pos` (eighths of a block), in the blocks' source.
    fn sound(name: &str, pos: [i32; 3], volume: f32, pitch: f32, seed: i64) -> C {
        C::Sound(play::Sound {
            sound: (-1, Some((name.into(), None))),
            source: SoundSource::Blocks.index() as i32,
            x: pos[0],
            y: pos[1],
            z: pos[2],
            volume,
            pitch,
            seed,
        })
    }

    /// What `viewed_level_sound` was asked: the level's name, the sound, its source, position,
    /// volume, pitch and seed.
    type Asked = (
        Option<&'static str>,
        String,
        SoundSource,
        [f64; 3],
        f32,
        f32,
        i64,
    );

    /// Places the anvil at (1.5, 70, -2.5) and attenuates it without changing pitch, leaves the bell
    /// unplayed and puts the harp at a position that is not finite; logs what it was asked.
    struct Portal(Arc<Mutex<Vec<Asked>>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Portal {
        fn viewed_level_sound(
            &self,
            _: &mut dyn mod_api::ClientHost,
            e: &mut mod_api::ViewedLevelSoundEvent<'_>,
        ) {
            let sound = e.sound.to_string();
            self.0.lock().unwrap().push((
                mod_api::__private::dimension_id(e.level.dimension()),
                sound.clone(),
                e.source,
                e.position,
                e.volume,
                e.pitch,
                e.seed,
            ));
            match sound.as_str() {
                "minecraft:block.anvil.land" => {
                    *e.play_at = Some([1.5, 70.0, -2.5]);
                    e.volume *= 0.125;
                }
                "minecraft:block.note_block.harp" => *e.play_at = Some([f64::NAN, 70.0, 0.0]),
                _ => {}
            }
        }
    }

    /// A viewed level's sound asks `viewed_level_sound` with the viewed level's handle, the
    /// sound's position there and the server's seed, and plays where the listener put it, with
    /// the listener's volume and pitch and the same seed. Left None, or put at a position that
    /// is not finite, it plays nothing. One bound to an entity of the viewed level is asked at that
    /// entity. A sound of the player's own level does not ask it.
    #[test]
    #[ignore = "daily"]
    fn a_viewed_sound_plays_where_a_listener_puts_it() {
        let asked = Arc::default();
        let mods = vec![mod_api::ModEntry::new(
            "portal",
            Box::new(Portal(Arc::clone(&asked))),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            let mut resources = None;
            let mut game = client(&mut resources);
            game.handle_packet(open(1, "minecraft:the_nether")).unwrap();
            game.client.audio.0.capture = Some(Vec::new());
            let anvil = sound("minecraft:block.anvil.land", [452, 512, 66], 2.0, -2.0, 42);
            game.handle_packet(run(1, std::slice::from_ref(&anvil)))
                .unwrap();
            let played = game.client.audio.0.capture.replace(Vec::new()).unwrap();
            assert_eq!(played.len(), 1);
            assert_eq!(played[0].event, "minecraft:block.anvil.land");
            assert_eq!(played[0].position, [1.5, 70.0, -2.5]);
            assert_eq!((played[0].volume, played[0].pitch), (0.25, -2.0));
            assert_eq!(played[0].source, SoundSource::Blocks.index());
            let C::Sound(p) = &anvil else { unreachable!() };
            assert_eq!(
                game.ask_viewed_level_sound(1, ViewedSound::at_position(p)),
                Some(ViewedPlay {
                    name: "minecraft:block.anvil.land".into(),
                    source: SoundSource::Blocks,
                    at: [1.5, 70.0, -2.5],
                    volume: 0.25,
                    pitch: -2.0,
                    seed: 42,
                }),
                "the server's seed"
            );
            let bell = sound("minecraft:block.bell.use", [0, 512, 0], 1.0, 1.0, 5);
            let harp = sound("minecraft:block.note_block.harp", [8, 512, 8], 1.0, 1.0, 6);
            game.handle_packet(run(1, &[bell, harp])).unwrap();
            let played = game.client.audio.0.capture.replace(Vec::new()).unwrap();
            assert_eq!(played.len(), 0, "the bell left None, the harp not finite");
            // Bound to entity 40 of the viewed level: asked at the entity there, played where
            // the listener put it; bound to an entity the world does not hold, not asked.
            let pig = C::AddEntity(play::AddEntity {
                id: 40,
                uuid: 40,
                kind: 7,
                position: [12.5, 70.0, -3.25],
                velocity: [0.0; 3],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 0,
            });
            let bound = |id| {
                C::SoundEntity(play::SoundEntity {
                    sound: (-1, Some(("minecraft:block.anvil.land".into(), None))),
                    source: SoundSource::Blocks.index() as i32,
                    id,
                    volume: -2.0,
                    pitch: -2.0,
                    seed: 9,
                })
            };
            game.handle_packet(run(1, &[pig, bound(40), bound(41)]))
                .unwrap();
            let played = game.client.audio.0.capture.replace(Vec::new()).unwrap();
            assert_eq!(played.len(), 1);
            assert_eq!(played[0].position, [1.5, 70.0, -2.5]);
            assert_eq!((played[0].volume, played[0].pitch), (-0.25, -2.0));
            // The player's own level: played where it is, without asking.
            game.handle_packet(sound(
                "minecraft:block.anvil.land",
                [8, 512, 8],
                1.0,
                1.0,
                7,
            ))
            .unwrap();
            let played = game.client.audio.0.capture.take().unwrap();
            assert_eq!(played.len(), 1);
            assert_eq!(played[0].position, [1.0, 64.0, 1.0]);
            let nether = Some("minecraft:the_nether");
            let blocks = SoundSource::Blocks;
            let anvil = |volume| {
                (
                    nether,
                    "minecraft:block.anvil.land".to_owned(),
                    blocks,
                    [56.5, 64.0, 8.25],
                    volume,
                    -2.0,
                    42,
                )
            };
            assert_eq!(
                *asked.lock().unwrap(),
                [
                    anvil(2.0),
                    anvil(2.0),
                    (
                        nether,
                        "minecraft:block.bell.use".into(),
                        blocks,
                        [0.0, 64.0, 0.0],
                        1.0,
                        1.0,
                        5
                    ),
                    (
                        nether,
                        "minecraft:block.note_block.harp".into(),
                        blocks,
                        [1.0, 64.0, 1.0],
                        1.0,
                        1.0,
                        6
                    ),
                    (
                        nether,
                        "minecraft:block.anvil.land".into(),
                        blocks,
                        [12.5, 70.0, -3.25],
                        -2.0,
                        -2.0,
                        9
                    ),
                ]
            );
        });
    }

    /// With no listener a run of a viewed level's sounds, as the client's own decoder reads it,
    /// costs one `listened` check, allocates nothing and plays nothing: both sounds carry their
    /// event inline, which decoding them would allocate.
    #[test]
    #[ignore = "daily"]
    fn with_no_listener_a_viewed_sound_is_dropped_after_one_check() {
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(open(1, "minecraft:the_nether")).unwrap();
        game.client.audio.0.capture = Some(Vec::new());
        let anvil = sound("minecraft:block.anvil.land", [452, 512, 66], 2.0, 0.5, 42);
        let bound = C::SoundEntity(play::SoundEntity {
            sound: (-1, Some(("minecraft:block.bell.use".into(), None))),
            source: SoundSource::Blocks.index() as i32,
            id: 40,
            volume: 1.0,
            pitch: 1.0,
            seed: 9,
        });
        let payload = run(1, &[anvil, bound]);
        let checks = || {
            sunrise_client_gameplay::game::client_side::viewed_worlds::sounds::LISTENED_CHECKS
                .with(std::cell::Cell::get)
        };
        let before = checks();
        let (routed, allocations) =
            sunrise_threads::allocation_count::measure(|| game.route_viewed_levels(data(&payload)));
        assert_eq!(routed, Ok(()));
        assert_eq!(allocations, 0);
        assert_eq!(checks() - before, 1);
        assert_eq!(
            game.client
                .audio
                .0
                .capture
                .take()
                .map(|played| played.len()),
            Some(0)
        );
    }
}
mod switch;
