use super::*;
use sunrise_gameplay_common::game::remote::minecarts::CartLerp;
use sunrise_gameplay_common::game::remote::minecarts::CartStep;

pub(in crate::game) trait CartLerpClientExt: Sized {
    fn at(&self, partial: f32) -> (Vec3, Vec3, f32, f32);
    fn new() -> Self;
    fn tick(&mut self, c: &mut super::super::minecarts::Minecart) -> bool;
}
impl CartLerpClientExt for CartLerp {
    /// `NewMinecartBehavior.getCartLerpPosition`, `getCartLerpMovements` and
    /// `getCartLerpYRot` / `getCartLerpXRot` at `partial`: the step the weights
    /// put this moment in, and how far along it from the one before.
    fn at(&self, partial: f32) -> (Vec3, Vec3, f32, f32) {
        let alpha = ((3 - self.delay) as f32 + partial) / 3.0;
        let mut count_up = 0.0f32;
        let mut in_step = 1.0f32;
        let mut found = None;
        for (i, s) in self.current.iter().enumerate() {
            if s.weight <= 0.0 {
                continue;
            }
            count_up += s.weight;
            if f64::from(count_up) >= self.total_weight * f64::from(alpha) {
                let before = count_up - s.weight;
                in_step = ((f64::from(alpha) * self.total_weight - f64::from(before))
                    / f64::from(s.weight)) as f32;
                found = Some(i);
                break;
            }
        }
        let index = found.unwrap_or(self.current.len() - 1);
        let current = self.current[index];
        let previous = if index > 0 {
            self.current[index - 1]
        } else {
            self.old
        };
        let t = f64::from(in_step);
        let lerp = |a: Vec3, b: Vec3| {
            Vec3::new(
                a.x + t * (b.x - a.x),
                a.y + t * (b.y - a.y),
                a.z + t * (b.z - a.z),
            )
        };
        (
            lerp(previous.position, current.position),
            lerp(previous.movement, current.movement),
            previous.yaw + in_step * wrap_degrees(current.yaw - previous.yaw),
            previous.pitch + in_step * wrap_degrees(current.pitch - previous.pitch),
        )
    }
    fn new() -> Self {
        Self {
            pending: Vec::new(),
            current: Vec::new(),
            total_weight: 0.0,
            delay: 0,
            old: CartStep {
                position: Vec3::ZERO,
                movement: Vec3::ZERO,
                yaw: 0.0,
                pitch: 0.0,
                weight: 0.0,
            },
        }
    }
    /// `NewMinecartBehavior.lerpClientPositionAndRotation`: once the steps
    /// being played are used up, the queued ones become the current ones;
    /// the cart then sits where the steps put it at the end of this tick.
    /// False when it has no steps to play.
    fn tick(&mut self, c: &mut super::super::minecarts::Minecart) -> bool {
        self.delay -= 1;
        if self.delay <= 0 {
            self.old = CartStep {
                position: c.position,
                movement: c.velocity,
                yaw: c.yaw,
                pitch: c.pitch,
                weight: 0.0,
            };
            self.current.clear();
            if !self.pending.is_empty() {
                self.current.append(&mut self.pending);
                self.total_weight = self.current.iter().map(|s| f64::from(s.weight)).sum();
                self.delay = if self.total_weight == 0.0 { 0 } else { 3 };
            }
        }
        if self.current.is_empty() {
            return false;
        }
        let (position, movement, yaw, pitch) = self.at(1.0);
        c.position = position;
        c.velocity = movement;
        c.yaw = yaw;
        c.pitch = pitch;
        true
    }
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `ClientPacketListener.handleMinecartAlongTrack`: the steps join the ones
    /// the minecart has queued, if it runs `NewMinecartBehavior`.
    pub(in crate::game) fn remote_move_minecart(&mut self, p: play::MoveMinecartAlongTrack) {
        if !self.minecart_improvements() || !self.common.carts.iter().any(|c| c.id == p.id as u32) {
            return;
        }
        let Some(r) = self.client.remote.as_mut() else {
            return;
        };
        let at = |v: [f64; 3]| Vec3::new(v[0], v[1], v[2]);
        r.cart_lerps
            .entry(p.id)
            .or_insert_with(CartLerp::new)
            .pending
            .extend(p.steps.iter().map(|s| CartStep {
                position: at(s.position),
                movement: at(s.movement),
                yaw: sunrise_protocol::buf::unpack_degrees(s.yaw),
                pitch: sunrise_protocol::buf::unpack_degrees(s.pitch),
                weight: s.weight,
            }));
    }
}

use sunrise_gameplay_common::game::minecarts::MinecartRailGameExt as _;

#[cfg(feature = "client")]
#[cfg(test)]
mod tests {
    use super::{CartLerp, CartLerpClientExt as _, CartStep, Vec3};
    use crate::game::native_entity_refs::tests::{run_fresh_cases, with_client_game};
    use mod_api::{CameraPositionEvent, ClientHost, ComputeCameraAnglesEvent, Hooks, ModEntry};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use sunrise_gameplay_common::game::Screen;
    use sunrise_gameplay_common::game::minecarts::{CartKind, Minecart, SEAT};
    use sunrise_protocol::connection::Pipe;
    use sunrise_sim::motion::PlayerMotion;

    struct CameraCalls {
        angles: Arc<AtomicUsize>,
        positions: Arc<Mutex<Vec<[f64; 3]>>>,
    }

    #[mod_api::__subscriptions]
    impl Hooks for CameraCalls {
        fn compute_camera_angles(&self, _: &dyn ClientHost, event: &mut ComputeCameraAnglesEvent) {
            self.angles.fetch_add(1, Ordering::Relaxed);
            event.roll = 90.0;
        }

        fn camera_position(&self, _: &dyn ClientHost, event: &mut CameraPositionEvent<'_>) {
            self.positions
                .lock()
                .unwrap()
                .push(event.position.coordinates());
        }
    }

    #[test]
    #[ignore = "daily"]
    fn interpolating_local_rider_skips_angles_but_lends_placed_position() {
        if run_fresh_cases(
            "game::client_side::remote_client::minecarts::tests::interpolating_local_rider_skips_angles_but_lends_placed_position",
            "SUNRISE_CAMERA_MINECART_CHILD",
            &["cart"],
        ) {
            return;
        }
        let angles = Arc::new(AtomicUsize::new(0));
        let positions = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new(
            "cart-camera",
            Box::new(CameraCalls {
                angles: angles.clone(),
                positions: positions.clone(),
            }),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_client_game(|game| {
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "CartCameraTest").unwrap();
                game.client.remote.as_mut().unwrap().features =
                    Arc::from(["minecraft:minecart_improvements".to_owned()]);
                game.common.player =
                    Some(PlayerMotion::new(game.common.motion, Vec3::ZERO).unwrap());
                game.client.title.0 = None;
                game.client.screen = Screen::Playing;
                game.client.camera_type = sunrise_level::camera_math::CameraType::FirstPerson;
                game.client.view.eye_height = 1.0;
                game.client.view.eye_height_old = 1.0;

                let id = 31;
                let mut cart = Minecart::new(id, CartKind::Rideable, Vec3::new(16.0, 64.0, 24.0));
                let mut lerp = CartLerp::new();
                lerp.pending.push(CartStep {
                    position: Vec3::new(24.0, 68.0, 40.0),
                    movement: Vec3::ZERO,
                    yaw: 0.0,
                    pitch: 0.0,
                    weight: 1.0,
                });
                assert!(lerp.tick(&mut cart));
                game.common.carts.push(cart);
                game.client
                    .remote
                    .as_mut()
                    .unwrap()
                    .cart_lerps
                    .insert(id as i32, lerp);
                game.common.riding = Some(id);

                // A quarter of the way from the old cart position to its first queued step,
                // plus the seat and the local player's interpolated eye height.
                let expected = [18.0, (65.0 + SEAT) + 1.0, 28.0];
                let (position, _, _, roll) = game.frame_camera(0.75).unwrap();
                assert_eq!(position.to_array(), expected);
                assert_eq!(roll, 0.0);
                assert_eq!(angles.load(Ordering::Relaxed), 0);
                assert_eq!(*positions.lock().unwrap(), [expected]);
            });
        });
    }
}
