"""Exercise the live remote_mining method with scripted world and packet sinks.

Vanilla reference: MultiPlayerGameMode.startDestroyBlock/continueDestroyBlock/stopDestroyBlock,
and the crack stages they tell ClientLevel.destroyBlockProgress (`client_break_progress`).
The first_person parity harness separately checks consecutive instant breaks
against the pinned client; this fixture catches drift in the multiplayer path.
"""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


def method(source, name):
    # A method moved out of game.rs keeps its file's visibility; the fixture pastes it private.
    visibilities = ("pub", "pub(super)", "pub(in crate::game)")
    plain = f"    fn {name}("
    start = source.index(plain) if plain in source else next(
        source.index(f"    {vis} fn {name}(") for vis in visibilities
        if f"    {vis} fn {name}(" in source)
    opening = source.index("{", start)
    depth = 1
    end = opening + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    body = source[start:end]
    for vis in visibilities:
        body = body.replace(f"{vis} fn", "fn", 1)
    return body


@unittest.skipUnless(shutil.which("rustc"), "requires Rust")
class RemoteMiningTests(unittest.TestCase):
    def test_break_timing_and_packets(self):
        remote = ROOT / "crates/sunrise-client-gameplay/src/game/network/remote"
        breaking = method((remote / "use_client.rs").read_text(encoding="utf-8"), "remote_mining")
        stopping = method((remote / "break_prediction_client.rs").read_text(encoding="utf-8"), "stop_remote_mining")
        hooks = ROOT / "crates/sunrise-client-gameplay/src/game/native_hooks/client_break_progress.rs"
        staging = method(hooks.read_text(encoding="utf-8"), "native_local_break_progress")
        source = breaking + "\n" + stopping + "\n" + staging
        fixture = r'''
#![allow(dead_code)]
#[derive(Clone, Copy)]
struct Hit { pos: [i32; 3], face: u8 }
/// The held stack: `ItemStack.item`, which the debug-stick check reads.
struct Stack { item: &'static str }
// Display-only crack state is covered by the client crack integration tests.
struct Mining { pos: [i32; 3], progress: f32, per_tick: f32, ticks: u32 }
#[derive(Default)]
struct Remote { destroy_delay: u32, destroying: Option<([i32; 3], u8, f32)>, destroying_item: String, is_destroying: bool }
struct Ground { on_ground: bool }
struct Player { ground: Ground }
#[derive(Clone)]
struct Sounds { hit: String, volume: f32, pitch: f32 }
#[derive(Clone)]
struct Facts { hardness: f32, name: String, requires_tool: bool, sounds: Sounds }
struct Resources { facts: Facts, speed: f32, tool_allows_creative: bool }
impl Resources {
    fn can_destroy_block(&self, _: Option<&Stack>, creative: bool) -> bool {
        !creative || self.tool_allows_creative
    }
    fn facts(&self) -> &Self { self }
    fn items(&self) -> &Self { self }
    fn get(&self, _: u32) -> &Facts { &self.facts }
    fn is_air(&self, state: u32) -> bool { state == 0 }
    fn mining_speed(&self, _: Option<&()>, _: &str) -> f32 { self.speed }
    fn correct_for_drops(&self, _: Option<&()>, _: &str) -> bool { true }
}
#[derive(Default)]
struct World { broken: Vec<[i32; 3]> }
impl World {
    fn block(&self, x: i32, y: i32, z: i32) -> u32 {
        u32::from(!self.broken.contains(&[x, y, z]))
    }
}
struct Audio { played: Vec<String> }
impl Audio {
    fn play(&mut self, name: &str, _: Option<[f64; 3]>, _: f32, _: f32) {
        self.played.push(name.to_string());
    }
}
mod play {
    #[derive(Debug, PartialEq)]
    pub struct PlayerAction {
        pub action: u8, pub pos: [i32; 3], pub direction: u8, pub sequence: i32,
    }
}
enum S { PlayerAction(play::PlayerAction) }
fn face_direction(face: u8) -> u8 { face }
struct CommonGame {
    mining: Option<Mining>, creative: bool,
    world: World, res: Resources, player: Option<Player>,
}
impl CommonGame {
    fn held(&self) -> Option<&Stack> { None }
    fn main_hand_key(&self) -> String { String::new() }
}
struct ClientState { remote: Option<Remote> }
struct ClientGame {
    common: CommonGame, client: ClientState, audio: Audio,
    hit: Hit, sequence: i32, packets: Vec<play::PlayerAction>, restricted: bool,
    veto: bool,
    /// The cracks told mods (`client_break_progress`): breaker, position and stage.
    cracks: std::cell::RefCell<Vec<(i32, [i32; 3], i32)>>,
}
impl ClientGame {
    // No display subscribers in this packet/gameplay fixture.
    fn set_client_mining(&mut self, mining: Option<Mining>) { self.common.mining = mining; }
    fn same_destroying_item(&self, began: &String) -> bool { *began == self.common.main_hand_key() }
    fn reset_attack_strength(&mut self) {}
    fn own_entity_id(&self) -> i32 { 7 }
    fn client_own_entity_id(&self) -> i32 { self.own_entity_id() }
    fn native_client_break_progress(&self, breaker: i32, pos: [i32; 3], progress: i32) {
        self.cracks.borrow_mut().push((breaker, pos, progress));
    }
    fn told(&self) -> Vec<i32> {
        self.cracks.borrow_mut().drain(..).map(|(breaker, pos, stage)| {
            assert_eq!((breaker, pos), (7, self.hit.pos), "the player's own crack on its block");
            stage
        }).collect()
    }
    fn new(hardness: f32, speed: f32, creative: bool) -> Self {
        Self {
            common: CommonGame {
                mining: None, creative,
                world: World::default(),
                res: Resources { facts: Facts { hardness, name: "block".into(),
                    requires_tool: false, sounds: Sounds { hit: "hit".into(), volume: 1.0, pitch: 1.0 } }, speed, tool_allows_creative: true },
                player: Some(Player { ground: Ground { on_ground: true } }),
            },
            client: ClientState { remote: Some(Remote::default()) },
            audio: Audio { played: Vec::new() },
            hit: Hit { pos: [0, 64, 0], face: 1 }, sequence: 0, packets: vec![],
            restricted: false, veto: false, cracks: Default::default(),
        }
    }
    fn pick(&self) -> Option<Hit> { Some(self.hit) }
    // Player.blockActionRestricted: adventure without a can_break item.
    fn remote_break_restricted(&self, _: [i32; 3]) -> bool { self.restricted }
    fn veto_mining_prediction(&self, _: [i32; 3], _: u8, _: bool, _: f32) -> bool {
        self.veto
    }
    // ClientGame::destroy_progress (hand.rs) for a harvestable block on the ground.
    fn destroy_progress(&self, _: [i32; 3]) -> f32 {
        self.common.res.speed / self.common.res.facts.hardness / 30.0
    }
    fn next_sequence(&mut self) -> i32 { self.sequence += 1; self.sequence }
    fn predict_break(&mut self, pos: [i32; 3], _: u32, _: i32) { self.common.world.broken.push(pos); }
    fn send(&mut self, S::PlayerAction(packet): S) { self.packets.push(packet); }
    fn crack_particle(&mut self, _: [i32; 3], _: u8, _: u32) {}
    // RedStoneOreBlock.attack's dust (sculk.rs): no particles in this fixture.
    fn redstone_ore_particles(&mut self, _: [i32; 3]) {}
    // ClientLevel.addBreakingBlockEffects: the hit sound once, when the dig
    // starts; the server's level events play it while the block is mined.
    fn breaking_block_effects(&mut self, _: [i32; 3], _: u8, play_sound: bool) {
        if play_sound {
            self.audio.play("hit", None, 1.0, 1.0);
        }
    }
    // Tutorial.onDestroyBlock (tutorial.rs): the hints play no part in mining.
    fn tutorial_destroy<T>(&mut self, _: T, _: f32) {}
METHOD
}
fn main() {
    let mut game = ClientGame::new(1.0, 1.0, false);
    game.veto = true;
    assert!(!game.remote_mining(true));
    assert!(game.packets.is_empty());
    assert!(game.common.mining.is_none());
    game.veto = false;
    assert!(game.remote_mining(true));
    let before = game.client.remote.as_ref().unwrap().destroying;
    game.veto = true;
    assert!(!game.remote_mining(false));
    assert_eq!(game.client.remote.as_ref().unwrap().destroying, before);
    assert!(game.client.remote.as_ref().unwrap().is_destroying);
    assert!(game.stop_remote_mining());
    assert!(!game.stop_remote_mining());
    assert_eq!(game.packets.iter().map(|p| p.action).collect::<Vec<_>>(), [0, 2]);
    // The start tells -1 (no progress yet), a vetoed tick tells nothing, and the stop tells -1
    // once: a dig that is not running has nothing to stop.
    assert_eq!(game.told(), [-1, -1]);

    // Zero-hardness blocks and a tool fast enough to break in one tick.
    for (hardness, speed) in [(0.0, 1.0), (0.5, 15.0), (0.5, 30.0)] {
        let mut game = ClientGame::new(hardness, speed, false);
        for tick in 0..8 {
            game.hit.pos[0] = tick;
            game.remote_mining(tick == 0);
            assert_eq!(game.common.world.broken.len(), tick as usize + 1,
                "instant break must happen every held tick");
            let remote = game.client.remote.as_ref().unwrap();
            assert_eq!(remote.destroy_delay, 0);
            assert!(remote.destroying.is_none());
            assert!(game.common.mining.is_none());
            assert_eq!(game.packets[tick as usize], play::PlayerAction {
                action: 0, pos: [tick, 64, 0], direction: 1, sequence: tick + 1,
            });
        }
        assert_eq!(game.packets.len(), 8, "instant breaks only send START");
        assert!(game.cracks.borrow().is_empty(), "an instant break tells no crack");
    }
    // Gradual breaking still sends STOP and waits five continuation ticks.
    let mut game = ClientGame::new(1.0, 15.0, false);
    game.remote_mining(true);
    assert!(game.common.world.broken.is_empty());
    // The dig plays the hit sound once, when it starts (#2277): the server's
    // destroy progress level events play it while the block is mined.
    assert_eq!(game.audio.played, ["hit"]);
    game.remote_mining(false);
    assert!(game.common.world.broken.is_empty());
    game.remote_mining(false);
    assert_eq!(game.common.world.broken.len(), 1);
    assert_eq!(game.audio.played, ["hit"], "the client must not play the mining sound in the loop");
    // Half the block a tick: -1 at the start, stage 5 on the next tick, and -1 once the
    // finished break resets the progress.
    assert_eq!(game.told(), [-1, 5, -1]);
    assert_eq!(game.packets.iter().map(|p| p.action).collect::<Vec<_>>(), [0, 3]);
    assert_eq!(game.client.remote.as_ref().unwrap().destroy_delay, 5);
    game.hit.pos[0] = 1;
    for _ in 0..5 {
        game.remote_mining(false);
        assert_eq!(game.packets.len(), 2);
    }
    game.remote_mining(false);
    assert_eq!(game.packets.len(), 3);
    assert_eq!(game.packets[2].action, 0);
    // Creative also keeps its five-tick delay, even for zero-hardness blocks.
    let mut game = ClientGame::new(0.0, 1.0, true);
    game.remote_mining(true);
    game.hit.pos[0] = 1;
    assert_eq!(game.client.remote.as_ref().unwrap().destroy_delay, 5);
    for _ in 0..5 {
        game.remote_mining(false);
        assert_eq!(game.common.world.broken.len(), 1);
    }
    game.remote_mining(false);
    assert_eq!(game.common.world.broken.len(), 2);
    assert_eq!(game.client.remote.as_ref().unwrap().destroy_delay, 5);
    // A restricted tool must neither predict a break nor send START in Creative.
    let mut game = ClientGame::new(1.0, 1.0, true);
    game.common.res.tool_allows_creative = false;
    game.remote_mining(true);
    game.remote_mining(false);
    assert!(game.common.world.broken.is_empty());
    assert!(game.packets.is_empty());
    // Survival still mines with the same tool. Switching to Creative aborts the crack.
    game.common.creative = false;
    game.remote_mining(true);
    assert_eq!(game.packets[0].action, 0);
    game.common.creative = true;
    game.remote_mining(false);
    assert_eq!(game.packets.iter().map(|p| p.action).collect::<Vec<_>>(), [0, 2]);
    // stopDestroyBlock keeps destroyBlockPos and the item and zeroes only the progress, so a
    // resumed dig on the same block sends no new START (#4124).
    let stopped = game.client.remote.as_ref().unwrap();
    assert_eq!(
        stopped.destroying.map(|(p, _, progress)| (p, progress)),
        Some((game.hit.pos, 0.0))
    );
    assert!(!stopped.is_destroying);
    assert!(game.common.mining.is_none());
    assert!(game.common.world.broken.is_empty());
    // An adventure player without can_break (#3027): startDestroyBlock
    // refuses before predicting, so nothing breaks, cracks or is sent.
    for (hardness, speed) in [(0.0, 1.0), (1.0, 15.0)] {
        let mut game = ClientGame::new(hardness, speed, false);
        game.restricted = true;
        game.remote_mining(true);
        for _ in 0..40 {
            game.remote_mining(false);
        }
        assert!(game.common.world.broken.is_empty());
        assert!(game.packets.is_empty());
        assert!(game.audio.played.is_empty());
        assert!(game.common.mining.is_none());
        assert!(game.client.remote.as_ref().unwrap().destroying.is_none());
    }
    // A gradual break at A, then an instant break at B, then another at C: the instant
    // break leaves A as the running target, so each later start aborts A again.
    let mut game = ClientGame::new(1.0, 1.0, false);
    game.hit.pos = [1, 64, 0];
    game.remote_mining(true);
    game.common.res.speed = 60.0;
    game.hit.pos = [2, 64, 0];
    game.remote_mining(true);
    assert_eq!(game.client.remote.as_ref().unwrap().destroying.map(|(p, _, _)| p), Some([1, 64, 0]));
    assert!(game.client.remote.as_ref().unwrap().is_destroying);
    game.hit.pos = [3, 64, 0];
    game.remote_mining(true);
    assert_eq!(
        game.packets.iter().map(|p| (p.action, p.pos)).collect::<Vec<_>>(),
        [
            (0, [1, 64, 0]),
            (2, [1, 64, 0]),
            (0, [2, 64, 0]),
            (2, [1, 64, 0]),
            (0, [3, 64, 0]),
        ]
    );
    assert_eq!(game.common.world.broken, [[2, 64, 0], [3, 64, 0]]);
}
'''.replace("METHOD", source)
        with tempfile.TemporaryDirectory(prefix="remote-mining-") as directory:
            path = Path(directory)
            (path / "main.rs").write_text(fixture)
            probe = path / ("probe.exe" if os.name == "nt" else "probe")
            compile_result = subprocess.run(
                ["rustc", "--edition=2024", str(path / "main.rs"), "-o", str(probe)],
                capture_output=True, text=True,
            )
            self.assertEqual(compile_result.returncode, 0, compile_result.stderr)
            result = subprocess.run([str(probe)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
