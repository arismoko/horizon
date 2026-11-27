use super::*;

pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// `IBlockExtension.getEnchantPowerBonus`: the power `state` at `pos` gives the enchanting
    /// table at `table_pos`. `power` is the block's own answer, which its parts may change.
    fn native_enchant_power_bonus(
        &self,
        level: Option<mod_api::LevelRef>,
        state: BlockStateId,
        pos: BlockPos,
        table_pos: BlockPos,
        power: &mut f32,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_enchant_power_bonus(
            self, level, state, pos, table_pos, power,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
