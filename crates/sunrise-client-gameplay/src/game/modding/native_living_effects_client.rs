use super::commands_common::Target;
use crate::game::ClientGame;
use mod_api::{EffectKey, LivingRef};

impl ClientGame<'_> {
    /// `LivingEntity.getEffectBlendFactor` for the local player on a client.
    pub(in crate::game) fn living_effect_blend_factor(
        &self,
        living: LivingRef,
        effect: EffectKey,
        partial: f32,
    ) -> Option<f32> {
        let def = mod_api::__private::effect_index(effect)
            .and_then(sunrise_game_data::potions::effect_by_index)?;
        let Target::Player = self.native_client_entity_target(living.entity())? else {
            return None;
        };
        let Some(e) = self.common.potions.effects.get_registered(def.index) else {
            return Some(0.0);
        };
        Some(match def.id {
            "darkness" => self.darkness_blend(partial),
            "nausea" => self.nausea_blend(partial),
            _ => {
                let (previous, current) = e.client_blend;
                let previous = u8::from(previous) as f32;
                let current = u8::from(current) as f32;
                previous + partial * (current - previous)
            }
        })
    }
}
