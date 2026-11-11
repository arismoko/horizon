/// L1: while a hopper cools down (its real cooldown above 1), take one tick off and skip its tick.
#[derive(Default)]
pub struct Throttle;

impl BlockEntityTickSlot for Throttle {
    fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
        if tick.kind != TickedKind::Hopper {
            return LoopFlow::Continue;
        }
        match tick.game.hoppers.get_mut(&tick.pos) {
            Some(state) if state.cooldown > 1 => {
                state.cooldown -= 1;
                LoopFlow::Skip
            }
            _ => LoopFlow::Continue,
        }
    }
}
