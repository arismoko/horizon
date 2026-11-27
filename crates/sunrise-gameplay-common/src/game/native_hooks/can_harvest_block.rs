use super::*;

pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// Whether the player's tool harvests the block `state` at `pos`
    /// (`IBlockExtension.canHarvestBlock`): `can_harvest` starts with
    /// vanilla's answer and the block's parts may change it.
    fn native_can_harvest_block(
        &mut self,
        player: u32,
        pos: [i32; 3],
        state: BlockStateId,
        can_harvest: &mut bool,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_can_harvest_block(
            self,
            player,
            pos,
            state,
            can_harvest,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
