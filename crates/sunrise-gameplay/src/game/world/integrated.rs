//! The integrated server (`IntegratedServer`): single player's world runs on
//! its own "Server thread", as vanilla's does, and the window joins it as a
//! player through memory, so a world tick never holds up a frame.
//!
//! The window asks its server for what only the server knows (Open to LAN,
//! the world's options) with [`Integrated::ask`] and an `OwnerRequest`, as vanilla's client calls
//! into its `IntegratedServer`.
//!
//! Every single-player world runs this way; LAN guests and dedicated servers
//! use the same server code.
use super::{
    Dim, Options, PackResources, Resources,
    execution_role::ExecutionRole,
    lan::Owner,
    native_singleplayer_owner::SingleplayerProfile,
    owner_link::OwnerRequest,
    world_loading::{WorldLoader, WorldRequest},
};
#[cfg(any())]
use crate::game::client_side::integrated_client::Integrated;
#[cfg(any(test, feature = "test-support"))]
use std::sync::mpsc::channel;
#[cfg(test)]
use std::time::Duration;
use std::{
    net::TcpListener,
    sync::{
        Arc,
        atomic::Ordering,
        mpsc::{Receiver, Sender},
    },
    time::Instant,
};
use sunrise_game_data::Result;
#[cfg(test)]
use sunrise_gameplay_common::game::integrated_link::stack_for;
use sunrise_gameplay_common::game::{
    integrated_link::{
        ComponentHarness, Flags, Launch, PLAYER_CHUNKS, ProfileResolverFactory, Ready,
        ServerOptions, server_thread_stack,
    },
    lan_common::{
        LanSettings,
        sync::{SyncPreload, preload_sync_data},
    },
};
use sunrise_level::level_loading::{STATUS_RADIUS, Stage};
use sunrise_level::sim_settings::SimSettings;
use sunrise_level::spans::startup;
use sunrise_protocol::{connection::Pipe, text::translatable};
use sunrise_sim::{
    chunk_io::Reports,
    lan::{Lan, MAX_PLAYERS, NetConfig, hub::HubConfig},
};

#[path = "integrated/capture.rs"]
pub(in crate::game) mod capture;

#[cfg(feature = "test-support")]
pub use super::owner_link_server::{ClientHandoff, test_owner_request};

#[cfg(feature = "test-support")]
pub fn run_test_owner_tasks(
    game: &mut crate::game::ServerGame<'_>,
    handoff: &mut ClientHandoff<'_>,
) {
    game.drain_owner_tasks(|game, request| game.dispatch_owner_request(request, handoff));
}

/// The server thread's side of the window's handle (`Game::serving`).
pub struct Serving {
    pub flags: Arc<Flags>,
    pub tasks: Receiver<OwnerRequest>,
    /// The window's profile.
    pub owner: u128,
    /// Open to LAN or closed from inside a player's command, while the hub
    /// is out of its place: applied after the tick.
    pub pending: Option<Publish>,
    pub previous_simulation_distance: i32,
    /// Where the load focus is and how far the player's chunks are.
    pub load: LoadPhase,
}
/// The part of `LevelLoadListener` after `prepareLevels`: `PrepareSpawnTask`'s
/// `LOAD_PLAYER_CHUNKS`.
#[derive(Default)]
pub struct LoadPhase {
    /// `updateFocus`: the spawn chunk, once the stage started.
    pub(in crate::game) focus: Option<(i32, i32)>,
    /// `finish(LOAD_PLAYER_CHUNKS)`.
    pub(in crate::game) done: bool,
}
impl Serving {
    /// A server thread's handle with no thread behind it, published on `port` (0: not).
    #[cfg(any(test, feature = "test-support"))]
    pub fn fixture(port: u16) -> Self {
        Self::fixture_with_reports(port, Arc::default())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(in crate::game) fn fixture_with_reports(port: u16, reports: Arc<Reports>) -> Self {
        let flags = Arc::new(Flags {
            reports,
            ..Flags::default()
        });
        flags.published.store(port, Ordering::Relaxed);
        let (_sender, tasks) = channel();
        Self {
            flags,
            tasks,
            owner: 1,
            pending: None,
            previous_simulation_distance: 0,
            load: LoadPhase::default(),
        }
    }

    pub(in crate::game) fn entity_distance(&self) -> f64 {
        f64::from_bits(self.flags.entity_distance.load(Ordering::Relaxed))
    }
    /// Send reports only to this server's owner, not to LAN clients.
    pub(in crate::game) fn reports(&self) -> &Reports {
        &self.flags.reports
    }

    /// The window reads this as `MinecraftServer.isCurrentlySaving`.
    pub(in crate::game::server_side) fn set_saving(&self, saving: bool) {
        self.flags.saving.store(saving, Ordering::Relaxed);
    }
}
pub enum Publish {
    Open(TcpListener, LanSettings),
    Close,
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn server_has_singleplayer_server(&self) -> bool {
        false
    }
    pub(in crate::game) fn server_is_lan_published(&self) -> bool {
        self.server.lan.is_some()
    }

    /// `PrepareSpawnTask`'s `LOAD_PLAYER_CHUNKS`, reported to the window's
    /// loading screen: the focus moves to the spawn chunk when the stage
    /// starts, the 7x7 chunks around it count as they are ready, and the
    /// status map around the focus follows (`ChunkLoadStatusView.get`).
    pub(in crate::game::server_side) fn publish_load_view(&mut self) {
        let Some(serving) = self.server.serving.as_mut() else {
            return;
        };
        if serving.load.done {
            return;
        }
        let view = serving.flags.load.clone();
        let focus = match serving.load.focus {
            Some(focus) => focus,
            None => {
                let Some(spawn) = self.common.spawn else {
                    return;
                };
                let focus = ((spawn.x.floor() as i32) >> 4, (spawn.z.floor() as i32) >> 4);
                view.start(Stage::LoadPlayerChunks, PLAYER_CHUNKS);
                serving.load.focus = Some(focus);
                focus
            }
        };
        let ready = (-3..=3)
            .flat_map(|dz| (-3..=3).map(move |dx| (focus.0 + dx, focus.1 + dz)))
            .filter(|&(x, z)| self.server.streaming.is_lit(x, z))
            .count();
        view.update(Stage::LoadPlayerChunks, ready, PLAYER_CHUNKS);
        let diameter = STATUS_RADIUS * 2 + 1;
        view.set_status(
            (0..diameter)
                .flat_map(|x| (0..diameter).map(move |z| (x, z)))
                .map(|(x, z)| {
                    self.server.streaming.latest_status(
                        &self.common.world,
                        focus.0 + x - STATUS_RADIUS,
                        focus.1 + z - STATUS_RADIUS,
                    )
                })
                .collect(),
        );
        if ready == PLAYER_CHUNKS {
            view.finish(Stage::LoadPlayerChunks);
            if let Some(serving) = self.server.serving.as_mut() {
                serving.load.done = true;
            }
        }
    }

    /// The server thread's loop (`MinecraftServer.runServer`): 20 ticks a
    /// second, the world held while the window is paused.
    pub(in crate::game::server_side) fn run_integrated(
        &mut self,
        launch: Launch,
        flags: Arc<Flags>,
        tasks: Receiver<OwnerRequest>,
        ready: &Sender<Ready>,
        opening: Opening,
    ) -> Result<()> {
        // `IntegratedServer`'s constructor: `setSingleplayerProfile(minecraft.getGameProfile())`,
        // fixed before the configs and startup hooks, which mods may read it in.
        self.server.singleplayer_profile = Some(SingleplayerProfile::launched(
            launch.player.clone(),
            launch.owner,
            launch.owner_profile.as_deref(),
        ));
        let result = self.run_server_lifecycle(false, |game| {
            game.run_integrated_inner(launch, flags, tasks, ready, opening)
        });
        self.finish_server_capture();
        result
    }

    pub(in crate::game::server_side) fn run_integrated_inner(
        &mut self,
        launch: Launch,
        flags: Arc<Flags>,
        tasks: Receiver<OwnerRequest>,
        ready: &Sender<Ready>,
        opening: Opening,
    ) -> Result<super::server_lifecycle::ServerRunOutcome> {
        let (pipe, world_name) = self.integrated_open(&launch, &flags, tasks, opening)?;
        // IntegratedServer.initServer ends with `ServerStartingEvent`, right before
        // `MinecraftServer.runServer` fires `ServerStartedEvent`.
        self.native_server_starting(false);
        // The server thread's `server_started` site, before the tick loop.
        self.native_lifecycle_world_ready();
        // Hand the local connection pipe to the client only after `server_started`,
        // so a failed or panicking startup never hands out a live pipe.
        let _ = ready.send(Ok((
            pipe,
            world_name,
            self.world_biomes_base
                .as_ref()
                .expect("world is open")
                .clone(),
        )));
        if let (Some(script), Some(output)) = (
            std::env::var_os("SUNRISE_COMMAND_SCRIPT"),
            std::env::var_os("SUNRISE_COMMAND_OUTPUT"),
        ) {
            // The script owns the clock: connection setup and terrain loading
            // cannot add wall-clock-dependent world ticks to oracle output.
            self.run_command_harness(script.as_ref(), output.as_ref())?;
            return Ok(super::server_lifecycle::ServerRunOutcome::Orderly);
        }
        let slow_ms = self
            .common
            .options
            .spikes
            .unwrap_or(super::dedicated::SLOW_TICK_MS);
        self.server
            .profile
            .server
            .get_or_insert_with(|| capture::ServerSide::new(slow_ms));
        self.integrated_tick_loop(&flags)?;
        Ok(super::server_lifecycle::ServerRunOutcome::Orderly)
    }

    pub fn integrated_tick_loop(&mut self, flags: &Flags) -> Result<()> {
        enum TickFailure {
            Error(Box<dyn std::error::Error>),
            Panic(Box<dyn std::any::Any + Send>),
        }
        let mut next = Instant::now();
        let mut was_paused = false;
        let mut smoothed_tick_ms = 0.0f32;
        let mut failure = None;
        while !flags.stop.load(Ordering::Relaxed) {
            self.run_tasks();
            if self.server.hold_server {
                // Keep servicing control requests so the fixture can release the barrier.
                // Unlike /tick freeze, no player's queued packets are applied here.
                self.wait_for_next_server_tick(&mut next, true);
                continue;
            }
            self.server.server_tick_deadline = Some(next + self.common.clock.tick_duration());
            let paused = flags.paused.load(Ordering::Relaxed);
            let ticked =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
                    #[cfg(any(test, feature = "test-support"))]
                    if flags.fail_tick.swap(false, Ordering::Relaxed) {
                        self.common.level.name = "saved after integrated tick panic".into();
                        self.common.experience.total = 37;
                        panic!("integrated tick fixture");
                    }
                    if paused && !was_paused {
                        // IntegratedServer.tickServer: the tick the game pauses saves
                        // everything (saveEverything, without flushing), as an
                        // autosave does.
                        log::info!("Saving and pausing game...");
                        self.save_game_with(false)?;
                    }
                    was_paused = paused;
                    if paused {
                        // Only the connections run.
                        self.tick_server_measured(true)?;
                    } else {
                        self.tick_integrated_unpaused(flags, &mut smoothed_tick_ms)?;
                    }
                    Ok(())
                }));
            match ticked {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    failure = Some(TickFailure::Error(error));
                    break;
                }
                Err(payload) => {
                    failure = Some(TickFailure::Panic(payload));
                    break;
                }
            }
            self.apply_publish();
            self.publish_load_view();
            self.wait_for_next_server_tick(&mut next, paused);
        }
        if let Some(failure) = failure {
            let finished = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.finish_server_capture();
            }));
            if let Err(payload) = finished {
                log::error!(
                    "Sunrise: the server capture could not be closed after the crash: {}",
                    super::dedicated::panic_message(payload.as_ref())
                );
            }
            return match failure {
                TickFailure::Error(error) => Err(error),
                TickFailure::Panic(payload) => std::panic::resume_unwind(payload),
            };
        }
        Ok(())
    }

    pub fn tick_integrated_unpaused(
        &mut self,
        flags: &Flags,
        smoothed_tick_ms: &mut f32,
    ) -> Result<()> {
        let cpu = sunrise_level::spans::thread_cpu();
        let started = Instant::now();
        self.tick_server_measured(false)?;
        // MinecraftServer.tickServer's tick times, for /tick query.
        self.common.clock.record(started.elapsed());
        // Parallel Tick's Auto decides here, between ticks, how the next one runs.
        let elapsed_ms = started.elapsed().as_secs_f32() * 1000.0;
        let on_cpu = sunrise_level::spans::thread_cpu().zip(cpu);
        sunrise_sim::parallel::after_tick(
            elapsed_ms,
            on_cpu.map(|(end, start)| (end - start).as_secs_f32() * 1000.0),
        );
        *smoothed_tick_ms = *smoothed_tick_ms * 0.8 + elapsed_ms * 0.2;
        flags
            .smoothed_tick_ms
            .store(smoothed_tick_ms.to_bits(), Ordering::Relaxed);
        if let (Some(terrain), Ok(mut terrains)) = (self.common.end.locator(), flags.terrain.lock())
        {
            terrains.insert(self.common.dim.id(), terrain);
        }
        self.follow_distances(flags);
        Ok(())
    }

    /// After the level of `dim` ticked: the window's debug entries' server
    /// lines, when they want them from this level.
    pub(in crate::game::server_side) fn publish_server_debug(&self, dim: Dim) {
        let Some(serving) = &self.server.serving else {
            return;
        };
        let want = serving.flags.debug_at.lock().ok().and_then(|w| *w);
        if let Some((at, feet)) = want
            && at == dim
        {
            let lines = self.server_debug_lines(feet);
            if let Ok(mut debug) = serving.flags.debug.lock() {
                *debug = (debug.0.wrapping_add(1), lines);
            }
        }
    }

    /// The integrated server's world open: `run_integrated`'s segment
    /// from `load_world` through `settle_player_spawn`, whole so its failures are
    /// one open failure (`WorldOpenFailed`, the engine's own error text).
    pub(in crate::game::server_side) fn integrated_open(
        &mut self,
        launch: &Launch,
        flags: &Arc<Flags>,
        tasks: Receiver<OwnerRequest>,
        opening: Opening,
    ) -> Result<(Pipe, String)> {
        if let Ok(dir) = std::env::current_dir() {
            let (cache, warnings) =
                sunrise_server_files::user_cache::UserCache::load(dir.join("usercache.json"));
            for warning in warnings {
                log::warn!("{warning}");
            }
            self.server.user_cache = Some(cache);
            self.native_configs_server_start(&dir.join("saves").join(&launch.folder), false);
        }
        let prepared = self.prepare_early_game_rules(opening.world?)?;
        self.native_server_about_to_start(false);
        self.native_lifecycle_open_started();
        // `load_world`, with the world `serve` started preparing.
        startup::time("server.wait_rest", || self.common.join_world_rest())?;
        sunrise_level::spans::startup::arm("world.opening");
        let open_step = startup::step("server.open_world");
        self.finish_open_world(prepared)?;
        drop(open_step);
        if let Some(sync) = opening.sync {
            startup::time("server.sync_adopt", || sync.adopt(&self.pack_files()));
        }
        let saved = Box::new(self.common.level.clone());
        // IntegratedServer runs the server mods; the window the client mods.
        startup::time("server.start_mods", || self.start_mods());
        self.refresh_loop_context()?;
        // Math.max(2, options.renderDistance): past vanilla's 32 when the
        // window's render distance is (Sunrise's options go to 64).
        let view_distance = self.server.settings.render_distance.max(2);
        let config = HubConfig {
            allow_flight: true,
            default_game_mode: self.common.level.mode(),
            // Only the owner until it is opened to the LAN.
            guest_commands: false,
            force_game_mode: false,
            hardcore: self.common.level.hardcore,
            view_distance,
            simulation_distance: self.server.settings.simulation_distance.max(2),
            dimension: self.common.dim.id().to_owned(),
            levels: self.server.level_shapes.clone(),
            level_seeds: self.server.levels.0.runtime.seeds(),
            seed: self.common.level.seed,
            max_players: MAX_PLAYERS,
            // No player of its own: the hub runs without a host.
            integrated: true,
        };
        let owner = launch.owner;
        let published = flags.clone();
        let net = NetConfig {
            // No network until it is opened to the LAN; the memory connection
            // is not compressed (`Connection.isMemoryConnection`).
            listen: false,
            online: sunrise_sim::lan::OnlineMode::lan(
                launch.online_auth,
                launch.session_verifier.clone(),
            ),
            gate: Some(Arc::new(move |address, uuid, _| {
                refused(
                    uuid == owner,
                    published.published.load(Ordering::Relaxed) != 0,
                    address == sunrise_sim::lan::conn::MEMORY_ADDRESS,
                )
                .then(|| translatable("multiplayer.disconnect.not_whitelisted", &[]))
            })),
            levels: self.server.level_shapes.clone(),
            owner_profile: launch.owner_profile.clone(),
            // `IntegratedServer.getSingleplayerProfile`: the window's own player logs in
            // through memory as the account's profile, whatever the hello says.
            singleplayer_profile: self
                .server
                .singleplayer_profile
                .as_ref()
                .map(|profile| (profile.name.clone(), profile.uuid)),
            // IntegratedServer.initServer: the MOTD names the owner and the world.
            ..NetConfig::lan(
                0,
                format!("{} - {}", launch.player, self.level.name),
                MAX_PLAYERS,
            )
        };
        let settings = LanSettings {
            port: 0,
            guest_commands: false,
            force_game_mode: false,
        };
        startup::time("server.lan_host", || self.lan_host(settings, net, config))?;
        self.server.settings.render_distance =
            sunrise_sim::lan::hub::effective_view_distance(view_distance);
        self.server.streaming.radius = self.server.settings.render_distance;
        let commands = self.common.allow_commands();
        let pipe = {
            let host = self
                .server
                .lan
                .as_mut()
                .ok_or("the integrated server did not start")?;
            // The owner is an operator when the world allows commands
            // (`IntegratedPlayerList`, `MinecraftServer.getProfilePermissions`).
            host.hub.ops.insert(owner, if commands { 4 } else { 0 });
            host.hub.singleplayer_owner = Some(owner);
            host.owner = Some(Owner {
                uuid: owner,
                saved: Some(saved),
            });
            host.hub.lan.connect_local()?
        };
        self.server.serving = Some(Serving {
            flags: flags.clone(),
            tasks,
            owner,
            pending: None,
            previous_simulation_distance: 0,
            load: LoadPhase::default(),
        });
        // MinecraftServer.prepareLevels: the saved tickets' chunks load; the
        // player's chunks come after.
        let view = &flags.load;
        view.start(Stage::LoadInitialChunks, 0);
        startup::time("server.prepare_levels", || self.prepare_levels())?;
        view.finish(Stage::LoadInitialChunks);
        startup::time("server.settle_spawn", || self.settle_player_spawn())?;
        startup::time("server.initial_save", || {
            if mod_api::__private::enabled_listened(mod_api::HookId::WorldSaveStart) {
                self.begin_world_save(true)?;
            }
            self.save_world_after_players(false)
        })?;
        Ok((pipe, self.common.level.name.clone()))
    }

    /// `MinecraftServer.stopServer`: the players are saved and sent away,
    /// then the world is saved.
    pub(in crate::game::server_side) fn stop_integrated(&mut self, crashed: bool) -> Result<()> {
        // Abort background terrain generation/decoration on shutdown, matching vanilla behavior.
        self.server.streaming.abort_generation();
        self.server.levels.0.abort_parked_generation();
        // The close request: lifecycle observation stops.
        self.common.native_lifecycle.begin_close();
        self.finish_server_save(false, crashed)
    }
}

/// The server thread's side.
impl crate::game::ServerGame<'_> {
    /// IntegratedServer.teardownPublishedState, also reached when a shutdown
    /// callback unwinds before the normal stop path can retire the server.
    pub(in crate::game::server_side) fn stop_serving(&mut self) {
        if let Some(s) = self.server.serving.take() {
            s.flags.published.store(0, Ordering::Relaxed);
        }
    }

    /// Whether this game is an integrated server's.
    pub(in crate::game) fn serving(&self) -> bool {
        self.server.serving.is_some()
    }
    /// The window's agent runs a command as single player's console does
    /// (`as_console`, as without a server thread): as the world's owner, in
    /// the owner's level, at the owner permission level. The feedback goes to
    /// the owner's chat and is returned, with the command's outcome
    /// (`recording_command_outcome`; None when nothing ran).
    pub(in crate::game::server_side) fn owner_console_command(
        &mut self,
        command: &str,
    ) -> (Vec<String>, Option<(bool, i32)>) {
        let Some(owner) = self.server.serving.as_ref().map(|s| s.owner) else {
            return (Vec::new(), None);
        };
        let Some(host) = self.server.lan.as_ref() else {
            return (Vec::new(), None);
        };
        let mut out = Vec::new();
        let mut outcome = None;
        if let Some(g) = host.hub.guests.iter().find(|g| g.uuid == owner && g.placed) {
            let conn = g.conn;
            // Commands.performCommand runs in the player's level.
            let (back, theirs) = (self.common.dim, super::Dim::from_id(&g.dimension));
            let entered = theirs == back || self.enter_level(theirs).is_ok();
            if entered
                && let Some((lines, ran)) = self.with_guest_swap(conn, back, |game| {
                    let command = command.strip_prefix('/').unwrap_or(command).to_owned();
                    game.recording_command_outcome(|game| {
                        game.as_console(|g| {
                            g.server.chat_capture = Some(Vec::new());
                            // A command a native mod registered goes to the mods
                            // first, as the owner's own chat does (the console is
                            // level 4, so every registered usage is allowed).
                            if g.perform_native_command(conn, 4, command.trim(), false)
                                .is_none()
                            {
                                g.run_command(&command);
                            }
                            g.server.chat_capture.take().unwrap_or_default()
                        })
                    })
                })
            {
                outcome = ran;
                if let Some(host) = self.server.lan.as_mut() {
                    host.hub.sync_all_effects();
                }
                // A teleport into another level happens before its feedback is
                // sent, and what the command earned is announced before it too.
                self.run_level_teleports();
                self.announce_guest_advancements();
                for line in lines {
                    out.push(line.plain());
                    if let Some(host) = self.server.lan.as_ref() {
                        host.hub.send_to(
                            conn,
                            &sunrise_protocol::packets::play::Clientbound::SystemChat(
                                sunrise_protocol::packets::play::SystemChat {
                                    content: sunrise_sim::lan::data::component_nbt(
                                        &line.component(),
                                    ),
                                    overlay: false,
                                },
                            ),
                        );
                    }
                }
                self.send_admin_feedback();
            }
            if theirs != back
                && let Err(e) = self.enter_level(back)
            {
                log::error!("could not reopen {}: {e}", back.id());
            }
        }
        (out, outcome)
    }

    /// Run `f` inside the placed single-player owner guest's turn and dimension.
    pub(in crate::game::server_side) fn in_owner_guest_turn<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> T,
    ) -> std::result::Result<T, String> {
        let owner = self
            .server
            .serving
            .as_ref()
            .ok_or_else(|| "no integrated server owner".to_owned())?
            .owner;
        let host = self
            .server
            .lan
            .as_ref()
            .ok_or_else(|| "no active integrated server host".to_owned())?;
        let guest = host
            .hub
            .guests
            .iter()
            .find(|g| g.uuid == owner && g.placed)
            .ok_or_else(|| "owner player is not present in the world".to_owned())?;
        let conn = guest.conn;
        let (back, theirs) = (self.common.dim, super::Dim::from_id(&guest.dimension));
        if theirs != back {
            self.enter_level(theirs).map_err(|e| e.to_string())?;
        }
        let out = self
            .with_guest_swap(conn, back, f)
            .ok_or_else(|| "owner player guest turn unavailable".to_owned());
        if theirs != back
            && let Err(e) = self.enter_level(back)
        {
            log::error!("could not reopen {}: {e}", back.id());
        }
        out
    }

    /// Run `f` with the owner's level in place (the server holds one per
    /// dimension), then put back the level that was.
    pub(in crate::game::server_side) fn in_owner_level<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let owner = self.server.serving.as_ref().map(|s| s.owner);
        let dim = self.server.lan.as_ref().and_then(|h| {
            let g = h.hub.guests.iter().find(|g| Some(g.uuid) == owner)?;
            Some(super::Dim::from_id(&g.dimension))
        });
        let back = dim.and_then(|d| self.enter_command_level(d));
        let out = f(self);
        if let Some(back) = back {
            self.enter_command_level(back);
        }
        out
    }

    /// The window's requests, between two ticks.
    pub(in crate::game::server_side) fn run_tasks(&mut self) {
        self.drain_owner_tasks(Self::serve_owner_request);
    }

    #[inline(always)]
    fn drain_owner_tasks(&mut self, mut dispatch: impl FnMut(&mut Self, OwnerRequest)) {
        let requests: Vec<OwnerRequest> = match &self.server.serving {
            Some(s) => s.tasks.try_iter().collect(),
            None => return,
        };
        for request in requests {
            dispatch(self, request);
            self.apply_publish();
        }
    }
    /// Whether anyone but a single-player owner can join this world: any hub of a client or
    /// a dedicated server, an integrated server's only once published to LAN.
    pub(in crate::game::server_side) fn open_to_others(&self) -> bool {
        self.server.lan.is_some() && (!self.serving() || self.published_port().is_some())
    }
    /// `IntegratedServer.getPort` while published.
    pub fn published_port(&self) -> Option<u16> {
        let s = self.server.serving.as_ref()?;
        Some(s.flags.published.load(Ordering::Relaxed)).filter(|&p| p != 0)
    }
    /// `IntegratedServer.publishServer`: listen on `settings.port` (0 picks
    /// one) and announce the world. The socket is bound at once so a busy
    /// port fails here; the hub starts listening once it is at hand.
    pub fn publish(&mut self, settings: LanSettings) -> std::result::Result<u16, String> {
        if self.common.options.developer_single_player {
            return Err(sunrise_game_data::lang::get(
                "sunrise.developer_offline.single_player",
            ));
        }
        let Some(s) = self.server.serving.as_mut() else {
            return Err("not an integrated server".into());
        };
        if s.flags.published.load(Ordering::Relaxed) != 0 {
            return Err("already published".into());
        }
        let listener = Lan::bind(None, settings.port).map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        log::info!("Published LAN server on port {port}");
        s.flags.published.store(port, Ordering::Relaxed);
        s.pending = Some(Publish::Open(listener, LanSettings { port, ..settings }));
        self.apply_publish();
        Ok(port)
    }
    /// `IntegratedServer.unpublishServer`: stop listening and send every
    /// player but the owner away. False when it was not published.
    pub fn unpublish(&mut self) -> bool {
        let Some(s) = self.server.serving.as_mut() else {
            return false;
        };
        if s.flags.published.swap(0, Ordering::Relaxed) == 0 {
            return false;
        }
        s.pending = Some(Publish::Close);
        self.apply_publish();
        true
    }
    /// IntegratedServer.tickServer: the server follows the window's render
    /// and simulation distances when they change.
    pub(in crate::game::server_side) fn follow_distances(&mut self, flags: &Flags) {
        let Some(old_view) = self.server_view_distance() else {
            return;
        };
        let view = flags.render_distance.load(Ordering::Relaxed).max(2);
        if view != old_view {
            log::info!("Changing view distance to {view}, from {old_view}");
            self.set_view_distance(view);
        }
        let Some(serving) = self.server.serving.as_mut() else {
            return;
        };
        let simulation = flags.simulation_distance.load(Ordering::Relaxed).max(2);
        let old_simulation = serving.previous_simulation_distance;
        if simulation != old_simulation {
            log::info!("Changing simulation distance to {simulation}, from {old_simulation}");
            serving.previous_simulation_distance = simulation;
            self.set_simulation_distance(simulation);
        }
    }

    pub(in crate::game::server_side) fn apply_publish(&mut self) {
        if self.server.lan.is_none() {
            return;
        }
        let Some(s) = self.server.serving.as_mut() else {
            return;
        };
        let (Some(pending), owner) = (s.pending.take(), s.owner) else {
            return;
        };
        let allow_commands = self.common.allow_commands();
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        match pending {
            Publish::Open(listener, settings) => {
                if let Err(e) = host.hub.lan.listen_on(listener, true) {
                    log::error!("Sunrise: Open to LAN: {e}");
                }
                host.settings = settings;
                host.hub.config.force_game_mode = settings.force_game_mode;
                // IntegratedServer.setGuestCommandAccess, then updatePermissions.
                host.hub.allow_commands = allow_commands;
                host.hub.config.guest_commands = settings.guest_commands;
                self.resend_commands_filtered();
            }
            Publish::Close => {
                host.hub.lan.stop_listening();
                log::info!(
                    "Unpublishing integrated server (was on port {})",
                    host.settings.port
                );
                let others: Vec<u32> = host
                    .hub
                    .guests
                    .iter()
                    .filter(|g| g.uuid != owner)
                    .map(|g| g.conn)
                    .collect();
                for conn in others {
                    host.hub.kick(
                        conn,
                        translatable("multiplayer.disconnect.server_shutdown", &[]),
                    );
                }
            }
        }
    }
    /// The world options screen's state, read on the server.
    pub(in crate::game::server_side) fn server_options(&self) -> ServerOptions {
        let owner = self.server.serving.as_ref().map(|s| s.owner);
        let host = self.server.lan.as_ref();
        let personal_mode = host
            .and_then(|h| h.hub.guests.iter().find(|g| Some(g.uuid) == owner))
            .map_or(self.common.level.mode(), |g| g.game_mode);
        let published = self.published_port();
        ServerOptions {
            default_mode: self.common.level.mode(),
            personal_mode,
            cheats: self.common.allow_commands(),
            difficulty: self.common.ui.difficulty,
            hardcore: self.common.level.hardcore,
            difficulty_locked: self.common.level.difficulty_locked,
            port: published,
            guest_commands: published.is_some() && host.is_some_and(|h| h.settings.guest_commands),
            force: published.is_none() || host.is_some_and(|h| h.settings.force_game_mode),
        }
    }
    /// `IntegratedServer.setPersonalGameType`: the owner's game mode.
    pub(in crate::game) fn set_owner_mode(&mut self, mode: i32) {
        let Some(owner) = self.server.serving.as_ref().map(|s| s.owner) else {
            return;
        };
        if let Some(host) = self.server.lan.as_mut()
            && let Some(conn) = host
                .hub
                .guests
                .iter()
                .find(|g| g.uuid == owner)
                .map(|g| g.conn)
        {
            host.hub
                .set_game_mode(conn, mode, &self.common.world, Some(self.common.collision));
        }
    }
    /// `IntegratedServer.setWorldAllowCommands`: the owner is an operator
    /// while the world allows commands; everyone's level is sent again.
    pub(in crate::game) fn owner_allow_commands(&mut self, allow: bool) {
        let Some(owner) = self.server.serving.as_ref().map(|s| s.owner) else {
            return;
        };
        let published = self.published_port().is_some();
        if let Some(host) = self.server.lan.as_mut() {
            host.hub.allow_commands = allow;
            let level = if allow { 4 } else { 0 };
            host.hub.ops.insert(owner, level);
            if published {
                host.hub.config.guest_commands = host.settings.guest_commands;
            }
            self.resend_commands_filtered();
        }
    }
}

/// The server thread: a game of its own over the shared resources.
#[expect(clippy::too_many_arguments)]
pub fn serve_with_ticket_activation(
    res: &Arc<Resources>,
    pack: PackResources,
    options: Options,
    settings: SimSettings,
    launch: Launch,
    flags: Arc<Flags>,
    tasks: Receiver<OwnerRequest>,
    ready: &Sender<Ready>,
    profile_resolver: ProfileResolverFactory,
    ticket_activation: &'static mod_api::__private::game::TicketActivation,
) -> Result<()> {
    serve_inner(
        res,
        pack,
        options,
        settings,
        launch,
        flags,
        tasks,
        ready,
        profile_resolver,
        Some(ticket_activation),
    )
}

#[cfg(any(test, feature = "test-support"))]
#[expect(clippy::too_many_arguments)]
pub fn serve(
    res: &Arc<Resources>,
    pack: PackResources,
    options: Options,
    settings: SimSettings,
    launch: Launch,
    flags: Arc<Flags>,
    tasks: Receiver<OwnerRequest>,
    ready: &Sender<Ready>,
    profile_resolver: ProfileResolverFactory,
) -> Result<()> {
    serve_inner(
        res,
        pack,
        options,
        settings,
        launch,
        flags,
        tasks,
        ready,
        profile_resolver,
        None,
    )
}

#[expect(clippy::too_many_arguments)]
fn serve_inner(
    res: &Arc<Resources>,
    pack: PackResources,
    options: Options,
    settings: SimSettings,
    launch: Launch,
    flags: Arc<Flags>,
    tasks: Receiver<OwnerRequest>,
    ready: &Sender<Ready>,
    profile_resolver: ProfileResolverFactory,
    ticket_activation: Option<&'static mod_api::__private::game::TicketActivation>,
) -> Result<()> {
    // The world prepares, and the registries the server sends are read, on
    // helper threads while this one builds the server's own Game: none of
    // them reads what another makes (`integrated_open` takes them up).
    let world = WorldRequest::new(
        &launch.folder,
        launch.name.as_deref(),
        launch.seed,
        launch.creative,
        launch.create.as_ref(),
        launch.rule_registry.clone(),
    )
    // `IntegratedServer.setSingleplayerProfile`: the account this world belongs to
    // (an older world's offline owner moves to it at open).
    .and_then(|mut request| {
        request.chunk_reports = flags.reports.clone();
        WorldLoader::start(
            request.owned_by(&launch.player, launch.owner),
            res.clone(),
            pack.clone(),
            options.clone(),
            flags.upgrade.clone(),
        )
    });
    let opening = Opening {
        world,
        sync: preload_sync_data(&res.core.item_data, res.shared_items(), res.biomes()),
    };
    let mut server = match startup::time("server.game_new", || {
        server_game(res, pack, options, settings, profile_resolver)
    }) {
        Ok(server) => server,
        Err(e) => {
            // The world's session lock is let go before the failure is told.
            if let Ok(world) = opening.world {
                drop(world.finish());
            }
            return Err(e);
        }
    };
    server.server.ticket_activation = ticket_activation;
    server.run_integrated(launch, flags, tasks, ready, opening)
}

/// What `serve` starts beside the server's own Game.
pub(in crate::game) struct Opening {
    pub(in crate::game) world: Result<WorldLoader>,
    pub(in crate::game) sync: Option<SyncPreload>,
}

pub fn run_component_harness_on_server(
    res: Arc<Resources>,
    pack: PackResources,
    mut options: Options,
    settings: SimSettings,
    fixture: ComponentHarness,
    profile_resolver: ProfileResolverFactory,
) -> Result<()> {
    options.execution_role = ExecutionRole::IntegratedServer;
    options.dedicated = Some(Vec::new());
    let result = std::thread::Builder::new()
        .name(sunrise_level::crash::INTEGRATED_SERVER_THREAD.into())
        .stack_size(server_thread_stack())
        .spawn(move || {
            let run = || -> Result<()> {
                let mut server = server_game(&res, pack, options, settings, profile_resolver)?;
                match fixture {
                    ComponentHarness::Dispenser(spec) => server.dispense_fixture(&spec),
                    ComponentHarness::Sculk(spec) => server.sculk_fixture(&spec),
                    ComponentHarness::HotPaths(output) => server.hot_paths(&output),
                    ComponentHarness::BoatAuthorityAttacks(dir, out) => {
                        server.run_boat_authority_attacks(&dir, &out)
                    }
                }
            };
            run().map_err(|e| e.to_string())
        })?
        .join()
        .map_err(|_| "the component harness server panicked")?;
    result.map_err(Into::into)
}

pub fn server_game(
    res: &Arc<Resources>,
    pack: PackResources,
    options: Options,
    settings: SimSettings,
    profile_resolver: ProfileResolverFactory,
) -> Result<crate::game::ServerGame<'_>> {
    crate::game::ServerGame::new(res, pack, options, settings, None, profile_resolver)
}

/// Who the integrated server turns away: anyone but the owner until it is opened to the LAN.
/// The owner joins only through memory. A host without a Microsoft account opens the world in
/// offline mode, where a
/// player's UUID comes from their name alone, so a LAN or friends player naming themselves after
/// the host would otherwise get the host's UUID, kick the host and take their operator rights and
/// player data (vanilla always authenticates LAN guests, so there it can't happen).
pub(in crate::game::server_side) fn refused(owner: bool, published: bool, memory: bool) -> bool {
    if owner { !memory } else { !published }
}

#[cfg(any())]
impl Integrated {
    /// The window's handle on a server fixture, with no server thread behind it.
    #[cfg(test)]
    pub(in crate::game) fn fixture_for(serving: &Serving) -> Self {
        let (tasks, _receiver) = channel();
        Self {
            flags: serving.flags.clone(),
            thread: None,
            emergency_save: None,
            tasks,
            world_name: "test".into(),
            save_folder: "test".into(),
            rule_registry: Default::default(),
        }
    }

    /// Tests only: `f` on the server thread between two ticks, its answer awaited as `ask` does.
    #[cfg(any())]
    pub fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut crate::game::ServerGame<'_>) -> T + Send + 'static,
    ) -> Option<T> {
        self.ask(|reply| {
            OwnerRequest::InTest(Box::new(move |g| {
                let _ = reply.send(f(g));
            }))
        })
    }
    /// Tests only: `f` on the server thread between two ticks, not awaited.
    #[cfg(any())]
    pub fn post(&self, f: impl FnOnce(&mut crate::game::ServerGame<'_>) + Send + 'static) -> bool {
        self.tell(OwnerRequest::InTest(Box::new(f)))
    }
    /// An integrated server whose thread already finished, just before the window disconnects.
    #[cfg(any(test, feature = "test-support"))]
    pub fn stopped_for_test() -> Self {
        Self {
            flags: Arc::new(Flags::default()),
            thread: None,
            emergency_save: None,
            tasks: channel().0,
            world_name: "Test world".into(),
            save_folder: "test_world".into(),
            rule_registry: Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla()),
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[cfg(test)]
    /// The integrated server keeps its stack floor and honors a larger
    /// `RUST_MIN_STACK` override (#2430).
    #[test]
    fn integrated_server_stack_honors_its_floor_and_larger_overrides() {
        let host_stack = 8 * 1024 * 1024;
        assert_eq!(super::stack_for(None), host_stack);
        assert_eq!(
            super::stack_for(Some("1048576")),
            host_stack,
            "smaller values keep the integrated server floor"
        );
        assert_eq!(super::stack_for(Some("16777216")), 16 * 1024 * 1024);
        assert_eq!(
            super::stack_for(Some("lots")),
            host_stack,
            "an unparsed value is ignored"
        );
    }

    /// Observe both teardownPublishedState and the dropped server task queue.
    pub fn install_serving(
        game: &mut crate::game::ServerGame<'_>,
        published: bool,
    ) -> impl Fn() -> bool + Send + Sync + 'static {
        let flags = Arc::new(Flags::default());
        flags
            .published
            .store(u16::from(published), Ordering::Relaxed);
        let (tasks, receiver) = channel();
        game.server.serving = Some(Serving {
            flags: flags.clone(),
            tasks: receiver,
            owner: 0,
            pending: None,
            previous_simulation_distance: 0,
            load: LoadPhase::default(),
        });
        move || {
            // Pregen is off in tests, so a probe the server does read changes nothing.
            flags.published.load(Ordering::Relaxed) == 0
                && tasks.send(OwnerRequest::SetPregen(false)).is_err()
        }
    }

    #[cfg(test)]
    #[test]
    fn emergency_save_waits_for_finished_and_times_out() {
        let flags = Arc::new(Flags::default());
        let saving = flags.clone();
        let server = std::thread::spawn(move || {
            let _finished =
                sunrise_gameplay_common::game::integrated_link::Finished::new(saving.clone());
            while !saving.stop.load(Ordering::Relaxed) {
                std::thread::yield_now();
            }
        });
        let id = server.thread().id();
        assert!(flags.emergency_save(id, Duration::from_secs(2)));
        assert!(flags.finished.load(Ordering::Acquire));
        server.join().unwrap();
        assert!(flags.emergency_save(id, Duration::ZERO));
        let flags = Flags::default();
        let started = Instant::now();
        assert!(!flags.emergency_save(id, Duration::from_millis(20)));
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(flags.stop.load(Ordering::Relaxed));
        let flags = Flags::default();
        assert!(flags.emergency_save(std::thread::current().id(), Duration::ZERO));
        assert!(!flags.stop.load(Ordering::Relaxed));
    }

    #[cfg(test)]
    #[test]
    fn the_owner_joins_only_through_memory_and_lan_guests_only_when_published() {
        for published in [false, true] {
            assert!(
                !refused(true, published, true),
                "the owner joins through memory"
            );
            assert!(
                refused(true, published, false),
                "the owner's identity is reserved"
            );
            for memory in [false, true] {
                assert_eq!(refused(false, published, memory), !published);
            }
        }
    }
}

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::mods::ModGameExt1 as _;

use sunrise_gameplay_common::game::native_configs::NativeConfigsGameExt1 as _;

use sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycleCommonGameExt1 as _;
