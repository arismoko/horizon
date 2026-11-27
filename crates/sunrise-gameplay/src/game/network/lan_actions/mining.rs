use super::*;

impl crate::game::ServerGame<'_> {
    /// Guest `conn` (at `gi`) breaks the block at `pos` at once, as a fake player's `BreakBlock`
    /// does: `handleBlockBreakAction`'s reach, build height, spawn protection and border, then
    /// `ServerPlayerGameMode.destroyBlock`, with its game-mode and tool rules, the native mods'
    /// `break_block`, the tool's wear and the drops, and no digging time. Outside creative, a
    /// block no digging breaks (destroy time -1) stays. True when the block broke.
    pub(in crate::game) fn guest_break_block(
        &mut self,
        conn: u32,
        gi: usize,
        pos: [i32; 3],
    ) -> Result<bool, ActionRefused> {
        let Some(host) = self.server.lan.as_mut() else {
            return Err(ActionRefused::Stale);
        };
        let context = &host.hub.loop_context;
        if !host
            .hub
            .guests
            .update(gi, |g| g.reaches_block(context, pos))
        {
            return Err(ActionRefused::OutOfReach);
        }
        let (min_y, max_y) = (
            self.common.world.shape.min_y,
            self.common.world.shape.max_y() - 1,
        );
        if !(min_y..=max_y).contains(&pos[1])
            || self.under_spawn_protection(gi, pos)
            || !self
                .common
                .border
                .contains(f64::from(pos[0]), f64::from(pos[2]))
        {
            return Err(ActionRefused::Protected);
        }
        let before = self.common.world.block(pos[0], pos[1], pos[2]);
        if self.common.res.facts().is_air(before) {
            return Ok(false);
        }
        // A survival player reaches `destroyBlock` only through its digging, which makes no
        // progress on a block of destroy time -1 (bedrock, barriers, command blocks): only a
        // creative one breaks those.
        let creative = self
            .server
            .lan
            .as_ref()
            .is_some_and(|host| host.hub.guests[gi].creative());
        if !creative && self.common.res.facts().get(before).hardness < 0.0 {
            return Ok(false);
        }
        self.guest_destroy_at(gi, conn, pos)
            .map_err(|_| ActionRefused::Stale)?;
        Ok(self.common.world.block(pos[0], pos[1], pos[2]) != before)
    }

    /// `level.destroyBlockProgress(id, pos, -1)` for a guest: its crack on `pos` is gone, for the
    /// host and the other guests.
    /// A player who dies digs and uses nothing any more: the block it was
    /// breaking, or had left to break (`hasDelayedDestroy`), stays, and its
    /// crack goes. Respawning gives it a new ServerPlayerGameMode (#1315).
    pub(in crate::game) fn guest_forget_digging(&mut self, conn: u32) {
        let Some(play) = self
            .server
            .lan
            .as_mut()
            .and_then(|host| host.play.get_mut(&conn))
        else {
            return;
        };
        let cracked = play.digging.take().filter(|_| play.last_progress >= 0);
        play.mining = None;
        play.delayed = None;
        // Respawning gives it a new ServerPlayerGameMode: the clock goes with the body.
        play.destroy_pos = None;
        play.destroy_delay = 0;
        play.using = None;
        play.last_progress = -1;
        if let Some((pos, _)) = cracked {
            self.guest_crack_gone(conn, pos);
        }
    }
    pub(in crate::game) fn guest_crack_gone(&mut self, conn: u32, pos: [i32; 3]) {
        self.guest_crack_progress(conn, pos, -1);
    }
    /// Only other players strictly within 32 blocks receive crack updates.
    pub(in crate::game) fn guest_crack_progress(
        &mut self,
        conn: u32,
        pos: [i32; 3],
        progress: i32,
    ) {
        let Some(id) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(|g| g.entity_id)
        else {
            return;
        };
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        host.hub.broadcast_near(
            [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])],
            32.0,
            &C::BlockDestruction(play::BlockDestruction {
                id,
                pos,
                progress: progress as u8,
            }),
            Some(conn),
        );
    }
}
