/// The pose a native mod asks for the first-person hand, each out-of-range
/// or non-finite component replaced by its identity.
pub fn hand_pose_matrix(values: [f32; 7]) -> glam::Mat4 {
    let valid = |value: f32, min: f32, max: f32, default: f32| {
        if value.is_finite() && (min..=max).contains(&value) {
            value
        } else {
            default
        }
    };
    let [x, y, z, pitch, yaw, roll, scale] = values;
    glam::Mat4::from_translation(glam::Vec3::new(
        valid(x, -16.0, 16.0, 0.0),
        valid(y, -16.0, 16.0, 0.0),
        valid(z, -16.0, 16.0, 0.0),
    )) * glam::Mat4::from_rotation_x(valid(pitch, -3600.0, 3600.0, 0.0).to_radians())
        * glam::Mat4::from_rotation_y(valid(yaw, -3600.0, 3600.0, 0.0).to_radians())
        * glam::Mat4::from_rotation_z(valid(roll, -3600.0, 3600.0, 0.0).to_radians())
        * glam::Mat4::from_scale(glam::Vec3::splat(valid(scale, 0.01, 100.0, 1.0)))
}
