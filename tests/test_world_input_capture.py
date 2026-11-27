"""Exercise the client's capture methods without opening or focusing an OS window.

Capture fixtures substitute winit; the raw-motion fixture uses the real mouse
accumulator and rotation math. Method bodies come from the client, so assertions
observe capture and camera outcomes. Vanilla 26.3 reference:
LevelLoadingScreen.tick/onClose, Gui.setScreen, MouseHandler.grabMouse/releaseMouse.
"""
from pathlib import Path
import os
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


def method(source, name):
    # Strip moved-method visibility (`pub`, `pub(crate)`, `pub(super)`, `pub(in crate::game)`, or side-private) for the private fixture.
    vis_prefixes = (
        "pub fn",
        "pub(crate) fn",
        "pub(super) fn",
        "pub(in crate::game) fn",
        "pub(in crate::game::client_side) fn",
        "pub(in crate::game::server_side) fn",
    )
    candidates = [f"    fn {name}(", *(f"    {vis} {name}(" for vis in vis_prefixes)]
    at = next((source.index(c) for c in candidates if c in source), None)
    if at is None:
        raise ValueError(f"{name} not found")
    start = at
    opening = source.index("{", start)
    depth = 1
    end = opening + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    body = source[start:end]
    for vis in vis_prefixes:
        body = body.replace(vis, "fn", 1)
    return body


def function(source, name):
    start = source.index(f"pub fn {name}(")
    opening = source.index("{", start)
    depth = 1
    end = opening + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end]


def pointer_projection_sources():
    source = (ROOT / "crates/sunrise-client-input/src/window.rs").read_text(encoding="utf-8")
    source = source[source.index("/// `Window.calculateScale`"):source.index("/// `InactivityFpsLimit`.")]
    # The OS query is stubbed, not the projection or its rounding.
    return source.replace(function(source, "from_window"), "")


class WorldInputCaptureTests(unittest.TestCase):
    def test_loading_completion_wiring(self):
        game = (ROOT / "crates/sunrise-gameplay/src/game/world/player_spawn.rs").read_text(encoding="utf-8")
        remote_dir = ROOT / "crates/sunrise-client-gameplay/src/game/network/remote"
        tick = (remote_dir / "tick_client.rs").read_text(encoding="utf-8")
        level_tick = (remote_dir / "client_level_tick_client.rs").read_text(encoding="utf-8")
        # Server-only spawn_player initializes self.common.player without client mouse capture;
        # client loading completion (singleplayer integrated server and multiplayer) captures
        # the pointer in tick_client_load below.
        spawn = method(game, "spawn_player")
        self.assertIn("self.common.player = Some(player)", spawn)
        self.assertNotIn("self.capture_loaded_world()", spawn)
        # ClientPacketListener.tick: the level load tracker ends the loading screen,
        # the server hears PlayerLoaded, the mouse is captured, then the player moves.
        ready = method(tick, "tick_client_load")
        self.assertLess(ready.index("r.loaded_sent = true"),
                        ready.index("self.capture_loaded_world()"))
        level_fn = method(level_tick, "tick_remote_level_body")
        self.assertIn("self.tick_remote_player(running, hold)", level_fn)
        level = method(tick, "tick_remote_player")
        self.assertLess(level.index("self.tick_client_load()"),
                        level.index("self.tick_remote_own_player()"))
        self.assertIn("self.tick_local_player(", method(tick, "tick_remote_own_player"))
        # The extracted local-player pass owns the movement input now.
        self.assertIn("let stick = self.pad_move()", method(tick, "tick_local_player"))

    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_capture_lifecycle(self):
        source = "".join((ROOT / "crates/sunrise-client-gameplay/src" / path).read_text(encoding="utf-8")
                         for path in ("game/game_client.rs", "game/interface/pointer_capture.rs"))
        fixture = r'''
use std::cell::Cell;
#[derive(Clone, Copy, PartialEq)]
enum Screen { Playing, Paused, Inventory, Chat }
#[derive(Clone, Copy)]
enum CursorGrabMode { Locked, Confined, None }
mod winit {
    pub mod dpi {
        #[derive(Clone, Copy, Debug)]
        pub struct PhysicalPosition<T> { pub x: T, pub y: T }
        impl<T> PhysicalPosition<T> { pub fn new(x: T, y: T) -> Self { Self { x, y } } }
        #[derive(Clone, Copy, Debug)]
        pub struct LogicalPosition<T> { pub x: T, pub y: T }
        impl<T> LogicalPosition<T> { pub fn new(x: T, y: T) -> Self { Self { x, y } } }
        pub struct PhysicalSize<T> { pub width: T, pub height: T }
        impl From<PhysicalSize<u32>> for [u32; 2] {
            fn from(size: PhysicalSize<u32>) -> Self { [size.width, size.height] }
        }
    }
}
use winit::dpi::{PhysicalPosition, PhysicalSize};
extern crate self as sunrise_client;
mod window {
    use super::{Window, winit::dpi::{LogicalPosition, PhysicalPosition}};
    POINTER_PROJECTION
    impl PointerSpace {
        pub fn from_window(window: &Window) -> Self { Self::new(window.inner_size().into(), 1.0) }
    }
}
struct GuiScale { requested: u32, unicode: bool }
struct Settings;
impl Settings { fn gui_scale_inputs(&self) -> GuiScale { GuiScale { requested: 2, unicode: false } } }
struct Config { width: u32, height: u32 }
struct Renderer { config: Config }
#[derive(Default)]
struct Window { grabs: Cell<u32>, visible: Cell<bool>, fail: bool, warps: Cell<u32>, at: Cell<(f64, f64)>,
                focus: Cell<bool>, size: Option<[u32; 2]> }
impl Window {
    fn inner_size(&self) -> PhysicalSize<u32> {
        let [width, height] = self.size.unwrap_or([854, 480]);
        PhysicalSize { width, height }
    }
    fn set_cursor_position(&self, position: PhysicalPosition<f64>) -> Result<(), ()> {
        self.warps.set(self.warps.get() + 1);
        self.at.set((position.x, position.y));
        Ok(())
    }
    fn set_cursor_grab(&self, mode: CursorGrabMode) -> Result<(), ()> {
        if !matches!(mode, CursorGrabMode::None) {
            self.grabs.set(self.grabs.get() + 1);
        }
        if self.fail { Err(()) } else { Ok(()) }
    }
    fn set_cursor_visible(&self, visible: bool) { self.visible.set(visible); }
    fn id(&self) -> WindowId { WindowId }
    fn has_focus(&self) -> bool { self.focus.get() }
}
#[derive(Clone, Copy)]
struct WindowId;
impl WindowId { fn dummy() -> Self { WindowId } }
enum WindowEvent { Focused(bool), ModifiersChanged(u8) }
mod input_trace { pub enum InputOrigin { User } }
trait EventLoopCtl {}
struct Loop;
impl EventLoopCtl for Loop {}
#[derive(Default)]
struct Mouse { ignore_first: bool }
impl Mouse { fn set_ignore_first_move(&mut self) { self.ignore_first = true; } }
struct Remote;
#[derive(Default)]
struct StudioFreecam { active: bool }
#[derive(Default)]
struct Studio { freecam: StudioFreecam, rmb_down: bool }
impl Studio { fn flying(&self) -> bool { self.freecam.active } }
#[derive(Default)]
struct Replay { driver: Option<()> }
impl Replay { fn playing(&self) -> bool { self.driver.is_some() } }
#[derive(Default)]
struct Options { offscreen: bool }
#[derive(Default)]
struct ServerPacks { prompt: Option<()> }
#[derive(Default)]
struct ScreenState { answered: Cell<bool> }
struct ClientState {
    window: (Option<Box<Window>>,), title: (Option<()>,),
    screen_state: (ScreenState,), renderer: (Option<Renderer>,),
    server_packs: (ServerPacks,), command_prompt: (Option<()>,),
    settings: Settings, lan_acting: Option<u32>,
    focused: bool, agent_pointer: bool, mouse: Mouse,
    captured: bool, capture_mode: Option<CursorGrabMode>, regrab_on_refocus: bool, screen: Screen,
    studio: Studio, cursor: (f32, f32),
    window_cursor: Option<window::PointerSample>,
    window_pointer_space: std::cell::OnceCell<window::PointerSpace>,
    unclaimed_window: bool, focus_after_run: Option<bool>, screen_grab: bool,
    unclaimed_modifiers: Option<u8>,
}
impl Default for ClientState {
    fn default() -> Self {
        Self { window: (Some(Box::default()),), title: (None,),
            screen_state: (ScreenState::default(),), renderer: (None,),
            server_packs: (ServerPacks::default(),), command_prompt: (None,),
            settings: Settings, lan_acting: None,
            focused: true, agent_pointer: false, mouse: Mouse::default(),
            captured: false, capture_mode: None, regrab_on_refocus: false,
            screen: Screen::Playing, studio: Studio::default(),
            cursor: (0.0, 0.0), window_cursor: None,
            window_pointer_space: std::cell::OnceCell::new(),
            unclaimed_window: false,
            focus_after_run: None, screen_grab: false, unclaimed_modifiers: None }
    }
}
struct CommonGame { options: Options }
impl Default for CommonGame {
    fn default() -> Self { Self { options: Options::default() } }
}
struct ClientGame {
    common: CommonGame,
    remote: Option<Remote>, replay: Replay, mining: bool,
    scripted: bool, applied_focus: Vec<bool>, applied_modifiers: Vec<u8>,
    client: ClientState,
}
impl Default for ClientGame {
    fn default() -> Self {
        Self { common: CommonGame::default(), remote: None, replay: Replay::default(), mining: false,
            scripted: false, applied_focus: Vec::new(), applied_modifiers: Vec::new(),
            client: ClientState::default() }
    }
}
impl ClientGame {
    fn stop_mining(&mut self) { self.mining = false; }
    fn studio_up(&self) -> bool { self.replay.playing() || self.client.studio.flying() }
    fn reset_inventory_clicks(&mut self) {}
    fn restore_held_keys(&mut self) {} // This fixture exercises pointer state, not key mappings.
    fn dialog_open(&self) -> bool { false }
    fn scripted_run(&self) -> bool { self.scripted }
    fn handle_window_event(&mut self, _: &dyn EventLoopCtl, _: WindowId, event: WindowEvent,
                           _: input_trace::InputOrigin) {
        // The `Focused` arm of the game's handler, as far as the pointer goes.
        match event {
            WindowEvent::Focused(focused) => {
                self.applied_focus.push(focused);
                self.client.focused = focused;
                if focused { self.regrab_on_refocus(); }
            }
            WindowEvent::ModifiersChanged(modifiers) => self.applied_modifiers.push(modifiers),
        }
    }
METHODS
}
fn main() {
    // The server pack prompt keeps the pointer: nothing grabs it while it is open.
    {
        let mut game = ClientGame::default();
        game.client.server_packs.0.prompt = Some(());
        game.capture(true);
        assert!(!game.client.captured);
        game.client.server_packs.0.prompt = None;
        game.capture(true);
        assert!(game.client.captured);
    }
    // So does a clicked command's confirmation.
    {
        let mut game = ClientGame::default();
        game.client.command_prompt.0 = Some(());
        game.capture(true);
        assert!(!game.client.captured);
        game.client.command_prompt.0 = None;
        game.capture(true);
        assert!(game.client.captured);
    }
    // New and existing worlds, returning to title, and reopening a world.
    for remote in [false, true] {
        let mut game = ClientGame::default();
        if remote { game.remote = Some(Remote); }
        for _ in 0..3 {
            game.client.title.0 = Some(());
            game.capture(false);
            game.capture_loaded_world();
            assert!(!game.client.captured);
            game.client.title.0 = None;
            game.capture_loaded_world();
            assert!(game.client.captured);
            assert!(game.client.mouse.ignore_first);
            assert!(!game.client.window.0.as_ref().unwrap().visible.get());
            for screen in [Screen::Paused, Screen::Inventory] {
                game.client.screen = screen;
                game.mining = true;
                game.capture(false);
                assert!(!game.mining);
                game.capture_loaded_world();
                assert!(!game.client.captured);
                assert!(game.client.window.0.as_ref().unwrap().visible.get());
                game.client.screen = Screen::Playing;
                game.capture(true);
                assert!(game.client.captured);
            }
        }
        // Alt+Tab during loading: no native grab or agent-pointer reset.
        game.client.focused = false;
        game.mining = true;
        game.capture(false);
        assert!(!game.mining);
        assert!(!game.client.captured);
        assert!(game.client.window.0.as_ref().unwrap().visible.get());
        game.client.agent_pointer = true;
        let grabs = game.client.window.0.as_ref().unwrap().grabs.get();
        game.capture_loaded_world();
        game.capture(true);
        assert!(!game.client.captured);
        assert!(game.client.agent_pointer);
        assert_eq!(game.client.window.0.as_ref().unwrap().grabs.get(), grabs);
        game.client.focused = true;
        assert!(!game.client.captured); // Setting the flag is not a grab: `capture` is.
        game.capture(true); // `regrab_on_refocus`, or a click on the world.
        assert!(game.client.captured);
        assert_eq!(game.client.cursor, (213.5, 120.0)); // The grab puts the mouse at the centre.
    }
    // A window that held the pointer asks for it back when the focus returns:
    // the system drops a grabbed pointer with the focus and vanilla's grab
    // outlives it (`MouseHandler.mouseGrabbed` stays true while GLFW keeps its
    // cursor mode), so no click is needed to lock it again.
    {
        let mut game = ClientGame::default();
        game.capture(true);
        assert!(game.client.captured);
        let grabs = game.client.window.0.as_ref().unwrap().grabs.get();
        game.client.focused = false; // Alt+Tab: the release `window_event` makes.
        game.client.regrab_on_refocus = game.client.captured;
        game.capture(false);
        assert!(!game.client.captured);
        game.client.focused = true;
        game.regrab_on_refocus();
        assert!(game.client.captured);
        assert!(game.client.window.0.as_ref().unwrap().grabs.get() > grabs);
        assert!(game.client.mouse.ignore_first);
        // Taken once: a focus gain that follows a screen's release takes nothing.
        game.capture(false);
        let grabs = game.client.window.0.as_ref().unwrap().grabs.get();
        game.regrab_on_refocus();
        assert!(!game.client.captured);
        assert_eq!(game.client.window.0.as_ref().unwrap().grabs.get(), grabs);
        // A screen open while the window was away keeps the pointer free.
        game.client.regrab_on_refocus = true;
        game.client.screen = Screen::Inventory;
        game.regrab_on_refocus();
        assert!(!game.client.captured);
        assert_eq!(game.client.window.0.as_ref().unwrap().grabs.get(), grabs);
        // A window that never held the pointer does not take it on focus.
        game.client.screen = Screen::Playing;
        game.regrab_on_refocus();
        assert!(!game.client.captured);
        assert_eq!(game.client.window.0.as_ref().unwrap().grabs.get(), grabs);
        // A prompt over the world keeps the pointer, as the grab itself checks.
        game.client.regrab_on_refocus = true;
        game.client.server_packs.0.prompt = Some(());
        game.regrab_on_refocus();
        assert!(!game.client.captured);
    }
    // Studio replays keep the pointer released after focus returns; camera look
    // only captures it while the right mouse button is held.
    {
        let mut game = ClientGame::default();
        game.replay.driver = Some(());
        game.client.studio.rmb_down = true;
        game.capture(true);
        assert!(game.client.captured);
        game.client.regrab_on_refocus = true;
        game.client.focused = false;
        game.capture(false);
        game.client.focused = true;
        game.regrab_on_refocus();
        assert!(!game.client.captured);
        assert!(!game.client.studio.rmb_down);
    }
    // The native pointer and the GUI cursor are centred in their own units.
    {
        let mut game = ClientGame::default();
        game.client.window.0.as_mut().unwrap().size = Some([1000, 600]);
        game.client.renderer.0 = Some(Renderer { config: Config { width: 1000, height: 600 } });
        game.capture(true);
        assert!(game.client.captured);
        assert_eq!(game.client.window.0.as_ref().unwrap().at.get(), (500.0, 300.0));
        assert_eq!(game.client.cursor, (250.0, 150.0));
        game.capture(false);
        assert!(!game.client.captured);
        assert_eq!(game.client.window.0.as_ref().unwrap().at.get(), (500.0, 300.0));
        assert_eq!(game.client.cursor, (250.0, 150.0));
    }
    // `--offscreen` shows no window: the grab is the game's own, and the system
    // pointer is never grabbed, warped or hidden.
    {
        let mut game = ClientGame::default();
        game.common.options.offscreen = true;
        game.client.window.0.as_ref().unwrap().visible.set(true);
        game.capture_loaded_world();
        assert!(game.client.captured);
        assert!(game.client.mouse.ignore_first);
        game.capture(false);
        assert!(!game.client.captured);
        game.capture(true);
        assert!(game.client.captured);
        let window = game.client.window.0.as_ref().unwrap();
        assert_eq!((window.grabs.get(), window.warps.get()), (0, 0));
        assert!(window.visible.get());
    }
    // An agent's window no one has clicked yet keeps its grab to itself as `--offscreen`
    // does, focused or not: the person's pointer is never grabbed, warped or hidden.
    for focused in [true, false] {
        let mut game = ClientGame::default();
        game.client.unclaimed_window = true;
        game.client.focused = focused;
        game.client.window.0.as_ref().unwrap().visible.set(true);
        game.capture_loaded_world();
        assert!(game.client.captured);
        game.capture(false);
        game.capture(true);
        let window = game.client.window.0.as_ref().unwrap();
        assert_eq!((window.grabs.get(), window.warps.get()), (0, 0));
        assert!(window.visible.get());
        // The person's press inside it claims it: the game's own grab is dropped, so that
        // press takes the system pointer (`MouseHandler.onPress`) instead of attacking. The
        // game hears of the focus the window has by then, if it didn't know.
        game.client.window.0.as_ref().unwrap().focus.set(true);
        game.claim_window(&Loop);
        assert!(!game.client.unclaimed_window);
        assert!(!game.client.captured);
        assert_eq!(game.applied_focus, if focused { vec![] } else { vec![true] });
        game.capture(true);
        assert!(game.client.captured);
        let window = game.client.window.0.as_ref().unwrap();
        assert_eq!(window.grabs.get(), 1);
        assert!(!window.visible.get());
    }
    // A press that reaches a window without the focus claims it too, and the game hears that
    // the focus is elsewhere: nothing is grabbed until it comes, as in any window.
    {
        let mut game = ClientGame::default();
        game.client.unclaimed_window = true;
        game.claim_window(&Loop);
        assert_eq!(game.applied_focus, vec![false]);
        game.capture(true);
        assert!(!game.client.captured);
        assert_eq!(game.client.window.0.as_ref().unwrap().grabs.get(), 0);
    }
    // An agent's `pad focus false` in play leaves a regrab owed. The claim drops it: the focus
    // catching up takes nothing, so the press still finds the pointer free and takes it.
    {
        let mut game = ClientGame::default();
        game.client.unclaimed_window = true;
        game.capture_loaded_world();
        game.client.focused = false;
        game.client.regrab_on_refocus = true;
        game.client.window.0.as_ref().unwrap().focus.set(true);
        game.claim_window(&Loop);
        assert_eq!(game.applied_focus, vec![true]);
        assert!(!game.client.captured, "the press must take the pointer, not attack");
        assert_eq!(game.client.window.0.as_ref().unwrap().grabs.get(), 0);
    }
    // A Shift the person held before the click reaches the game at the claim, after the focus.
    {
        let mut game = ClientGame::default();
        game.client.unclaimed_window = true;
        game.client.unclaimed_modifiers = Some(1);
        game.client.window.0.as_ref().unwrap().focus.set(true);
        game.claim_window(&Loop);
        assert_eq!(game.applied_modifiers, vec![1]);
        assert_eq!(game.client.unclaimed_modifiers, None);
    }
    // A scripted run's grab is the game's own, even in a window the person clicked into,
    // and once it ends the system pointer follows: a run that closed a screen leaves the
    // pointer to be taken then, as the window's own grab would have.
    {
        let mut game = ClientGame::default();
        game.client.window.0.as_ref().unwrap().visible.set(true);
        game.scripted = true;
        game.capture(true);
        assert!(game.client.captured);
        let window = game.client.window.0.as_ref().unwrap();
        assert_eq!((window.grabs.get(), window.warps.get()), (0, 0));
        assert!(window.visible.get());
        game.settle_after_run(&Loop);
        assert_eq!(game.client.window.0.as_ref().unwrap().grabs.get(), 0, "nothing moves while it runs");
        game.scripted = false;
        game.settle_after_run(&Loop);
        assert!(game.client.captured);
        let window = game.client.window.0.as_ref().unwrap();
        assert_eq!(window.grabs.get(), 1);
        assert!(!window.visible.get());
        game.settle_after_run(&Loop);
        assert_eq!(game.client.window.0.as_ref().unwrap().grabs.get(), 1, "settled once");
    }
    // A pointer the window held as the run began stays the system's while it runs, and the
    // run's release lets it go when the run ends.
    {
        let mut game = ClientGame::default();
        game.capture(true);
        let warps = game.client.window.0.as_ref().unwrap().warps.get();
        game.scripted = true;
        game.capture(false);
        assert!(!game.client.captured);
        assert!(game.client.capture_mode.is_some());
        assert_eq!(game.client.window.0.as_ref().unwrap().warps.get(), warps);
        game.scripted = false;
        game.settle_after_run(&Loop);
        assert!(!game.client.captured);
        assert!(game.client.capture_mode.is_none());
        assert!(game.client.window.0.as_ref().unwrap().visible.get());
    }
    // A person's click into the window or away from it during the run applies once it ends.
    {
        let mut game = ClientGame::default();
        game.scripted = true;
        game.client.focus_after_run = Some(false);
        game.settle_after_run(&Loop);
        assert!(game.applied_focus.is_empty());
        game.scripted = false;
        game.settle_after_run(&Loop);
        assert_eq!(game.applied_focus, vec![false]);
        assert!(!game.client.focused);
    }
    let mut game = ClientGame::default();
    // On a server the chat is ChatScreen (#708): loading never grabs over it.
    game.remote = Some(Remote);
    game.client.screen = Screen::Chat;
    game.capture_loaded_world();
    assert!(!game.client.captured);
    assert_eq!(game.client.window.0.as_ref().unwrap().grabs.get(), 0);
    game.remote = None;
    game.client.screen = Screen::Playing;
    // Grabbing and releasing put the pointer at the window's centre
    // (MouseHandler.grabMouse / releaseMouse), in the surface's pixels when it
    // has one; releasing an already released pointer leaves it where it is.
    // A grab warps once on every system: Windows before the Locked grab, which
    // clips the pointer where it is (#6076).
    let grab_warps = 1;
    game.client.window.0.as_mut().unwrap().size = Some([1000, 600]);
    game.client.window_pointer_space.take();
    game.client.renderer.0 = Some(Renderer { config: Config { width: 1000, height: 600 } });
    game.capture(true);
    assert!(game.client.captured);
    let window = game.client.window.0.as_ref().unwrap();
    assert_eq!((window.warps.get(), window.at.get()), (grab_warps, (500.0, 300.0)));
    game.capture(false);
    let window = game.client.window.0.as_ref().unwrap();
    assert_eq!(window.warps.get(), grab_warps + 2);
    assert_eq!(game.client.cursor, (250.0, 150.0));
    game.client.cursor = (1.0, 1.0);
    game.capture(false);
    assert_eq!(game.client.window.0.as_ref().unwrap().warps.get(), grab_warps + 2);
    assert_eq!(game.client.cursor, (1.0, 1.0));
    game.client.renderer.0 = None;
    game.client.window.0.as_mut().unwrap().fail = true;
    game.capture_loaded_world();
    assert!(!game.client.captured);
    assert!(game.client.window.0.as_ref().unwrap().visible.get());
}
'''
        fixture = fixture.replace("POINTER_PROJECTION", pointer_projection_sources())
        fixture = fixture.replace("METHODS",
                                  method(source, "capture") + "\n" +
                                  method(source, "pointer_space") + "\n" +
                                  method(source, "pointer_projection") + "\n" +
                                  method(source, "set_window_cursor") + "\n" +
                                  method(source, "capture_loaded_world") + "\n" +
                                  method(source, "regrab_on_refocus") + "\n" +
                                  method(source, "internal_grab") + "\n" +
                                  method(source, "claim_window") + "\n" +
                                  method(source, "settle_after_run"))
        with tempfile.TemporaryDirectory(prefix="sunrise-input-") as folder:
            folder = Path(folder)
            rust = folder / "capture.rs"
            binary = folder / ("capture.exe" if os.name == "nt" else "capture")
            rust.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", str(rust), "-o", str(binary)],
                           check=True, capture_output=True, text=True, timeout=60)
            subprocess.run([str(binary)], check=True, capture_output=True, text=True, timeout=10)


    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_native_mouse_turns_only_during_active_gameplay(self):
        source = "".join((ROOT / "crates/sunrise-client-gameplay/src" / path).read_text(encoding="utf-8")
                          for path in ("game/runtime/window_handler.rs",
                                       "game/interface/pointer_capture.rs"))
        fixture = r'''
use std::time::Instant;
extern crate self as sunrise_game_data;
pub mod move_input { pub fn spectator_flying_speed(_: f32) -> f32 { 0.0 } }
#[path = "MOUSE_PATH"] mod mouse;
use mouse::{MouseLook, LookOptions, Rotation};
struct ActiveEventLoop;
struct DeviceId;
enum DeviceEvent { MouseMotion { delta: (f64, f64) }, Other }
#[derive(PartialEq)] enum Screen { Playing, Paused }
struct Options { offscreen: bool }
mod input_trace {
    pub enum InputOrigin { User }
    pub enum TraceKind { Motion { x: f64, y: f64 } }
}
struct Radial(bool);
impl Radial { fn is_open(&self) -> bool { self.0 } }
struct Pad { radial: Radial }
struct CommonGame { options: Options }
struct ClientState {
    pending_save: (Option<()>,),
    captured: bool, focused: bool, screen: Screen, mouse: MouseLook, last_input: Instant,
    unclaimed_window: bool,
}
struct ClientGame {
    common: CommonGame, pad: Pad, scripted: bool, client: ClientState,
}
impl ClientGame {
    fn scripted_run(&self) -> bool { self.scripted }
    // The input trace records the motion; it does not change the turn.
    fn note_input(&mut self, _: input_trace::InputOrigin, _: input_trace::TraceKind) {}
    // Handing input back from the controller does not change the turn either.
    fn pad_yield_input(&mut self, _: &ActiveEventLoop) {}
    // `game/pad/capture.rs`: the radial menu owns the pad while it is open.
    fn pad_radial_captures(&self) -> bool { self.pad.radial.is_open() }
    // `game/interface/pointer_capture.rs`: a mod screen's pointer grab and its deferred request;
    // this fixture holds no mod screen, so motion always turns the camera.
    fn apply_pointer_grab_request(&mut self) {}
    fn mod_screen_grabs_pointer(&self) -> bool { false }
    fn screen_pointer_motion(&mut self, _: [f64; 2]) {}
METHOD
}
fn main() {
    // (offscreen, active, gameplay, saving, radial menu, scripted run, unclaimed window, turns).
    for (offscreen, active, playing, saving, radial, scripted, unclaimed, turns) in [
        (false, true, true, false, false, false, false, true),
        (true, true, true, false, false, false, false, false),
        (false, false, true, false, false, false, false, false),
        (false, true, false, false, false, false, false, false),
        (false, true, true, true, false, false, false, false),
        (false, true, true, false, true, false, false, false),
        (false, true, true, false, false, true, false, false),
        (false, true, true, false, false, false, true, false),
    ] {
        let mut game = ClientGame {
            common: CommonGame { options: Options { offscreen } },
            pad: Pad { radial: Radial(radial) },
            scripted,
            client: ClientState {
                pending_save: (saving.then_some(()),),
                captured: true, focused: active,
                screen: if playing { Screen::Playing } else { Screen::Paused },
                mouse: MouseLook::default(), last_input: Instant::now(),
                unclaimed_window: unclaimed,
            },
        };
        game.device_event(&ActiveEventLoop, DeviceId, DeviceEvent::MouseMotion { delta: (0.0, 0.0) });
        game.device_event(&ActiveEventLoop, DeviceId, DeviceEvent::MouseMotion { delta: (20.0, -10.0) });
        let look = LookOptions { sensitivity: 0.5, smooth_camera: false, scoping: false, invert_x: false, invert_y: false };
        let (x, y) = game.client.mouse.handle_accumulated_movement(1.0, true, true, look).unwrap();
        let mut rotation = Rotation { x_rot: 0.0, y_rot: 0.0, x_rot_o: 0.0, y_rot_o: 0.0 };
        rotation.turn(x, y);
        let want = if turns { (3.0, -1.5) } else { (0.0, 0.0) };
        assert!((rotation.y_rot - want.0).abs() < 0.00001 && (rotation.x_rot - want.1).abs() < 0.00001,
            "rotation=({}, {}), offscreen={offscreen}, active={active}, playing={playing}, saving={saving}, radial={radial}, scripted={scripted}, unclaimed={unclaimed}", rotation.y_rot, rotation.x_rot);
    }
}
'''
        fixture = fixture.replace("MOUSE_PATH", (ROOT / "crates/sunrise-client-input/src/mouse.rs").as_posix())
        fixture = fixture.replace(
            "METHOD", method(source, "device_event") + "\n" + method(source, "grabbed_pointer_motion"))
        self.run_fixture(fixture, "native-mouse")

    def run_fixture(self, fixture, name):
        with tempfile.TemporaryDirectory(prefix="sunrise-input-") as folder:
            folder = Path(folder)
            rust = folder / f"{name}.rs"
            binary = folder / (name + (".exe" if os.name == "nt" else ""))
            rust.write_text(fixture, encoding="utf-8")
            built = subprocess.run(["rustc", "--edition=2024", str(rust), "-o", str(binary)],
                                   capture_output=True, text=True, timeout=60)
            self.assertEqual(built.returncode, 0, built.stderr)
            ran = subprocess.run([str(binary)], capture_output=True, text=True, timeout=10)
            self.assertEqual(ran.returncode, 0, ran.stderr)

if __name__ == "__main__":
    unittest.main()
