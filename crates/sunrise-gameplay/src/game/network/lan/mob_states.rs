//! Mob metadata and live attributes, built on the owning server thread.
use super::*;
use sunrise_gameplay_common::game::lan_mob_metadata::mob_freeze_meta;
impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn fill_mob_state(
        &self,
        m: &Mob,
        hub: &Hub,
        player_entity: Option<i32>,
        speeds: &mut super::super::lan_attributes::SpeedByType,
        e: &mut sunrise_sim::lan::entity::EntityState,
    ) {
        super::super::villager_client::server_meta(
            m,
            hub,
            player_entity,
            self.common.world.game_time,
            &mut e.meta,
            &mut e.defaults,
            &self.stations().professions,
        );
        // HappyGhast.aiStep: requiresPrecisePosition while on a still timeout.
        e.precise = m.species() == sunrise_sim::mobs::Species::HappyGhast
            && sunrise_sim::mobs::happy_ghast::on_still_timeout(m);
        mob_freeze_meta(m, &mut e.meta, &mut e.defaults);
        self.mob_attributes(m, speeds, &mut e.attributes);
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;
