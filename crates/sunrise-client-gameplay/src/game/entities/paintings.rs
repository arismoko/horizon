#[cfg(test)]
pub(in crate::game) mod face_tests {
    use crate::game::client_side::paintings_client::faces_camera;
    use glam::DVec3;

    /// A quad at z = 2 facing -z (towards a camera at the origin) shows; seen from behind
    /// (the camera on the other side, the quad now at z = -2) it does not.
    #[test]
    fn a_quad_shows_only_to_the_side_it_faces() {
        let at = |z: f64| {
            [
                DVec3::new(-1.0, -1.0, z),
                DVec3::new(1.0, -1.0, z),
                DVec3::new(1.0, 1.0, z),
                DVec3::new(-1.0, 1.0, z),
            ]
        };
        let normal = DVec3::new(0.0, 0.0, -1.0);
        assert!(faces_camera(&at(2.0), normal));
        assert!(!faces_camera(&at(-2.0), normal));
        // Edge on: nothing to draw.
        assert!(!faces_camera(&at(0.0), normal));
    }
}
