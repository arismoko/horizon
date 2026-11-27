use sunrise_core::{geometry::Vec3, random::RandomSource};

/// A dropped item's `bobOffs` (its bob and spin phase) from its random's draw `drawn`, or under
/// `SUNRISE_STILL` from a random seeded by its position alone: a client seeds an item's random
/// with the server's entity id, which follows the order chunks load in and differs between
/// launches. The draw is made either way, so the item's random stays where vanilla's would be.
pub(in crate::game) fn bob_offset(position: Vec3, drawn: f32, still: bool) -> f32 {
    let drawn = if still {
        sunrise_sim::mobs::entity_random(0, position).next_float()
    } else {
        drawn
    };
    drawn * std::f32::consts::PI * 2.0
}
