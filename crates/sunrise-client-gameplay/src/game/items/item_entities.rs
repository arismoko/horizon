#[cfg(test)]
pub(in crate::game) mod bob_tests {
    use sunrise_core::geometry::Vec3;
    use sunrise_core::random::RandomSource;

    /// The bob a client gives the item with entity id `id` at `at`.
    fn bob(id: u32, at: Vec3, still: bool) -> f32 {
        let mut random = sunrise_sim::mobs::entity_random(id, at);
        crate::game::client_side::item_entities_client::bob_offset(at, random.next_float(), still)
    }

    /// Under `SUNRISE_STILL` the item's position decides its bob, not the id the server gave
    /// it; without it the bob is the id-seeded random's own draw.
    #[test]
    fn a_still_items_bob_comes_from_its_position() {
        let at = Vec3::new(0.5, 64.0, 2.5);
        assert_eq!(bob(17, at, true), bob(22, at, true));
        for id in [17, 22] {
            let mut random = sunrise_sim::mobs::entity_random(id, at);
            let vanilla = random.next_float() * std::f32::consts::PI * 2.0;
            assert_eq!(bob(id, at, false), vanilla);
        }
        assert_ne!(bob(17, at, false), bob(22, at, false));
    }
}
