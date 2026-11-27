use super::*;

impl crate::game::ServerGame<'_> {
    /// ServerPlayerGameMode.tick's delayed destroy: the block breaks once its progress since
    /// the digging started is complete, unless something else removed it first.
    pub(in crate::game) fn guest_delayed_destroy(&mut self, conn: u32) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let Some((pos, start)) = host.play.get(&conn).and_then(|p| p.delayed) else {
            return;
        };
        let ticks = host.hub.ticks;
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        let progress = if self.common.res.facts().is_air(state) {
            None
        } else {
            self.as_guest(conn, |g| g.destroy_progress(pos))
                .map(|p| p * (ticks - start + 1) as f32)
        };
        let done = match progress {
            None => false,
            Some(p) if p >= 1.0 => true,
            Some(_) => return,
        };
        if let Some(play) = self
            .server
            .lan
            .as_mut()
            .and_then(|host| host.play.get_mut(&conn))
        {
            play.delayed = None;
        }
        if done {
            self.guest_destroy(conn, pos);
        }
    }
    /// ServerPlayerGameMode.destroyBlock for a guest, its crack cleared for the others
    /// (`destroyBlockProgress(id, pos, -1)`).
    pub(in crate::game::server_side) fn guest_destroy(&mut self, conn: u32, pos: [i32; 3]) {
        if let Some(gi) = self.guest_index(conn) {
            let _ = self.guest_destroy_at(gi, conn, pos);
        }
    }
    /// `guest_destroy` for the guest at `gi`.
    pub(in crate::game) fn guest_destroy_at(
        &mut self,
        gi: usize,
        conn: u32,
        pos: [i32; 3],
    ) -> Result<(), TurnRefused> {
        self.as_guest_at(gi, conn, |g| {
            let _ = g.break_block(pos);
        })?;
        self.guest_crack_gone(conn, pos);
        Ok(())
    }
    /// ServerPlayerGameMode.incrementDestroyProgress, including the first hit.
    pub fn guest_dig_feedback(&mut self, conn: u32) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let Some(play) = host.play.get_mut(&conn) else {
            return;
        };
        let Some((pos, _)) = play.digging else {
            return;
        };
        if play.last_progress >= 0
            && self
                .common
                .res
                .facts()
                .is_air(self.common.world.block(pos[0], pos[1], pos[2]))
        {
            play.last_progress = -1;
            self.guest_crack_gone(conn, pos);
            return;
        }
        let Some(mining) = play.mining.as_ref() else {
            return;
        };
        let progress =
            (mining.per_tick * (host.hub.ticks - play.dig_start + 1) as f32 * 10.0) as i32;
        if progress != play.last_progress && progress >= 0 {
            play.last_progress = progress;
            if mod_api::__private::listened(mod_api::HookId::BlockBreakProgressUpdate) {
                let Some(guest) = host.hub.guest(conn) else {
                    return;
                };
                let (uuid, name) = (guest.uuid, guest.name.clone());
                self.native_block_break_progress_update(uuid, &name, pos, progress);
            }
            self.guest_crack_progress(conn, pos, progress);
        }
    }
    /// ServerPlayerGameMode.handleBlockBreakAction, STOP_DESTROY_BLOCK: the client finished
    /// digging `pos` (and predicted the break). The server breaks it if its own count of the
    /// digging since the START that named `pos` is at least 70 % done; otherwise it keeps digging
    /// alone and breaks it when done (`guest_delayed_destroy`), so the client's broken block is
    /// never left standing on the server.
    pub(in crate::game::server_side) fn guest_stop_digging(&mut self, conn: u32, pos: [i32; 3]) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let ticks = host.hub.ticks;
        let air = self
            .common
            .res
            .facts()
            .is_air(self.common.world.block(pos[0], pos[1], pos[2]));
        let per_tick = if !air
            && host
                .play
                .get(&conn)
                .is_some_and(|p| p.dig_start_for(pos).is_some())
        {
            self.as_guest(conn, |g| g.destroy_progress(pos))
                .unwrap_or(0.0)
        } else {
            0.0
        };
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let decision = {
            let play = host.play.entry(conn).or_default();
            play.dig_ended();
            stop_dig(play, pos, air, ticks, per_tick)
        };
        match decision {
            StopDig::Other => {}
            // Already broken (the guest's own tick dug it out): only the crack goes.
            StopDig::Gone => self.guest_crack_gone(conn, pos),
            StopDig::Break => self.guest_destroy(conn, pos),
            StopDig::Delay(start) => {
                let play = host.play.entry(conn).or_default();
                if play.delayed.is_none() {
                    play.delayed = Some((pos, start));
                }
            }
        }
    }
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
