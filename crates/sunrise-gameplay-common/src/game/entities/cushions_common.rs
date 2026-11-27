use sunrise_core::geometry::{Aabb, Vec3};

/// `Direction.fromYRot(yaw).toYRot()`: the nearest 90 degree step.
pub fn snap_yaw(yaw: f32) -> f32 {
    (((f64::from(yaw) / 90.0 + 0.5).floor() as i64) & 3) as f32 * 90.0
}

pub fn aabb(bounds: ([f64; 3], [f64; 3])) -> Aabb {
    Aabb::new(
        Vec3::new(bounds.0[0], bounds.0[1], bounds.0[2]),
        Vec3::new(bounds.1[0], bounds.1[1], bounds.1[2]),
    )
}
