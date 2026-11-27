//! Shared moving-collider shoving math (`SweepKind`, turning/translation sweeps,
//! and velocity cancellation). Server shoving lives in `shoving_server.rs`;
//! client prediction lives in `piston_push_client.rs`.
use sunrise_core::geometry::{Aabb, Vec3, oriented::OrientedCollider};
use sunrise_level::fast_hash::FxHashMap;
use sunrise_world::moving_colliders::{Placed, Prepared};
use sunrise_world::piston_push;

#[derive(Clone, Copy)]
pub enum SweepKind<'a> {
    Translation {
        before: &'a OrientedCollider,
        delta: Vec3,
    },
    General {
        before: &'a OrientedCollider,
    },
}

impl<'a> SweepKind<'a> {
    pub fn before(&self) -> &'a OrientedCollider {
        match *self {
            Self::Translation { before, .. } | Self::General { before } => before,
        }
    }
}

/// Collects every collider that translated or turned between `start` and `end`,
/// sorted by collider entity id.
pub fn collect_sweeps<'a>(
    start: &'a Prepared,
    end: &'a Prepared,
) -> Vec<(&'a Placed, SweepKind<'a>, Aabb)> {
    let starts: FxHashMap<u32, &Placed> = start.placed().map(|p| (p.id, p)).collect();
    let mut sweeps: Vec<_> = end
        .placed()
        .filter_map(|after| {
            let before = &starts.get(&after.id)?.collider;
            if before.frame == after.collider.frame {
                return None;
            }
            if before.frame.same_turn(&after.collider.frame) {
                let delta = after.collider.frame.origin.subtract(before.frame.origin);
                if delta == Vec3::ZERO {
                    return None;
                }
                let swept = before.world_bounds()?.expand_towards(delta);
                Some((after, SweepKind::Translation { before, delta }, swept))
            } else {
                let bounds = before.boxes.bounds()?;
                let swept = before.frame.swept_bounds(&after.collider.frame, bounds);
                Some((after, SweepKind::General { before }, swept))
            }
        })
        .collect();
    sweeps.sort_unstable_by_key(|(after, ..)| after.id);
    sweeps
}

/// Displacement needed to push `bounds` clear of `after`'s sweep described by `kind`.
pub fn sweep_displacement(
    after: &OrientedCollider,
    kind: SweepKind<'_>,
    bounds: Aabb,
) -> Option<Vec3> {
    match kind {
        SweepKind::Translation { before, delta } => {
            let fraction = before.translation_push_fraction(after, bounds, delta);
            if fraction > 0.0 {
                let direction = delta.scale(1.0 / delta.length());
                // The piston's clearance is added once to its largest box advance.
                Some(
                    delta
                        .scale(fraction)
                        .add_vector(direction.scale(piston_push::PUSH_OFFSET)),
                )
            } else {
                None
            }
        }
        SweepKind::General { before } => {
            before.sweep_push_displacement(after, bounds, piston_push::PUSH_OFFSET)
        }
    }
}

/// Removes the part of `velocity` that points into a shove along `normal`. An upward shove
/// raises vertical velocity to the collider's own vertical speed `collider_rise` but never
/// lowers it, so a body already rising faster keeps its speed.
pub fn cancel_shove_velocity(
    velocity: Vec3,
    normal: Vec3,
    upward: f64,
    collider_rise: f64,
) -> Vec3 {
    let inward = velocity.dot(normal);
    if inward >= 0.0 {
        return velocity;
    }
    let mut velocity = velocity.subtract(normal.scale(inward));
    if upward > 0.0 {
        velocity.y = velocity.y.max(collider_rise);
    }
    velocity
}
