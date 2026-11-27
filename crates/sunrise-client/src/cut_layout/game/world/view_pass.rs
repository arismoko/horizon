#![cfg(test)]
use sunrise_gameplay_common::game::Dim;
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_gameplay_common::game::native_block_entity_read::NativeBlockEntityReadGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
    use sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycleCommonGameExt1;
    use sunrise_gameplay_common::game::server_side::view_pass::tests::decode_ordered;
    use sunrise_gameplay_common::game::server_side::view_pass::tests::{Got, Sent};
    use sunrise_gameplay_common::game::transactions::TransactionsGameExt;

    use sunrise_gameplay_common::game::server_side::view_pass::tests::every;

    use sunrise_gameplay_common::game::server_side::view_pass::tests::respawns;

    use sunrise_gameplay_common::game::server_side::view_pass::tests::sent;

    use sunrise_gameplay_common::game::server_side::view_pass::tests::square;

    use sunrise_gameplay_common::game::server_side::view_pass::tests::tracked;

    use sunrise_gameplay::game::server_side::view_pass::tests::clocks_after;

    use sunrise_gameplay::game::server_side::view_pass::tests::host_a_viewer;

    use sunrise_gameplay::game::server_side::view_pass::tests::host_with_viewer;

    use sunrise_gameplay::game::server_side::view_pass::tests::summon_in_the_nether;

    use sunrise_gameplay::game::server_side::view_pass::tests::travel;

    use sunrise_gameplay::game::server_side::view_pass::tests::viewer;

    use super::*;

    use std::time::{Duration, Instant};

    use sunrise_protocol::{packets::play::Clientbound as C, viewed_levels::Level::Index};

    /// A server game on a scratch world, as `levels.rs`'s tests open one.
    pub(crate) fn open_game<'a>(
        resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
        root: &std::path::Path,
        home: &std::path::Path,
    ) -> sunrise_gameplay::game::ServerGame<'a> {
        use crate::game::load_resources;
        use crate::settings::Settings;
        use {sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress};
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
        let resources = resources.insert(loaded.resources);
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.to_path_buf();
        options.game_dir = Some(home.to_path_buf());
        options.player_name = Some("Viewer".into());
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
        let folder = home.join("saves/viewed").to_string_lossy().into_owned();
        game.load_world(
            sunrise_gameplay::game::world_loading::WorldRequest::new(
                &folder,
                Some("viewed"),
                Some(3716),
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
    /// A game on a scratch world in `SUNRISE_HOME`'s `name`, its viewer, and a `Loaded` Nether
    /// area at (40, 40), far from where anything else streams, ticked through `tick_server` until
    /// its 9 chunks were sent: what the viewer was sent until then.
    fn stream_the_nether<'a>(
        resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
        name: &str,
    ) -> (
        sunrise_gameplay::game::ServerGame<'a>,
        std::sync::mpsc::Receiver<sunrise_sim::lan::Out>,
        Sent,
    ) {
        use sunrise_sim::lan::viewed_levels::areas::ChunkAreaMode;
        stream_the_nether_in(resources, name, ChunkAreaMode::Loaded)
    }
    /// [`stream_the_nether`] with an area of `mode`: a `Ticking` one ticks the Nether.
    fn stream_the_nether_in<'a>(
        resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
        name: &str,
        mode: sunrise_sim::lan::viewed_levels::areas::ChunkAreaMode,
    ) -> (
        sunrise_gameplay::game::ServerGame<'a>,
        std::sync::mpsc::Receiver<sunrise_sim::lan::Out>,
        Sent,
    ) {
        use sunrise_sim::lan::viewed_levels::areas::{ChunkArea, ChunkAreaMode};
        let ticking = mode == ChunkAreaMode::Ticking;
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let home = std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap()).join(name);
        let _ = std::fs::remove_dir_all(&home);
        let mut game = open_game(resources, &root, &home);
        let rx = host_a_viewer(&mut game);
        let area = ChunkArea {
            level: Dim::Nether.id().into(),
            center: (40, 40),
            radius: 1,
            mode,
        };
        let hub = &mut game.server.lan.as_mut().unwrap().hub;
        hub.set_viewed_areas(2, "test:portal", vec![area]).unwrap();
        let mut all = Sent::default();
        let deadline = Instant::now() + Duration::from_secs(120);
        while all.chunks.len() < 9 && Instant::now() < deadline {
            game.tick_server(false).unwrap();
            assert_eq!(
                game.levels_to_tick().contains(&Dim::Nether),
                ticking,
                "ticked only for a Ticking area"
            );
            let s = sent(&rx);
            all.opened |= s.opened;
            all.chunks.extend(s.chunks);
            all.times.extend(s.times);
            std::thread::sleep(Duration::from_millis(20));
        }
        all.chunks.sort_unstable();
        (game, rx, all)
    }
    /// The view pass generates, postprocesses and sends a `Loaded` area's chunks in a level nobody
    /// is in, keeps them loaded across ticks (`lan_update_streaming`'s rebuild keeps the area), never ticks
    /// the level, and sends its game time, the server's, every 20 ticks.
    #[test]
    #[ignore = "daily"]
    fn a_loaded_area_in_a_level_nobody_is_in_is_streamed_and_sent_without_ticking() {
        let mut resources = None;
        let (mut game, rx, mut all) = stream_the_nether(&mut resources, "view-pass");
        assert!(all.opened, "the Nether was opened on the client");
        assert_eq!(all.chunks, square(), "the area's chunks, wrapped");
        game.native_lifecycle
            .begin_open(Some(mod_api::Authority::IntegratedServer));
        let nether = game.level(Dim::Nether.id()).unwrap();
        let chunk = game.chunk(nether, [40, 40]).unwrap();
        assert_eq!(chunk.resend(&mut game), Ok(()));
        let mut resent = Vec::new();
        for _ in 0..40 {
            game.tick_server(false).unwrap();
            let updates = sent(&rx);
            all.times.extend(updates.times);
            resent.extend(updates.chunks);
        }
        assert_eq!(resent, [(40, 40)], "resend goes to the viewed-level holder");
        let world = game.server.levels.0.parked_world(Dim::Nether).unwrap();
        assert!(
            square().iter().all(|c| world.chunks.contains_key(c)),
            "still loaded"
        );
        assert!(
            square()
                .iter()
                .all(|c| world.chunks[c].postprocess.is_empty()),
            "postprocessed by the view pass"
        );
        assert_eq!(world.game_time, game.world.game_time);
        let times = all.times;
        assert!(times.len() >= 2, "a game time every 20 ticks: {times:?}");
        assert!(
            times.windows(2).all(|w| w[1] - w[0] == 20),
            "the server's clock: {times:?}"
        );
    }
    /// The last area of a level nobody is in, withdrawn: the level keeps its view passes, with
    /// no ticket and no loading square, until its chunks are forgotten 52 ticks later and it is
    /// closed on the client; then the passes stop.
    #[test]
    #[ignore = "daily"]
    fn a_withdrawn_last_area_is_forgotten_and_closed_through_tick_server() {
        let mut resources = None;
        let (mut game, rx, all) = stream_the_nether(&mut resources, "view-pass-withdraw");
        assert_eq!(all.chunks, square());
        let hub = &mut game.server.lan.as_mut().unwrap().hub;
        hub.set_viewed_areas(2, "test:portal", Vec::new()).unwrap();
        let nether = Dim::Nether.id();
        assert!(hub.viewed_tickets(nether).is_empty() && hub.viewed_extra(nether).is_empty());
        assert_eq!(game.viewed_level(Dim::Nether), None);
        assert!(game.needs_view_pass(Dim::Nether), "its view is still open");
        // The first pass after the withdrawal starts the linger at this hub tick.
        game.tick_server(false).unwrap();
        let start = game.server.lan.as_ref().unwrap().hub.ticks;
        assert!(sent(&rx).forgotten.is_empty());
        let mut forgot_at = None;
        let mut last = Sent::default();
        for _ in 0..60 {
            game.tick_server(false).unwrap();
            let s = sent(&rx);
            if !s.forgotten.is_empty() {
                forgot_at = Some(game.server.lan.as_ref().unwrap().hub.ticks - start);
                last = s;
                break;
            }
        }
        assert_eq!(
            forgot_at,
            Some(52),
            "forgotten 52 ticks after the linger started"
        );
        last.forgotten.sort_unstable();
        assert_eq!(last.forgotten, square());
        assert!(last.closed, "closed once nothing of it is left");
        let hub = &game.server.lan.as_ref().unwrap().hub;
        let viewer = hub.guests.iter().find(|g| g.uuid == 2).unwrap();
        assert!(viewer.viewed.view(nether).is_none());
        assert!(!hub.viewed.active());
        assert!(!game.needs_view_pass(Dim::Nether), "no more passes");
    }
    /// An entity in a viewed `Ticking` Nether area reaches the viewer as a wrapped `AddEntity`,
    /// then its fall onto the bedrock roof (y 128) as wrapped moves, and once killed its death
    /// event (3) and, its death animation over, its removal.
    #[test]
    #[ignore = "daily"]
    fn an_entity_in_a_ticking_viewed_area_is_added_moved_and_removed_wrapped() {
        use sunrise_sim::lan::viewed_levels::areas::ChunkAreaMode;
        let mut resources = None;
        let (mut game, rx, _) =
            stream_the_nether_in(&mut resources, "view-entities", ChunkAreaMode::Ticking);
        let pig = summon_in_the_nether(&mut game, "minecraft:pig");
        let mut packets = Vec::new();
        for _ in 0..60 {
            game.tick_server(false).unwrap();
            packets.extend(sent(&rx).others);
        }
        let t = tracked(&packets, pig);
        let (_, at) = t.added.expect("added, wrapped in a Nether run");
        assert_eq!(at, [648.5, 130.0, 648.5]);
        let last = *t.moves.last().expect("its fall, wrapped");
        // It may walk off once it landed.
        assert_eq!(last[1], 128.0, "on the bedrock roof: {:?}", t.moves);
        assert!(!t.removed);
        game.enter_level(Dim::Nether).unwrap();
        assert!(game.run_command("kill @e[type=minecraft:pig]"));
        for _ in 0..40 {
            game.tick_server(false).unwrap();
            packets.extend(sent(&rx).others);
        }
        let t = tracked(&packets, pig);
        assert!(t.events.contains(&3), "its death, wrapped: {:?}", t.events);
        assert!(t.removed, "removed, wrapped");
        let viewer = game
            .server
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guests
            .iter()
            .find(|g| g.uuid == 2);
        assert!(
            !viewer.unwrap().tracker.is_tracking(pig),
            "not the player's own"
        );
    }
    /// An entity in a viewed `Loaded` Nether area is added, wrapped, and stands still in the air:
    /// nothing ticks there, and a position sync, if any, gives the same place.
    #[test]
    #[ignore = "daily"]
    fn an_entity_in_a_loaded_viewed_area_is_added_and_stands_still() {
        let mut resources = None;
        let (mut game, rx, _) = stream_the_nether(&mut resources, "view-entities-loaded");
        let pig = summon_in_the_nether(&mut game, "minecraft:pig");
        let mut packets = Vec::new();
        for _ in 0..60 {
            game.tick_server(false).unwrap();
            packets.extend(sent(&rx).others);
        }
        let t = tracked(&packets, pig);
        assert_eq!(t.added.map(|(_, at)| at), Some([648.5, 130.0, 648.5]));
        assert!(
            t.moves.iter().all(|&at| at == [648.5, 130.0, 648.5]),
            "no move: {:?}",
            t.moves
        );
    }
    #[test]
    #[ignore = "daily"]
    fn a_saved_block_entity_republishes_its_group_before_a_loaded_only_column_is_sent() {
        use mod_api::{
            Block, BlockEntityEvent, BlockEntityOnLoadEvent, BlockEntityPart, BlockEntitySideScope,
            BlockPart, BlockUpdateFlags, BlocksEvent, Compound, Hooks, LevelSide, ModEntry, Mods,
            Registrar, RegistrationContext, Tag,
        };
        use sunrise_protocol::connection_groups::{Cell, Entry, Section};
        use sunrise_sim::{
            lan::{
                PlayerSender,
                viewed_levels::areas::{ChunkArea, ChunkAreaMode},
            },
            raw_payloads::{Side, Transport, connection::record_overhead},
        };
        use {
            sunrise_gameplay::game::native_entity_refs::tests::fixture_root,
            sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
        };

        const NAME: &str = "cut_layout::game::server_side::view_pass::tests::a_saved_block_entity_republishes_its_group_before_a_loaded_only_column_is_sent";
        if run_fresh_cases(NAME, "SUNRISE_VIEW_GROUP_REOPEN", &["run"]) {
            return;
        }
        const BLOCK: &str = "view_group:holder";
        const COLUMN: (i32, i32) = (40, 40);
        const AT: [i32; 3] = [648, 120, 648];

        struct Holder;
        #[mod_api::sunrise_part]
        impl BlockPart for Holder {}
        #[mod_api::sunrise_part]
        impl BlockEntityPart for Holder {
            fn block_entity(
                &self,
                _: LevelSide<'_>,
                _: &mut BlockEntityEvent<'_>,
            ) -> Option<Compound> {
                Some(Compound::new())
            }
            fn block_entity_on_load(&self, event: &mut BlockEntityOnLoadEvent<'_>) {
                let BlockEntitySideScope::Server(scope) = &mut event.subject else {
                    return;
                };
                let Some((level, pos, group)) = scope
                    .with_view(|view| {
                        let Some(Tag::Long(group)) = view.nbt().get(&"group".into()) else {
                            return None;
                        };
                        Some((view.level(), view.position(), *group as u64))
                    })
                    .expect("live resident block entity")
                else {
                    return;
                };
                level
                    .set_connection_group(scope.host(), pos, Some(group))
                    .unwrap();
            }
        }

        struct Publisher;
        #[mod_api::__subscriptions]
        impl Hooks for Publisher {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let key = r.block_entity_type("holder").blocks(&[BLOCK]).build();
                r.block_entity_part(&key, Holder).unwrap();
            }
            fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
                event
                    .register(Block::new(BLOCK, "minecraft:stone"))
                    .unwrap();
            }
        }

        let mods: Mods = vec![ModEntry::new("view_group", Box::new(Publisher))].into();
        mod_platform::__private::install(mods.clone());
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        let content =
            sunrise_gameplay_common::game::mods::content_root(&fixture_root(), &Default::default())
                .unwrap();
        let root = content.prepared();
        let home = tempfile::tempdir().unwrap();
        for reopen in [false, true] {
            let mut resources = None;
            let mut game = open_game(&mut resources, root, home.path());
            // `load_world` is the synchronous test shortcut; open the handle epoch as the
            // server startup does before any block-entity load callbacks need it.
            game.native_lifecycle_open_started();
            game.native_lifecycle_world_ready();
            let transport = Transport::capture(Side::Server, record_overhead(&[])).unwrap();
            transport
                .accept(transport.local.manifest.entries())
                .unwrap();
            transport.finish_configuration().unwrap();
            let out = host_with_viewer(&mut game, || {
                PlayerSender::payload_executor_test_sender(transport)
            });
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .set_viewed_areas(
                    2,
                    "test:portal",
                    vec![ChunkArea {
                        level: Dim::Nether.id().into(),
                        center: COLUMN,
                        radius: 0,
                        mode: ChunkAreaMode::Loaded,
                    }],
                )
                .unwrap();

            let deadline = Instant::now() + Duration::from_secs(120);
            let (packets, body) = loop {
                game.tick_server(false).unwrap();
                assert!(!game.levels_to_tick().contains(&Dim::Nether));
                let packets = decode_ordered(out());
                if let Some(body) = packets.iter().position(|p| {
                    matches!(
                        p, Got::Wrapped(2, C::LevelChunkWithLight(p)) if (p.x, p.z) == COLUMN
                    )
                }) {
                    break (packets, body);
                }
                assert!(Instant::now() < deadline, "Nether column was never sent");
                std::thread::sleep(Duration::from_millis(20));
            };
            if reopen {
                let [
                    Got::Plain(C::BundleDelimiter(_)),
                    _,
                    Got::Groups(2, group),
                    Got::Plain(C::BundleDelimiter(_)),
                ] = &packets[body - 1..=body + 2]
                else {
                    panic!("reloaded column was not bundled with its group");
                };
                assert_eq!(
                    *group,
                    Section {
                        pos: AT.map(|v| v >> 4),
                        entries: vec![Entry {
                            cell: Cell::of(AT),
                            group: Some(729)
                        }],
                    }
                );

                // The viewer already has this column. A committed round trip to
                // the same block state still sends its changed group with that state.
                game.enter_level(Dim::Nether).unwrap();
                game.options.dedicated = Some(Vec::new());
                let level = game.level(Dim::Nether.id()).unwrap();
                let host: &dyn mod_api::Host = &game;
                let state = host.block_state_id(BLOCK).unwrap();
                let air = host.block_state_id("minecraft:air").unwrap();
                game.storage_scope(|game, _| {
                    assert_eq!(
                        level.set_block(game, AT, air, BlockUpdateFlags::NONE),
                        Ok(true)
                    );
                    assert_eq!(
                        level.set_block(game, AT, state, BlockUpdateFlags::NONE),
                        Ok(true)
                    );
                    assert_eq!(level.set_connection_group(game, AT, Some(730)), Ok(true));
                    ((), true)
                });
                let expected = game.world.block(AT[0], AT[1], AT[2]).0 as i32;
                game.enter_level(Dim::Overworld).unwrap();
                game.tick_server(false).unwrap();
                let packets = decode_ordered(out());
                let updated = packets.windows(4).find_map(|packets| match packets {
                    [
                        Got::Plain(C::BundleDelimiter(_)),
                        Got::Wrapped(2, C::BlockUpdate(block)),
                        Got::Groups(2, group),
                        Got::Plain(C::BundleDelimiter(_)),
                    ] if block.pos == AT && block.state == expected => Some(group),
                    _ => None,
                });
                assert_eq!(
                    updated,
                    Some(&Section {
                        pos: AT.map(|v| v >> 4),
                        entries: vec![Entry {
                            cell: Cell::of(AT),
                            group: Some(730)
                        }],
                    })
                );
            } else {
                game.enter_level(Dim::Nether).unwrap();
                game.options.dedicated = Some(Vec::new());
                let level = game.level(Dim::Nether.id()).unwrap();
                let state = game.pack.assets.registry.default_state(BLOCK).unwrap();
                assert_eq!(
                    game.level_set_block(level, AT, state, BlockUpdateFlags::NONE),
                    Ok(true)
                );
                let entity = game.block_entity(level, AT).expect("placed block entity");
                assert_eq!(
                    entity.merge_data(
                        &mut game,
                        Compound::from([("group".into(), Tag::Long(729))])
                    ),
                    Ok(true)
                );
                game.enter_level(Dim::Overworld).unwrap();
                game.save_game().unwrap();
            }
        }
    }
    /// A travel into the viewed Nether: a Respawn keeping all data, and none of the Nether's
    /// chunks sent again, plain or wrapped: they are the player's own now. The Overworld's chunks
    /// are a view of level 0 instead of being forgotten: no plain forget, no `open`; nothing
    /// covers them, so 52 ticks on they are forgotten wrapped and the level is closed.
    #[test]
    #[ignore = "daily"]
    fn a_travel_into_the_viewed_nether_sends_no_chunk_again_and_keeps_the_overworld_viewed() {
        let mut resources = None;
        let (mut game, rx, all) = stream_the_nether(&mut resources, "view-travel");
        assert_eq!(all.chunks, square());
        // The Overworld's chunks the player's client holds, around the spawn.
        let overworld: Vec<(i32, i32)> = (-1..=1)
            .flat_map(|x| (-1..=1).map(move |z| (x, z)))
            .collect();
        let hub = &mut game.server.lan.as_mut().unwrap().hub;
        let i = hub.guests.iter().position(|g| g.uuid == 2).unwrap();
        hub.guests
            .update(i, |g| g.sent_chunks = overworld.iter().copied().collect());
        let _ = every(&rx);
        travel(&mut game, Dim::Nether, [648.5, 130.0, 648.5]);
        let g = viewer(&game);
        assert_eq!(g.dimension, Dim::Nether.id());
        assert!(square().iter().all(|c| g.sent_chunks.contains(c)));
        assert!(g.viewed.view(Dim::Nether.id()).is_none(), "its own level");
        let kept = g
            .viewed
            .view(Dim::Overworld.id())
            .expect("the Overworld, viewed");
        let mut kept_sent: Vec<(i32, i32)> = kept.sent.iter().copied().collect();
        kept_sent.sort_unstable();
        assert_eq!((kept.level, kept_sent), (Index(0), overworld.clone()));
        let mut got = every(&rx);
        for _ in 0..70 {
            game.tick_server(false).unwrap();
            got.extend(every(&rx));
        }
        assert_eq!(respawns(&got), vec![(Dim::Nether.id().to_owned(), 3)]);
        let resent: Vec<(i32, i32)> = (got.iter())
            .filter_map(|g| match g {
                Got::Plain(C::LevelChunkWithLight(c))
                | Got::Wrapped(_, C::LevelChunkWithLight(c)) => Some((c.x, c.z)),
                _ => None,
            })
            .collect();
        assert!(
            square().iter().all(|c| !resent.contains(c)),
            "sent again: {resent:?}"
        );
        let plain_forgets: Vec<(i32, i32)> = (got.iter())
            .filter_map(|g| match g {
                Got::Plain(C::ForgetLevelChunk(f)) => Some((f.pos[0], f.pos[1])),
                _ => None,
            })
            .collect();
        assert!(
            overworld.iter().all(|c| !plain_forgets.contains(c)),
            "{plain_forgets:?}"
        );
        assert!(!got.iter().any(|g| matches!(g, Got::Open(0))), "no open");
        let mut forgotten: Vec<(i32, i32)> = (got.iter())
            .filter_map(|g| match g {
                Got::Wrapped(0, C::ForgetLevelChunk(f)) => Some((f.pos[0], f.pos[1])),
                _ => None,
            })
            .collect();
        forgotten.sort_unstable();
        assert_eq!(forgotten, overworld, "forgotten wrapped after the linger");
        let last_forget = got
            .iter()
            .rposition(|g| matches!(g, Got::Wrapped(0, C::ForgetLevelChunk(_))));
        let close = got.iter().position(|g| matches!(g, Got::Close(0)));
        assert!(close > last_forget, "closed once forgotten");
        assert!(viewer(&game).viewed.view(Dim::Overworld.id()).is_none());
    }
    /// A travel into a level the client does not view drops its views: the Nether is forgotten
    /// without `close` and, its area still asking for it, opened again after the Respawn and its
    /// chunks sent again; the Overworld left is not kept.
    #[test]
    #[ignore = "daily"]
    fn a_travel_into_a_level_not_viewed_drops_the_views() {
        let mut resources = None;
        let (mut game, rx, all) = stream_the_nether(&mut resources, "view-travel-end");
        assert_eq!(all.chunks, square());
        travel(&mut game, Dim::End, [8.5, 80.0, 8.5]);
        let g = viewer(&game);
        assert_eq!(g.dimension, Dim::End.id());
        assert!(g.viewed.view(Dim::Nether.id()).is_none(), "forgotten");
        assert!(g.viewed.view(Dim::Overworld.id()).is_none(), "not kept");
        let mut got = every(&rx);
        let deadline = Instant::now() + Duration::from_secs(120);
        let nether_chunks = |got: &[Got]| {
            let mut c: Vec<(i32, i32)> = (got.iter())
                .filter_map(|g| match g {
                    Got::Wrapped(2, C::LevelChunkWithLight(c)) => Some((c.x, c.z)),
                    _ => None,
                })
                .collect();
            c.sort_unstable();
            c
        };
        while nether_chunks(&got).len() < 9 && Instant::now() < deadline {
            game.tick_server(false).unwrap();
            got.extend(every(&rx));
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(respawns(&got), vec![(Dim::End.id().to_owned(), 3)]);
        let respawn = got
            .iter()
            .position(|g| matches!(g, Got::Plain(C::Respawn(_))))
            .unwrap();
        let open = got.iter().position(|g| matches!(g, Got::Open(2)));
        assert!(
            open.is_some_and(|o| o > respawn),
            "opened again after the Respawn"
        );
        assert_eq!(nether_chunks(&got), square(), "sent again");
        assert!(
            !got.iter()
                .any(|g| matches!(g, Got::Close(_) | Got::Open(0)))
        );
    }
    /// The server's clocks reach a viewer of a level nobody is in when they change, not only at
    /// `open`: `time set`, `time add` and the day stopping each send the clocks, wrapped.
    #[test]
    #[ignore = "daily"]
    fn clock_changes_reach_the_viewer_of_a_level_nobody_is_in() {
        let mut resources = None;
        let (mut game, rx, _) = stream_the_nether(&mut resources, "view-clocks");
        game.set_allow_commands(true);
        let told = clocks_after(&mut game, &rx, "time set 1234");
        assert_eq!(told.len(), 1, "{told:?}");
        assert!(
            told[0]
                .iter()
                .any(|&(total, rate)| (1234..=1235).contains(&total) && rate == 1.0),
            "{told:?}"
        );
        let told = clocks_after(&mut game, &rx, "time add 1000");
        assert!(
            told[0]
                .iter()
                .any(|&(total, _)| (2234..=2236).contains(&total)),
            "{told:?}"
        );
        // A clock that only runs on sends nothing but the game time every 20 ticks.
        for _ in 0..30 {
            assert_eq!(
                clocks_after(&mut game, &rx, "seed"),
                Vec::<Vec<(i64, f32)>>::new()
            );
        }
        let told = clocks_after(&mut game, &rx, "gamerule advance_time false");
        assert!(told[0].iter().any(|&(_, rate)| rate == 0.0), "{told:?}");
    }
    /// A mod's sound in the Nether, a level nobody is in, reaches the viewer whose client holds
    /// its chunk, wrapped, through `tick_server` (#3983).
    #[test]
    #[ignore = "daily"]
    fn a_sound_in_a_viewed_level_reaches_its_viewer_through_tick_server() {
        let mut resources = None;
        let (mut game, rx, all) = stream_the_nether(&mut resources, "view-pass-sound");
        assert_eq!(all.chunks, square());
        assert!(all.sounds.is_empty());
        // A mod's handle needs the world's epoch, which this test world never began.
        game.native_lifecycle
            .begin_open(Some(mod_api::Authority::IntegratedServer));
        let nether = game.level(Dim::Nether.id()).unwrap();
        let blocks = mod_api::SoundSource::Blocks;
        let at = [648.5, 70.0, 650.25];
        assert!(game.level_play_sound(
            nether,
            "minecraft:block.bell.use",
            at,
            blocks,
            1.0,
            0.75,
            None
        ));
        game.tick_server(false).unwrap();
        let sounds = sent(&rx).sounds;
        assert_eq!(sounds.len(), 1, "{sounds:?}");
        let sound = &sounds[0];
        assert_eq!((sound.x, sound.y, sound.z), (5188, 560, 5202));
        assert_eq!((sound.source, sound.volume, sound.pitch), (4, 1.0, 0.75));
    }
    /// A mod's sound bound to a pig in the viewed Nether reaches the viewer whose view tracks the
    /// pig, wrapped, bound to the same entity, through `tick_server` (#3983).
    #[test]
    #[ignore = "daily"]
    fn a_sound_bound_to_a_viewed_entity_reaches_the_view_tracking_it() {
        let mut resources = None;
        let (mut game, rx, _) = stream_the_nether(&mut resources, "view-pass-bound-sound");
        let pig = summon_in_the_nether(&mut game, "minecraft:pig");
        for _ in 0..60 {
            game.tick_server(false).unwrap();
        }
        let _ = sent(&rx);
        let tracks = |game: &sunrise_gameplay::game::ServerGame<'_>| {
            let view = viewer(game).viewed.view(Dim::Nether.id()).unwrap();
            view.tracker.is_tracking(pig)
        };
        assert!(tracks(&game), "the view tracks the pig");
        game.enter_level(Dim::Nether).unwrap();
        game.native_lifecycle
            .begin_open(Some(mod_api::Authority::IntegratedServer));
        let emitter = game.entity(pig as u32).unwrap();
        let neutral = mod_api::SoundSource::Neutral;
        let sound = "minecraft:entity.pig.ambient";
        assert!(game.entity_play_bound_sound(emitter, sound, neutral, 1.0, 1.25));
        game.tick_server(false).unwrap();
        let bound: Vec<_> = (sent(&rx).others.into_iter())
            .filter_map(|p| match p {
                C::SoundEntity(p) => Some((p.id, p.source, p.volume, p.pitch)),
                _ => None,
            })
            .collect();
        assert_eq!(bound, [(pig, 6, 1.0, 1.25)]);
    }
}
