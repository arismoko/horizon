//! The tail of a mob's death: the wither rose, the copper golem's preserved
//! equipment, and the notice `living_death_post` hears once it has all run.
use super::*;
use sunrise_gameplay_common::game::native_namespaced_id::namespaced;

impl crate::game::ServerGame<'_> {
    /// `LivingEntity.die` after `dropAllDeathLoot`: the wither rose, the
    /// equipment `dropEquipment` owes the copper golem, and then the
    /// `broadcastEntityEvent` moment (`ServerLivingEntityEvents.AFTER_DEATH`).
    /// `source` is the blow `living_death` heard; a death it cancelled never
    /// reaches here.
    pub(in crate::game) fn finish_mob_death(
        &mut self,
        m: &Mob,
        dying: &mut Mob,
        source: Option<(&'static str, u32)>,
        credited_player: Option<u128>,
    ) {
        self.create_wither_rose(m);
        if m.species() == Species::CopperGolem {
            // CopperGolem.dropEquipment: the preserved equipment too.
            self.golem_drop_equipment(dying);
        }
        if !mod_api::__private::listened(mod_api::HookId::LivingDeathPost) {
            return;
        }
        let (damage_type, attacker) = source.unwrap_or(("minecraft:generic", 0));
        let kind = namespaced(&m.kind.id);
        self.native_living_death_post(mod_api::__private::LivingDeathPostCall {
            entity: m.id,
            kind: &kind,
            damage_type,
            attacker,
            player: None,
            credited_player,
            message_key: None,
        });
    }
}
