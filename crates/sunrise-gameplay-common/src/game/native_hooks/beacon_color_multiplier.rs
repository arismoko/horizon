use super::*;

pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// `IBlockExtension.getBeaconColorMultiplier`: the colour `state` at `pos` gives the beam of
    /// the beacon at `beacon_pos`. `color` is the block's own answer, which its parts may change.
    fn native_beacon_color_multiplier(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        beacon_pos: BlockPos,
        color: &mut Option<u32>,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_beacon_color_multiplier(
            self, state, pos, beacon_pos, color,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
