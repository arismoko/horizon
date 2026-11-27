//! The tracking hooks at their pairing (#4522). NeoForge posts `PlayerEvent.StartTracking` at the
//! end of `ServerEntity.addPairing`, after the spawn bundle, and `StopTracking` at the end of
//! `removePairing`, after the removal: what a listener sends the player follows that packet. A
//! level's tracking pass builds every player's output at once, on several threads, so while a
//! native mod listens the pass keeps back each output that started or stopped tracking, with
//! where each start's bundle and each stop's removal ends in it (`TrackEvent::at`). The game
//! sends it up to each of those packets, tells the mods, and goes on (`send_tracking_out`); the
//! players' own state follows the pass as before (`finish_tracking_pass`).
use super::Hub;
use crate::lan::Body;

/// Whether a native mod hears tracking: only then does a pass keep its output for the game.
pub(super) fn tracking_listened() -> bool {
    mod_api::__private::listened(mod_api::HookId::PlayerStartTracking)
        || mod_api::__private::listened(mod_api::HookId::PlayerStopTracking)
}

impl Hub {
    /// Send part of a pass's output kept for the game (`TrackingBatch::held`) to the player on
    /// `conn`, as the pass would have: a removal a sound still needs waits for it, sent by
    /// `finish_captured_sounds`. Gives back the ids whose removal waits, so the game tells their
    /// stops after it (`tracking_after_sounds`).
    pub fn send_tracking_out(&self, conn: u32, mut bodies: Vec<Body>) -> Vec<i32> {
        if bodies.is_empty() {
            return Vec::new();
        }
        let Some(g) = self.guest(conn) else {
            return Vec::new();
        };
        let waiting = self.hold_sound_removals(conn, &mut bodies);
        g.sender.send_tracking_bodies(&mut bodies);
        waiting
    }

    /// After the game told the mods of the level's pass, or where nothing tells them (a client's
    /// hub, a caller of `tick`): what is still kept goes out, then the players' own state, which
    /// follows the pass.
    pub fn finish_tracking_pass(&mut self) {
        let kept: Vec<(u32, Vec<Body>)> = self
            .native_tracking
            .iter_mut()
            .filter(|b| !b.held.is_empty())
            .map(|b| {
                for e in &mut b.events {
                    e.at = None;
                }
                (b.conn, std::mem::take(&mut b.held))
            })
            .collect();
        for (conn, bodies) in kept {
            let _ = self.send_tracking_out(conn, bodies);
        }
        if std::mem::take(&mut self.players_after_tracking) {
            self.sync_players();
        }
    }
}
