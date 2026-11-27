//! winit's `ApplicationHandler` for the game window: window and device events, shutting down,
//! and the idle step (`about_to_wait`).
#[cfg(feature = "client")]
use super::saving::ClientQuit;
#[cfg(feature = "client")]
use super::*;
#[cfg(feature = "client")]
#[cfg(feature = "client")]
use crate::game::client_side::key_codes::{glfw_button, glfw_modifiers};
#[cfg(feature = "client")]
use crate::game::client_side::launch_options_client::{is_client_start, time_limit};
#[cfg(feature = "client")]
use sunrise_client::window::PointerSample;
#[cfg(feature = "client")]
use winit::event::WindowEvent;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// A run that a person's stray key, click or mouse move would spoil: a screenshot sequence, a
    /// `SUNRISE_TICK_SCRIPT` scenario or a `record-replay`. While one runs, the keyboard, mouse
    /// and pad at this computer don't reach the game (`locks_out`, `poll_pads`), the window's
    /// focus stays as the run found it (`held_focus`) and its pointer grab stays inside the game
    /// (`internal_grab`).
    pub(in crate::game::client_side) fn scripted_run(&self) -> bool {
        self.client.screenshot_sequence.0.is_some()
            || self.client.tick_script.playing()
            || self.client.repro.0.replaying()
    }
}

/// A key, a modifier, a click, the wheel, the pointer or the input method. Resize and close are
/// not, so a person can still close the window. A modifier change counts: `reconcile` would
/// release a Shift or Ctrl the run holds.
#[cfg(feature = "client")]
fn is_input(event: &WindowEvent) -> bool {
    matches!(
        event,
        WindowEvent::KeyboardInput { .. }
            | WindowEvent::ModifiersChanged(_)
            | WindowEvent::MouseInput { .. }
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::CursorMoved { .. }
            | WindowEvent::Ime(_)
    )
}

/// A person's input during a scripted run is dropped. What the run itself sends (the control
/// socket, a replay, the game's own) still plays.
#[cfg(feature = "client")]
fn locks_out(origin: input_trace::InputOrigin, scripted: bool, event: &WindowEvent) -> bool {
    scripted && origin == input_trace::InputOrigin::User && is_input(event)
}

/// A person clicking into the window or away from it during a scripted run. A replay's look and
/// clicks act by the focus and the pointer grab, which that would change mid-run: the run keeps
/// the focus it started with, and the last change applies once it ends (`settle_after_run`).
#[cfg(feature = "client")]
fn held_focus(
    origin: input_trace::InputOrigin,
    scripted: bool,
    event: &WindowEvent,
) -> Option<bool> {
    match event {
        WindowEvent::Focused(focused) if scripted && origin == input_trace::InputOrigin::User => {
            Some(*focused)
        }
        _ => None,
    }
}

/// What a person's event does at an agent's window no one has clicked yet.
#[cfg(feature = "client")]
#[derive(Debug, PartialEq)]
enum Unclaimed {
    /// A mouse button pressed inside it: the window is theirs from this press on.
    Claims,
    /// Their keys, wheel, input method and button releases, and the focus the system gives the
    /// window or takes from it: the game the agent drives hears none of them.
    Dropped,
    /// Everything else, and whatever the agent or the game sends.
    Passes,
}

/// Only a press claims the window. The focus doesn't: Windows and X11 can hand it to a window
/// nobody clicked, and the person's keys and mouse would then steer the agent's game (#15449).
/// The pointer's position still passes, so the press that claims a menu lands where it points.
#[cfg(feature = "client")]
fn at_unclaimed(
    origin: input_trace::InputOrigin,
    unclaimed: bool,
    event: &WindowEvent,
) -> Unclaimed {
    if !unclaimed || origin != input_trace::InputOrigin::User {
        return Unclaimed::Passes;
    }
    match event {
        WindowEvent::MouseInput {
            state: ElementState::Pressed,
            ..
        } => Unclaimed::Claims,
        WindowEvent::CursorMoved { .. } => Unclaimed::Passes,
        WindowEvent::Focused(_) => Unclaimed::Dropped,
        _ if is_input(event) => Unclaimed::Dropped,
        _ => Unclaimed::Passes,
    }
}

/// The Shift, Ctrl or Alt a person holds at an agent's window no one has clicked yet. The game
/// doesn't hear of them, but winit tells of modifiers only as they change: the last state is
/// kept for the claim, where a Ctrl held since before the click would otherwise read as up.
#[cfg(feature = "client")]
fn held_modifiers(
    origin: input_trace::InputOrigin,
    unclaimed: bool,
    event: &WindowEvent,
) -> Option<winit::event::Modifiers> {
    match event {
        WindowEvent::ModifiersChanged(modifiers)
            if unclaimed && origin == input_trace::InputOrigin::User =>
        {
            Some(*modifiers)
        }
        _ => None,
    }
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// winit's `window_event`. The control socket and the pad menu call it too, with the
    /// input they make up.
    pub fn handle_window_event(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        id: WindowId,
        event: WindowEvent,
        origin: input_trace::InputOrigin,
    ) {
        self.handle_window_event_with_sample(event_loop, id, event, origin, None);
    }

    pub(in crate::game::client_side) fn handle_window_event_with_sample(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        id: WindowId,
        event: WindowEvent,
        origin: input_trace::InputOrigin,
        retained_sample: Option<PointerSample>,
    ) {
        self.handle_window_event_as(event_loop, id, event, origin, retained_sample);
    }

    pub(in crate::game::client_side) fn handle_window_event_as(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        id: WindowId,
        mut event: WindowEvent,
        origin: input_trace::InputOrigin,
        retained_sample: Option<PointerSample>,
    ) {
        // A headless game has no window: its events (the frame, the control socket's input)
        // carry `WindowId::dummy()`.
        let ours = match &self.client.window.0 {
            Some(w) => w.id() == id,
            None => self.common.options.headless,
        };
        if !ours {
            return;
        }
        // Flush drops before another event can change their receiving screen.
        if !matches!(event, WindowEvent::DroppedFile(_)) {
            self.fire_dropped_files();
        }
        // The window's state, not input: kept whoever's turn it is.
        track_window_state(
            origin,
            &event,
            &mut self.client.window_hovered,
            &mut self.client.window_occluded,
        );
        if let Some(modifiers) = held_modifiers(origin, self.client.unclaimed_window, &event) {
            self.client.unclaimed_modifiers = Some(modifiers);
        }
        if locks_out(origin, self.scripted_run(), &event) {
            return;
        }
        if let Some(focused) = held_focus(origin, self.scripted_run(), &event) {
            self.client.focus_after_run = Some(focused);
            return;
        }
        match at_unclaimed(origin, self.client.unclaimed_window, &event) {
            Unclaimed::Claims => self.claim_window(event_loop),
            Unclaimed::Dropped => return,
            Unclaimed::Passes => {}
        }
        follow_keymap(&mut event);
        // MacosUtil.setCtrlClickEmulatesRightClick: Control + left click is a right click.
        if let WindowEvent::MouseInput { state, button, .. } = &mut event
            && *button == MouseButton::Left
            && self.client.right_click_emulation.is_right_click(
                *state == ElementState::Pressed,
                self.client.modifiers.control_key(),
                cfg!(target_os = "macos") && self.client.settings.ctrl_click_emulates_right_click,
            )
        {
            *button = MouseButton::Right;
        }
        let cursor_sample = if let WindowEvent::CursorMoved { position, .. } = &event {
            Some(retained_sample.unwrap_or_else(|| {
                // Only the coordinate scale matters here, not the screen extent.
                let window = self.pointer_space([1, 1]);
                if origin == input_trace::InputOrigin::User {
                    window.os_sample(*position)
                } else {
                    window.physical_sample(*position)
                }
            }))
        } else {
            None
        };
        // Who caused it, before the event is handled: the trace wants the
        // window's own keys, never the ones the socket made up.
        self.trace_window_event(origin, &event, cursor_sample);
        if let WindowEvent::KeyboardInput { event, .. } = &event
            && let PhysicalKey::Code(key) = event.physical_key
        {
            if event.state == ElementState::Pressed {
                self.client.physical_keys.insert(key);
                sunrise_client::keybinds::learn_layout(event);
            } else if !self
                .client
                .screen_state
                .0
                .context_menu_input
                .holds_other_source(sunrise_client::held::Source::Keyboard, key)
            {
                self.client.physical_keys.remove(&key);
            }
        }
        if matches!(
            &event,
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            }
        ) && !self.input_made_pointer_event()
        {
            self.client.pad.0.keyboard_took_over = true;
        }
        if let WindowEvent::MouseInput { state, button, .. } = &event {
            if *state == ElementState::Pressed {
                self.client.physical_buttons.insert(*button);
                self.client.physical_presses += 1;
            } else {
                self.client.physical_buttons.remove(button);
            }
            // A container hook's press latch ends here, before any hook can cancel the release.
            self.client.container_owned_up = *state == ElementState::Released
                && !self.pad_synthetic_input()
                && self.container_button_up(*button);
        }
        let mouse = if !self.pad_synthetic_input()
            && let WindowEvent::MouseInput { state, button, .. } = &event
        {
            Some((
                glfw_button(*button),
                *state == ElementState::Pressed,
                glfw_modifiers(self.client.modifiers),
            ))
        } else {
            None
        };
        if let Some((button, pressed, modifiers)) = mouse {
            let mut canceled = false;
            self.native_mouse_button_pre(button, pressed, modifiers, &mut canceled);
            // This is release cleanup, not gameplay dispatch. Even a consumed release
            // must not leave a mod's mapping down; presses wait for the screen outcome.
            if let WindowEvent::MouseInput {
                state: ElementState::Released,
                button,
                ..
            } = &event
            {
                self.mod_mappings_button(*button, false);
            }
            if canceled {
                return;
            }
        }
        let resized = matches!(event, WindowEvent::Resized(_));
        let mouse_press_millis = (origin == input_trace::InputOrigin::User
            && mouse.is_some_and(|(_, pressed, _)| pressed))
        .then(|| self.millis());
        let title_settled = self.handle_window_event_inner_as(
            event_loop,
            event,
            cursor_sample,
            mouse_press_millis,
            origin,
        );
        if resized {
            self.notify_window_resize();
        }
        // Button receivers settle before their Post; other events settle here.
        if !title_settled {
            self.settle_title_mode();
        }
        if let Some((button, pressed, modifiers)) = mouse {
            self.native_mouse_button_post(button, pressed, modifiers);
        }
    }

    /// The loop ended: close the pads' output, save the world.
    pub fn shut_down(&mut self) {
        self.client.client_quit.set(ClientQuit::Stopping);
        self.close_client_tasks();
        if self.common.error.is_none()
            && let Some(renderer) = &mut self.client.renderer.0
        {
            renderer.drop_hdr_marker();
        }
        self.end_screenshot_sequence(Err("cancelled: the game is closing".into()));
        self.pad_close_output();
        if let Err(e) = self.save_for_exit() {
            log::warn!("Sunrise: failed to save the world on exit: {e}");
        }
        self.finish_profile();
        // The client's last moment: its world is left and saved, its options and
        // resources are still held. A dedicated server skips it, as `client_setup` does.
        if is_client_start(&self.common.options) {
            self.native_client_stopping();
        }
    }

    /// winit's `about_to_wait`: what happens between frames (stop requests, the closing
    /// save, world entry, the control socket, pads), then the next frame.
    pub fn idle(&mut self, event_loop: &dyn EventLoopCtl) {
        self.fire_dropped_files();
        let quit_requested = match self.client.client_quit.get() {
            ClientQuit::Running => sunrise_threads::stop_requests() > 0,
            ClientQuit::Requested => true,
            ClientQuit::Stopping => false,
        };
        if quit_requested {
            // Mod requests and stop signals take the same path as a window close.
            if let Err(error) = self.begin_closing_save(true) {
                self.common.error = Some(error);
                event_loop.exit();
                return;
            }
        }
        if self.client.pending_save.0.is_some() {
            match self.poll_closing_save() {
                Ok(true) => event_loop.exit(),
                Ok(false) => {
                    if let Some(window) = &self.client.window.0 {
                        window.request_redraw();
                    }
                    // Yield between polls even when disk I/O is slow.
                    event_loop.set_control_flow(ControlFlow::WaitUntil(
                        Instant::now() + Duration::from_millis(4),
                    ));
                }
                Err(error) => {
                    self.common.error = Some(error);
                    event_loop.exit();
                }
            }
            return;
        }
        if let Err(error) = sunrise_level::spans::time("world_entry", || self.resume_world_entry())
        {
            self.common.error = Some(error);
            event_loop.exit();
            return;
        }
        // The world a backup made before its upgrade opens (EditWorldScreen's callback).
        if let Some(folder) = self
            .client
            .title
            .0
            .as_mut()
            .and_then(|t| t.take_open_request())
            && let Err(error) = self.open_world(&folder, None, None, false, None)
        {
            self.common.error = Some(error);
            event_loop.exit();
            return;
        }
        // A Realm's address came back (GetServerDetailsTask hands over to ConnectScreen).
        if let Some(action) = self
            .client
            .title
            .0
            .as_mut()
            .and_then(|t| t.take_connect_request())
            && let Err(error) = self.title_action(action)
        {
            self.common.error = Some(error);
            event_loop.exit();
            return;
        }
        sunrise_level::spans::time("server_packs", || self.tick_server_packs());
        sunrise_level::spans::time("control", || self.poll_control(event_loop));
        if self.client.pending_save.0.is_some() {
            return;
        }
        sunrise_level::spans::time("pads", || self.poll_pads(event_loop));
        self.settle_after_run(event_loop);
        self.pause_if_inactive();
        if let Some(limit) = time_limit(&self.common.options)
            && self.client.started.elapsed() > limit
        {
            self.common.error = Some(format!("run timed out after {} s", limit.as_secs()).into());
            event_loop.exit();
            return;
        }
        if let Some(w) = &self.client.window.0 {
            if self.common.options.offscreen {
                // Offscreen frames don't wait for the window to be drawn: an
                // occluded window (a locked macOS session) gets no redraws.
                let id = w.id();
                self.handle_window_event(
                    event_loop,
                    id,
                    WindowEvent::RedrawRequested,
                    input_trace::InputOrigin::Game,
                );
            } else {
                w.request_redraw();
            }
        } else if self.common.options.headless {
            // The headless loop paces the frames itself (`run_headless`).
            self.handle_window_event(
                event_loop,
                WindowId::dummy(),
                WindowEvent::RedrawRequested,
                input_trace::InputOrigin::Game,
            );
            return;
        }
        // Consume the last frame's result: a hidden compositor may stop sending
        // redraws altogether, leaving a successful result stale indefinitely.
        event_loop.set_control_flow(event_loop::redraw_flow(
            &mut self.client.frame_drawn,
            self.common.options.offscreen,
            Instant::now(),
        ));
    }
}

#[cfg(feature = "client")]
impl ApplicationHandler for crate::game::ClientGame<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.client.window.0.is_some() {
            return;
        }
        debug_keys::warm_debug_cpu();
        // The Window constructor: created fullscreen when options.txt or --fullscreen says so.
        let fullscreen = self.client.settings.fullscreen || self.common.options.fullscreen;
        let size = self.common.options.window_size.map(f64::from);
        self.client.fullscreen = sunrise_client::window::FullscreenState::new(fullscreen);
        let attributes = sunrise_client::window::with_icon(
            Window::default_attributes().with_title(super::window_title::base()),
        );
        // Wayland names the window by its app id, which the horizon.desktop entry matches.
        #[cfg(target_os = "linux")]
        let attributes = winit::platform::wayland::WindowAttributesExtWayland::with_name(
            attributes, "sunrise", "sunrise",
        );
        // A game an agent started opens without the focus and
        // the person keeps their keyboard and pointer until they click it (`unclaimed_window`).
        // X11 activates it anyway, and Windows can too: the focus claims nothing.
        let agent_started = self.common.options.control.is_some();
        self.client.unclaimed_window = agent_started && !self.common.options.offscreen;
        // `--offscreen` renders into a texture: its window would only show white.
        let window_step = startup::step("window.create");
        let window = match event_loop.create_window(
            attributes
                .with_inner_size(winit::dpi::LogicalSize::new(size[0], size[1]))
                .with_fullscreen(fullscreen.then_some(winit::window::Fullscreen::Borderless(None)))
                .with_visible(!self.common.options.offscreen)
                .with_active(!agent_started),
        ) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.common.error = Some(e.into());
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
        let mut pending = self.client.pending_initial_models.0.take();
        let renderer = (|| {
            let Some(start) = &mut pending else {
                return self.new_renderer(window.clone());
            };
            let models = &mut start.models;
            let device = pollster::block_on(sunrise_client::gfx::Opened::for_window(
                &window,
                self.common.options.offscreen,
                self.client.settings.sunrise.api,
                models.client_pack.render.texture_layers,
            ))?;
            if models.gui.stitch_for_device(
                &models.pack.source,
                &self.common.vanilla_root(),
                device.max_atlas_size(),
            )? {
                self.client.particles.0.reload(
                    &models.pack.source,
                    &models.gui.atlas.sprites,
                    &models.gui.atlas.translucent,
                );
            }
            self.renderer_for(
                window.clone(),
                &models.pack,
                &models.client_pack,
                &models.gui,
                Some(device),
            )
        })();
        self.client.resize_state = Some(self.resize_state_of(window.inner_size()));
        self.client.window.0 = Some(window);
        if let Some(pending) = pending {
            match self.apply_pending_client_start(*pending, renderer.map(Some)) {
                Ok(true) => {
                    self.client.settings.file.set(
                        "startedCleanly",
                        sunrise_client::options_file::V::Bool(true),
                    );
                    self.client
                        .settings
                        .save(&sunrise_gameplay_common::game::options_path());
                }
                Ok(false) => {
                    event_loop.exit();
                    return;
                }
                Err(error) => {
                    self.common.error = Some(error);
                    event_loop.exit();
                    return;
                }
            }
        } else {
            match renderer {
                Ok(renderer) => {
                    self.set_renderer(renderer);
                    self.client.world_renderer_ready = true;
                }
                Err(error) => {
                    self.common.error = Some(error);
                    event_loop.exit();
                    return;
                }
            }
        }
        self.client.ime_allowed = false;
        self.client.ime_area = None;
        self.apply_settings();
        self.common.previous = Instant::now();
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        self.apply_pointer_grab_request();
        let key = if self.client.window.0.as_ref().is_some_and(|w| w.id() == id)
            && !self.scripted_run()
            && !self.client.unclaimed_window
            && let WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } = &event
        {
            Some((event.clone(), glfw_modifiers(self.client.modifiers)))
        } else {
            None
        };
        self.handle_window_event(event_loop, id, event, input_trace::InputOrigin::User);
        if let Some((event, modifiers)) = key {
            use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
            let name = match event.physical_key {
                PhysicalKey::Code(code) => sunrise_client::keybinds::key_name(code),
                PhysicalKey::Unidentified(_) => None,
            };
            let layout_key = sunrise_client::keybinds::layout_key_name(
                event.physical_key,
                &event.key_without_modifiers(),
            );
            let raw_code = sunrise_client::keybinds::raw_key_code(event.physical_key);
            let action = match (event.state, event.repeat) {
                (winit::event::ElementState::Released, _) => mod_api::KeyAction::Release,
                (_, false) => mod_api::KeyAction::Press,
                (_, true) => mod_api::KeyAction::Repeat,
            };
            self.key_input(name, layout_key, raw_code, action, modifiers);
        }
    }
    fn device_event(&mut self, event_loop: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        self.apply_pointer_grab_request();
        // Offscreen runs do not use native pointer motion.
        if self.client.pending_save.0.is_none()
            && !self.scripted_run()
            && !self.client.unclaimed_window
            && self.client.captured
            && self.client.focused
            && !self.common.options.offscreen
            && (self.client.screen == Screen::Playing || self.mod_screen_grabs_pointer())
            && let DeviceEvent::MouseMotion { delta: (x, y) } = event
        {
            if !self.mod_screen_grabs_pointer() && self.pad_radial_captures() {
                return;
            }
            if x != 0.0 || y != 0.0 {
                self.pad_yield_input(event_loop);
            }
            self.grabbed_pointer_motion([x, y]);
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.apply_pointer_grab_request();
        // Every event-loop pass, drawn or not: a window that stops redrawing (a lost
        // device, an occluded window) must still end after a server crash.
        self.exit_after_server_crash();
        self.idle(event_loop);
    }
    /// The application is ending. On macOS, Quit (Cmd+Q) ends it through
    /// applicationWillTerminate without a CloseRequested; vanilla's GLFW
    /// window turns it into a window close, so Minecraft.stop saves the world.
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.shut_down();
    }
}

#[cfg(feature = "client")]
fn track_window_state(
    origin: input_trace::InputOrigin,
    event: &WindowEvent,
    hovered: &mut bool,
    occluded: &mut bool,
) {
    match event {
        WindowEvent::CursorEntered { .. } => *hovered = true,
        WindowEvent::CursorLeft { .. } => *hovered = false,
        WindowEvent::CursorMoved { .. } if origin == input_trace::InputOrigin::User => {
            *hovered = true;
        }
        WindowEvent::Occluded(now) => *occluded = *now,
        _ => {}
    }
}

#[cfg(all(test, feature = "client"))]
mod tests {
    use super::*;
    use input_trace::InputOrigin;
    use winit::dpi::{PhysicalPosition, PhysicalSize};
    use winit::event::{Ime, Modifiers, TouchPhase};
    use winit::keyboard::ModifiersState;

    // A key's `KeyEvent` can't be made outside winit; these are the person's other inputs.
    fn inputs() -> Vec<WindowEvent> {
        let device_id = DeviceId::dummy();
        vec![
            WindowEvent::MouseInput {
                device_id,
                state: ElementState::Pressed,
                button: MouseButton::Left,
            },
            WindowEvent::MouseWheel {
                device_id,
                delta: MouseScrollDelta::LineDelta(0.0, 1.0),
                phase: TouchPhase::Moved,
            },
            WindowEvent::CursorMoved {
                device_id,
                position: PhysicalPosition::new(10.0, 20.0),
            },
            WindowEvent::ModifiersChanged(Modifiers::from(ModifiersState::empty())),
            WindowEvent::Ime(Ime::Commit("a".into())),
        ]
    }

    #[test]
    fn the_window_tracks_the_pointer_over_it_and_its_occlusion() {
        let device_id = DeviceId::dummy();
        let moved = WindowEvent::CursorMoved {
            device_id,
            position: PhysicalPosition::new(10.0, 20.0),
        };
        let (mut hovered, mut occluded) = (false, false);
        let mut track = |origin, event: &WindowEvent| {
            track_window_state(origin, event, &mut hovered, &mut occluded);
            (hovered, occluded)
        };
        // An agent's move says nothing of the person's pointer; theirs does.
        assert_eq!(track(InputOrigin::Agent, &moved), (false, false));
        assert_eq!(track(InputOrigin::User, &moved), (true, false));
        let left = WindowEvent::CursorLeft { device_id };
        assert_eq!(track(InputOrigin::User, &left), (false, false));
        let entered = WindowEvent::CursorEntered { device_id };
        assert_eq!(track(InputOrigin::User, &entered), (true, false));
        assert_eq!(
            track(InputOrigin::User, &WindowEvent::Occluded(true)),
            (true, true)
        );
        // Focus and other events change neither.
        assert_eq!(
            track(InputOrigin::User, &WindowEvent::Focused(false)),
            (true, true)
        );
        assert_eq!(
            track(InputOrigin::User, &WindowEvent::Occluded(false)),
            (true, false)
        );
    }

    #[test]
    fn a_person_is_locked_out_of_a_scripted_run_only() {
        for event in inputs() {
            assert!(locks_out(InputOrigin::User, true, &event), "{event:?}");
            assert!(!locks_out(InputOrigin::User, false, &event), "{event:?}");
        }
    }

    #[test]
    fn what_the_run_sends_still_plays() {
        for event in inputs() {
            assert!(!locks_out(InputOrigin::Agent, true, &event), "{event:?}");
            assert!(!locks_out(InputOrigin::Game, true, &event), "{event:?}");
        }
    }

    #[test]
    fn a_person_can_still_close_the_window() {
        for event in [
            WindowEvent::CloseRequested,
            WindowEvent::Resized(PhysicalSize::new(854, 480)),
        ] {
            assert!(!locks_out(InputOrigin::User, true, &event), "{event:?}");
            assert_eq!(
                held_focus(InputOrigin::User, true, &event),
                None,
                "{event:?}"
            );
        }
    }

    #[test]
    fn only_a_persons_press_claims_an_agents_window() {
        let device_id = DeviceId::dummy();
        let press = WindowEvent::MouseInput {
            device_id,
            state: ElementState::Pressed,
            button: MouseButton::Left,
        };
        assert_eq!(
            at_unclaimed(InputOrigin::User, true, &press),
            Unclaimed::Claims
        );
        // The focus alone claims nothing, and the game doesn't hear of it.
        for focused in [true, false] {
            let event = WindowEvent::Focused(focused);
            assert_eq!(
                at_unclaimed(InputOrigin::User, true, &event),
                Unclaimed::Dropped
            );
        }
        let release = WindowEvent::MouseInput {
            device_id,
            state: ElementState::Released,
            button: MouseButton::Left,
        };
        for event in inputs().into_iter().skip(1).chain([release]) {
            let want = match event {
                WindowEvent::CursorMoved { .. } => Unclaimed::Passes,
                _ => Unclaimed::Dropped,
            };
            assert_eq!(
                at_unclaimed(InputOrigin::User, true, &event),
                want,
                "{event:?}"
            );
        }
        for event in [
            WindowEvent::CloseRequested,
            WindowEvent::Resized(PhysicalSize::new(854, 480)),
        ] {
            assert_eq!(
                at_unclaimed(InputOrigin::User, true, &event),
                Unclaimed::Passes,
                "{event:?}"
            );
        }
    }

    #[test]
    fn a_persons_modifiers_are_kept_for_the_claim() {
        let shift = Modifiers::from(ModifiersState::SHIFT);
        let event = WindowEvent::ModifiersChanged(shift);
        assert_eq!(held_modifiers(InputOrigin::User, true, &event), Some(shift));
        assert_eq!(held_modifiers(InputOrigin::User, false, &event), None);
        assert_eq!(held_modifiers(InputOrigin::Agent, true, &event), None);
        assert_eq!(
            held_modifiers(InputOrigin::User, true, &WindowEvent::Focused(true)),
            None
        );
    }

    #[test]
    fn the_agent_still_drives_its_unclaimed_window() {
        let events = inputs().into_iter().chain([WindowEvent::Focused(true)]);
        for event in events {
            for origin in [InputOrigin::Agent, InputOrigin::Game] {
                assert_eq!(
                    at_unclaimed(origin, true, &event),
                    Unclaimed::Passes,
                    "{event:?}"
                );
            }
        }
    }

    #[test]
    fn an_ordinary_window_hears_everything() {
        let events = inputs().into_iter().chain([WindowEvent::Focused(true)]);
        for event in events {
            assert_eq!(
                at_unclaimed(InputOrigin::User, false, &event),
                Unclaimed::Passes,
                "{event:?}"
            );
        }
    }

    #[test]
    fn a_persons_focus_change_waits_for_the_run_to_end() {
        for focused in [true, false] {
            let event = WindowEvent::Focused(focused);
            assert_eq!(held_focus(InputOrigin::User, true, &event), Some(focused));
            assert_eq!(held_focus(InputOrigin::User, false, &event), None);
            assert!(!locks_out(InputOrigin::User, true, &event));
        }
    }
}

/// Key events carry the scancode's US-layout code, which ignores XKB options like
/// `caps:swapescape`. For the keys whose meaning the keymap may remap, take the code
/// from the keysym the keymap resolved instead. This includes function keys on
/// remapped media rows; ordinary character bindings still use their physical key.
pub(in crate::game) fn follow_keymap(event: &mut WindowEvent) {
    if let WindowEvent::KeyboardInput { event, .. } = event {
        crate::game::client_side::key_codes::follow_named_keymap(
            &event.logical_key,
            &mut event.physical_key,
        );
    }
}
