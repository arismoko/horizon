use crate::game::spill_common::Spill;
use mod_api::{LevelRef, PlayerRef};
use sunrise_world::level::FluidWrite;

/// A bucket's sound and fluid game event waiting for the outermost commit.
pub struct FluidEffects {
    pub level: LevelRef,
    pub pos: [i32; 3],
    /// The bucket sound event resolved at enqueue time; none where the fluid has no sound.
    pub sound: Option<&'static str>,
    /// A fill (`fluid_pickup`), else an empty (`fluid_place`).
    pub pickup: bool,
    pub player: Option<PlayerRef>,
}

/// A world pickup or placement decided at the call, its block already set in the transaction:
/// what the outermost commit still does.
pub struct WrittenFluid {
    pub level: LevelRef,
    pub write: FluidWrite,
    /// The broken block's drops, made from the block as it was at the call.
    pub drops: Spill,
    /// A pickup (the bucket's fill sound and `fluid_pickup`), else a placement.
    pub pickup: bool,
    pub player: Option<PlayerRef>,
}
