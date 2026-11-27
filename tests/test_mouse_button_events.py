"""Run the actual raw mouse dispatch with a fake mod and window events."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from tests.test_world_input_capture import method

ROOT = Path(__file__).resolve().parents[1]


class MouseButtonEventsTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_order_cancellation_release_and_synthetic_input(self):
        source = (ROOT / "crates/sunrise-client-gameplay/src/game/runtime/window_handler.rs").read_text(encoding="utf-8")
        outer = method(source, "handle_window_event_as")
        dispatch = outer[outer.index("        if let WindowEvent::MouseInput { state, button, .. } = &event {"):]
        fixture = r"""
use std::collections::HashSet;
#[derive(Clone, Copy, PartialEq)] enum ElementState { Pressed, Released }
struct PointerSample;
enum WindowEvent { MouseInput { state: ElementState, button: i32 }, Resized(()), Other }
mod input_trace {
    #[derive(Clone, Copy, PartialEq)]
    pub enum InputOrigin { User, Agent }
}
use input_trace::InputOrigin;
fn glfw_button(button: i32) -> i32 { button }
fn glfw_modifiers(modifiers: u32) -> u32 { modifiers }
struct ClientState { physical_buttons: HashSet<i32>, modifiers: u32, container_owned_up: bool, physical_presses: u64 }
struct Game { client: ClientState, synthetic: bool, cancel: bool,
              early: bool, calls: Vec<&'static str>, press_millis: Option<u64> }
impl Game {
    fn pad_synthetic_input(&self) -> bool { self.synthetic }
    // A container hook's press latch: retired at a real button-up, before the raw Pre.
    fn container_button_up(&mut self, button: i32) -> bool {
        assert!(!self.client.physical_buttons.contains(&button));
        self.calls.push("up"); false
    }
    fn native_mouse_button_pre(&mut self, button: i32, pressed: bool, modifiers: u32, canceled: &mut bool) {
        assert_eq!((button, modifiers), (2, 5));
        assert_eq!(self.client.physical_buttons.contains(&button), pressed);
        self.calls.push("pre"); *canceled = self.cancel;
    }
    fn native_mouse_button_post(&mut self, button: i32, pressed: bool, modifiers: u32) {
        assert_eq!((button, modifiers), (2, 5));
        assert_eq!(self.client.physical_buttons.contains(&button), pressed);
        self.calls.push("post");
    }
    fn mod_mappings_button(&mut self, button: i32, pressed: bool) {
        assert_eq!(button, 2);
        assert_eq!(self.client.physical_buttons.contains(&button), pressed);
        self.calls.push("mappings");
    }
    fn millis(&self) -> u64 { 100 }
    fn settle_title_mode(&mut self) { self.calls.push("settle"); }
    fn notify_window_resize(&mut self) {}
    fn handle_window_event_inner_as(&mut self, _: (), _: WindowEvent, cursor_sample: Option<PointerSample>,
                                    press_millis: Option<u64>, _: InputOrigin) -> bool {
        assert!(cursor_sample.is_none());
        self.press_millis = press_millis;
        self.calls.push("handle");
        if self.early { return false; }
        self.calls.push("finish");
        true
    }
    fn dispatch(&mut self, event: WindowEvent, origin: InputOrigin) {
        let event_loop = ();
        let cursor_sample = None;
DISPATCH
}
#[test]
fn dispatch_order_and_bookkeeping() {
    for state in [ElementState::Pressed, ElementState::Released] {
        for (cancel, synthetic, early, origin, mut expected) in [
            (false, false, false, InputOrigin::User, vec!["pre", "handle", "finish", "post"]),
            (true, false, false, InputOrigin::User, vec!["pre"]),
            (false, false, true, InputOrigin::User, vec!["pre", "handle", "settle", "post"]),
            (false, false, false, InputOrigin::Agent, vec!["pre", "handle", "finish", "post"]),
            (true, true, false, InputOrigin::User, vec!["handle", "finish"])] {
            let mut game = Game {
                client: ClientState { physical_buttons: HashSet::from([2]), modifiers: 5,
                    container_owned_up: false, physical_presses: 0 },
                synthetic, cancel, early, calls: vec![], press_millis: None };
            game.dispatch(WindowEvent::MouseInput { state, button: 2 }, origin);
            let pressed = state == ElementState::Pressed;
            if !pressed && !synthetic {
                expected.insert(0, "up");
                expected.insert(2, "mappings");
            }
            assert_eq!(game.calls, expected);
            assert_eq!(game.client.physical_presses, u64::from(pressed));
            assert_eq!(game.client.physical_buttons.contains(&2), pressed);
            assert_eq!(game.press_millis,
                (pressed && !cancel && !synthetic && origin == InputOrigin::User).then_some(100));
        }
    }
    let mut game = Game {
        client: ClientState { physical_buttons: HashSet::new(), modifiers: 5,
            container_owned_up: false, physical_presses: 0 },
        synthetic: false, cancel: true, early: false, calls: vec![], press_millis: None };
    game.dispatch(WindowEvent::Other, InputOrigin::User);
    assert_eq!(game.calls, ["handle", "finish"]);
}
""".replace("DISPATCH", dispatch)
        with tempfile.TemporaryDirectory() as folder:
            src, exe = Path(folder) / "mouse.rs", Path(folder) / "mouse.exe"
            src.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
