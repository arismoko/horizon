//! L1c: the floor cost of a filled slot, answering Continue for every block entity.
use mod_api::loops::LoopFlow;
use mod_api::loops::block_entity_tick::{BlockEntityTickSlot, BlockEntityTickView};

#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl mod_api::Hooks for Mod {}

#[derive(Default)]
pub struct Floor;

impl BlockEntityTickSlot for Floor {
    fn before(&mut self, _: &mut impl BlockEntityTickView) -> LoopFlow {
        LoopFlow::Continue
    }
}
