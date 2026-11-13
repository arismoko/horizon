//! The view pass (#3716): a level that only viewed `Loaded` areas name is not ticked, but its
//! chunks are loaded, generated and sent to the players whose clients view it.
//!
//! After the ticked levels, `tick_server` runs a pass for each such level: it enters the level,
//! rebuilds `Streaming.extra` (`lan_update_streaming`, where the areas are added: no `tick_lan`
//! runs here to call it), collects the finished columns (`update_world`), postprocesses them as
//! vanilla does before a chunk is sent (`postprocess_chunks`: what it schedules waits for a tick),
//! then runs only the hub's viewed-level sending. Nothing ticks there: no block, fluid, scheduled or random ticks, no
//! entities, no weather cycle, no level tick hooks and no player work. Its time is the server's,
//! as in 26.3, where no level but the Overworld runs a clock of its own: `enter_level` gives it
//! the clocks as they are, and its viewers get its game time every 20 ticks. A level whose last
//! area was withdrawn keeps its passes, with no ticket, until its chunks were forgotten and it was
//! closed on the clients.
use super::{Dim, Game};
use horizon_client::{Result, lan::hub::TickCtx};

impl Game<'_> {
    /// What the players' viewed areas ask of level `dim`: None for nothing, else whether one of
    /// them ticks.
    pub(super) fn viewed_level(&self, dim: Dim) -> Option<bool> {
        self.lan.as_ref()?.hub.viewed_level(dim.id())
    }

    /// Whether level `dim` needs a view pass when it is not ticked: areas name it, or a client
    /// still holds chunks of it whose linger, forgets and `close` are not done.
    pub(super) fn needs_view_pass(&self, dim: Dim) -> bool {
        self.viewed_level(dim).is_some()
            || self
                .lan
                .as_ref()
                .is_some_and(|h| h.hub.viewed_open(dim.id()))
    }

    /// A view pass for each level the viewed areas name that `ticked` does not hold.
    pub(super) fn run_view_passes(&mut self, ticked: &[Dim]) -> Result<()> {
        if !self.lan.as_ref().is_some_and(|h| h.hub.viewed.active()) {
            return Ok(());
        }
        let viewed: Vec<Dim> = Dim::all()
            .into_iter()
            .filter(|d| !ticked.contains(d) && self.needs_view_pass(*d))
            .collect();
        for dim in viewed {
            let _span = horizon_client::spans::guard("view_pass");
            self.enter_level(dim)?;
            self.update_dedicated_streaming();
            self.lan_update_streaming();
            self.update_world();
            // `LevelChunk.postProcessGeneration` comes before a chunk is sent
            // (`ChunkMap.prepareTickingChunk`, then `onChunkReadyToSend`), ticked or not.
            self.postprocess_chunks()?;
            self.view_pass_send();
        }
        Ok(())
    }

    /// The hub's viewed-level sending for the level in place, without its tick.
    fn view_pass_send(&mut self) {
        let spawn = self.world_spawn_position();
        let clock_rate = self.network_clock_rate();
        let sea_level = self.end.locator().map_or(63, |terrain| terrain.sea_level());
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let mut ctx = TickCtx {
            sea_level,
            world: &mut self.world,
            entities: &[],
            next_entity: &mut self.next_entity,
            spawn,
            spawn_angle: self.level.world_spawn_angle,
            difficulty: i32::from(self.ui.difficulty),
            difficulty_locked: self.level.difficulty_locked,
            clock_rate,
            tick_rate: self.clock.rate(),
            tick_frozen: self.clock.frozen(),
            tick_steps: self.clock.step_state(),
            collision: Some(self.collision),
            border: &self.border,
            streaming: Some(&self.streaming),
        };
        host.hub.viewed_pass(&mut ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_protocol::{
        buf::Reader,
        packets::play::Clientbound as C,
        viewed_levels::{CHANNEL, Message},
    };
    use std::time::{Duration, Instant};

    /// A server game on a scratch world, as `levels.rs`'s tests open one.
    fn open_game<'a>(
        resources: &'a mut Option<std::sync::Arc<crate::game::Resources>>,
        root: &std::path::Path,
        home: &std::path::Path,
    ) -> Game<'a> {
        use crate::game::{Audio, Options, Progress, Settings, load_resources};
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            Vec::new(),
            None,
            root,
            &settings,
            Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let resources = resources.insert(loaded.resources);
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.to_path_buf();
        options.game_dir = Some(home.to_path_buf());
        options.player_name = Some("Viewer".into());
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.to_path_buf(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();
        let folder = home.join("saves/viewed").to_string_lossy().into_owned();
        game.load_world(
            crate::game::world_loading::WorldRequest::new(
                &folder,
                Some("viewed"),
                Some(3716),
                true,
                None,
                std::sync::Arc::new(crate::game::api_values::game_rule_registry()),
            )
            .unwrap(),
        )
        .unwrap();
        game
    }

    /// Opens the world to the network with one player in the Overworld whose client views 2
    /// levels; its packets.
    fn host_a_viewer(game: &mut Game<'_>) -> std::sync::mpsc::Receiver<horizon_client::lan::Out> {
        use horizon_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
        let net = NetConfig {
            announce: false,
            ..NetConfig::lan(0, "view-pass-test".into(), 2)
        };
        let settings = crate::game::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        };
        let hub = HubConfig {
            default_game_mode: 0,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: Dim::Overworld.id().into(),
            allow_flight: false,
            seed: 0,
            max_players: 2,
            integrated: false,
            levels: Default::default(),
        };
        game.lan_host(settings, net, hub).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut sender = PlayerSender::new(tx, std::sync::Arc::default());
        sender.max_viewed_levels = 2;
        let joined = Joined {
            conn: 2,
            uuid: 2,
            name: "Viewer".into(),
            properties: Vec::new(),
            info: horizon_protocol::packets::configuration::ClientInformation::new(2),
            brand: "horizon".into(),
            sender,
            latency: std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
        };
        game.next_entity += 1;
        let mut guest = Guest::new(joined, game.next_entity as i32, [0.5, 80.0, 0.5], 0);
        guest.placed = true;
        guest.loaded = true;
        guest.dimension = Dim::Overworld.id().into();
        guest.max_unacknowledged = 1000;
        game.lan.as_mut().unwrap().hub.guests.push(guest);
        rx
    }

    /// What a client was sent of the Nether (level 2) on `horizon:levels`.
    #[derive(Default)]
    struct Sent {
        opened: bool,
        chunks: Vec<(i32, i32)>,
        forgotten: Vec<(i32, i32)>,
        closed: bool,
        times: Vec<i64>,
    }

    fn sent(rx: &std::sync::mpsc::Receiver<horizon_client::lan::Out>) -> Sent {
        let mut sent = Sent::default();
        let bodies = rx.try_iter().flat_map(|o| match o {
            horizon_client::lan::Out::Packet(body) => vec![body],
            horizon_client::lan::Out::Packets(bodies) => bodies,
            horizon_client::lan::Out::Disconnect(_) => Vec::new(),
        });
        for body in bodies {
            let mut r = Reader::new(&body);
            let id = r.varint().unwrap();
            let Ok(C::CustomPayload(p)) = C::decode(id, r.rest()) else {
                continue;
            };
            if p.channel != CHANNEL {
                continue;
            }
            match Message::decode(&p.data).unwrap() {
                Message::Open { level, .. } => sent.opened |= level == 2,
                Message::Close { level } => sent.closed |= level == 2,
                Message::Run { level: 2, packets } => {
                    for (id, body) in packets {
                        match C::decode(id, body) {
                            Ok(C::LevelChunkWithLight(c)) => sent.chunks.push((c.x, c.z)),
                            Ok(C::ForgetLevelChunk(f)) => sent.forgotten.push((f.pos[0], f.pos[1])),
                            Ok(C::SetTime(t)) if t.clocks.is_empty() => {
                                sent.times.push(t.game_time)
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        sent
    }

    /// The chunks of a radius-1 square at (40, 40).
    fn square() -> Vec<(i32, i32)> {
        (39..=41)
            .flat_map(|x| (39..=41).map(move |z| (x, z)))
            .collect()
    }

    /// A game on a scratch world in `HORIZON_HOME`'s `name`, its viewer, and a `Loaded` Nether
    /// area at (40, 40), far from where anything else streams, ticked through `tick_server` until
    /// its 9 chunks were sent: what the viewer was sent until then.
    fn stream_the_nether<'a>(
        resources: &'a mut Option<std::sync::Arc<crate::game::Resources>>,
        name: &str,
    ) -> (
        Game<'a>,
        std::sync::mpsc::Receiver<horizon_client::lan::Out>,
        Sent,
    ) {
        use horizon_client::lan::viewed_levels::areas::{ChunkArea, ChunkAreaMode};
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let home = std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap()).join(name);
        let _ = std::fs::remove_dir_all(&home);
        let mut game = open_game(resources, &root, &home);
        let rx = host_a_viewer(&mut game);
        let area = ChunkArea {
            level: Dim::Nether.id().into(),
            center: (40, 40),
            radius: 1,
            mode: ChunkAreaMode::Loaded,
        };
        let hub = &mut game.lan.as_mut().unwrap().hub;
        hub.set_viewed_areas(2, "test:portal", vec![area]).unwrap();
        let mut all = Sent::default();
        let deadline = Instant::now() + Duration::from_secs(120);
        while all.chunks.len() < 9 && Instant::now() < deadline {
            game.tick_server(false).unwrap();
            assert!(
                !game.levels_to_tick().contains(&Dim::Nether),
                "never ticked"
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
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and a scratch HORIZON_HOME"]
    fn a_loaded_area_in_a_level_nobody_is_in_is_streamed_and_sent_without_ticking() {
        let mut resources = None;
        let (mut game, rx, mut all) = stream_the_nether(&mut resources, "view-pass");
        assert!(all.opened, "the Nether was opened on the client");
        assert_eq!(all.chunks, square(), "the area's chunks, wrapped");
        // Kept loaded for 40 more ticks, its clock the server's.
        for _ in 0..40 {
            game.tick_server(false).unwrap();
            all.times.extend(sent(&rx).times);
        }
        let world = game.levels.parked_world(Dim::Nether).unwrap();
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
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and a scratch HORIZON_HOME"]
    fn a_withdrawn_last_area_is_forgotten_and_closed_through_tick_server() {
        let mut resources = None;
        let (mut game, rx, all) = stream_the_nether(&mut resources, "view-pass-withdraw");
        assert_eq!(all.chunks, square());
        let hub = &mut game.lan.as_mut().unwrap().hub;
        hub.set_viewed_areas(2, "test:portal", Vec::new()).unwrap();
        let nether = Dim::Nether.id();
        assert!(hub.viewed_tickets(nether).is_empty() && hub.viewed_extra(nether).is_empty());
        assert_eq!(game.viewed_level(Dim::Nether), None);
        assert!(game.needs_view_pass(Dim::Nether), "its view is still open");
        // The first pass after the withdrawal starts the linger at this hub tick.
        game.tick_server(false).unwrap();
        let start = game.lan.as_ref().unwrap().hub.ticks;
        assert!(sent(&rx).forgotten.is_empty());
        let mut forgot_at = None;
        let mut last = Sent::default();
        for _ in 0..60 {
            game.tick_server(false).unwrap();
            let s = sent(&rx);
            if !s.forgotten.is_empty() {
                forgot_at = Some(game.lan.as_ref().unwrap().hub.ticks - start);
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
        let hub = &game.lan.as_ref().unwrap().hub;
        let viewer = hub.guests.iter().find(|g| g.uuid == 2).unwrap();
        assert!(viewer.viewed.view(nether).is_none());
        assert!(!hub.viewed.active());
        assert!(!game.needs_view_pass(Dim::Nether), "no more passes");
    }
}
