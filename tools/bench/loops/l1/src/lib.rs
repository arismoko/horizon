//! L1: while a hopper cools down (cooldown above 1), take one tick off and skip its tick.
use mod_api::loops::LoopFlow;
use mod_api::loops::block_entity_tick::{BlockEntityTickSlot, BlockEntityTickView};

#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl mod_api::Hooks for Mod {}

#[derive(Default)]
pub struct Throttle;

impl BlockEntityTickSlot for Throttle {
    fn before(&mut self, view: &mut impl BlockEntityTickView) -> LoopFlow {
        match view.hopper_cooldown() {
            Some(cooldown) if cooldown > 1 => {
                view.set_hopper_cooldown(cooldown - 1);
                LoopFlow::Skip
            }
            _ => LoopFlow::Continue,
        }
    }
}
