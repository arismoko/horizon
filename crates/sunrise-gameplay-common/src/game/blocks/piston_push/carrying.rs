//! Carrying: role-neutral moving-collider support lookup and dispatcher.
//! Server carry/shove passes live in `carrying_server.rs` / `shoving_server.rs`;
//! client carry/shove prediction lives in `piston_push_client.rs`.
#[path = "shoving.rs"]
pub mod shoving;
use super::*;
pub use shoving::{SweepKind, cancel_shove_velocity, collect_sweeps, sweep_displacement};
use sunrise_core::geometry::{Axis, COLLISION_EPSILON, oriented::OrientedCollider};
#[allow(unused_imports)]
pub(in crate::game) use sunrise_world::moving_colliders::{self, Prepared};

/// How far under its feet a body may have a box and still stand on it.
pub const SUPPORT_PROBE: f64 = 0.01;

/// A body standing on a collider.
pub struct Rider {
    pub target: MoveWho,
    pub id: u32,
    /// The collider entity it stands on.
    pub support: u32,
}

/// The collider a body with `bounds` and entity `id` stands on: the one box under its feet with
/// the highest top (a tie to the lower collider entity id), found through the level's index.
pub fn support_of(start: &Prepared, bounds: Aabb, id: u32) -> Option<u32> {
    let mut best: Option<(f64, u32)> = None;
    let probe = Aabb::new(
        bounds
            .min_position()
            .subtract(Vec3::new(0.0, SUPPORT_PROBE, 0.0)),
        bounds.max_position(),
    );
    start.meeting(probe, |placed| {
        if placed.exempt.contains(&id) {
            return;
        }
        let allowed = placed.collider.collide(bounds, Axis::Y, -SUPPORT_PROBE);
        if allowed <= -SUPPORT_PROBE {
            return;
        }
        if placed.collider.intersects(bounds) {
            return;
        }
        let gap = -allowed;
        let better =
            best.is_none_or(|(g, c)| gap < g - 1.0e-9 || (gap <= g + 1.0e-9 && placed.id < c));
        if better {
            best = Some((gap, placed.id));
        }
    });
    best.map(|(_, support)| support)
}

/// A rigid contact point turns with its support, but the body stays upright. When the support
/// tilts, the body's uphill bottom edge can dip below the tilted face even when its step height
/// is 0. Find the smallest upward lift onto a face of `support` that clears the embedded
/// footprint while keeping the body resting on `support`.
pub fn upright_tilt_clearance(
    support: &OrientedCollider,
    bounds: Aabb,
    rigid: Vec3,
    heights: &mut Vec<f64>,
) -> Vec3 {
    let proposed = bounds.move_by(rigid);
    if !support.intersects(proposed) {
        return rigid;
    }
    let mut max_top = proposed.min(Axis::Y);
    support.overlapping(proposed, |index| {
        let local = support.boxes.boxes()[index as usize];
        max_top = max_top.max(support.frame.world_bounds(local).max(Axis::Y));
        false
    });
    let max_lift = (max_top - proposed.min(Axis::Y)).max(0.0) + COLLISION_EPSILON;
    if max_lift <= COLLISION_EPSILON {
        return rigid;
    }
    heights.clear();
    support.step_heights(proposed, Vec3::ZERO, max_lift, heights);
    heights.sort_unstable_by(f64::total_cmp);
    heights.dedup();
    for &height in heights.iter() {
        let lifted = proposed.move_by(Vec3::new(0.0, height, 0.0));
        if !support.intersects(lifted)
            && support.collide(lifted, Axis::Y, -SUPPORT_PROBE) > -SUPPORT_PROBE
        {
            return rigid.add_vector(Vec3::new(0.0, height, 0.0));
        }
    }
    rigid
}

pub trait CarryingGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// Carries bodies on colliders that moved this tick, then shoves bodies in the path of a
    /// moving or turning collider. `start` is the colliders as the tick began. Dispatched by role.
    fn carry_pass(&mut self, start: &Prepared) {
        self.side_carry_pass(start);
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> CarryingGameExt1<'__sg> for G {}

use crate::game::side_ops::blocks::BlocksSideGameExt as _;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use sunrise_core::geometry::oriented::{ColliderBoxes, Frame};

    /// A tilting deck clears an upright body's uphill edge regardless of step height (including
    /// step-height-0 bodies such as items).
    #[test]
    fn upright_tilt_clearance_never_embeds_a_zero_step_body() {
        let boxes = Arc::new(ColliderBoxes::new(vec![Aabb::new(
            Vec3::new(-4.0, -1.0, -4.0),
            Vec3::new(4.0, 0.0, 4.0),
        )]));
        let origin = Vec3::new(0.0, 10.0, 0.0);
        let before = Frame::translation(origin);
        let mut heights = Vec::new();
        for (half_w, height) in [(0.125, 0.25), (0.3, 1.8)] {
            for contact in [Vec3::new(2.0, 10.0, 0.0), origin] {
                let body = Aabb::new(
                    contact.add_vector(Vec3::new(-half_w, 0.0, -half_w)),
                    contact.add_vector(Vec3::new(half_w, height, half_w)),
                );
                for axis in [Axis::X, Axis::Z] {
                    for angle in [-0.25_f64, 0.25] {
                        let mut rotation = [0.0, 0.0, 0.0, (angle * 0.5).cos()];
                        rotation[if axis == Axis::X { 0 } else { 2 }] = (angle * 0.5).sin();
                        let support = OrientedCollider::new(
                            Frame::from_quaternion(origin, rotation),
                            boxes.clone(),
                        );
                        let rigid = support
                            .frame
                            .to_world(before.to_local(contact))
                            .subtract(contact);
                        assert!(support.intersects(body.move_by(rigid)));
                        let carried = upright_tilt_clearance(&support, body, rigid, &mut heights);
                        let corrected = body.move_by(carried);
                        assert!(!support.intersects(corrected));
                        assert_eq!(support.collide(corrected, Axis::Y, -SUPPORT_PROBE), 0.0);
                        let expected_extra = half_w * angle.abs().tan();
                        assert!((carried.y - rigid.y - expected_extra).abs() < 1.0e-12);
                    }
                }
            }
        }
    }
}
