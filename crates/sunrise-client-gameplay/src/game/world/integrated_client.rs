use super::WorldIcon;
use std::{
    sync::{
        Arc,
        atomic::Ordering,
        mpsc::{Receiver, RecvTimeoutError, Sender, channel, sync_channel},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use sunrise_game_data::Result;
use sunrise_gameplay_common::game::Dim;
use sunrise_gameplay_common::game::execution_role::ExecutionRole;
use sunrise_gameplay_common::game::integrated_link::{
    CALL_TIMEOUT, Finished, Flags, Launch, Ready, server_thread_stack,
};
use sunrise_gameplay_common::game::owner_link::{OwnerRequest, Reply};
use sunrise_level::level_loading::ServerLoadView;
use sunrise_protocol::crypto::offline_uuid;
use sunrise_sim::lan::OwnerProfile;
use sunrise_sim::world_options::WorldOptions;

/// The window's handle on its server thread.
pub struct Integrated {
    pub flags: Arc<Flags>,
    pub thread: Option<JoinHandle<()>>,
    pub emergency_save: Option<sunrise_level::crash::EmergencySave>,
    pub tasks: Sender<OwnerRequest>,
    /// The world's name (`level.dat`'s `LevelName`), for the pause menu.
    pub world_name: String,
    pub save_folder: String,
    pub rule_registry: Arc<sunrise_game_data::gamerules::RuleRegistry>,
}
impl Integrated {
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

    /// Queue every change, even if the currently published tree still has the requested state:
    /// an earlier update may be waiting for a busy server thread.
    pub(in crate::game) fn queue_pregen_option(&self, enabled: bool) {
        self.tell(OwnerRequest::SetPregen(enabled));
    }
    /// The generator of the server's level in `dimension`, once it has run.
    pub fn terrain(&self, dimension: &str) -> Option<Arc<sunrise_sim::vanillagen::VanillaTerrain>> {
        self.flags.terrain.lock().ok()?.get(dimension).cloned()
    }
    /// The server's load progress and chunk status map.
    pub fn load_view(&self) -> &Arc<ServerLoadView> {
        &self.flags.load
    }
    /// The world's upgrade while the server opens it.
    pub fn upgrade(&self) -> &sunrise_world::datafix::progress::UpgradeProgress {
        &self.flags.upgrade
    }
    pub fn smoothed_tick_ms(&self) -> f32 {
        f32::from_bits(self.flags.smoothed_tick_ms.load(Ordering::Relaxed))
    }
    /// Asks for the debug entries' server lines at `at`, or stops asking.
    pub(in crate::game) fn request_debug(&self, at: Option<(Dim, [i32; 3])>) {
        if let Ok(mut want) = self.flags.debug_at.lock() {
            *want = at;
        }
    }
    /// The debug entries' server lines of the last tick.
    pub(in crate::game) fn debug(
        &self,
    ) -> (u64, sunrise_gameplay_common::game::debug_lines::ServerDebug) {
        self.flags.debug.lock().map(|d| *d).unwrap_or_default()
    }
    /// `MinecraftServer.isCurrentlySaving`.
    pub fn is_saving(&self) -> bool {
        self.flags.saving.load(Ordering::Relaxed)
    }
    /// `IntegratedServer.tickServer` skips the world while the game is paused.
    pub fn set_paused(&self, paused: bool) {
        self.flags.paused.store(paused, Ordering::Relaxed);
    }
    /// The window's render and simulation distances and its Entity Distance
    /// option, for the server to follow.
    pub fn set_distances(&self, render: i32, simulation: i32, entity: f64) {
        self.flags.render_distance.store(render, Ordering::Relaxed);
        self.flags
            .simulation_distance
            .store(simulation, Ordering::Relaxed);
        self.flags
            .entity_distance
            .store(entity.to_bits(), Ordering::Relaxed);
    }
    /// `IntegratedServer.getPort` while `isPublished`.
    pub fn published(&self) -> Option<u16> {
        Some(self.flags.published.load(Ordering::Relaxed)).filter(|&p| p != 0)
    }
    /// `executeBlocking`: `None` once the server stopped or after `CALL_TIMEOUT` (the log tells which).
    pub(in crate::game) fn ask<T: Send + 'static>(
        &self,
        build: impl FnOnce(Reply<T>) -> OwnerRequest,
    ) -> Option<T> {
        if self.flags.stop.load(Ordering::Relaxed) || self.flags.finished.load(Ordering::Acquire) {
            return None;
        }
        let (tx, rx) = sync_channel(1);
        self.tasks.send(build(tx)).ok()?;
        let deadline = Instant::now() + CALL_TIMEOUT;
        loop {
            match rx.recv_timeout(Duration::from_millis(25)) {
                Ok(answer) => return Some(answer),
                Err(RecvTimeoutError::Disconnected) => return None,
                Err(RecvTimeoutError::Timeout) => {
                    if self.flags.stop.load(Ordering::Relaxed)
                        || self.flags.finished.load(Ordering::Acquire)
                    {
                        return None;
                    }
                    if Instant::now() >= deadline {
                        log::warn!(
                            "Sunrise: the server thread did not answer within {} s (running: {}, \
                             autosaving: {}, smoothed tick {:.1} ms)",
                            CALL_TIMEOUT.as_secs(),
                            !self.stopped(),
                            self.is_saving(),
                            self.smoothed_tick_ms()
                        );
                        return None;
                    }
                }
            }
        }
    }
    /// `execute`: send a request without waiting for it. False once the server has stopped.
    pub(in crate::game) fn tell(&self, request: OwnerRequest) -> bool {
        self.tasks.send(request).is_ok()
    }
    /// `IntegratedServer.halt(true)`: returns once the world is saved and let go.
    pub fn stop(mut self) -> Result<()> {
        self.halt()
    }
    /// Stop on the server thread; the window polls completion while drawing.
    pub fn request_stop(&self) {
        self.flags.stop.store(true, Ordering::Relaxed);
    }
    pub fn stopped(&self) -> bool {
        self.thread
            .as_ref()
            .is_none_or(|thread| thread.is_finished())
    }
    pub fn halt(&mut self) -> Result<()> {
        self.request_stop();
        self.emergency_save.take();
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| "the integrated server panicked")?;
        }
        match self
            .flags
            .failure
            .lock()
            .map_err(|_| "server status poisoned")?
            .as_ref()
        {
            Some(error) => Err(error.clone().into()),
            None => Ok(()),
        }
    }
}
impl Drop for Integrated {
    fn drop(&mut self) {
        if let Err(error) = self.halt() {
            log::error!("Sunrise: integrated server shutdown: {error}");
        }
    }
}

/// An integrated server loading its world: the window keeps drawing its
/// loading screen and joins once the server is ready, as vanilla's client
/// does while its server prepares (`Minecraft.doWorldLoad`), instead of
/// waiting for it frozen for the whole load.
pub struct Starting {
    pub handle: Integrated,
    pub ready: Receiver<Ready>,
    pub player: String,
    /// The save's folder in `saves/`, for the world icon once joined.
    pub folder: String,
    /// `Minecraft.doWorldLoad`'s `newWorld`: the loading screen stays 500 ms
    /// once the level is ready.
    pub new_world: bool,
}

impl crate::game::ClientGame<'_> {
    /// Only the owner window changes its integrated server's command tree.
    pub(in crate::game::client_side) fn sync_pregen_option(&self) {
        let Some(server) = &self.client.integrated else {
            return;
        };
        let enabled = self.common.options.pregen || self.client.settings.extras.pregen;
        // Check on the server after earlier updates so a later toggle wins without blocking.
        server.queue_pregen_option(enabled);
    }
    pub(in crate::game) fn client_has_singleplayer_server(&self) -> bool {
        self.client.remote.is_none() || self.client.integrated.is_some()
    }
    pub(in crate::game) fn client_is_published(&self) -> bool {
        self.client
            .integrated
            .as_ref()
            .is_some_and(|s| s.published().is_some())
    }
    #[inline(always)]
    #[allow(dead_code)]
    pub(in crate::game) fn integrated_tick_loop_client(&mut self, started: Instant) -> f32 {
        self.client
            .debug_charts
            .log_tick(started, started.elapsed());
        self.client.debug_charts.smoothed_tick_ms
    }
    /// `Minecraft.doWorldLoad`: start the server thread on the world; the
    /// window joins it through memory once it is ready (`poll_integrated_start`).
    pub fn open_integrated(
        &mut self,
        folder: &str,
        name: Option<&str>,
        seed: Option<i64>,
        creative: bool,
        create: Option<&WorldOptions>,
    ) -> Result<()> {
        self.reset_server_debug_lines();
        self.client_logging_out();
        sunrise_command::vanilla::set_sunrise_commands(
            self.common.options.pregen || self.client.settings.extras.pregen,
        );
        let auth = sunrise_client::auth::current();
        let player = auth
            .as_ref()
            .map_or_else(|| self.common.options.player_name(), |a| a.name.clone());
        let new_world = create.is_some();
        let rule_registry = create.map_or_else(
            || Arc::new(sunrise_gameplay_common::game::api_values::game_rule_registry()),
            |options| options.rules.registry().clone(),
        );
        // `IntegratedServer`'s constructor: `setSingleplayerProfile(minecraft.getGameProfile())`
        // — the signed-in account's own UUID. Only an offline start names its player by the
        // offline profile. The owner is the world's player, so a world without
        // `singleplayer_uuid` (a Realms download, `renameAndDropPlayer`) finds the owner's
        // `players/data/<uuid>.dat` instead of starting a new player.
        let owner = auth
            .as_ref()
            .map_or_else(|| offline_uuid(&player), |a| a.uuid);
        let launch = Launch {
            online_auth: auth.is_some(),
            session_verifier: self.client.session_verifier.clone(),
            rule_registry: rule_registry.clone(),
            folder: folder.to_owned(),
            name: name.map(str::to_owned),
            seed,
            creative,
            create: create.cloned(),
            owner,
            player: player.clone(),
            owner_profile: auth.as_ref().map(|a| {
                let id = a.uuid;
                Arc::new(OwnerProfile::new(move || {
                    sunrise_client::skins::fetch_own_properties(id)
                        .into_iter()
                        .map(
                            |(name, value, signature)| sunrise_core::profile::AccountProperty {
                                name,
                                value,
                                signature,
                            },
                        )
                        .collect()
                }))
            }),
        };
        let mut options = self.common.options.clone();
        // A server without a console: no local player, no window, no control socket.
        // Compatibility for gameplay checks outside startup/world loading.
        options.dedicated = Some(Vec::new());
        options.execution_role = ExecutionRole::IntegratedServer;
        options.control = None;
        options.benchmark = None;
        options.frames = None;
        options.world_frames = None;
        options.report = None;
        options.server = None;
        options.world = None;
        self.common.join_world_rest()?;
        let (res, pack, settings) = (
            self.common.res.clone(),
            self.common.pack.clone(),
            self.client.settings.sim_settings(),
        );
        sunrise_sim::parallel::set_setting(settings.parallel_tick);
        let flags = Arc::new(Flags {
            reports: self.client.client_reports.0.clone(),
            ..Flags::default()
        });
        flags
            .render_distance
            .store(settings.render_distance, Ordering::Relaxed);
        flags
            .simulation_distance
            .store(settings.simulation_distance, Ordering::Relaxed);
        flags
            .entity_distance
            .store(settings.entity_distance.to_bits(), Ordering::Relaxed);
        let (ready_tx, ready) = channel();
        let (tasks, server_tasks) = channel();
        let server_flags = flags.clone();
        let serve = self.client.serve_integrated;
        let profile_resolver = self.client.profile_resolver;
        let run_server = move |_: &()| {
            let _finished = Finished::new(server_flags.clone());
            // MinecraftServer.spin's Thread.setPriority(8).
            sunrise_threads::favour_current();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                serve(
                    &res,
                    pack,
                    options,
                    settings,
                    launch,
                    server_flags.clone(),
                    server_tasks,
                    &ready_tx,
                    profile_resolver,
                )
            }))
            .unwrap_or_else(|payload| {
                sunrise_level::crash::report_server_panic(&*payload);
                Err("the integrated server panicked".into())
            });
            if let Err(e) = result {
                log::error!("Sunrise: integrated server: {e}");
                *server_flags.failure.lock().unwrap() = Some(e.to_string());
                let _ = ready_tx.send(Err(e.to_string()));
            }
        };
        // Production installations are process-wide. A test installation is scoped
        // to its owning thread; the real server thread must inherit that snapshot.
        #[cfg(any(test, feature = "test-support"))]
        let run_server = mod_api::testing::inherit_mods(run_server);
        let thread = std::thread::Builder::new()
            .name(sunrise_level::crash::INTEGRATED_SERVER_THREAD.into())
            .stack_size(server_thread_stack())
            .spawn(move || run_server(&()))?;
        let emergency_flags = flags.clone();
        let server_id = thread.thread().id();
        let emergency_save = sunrise_level::crash::on_emergency_save(move || {
            emergency_flags.emergency_save(server_id, Duration::from_secs(30));
        });
        let handle = Integrated {
            emergency_save: Some(emergency_save),
            flags,
            thread: Some(thread),
            tasks,
            world_name: String::new(),
            save_folder: folder.to_owned(),
            rule_registry,
        };
        self.client.integrated_starting = Some(Starting {
            handle,
            ready,
            player,
            folder: folder.to_owned(),
            new_world,
        });
        // Harnesses, control scripts and runs without a window go on with the world open.
        if self.client.window.0.is_none() || super::synchronous_startup(&self.common.options) {
            while self.client.integrated_starting.is_some() {
                self.poll_integrated_start()?;
                if self.client.integrated_starting.is_some() {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
        Ok(())
    }
    /// The starting server's answer, polled each frame: joined once it is ready.
    pub(in crate::game::client_side) fn poll_integrated_start(&mut self) -> Result<()> {
        if self.client.integrated_starting.is_none() {
            return Ok(());
        }
        self.tick_load_progress();
        let Some(starting) = &self.client.integrated_starting else {
            return Ok(());
        };
        let (pipe, world_name, biomes) = match starting.ready.try_recv() {
            Ok(Ok(ready)) => ready,
            Err(std::sync::mpsc::TryRecvError::Empty) => return Ok(()),
            // FileFixerProgressScreen's Cancel: the upgrade stopped, back to the title.
            Ok(Err(_)) if starting.handle.upgrade().is_canceled() => {
                self.client.integrated_starting = None;
                self.client.perf.cancel_world_opening();
                return Ok(());
            }
            Ok(Err(e)) => {
                self.client.integrated_starting = None;
                return Err(e.into());
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.client.integrated_starting = None;
                return Err("the integrated server stopped".into());
            }
        };
        let Some(Starting {
            mut handle,
            player,
            folder,
            new_world,
            ..
        }) = self.client.integrated_starting.take()
        else {
            return Ok(());
        };
        handle.world_name = world_name;
        self.server_packs_world_pack(&folder);
        // Use the logical server's admitted table, including its saved-name IDs.
        self.world_biomes_base = Some(biomes.clone());
        self.world_biomes = Some(biomes);
        self.connect_local(pipe, &player)?;
        if let Some(r) = self.client.remote.as_mut() {
            // `new LevelLoadTracker(newWorld ? 500L : 0L)`.
            r.level_load = sunrise_client::level_loading::LevelLoadTracker::new(if new_world {
                sunrise_client::level_loading::LEVEL_LOAD_CLOSE_DELAY_MS
            } else {
                0
            });
        }
        // IntegratedServer.getWorldScreenshotFile: the client grabs the frame
        // the Select World list shows, and needs the save folder for it.
        self.client.world_icon = Some(WorldIcon {
            path: std::env::current_dir()?
                .join("saves")
                .join(&folder)
                .join("icon.png"),
            last_attempt: None,
        });
        self.reset_server_debug_lines();
        self.client.integrated = Some(handle);
        Ok(())
    }
    /// `LevelLoadingScreen.tick`'s easing of the server's progress, every 50 ms
    /// of wall time (the starting server has no client tick yet).
    pub(in crate::game::client_side) fn tick_load_progress(&mut self) {
        let view = match (&self.client.integrated_starting, &self.client.integrated) {
            (Some(s), _) => s.handle.load_view().clone(),
            (None, Some(i)) => i.load_view().clone(),
            _ => return,
        };
        let now = Instant::now();
        let at = self.client.loading_progress_at.get_or_insert(now);
        let steps = (now.duration_since(*at).as_millis() / 50).min(20) as u32;
        *at += Duration::from_millis(u64::from(steps) * 50);
        if now.duration_since(*at) > Duration::from_secs(1) {
            *at = now;
        }
        for _ in 0..steps {
            self.client.loading_progress = sunrise_client::level_loading::smooth(
                self.client.loading_progress,
                view.server_progress(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pregen_toggle_back_is_queued_while_server_is_busy() {
        let (tasks, waiting_server) = channel();
        let server = Integrated {
            emergency_save: None,
            flags: Arc::new(Flags::default()),
            thread: None,
            tasks,
            world_name: String::new(),
            save_folder: String::new(),
            rule_registry: Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla()),
        };
        // The receiver deliberately does not run either task between the updates. In particular,
        // the dispatcher can still show the old value when the owner toggles back.
        server.queue_pregen_option(true);
        server.queue_pregen_option(false);
        assert!(
            matches!(waiting_server.try_recv(), Ok(OwnerRequest::SetPregen(true))),
            "enable must reach the server"
        );
        assert!(
            matches!(
                waiting_server.try_recv(),
                Ok(OwnerRequest::SetPregen(false))
            ),
            "disable must follow it"
        );
        assert!(waiting_server.try_recv().is_err());
    }

    #[test]
    fn shutdown_request_does_not_join_a_server_still_saving() {
        let flags = Arc::new(Flags::default());
        let (release, wait) = channel();
        let (tasks, _) = channel();
        let server = Integrated {
            emergency_save: None,
            flags: flags.clone(),
            thread: Some(std::thread::spawn(move || wait.recv().unwrap())),
            tasks,
            world_name: String::new(),
            save_folder: String::new(),
            rule_registry: Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla()),
        };
        server.request_stop();
        assert!(flags.stop.load(Ordering::Relaxed));
        assert!(!server.stopped());
        release.send(()).unwrap();
        server.stop().unwrap();
    }

    #[test]
    fn shutdown_reports_a_failed_server_save() {
        let flags = Arc::new(Flags::default());
        let saving = flags.clone();
        let (tasks, _) = channel();
        let server = Integrated {
            emergency_save: None,
            flags,
            thread: Some(std::thread::spawn(move || {
                *saving.failure.lock().unwrap() = Some("disk write failed".into());
            })),
            tasks,
            world_name: String::new(),
            save_folder: String::new(),
            rule_registry: Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla()),
        };
        assert_eq!(server.stop().unwrap_err().to_string(), "disk write failed");
    }
}
