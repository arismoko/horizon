//! Ender dragon handles for native server mods: the `Mob` state the dragon
//! keeps, its phase and the fight it belongs to, and the End's fight
//! itself, read and driven through the level.
use super::{Dim, commands::Target, dragon::Fight, dragon_common::Dragon};
use mod_api::{DragonFightState, DragonPhase, DragonRef, EntityRef, LevelRef, LivingRef, MobRef};
use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;

impl crate::game::ServerGame<'_> {
    /// The dragon an entity handle names now.
    pub(in crate::game::server_side) fn native_dragon(&self, entity: EntityRef) -> Option<&Dragon> {
        let Target::Dragon(id) = self.native_entity_target(entity)? else {
            return None;
        };
        self.common.dragon.as_ref().filter(|d| d.id == id)
    }
    pub(in crate::game::server_side) fn native_dragon_mut(
        &mut self,
        entity: EntityRef,
    ) -> Option<&mut Dragon> {
        let Target::Dragon(id) = self.native_entity_target(entity)? else {
            return None;
        };
        self.common.dragon.as_mut().filter(|d| d.id == id)
    }

    /// The entity as the ender dragon, or None for another kind, a stale
    /// handle or a client.
    pub fn dragon(&self, entity: EntityRef) -> Option<DragonRef> {
        self.native_dragon(entity)?;
        Some(DragonRef::new(entity))
    }

    /// `EnderDragonPhaseManager.getCurrentPhase`.
    pub(in crate::game) fn dragon_phase(&self, dragon: DragonRef) -> Option<DragonPhase> {
        Some(self.native_dragon(dragon.entity())?.native_phase())
    }

    /// `EnderDragonPhaseManager.setPhase`: the dragon ends its phase and
    /// begins the new one fresh (targets and counters cleared), and the
    /// clients get the synced `DATA_PHASE`. Setting the phase it holds does
    /// nothing, as in vanilla (a strafing dragon keeps its player). No
    /// random draw. False for a stale handle or a client. The new phase
    /// runs from the dragon's next tick.
    pub(in crate::game) fn dragon_set_phase(
        &mut self,
        dragon: DragonRef,
        phase: DragonPhase,
    ) -> bool {
        let Some(d) = self.native_dragon_mut(dragon.entity()) else {
            return false;
        };
        d.set_native_phase(phase);
        self.end_dragon_flame();
        true
    }

    /// The dragon's fight: Sunrise keeps the End's fight only, so it is the
    /// fight of a dragon in the level in place when that is the End.
    fn native_dragon_fight(&self, dragon: DragonRef) -> Option<&Fight> {
        self.native_dragon(dragon.entity())?;
        (self.common.dim == Dim::End && self.server.fight.loaded).then_some(&self.server.fight)
    }

    /// `EnderDragonFight.aliveCrystals`: the crystals the last scan found
    /// (every 100 ticks). None for a stale handle, a client, or a dragon
    /// outside the End or whose fight is not loaded.
    pub(in crate::game) fn dragon_alive_crystals(&self, dragon: DragonRef) -> Option<i32> {
        Some(self.native_dragon_fight(dragon)?.alive_crystals)
    }

    /// `EnderDragonFight.hasPreviouslyKilledDragon`. None as
    /// `alive_crystals` is.
    pub(in crate::game) fn dragon_previously_killed(&self, dragon: DragonRef) -> Option<bool> {
        Some(self.native_dragon_fight(dragon)?.previously_killed)
    }

    /// `EnderDragonFight`'s state, read in place or parked: Sunrise keeps the
    /// End's fight only. None as the host call says.
    pub(in crate::game) fn level_dragon_fight(&self, level: LevelRef) -> Option<DragonFightState> {
        self.native_level_world(level)?;
        (level.dimension_id() == Dim::End.id() && self.server.fight.loaded)
            .then(|| self.server.fight.api())
    }

    /// Whether a fight write may run on `level`: the End in place, on the
    /// server, named by a current handle.
    fn native_fight_write_allowed(&self, level: LevelRef) -> bool {
        self.common.is_logical_server()
            && self.level_handle_current(level)
            && self.common.dim == Dim::End
            && self.common.dim.key() == level.dimension()
    }

    /// `EnderDragonFight.tryRespawn`. True when a respawn started.
    pub(in crate::game) fn level_dragon_fight_try_respawn(&mut self, level: LevelRef) -> bool {
        self.native_fight_write_allowed(level) && self.try_respawn()
    }

    /// `EnderDragonFight.resetSpikeCrystals`.
    pub(in crate::game) fn level_dragon_fight_reset_spike_crystals(
        &mut self,
        level: LevelRef,
    ) -> bool {
        if !self.native_fight_write_allowed(level) {
            return false;
        }
        self.load_fight();
        self.reset_spike_crystals();
        true
    }

    /// `EnderDragon.canAttack` (`canBeSeenAsEnemy`) and `Mob.asValidTarget`
    /// for the targets Sunrise models: mobs and players. Armor stands, end
    /// crystals and the like are not supported.
    fn native_dragon_valid_target(&self, target: Target) -> bool {
        match target {
            Target::Mob(id) => self
                .common
                .mobs
                .iter()
                .any(|m| m.id == id && !m.removed && m.alive() && !m.invulnerable_flag),
            Target::Player | Target::Guest(_) => self
                .native_player_facts(target)
                .is_some_and(|p| p.alive && !p.invulnerable && !matches!(p.mode, 1 | 3)),
            _ => false,
        }
    }

    /// `Mob.getTarget` for the dragon: the mob or player a mod set, None once
    /// it is no longer a valid target. The dragon's flight never sets or
    /// reads it.
    pub(in crate::game::server_side) fn native_dragon_target(
        &self,
        entity: EntityRef,
    ) -> Option<LivingRef> {
        let target = self.entity(self.native_dragon(entity)?.target?)?;
        self.native_dragon_valid_target(self.native_entity_target(target)?)
            .then(|| self.living(target))?
    }

    /// `Mob.setTarget` for the dragon; None clears it. An invalid target is
    /// refused (false, unchanged) where vanilla stores None. False too for a
    /// stale handle or a target that is not a mob or a player.
    pub(in crate::game::server_side) fn native_dragon_set_target(
        &mut self,
        entity: EntityRef,
        target: Option<LivingRef>,
    ) -> bool {
        if self.native_dragon(entity).is_none() {
            return false;
        }
        if let Some(target) = target
            && !self
                .native_entity_target(target.entity())
                .is_some_and(|t| self.native_dragon_valid_target(t))
        {
            return false;
        }
        let Some(target) = self.native_mob_target_event(MobRef::new(entity), target, false) else {
            return false;
        };
        let id = match target {
            None => None,
            Some(target) => {
                let valid = self
                    .native_entity_target(target.entity())
                    .is_some_and(|t| self.native_dragon_valid_target(t));
                if !valid {
                    return false;
                }
                Some(target.entity().id())
            }
        };
        let Some(dragon) = self.native_dragon_mut(entity) else {
            return false;
        };
        dragon.target = id;
        true
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
