use super::*;
use crate::game::tables::BlockProviderQuery;

pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// One dynamic provider answer: outer `None` refuses an active cycle, `Some(None)` keeps
    /// the state's own value. Publication is separate from reading this raw answer.
    fn query_dynamic_light_emission(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        dimension: impl FnOnce() -> LevelDimension,
    ) -> Option<Option<u8>> {
        if !crate::game::api_values::block_state::block_listens(
            mod_api::HookId::LightEmission,
            state,
        ) {
            return Some(None);
        }
        self.with_block_provider_query(state, BlockProviderQuery::LightEmission, || {
            let mut emission = None;
            self.native_light_emission(dimension(), pos, state, &mut emission);
            emission
        })
    }

    /// Raw Part dispatch shared by contextual reads and the light publication path.
    #[inline(always)]
    fn native_light_emission(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        emission: &mut Option<u8>,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_light_emission(
            self, dimension, pos, state, emission,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}

use crate::game::tables::WorldTablesExt as _;
