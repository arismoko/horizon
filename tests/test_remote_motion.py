"""Exercise Entity motion delivery through the live multiplayer handler."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from tools.ci import rust_tests

ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(shutil.which("rustc"), "requires Rust")
class RemoteMotionTests(unittest.TestCase):
    def test_mob_velocity_reaches_squid_animation(self):
        source = (ROOT / "crates/sunrise-client-gameplay/src/game/network/remote/motion_client.rs").read_text(encoding="utf-8")
        code = rust_tests.blank(source)
        start = code.index("fn remote_motion(")
        opening = code.index("{", start)
        method_source = source[start:rust_tests.matching_brace(code, opening)]
        fixture = r'''
#![allow(dead_code)]
use std::collections::HashMap;
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Vec3(f64, f64, f64);
impl Vec3 {
    fn new(x: f64, y: f64, z: f64) -> Self { Self(x,y,z) }
    fn length_squared(self) -> f64 { self.0 * self.0 + self.1 * self.1 + self.2 * self.2 }
}
#[derive(Default)]
struct ArrowState { in_ground: bool, life: i32 }
#[derive(Default)]
struct Entity {
    id: u32, velocity: Vec3, arrow: Option<ArrowState>, stuck: bool, in_ground: bool, in_ground_time: u32,
}
// Scoped entity edits used by remote motion.
trait EditEntity { fn update<R>(&mut self, i: usize, edit: impl FnOnce(&mut Entity) -> R) -> R; }
impl EditEntity for Vec<Entity> { fn update<R>(&mut self, i: usize, edit: impl FnOnce(&mut Entity) -> R) -> R { edit(&mut self[i]) }
}
struct PendingItem { join: u64, position: [f64; 3], velocity: Vec3 }
#[derive(Default)]
struct Remote { pending_items: HashMap<i32, PendingItem> }
#[derive(Default)]
struct Creatures { projectiles: Vec<Entity> }
#[derive(Default)]
struct Potions;
mod game {
    pub mod client_side {
        pub mod potions {
            pub fn remote_thrown_motion(_: &mut crate::Potions, _id: u32, _v: crate::Vec3) -> bool { false }
        }
    }
}
#[derive(Default)]
struct End;
impl End { fn remote_eye_motion(&mut self, _id: u32, _v: [f64; 3]) {} }
// game/mobs/mob_index.rs: the same answers as a linear search.
#[derive(Default)]
struct MobIndex;
impl MobIndex {
    fn get_mut<'a>(&mut self, mobs: &'a mut [Entity], id: u32) -> Option<&'a mut Entity> {
        mobs.iter_mut().find(|m| m.id == id)
    }
}
#[derive(Default)]
struct CommonGame {
    mobs: Vec<Entity>, mob_index: MobIndex, dropped: Vec<Entity>, falling: Vec<Entity>, tnts: Vec<Entity>, orbs: Vec<Entity>,
    creatures: Creatures, dragon_fireballs: Vec<Entity>, tridents: Vec<Entity>,
    rockets: Vec<Entity>, carts: Vec<Entity>, boats: Vec<Entity>,
    potions: Potions, end: End,
}
#[derive(Default)]
struct ClientState { remote: Option<Remote> }
#[derive(Default)]
struct ClientGame { common: CommonGame, client: ClientState }
impl ClientGame {
fn remote_dragon_motion(&mut self, _id: i32, _velocity: [f64; 3]) {}
fn remote_hook_motion(&mut self, _id: u32, _v: Vec3) -> bool { false }
METHOD
}
fn main() {
    let mut game = ClientGame::default();
    game.common.mobs.push(Entity { id: 42, ..Entity::default() });
    game.common.dropped.push(Entity { id: 43, ..Entity::default() });
    // AddEntity initial motion and subsequent SetEntityMotion share this handler.
    for motion in [[0.12, -0.04, 0.08], [-0.05, 0.03, 0.2], [0.0; 3]] {
        game.remote_motion(42, motion);
        assert_eq!(game.common.mobs[0].velocity, Vec3::new(motion[0], motion[1], motion[2]));
        assert_eq!(game.common.dropped[0].velocity, Vec3::default());
    }
    game.remote_motion(43, [0.1, 0.2, 0.3]);
    assert_eq!(game.common.dropped[0].velocity, Vec3::new(0.1, 0.2, 0.3));
    // The stack has not arrived yet: retain the admission slot and store the motion.
    game.client.remote = Some(Remote { pending_items: HashMap::from([(48, PendingItem {
        join: 7, position: [1.0, 2.0, 3.0], velocity: Vec3::default(),
    })]) });
    game.remote_motion(48, [-0.5, 0.0, 0.2]);
    let pending = &game.client.remote.as_ref().unwrap().pending_items[&48];
    assert_eq!(pending.velocity, Vec3::new(-0.5, 0.0, 0.2));
    assert_eq!((pending.join, pending.position), (7, [1.0, 2.0, 3.0]));
    game.common.carts.push(Entity { id: 44, ..Entity::default() });
    game.remote_motion(44, [-0.7, 0.25, 1.5]);
    assert_eq!(game.common.carts[0].velocity, Vec3::new(-0.7, 0.25, 1.5));
    // Entity.lerpMotion on a boat, the one its own player drives included.
    game.common.boats.push(Entity { id: 47, ..Entity::default() });
    game.remote_motion(47, [0.0; 3]);
    game.remote_motion(47, [0.3, -0.1, 0.2]);
    assert_eq!(game.common.boats[0].velocity, Vec3::new(0.3, -0.1, 0.2));
    game.remote_motion(999, [1.0; 3]); // Untracked entity is harmless.
    // AbstractArrow.lerpMotion: life restarts on any motion; only a non-zero
    // motion takes a stuck arrow or trident out of its block.
    let stuck_arrow = |id| Entity {
        id, stuck: true, arrow: Some(ArrowState { in_ground: true, life: 600 }), ..Entity::default()
    };
    game.common.creatures.projectiles.push(stuck_arrow(46));
    game.remote_motion(46, [0.0; 3]);
    let a = &game.common.creatures.projectiles[0];
    assert!(a.stuck && a.arrow.as_ref().unwrap().in_ground);
    assert_eq!(a.arrow.as_ref().unwrap().life, 0);
    game.common.creatures.projectiles[0].arrow.as_mut().unwrap().life = 600;
    game.remote_motion(46, [0.0, 0.5, 0.0]);
    let a = &game.common.creatures.projectiles[0];
    assert!(!a.stuck && !a.arrow.as_ref().unwrap().in_ground);
    assert_eq!(a.arrow.as_ref().unwrap().life, 0);
    game.common.tridents.push(Entity { id: 45, in_ground: true, in_ground_time: 30, ..Entity::default() });
    game.remote_motion(45, [0.0; 3]);
    assert!(game.common.tridents[0].in_ground);
    assert_eq!(game.common.tridents[0].in_ground_time, 0);
    game.remote_motion(45, [0.1, 0.0, 0.0]);
    assert!(!game.common.tridents[0].in_ground);
}
'''.replace("METHOD", method_source)
        with tempfile.TemporaryDirectory(prefix="remote-motion-") as directory:
            path = Path(directory)
            (path / "main.rs").write_text(fixture)
            compile_result = subprocess.run(
                ["rustc", "--edition=2024", str(path / "main.rs"), "-o", str(path / "probe")],
                capture_output=True, text=True)
            self.assertEqual(compile_result.returncode, 0, compile_result.stderr)
            result = subprocess.run([str(path / "probe")], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
