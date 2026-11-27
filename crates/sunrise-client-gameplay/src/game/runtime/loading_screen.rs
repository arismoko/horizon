//! The loading screen while the resources load (`Loading`), the window's app before the game
//! takes over (`App`), and the checks a start makes first: missing or damaged prepared files,
//! an unclean last exit, a fullscreen the window system could not show.
use super::*;
#[cfg(feature = "client")]
use crate::game::client_side::{
    launch_options_client::is_client_start, resource_tables_client::UiResources,
    window_handler::follow_keymap,
};
use sunrise_gameplay_common::game::integrated_link::{
    ProfileResolverFactory, RunComponentHarness, Serve,
};
use sunrise_sim::lan::SessionVerifier;

/// The window system ended the event loop with a failure (on Linux, a compositor dropping the
/// connection after a protocol error) while fullscreen was on: the next start opens windowed, so a
/// fullscreen the system can't show doesn't crash every launch (#2904).
#[cfg(feature = "client")]
pub(in crate::game) fn leave_fullscreen_after(
    result: &std::result::Result<(), EventLoopError>,
    settings: &mut Settings,
) {
    if matches!(result, Err(EventLoopError::ExitFailure(_))) && settings.fullscreen {
        settings.fullscreen = false;
        settings.save(&sunrise_gameplay_common::game::options_path());
        log::warn!(
            "the window system closed the game in fullscreen; the next start opens windowed"
        );
    }
}

#[cfg(feature = "client")]
pub fn run_event_loop(mut game: crate::game::ClientGame<'_>) -> Result<()> {
    // Minecraft.onGameLoadFinished: a start that made it this far is clean.
    if is_client_start(&game.options) && game.options.headless {
        game.client.settings.file.set(
            "startedCleanly",
            sunrise_client::options_file::V::Bool(true),
        );
        game.client
            .settings
            .save(&sunrise_gameplay_common::game::options_path());
    }
    if game.options.headless {
        // No display to open winit's loop on: the game's own loop.
        game.run_headless();
        // narrator::start ran for it too (options.txt's narrator setting).
        narrator::stop();
        let result = write_report(&game).and_then(|()| game.error.take().map_or(Ok(()), Err));
        if is_client_start(&game.options) {
            game.native_client_stopped();
            game.native_game_shutting_down();
        }
        return result;
    }
    let result = EventLoop::new()?.run_app(&mut game);
    narrator::stop();
    leave_fullscreen_after(&result, &mut game.client.settings);
    let result = result
        .map_err(Into::into)
        .and_then(|()| write_report(&game))
        .and_then(|()| game.error.take().map_or(Ok(()), Err));
    if is_client_start(&game.options) {
        game.native_client_stopped();
        game.native_game_shutting_down();
    }
    result
}

/// The loading screen's state while the worker loads the resources.
#[cfg(feature = "client")]
pub struct Loading {
    pub serve_integrated: Serve,
    pub run_component_harness: RunComponentHarness,
    pub profile_resolver: ProfileResolverFactory,
    pub session_verifier: SessionVerifier,
    pub window: Option<Arc<Window>>,
    pub renderer: Option<Renderer>,
    /// The opened GPU device's `max_texture_dimension_2d`, shared with the background loader.
    pub max_atlas_size: Arc<AtomicU32>,
    /// The boot screen's atlas, until the pack's GUI atlas replaces it.
    pub boot: sunrise_client::hud::HudAtlas,
    pub gui: Option<UiResources>,
    pub settings: Settings,
    pub fullscreen: sunrise_client::window::FullscreenState,
    pub video_modes: sunrise_client::window::VideoModeList,
    pub options: Option<Options>,
    pub vanilla_root: PathBuf,
    pub pack_error: Option<String>,
    pub started: Instant,
    pub panorama_rotation: sunrise_client::gfx::panorama::Rotation,
    pub progress: Progress,
    /// The bar as drawn, easing toward `progress` (`LoadingOverlay.currentProgress`).
    pub bar: f32,
    pub receiver: Receiver<LoaderMsg>,
    pub loaded: Option<std::result::Result<Loaded, super::resource_loader::LoaderFailure>>,
    /// The loader's `Done` arrived: its channel closing afterwards is expected.
    pub delivered: bool,
    pub handle: Option<JoinHandle<()>>,
    pub shown: bool,
    /// When the menu bundle arrived (`set_ui`), and whether a title frame with
    /// it was presented since: the world renderer waits for that frame
    /// (`ready_for_handoff`).
    pub ui_at: Option<Instant>,
    pub title_drawn: bool,
    pub menu: startup_menu::StartupMenu,
    pub error: Option<Box<dyn std::error::Error>>,
}
/// How long the handoff waits for the first title frame before building the
/// world renderer anyway (a hidden window may never be asked to redraw).
pub(in crate::game) const TITLE_FRAME_WAIT: Duration = Duration::from_secs(1);
#[cfg(feature = "client")]
impl Loading {
    /// Reconstitute the main-thread error without erasing declaration refusals.
    pub fn take_worker_result(&mut self) -> Option<Result<Loaded>> {
        self.loaded.take().map(|result| result.map_err(Into::into))
    }

    /// The world renderer is built on the main thread, so building it before
    /// the title is on the screen holds the title back by that build. Wait for
    /// one title frame first, unless a world is waiting to open.
    pub fn ready_for_handoff(&self) -> bool {
        self.title_drawn
            || self.menu.pending.is_some()
            || self.ui_at.is_none_or(|at| at.elapsed() >= TITLE_FRAME_WAIT)
    }
    pub(in crate::game::client_side) fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        // The Window constructor: created fullscreen when options.txt or --fullscreen says so.
        let fullscreen =
            self.settings.fullscreen || self.options.as_ref().is_some_and(|o| o.fullscreen);
        let size = self
            .options
            .as_ref()
            .map_or([1280, 720], |o| o.window_size)
            .map(f64::from);
        self.fullscreen = sunrise_client::window::FullscreenState::new(fullscreen);
        let attributes = sunrise_client::window::with_icon(
            Window::default_attributes().with_title(super::window_title::base()),
        );
        // Wayland names the window by its app id, which the horizon.desktop entry matches.
        #[cfg(target_os = "linux")]
        let attributes = winit::platform::wayland::WindowAttributesExtWayland::with_name(
            attributes, "sunrise", "sunrise",
        );
        // `--offscreen` renders into a texture: its window would only show white.
        let window_step = startup::step("window.create");
        let window = match event_loop.create_window(
            attributes
                .with_inner_size(winit::dpi::LogicalSize::new(size[0], size[1]))
                .with_fullscreen(fullscreen.then_some(winit::window::Fullscreen::Borderless(None)))
                .with_visible(!self.options.as_ref().is_some_and(|o| o.offscreen)),
        ) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.error = Some(e.into());
                event_loop.exit();
                return;
            }
        };
        sunrise_client::window::settle_created_fullscreen(&window, fullscreen);
        #[cfg(target_os = "windows")]
        if let Some(hwnd) = hwnd(&window) {
            sunrise_threads::system_menu::suppress(hwnd);
        }
        drop(window_step);
        // The renderer takes seconds to build and winit's window sits white
        // until the first frame presents, so paint the boot screen on it and
        // keep repainting it as the console's lines arrive; the wgpu frames
        // redraw the same design when they start. GDI is unsafe:
        // sunrise-threads holds it (this crate forbids unsafe code).
        #[cfg(target_os = "windows")]
        // In the fonts given to `menu_style`; with none, the window stays as it is.
        let boot_paint_stop = hwnd(&window)
            .zip(sunrise_client::menu_style::get().fonts)
            .map(|(hwnd, fonts)| {
                let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let repaint_stop = stop.clone();
                std::thread::spawn(move || {
                    while !repaint_stop.load(std::sync::atomic::Ordering::Relaxed) {
                        sunrise_threads::boot_paint::paint(
                            hwnd,
                            fonts.semibold,
                            fonts.mono,
                            &sunrise_level::logging::recent_lines(),
                        );
                        std::thread::sleep(std::time::Duration::from_millis(300));
                    }
                });
                stop
            });
        let offscreen = self.options.as_ref().is_some_and(|o| o.offscreen);
        let boot_step = startup::step("renderer.boot");
        match loading_renderer(
            window.clone(),
            offscreen,
            Some(&self.boot),
            None,
            None,
            self.settings.sunrise.api,
        ) {
            Ok(r) => {
                self.max_atlas_size.store(
                    r.device.limits().max_texture_dimension_2d,
                    Ordering::Relaxed,
                );
                self.renderer = Some(r);
            }
            Err(e) => {
                #[cfg(target_os = "windows")]
                if let Some(stop) = &boot_paint_stop {
                    stop.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                self.error = Some(e);
                event_loop.exit();
                return;
            }
        }
        drop(boot_step);
        // The wgpu frames take the window over: stop the boot paint thread
        // before the first one presents.
        #[cfg(target_os = "windows")]
        if let Some(stop) = &boot_paint_stop {
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }

        self.window = Some(window);
    }
    pub(in crate::game::client_side) fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: WindowId,
        mut event: WindowEvent,
    ) {
        if self.window.as_ref().is_none_or(|w| w.id() != id) {
            return;
        }
        follow_keymap(&mut event);
        if let Err(error) = self.menu_event(event_loop, event) {
            self.error = Some(error);
            event_loop.exit();
        }
    }
    /// The menu bundle arrived: rebuild the loading screen with the real atlas
    /// and title panorama.
    pub fn set_ui(
        &mut self,
        mut gui: UiResources,
        root: PathBuf,
        source: sunrise_assets::resource_source::ResourceSource,
    ) {
        log::info!(
            "Sunrise: loading the title artwork after {} ms",
            self.started.elapsed().as_millis()
        );
        if let Some(renderer) = &self.renderer
            && let Err(error) = gui.stitch_for_device(
                &source,
                &self.vanilla_root,
                renderer.device.limits().max_texture_dimension_2d,
            )
        {
            self.error = Some(error);
            return;
        }
        let additional = self.options.as_ref().map_or_else(Vec::new, |options| {
            sunrise_gameplay_common::game::pack_discovery::data_packs(&options.mods)
        });
        self.menu
            .set_root(root, &self.settings, additional.into(), &gui.splashes);
        self.gui = Some(gui);
        self.title_drawn = false;
        sunrise_client::ui_theme::reset_atlas();
        self.apply_menu_settings();
        let Some(window) = self.window.clone() else {
            return;
        };
        let offscreen = self.options.as_ref().is_some_and(|o| o.offscreen);
        if let Some(renderer) = &mut self.renderer {
            renderer.release_surface();
        }
        let bundle = self.gui.as_ref().expect("the menu bundle was just set");
        let _step = startup::step("renderer.title_artwork");
        match loading_renderer(
            window,
            offscreen,
            Some(&bundle.atlas),
            Some(&bundle.panorama),
            self.renderer.as_ref(),
            self.settings.sunrise.api,
        ) {
            Ok(r) => {
                self.renderer = Some(r);
            }
            Err(e) => self.error = Some(e),
        }
        // The title frame's wait starts once its renderer is built.
        self.ui_at = Some(Instant::now());
    }
    pub(in crate::game::client_side) fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // The loop may run once more after exit(): keep the first error, the
        // one that ended loading.
        if self.error.is_some() {
            event_loop.exit();
            return;
        }
        if sunrise_threads::stop_requests() > 0 {
            // No world is open yet: nothing to save.
            event_loop.exit();
            return;
        }
        if let Err(error) = self.poll_menu_resources() {
            self.error = Some(error);
            event_loop.exit();
            return;
        }
        self.poll_menu_realms(event_loop);
        if let Some(w) = &self.window {
            if self.options.as_ref().is_some_and(|o| o.offscreen) {
                self.draw();
            } else {
                w.request_redraw();
            }
        }
        event_loop.set_control_flow(ControlFlow::Poll);
    }
    pub(in crate::game::client_side) fn draw(&mut self) {
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return;
        };
        let mut batch = HudBatch::new(size, self.settings.gui_scale_inputs());
        self.draw_menu(&mut batch);
        self.menu.frame_drawn((batch.width, batch.height));
        let mut frame = loading_frame(&batch);
        frame.yaw = self.panorama_rotation.update(
            self.started.elapsed().as_secs_f32(),
            self.settings.file.double("panoramaScrollSpeed") as f32,
        );
        if let Some(renderer) = &mut self.renderer {
            match renderer.render(&frame, None) {
                Ok(true) if self.gui.is_some() && self.menu.pending.is_none() => {
                    self.title_drawn = true;
                }
                Ok(_) => {}
                Err(e) => log::warn!("Sunrise: loading screen frame failed: {e}"),
            }
        }
        if !self.shown {
            self.shown = true;
            startup::mark("loading_screen.shown");
            log::info!(
                "Sunrise: loading screen shown after {} ms",
                self.started.elapsed().as_millis()
            );
        }
    }
    #[cfg(feature = "client")]
    pub(in crate::game::client_side) fn finish(
        &mut self,
        mut loaded: Loaded,
    ) -> Result<Option<crate::game::ClientGame<'static>>> {
        let handoff_at = Instant::now();
        let mut options = self.options.take().ok_or("loading already finished")?;
        options.resources = loaded.root.clone();
        // StartupMenu owns world selection until it can hand it to Game.
        options.world = None;
        options.server = None;
        let gui = self.gui.take().ok_or("missing menu bundle")?;
        let client_pack = loaded.client_pack.ok_or("missing client pack")?;
        let client_res = loaded.client_res.ok_or("missing client resource slot")?;
        let window = self.window.clone().ok_or("window closed")?;
        // Keep the loader's device and swap in the real textures in one step.
        if let Some(renderer) = &mut self.renderer {
            renderer.release_surface();
        }
        let renderer_at = Instant::now();
        let mut pick_hdr = shader_packs::PickHdr::default();
        let (renderer, required) = startup::time("renderer.world", || {
            build_renderer(
                window.clone(),
                options.offscreen,
                &loaded.pack,
                &client_pack,
                &gui,
                self.renderer.as_ref().map_or(
                    sunrise_client::gfx::RendererDevice::New,
                    sunrise_client::gfx::RendererDevice::Reuse,
                ),
                loaded.renderer_cpu,
                self.settings.sunrise.api,
                self.settings.sunrise.translucent_batching,
                self.settings.vsync,
                &self.settings.sunrise,
                shader_packs::startup_pack_shape(&self.settings, &mut pick_hdr),
                self.renderer
                    .as_ref()
                    .map(|r| r.display_reading())
                    .unwrap_or_default(),
                self.settings.texture_atlas_settings(),
            )
        })?;
        log::info!(
            "Sunrise: world renderer ready after {} ms",
            self.started.elapsed().as_millis()
        );
        let renderer_ms = renderer_at.elapsed().as_millis();
        // Audio is created on the main thread, after the core is ready.
        let audio = if !options.execution_role.is_client() {
            Audio::silent(&loaded.pack.source)
        } else {
            Audio::with_device(&loaded.pack.source, &self.settings.sound_device)
        };
        // The menu read its language from the prepared root alone (`set_root`) and
        // `load_resources` skips it on this path: read it again with the mods' layers, in the
        // language picked in the menu.
        sunrise_game_data::lang::set(sunrise_game_data::lang::Language::load(
            &[&loaded.pack.source],
            &sunrise_game_data::lang::code(),
        ));
        let pack_error = loaded.pack_error.or_else(|| self.pack_error.take());
        // The game borrows its shared resources for the process's lifetime,
        // like the single copy an integrated server would share.
        let res: &'static Arc<Resources> = Box::leak(Box::new(loaded.resources));
        let game_at = Instant::now();
        let game_step = startup::step("handoff.start_game");
        let Some(mut game) = start_client_game(
            res,
            client_res,
            loaded.pack,
            client_pack,
            gui,
            options,
            pack_error,
            audio,
            self.vanilla_root.clone(),
            self.started,
            loaded.rest_loader,
            self.settings.clone(),
            loaded.game_cpu,
            std::mem::take(&mut loaded.client_applies),
            loaded.entity_models,
            Some((window, renderer)),
            self.serve_integrated,
            self.run_component_harness,
            self.session_verifier.clone(),
            self.profile_resolver,
        )?
        else {
            return Ok(None);
        };
        drop(game_step);
        let game_ms = game_at.elapsed().as_millis();
        game.client.fullscreen = self.fullscreen;
        game.client.panorama_rotation = self.panorama_rotation;
        *game.client.native_shaders.pick_hdr.get_mut() = pick_hdr;
        self.menu.handoff(&mut game);
        game.apply_settings();
        if let Err(error) = required {
            game.selection_reload_failed(&error);
        }
        game.previous = Instant::now();
        // Minecraft.onGameLoadFinished: a start that made it this far is clean.
        game.client.settings.file.set(
            "startedCleanly",
            sunrise_client::options_file::V::Bool(true),
        );
        game.client
            .settings
            .save(&sunrise_gameplay_common::game::options_path());
        log::info!(
            "Sunrise: startup handoff {} ms (renderer {} ms, start_game {} ms)",
            handoff_at.elapsed().as_millis(),
            renderer_ms,
            game_ms
        );
        Ok(Some(game))
    }
}

/// The application: the loading screen until the resources are ready, then
/// the game itself, on the same event loop and window.
#[cfg(feature = "client")]
pub(in crate::game) enum App {
    Loading(Box<Loading>),
    Ready(Box<crate::game::ClientGame<'static>>),
}
#[cfg(feature = "client")]
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        match self {
            App::Loading(loading) => loading.resumed(event_loop),
            App::Ready(game) => game.resumed(event_loop),
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        match self {
            App::Loading(loading) => loading.window_event(event_loop, id, event),
            App::Ready(game) => game.window_event(event_loop, id, event),
        }
    }
    fn device_event(&mut self, event_loop: &ActiveEventLoop, id: DeviceId, event: DeviceEvent) {
        if let App::Ready(game) = self {
            game.device_event(event_loop, id, event);
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        enum Step {
            Waiting,
            Ready(Box<crate::game::ClientGame<'static>>),
            Done,
            Failed(Box<dyn std::error::Error>),
        }
        let step = match self {
            App::Ready(game) => {
                game.about_to_wait(event_loop);
                return;
            }
            App::Loading(loading) => {
                loading.about_to_wait(event_loop);
                if loading.error.is_some() {
                    event_loop.exit();
                    return;
                }
                match if loading.menu.reload_packs || !loading.ready_for_handoff() {
                    None
                } else {
                    loading.take_worker_result()
                } {
                    None => Step::Waiting,
                    Some(Err(e)) => Step::Failed(e),
                    Some(Ok(loaded)) => match loading.finish(loaded) {
                        Ok(Some(game)) => Step::Ready(Box::new(game)),
                        Ok(None) => Step::Done,
                        Err(e) => Step::Failed(e),
                    },
                }
            }
        };
        match step {
            Step::Waiting => {}
            Step::Ready(game) => *self = App::Ready(game),
            Step::Done => event_loop.exit(),
            Step::Failed(e) => {
                if let App::Loading(loading) = self {
                    loading.error = Some(e);
                }
                event_loop.exit();
            }
        }
    }
    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        if let App::Ready(game) = self {
            game.exiting(event_loop);
        }
    }
}

/// `Minecraft.<init>`: `startedCleanly` left false means the previous start
/// died before `onGameLoadFinished`. Vanilla then leaves fullscreen
/// ("resetting fullscreen mode") so a start that died on the display mode can
/// come back; the resource packs are rolled back only when their load fails
/// (`rollbackResourcePacks`), not here. True when the last start was unclean.
pub fn unclean_start_recovery(settings: &mut Settings) -> bool {
    // Vanilla's field defaults true: only an explicit `false` is an unclean
    // start; an options file that never had the key is not one.
    let unclean = matches!(
        settings.file.get("startedCleanly"),
        Some(sunrise_client::options_file::V::Bool(false))
    );
    if unclean {
        settings.fullscreen = false;
        settings.file.fullscreen_resolution = None;
    }
    unclean
}

use sunrise_gameplay_common::game::native_hooks::game_shutting_down::GameHooks as _;
