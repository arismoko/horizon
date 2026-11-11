/// L1c: the floor cost of a filled slot, answering Continue for every block entity.
#[derive(Default)]
pub struct Floor;

impl BlockEntityTickSlot for Floor {
    fn before(&mut self, _: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
        LoopFlow::Continue
    }
}
