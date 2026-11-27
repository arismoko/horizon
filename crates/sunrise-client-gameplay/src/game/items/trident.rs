#[cfg(test)]
pub(in crate::game) mod tests {
    use crate::game::client_side::trident_client::remote_tick;
    use sunrise_core::geometry::{Aabb, Vec3};
    use sunrise_sim::thrown_trident::ThrownTrident;
    use sunrise_sim::{mobs::Who, projectiles::ProjectileWorld};

    struct Air;
    impl ProjectileWorld for Air {
        fn clip(&self, _: Vec3, _: Vec3) -> Option<(Vec3, [i32; 3], [i32; 3])> {
            None
        }
        fn in_water(&self, _: Aabb) -> bool {
            false
        }
        fn targets(&self) -> &[(Who, Aabb)] {
            &[]
        }
    }

    #[test]
    fn remote_trident_waits_for_a_non_self_move_to_leave_its_block() {
        let mut trident = ThrownTrident::remote(1, Vec3::new(0.5, 100.0, 0.5), 0.0, 0.0);
        trident.in_ground = true;
        trident.velocity = Vec3::new(1.0, -2.0, 3.0);
        remote_tick(&mut trident, &Air, None);
        assert!(trident.in_ground);
        assert_eq!(trident.velocity, Vec3::new(1.0, -2.0, 3.0));
    }
}
