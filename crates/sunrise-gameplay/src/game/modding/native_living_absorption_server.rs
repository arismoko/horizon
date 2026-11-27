//! Server-side absorption read and write for native mods (`LivingEntity.getAbsorptionAmount`
//! and `setAbsorptionAmount`).
use super::commands::Target;
use crate::game::ServerGame;
use mod_api::LivingRef;

impl ServerGame<'_> {
    /// Server arm for `LivingEntity.getAbsorptionAmount`.
    pub(in crate::game) fn server_living_absorption(&self, living: LivingRef) -> Option<f32> {
        let target = self.server_living_read_target(living)?;
        super::native_living_body::body_absorption(self.native_living_body(target)?)
    }

    /// Sets the entity's absorption amount (`LivingEntity.setAbsorptionAmount`),
    /// cut to `0..=max_absorption`. No heal or damage hook runs. A player's new
    /// amount reaches their client and the trackers through the synced data.
    /// False for a stale handle, a non-finite amount, an armor stand or the
    /// dragon.
    pub(in crate::game) fn living_set_absorption(
        &mut self,
        living: LivingRef,
        amount: f32,
    ) -> bool {
        if !amount.is_finite() {
            return false;
        }
        match self.native_living_target(living) {
            Some(t @ (Target::Player | Target::Guest(_))) => self
                .for_player(t, |g| {
                    g.player.as_ref()?;
                    g.journal_player_health();
                    g.player.as_mut()?.set_health(amount, true).ok()
                })
                .flatten()
                .is_some(),
            Some(Target::Mob(id)) => {
                self.journal_mob_health(id);
                let Some(m) = self
                    .common
                    .mobs
                    .iter_mut()
                    .find(|m| m.id == id && !m.removed)
                else {
                    return false;
                };
                let max = sunrise_sim::mobs::armor::max_absorption(m);
                m.absorption = if amount < 0.0 {
                    0.0
                } else {
                    amount.min(max.max(0.0))
                };
                m.needs_sync = true;
                true
            }
            _ => false,
        }
    }
}

use sunrise_gameplay_common::game::native_living_body::NativeLivingBodyGameExt1 as _;
