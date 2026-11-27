"""A rocket boosting a server client's own player, run through the live methods.

Vanilla reference: FireworkRocketEntity.tick after the player's tick in ClientLevel.tickEntities;
Entity.getHandHoldingItemAngle.
"""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from rust_items import rust_fn


@unittest.skipUnless(shutil.which("rustc"), "requires Rust")
class RemoteOwnRocketTests(unittest.TestCase):
    def test_the_spark_follows_the_moved_player_at_the_rocket_hand(self):
        methods = "\n".join([
            rust_fn("rocket_hand_angle", owner="sunrise-gameplay-common"),
            rust_fn("follow_own_player", owner="sunrise-gameplay-common"),
            rust_fn("rocket_spark", owner="sunrise-client-gameplay"),
            rust_fn("remote_player_id", owner="sunrise-client-gameplay"),
            rust_fn("tick_own_rockets", owner="sunrise-client-gameplay"),
        ])
        fixture = r'''
#![allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Vec3 { x: f64, y: f64, z: f64 }
impl Vec3 {
    fn new(x: f64, y: f64, z: f64) -> Self { Self { x, y, z } }
    fn add_vector(self, o: Vec3) -> Self { Self::new(self.x + o.x, self.y + o.y, self.z + o.z) }
    fn components(self) -> [f64; 3] { [self.x, self.y, self.z] }
}
// Entity.calculateViewVector(pitch, yaw).
fn view_vector(yaw: f32, pitch: f32) -> [f64; 3] {
    let (p, y) = (f64::from(pitch).to_radians(), f64::from(-yaw).to_radians());
    [y.sin() * p.cos(), -p.sin(), y.cos() * p.cos()]
}
mod sunrise_world {
    pub mod view_direction {
        pub fn view_vector(pitch: f32, yaw: f32) -> crate::Vec3 {
            let [x, y, z] = crate::view_vector(yaw, pitch);
            crate::Vec3::new(x, y, z)
        }
    }
}
// Storage is not the subject of this physics fixture. Real numbered-list
// identity and restoration have separate production tests.
mod sunrise_sim {
    pub mod join_seq { pub enum RestoreOrder { SurvivorsFirst } }
}
trait Lift: Default + Sized {
    fn lift(&mut self) -> Self { std::mem::take(self) }
    fn restore(&mut self, taken: Self, order: sunrise_sim::join_seq::RestoreOrder);
}
impl<T> Lift for Vec<T> {
    fn restore(&mut self, taken: Self, _: sunrise_sim::join_seq::RestoreOrder) {
        assert!(self.is_empty(), "this physics fixture injects no nested spawn");
        *self = taken;
    }
}
struct ItemStack { item: String }
struct Remote { entity_id: i32 }
mod sunrise_core {
    pub mod random {
        // Gaussians in turn from a list.
        pub struct LegacyRandom(pub Vec<f64>);
        impl LegacyRandom {
            pub fn next_gaussian(&mut self) -> f64 { self.0.remove(0) }
        }
    }
}
struct World { random: sunrise_core::random::LegacyRandom }
enum ParticleType { Firework }
struct ParticleOptions;
impl ParticleOptions { fn simple(_: ParticleType) -> Self { Self } }
#[derive(Default)]
struct Particles { added: Vec<([f64; 3], [f64; 3])> }
impl Particles {
    fn add(&mut self, _: &ParticleOptions, at: [f64; 3], v: [f64; 3]) { self.added.push((at, v)); }
}
struct Clock { runs: bool }
impl Clock {
    fn runs_normally(&self) -> bool { self.runs }
    fn world_runs(&mut self) -> bool { panic!("TickRateManager.tick a second time in one client tick") }
}
struct Settings { main_hand_left: bool }
struct ClientState { remote: Option<Remote>, settings: Settings, particles: (Particles,) }
mod game {
    pub struct Side;
    impl Side {
        pub fn main_hand_left(side: &crate::ClientState) -> bool { side.settings.main_hand_left }
    }
}
struct Player { position: Vec3, velocity: Vec3, fall_flying: bool }
#[derive(Default)]
struct Rocket { id: u32, attached_to: Option<u32>, position: Vec3, velocity: Vec3 }
struct CommonGame {
    clock: Clock, player: Option<Player>,
    hotbar: Option<ItemStack>, offhand: Option<ItemStack>, selected: usize,
    rockets: Vec<Rocket>, yaw: f32, pitch: f32, world: World,
}
struct ClientGame {
    common: CommonGame,
    client: ClientState,
}
impl std::ops::Deref for ClientGame {
    type Target = CommonGame;
    fn deref(&self) -> &CommonGame { &self.common }
}
impl std::ops::DerefMut for ClientGame {
    fn deref_mut(&mut self) -> &mut CommonGame { &mut self.common }
}
impl ClientGame {
fn side_state(&self) -> &ClientState { &self.client }
fn stack_in(&self, slot: usize) -> Option<&ItemStack> { self.common.hotbar.as_ref().filter(|_| slot == 3) }
// The pick-section index is not the subject of this physics fixture.
fn pick_section_move(&mut self, _: u32, _: [f64; 3]) {}
METHODS
}
fn rocket() -> Option<ItemStack> { Some(ItemStack { item: "minecraft:firework_rocket".into() }) }
fn game(runs: bool) -> ClientGame {
    ClientGame {
        common: CommonGame {
            clock: Clock { runs },
            // Where the player's own tick has just moved it, facing west.
            player: Some(Player {
                position: Vec3::new(10.0, 70.0, 20.0),
                velocity: Vec3::new(0.0, -0.2, 1.0),
                fall_flying: true,
            }),
            hotbar: rocket(), offhand: None, selected: 3,
            rockets: vec![
                Rocket { attached_to: Some(7), ..Rocket::default() },
                Rocket { attached_to: Some(8), position: Vec3::new(1.0, 2.0, 3.0), ..Rocket::default() },
                Rocket { attached_to: None, position: Vec3::new(4.0, 5.0, 6.0), ..Rocket::default() },
            ],
            yaw: 90.0, pitch: 0.0,
            world: World { random: sunrise_core::random::LegacyRandom(vec![3.0, -2.0, 3.0, -2.0]) },
        },
        client: ClientState {
            remote: Some(Remote { entity_id: 7 }),
            settings: Settings { main_hand_left: false },
            particles: (Particles::default(),),
        },
    }
}
fn near(a: [f64; 3], b: [f64; 3]) -> bool { a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-4) }
fn at(v: Vec3) -> [f64; 3] { [v.x, v.y, v.z] }
// The rocket's spot after one tick with this hand setup.
fn hand(main_hand_left: bool, main: Option<ItemStack>, offhand: Option<ItemStack>) -> [f64; 3] {
    let mut g = game(true);
    g.client.settings.main_hand_left = main_hand_left;
    g.common.hotbar = main;
    g.common.offhand = offhand;
    g.tick_own_rockets();
    at(g.common.rockets[0].position)
}
fn main() {
    let mut g = game(true);
    g.tick_own_rockets();
    // Looking west (-x): movement + look * 0.1 + (look * 1.5 - movement) * 0.5.
    let boosted = [-0.85, -0.1, 0.5];
    assert!(near(at(g.common.player.as_ref().unwrap().velocity), boosted), "{:?}", g.common.player.as_ref().unwrap().velocity);
    assert_eq!(g.common.player.as_ref().unwrap().position, Vec3::new(10.0, 70.0, 20.0));
    // Facing west the right hand is north: half a block out at yaw + 80.
    let right = [10.0 - 0.0868, 70.0, 20.0 - 0.4924];
    assert!(near(at(g.common.rockets[0].position), right), "{:?}", g.common.rockets[0].position);
    assert!(near(at(g.common.rockets[0].velocity), boosted));
    // One firework spark at the rocket, drifting against the boosted motion
    // with a gaussian spread of 0.05 across it.
    let sparks = &g.client.particles.0.added;
    assert_eq!(sparks.len(), 1);
    assert!(near(sparks[0].0, right) && near(sparks[0].1, [0.15, 0.05, -0.1]), "{:?}", sparks);
    // Rockets of another entity, or free ones, tick with the other entities.
    assert_eq!(g.common.rockets[1].position, Vec3::new(1.0, 2.0, 3.0));
    assert_eq!(g.common.rockets[2].position, Vec3::new(4.0, 5.0, 6.0));

    // The other arm: a left main arm, or a rocket only in the offhand.
    let left = [10.0 - 0.0868, 70.0, 20.0 + 0.4924];
    assert!(near(hand(true, rocket(), None), left));
    assert!(near(hand(false, None, rocket()), left));
    assert!(near(hand(true, None, rocket()), right));
    assert!(near(hand(false, rocket(), rocket()), right));

    // Not gliding: it follows the player itself, without boosting it.
    let mut g = game(true);
    g.common.player.as_mut().unwrap().fall_flying = false;
    g.tick_own_rockets();
    assert_eq!(g.common.player.as_ref().unwrap().velocity, Vec3::new(0.0, -0.2, 1.0));
    assert_eq!(g.common.rockets[0].position, Vec3::new(10.0, 70.0, 20.0));
    assert_eq!(g.client.particles.0.added.len(), 1);

    // A frozen level ticks no entities.
    let mut g = game(false);
    g.tick_own_rockets();
    assert!(g.client.particles.0.added.is_empty());
    assert_eq!(g.common.player.as_ref().unwrap().velocity, Vec3::new(0.0, -0.2, 1.0));
}
'''.replace("METHODS", methods)
        with tempfile.TemporaryDirectory(prefix="remote-own-rockets-") as directory:
            path = Path(directory)
            (path / "main.rs").write_text(fixture)
            built = subprocess.run(
                ["rustc", "--edition=2024", str(path / "main.rs"), "-o", str(path / "probe")],
                capture_output=True,
                text=True,
            )
            self.assertEqual(built.returncode, 0, built.stderr)
            result = subprocess.run([str(path / "probe")], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
