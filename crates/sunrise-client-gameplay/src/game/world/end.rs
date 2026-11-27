#[cfg(test)]
pub(crate) mod tests {

    /// The eye sprite's atlas region; only its UVs are used here.
    pub fn eye_rect() -> sunrise_client::hud::Rect {
        sunrise_client::hud::Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 16,
            h: 16,
        }
    }

    use crate::game::client_side::end_client::eye_quad;

    #[test]
    fn a_thrown_eye_is_lifted_by_the_item_ground_transform() {
        let right = glam::Vec3::X * 0.25;
        let up = glam::Vec3::Y * 0.25;
        let quad = eye_quad(glam::Vec3::new(0.0, 100.0, 0.0), right, up, eye_rect());
        let corners = quad.map(|v| glam::Vec3::from_array(v.pos));
        let centre = corners.iter().copied().sum::<glam::Vec3>() / 4.0;
        let low = corners.iter().copied().reduce(glam::Vec3::min).unwrap();
        let high = corners.iter().copied().reduce(glam::Vec3::max).unwrap();
        // The eye at y = 100 + item/generated's 0.125 lift.
        assert!(
            centre.abs_diff_eq(glam::Vec3::new(0.0, 100.125, 0.0), 1e-5),
            "{centre}"
        );
        assert_eq!((low.x, high.x), (-0.25, 0.25));
        assert_eq!((high.x - low.x) * 0.5, 0.25);
        assert_eq!((high.y - low.y) * 0.5, 0.25);
        // ThrownItemRenderer draws the eye full-bright.
        assert!(quad.iter().all(|v| v.light == 0xF0F0));
    }
    #[test]
    fn a_thrown_eye_is_lifted_along_the_cameras_up_axis() {
        // A camera tilted down 45 degrees: its up axis is (0, 1, 1)/sqrt(2), so the lift must
        // move the quad along that axis, not straight up world Y.
        let up_dir = glam::Vec3::new(0.0, 1.0, 1.0).normalize();
        let at = glam::Vec3::new(0.0, 100.0, 0.0);
        let quad = eye_quad(at, glam::Vec3::X * 0.25, up_dir * 0.25, eye_rect());
        let corners = quad.map(|v| glam::Vec3::from_array(v.pos));
        let centre = corners.iter().copied().sum::<glam::Vec3>() / 4.0;
        assert!((centre - at).abs_diff_eq(up_dir * 0.125, 1e-5), "{centre}");
    }
}
