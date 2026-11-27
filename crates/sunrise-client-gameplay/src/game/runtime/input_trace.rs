//! Who caused an input event: a person at the keyboard (`user`), an agent on the
//! control socket (`agent`), or the game itself (`game`, made-up input no one
//! asked for). Agents read this with `input-log` and the `input` section of
//! `state` and `status`, so a script can tell that a person took over instead of
//! mistaking their keystrokes for its own. A `user-input` event also lands in
//! `events` (and so in `wait-for event user-input`) when a person steps in
//! while an agent was driving.
//!
//! The events are also what a repro recording writes to `inputs.jsonl`
//! (game/runtime/repro.rs).
use super::Screen;
use serde_json::json;
use sunrise_client::{keybinds, window::PointerSample};
use winit::{
    event::{ElementState, MouseScrollDelta, WindowEvent},
    keyboard::PhysicalKey,
};

pub(in crate::game) use sunrise_client_gameplay::input_trace::{
    HELP, InputOrigin, InputTrace, TraceEvent, TraceKind, is_read_only, is_session_command,
    redact_command,
};

impl crate::game::ClientGame<'_> {
    /// An agent's control line: input unless it only reads, which still counts
    /// as the agent watching.
    pub(in crate::game::client_side) fn note_socket_line(&mut self, line: &str) {
        if is_read_only(line) || is_session_command(line) {
            self.client.input_trace.0.agent_activity();
            return;
        }
        self.note_input(
            InputOrigin::Agent,
            TraceKind::Command {
                line: redact_command(line),
            },
        );
    }

    /// Whether anything can read the trace: the control socket (agent control
    /// on), the repro shortcuts, or a recording under way. Otherwise every key
    /// and mouse motion skips it, so a player who uses none of them pays
    /// nothing per input event.
    fn tracing_input(&self) -> bool {
        self.common.control.is_some()
            || self.client.settings.extras.repro_keys
            || self.client.repro.0.recording_active()
    }

    /// Record an input event and, when a person steps into an agent's run, tell
    /// every watcher once.
    pub(in crate::game::client_side) fn note_input(
        &mut self,
        origin: InputOrigin,
        kind: TraceKind,
    ) {
        if !self.tracing_input() {
            return;
        }
        let kind_name = kind.name();
        // An agent waiting on a task or a wait, or a client still connected, is
        // watching even when its last line was minutes ago.
        let agent_active = self
            .common
            .control
            .as_ref()
            .is_some_and(|c| c.clients() > 0)
            || self.agent_busy()
            || super::events::has_waits(&self.common.events);
        if self
            .client
            .input_trace
            .0
            .record(self.common.ticks, origin, kind, agent_active)
        {
            let event = json!({ "event": "user-input", "input": kind_name });
            self.client.repro.0.note_event(self.common.ticks, &event);
            super::events::note(&mut self.common.events, self.common.ticks, event);
            log::info!(
                "Sunrise: {kind_name} input from a person while an agent was driving \
                 (input-log has the run)"
            );
        }
    }

    /// Keep the window events that are input. Agent-made window events are left
    /// out: the control-socket line that caused them is recorded instead.
    pub(in crate::game::client_side) fn trace_window_event(
        &mut self,
        origin: InputOrigin,
        event: &WindowEvent,
        cursor_sample: Option<PointerSample>,
    ) {
        if origin == InputOrigin::Agent || !self.tracing_input() {
            return;
        }
        match event {
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                let Some(name) = keybinds::key_name(code) else {
                    return;
                };
                let action = match (event.state, event.repeat) {
                    (ElementState::Released, _) => "up",
                    (_, true) => "repeat",
                    (ElementState::Pressed, false) => "down",
                };
                // The platform made this one up (an IME's dead key): not a person.
                let origin = if *is_synthetic {
                    InputOrigin::Game
                } else {
                    origin
                };
                self.note_input(
                    origin,
                    TraceKind::Key {
                        key: name.to_owned(),
                        action: action.to_owned(),
                    },
                );
            }
            WindowEvent::MouseInput { state, button, .. } => self.note_input(
                origin,
                TraceKind::Mouse {
                    button: keybinds::mouse_name(*button),
                    action: match state {
                        ElementState::Pressed => "down".to_owned(),
                        ElementState::Released => "up".to_owned(),
                    },
                },
            ),
            WindowEvent::MouseWheel { delta, .. } => {
                let (x, y, lines) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (f64::from(*x), f64::from(*y), true),
                    MouseScrollDelta::PixelDelta(p) => (p.x, p.y, false),
                };
                self.note_input(origin, TraceKind::Wheel { x, y, lines });
            }
            // GUI clicks land where the pointer is; in the world the pointer is
            // grabbed and turns the view through `device_event` instead.
            WindowEvent::CursorMoved { .. } if self.client.screen != Screen::Playing => {
                if let Some(sample) = cursor_sample {
                    self.note_input(origin, TraceKind::cursor(sample));
                }
            }
            _ => {}
        }
    }
}
