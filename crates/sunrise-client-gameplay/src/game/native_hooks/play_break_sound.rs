use super::*;
use sunrise_core::position::BlockPos;

impl crate::game::ClientGame<'_> {
    /// `IClientBlockExtensions.playBreakSound` as the `LevelEventHandler` patch asks it: the
    /// block's parts for a non-air `state` at `pos`; true when one cancelled vanilla's sound.
    pub(in crate::game) fn mod_play_break_sound(
        &mut self,
        state: BlockStateId,
        pos: [i32; 3],
    ) -> bool {
        let mut cancel = false;
        // A client hook: the server thread handles level event 2001 too, and never asks it.
        if self.native_window()
            && mod_api::__private::has_scoped_hook(mod_api::HookId::PlayBreakSound)
        {
            self.native_play_break_sound(super::fire::PlayBreakSoundCall {
                state,
                pos: BlockPos::new(pos[0], pos[1], pos[2]),
                cancel: &mut cancel,
            });
        }
        cancel
    }

    pub(in crate::game) fn native_play_break_sound(
        &mut self,
        call: super::fire::PlayBreakSoundCall<'_>,
    ) {
        self.fire_play_break_sound(call);
    }
}
