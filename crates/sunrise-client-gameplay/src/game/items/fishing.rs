#[cfg(test)]
pub(crate) mod tests {

    use sunrise_core::geometry::Vec3;

    /// `FishingHookRenderer.getPlayerHandPos` in first person on HotSpot 25.0.4.1: the camera at
    /// yaw 30 looking straight up, fov 70 on 854 x 480, the right arm mid-swing (0.5). The
    /// offset from the eye, as double bits.
    #[test]
    fn the_rod_hand_stays_on_the_near_plane_looking_straight_up() {
        let mut camera = sunrise_client::camera::Camera::default();
        camera.set_rotation(30.0, -90.0);
        let plane = camera.near_plane(70.0, 854.0, 480.0);
        assert_eq!(
            crate::game::client_side::fishing_client::near_plane_hand(plane, 70.0, 1.0, 0.5)
                .to_array()
                .map(f64::to_bits),
            [
                0xbfdc_771f_22e0_62d8,
                0x3fe5_6f4f_fc00_3d1e,
                0x3fc5_139d_516c_7487,
            ]
        );
    }
    #[test]
    fn removing_one_of_two_remote_hooks_clears_owner_fishing() {
        let mut hooks = Vec::new();
        crate::game::client_side::fishing_client::push_remote_hook(
            &mut hooks,
            10,
            7,
            Vec3::ZERO,
            Vec3::ZERO,
            [0; 4],
        );
        crate::game::client_side::fishing_client::push_remote_hook(
            &mut hooks,
            11,
            7,
            Vec3::ZERO,
            Vec3::ZERO,
            [0; 4],
        );
        crate::game::client_side::fishing_client::push_remote_hook(
            &mut hooks,
            20,
            12,
            Vec3::ZERO,
            Vec3::ZERO,
            [0; 4],
        );
        assert!(!hooks[0].owner_fishing);
        assert!(hooks[1].owner_fishing);
        assert!(hooks[2].owner_fishing);
        crate::game::client_side::fishing_client::remove_remote_hook_from(&mut hooks, 10);
        assert_eq!(hooks.len(), 2);
        assert!(!hooks.iter().any(|h| h.owner == 7 && h.owner_fishing));
        assert!(hooks.iter().any(|h| h.owner == 12 && h.owner_fishing));
    }
}
