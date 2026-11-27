use super::commands::Target;
use crate::game::ServerGame;

impl ServerGame<'_> {
    /// The entity id `who` names as a hit's causing entity: a mob's own, the player in place's
    /// for `Who::Player`; 0 for none.
    pub(in crate::game) fn causer_id(&self, who: Option<sunrise_sim::mobs::Who>) -> i32 {
        match who {
            Some(sunrise_sim::mobs::Who::Mob(id)) => id as i32,
            Some(sunrise_sim::mobs::Who::Player) => {
                self.common.player.as_ref().map_or(0, |p| p.entity_id)
            }
            None => 0,
        }
    }
    /// The guest whose entity id `id` is, when the `LivingRef` reads answer for it from its hub
    /// record now (`Target::Guest`, not the player in place): the causing entity a hit may lend
    /// read-only ([`HitLend::parked_guest`](sunrise_client::health::HitLend::parked_guest)).
    /// None for anything else, so a hit's view never answers where `LivingRef` does not.
    pub(in crate::game) fn native_parked_causer(&self, id: i32) -> Option<u32> {
        let epoch = self.common.native_lifecycle.epoch()?;
        let living =
            mod_api::LivingRef::new(mod_api::EntityRef::new(epoch, u32::try_from(id).ok()?));
        match self.native_living_target(living)? {
            Target::Guest(conn) if self.server.lan_acting != Some(conn) => Some(conn),
            _ => None,
        }
    }
}
