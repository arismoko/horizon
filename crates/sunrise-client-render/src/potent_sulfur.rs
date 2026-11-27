//! Client render adapter over [`sunrise_level::potent_sulfur_level`] for [`ParticleLevel`].
use crate::particles::ParticleLevel;
pub use sunrise_level::potent_sulfur_level::SulfurLevel;

#[inline]
pub fn is_geyser_passable_block(level: &ParticleLevel, pos: [i32; 3], context_y: i32) -> bool {
    level.with_sulfur_level(|s| {
        sunrise_level::potent_sulfur_level::is_geyser_passable_block(s, pos, context_y)
    })
}

#[inline]
pub fn find_noxious_gas_source_block(level: &ParticleLevel, origin: [i32; 3]) -> Option<[i32; 3]> {
    level.with_sulfur_level(|s| {
        sunrise_level::potent_sulfur_level::find_noxious_gas_source_block(s, origin)
    })
}

#[inline]
pub fn unobstructed_block_count(level: &ParticleLevel, pos: [i32; 3], water_blocks: i32) -> i32 {
    level.with_sulfur_level(|s| {
        sunrise_level::potent_sulfur_level::unobstructed_block_count(s, pos, water_blocks)
    })
}

#[inline]
pub fn can_be_reached_by_noxious_gas(
    level: &ParticleLevel,
    source_block: [i32; 3],
    pos: [f64; 3],
) -> bool {
    level.with_sulfur_level(|s| {
        sunrise_level::potent_sulfur_level::can_be_reached_by_noxious_gas(s, source_block, pos)
    })
}
