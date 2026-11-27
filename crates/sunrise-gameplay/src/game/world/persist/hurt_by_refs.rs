//! `LivingEntity.lastHurtByMob` is an `EntityReference`: a UUID that stands for
//! the entity once it is in the level. What a mob saves of it, how a value read
//! back resolves, and the same for the player in `lastHurtByPlayer`.
use crate::game::ServerGame;
use sunrise_core::nbt::{JavaString, Tag};
use sunrise_sim::mobs::{Mob, Who};

impl ServerGame<'_> {
    /// `LivingEntity.addAdditionalSaveData`'s `last_hurt_by_mob` and
    /// `ticks_since_last_hurt_by_mob`, a PARTIAL projection at save time: both
    /// from `ai.last_hurt_by` while the attacker is a living mob with a
    /// registered UUID, in the level or in another loaded one it travelled to
    /// since, and at most 100 ticks have passed (`baseTick`
    /// drops the record after that, or once the attacker is dead). Otherwise
    /// both keys go, a copy read from loaded data included. A player attacker
    /// is not written. A reference read back that names nothing in the level
    /// yet (`ai.last_hurt_by_ref`) is written as it is. This is not
    /// `baseTick`'s cleanup itself, the AI's record, or `#no_anger`.
    pub fn put_last_hurt_by_mob(&self, m: &Mob, tag: &mut Tag) {
        let Tag::Compound(c) = tag else { return };
        // A reference read back and not resolved yet (its attacker is not in
        // the level) is kept as it is, for ever: `baseTick` cannot resolve it,
        // so it does not end it.
        if let Some(uuid) = m.ai.last_hurt_by_ref {
            c.insert("last_hurt_by_mob".into(), Tag::IntArray(uuid.to_vec()));
            c.insert(
                "ticks_since_last_hurt_by_mob".into(),
                Tag::Int((m.age as i32).wrapping_sub(m.ai.last_hurt_by_time)),
            );
            return;
        }
        let since = (m.age as i32).wrapping_sub(m.ai.last_hurt_by_time);
        let reference = match m.ai.last_hurt_by {
            Some(Who::Mob(attacker)) if since <= 100 => self.credited_mob_uuid(attacker),
            _ => None,
        };
        match reference {
            Some(uuid) => {
                c.insert("last_hurt_by_mob".into(), Tag::IntArray(uuid.to_vec()));
                c.insert("ticks_since_last_hurt_by_mob".into(), Tag::Int(since));
            }
            None => {
                c.remove(&JavaString::from("last_hurt_by_mob"));
                c.remove(&JavaString::from("ticks_since_last_hurt_by_mob"));
            }
        }
    }
    /// The UUID of the living mob a kill credit recorded as `attacker` names
    /// now: that mob in the level, or in another loaded level, where the
    /// dimension changes since took it (`Levels::travelled`).
    fn credited_mob_uuid(&self, attacker: u32) -> Option<[i32; 4]> {
        let mut id = attacker;
        loop {
            if self.common.mobs.iter().any(|a| a.id == id && a.alive()) {
                return self.common.persist.uuid_of(id);
            }
            if let Some((_, mob)) = self.mob_in_other_level(id) {
                if !mob.alive() {
                    return None;
                }
                return self
                    .common
                    .resident_entities
                    .table
                    .borrow_mut()
                    .resident_uuid(id);
            }
            id = self.server.levels.0.travelled(id)?;
        }
    }
    /// `EntityReference.getEntity` for `last_hurt_by_mob` read back from a save:
    /// a mob whose reference names an entity now in the level (a mob, or the
    /// player), or a mob in another loaded level (`getEntityInAnyDimension`),
    /// takes it as `last_hurt_by`, and `baseTick` ends it as it does any other.
    /// One naming nothing loaded stays a bare UUID, still saved. A reference to
    /// the mob itself resolves like any other (`getEntity` looks up the UUID
    /// and does not exclude the holder).
    pub fn resolve_last_hurt_by_refs(&mut self) {
        let resolved: Vec<(usize, Who)> = self
            .common
            .mobs
            .iter()
            .enumerate()
            .filter_map(|(i, m)| {
                let uuid = m.ai.last_hurt_by_ref?;
                let who = self
                    .who_by_uuid(uuid)
                    .or_else(|| self.mob_by_uuid_elsewhere(uuid).map(Who::Mob))?;
                Some((i, who))
            })
            .collect();
        for (i, who) in resolved {
            let ai = &mut self.common.mobs[i].ai;
            ai.last_hurt_by = Some(who);
            ai.last_hurt_by_ref = None;
        }
    }
    /// The mob resident under `uuid` in a loaded level other than the one in
    /// place, through each level's UUID index.
    fn mob_by_uuid_elsewhere(&self, uuid: [i32; 4]) -> Option<u32> {
        for dim in self.server.levels.0.parked.keys() {
            let mut after = None;
            loop {
                let next = self
                    .common
                    .resident_entities
                    .table
                    .borrow_mut()
                    .level_uuid_candidate(dim.key(), uuid, after);
                let Some(candidate) = next else { break };
                if self.mob_in_other_level(candidate.get()).is_some() {
                    return Some(candidate.get());
                }
                after = Some(candidate);
            }
        }
        None
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
