"""Run the actual outline gate and highlight filter with fake state, without a renderer."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from tests.rust_items import rust_fn

ROOT = Path(__file__).resolve().parents[1]


class BlockHighlightHookTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_cancel_and_camera_outline_gates(self):
        source = (ROOT / "crates/sunrise-client-gameplay/src/game/game_client.rs").read_text(encoding="utf-8")
        call = source.index("self.native_block_highlight(")
        start = source.rindex(".filter(|h| {", 0, call)
        end = source.index(".map(|h| {", call)
        self.assertLess(source.index(".block_outline_hit()"), start)
        gate_hit = rust_fn("block_outline_hit", owner="sunrise-client-gameplay")
        gate_fn = rust_fn("should_render_block_outline_for_hit", owner="sunrise-client-gameplay")
        fixture = r"""
use std::cell::Cell;

mod sunrise_items {
    pub mod equipment {
        pub const MAINHAND: usize = 0;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Hit { pos: [i32; 3] }

#[derive(Clone, Copy, Debug, PartialEq)]
struct BlockStateId(u32);

mod sunrise_client_gameplay {
    pub mod hud_gates {
        pub fn block_outline_drawn(
            hide_gui: bool,
            entity_targeted: bool,
            can_build: bool,
            panorama: bool,
        ) -> bool {
            !hide_gui && !entity_targeted && can_build && !panorama
        }
    }
}

struct World;
impl World {
    fn block(&self, x: i32, y: i32, z: i32) -> BlockStateId {
        assert_eq!([x, y, z], [1, 2, 3]);
        BlockStateId(42)
    }
}

#[derive(Clone)]
struct ItemStack {
    can_break: bool,
    can_place_on: bool,
}

struct Extra { main_hand: Option<ItemStack> }
impl Extra {
    fn slot_stack(&self, slot: usize) -> Option<&ItemStack> {
        assert_eq!(slot, sunrise_items::equipment::MAINHAND);
        self.main_hand.as_ref()
    }
}

struct Mob { id: u32, extra: Extra }
struct MobIndex;
impl MobIndex {
    fn find_ref(&self, mobs: &[Mob], id: u32) -> Option<usize> {
        mobs.iter().position(|m| m.id == id)
    }
}

#[derive(Clone)]
struct DetachedPlayerHud {
    main_hand: Option<ItemStack>,
}

struct ChatUi { hide_gui: bool }
struct ClientState { chat_ui: ChatUi }

struct CommonGame {
    world: World,
    mobs: Vec<Mob>,
    mob_index: MobIndex,
    spectator_mode: bool,
    local_held: Option<ItemStack>,
}
impl CommonGame {
    fn spectator(&self) -> bool { self.spectator_mode }
    fn held(&self) -> Option<&ItemStack> { self.local_held.as_ref() }
}

#[derive(Clone)]
struct ResolvedCameraEntity {
    entity_id: u32,
    is_local_player: bool,
    is_player: bool,
    may_build: bool,
    detached_hud: Option<DetachedPlayerHud>,
}

mod game_impl {
    use super::*;

    pub struct ClientGame {
        pub common: CommonGame,
        pub client: ClientState,
        pub camera: Option<ResolvedCameraEntity>,
        pub hit: Option<Hit>,
        pub entity_hit: bool,
        pub panorama: bool,
        pub interactable: bool,
        pub cancel: bool,
        pub calls: Cell<usize>,
    }

    impl ClientGame {
        fn camera_subject(&self) -> Option<ResolvedCameraEntity> { self.camera.clone() }
        fn entity_targeted(&self) -> bool { self.entity_hit }
        fn pick(&self) -> Option<Hit> { self.hit }
        fn shows_panorama(&self) -> bool { self.panorama }
        fn block_interactable_for_spectator(&self, _hit: &Hit) -> bool { self.interactable }
        fn stack_adventure_predicate(&self, stack: &ItemStack, component: &str, _pos: [i32; 3]) -> bool {
            match component {
                "minecraft:can_break" => stack.can_break,
                "minecraft:can_place_on" => stack.can_place_on,
                _ => false,
            }
        }
        fn native_block_highlight(&self, pos: [i32; 3], state: BlockStateId, partial: f32, canceled: &mut bool) {
            assert_eq!(pos, [1, 2, 3]);
            assert_eq!(state, BlockStateId(42));
            assert_eq!(partial, 0.5);
            self.calls.set(self.calls.get() + 1);
            *canceled = self.cancel;
        }
GATE_HIT
GATE_FN
        pub fn run_outline(&self) -> Option<Hit> {
            let alpha = 0.5;
            self.block_outline_hit() FILTER
        }
    }
}

use game_impl::ClientGame;

fn base_game() -> ClientGame {
    ClientGame {
        common: CommonGame {
            world: World,
            mobs: Vec::new(),
            mob_index: MobIndex,
            spectator_mode: false,
            local_held: None,
        },
        client: ClientState { chat_ui: ChatUi { hide_gui: false } },
        camera: Some(ResolvedCameraEntity {
            entity_id: 1,
            is_local_player: true,
            is_player: true,
            may_build: true,
            detached_hud: None,
        }),
        hit: Some(Hit { pos: [1, 2, 3] }),
        entity_hit: false,
        panorama: false,
        interactable: false,
        cancel: false,
        calls: Cell::new(0),
    }
}

#[test]
fn cancelled_default_and_missing_target() {
    for cancel in [false, true] {
        let mut game = base_game();
        game.cancel = cancel;
        assert_eq!(game.run_outline(), if cancel { None } else { game.hit });
        game.hit = None;
        assert_eq!(game.run_outline(), None);
        assert_eq!(game.calls.get(), 1);
    }
}

#[test]
fn camera_entity_and_mode_gate_block_outline_before_hook() {
    // Non-player camera never outlines or invokes highlight hook.
    let mut game = base_game();
    game.camera = Some(ResolvedCameraEntity {
        entity_id: 9,
        is_local_player: false,
        is_player: false,
        may_build: false,
        detached_hud: None,
    });
    assert_eq!(game.run_outline(), None);
    assert_eq!(game.calls.get(), 0);

    // Local spectator outlines only interactable blocks.
    let mut game = base_game();
    game.camera.as_mut().unwrap().may_build = false;
    game.common.spectator_mode = true;
    game.interactable = false;
    assert_eq!(game.run_outline(), None);
    game.interactable = true;
    assert_eq!(game.run_outline(), game.hit);

    // Local adventure requires can_break or can_place_on on camera player's main hand.
    let mut game = base_game();
    game.camera.as_mut().unwrap().may_build = false;
    assert_eq!(game.run_outline(), None);
    game.common.local_held = Some(ItemStack { can_break: false, can_place_on: true });
    assert_eq!(game.run_outline(), game.hit);

    // Spectating a remote player uses remote player's may_build (true) even when viewer is spectator.
    let mut game = base_game();
    game.common.spectator_mode = true;
    game.interactable = false;
    game.camera = Some(ResolvedCameraEntity {
        entity_id: 22,
        is_local_player: false,
        is_player: true,
        may_build: true,
        detached_hud: None,
    });
    assert_eq!(game.run_outline(), game.hit);

    // Hidden HUD suppresses outline.
    game.client.chat_ui.hide_gui = true;
    assert_eq!(game.run_outline(), None);
}
""".replace("GATE_HIT", gate_hit).replace("GATE_FN", gate_fn).replace("FILTER", source[start:end])
        with tempfile.TemporaryDirectory() as folder:
            src, exe = Path(folder) / "outline.rs", Path(folder) / "outline.exe"
            src.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
