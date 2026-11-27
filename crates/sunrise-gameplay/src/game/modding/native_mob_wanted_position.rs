//! Direct move-control wishes, independent of navigation and terrain availability.
use super::commands::Target;
use mod_api::{MobRef, WantedPositionError};

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn mob_set_wanted_position(
        &mut self,
        mob: MobRef,
        at: [f64; 3],
        speed: f64,
    ) -> Result<bool, WantedPositionError> {
        if at.iter().any(|v| !v.is_finite()) {
            return Err(WantedPositionError::NonFinitePosition);
        }
        if !speed.is_finite() {
            return Err(WantedPositionError::NonFiniteSpeed);
        }
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return Ok(false);
        };
        let Some(mob) = self
            .common
            .mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed)
        else {
            return Ok(false);
        };
        mob.ai.mov.set_wanted(at[0], at[1], at[2], speed);
        Ok(true)
    }

    pub(in crate::game) fn mob_wanted_position(&self, mob: MobRef) -> Option<[f64; 3]> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        Some(
            self.common
                .mobs
                .iter()
                .find(|m| m.id == id && !m.removed)?
                .ai
                .mov
                .wanted,
        )
    }

    pub(in crate::game) fn mob_speed_modifier(&self, mob: MobRef) -> Option<f64> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        Some(
            self.common
                .mobs
                .iter()
                .find(|m| m.id == id && !m.removed)?
                .ai
                .mov
                .speed_modifier,
        )
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
