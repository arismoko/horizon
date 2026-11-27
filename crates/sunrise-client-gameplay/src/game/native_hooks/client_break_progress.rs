use super::*;

impl crate::game::ClientGame<'_> {
    /// `ClientLevel.destroyBlockProgress` told mods: `breaker` cracked `pos` to `progress`, any
    /// stage outside 0..10 taking the crack away. Only a client of a server has a heard level, so
    /// single player's server thread never tells it; with no listener it costs one flag check.
    /// Client-side.
    pub(in crate::game) fn native_client_break_progress(
        &self,
        breaker: i32,
        pos: [i32; 3],
        progress: i32,
    ) {
        if !mod_api::__private::listened(mod_api::HookId::ClientBreakProgress) {
            return;
        }
        let Some(dimension) = self
            .client
            .remote
            .as_ref()
            .and_then(|r| r.heard_level.as_deref())
        else {
            return;
        };
        self.fire_client_break_progress(
            dimension,
            breaker,
            BlockPos::new(pos[0], pos[1], pos[2]),
            progress,
        );
    }

    /// The local player's dig told mods as `MultiPlayerGameMode` tells its level: the dig's
    /// stage (`getDestroyStage`), -1 before it has any progress.
    pub(in crate::game) fn native_local_break_progress(&self, pos: [i32; 3], progress: f32) {
        let stage = if progress > 0.0 {
            (progress * 10.0) as i32
        } else {
            -1
        };
        self.native_client_break_progress(self.client_own_entity_id(), pos, stage);
    }
}
