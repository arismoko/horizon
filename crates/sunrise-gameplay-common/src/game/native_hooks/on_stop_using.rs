use super::*;

/// The hook points. Each does nothing in Sunrise.
pub trait GameHooks<'g>: crate::game::SideGame<'g> {
    /// `IItemExtension.onStopUsing`: the use of `stack` by `entity` (`None`
    /// for a player) in `hand` is over, `time_left` ticks before its end (0
    /// for a finished use). After the use is cleared. Both sides.
    #[inline(never)]
    fn native_on_stop_using(
        &mut self,
        stack: &ItemStack,
        entity: Option<i32>,
        hand: Hand,
        time_left: i32,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_on_stop_using(
            self, stack, entity, hand, time_left,
        );
    }
}

impl<'g, G: crate::game::SideGame<'g>> GameHooks<'g> for G {}
