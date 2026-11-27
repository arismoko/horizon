use super::*;
use sunrise_gameplay_common::game::native_hooks::player_bed_leave::WAKING;

/// The hook points. Each does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// [`Self::native_player_bed_leave`] for the player acting now, who sleeps
    /// in `self.sleeping`; its name is copied only when a mod listens.
    pub(crate) fn native_acting_bed_leave(&mut self, wake_immediately: bool, update_level: bool) {
        if let (Some(head), Some(player)) = (self.common.sleeping, self.server.lan_acting_uuid)
            && mod_api::__private::listened(mod_api::HookId::PlayerBedLeave)
        {
            let name = self.server.lan_acting_name.clone().unwrap_or_default();
            self.native_player_bed_leave(player, &name, head, wake_immediately, update_level);
        }
    }
    /// `PlayerWakeUpEvent`: a server player is getting out of the bed at
    /// `pos` (`Player.stopSleepInBed`), heard first, while they still read
    /// as sleeping there. `wake_immediately` is vanilla's flag for skipping
    /// the wake-up fade. Morning uses (false, false), explicit leave/sneak
    /// and bed-rule refusal use (false, true), and generic stop-sleeping,
    /// death and living-entity continuation failure use (true, true).
    pub(crate) fn native_player_bed_leave(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        wake_immediately: bool,
        update_level: bool,
    ) {
        // A listener that wakes this player again (`stop_sleeping`) must not
        // be told again; the caller re-reads the sleep state afterwards.
        if WAKING.with_borrow(|waking| waking.contains(&player)) {
            return;
        }
        struct Heard(u128);
        impl Drop for Heard {
            fn drop(&mut self) {
                WAKING.with_borrow_mut(|waking| waking.retain(|p| *p != self.0));
            }
        }
        WAKING.with_borrow_mut(|waking| waking.push(player));
        let _heard = Heard(player);
        self.fire_player_bed_leave(player, name, pos, wake_immediately, update_level);
    }
}
