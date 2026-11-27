//! The mouse in play: the look it turns, the pointer grab (`MouseHandler.grabMouse` /
//! `releaseMouse`), the cursor icon, and pausing a world whose window loses focus.
use super::*;
use crate::game::client_side::screens::ScreenClientExt as _;

fn studio_suppresses_focus_pause(replay_active: bool, freecam_active: bool) -> bool {
    replay_active || freecam_active
}

impl crate::game::ClientGame<'_> {
    /// The options `MouseHandler.turnPlayer` reads.
    pub(in crate::game::client_side) fn look_options(&self) -> LookOptions {
        LookOptions {
            sensitivity: self.client.settings.sensitivity,
            smooth_camera: self.client.smooth_camera,
            scoping: self.scoping(),
            invert_x: self.client.settings.invert_x,
            invert_y: self.client.settings.invert_y,
        }
    }
    /// `MouseHandler.handleAccumulatedMovement` and `Entity.turn`, once per frame.
    /// An agent moving the mouse (`mouse-move`) counts as a grabbed pointer in
    /// an active window, as it cannot grab the real one.
    pub(in crate::game::client_side) fn apply_mouse_look(&mut self) {
        let mut look = self.look_options();
        let time = self.client.started.elapsed().as_secs_f64();
        let pointer = self.client.captured || self.client.agent_pointer;
        let grabbed = pointer
            && self.client.screen == Screen::Playing
            && (self.common.player.is_some() || self.client.studio.flying());
        let active = self.client.focused || self.client.agent_pointer;
        if active
            && grabbed
            && mod_api::__private::listened(mod_api::HookId::CalculatePlayerTurn)
            && let Some(remote) = &self.client.remote
        {
            self.native_calculate_player_turn(
                self.own_entity_id(),
                remote.profile,
                &mut look.sensitivity,
                &mut look.smooth_camera,
            );
        }
        if let Some((xo, yo)) = self
            .client
            .mouse
            .handle_accumulated_movement(time, active, grabbed, look)
        {
            self.look_turn(xo, yo);
        }
    }
    /// The tutorial hears the mouse or controller's turn before the player applies it.
    pub(in crate::game::client_side) fn look_turn(&mut self, x: f64, y: f64) {
        self.client.tutorial.mouse(x, y);
        self.turn_player(x, y);
    }
    /// `Minecraft.pause`: a single-player game with a pausing screen open.
    pub(in crate::game::client_side) fn pauses_game(&self) -> bool {
        self.has_singleplayer_server()
            && !self.is_published()
            && self.client.title.0.is_none()
            && top_screen_pauses(
                // A dialog (and its warning or waiting screen) is the
                // screen Minecraft set, over whatever was open.
                self.dialog_open()
                    .then(|| self.client.dialogs.0.pauses_world()),
                self.client.book.as_ref().map(|book| book.pauses_world()),
                self.client.screen.is_pause_screen(),
                self.win_screen_pauses(),
            )
    }
    /// A screen open over gameplay while `screen` stays `Playing`: a book or
    /// lectern, a sign, a server dialog, the code of conduct, a link or
    /// server pack prompt (vanilla's `Minecraft.screen` is set for each).
    pub fn screen_over_playing(&self) -> bool {
        self.client.book.is_some() || self.client.sign_edit.is_some() || self.prompt_open()
    }
    /// A prompt is up: over the world or over another open screen, it takes the input and is
    /// answered first, in the order of `handle_window_event`'s arms and of what draws on top.
    pub(in crate::game::client_side) fn prompt_open(&self) -> bool {
        self.dialog_open()
            || self.client.code_of_conduct.is_some()
            || self.client.link_prompt.is_some()
            || self.client.command_prompt.0.is_some()
            || self.client.server_packs.0.prompt.is_some()
    }
    /// `Minecraft.pauseIfInactive`: a window without focus for more than 500 ms
    /// pauses the game. An open screen (books, signs, the pack prompt, a server
    /// dialog) stays instead of pausing (`Gui.setPauseScreen`).
    pub(in crate::game::client_side) fn pause_if_inactive(&mut self) {
        if self
            .client
            .focus_lost
            .is_some_and(|t| t.elapsed() > Duration::from_millis(500))
            && self.common.control.is_none()
            && self.client.screen == Screen::Playing
            && !studio_suppresses_focus_pause(
                self.client.replay.playing(),
                self.client.studio.flying(),
            )
            && self.common.player.is_some()
            && !self.common.options.no_pause
            && self.client.settings.file.bool("pauseOnLostFocus")
            && !self.pad_keeps_playing_unfocused()
            && self.client.book.is_none()
            && self.client.sign_edit.is_none()
            && self.client.server_packs.0.prompt.is_none()
            && !self.dialog_open()
        {
            self.client.pause_focus = None;
            self.set_screen(Screen::Paused);
        }
    }
    /// `KeyMapping.setAll`: every key physically held is held again, as the
    /// mapping it is bound to, so a key pressed or kept down behind a screen
    /// counts once the game has the keyboard back. A sneak or sprint that
    /// toggles keeps its toggle state (`shouldSetOnIngameFocus`).
    pub(in crate::game::client_side) fn restore_held_keys(&mut self) {
        let held: Vec<KeyCode> = self.client.physical_keys.iter().copied().collect();
        let options = self.toggle_options();
        let has_menu_keys = !self
            .client
            .screen_state
            .0
            .context_menu_input
            .owned_keys_empty();
        for key in held {
            let restore_sources = if has_menu_keys {
                match self.context_menu_restore_sources(key) {
                    Some(0) => continue,
                    other => other,
                }
            } else {
                None
            };
            if let Some(name) = sunrise_client::keybinds::key_name(key) {
                for slot in self.client.settings.keys.slots_for(name) {
                    let option =
                        sunrise_client::keymapping::toggle_index(slot).is_some_and(|i| options[i]);
                    if self
                        .client
                        .vanilla_mappings
                        .state(slot)
                        .should_set_on_ingame_focus(option)
                    {
                        self.client.vanilla_mappings.set_down(slot, true, option);
                    }
                }
            }
            for mapped in self.client.settings.keys.translate_all(key) {
                match mapped {
                    Ok(KeyCode::ShiftLeft | KeyCode::ControlLeft) => {}
                    Ok(other) => match restore_sources {
                        None => {
                            self.insert_device_key(other);
                        }
                        Some(mask) => {
                            if mask & 1 != 0 {
                                self.press_device_key(other, Source::Keyboard);
                            }
                            if mask & 2 != 0 {
                                self.press_device_key(other, Source::Agent);
                            }
                        }
                    },
                    Err(_) => {}
                }
            }
        }
    }
    /// `MouseHandler.grabMouse` / `releaseMouse`. Both put the pointer at
    /// the window's centre, so a screen opened from gameplay (inventory,
    /// chest, pause, chat, sign, book, death) starts with the cursor in the
    /// middle instead of wherever the system last left it. Releasing when
    /// already released does nothing, so moving from screen to screen keeps
    /// the cursor where it is. Like GLFW's `glfwSetCursorPos`, nothing warps
    /// while the window is unfocused (losing focus releases the pointer).
    pub fn capture(&mut self, capture: bool) {
        // MouseHandler.grabMouse never captures an inactive window, and vanilla
        // grabs only on `setScreen(null)`: the pack prompt and server dialogs
        // are screens over the world that keep the pointer, as does a screen a
        // mod's `screen_opening` answer kept or opened in place of none.
        if capture
            && ((self.client.screen_state.0.answered.get()
                && self.client.screen != Screen::Playing)
                || (!self.client.focused && !self.client.unclaimed_window && !self.scripted_run())
                || self.client.server_packs.0.prompt.is_some()
                || self.dialog_open()
                || self.client.command_prompt.0.is_some())
        {
            return;
        }
        // The container screen's press state belongs to the screen: a screen
        // that takes the pointer over starts with its own state, as
        // `AbstractContainerScreen` does, so a press whose release never
        // arrived (the pointer left the window) cannot wedge the next
        // screen's quick move and clicks.
        self.reset_inventory_clicks();
        if !capture {
            self.stop_mining();
            self.client.studio.rmb_down = false;
        }
        // The real pointer takes over from an agent's.
        self.client.agent_pointer = false;
        if !capture {
            self.client.screen_grab = false;
        }
        if capture
            && !self.client.captured
            && !self.client.screen_grab
            && !cfg!(target_os = "macos")
        {
            // MouseHandler.grabMouse: KeyMapping.setAll unless
            // InputQuirks.RESTORE_KEY_STATE_AFTER_MOUSE_GRAB is off (macOS).
            self.restore_held_keys();
        }
        let Some(window) = &self.client.window.0 else {
            return;
        };
        let size = self.client.renderer.0.as_ref().map_or_else(
            || window.inner_size().into(),
            |r| [r.config.width, r.config.height],
        );
        let projection = self.pointer_projection(size);
        let centre = projection.window.center();
        let warp = |window: &Window| {
            let _ = window.set_cursor_position(centre);
        };
        let was_grabbed = self.client.captured;
        if self.internal_grab() {
            // Its grab is internal (an agent turns the view with `mouse-move`). A grab the
            // window already held when a scripted run began stays the system's until the
            // run ends (`settle_after_run`).
            if capture {
                self.client.mouse.set_ignore_first_move();
            }
            self.client.captured = capture;
        } else if capture {
            self.client.mouse.set_ignore_first_move();
            // macOS warps by re-associating the mouse with the pointer,
            // which undoes a Locked grab and leaves the pointer free: warp
            // first there. Wayland moves a pointer only while it is locked,
            // so everywhere else warp after grabbing (Windows does both).
            #[cfg(target_os = "macos")]
            warp(window);
            // Windows clips a Locked pointer to a 1x1 rectangle where it is, and `SetCursorPos`
            // stays inside that clip: warp first, or the pointer stays frozen wherever it was
            // (over another window, on another monitor) instead of the window's centre (#6076).
            #[cfg(windows)]
            if self.client.focused {
                warp(window);
            }
            self.client.capture_mode = if window.set_cursor_grab(CursorGrabMode::Locked).is_ok() {
                Some(CursorGrabMode::Locked)
            } else if window.set_cursor_grab(CursorGrabMode::Confined).is_ok() {
                Some(CursorGrabMode::Confined)
            } else {
                None
            };
            self.client.captured = self.client.capture_mode.is_some();
            // Windows already warped above, inside the clip's own position.
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            if self.client.captured && self.client.focused {
                warp(window);
            }
        } else {
            if self.client.captured {
                // Wayland moves a pointer only as the locked pointer's
                // position hint, which the compositor applies as the lock
                // goes (sway and Hyprland do): set it before ungrabbing.
                // X11, Windows and macOS warp, so warp again once free
                // (Wayland refuses that one).
                if self.client.focused {
                    warp(window);
                }
                let _ = window.set_cursor_grab(CursorGrabMode::None);
                if self.client.focused {
                    warp(window);
                }
            } else {
                let _ = window.set_cursor_grab(CursorGrabMode::None);
            }
            self.client.captured = false;
            self.client.capture_mode = None;
        }
        // MouseHandler.grabMouse and releaseMouse: the mouse position is
        // the window's centre whenever the pointer's grab state changes, so
        // a screen opened from gameplay starts with the cursor in the middle
        // and the game starts with it away from the screen's last spot.
        if !self.internal_grab() {
            window.set_cursor_visible(!self.client.captured);
        }
        if let Some(pos) = projection
            .window
            .center_after_grab_change(capture, was_grabbed)
        {
            self.set_window_cursor(projection.window.physical_sample(pos), projection);
        }
    }

    /// The open mod screen holds the pointer (`ScreensClientHost::set_pointer_grab`) and the
    /// window has it: the device's motion is the screen's.
    pub(super) fn mod_screen_grabs_pointer(&self) -> bool {
        self.client.screen_grab && self.client.captured && self.client.screen == Screen::Mod
    }

    /// The device's motion with the pointer grabbed (`MouseHandler.onMove`, applied once per
    /// frame): a grabbing mod screen's `screen_pointer_motion`, otherwise the camera's.
    pub(in crate::game) fn grabbed_pointer_motion(&mut self, [x, y]: [f64; 2]) {
        self.note_input(
            input_trace::InputOrigin::User,
            input_trace::TraceKind::Motion { x, y },
        );
        if self.mod_screen_grabs_pointer() {
            self.screen_pointer_motion([x, y]);
        } else {
            self.client.mouse.on_move(0.0, 0.0, x, y, true, true);
        }
        self.client.last_input = Instant::now();
    }

    /// Dispatch the grabbing mod screen's raw device motion.
    pub(in crate::game) fn screen_pointer_motion(&self, delta: [f64; 2]) {
        if delta == [0.0; 2]
            || !self.mod_screen_grabs_pointer()
            || !mod_api::__private::listened(mod_api::HookId::ScreenPointerMotion)
        {
            return;
        }
        if let Some(receiver) = self.observed_input_screen() {
            self.native_screen_pointer_motion(&receiver.view(), delta);
        }
    }

    /// Whether the open screen is the calling mod's own (`mod_api::content::crash::current_native_mod`).
    fn calling_mod_owns_screen(&self) -> bool {
        mod_api::content::crash::current_native_mod()
            .is_some_and(|owner| self.mod_screen() == Some(owner))
    }

    /// `ScreensClientHost::set_pointer_grab`: the grab of the calling mod's own open screen,
    /// taken at the next `apply_pointer_grab_request`. Refused (nothing changes) when the open
    /// screen is another mod's or none, or, for a grab, when the window is not focused
    /// (`MouseHandler.grabMouse` never grabs an inactive window; `capture` allows the same
    /// unclaimed and scripted windows).
    pub(in crate::game) fn set_pointer_grab(&self, grab: bool) -> bool {
        if !self.calling_mod_owns_screen() {
            return false;
        }
        if !grab {
            if self.pointer_grabbed() {
                self.client.pointer_grab_request.set(Some(false));
            }
            return true;
        }
        if !(self.client.focused || self.client.unclaimed_window || self.scripted_run())
            || self.has_live_context_menu()
        {
            return false;
        }
        self.client.pointer_grab_request.set(Some(true));
        true
    }

    /// `ScreensClientHost::pointer_grabbed`: the calling mod's open screen holds the grab, as
    /// asked for if the request is still waiting.
    pub(in crate::game) fn pointer_grabbed(&self) -> bool {
        self.calling_mod_owns_screen()
            && self
                .client
                .pointer_grab_request
                .get()
                .unwrap_or_else(|| self.mod_screen_grabs_pointer())
    }

    /// Applies what `set_pointer_grab` asked for, and ends a grab whose screen's mod is off.
    /// The screen's own `capture(true)` skips vanilla's `KeyMapping.setAll` (the screen, not
    /// the game, has the keyboard) and is not refused by an earlier `screen_opening` answer,
    /// which only meant that caller's grab after that change. Released by `capture(false)`,
    /// which every screen change and focus loss run.
    pub(in crate::game) fn apply_pointer_grab_request(&mut self) {
        let request = self.client.pointer_grab_request.take();
        let owner_off = self
            .client
            .native_mod_screen
            .as_deref()
            .is_some_and(|owner| {
                mod_api::__private::mod_index(owner)
                    .is_some_and(|i| !mod_api::__private::mod_index_on(i))
            });
        if self.client.screen_grab && (owner_off || request == Some(false)) {
            self.capture(false);
        } else if request == Some(true)
            && !owner_off
            && self.client.screen == Screen::Mod
            && !self.live_context_menu()
        {
            self.client.screen_grab = true;
            self.client.screen_state.0.answered.set(false);
            self.capture(true);
            if !self.client.captured {
                self.client.screen_grab = false;
            }
        }
    }

    /// `MouseHandler.isMouseGrabbed`: whether the window holds the pointer.
    pub(in crate::game) fn is_mouse_grabbed(&self) -> bool {
        self.client.captured
    }

    /// Close the screen before grabbing the pointer for gameplay.
    pub(in crate::game) fn set_mouse_grabbed(&mut self, grabbed: bool) {
        if !self.common.options.execution_role.is_client() {
            return;
        }
        if !grabbed {
            if self.client.captured {
                self.capture(false);
            }
            return;
        }
        if self.client.captured
            || !(self.client.focused || self.client.unclaimed_window || self.scripted_run())
            || self.common.player.is_none()
            || self.remote_screen_up()
            || self.client.screen == Screen::Dead
        {
            return;
        }
        if self.client.screen == Screen::Inventory {
            self.close_container_client();
        }
        self.set_screen(Screen::Playing);
        if self.client.screen == Screen::Playing
            && self.client.title.0.is_none()
            && !self.screen_over_playing()
        {
            self.capture(true);
            if self.client.captured {
                self.common.clicks.miss_time = 10000;
            }
        }
    }
    /// The game's pointer grab leaves the system pointer where the person at the computer has
    /// it: a genuinely hidden `--offscreen` run, an agent's window no one has clicked yet, or a
    /// scripted run, which plays the game while the person may be using the computer.
    pub(in crate::game::client_side) fn internal_grab(&self) -> bool {
        self.common.options.offscreen || self.client.unclaimed_window || self.scripted_run()
    }
    /// A person pressed a mouse button inside the agent's window (`at_unclaimed`): from now on
    /// it is a window like any other, and that press takes the pointer
    /// (`MouseHandler.onPress`) instead of attacking. The focus changes it didn't hear while
    /// unclaimed, or an agent's `pad focus`, may have left the game's idea of the focus apart
    /// from the window's: it catches up first, as a `Focused` would have told it. So do the
    /// modifiers the person holds (`held_modifiers`).
    pub(in crate::game::client_side) fn claim_window(&mut self, event_loop: &dyn EventLoopCtl) {
        self.client.unclaimed_window = false;
        self.client.captured = false;
        // A regrab owed since an agent's `pad focus false` would take the pointer as the focus
        // catches up, and the press would then attack.
        self.client.regrab_on_refocus = false;
        let Some(window) = &self.client.window.0 else {
            return;
        };
        let (id, focused) = (window.id(), window.has_focus());
        if focused != self.client.focused {
            self.handle_window_event(
                event_loop,
                id,
                WindowEvent::Focused(focused),
                input_trace::InputOrigin::User,
            );
        }
        if let Some(modifiers) = self.client.unclaimed_modifiers.take() {
            self.handle_window_event(
                event_loop,
                id,
                WindowEvent::ModifiersChanged(modifiers),
                input_trace::InputOrigin::User,
            );
        }
    }
    /// Once a scripted run is over, the person's last click into the window or away from it
    /// applies (`held_focus`), and the system pointer follows the grab the run left inside the
    /// game, as if the window had taken or released it now.
    pub(in crate::game::client_side) fn settle_after_run(&mut self, event_loop: &dyn EventLoopCtl) {
        if self.scripted_run() {
            return;
        }
        if let Some(focused) = self.client.focus_after_run.take() {
            let id = self
                .client
                .window
                .0
                .as_ref()
                .map_or_else(WindowId::dummy, |w| w.id());
            self.handle_window_event(
                event_loop,
                id,
                WindowEvent::Focused(focused),
                input_trace::InputOrigin::User,
            );
        }
        // Outside a run, a window whose grab is the system's holds it exactly while the game
        // has it grabbed; only a run leaves the two apart.
        if !self.internal_grab() && self.client.captured != self.client.capture_mode.is_some() {
            let grab = self.client.captured;
            self.client.captured = self.client.capture_mode.is_some();
            self.capture(grab);
        }
    }
    /// A Linux window manager can release the native pointer grab while the
    /// window remains focused. Reapply the recorded mode on the next gameplay
    /// click or wheel event, without warping the pointer or consuming input.
    pub(in crate::game::client_side) fn reassert_pointer_capture(&mut self) {
        #[cfg(target_os = "linux")]
        if self.client.screen == Screen::Playing
            && self.client.focused
            && self.client.captured
            && !self.common.options.offscreen
            && !self.internal_grab()
            && let (Some(window), Some(mode)) = (&self.client.window.0, self.client.capture_mode)
        {
            let other = match mode {
                CursorGrabMode::Locked => CursorGrabMode::Confined,
                CursorGrabMode::Confined => CursorGrabMode::Locked,
                CursorGrabMode::None => return,
            };
            self.client.capture_mode = if window.set_cursor_grab(mode).is_ok() {
                Some(mode)
            } else if window.set_cursor_grab(other).is_ok() {
                Some(other)
            } else {
                None
            };
            self.client.captured = self.client.capture_mode.is_some();
            window.set_cursor_visible(!self.client.captured);
        }
    }
    /// `GuiGraphicsExtractor.applyCursor` -> `Window.selectCursor` at the end
    /// of a frame: the cursor the frame's widgets asked for, set on the window
    /// only when it changes.
    pub(in crate::game::client_side) fn apply_cursor_icon(&mut self) {
        use sunrise_client::widgets::Cursor;
        let wanted = sunrise_client::widgets::take_cursor();
        // The widgets under the keyboard's hidden pointer aren't under the
        // mouse (`handleCursor` asks only for a hovered widget).
        let effective = if self.client.settings.allow_cursor_changes && !self.key_focus_driving() {
            wanted
        } else {
            Cursor::Arrow
        };
        if effective == self.client.cursor_icon {
            return;
        }
        if let Some(window) = &self.client.window.0 {
            window.set_cursor(match effective {
                Cursor::Arrow => winit::window::CursorIcon::Default,
                Cursor::PointingHand => winit::window::CursorIcon::Pointer,
                Cursor::Ibeam => winit::window::CursorIcon::Text,
                Cursor::NotAllowed => winit::window::CursorIcon::NotAllowed,
                Cursor::ResizeNs => winit::window::CursorIcon::NsResize,
                Cursor::ResizeEw => winit::window::CursorIcon::EwResize,
            });
        }
        self.client.cursor_icon = effective;
    }
    /// LevelLoadingScreen.onClose -> Gui.setScreen(null) -> grabMouse.
    pub(in crate::game::client_side) fn capture_loaded_world(&mut self) {
        // Loading may finish after focus was lost or another screen opened.
        if self.client.title.0.is_none() && self.client.screen == Screen::Playing {
            self.capture(true);
        }
    }
    /// The window came back into focus after holding the pointer: the system
    /// dropped the grab with the focus, and vanilla's grab outlives it
    /// (`MouseHandler.mouseGrabbed` stays true while GLFW keeps the cursor
    /// mode), so the game takes the pointer again without waiting for a
    /// click. A screen that is open keeps it free, as releasing the pointer
    /// for that screen did: it is the screen, not the focus, that shows it.
    pub(in crate::game::client_side) fn regrab_on_refocus(&mut self) {
        if self.studio_up() {
            self.client.regrab_on_refocus = false;
            self.client.studio.rmb_down = false;
            self.capture(false);
            return;
        }
        if std::mem::take(&mut self.client.regrab_on_refocus)
            && self.client.screen == Screen::Playing
        {
            self.capture(true);
        }
    }
}

/// Which of the things over gameplay decides `Minecraft.pause`: a dialog is
/// its own `DialogScreen` (Minecraft set it over whatever was open) and
/// comes first, then a book, else `Gui.screen`; the win screen's poem rides
/// with its screen. `None` means that kind of screen is not open.
fn top_screen_pauses(dialog: Option<bool>, book: Option<bool>, screen: bool, win: bool) -> bool {
    dialog.or(book).unwrap_or(screen || win)
}

use sunrise_gameplay_common::game::integrated_common::IntegratedCommonGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::{studio_suppresses_focus_pause, top_screen_pauses};

    #[test]
    fn studio_replay_and_freecam_never_pause_on_focus_loss() {
        assert!(studio_suppresses_focus_pause(true, false));
        assert!(studio_suppresses_focus_pause(false, true));
        assert!(!studio_suppresses_focus_pause(false, false));
    }

    /// `Minecraft.pause` asks the screen Minecraft set, and each of a dialog,
    /// a book and the win screen is a screen of its own that replaces what
    /// was open: the dialog's `pause`, the book's `BookViewScreen`/
    /// `LecternScreen`, and `WinScreen`'s `!poem` decide, not the screen
    /// underneath.
    #[test]
    fn the_screen_on_top_decides_whether_the_world_pauses() {
        // Nothing replaces the screen: its own isPauseScreen answer, with
        // the win screen's poem rule when it is the win screen.
        assert!(top_screen_pauses(None, None, true, false));
        assert!(top_screen_pauses(None, None, false, true));
        assert!(!top_screen_pauses(None, None, false, false));
        // A dialog over a pause screen: its own pause answers.
        assert!(!top_screen_pauses(Some(false), None, true, false));
        assert!(top_screen_pauses(Some(true), None, false, false));
        // A lectern's book over a pause screen does not pause; a book in
        // hand does.
        assert!(!top_screen_pauses(None, Some(false), true, false));
        assert!(top_screen_pauses(None, Some(true), false, false));
        // A dialog opens over the book.
        assert!(!top_screen_pauses(Some(false), Some(true), false, false));
    }
}
