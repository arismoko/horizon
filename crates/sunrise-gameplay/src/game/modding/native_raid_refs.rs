//! Raid handles for native server mods (`Raid`, `Raids`). Reads use the raid
//! data of the current or parked server level without moving either level.
use super::commands::Target;
use core::ops::ControlFlow;
use mod_api::{EntityRef, LevelReadError, LevelRef, MobRef, RaidReadError, RaidRef, RaidStatus};
use sunrise_sim::{
    lan::playerdata::ints_uuid,
    mobs::Mob,
    raid::{Raid, Raids, Status},
};

impl crate::game::ServerGame<'_> {
    // Diverges from upstream: handles resolve the requested server level without
    // swapping it into place, including during hooks for another level.
    fn native_level_raid_collections(
        &self,
        level: LevelRef,
    ) -> Result<(&Raids, &[Mob]), LevelReadError> {
        self.native_level_world_result(level)?;
        let (raids, mobs) = if level.dimension() == self.common.dim.key() {
            (self.raids_in_place(), self.common.mobs.as_ref())
        } else {
            let parked = self
                .server
                .levels
                .0
                .parked_of(level.dimension_id())
                .ok_or(LevelReadError::Stale)?;
            (
                parked.raiding.raids(level.dimension_id()),
                parked.mobs.as_ref(),
            )
        };
        Ok((raids.ok_or(LevelReadError::Unavailable)?, mobs))
    }
    pub fn native_level_raids_mut(&mut self, level: LevelRef) -> Option<&mut Raids> {
        self.native_level_world(level)?;
        if level.dimension() == self.common.dim.key() {
            self.server.raiding.0.raids_mut(level.dimension_id())
        } else {
            self.server
                .levels
                .0
                .parked_mut_of(level.dimension_id())?
                .raiding
                .raids_mut(level.dimension_id())
        }
    }
    fn native_raid_collections(&self, raid: RaidRef) -> Result<(&Raid, &[Mob]), RaidReadError> {
        let (raids, mobs) = self.native_level_raid_collections(raid.level())?;
        Ok((raids.get(raid.id()).ok_or(RaidReadError::Stale)?, mobs))
    }
    fn native_raid_members(
        &self,
        raid: RaidRef,
    ) -> Result<impl Iterator<Item = &Mob>, RaidReadError> {
        let (_, mobs) = self.native_raid_collections(raid)?;
        Ok(super::raids::members(mobs, raid.id()))
    }
    /// The raid a handle names now, or None for a stale or foreign one.
    fn native_raid_data(&self, raid: RaidRef) -> Option<&Raid> {
        self.native_raid_collections(raid)
            .ok()
            .map(|(raid, _)| raid)
    }

    /// `Raids.get(id)`: the raid saved under `id` in the requested level, if
    /// `Raids` still holds it (a stopped raid goes at its next tick).
    pub fn level_raid(&self, level: LevelRef, id: i32) -> Option<RaidRef> {
        let raid = RaidRef::new(level, id);
        self.native_raid_data(raid).map(|_| raid)
    }

    /// `ServerLevel.getRaidAt(pos)`: the closest active raid whose centre is
    /// less than 96 blocks away (`Raids.getNearbyRaid`, 9216 squared).
    pub fn level_raid_at(&self, level: LevelRef, pos: [i32; 3]) -> Option<RaidRef> {
        let (id, _) = self
            .native_level_raid_collections(level)
            .ok()?
            .0
            .nearby(pos)?;
        Some(RaidRef::new(level, id))
    }

    /// Every raid `Raids` holds, in map order, including stopped raids until their next tick.
    pub fn level_visit_raids(
        &self,
        level: LevelRef,
        visit: &mut dyn FnMut(RaidRef) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, LevelReadError> {
        let (raids, _) = self.native_level_raid_collections(level)?;
        Ok(raids
            .iter_ids()
            .map(|id| RaidRef::new(level, id))
            .try_for_each(visit))
    }

    /// `Raider.getCurrentRaid`: the raid a raider belongs to, even a stopped
    /// one. None for a mob that is no raider or in no raid.
    pub fn mob_raid(&self, mob: MobRef) -> Option<RaidRef> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        let raider = self.common.mobs.iter().find(|m| m.id == id && !m.removed)?;
        let id = sunrise_sim::mobs::raider::state(raider)?.raid_id?;
        let level = self.entity_level(mob.entity())?;
        self.level_raid(level, id)
    }

    /// `Raid.getCenter`.
    pub fn raid_center(&self, raid: RaidRef) -> Option<[i32; 3]> {
        Some(self.native_raid_data(raid)?.center)
    }

    /// `Raid.getRaidOmenLevel`: 0 to 5 in play, any value a mod set.
    pub fn raid_omen_level(&self, raid: RaidRef) -> Option<i32> {
        Some(self.native_raid_data(raid)?.omen_level)
    }

    /// `Raid.setRaidOmenLevel`: any value, as vanilla assigns it (only
    /// `absorbRaidOmen` clamps, to 0 to 5). The raid is saved with it.
    pub fn raid_set_omen_level(&mut self, raid: RaidRef, level: i32) -> bool {
        self.native_level_raids_mut(raid.level())
            .and_then(|raids| raids.set_omen_level(raid.id(), level))
            .is_some()
    }

    /// `Raid.getGroupsSpawned`: the waves spawned so far.
    pub fn raid_groups_spawned(&self, raid: RaidRef) -> Option<i32> {
        Some(self.native_raid_data(raid)?.groups_spawned)
    }

    /// `Raid.numGroups`, set from `getNumGroups(difficulty)` when the raid
    /// began: 0, 3, 5 or 7. A raid of omen level 2 or more adds a bonus wave
    /// (`hasBonusWave`).
    pub fn raid_num_groups(&self, raid: RaidRef) -> Option<i32> {
        Some(self.native_raid_data(raid)?.num_groups)
    }

    /// Whether the omen level is above 1, which adds a wave after the last
    /// (`Raid.hasBonusWave`).
    pub fn raid_has_bonus_wave(&self, raid: RaidRef) -> Option<bool> {
        Some(self.native_raid_data(raid)?.omen_level > 1)
    }

    /// `Raid.isStarted`.
    pub fn raid_is_started(&self, raid: RaidRef) -> bool {
        self.native_raid_data(raid).is_some_and(|r| r.started)
    }

    /// `Raid.isActive`: false once stopped.
    pub fn raid_is_active(&self, raid: RaidRef) -> bool {
        self.native_raid_data(raid).is_some_and(|r| r.active)
    }

    /// `Raid.RaidStatus`.
    pub fn raid_status(&self, raid: RaidRef) -> Option<RaidStatus> {
        Some(match self.native_raid_data(raid)?.status {
            Status::Ongoing => RaidStatus::Ongoing,
            Status::Victory => RaidStatus::Victory,
            Status::Loss => RaidStatus::Loss,
            Status::Stopped => RaidStatus::Stopped,
        })
    }

    /// `Raid.getTotalHealth`: the health of every raider that joined, which
    /// the boss bar divides by.
    pub fn raid_total_health(&self, raid: RaidRef) -> Option<f32> {
        Some(self.native_raid_data(raid)?.total_health)
    }

    /// `Raid.getHealthOfLivingRaiders`.
    pub fn raid_living_health(&self, raid: RaidRef) -> Option<f32> {
        Some(
            self.native_raid_members(raid)
                .ok()?
                .fold(0.0, |a, m| a + m.health),
        )
    }

    /// The boss bar's progress, 0 to 1 (`Raid.raidEvent.getProgress`).
    pub fn raid_bar_progress(&self, raid: RaidRef) -> Option<f32> {
        Some(self.native_raid_data(raid)?.bar.progress)
    }

    /// `Raid.getAllRaiders`: the living raiders that joined, in mob-list order.
    pub fn raid_visit_raiders(
        &self,
        raid: RaidRef,
        visit: &mut dyn FnMut(EntityRef) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, RaidReadError> {
        let epoch = raid.level().epoch();
        Ok(self
            .native_raid_members(raid)?
            .map(|m| EntityRef::new(epoch, m.id))
            .try_for_each(visit))
    }

    /// The living raiders of one wave (`groupRaiderMap.get(wave)`).
    pub fn raid_visit_wave_raiders(
        &self,
        raid: RaidRef,
        wave: i32,
        visit: &mut dyn FnMut(EntityRef) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, RaidReadError> {
        let epoch = raid.level().epoch();
        Ok(self
            .native_raid_members(raid)?
            .filter(|m| sunrise_sim::mobs::raider::state(m).is_some_and(|s| s.wave == wave))
            .map(|m| EntityRef::new(epoch, m.id))
            .try_for_each(visit))
    }

    /// `Raid.heroesOfTheVillage`: every recorded UUID in the vanilla set's order.
    pub fn raid_visit_heroes(
        &self,
        raid: RaidRef,
        visit: &mut dyn FnMut(u128) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, RaidReadError> {
        let (raid, _) = self.native_raid_collections(raid)?;
        Ok(raid.iter_heroes().map(ints_uuid).try_for_each(visit))
    }

    /// `Raid.stop`: the raid ends as `stopped` and its boss bar leaves the
    /// players; the `raid_state_change` hook fires. False if the raid is
    /// already stopped.
    pub fn raid_stop(&mut self, raid: RaidRef) -> bool {
        if self.native_raid_data(raid).is_none_or(|r| r.is_stopped()) {
            return false;
        }
        let Some(center) = self
            .native_level_raids_mut(raid.level())
            .and_then(|raids| raids.stop(raid.id()))
        else {
            return false;
        };
        self.native_raid_state_change(raid, center, "stopped");
        true
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
