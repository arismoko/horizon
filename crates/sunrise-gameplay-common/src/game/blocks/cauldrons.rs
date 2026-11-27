//! Using an item on a cauldron (`CauldronInteraction`): what the click does, and doing it.
use super::*;

/// `CauldronInteractions.isUnderWater`: the *fluid* above the cauldron is in
/// `FluidTags.WATER`. A waterlogged slab or stair, kelp, seagrass and a bubble
/// column are water there even though the block is not named `water`.
pub(in crate::game) fn is_under_water(facts: &BlockFacts, above: BlockStateId) -> bool {
    facts.get(above).fluid == Fluid::Water
}
