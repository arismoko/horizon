use super::*;

pub trait GameHooks<'g>: crate::game::SideGame<'g> {
    /// `BlockBehaviour.getDestroyProgress` as the block of `state` overrides it: a block Part's.
    fn native_destroy_progress(
        &mut self,
        player: PlayerRef,
        pos: [i32; 3],
        state: BlockStateId,
        progress: &mut Option<f32>,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_destroy_progress(
            self, player, pos, state, progress,
        );
    }
}

impl<'g, G: crate::game::SideGame<'g>> GameHooks<'g> for G {}
