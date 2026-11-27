use super::*;
use sunrise_gameplay_common::game::remote::effect::force_add_effect;
use sunrise_level::effects::EffectInstance;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `ClientPacketListener.handleUpdateMobEffect` (`LivingEntity.forceAddEffect`): for the
    /// player itself, or another living entity the level holds (a mob, another player or an
    /// armor stand). Any other entity, or an effect the registry doesn't name, changes nothing.
    pub(in crate::game::client_side) fn remote_effect(&mut self, p: &play::UpdateMobEffect) {
        let Some(def) = self.remote_effect_def(p.effect) else {
            return;
        };
        let mut e = EffectInstance::from_def(def, p.duration, p.amplifier);
        e.ambient = p.flags & 1 != 0;
        e.visible = p.flags & 2 != 0;
        e.show_icon = p.flags & 4 != 0;
        let blend = p.flags & 8 != 0;
        let darkness_added =
            |e: &EffectInstance| super::super::sculk::darkness_blend_added(e, blend);
        if self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.entity_id == p.id)
        {
            let list = &mut self.common.potions.effects.list;
            match force_add_effect(list, e, blend, &["darkness", "nausea"]) {
                Some(e) if e.id == "darkness" => self.client.darkness_blend = darkness_added(e),
                Some(e) if e.id == "nausea" => {
                    let (o_nausea, nausea) = screen_effects::nausea_blend_added(e, blend);
                    self.client.screen_fx.o_nausea = o_nausea;
                    self.client.screen_fx.nausea = nausea;
                }
                _ => {}
            }
            return;
        }
        let id = p.id as u32;
        let (list, darkness) =
            if let Some(m) = self.common.mob_index.get_mut(&mut self.common.mobs, id) {
                (
                    &mut m.extra.client_effects.list,
                    &mut m.extra.darkness_blend,
                )
            } else if let Some(a) = self.common.armor_stands.iter_mut().find(|a| a.id == id) {
                (&mut a.client_effects.list, &mut a.darkness_blend)
            } else {
                return;
            };
        if let Some(e) = force_add_effect(list, e, blend, &["darkness"])
            && e.id == "darkness"
        {
            *darkness = darkness_added(e);
        }
    }
}
