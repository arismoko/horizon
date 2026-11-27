//! Controllers through gilrs (Sunrise's own; docs/controllers.md): the
//! active pad's state each loop iteration, its button edges dispatched as
//! the vanilla key mappings they are bound to, the left stick as an analog
//! move vector and the right stick turning the view. `sunrise-ctl pad`
//! injects a virtual pad on top of the real one.
use super::Screen;
use super::event_loop::EventLoopCtl;
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use sunrise_client::actions::Action;
use sunrise_client::held::Source;
use sunrise_client::{
    controller_notice::Notice,
    controller_ui::Device,
    gamepad::{
        self, Button, Edges, GlyphOf, GlyphStyle, HealthWatch, MenuJob, PadState, Route, Rumble,
    },
    hud::HudBatch,
    keymapping::SNEAK,
    pad_glyph::{Glyph, Glyphs},
};
use sunrise_client_input::radial;
use sunrise_game_data::Result;
use winit::keyboard::KeyCode;

pub(in crate::game) const HELP: &[&str] = &[
    "pad BUTTON down|up|tap         a virtual controller button (south east west north lb rb lt rt select start ls rs up down left right)",
    "pad stick left|right X Y       tilt a virtual stick (-1..1, up positive); 0 0 centres it",
    "pad trigger left|right VALUE   pull a virtual trigger (0..1)",
    "pad release                    centre and release the virtual controller (as an unplug: the radial menu cancels)",
    "pad focus true|false           the window gains or loses the focus, as a window focus event does",
    "pad status                     connected controllers, the active one and its state",
    "pad connection connected [FAMILY]|disconnected   as if a controller (PlayStation unless FAMILY) came or went at this game's own reader",
    "pad family xbox|playstation|nintendo|auto   the pad playing is that family's (its glyphs; a Nintendo one's letters)",
];

/// A pad family `pad` names; `auto` is a pad of no known maker.
pub(in crate::game::client_side) fn style_named(name: &str) -> Result<GlyphStyle> {
    GlyphStyle::ALL
        .into_iter()
        .find(|style| style.name() == name)
        .ok_or_else(|| {
            format!("family must be xbox, playstation, nintendo or auto, not {name}").into()
        })
}

/// The controllers and the state the game last acted on.
#[derive(Default)]
pub struct Pads {
    /// The real controllers and the DualSense's output.
    #[cfg(feature = "controllers")]
    pub(in crate::game) devices: hardware::Devices,
    /// The virtual pad of `sunrise-ctl pad`.
    pub(in crate::game) injected: PadState,
    /// The virtual pad was in use at the last poll.
    pub virtual_input: bool,
    /// Buttons a `pad ... tap` releases at the next poll.
    pub(in crate::game) taps: Vec<Button>,
    pub edges: Edges,
    pub(in crate::game) activity: gamepad::Activity,
    pub(in crate::game) context: Option<Context>,
    pub(in crate::game) blocked_sticks: [bool; 2],
    /// Releases use the action that received the press, even after a rebind.
    pub(in crate::game) actions: [Vec<&'static str>; 16],
    /// The game tick each button went down in, and releases held back until
    /// a tick has passed: vanilla samples keys once per tick, so a click
    /// shorter than that (a quick stick click for sprint) would be lost.
    pub(in crate::game) pressed_at: [Option<u64>; 16],
    pub(in crate::game) late_releases: Vec<Button>,
    /// The merged state of the last poll.
    pub state: PadState,
    /// The hardware's buttons at the last poll, for `input-log`'s `pad`
    /// events: the merged state hides which source moved.
    pub(in crate::game) last_hardware: PadState,
    /// The pad drove the game last (the mouse and keyboard hand control back).
    pub(in crate::game) in_use: bool,
    /// A key or mouse button was pressed in play since the pad was last touched: the keyboard and
    /// mouse have it, though only moving the mouse takes the pad out of use.
    pub(in crate::game) keyboard_took_over: bool,
    pub(in crate::game) style: GlyphStyle,
    /// The controller driving this player, or else the first one its own reader sees.
    pub(in crate::game) device: Option<Device>,
    /// Controllers that came and went at this game's own reader since the last poll.
    pub(in crate::game) connections: Vec<Connection>,
    /// The right stick's turn still easing out under the cinematic camera.
    pub(in crate::game) look: gamepad::SmoothLook,
    /// The cursor the pad drives in menus.
    pub menu: super::pad_menu::MenuCursor,
    /// The radial menu while its opener is held, and the page it stays on.
    pub(in crate::game) radial: radial::RadialMenu,
    /// What the radial menu ran, for `pad status`.
    pub(in crate::game) radial_runs: RadialRuns,
    /// The moments the radial menu's overlay moves from (`radial_overlay::Look`).
    pub(in crate::game) radial_look: sunrise_client_render::radial_overlay::Look,
    /// The D-pad's quiet after the menu closed on a D-pad opener.
    pub(in crate::game) radial_settle: radial::DpadSettle,
    /// The narrator said the page shown, and the slot selected.
    pub(in crate::game) radial_page_said: bool,
    pub(in crate::game) radial_said: radial::Settle<(usize, usize)>,
    /// A page turn's second tick, when the pages went round an end.
    pub(in crate::game) radial_rumble_due: Option<(Instant, Rumble)>,
    /// The furthest each stick leant (left, right) between the last two polls, from the pad's
    /// own events: a flick out and back between two frames still reaches the radial menu.
    pub(in crate::game) peaks: [[f32; 2]; 2],
    /// With Either selecting: the stick that last leant past its select tilt (0 left, 1 right),
    /// and whether each was past it at the last poll.
    pub(in crate::game) radial_either: Option<usize>,
    pub(in crate::game) radial_over: [bool; 2],
    /// While the radial menu is open with Either: each stick has been at rest since the open, so
    /// it may take over. A thumb still on the walking stick can't commit for the other.
    pub(in crate::game) radial_rested: [bool; 2],
    /// The last vibration asked for, for `pad status`.
    pub(in crate::game) last_rumble: Option<(&'static str, gamepad::Pulse)>,
    /// The drop button, while it is held in play.
    pub(in crate::game) held_drop: Option<gamepad::HeldDrop>,
    /// The drop about to happen is the pad's stack form: it has no Control for `drop_whole_stack`.
    /// On a server the drop waits for the tick.
    pub(in crate::game) drop_stack: bool,
    /// The player's health between polls (a drop is damage taken).
    pub(in crate::game) health: HealthWatch,
    /// The gameplay guide's facts and the tick they were read in.
    pub(in crate::game) guide_facts: Option<(u64, sunrise_client::gameplay_guide::Facts)>,
    /// The guide's rows as last drawn.
    pub(in crate::game) guide_shown: Vec<guide::Shown>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::game) struct Context {
    pub(in crate::game) screen: u64,
    pub(in crate::game) focus_navigation: bool,
    pub(in crate::game) focused: bool,
    pub(in crate::game) osk: bool,
    pub(in crate::game) dialog: bool,
    pub(in crate::game) enabled: bool,
}

pub(in crate::game) mod capture;
pub(in crate::game) mod guide;
#[cfg(feature = "controllers")]
pub(in crate::game) mod hardware;
pub(in crate::game) mod radial_play;

/// How many of the radial menu's last actions `pad status` lists.
pub(in crate::game) const RADIAL_RUNS_KEPT: usize = 16;

/// What the radial menu ran: how many actions in all, and the last few, last one last.
#[derive(Default)]
pub(in crate::game) struct RadialRuns {
    pub(in crate::game) count: u64,
    pub(in crate::game) last: std::collections::VecDeque<&'static str>,
}
impl RadialRuns {
    pub(in crate::game::client_side) fn push(&mut self, action: &'static str) {
        self.count += 1;
        if self.last.len() == RADIAL_RUNS_KEPT {
            self.last.pop_front();
        }
        self.last.push_back(action);
    }
}

/// The volume of a UI button's click (`AbstractButton.playDownSound`).
pub(in crate::game) const UI_CLICK_VOLUME: f32 = 0.25;

/// How long the death screen's confirm ignores cancelling (`ConfirmScreen`).
pub(in crate::game) const DEATH_CONFIRM_WAIT: Duration = Duration::from_secs(1);

/// A controller coming or going, for this player's notice (`pad_connection_notice`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) enum Connection {
    /// A controller of this glyph family connected.
    Connected(GlyphStyle),
    /// One disconnected; `active`: it was driving this player.
    Disconnected { active: bool },
}

/// A build without the `controllers` feature (the server build, which must not need libudev): no
/// hardware pads, only `sunrise-ctl pad`'s virtual one.
#[cfg(not(feature = "controllers"))]
pub(in crate::game) mod hardware {
    use super::Pads;
    use serde_json::{Value, json};
    use sunrise_client::gamepad::{PadState, Pulse, Rumble};

    impl Pads {
        pub(in crate::game::client_side) fn read(&mut self) -> PadState {
            PadState::default()
        }
    }

    impl crate::game::ClientGame<'_> {
        pub(in crate::game::client_side) fn pad_output_off(&mut self) {}
        pub(in crate::game::client_side) fn pad_output_frame(&mut self) {}
        pub(in crate::game::client_side) fn pad_close_output(&mut self) {}
        pub(in crate::game::client_side) fn pad_vibrate(&mut self, _: Rumble, _: Pulse) {}
        pub(in crate::game::client_side) fn pad_devices_status(&self) -> Value {
            json!({ "backend": false, "controllers": [] })
        }
    }
}

impl Pads {
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_set_in_use(&mut self, in_use: bool) {
        self.in_use = in_use;
    }

    /// The digital buttons the person's pad changed since the last poll.
    pub(in crate::game::client_side) fn hardware_edges(
        &mut self,
        hardware: PadState,
    ) -> Vec<(Button, bool)> {
        let edges = Button::ALL
            .into_iter()
            .filter_map(|button| {
                let was = self.last_hardware.held(button);
                let now = hardware.held(button);
                (was != now).then_some((button, now))
            })
            .collect();
        self.last_hardware = hardware;
        edges
    }

    /// A replay's pad button, in place of `pad`'s line.
    pub(in crate::game::client_side) fn inject_button(&mut self, button: Button, down: bool) {
        self.injected.set(button, down);
    }
    /// The device the controller settings name: a real one, else `sunrise-ctl`'s virtual pad.
    pub(in crate::game::client_side) fn device_or_virtual(&self) -> Option<Device> {
        self.device.clone().or_else(|| {
            self.in_use.then(|| Device {
                name: "Virtual controller".to_owned(),
                vendor: None,
                product: None,
            })
        })
    }
    /// A replay ended: let go of everything it held.
    pub(in crate::game::client_side) fn clear_injected(&mut self) {
        self.menu.armed = None;
        self.radial.cancel();
        self.menu.stop_craft();
        self.injected = PadState::default();
    }

    /// Whether a held button's press went to `action`, including releases delayed until the next
    /// simulation tick. What the press reached, not what the button is bound to now: a press the
    /// radial menu swallowed holds nothing, while a sneak held since before the menu opened is
    /// still held through it.
    pub(in crate::game::client_side) fn holds_action(&self, action: &str) -> bool {
        Button::ALL.into_iter().any(|button| {
            self.actions[button as usize].contains(&action)
                && (self.edges.is_held(button) || self.late_releases.contains(&button))
        })
    }

    /// A new player entity took over (spawn, respawn, a server joined, a
    /// dimension change): its health is not compared with the old one's.
    /// The radial menu closes with it, in `Game::pad_player_replaced`.
    pub(in crate::game::client_side) fn player_replaced(&mut self) {
        self.health.forget();
    }

    /// The active pad is a Nintendo one going by its letters.
    pub(in crate::game::client_side) fn by_letter(&self, settings: &gamepad::PadSettings) -> bool {
        settings.by_letter(self.style)
    }
}

impl crate::game::ClientGame<'_> {
    /// The glyph for what a button does, in this player's glyphs (`ui.options.pad_glyphs`, a
    /// Nintendo pad's letters included): the way gameplay asks for one. None while an action is
    /// unbound.
    pub(in crate::game::client_side) fn glyph(&self, of: impl Into<GlyphOf>) -> Option<Glyph> {
        Glyph::of(
            of.into(),
            &self.client.settings.controller,
            self.client.chat_ui.options.pad_glyphs,
        )
    }

    /// A menu job's glyph in this player's glyphs.
    pub(in crate::game::client_side) fn menu_glyph(&self, job: MenuJob) -> Glyph {
        Glyph::menu(job, self.client.chat_ui.options.pad_glyphs)
    }

    pub(in crate::game::client_side) fn pad_context(&self) -> Context {
        Context {
            screen: self.key_screen(),
            focus_navigation: self.pad_navigation_mode(),
            focused: self.client.focused,
            osk: self.pad_keyboard_captures(),
            dialog: self.dialog_open() || self.client.code_of_conduct.is_some(),
            enabled: self.client.settings.controller.enabled,
        }
    }

    /// Once per loop iteration: read the pads and act on button changes.
    pub(in crate::game::client_side) fn poll_pads(&mut self, event_loop: &dyn EventLoopCtl) {
        self.client.pad.0.virtual_input = self.client.pad.0.injected != PadState::default();
        let context = self.pad_context();
        if let Some(old) = self.client.pad.0.context.filter(|old| *old != context) {
            if old.screen != context.screen {
                self.pad_remember_focus(old.screen);
            }
            self.pad_cancel_inputs(event_loop);
        }
        self.client.pad.0.context = Some(context);
        if !self.client.settings.controller.enabled {
            self.pad_output_off();
            self.pad_cancel_inputs(event_loop);
            // The radial menu closed with the pad: its overlay fades out, as for a cancel.
            self.client
                .pad
                .0
                .radial_look
                .observe(None, std::time::Instant::now());
            self.client.pad.0.state = PadState::default();
            self.client.pad.0.in_use = false;
            // The keyboard's menu focus follows its widget all the same.
            self.pad_menu_frame(event_loop);
            return;
        }
        // A scripted run leaves the person's pad at rest and unread, just like its
        // keyboard and mouse. Reading it could change the active pad mid-run.
        let hardware = if self.scripted_run() {
            PadState::default()
        } else {
            self.client.pad.0.read()
        };
        let hardware = if self
            .client
            .pad
            .0
            .by_letter(&self.client.settings.controller)
        {
            hardware.by_letter()
        } else {
            hardware
        };
        // A person's pad, for `input-log` and the repro recording; the virtual
        // pad is an agent's `pad` line and is recorded as that.
        let hardware_edges = self.client.pad.0.hardware_edges(hardware);
        for (button, down) in hardware_edges {
            self.note_input(
                super::input_trace::InputOrigin::User,
                super::input_trace::TraceKind::Pad {
                    button: button.name().to_owned(),
                    action: if down { "down" } else { "up" }.to_owned(),
                },
            );
        }
        for change in std::mem::take(&mut self.client.pad.0.connections) {
            self.pad_connection_notice(change);
        }
        self.client.chat_ui.options.pad_glyphs =
            Glyphs::of(&self.client.settings.controller, self.client.pad.0.style);
        self.client.chat_ui.options.pad_driving = self.client.pad.0.in_use && self.pad_focus_mode();
        let mut state = hardware.merge(&self.client.pad.0.injected);
        let activity = self
            .client
            .pad
            .0
            .activity
            .observe(&state, &self.client.settings.controller);
        for (i, (stick, deadzone)) in [
            (
                &mut state.left,
                self.client.settings.controller.move_deadzone,
            ),
            (
                &mut state.right,
                self.client.settings.controller.look_deadzone,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            if gamepad::radial(*stick, deadzone) == [0.0; 2] {
                self.client.pad.0.blocked_sticks[i] = false;
            }
            if self.client.pad.0.blocked_sticks[i] || !self.pad_focused() {
                *stick = [0.0; 2];
                self.client.pad.0.peaks[i] = [0.0; 2];
            }
        }
        for button in std::mem::take(&mut self.client.pad.0.taps) {
            self.client.pad.0.injected.set(button, false);
        }
        let threshold = self.client.settings.controller.trigger_threshold;
        let edges = self.client.pad.0.edges.update(&state, threshold);
        if activity && self.pad_focused() {
            self.client.last_input = Instant::now();
            self.client.pad.0.in_use = true;
            self.client.pad.0.keyboard_took_over = false;
        }
        let previous_move = self.pad_move();
        self.client.pad.0.state = state;
        self.resume_stick_mappings(previous_move);
        self.pad_radial_frame();
        self.pad_drop_frame();
        self.pad_output_frame();
        // Damage from any source, here or on a server; nothing counts on
        // the title screen.
        let health = self
            .common
            .player
            .as_ref()
            .filter(|_| self.client.title.0.is_none())
            .map(|p| p.health.health());
        if let Some(damage) = self.client.pad.0.health.observe(health) {
            self.pad_rumble(Rumble::Hurt(damage));
        }
        let ticking = self.client.title.0.is_none()
            && self.client.screen == Screen::Playing
            && !self.common.pause;
        let mut due = Vec::new();
        for button in std::mem::take(&mut self.client.pad.0.late_releases) {
            if !ticking || self.client.pad.0.pressed_at[button as usize] != Some(self.common.ticks)
            {
                due.push(button);
            } else {
                self.client.pad.0.late_releases.push(button);
            }
        }
        // The steps, in order: a press in the same poll as the opener's release, held back or
        // not, gets there first, so B still cancels.
        for step in radial_play::poll_order(due, edges, self.pad_radial_captures()) {
            let (button, down) = match step {
                radial_play::PollStep::Late(button) => {
                    self.pad_button_logged(event_loop, button, false);
                    continue;
                }
                radial_play::PollStep::Edge(button, down) => (button, down),
            };
            self.pad_edge(event_loop, button, down);
            let context = self.pad_context();
            if let Some(old) = self.client.pad.0.context.filter(|old| *old != context) {
                if old.screen != context.screen {
                    self.pad_remember_focus(old.screen);
                }
                self.pad_cancel_inputs(event_loop);
                self.client.pad.0.context = Some(context);
                break;
            }
        }
        self.pad_menu_frame(event_loop);
    }

    pub(in crate::game::client_side) fn pad_edge(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        button: Button,
        down: bool,
    ) {
        let ticking = self.client.title.0.is_none()
            && self.client.screen == Screen::Playing
            && !self.common.pause;
        if down {
            // Pressed again before its held-back release went out.
            if let Some(i) = self
                .client
                .pad
                .0
                .late_releases
                .iter()
                .position(|b| *b == button)
            {
                self.client.pad.0.late_releases.remove(i);
                self.pad_button_logged(event_loop, button, false);
            }
            self.client.pad.0.pressed_at[button as usize] = Some(self.common.ticks);
        } else if ticking
            && self.client.pad.0.pressed_at[button as usize] == Some(self.common.ticks)
        {
            self.client.pad.0.late_releases.push(button);
            return;
        }
        self.pad_button_logged(event_loop, button, down);
    }

    /// End held actions and captures before another context receives input.
    pub(in crate::game::client_side) fn pad_cancel_inputs(
        &mut self,
        event_loop: &dyn EventLoopCtl,
    ) {
        // Before the releases below: a cancelled press must not click, a cancelled trigger
        // change the recipe book's category as it comes up, nor a cancelled radial menu
        // commit when its opener comes up.
        self.client.pad.0.menu.armed = None;
        self.client.pad.0.menu.trigger = None;
        self.client.pad.0.radial.cancel();
        self.direct_craft_stop();
        let held_sticks = [
            gamepad::radial(
                self.client.pad.0.state.left,
                self.client.settings.controller.move_deadzone,
            ) != [0.0; 2],
            gamepad::radial(
                self.client.pad.0.state.right,
                self.client.settings.controller.look_deadzone,
            ) != [0.0; 2],
        ];
        for (blocked, held) in self.client.pad.0.blocked_sticks.iter_mut().zip(held_sticks) {
            *blocked |= held;
        }
        self.client.pad.0.state.left = [0.0; 2];
        self.client.pad.0.state.right = [0.0; 2];
        self.client.pad.0.late_releases.clear();
        for (button, down) in self.client.pad.0.edges.cancel() {
            self.pad_button_logged(event_loop, button, down);
        }
        // A short tap may already be up in Edges but still await its simulation tick.
        for button in Button::ALL {
            for action in std::mem::take(&mut self.client.pad.0.actions[button as usize]) {
                if let Err(e) = self.pad_action(action, false) {
                    log::warn!("Sunrise: releasing controller action {action}: {e}");
                }
            }
        }
        self.pad_cancel_menu(event_loop);
    }

    pub(in crate::game::client_side) fn pad_yield_input(&mut self, event_loop: &dyn EventLoopCtl) {
        if self.client.pad.0.in_use {
            self.client.pad.0.in_use = false;
            self.pad_cancel_inputs(event_loop);
        }
    }

    pub(in crate::game::client_side) fn pad_button_logged(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        button: Button,
        down: bool,
    ) {
        if let Err(e) = self.pad_button(event_loop, button, down) {
            log::warn!("Sunrise: controller {}: {e}", button.name());
        }
    }

    /// Input from the pad counts while the window has focus, or always for
    /// the virtual pad (a script can't focus the window).
    pub(in crate::game::client_side) fn pad_focused(&self) -> bool {
        self.pad_has_window() || self.client.pad.0.virtual_input
    }

    /// The window has focus, or the player lets the pad play without it (Play in Background).
    pub(in crate::game::client_side) fn pad_has_window(&self) -> bool {
        self.client.focused || self.client.settings.controller.play_in_background
    }

    /// Losing focus doesn't pause a game the pad is playing in the background.
    pub(in crate::game::client_side) fn pad_keeps_playing_unfocused(&self) -> bool {
        self.client.settings.controller.enabled
            && self.client.settings.controller.play_in_background
            && self.client.pad.0.in_use
    }

    pub fn pad_button(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        button: Button,
        down: bool,
    ) -> Result<()> {
        if down && !self.pad_focused() {
            return Ok(());
        }
        if !down {
            self.pad_radial_button_up(button);
        }
        // While the radial menu is open its presses are its own, but for the triggers'.
        if down && self.pad_radial_captures() && self.pad_radial_button(button)? {
            return Ok(());
        }
        let playing = !self.pad_in_menu();
        // A thumb rolling off a D-pad opener: its press, and so its release, reach nothing.
        if down && playing && self.pad_radial_settling(button) {
            self.client.pad.0.actions[button as usize].clear();
            return Ok(());
        }
        if down && playing && !self.client.captured && self.client.focused {
            // Like a click into the window: take the pointer.
            self.capture(true);
        }
        if !down && playing {
            let _ = self
                .client
                .screen_state
                .0
                .context_menu_input
                .take_pad(button);
        }
        if !playing {
            self.pad_menu_button(event_loop, button, down)?;
            if down {
                return Ok(());
            }
        } else if button == Button::Start {
            return if down { self.pad_back() } else { Ok(()) };
        }
        // Releases always reach the mappings, so nothing stays held when a
        // screen opened in between.
        let actions = if down {
            let mut actions = self.client.settings.controller.actions_of(button);
            // An opener sharing its button (marked in the binds list) has it to
            // itself wherever it can open, and gives it up elsewhere.
            if actions.contains(&gamepad::RADIAL) {
                if self.pad_radial_can_open() {
                    actions = vec![gamepad::RADIAL];
                } else {
                    actions.retain(|action| *action != gamepad::RADIAL);
                }
            }
            self.client.pad.0.actions[button as usize] = actions.clone();
            actions
        } else {
            std::mem::take(&mut self.client.pad.0.actions[button as usize])
        };
        for action in actions {
            self.pad_action(action, down)?;
        }
        Ok(())
    }

    /// A bound action: a vanilla mapping acts as its default key or mouse
    /// button, like a rebound keyboard key.
    pub fn pad_action(&mut self, action: &str, down: bool) -> Result<()> {
        let Some(route) = gamepad::route(action) else {
            log::warn!("Sunrise: controller action {action} has no handler");
            return Ok(());
        };
        match route {
            Route::Radial => {
                if down {
                    self.pad_radial_open();
                    Ok(())
                } else {
                    self.pad_radial_release()
                }
            }
            Route::HotbarStep { next } => {
                if down {
                    self.hotbar_step(next);
                }
                Ok(())
            }
            Route::Sneak
                if self.client.settings.controller.toggle_sneak
                    && !self.client.settings.toggle_crouch =>
            {
                // A stick click flips sneaking instead of holding it, as
                // vanilla's toggle crouch does: a screen opening releases it
                // and closing restores it (ToggleKeyMapping).
                if !down {
                    return Ok(());
                }
                let screen_open =
                    self.client.screen != Screen::Playing || self.client.title.0.is_some();
                self.client.keys.resume_device_input(&KeyCode::ShiftLeft);
                if !screen_open {
                    let state = self.client.vanilla_mappings.toggle_state_mut(SNEAK);
                    state.pad_toggle();
                    state.click();
                }
                self.key_pressed(KeyCode::ShiftLeft, KeyCode::ShiftLeft)
            }
            Route::Sneak => self.mapped_key(
                KeyCode::ShiftLeft,
                down,
                false,
                KeyCode::ShiftLeft,
                Source::Pad,
            ),
            Route::Screenshot => {
                if down {
                    self.screenshot_action();
                }
                Ok(())
            }
            Route::SmoothCamera => {
                if down {
                    self.queue_mapping_click("key.smoothCamera");
                }
                Ok(())
            }
            Route::Fullscreen => {
                if down {
                    self.toggle_fullscreen();
                }
                Ok(())
            }
            // KeyboardHandler: F3 alone, let go, shows or hides the debug screen.
            Route::DebugOverlay => {
                if down {
                    self.client.debug = !self.client.debug;
                }
                Ok(())
            }
            Route::Held => {
                if let Some(key) = Action::parse(action).and_then(Action::key) {
                    self.mapped_key(key, down, false, key, Source::Pad)?;
                }
                Ok(())
            }
            Route::Mouse(button) => {
                let playing =
                    self.client.title.0.is_none() && self.client.screen == Screen::Playing;
                if !down || playing {
                    self.mouse_mapping(button, down, Source::Pad)?;
                }
                Ok(())
            }
            Route::Key(KeyCode::KeyQ) => {
                self.client.pad.0.drop_stack = false;
                self.client.pad.0.held_drop = down.then(|| {
                    let delay = self.client.settings.controller.repeat_delay;
                    gamepad::HeldDrop::new(Instant::now(), Duration::from_secs_f32(delay))
                });
                self.mapped_key(KeyCode::KeyQ, down, false, KeyCode::KeyQ, Source::Pad)
            }
            Route::Key(key) => self.mapped_key(key, down, false, key, Source::Pad),
        }
    }

    /// Each poll: a held drop button, as a held Q's key repeats reach the mapping.
    pub(in crate::game::client_side) fn pad_drop_frame(&mut self) {
        if self.pad_in_menu() || self.screen_over_playing() || self.pad_radial_captures() {
            return;
        }
        let interval = Duration::from_secs_f32(self.client.settings.controller.repeat_interval);
        let step = self
            .client
            .pad
            .0
            .held_drop
            .as_mut()
            .and_then(|held| held.due(Instant::now(), interval));
        let Some(step) = step else {
            return;
        };
        self.client.pad.0.drop_stack = step == gamepad::DropStep::Stack;
        if let Err(e) = self.mapped_key(KeyCode::KeyQ, true, true, KeyCode::KeyQ, Source::Pad) {
            log::warn!("Sunrise: controller drop: {e}");
        }
    }

    /// B and Start: what Escape does where the pad is.
    pub fn pad_back(&mut self) -> Result<()> {
        if self.context_menu_key(KeyCode::Escape) {
            return Ok(());
        }
        // The death confirm's buttons wait a second, as its Escape does.
        if self
            .client
            .chat_ui
            .death_confirm
            .is_some_and(|shown| shown.elapsed() < DEATH_CONFIRM_WAIT)
        {
            return Ok(());
        }
        if self.prompt_open() {
            return self.pad_prompt_back();
        }
        // The options open over the title screen too: they come first, as
        // for the keyboard's Escape.
        if self.client.screen == Screen::Options {
            return self.options_key(KeyCode::Escape, None).map(|_| ());
        }
        // A mod's screen in front of the title takes the Escape itself.
        if self.title_input_active()
            && let Some(title) = self.client.title.0.as_mut()
        {
            title.escape();
            self.save_title_options();
            return Ok(());
        }
        if self.client.sign_edit.is_some() {
            self.close_sign_editor();
            return Ok(());
        }
        if self.client.book.is_some() {
            self.close_book();
            return Ok(());
        }
        self.key_pressed(KeyCode::Escape, KeyCode::Escape)
    }

    /// The left stick as the move vector, while playing.
    pub fn pad_move(&self) -> Option<[f32; 2]> {
        let settings = &self.client.settings.controller;
        if !settings.enabled
            || self.pad_in_menu()
            || !self.pad_focused()
            || (self.pad_radial_captures() && self.pad_radial_selects(0))
        {
            return None;
        }
        gamepad::move_vector(self.client.pad.0.state.left, settings.move_deadzone)
    }

    /// A changed stick component is input for that direction, even below the digital
    /// threshold. Positive and negative travel take over their own mappings only.
    pub fn resume_stick_mappings(&mut self, previous: Option<[f32; 2]>) {
        let before = previous.unwrap_or_default();
        let after = self.pad_move().unwrap_or_default();
        for (axis, (positive, negative)) in [
            (KeyCode::KeyA, KeyCode::KeyD),
            (KeyCode::KeyW, KeyCode::KeyS),
        ]
        .into_iter()
        .enumerate()
        {
            if before[axis].max(0.0) != after[axis].max(0.0) {
                self.client.keys.resume_device_input(&positive);
            }
            if before[axis].min(0.0) != after[axis].min(0.0) {
                self.client.keys.resume_device_input(&negative);
            }
        }
    }

    /// The right stick turns the view, once per frame (`Entity.turn`).
    pub(in crate::game::client_side) fn pad_look(&mut self, seconds: f32) {
        let settings = &self.client.settings.controller;
        if !settings.enabled
            || self.pad_in_menu()
            || self.common.player.is_none()
            || !self.pad_focused()
            || (self.pad_radial_captures() && self.pad_radial_selects(1))
        {
            return;
        }
        // A long frame (a hitch) must not spin the view.
        let seconds = seconds.min(0.1);
        let turn = gamepad::look_delta(
            self.client.pad.0.state.right,
            settings,
            seconds,
            self.scoping(),
        );
        // The cinematic camera eases the stick's turn in and out, as it does the mouse's.
        let (yaw, pitch) = self
            .client
            .pad
            .0
            .look
            .turn(turn, seconds, self.client.smooth_camera);
        if yaw == 0.0 && pitch == 0.0 {
            return;
        }
        self.look_turn(yaw / 0.15, pitch / 0.15);
    }

    /// Vibrate the active pad, when vibration is on and the pad is the one
    /// playing (a pad lying idle while the mouse plays stays still).
    pub(in crate::game::client_side) fn pad_rumble(&mut self, rumble: Rumble) {
        let settings = &self.client.settings.controller;
        if !settings.enabled || !settings.vibration || !self.client.pad.0.in_use {
            return;
        }
        let Some(pulse) = rumble.pulse() else {
            return;
        };
        self.client.pad.0.last_rumble = Some((rumble.name(), pulse));
        self.pad_vibrate(rumble, pulse);
    }

    /// Show a controller connection change without changing which device drives the game.
    pub(in crate::game::client_side) fn pad_connection_notice(&mut self, change: Connection) {
        // The pad that was playing is gone: nothing is driving until another is touched.
        if matches!(change, Connection::Disconnected { active: true }) {
            self.client.pad.0.in_use = false;
        }
        if !self.client.settings.controller.enabled {
            return;
        }
        let label = sunrise_game_data::lang::get;
        let notice = match change {
            Connection::Connected(family) => Notice::new(
                label("sunrise.controller.notice.controller_connected"),
                label("sunrise.pad.notice.connected.press"),
            )
            // In the new pad's own glyphs, its letters included: pressed, it plays.
            .with_glyph(Glyph::menu(
                MenuJob::Confirm,
                Glyphs::of(&self.client.settings.controller, family),
            )),
            Connection::Disconnected { .. } => Notice::new(
                label("sunrise.controller.notice.controller_disconnected"),
                label("sunrise.controller.notice.controller_disconnected.hint"),
            ),
        };
        let clock = sunrise_client::toasts::ToastClock::of(self.common.clock.still());
        self.client.controller_notice.raise(notice, clock);
    }

    /// Connection notices sit below the boss bars and last as long as a toast.
    pub(in crate::game::client_side) fn draw_controller_notice(
        &mut self,
        batch: &mut HudBatch,
        clock: sunrise_client::toasts::ToastClock,
    ) {
        let shown_for =
            Duration::from_secs_f64(5.0 * self.client.settings.notification_display_time);
        self.client.controller_notice.draw(
            batch,
            &self.client.gui.atlas,
            self.client.chat_ui.room.below_top(18.0),
            clock,
            shown_for,
        );
    }

    /// `sunrise-ctl pad ...`.
    pub(in crate::game::client_side) fn pad_command(&mut self, words: &[&str]) -> Result<Value> {
        let arg = |i: usize| -> Result<&str> {
            words
                .get(i)
                .copied()
                .ok_or_else(|| format!("usage: {}", HELP[0]).into())
        };
        let number = |i: usize| -> Result<f32> {
            let v: f32 = arg(i)?.parse()?;
            if v.is_finite() {
                Ok(v)
            } else {
                Err("not a number".into())
            }
        };
        match arg(1)? {
            "status" => {
                let s = &self.client.pad.0.state;
                let held: Vec<&str> = Button::ALL
                    .into_iter()
                    .filter(|b| self.client.pad.0.edges.is_held(*b))
                    .map(Button::name)
                    .collect();
                let mut status = json!({
                    "enabled": self.client.settings.controller.enabled,
                    "glyphs": self.client.pad.0.style.name(),
                    "last_rumble": self.client.pad.0.last_rumble.map(|(why, p)| json!({
                        "reason": why, "strong": p.strong, "weak": p.weak, "ms": p.millis,
                    })),
                    "in_use": self.client.pad.0.in_use,
                    "notice": self.client.controller_notice.showing().map(|(title, body)| {
                        json!({ "title": title, "body": body })
                    }),
                    "menu": self.pad_menu_status(),
                    "radial": self.pad_radial_status(),
                    "guide": self.pad_guide_status(),
                    "held": held,
                    "left": s.left,
                    "right": s.right,
                    "triggers": s.triggers,
                });
                if let (Some(status), Value::Object(devices)) =
                    (status.as_object_mut(), self.pad_devices_status())
                {
                    status.extend(devices);
                }
                Ok(status)
            }
            "connection" => {
                let change = match arg(2)? {
                    "connected" => Connection::Connected(match words.get(3) {
                        Some(family) => style_named(family)?,
                        None => GlyphStyle::PlayStation,
                    }),
                    "disconnected" => Connection::Disconnected { active: true },
                    other => {
                        return Err(format!(
                            "connection must be connected or disconnected, not {other}"
                        )
                        .into());
                    }
                };
                self.client.pad.0.connections.push(change);
                Ok(json!({ "ok": true }))
            }
            // The pad playing poses as this family's: its glyphs, and a Nintendo one's letters.
            "family" => {
                self.client.pad.0.style = style_named(arg(2)?)?;
                Ok(json!({ "ok": true }))
            }
            "release" => {
                self.client.pad.0.menu.armed = None;
                self.client.pad.0.radial.cancel();
                self.direct_craft_stop();
                self.client.pad.0.injected = PadState::default();
                self.client.pad.0.taps.clear();
                Ok(json!({ "ok": true }))
            }
            "stick" => {
                let v = [number(3)?.clamp(-1.0, 1.0), number(4)?.clamp(-1.0, 1.0)];
                match arg(2)? {
                    "left" => self.client.pad.0.injected.left = v,
                    "right" => self.client.pad.0.injected.right = v,
                    other => return Err(format!("stick must be left or right, not {other}").into()),
                }
                Ok(json!({ "ok": true }))
            }
            "trigger" => {
                let v = number(3)?.clamp(0.0, 1.0);
                match arg(2)? {
                    "left" => self.client.pad.0.injected.triggers[0] = v,
                    "right" => self.client.pad.0.injected.triggers[1] = v,
                    other => {
                        return Err(format!("trigger must be left or right, not {other}").into());
                    }
                }
                Ok(json!({ "ok": true }))
            }
            name => {
                let button = Button::parse(name)
                    .ok_or_else(|| format!("unknown pad button {name}; {}", HELP[0]))?;
                match arg(2)? {
                    "down" => self.client.pad.0.injected.set(button, true),
                    "up" => self.client.pad.0.injected.set(button, false),
                    "tap" => {
                        self.client.pad.0.injected.set(button, true);
                        self.client.pad.0.taps.push(button);
                    }
                    other => {
                        return Err(
                            format!("pad action must be down, up or tap, not {other}").into()
                        );
                    }
                }
                Ok(json!({ "ok": true }))
            }
        }
    }
}
