//! A mob whose AI step a mod paused while its physics runs on, for
//! `MobRef::set_ai_paused` (#4478). Vanilla's `NoAI` skips `travel` with the
//! AI step, so a `NoAI` mob neither falls nor carries a push; this is
//! vanilla's `isImmobile` instead (a dying mob, a stunned ravager): the AI
//! step is skipped and the inputs are zeroed while `travel` runs on.
//!
//! The flag (`Ai::ai_paused`) is read once, at the AI step (`mobs::ai_pause`).
//! It is saved with the mob as an attachment-style value
//! (`entity_nbt::write_ai_paused`), never as `NoAI`, and read back on load.
//!
//! Server authority: each case the pause cannot apply to is told apart here,
//! where the side, the epoch and the held-out list are known.
use super::commands::Target;
use mod_api::{EntityRef, MobAiPauseRefusal, MobRef};

impl crate::game::ServerGame<'_> {
    /// Whether a mod paused the mob's AI step. None for a stale or held-out
    /// handle, anything but a mob, the dragon, or on a client.
    pub(in crate::game) fn mob_ai_paused(&self, mob: MobRef) -> Option<bool> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        let m = self.common.mobs.iter().find(|m| m.id == id && !m.removed)?;
        Some(m.ai.ai_paused)
    }

    /// Pauses or resumes the mob's AI step while `aiStep`'s physics runs on.
    /// Refused, with nothing changed, as `MobAiPauseRefusal` names.
    pub(in crate::game) fn mob_set_ai_paused(
        &mut self,
        mob: MobRef,
        paused: bool,
    ) -> Result<(), MobAiPauseRefusal> {
        let id = self.ai_pause_target(mob.entity())?;
        let m = self
            .common
            .mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed)
            .ok_or(MobAiPauseRefusal::Stale)?;
        m.ai.ai_paused = paused;
        if paused {
            // LivingEntity.aiStep's isImmobile branch: no jump or input is left
            // over. The move target (`mov.wanted`) is kept for the resume.
            m.ai.jumping = false;
            m.ai.xxa = 0.0;
            m.ai.yya = 0.0;
            m.ai.zza = 0.0;
        }
        Ok(())
    }

    /// The mob a pause names, or which of the named refusals it is.
    fn ai_pause_target(&self, entity: EntityRef) -> Result<u32, MobAiPauseRefusal> {
        if !self.common.is_logical_server() {
            return Err(MobAiPauseRefusal::Client);
        }
        if self.common.native_lifecycle.epoch() != Some(entity.epoch()) {
            return Err(MobAiPauseRefusal::Stale);
        }
        if self.server.held_out.contains(&entity.id()) {
            return Err(MobAiPauseRefusal::HeldOut);
        }
        match self.native_entity_target(entity) {
            Some(Target::Mob(id)) => Ok(id),
            Some(Target::Dragon(_) | Target::DragonPart(_)) => Err(MobAiPauseRefusal::NoAiStep),
            Some(_) => Err(MobAiPauseRefusal::NotAMob),
            None if self.level_dragon_part(entity.id()).is_some() => {
                Err(MobAiPauseRefusal::NoAiStep)
            }
            None => Err(MobAiPauseRefusal::Stale),
        }
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
