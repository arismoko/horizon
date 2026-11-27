//! Mob handles for native server mods.
use super::commands::Target;
use mod_api::{EntityRef, LivingRef, MobRef};
use sunrise_core::geometry::{Aabb, Axis};
use sunrise_game_data::scoreboard::Team;
use sunrise_sim::mobs::{self, Mob, MobWorld, Species, Who, ai::brain_target};

pub(in crate::game::server_side) fn warden_mob_target_bounds(mob: &Mob) -> Option<Aabb> {
    (!mob.removed && mob.alive() && !mob.is_invulnerable() && mob.species() != Species::Warden)
        .then(|| mob.bounds())
}

/// What decides whether a player can be a mob's target.
pub(in crate::game) struct PlayerFacts {
    pub mode: i32,
    pub alive: bool,
    pub invulnerable: bool,
    pub uuid: [i32; 4],
}

impl crate::game::ServerGame<'_> {
    /// Restore a saved anger target after insertion, so the target-change hook
    /// receives a live actor and a currently resolvable living target. A
    /// target that is absent or invalid resolves to none, as EntityReference
    /// does while vanilla loads a neutral mob. Returns false only when the
    /// actor cannot yet be represented by a native handle.
    pub(in crate::game::server_side) fn native_restore_loaded_target(
        &mut self,
        id: u32,
        uuid: Option<[i32; 4]>,
    ) -> bool {
        let Some(mob) = self.entity(id).and_then(|entity| self.mob(entity)) else {
            return false;
        };
        let target = uuid.and_then(|words| {
            let uuid = sunrise_sim::lan::playerdata::ints_uuid(words);
            let entity = self
                .player(uuid)
                .map(|player| player.entity())
                .or_else(|| {
                    let target = self
                        .common
                        .persist
                        .uuids
                        .iter()
                        .find_map(|(&id, found)| (*found == words).then_some(id))?;
                    self.entity(target)
                })?;
            self.living(entity)
        });
        let target = target.filter(|target| {
            let Some(kind) = self.native_entity_target(target.entity()) else {
                return false;
            };
            self.common
                .mobs
                .iter()
                .find(|state| state.id == id)
                .is_some_and(|state| self.native_mob_valid_target(state, kind))
        });
        // Cancellation leaves the newly loaded mob's target unchanged.
        let _ = self.mob_set_target(mob, target);
        true
    }

    /// A player target's game mode, liveness, invulnerability (the ability,
    /// the `Invulnerable` flag or its window) and uuid.
    pub(in crate::game::server_side) fn native_player_facts(
        &self,
        target: Target,
    ) -> Option<PlayerFacts> {
        match target {
            Target::Guest(conn) => {
                let g = self.command_guest(conn)?;
                if g.is_fake() {
                    return None;
                }
                Some(PlayerFacts {
                    mode: g.game_mode,
                    alive: !g.dead && g.health > 0.0,
                    invulnerable: g.motion.as_ref().is_some_and(|p| {
                        p.invulnerable
                            || p.health.permanently_invulnerable
                            || p.health.invulnerable_time > 0
                    }),
                    uuid: sunrise_sim::lan::playerdata::uuid_ints(g.uuid),
                })
            }
            Target::Player => {
                if self.server.lan_acting_fake {
                    return None;
                }
                let player = self.common.player.as_ref()?;
                Some(PlayerFacts {
                    mode: self.common.mode_id(),
                    alive: !player.health.dead_or_dying(),
                    invulnerable: player.invulnerable
                        || player.health.permanently_invulnerable
                        || player.health.invulnerable_time > 0,
                    uuid: self
                        .server
                        .lan_acting_uuid
                        .map(sunrise_sim::lan::playerdata::uuid_ints)
                        .or(self.common.level.player_uuid)
                        .unwrap_or_default(),
                })
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::game) enum MobFlag {
    Aggressive,
    Persistent,
}

impl crate::game::ServerGame<'_> {
    /// `Mob.asValidTarget` and the modeled `canAttack` overrides: the game's
    /// own state as the target's facts, through the one rule set
    /// [`Mob::valid_target`] (#1175) a goal tick's view shares.
    pub fn native_mob_valid_target(&self, mob: &Mob, target: Target) -> bool {
        if mob.species() == Species::Warden {
            return self.warden_valid_target(self.mob_team_of(mob), target);
        }
        match target {
            Target::Mob(id) => {
                let Some(other) = self.common.mobs.iter().find(|m| m.id == id && !m.removed) else {
                    return false;
                };
                let seen = [sunrise_gameplay_common::game::creatures::seen(
                    other,
                    self.common.level.player_uuid,
                    self.items(),
                )];
                mob.valid_target(
                    &self.mob_world_base(&seen),
                    &mobs::goals::TargetFacts::Mob {
                        id,
                        species: other.species(),
                        alive: other.alive(),
                        invulnerable: other.invulnerable_flag,
                        baby: other.is_baby(),
                    },
                )
            }
            Target::Player | Target::Guest(_) => {
                let Some(PlayerFacts {
                    mode,
                    alive,
                    invulnerable,
                    uuid,
                }) = self.native_player_facts(target)
                else {
                    return false;
                };
                mob.valid_target(
                    &self.mob_world_base(&[]),
                    &mobs::goals::TargetFacts::Player {
                        alive,
                        invulnerable,
                        // The game modes a target cannot be attacked in:
                        // creative (1) and spectator (3).
                        creative: mode == 1,
                        spectator: mode == 3,
                        uuid,
                    },
                )
            }
            _ => false,
        }
    }
    /// `Warden.getTarget` resolves the stored entity and validates the read;
    /// the brain memory is not changed when that entity becomes invalid.
    pub(in crate::game::server_side) fn warden_attack_target(
        &self,
        mob: &Mob,
        scope: Option<(&Mob, &MobWorld<'_>)>,
    ) -> Option<(u32, Target)> {
        use sunrise_sim::join_seq::residency::table::EntityLookup;
        let id = mobs::warden::brain_target_id(mob)?;
        let own_team = scope.map_or_else(
            || self.mob_team_of(mob),
            |(actor, world)| self.mob_team_scoped(mob, actor, world),
        );
        if let Some((actor, world)) = scope {
            // The actor and disjoint mob loan are not in Game's resident list
            // during a tick. Players and the dragon remain on the level.
            let loaned = if id == actor.id {
                Some(actor)
            } else {
                world.live_mobs.and_then(|mobs| mobs.find(id))
            };
            if let Some(other) = loaned {
                let bounds = warden_mob_target_bounds(other)?;
                let team = self.mob_team_scoped(other, actor, world);
                return self
                    .warden_target_in_bounds(bounds, own_team, team)
                    .then_some((id, Target::Mob(id)));
            }
        }
        match self.server_resident_entity(id) {
            EntityLookup::Found { location, value }
                if location.owner.dimension == self.common.dim.key()
                    && self.warden_valid_target(own_team, value) =>
            {
                Some((id, value))
            }
            _ => None,
        }
    }

    /// `Warden.canTargetEntity`, which overrides the usual `Mob.canAttack`.
    pub(in crate::game::server_side) fn warden_valid_target(
        &self,
        own_team: Option<&Team>,
        target: Target,
    ) -> bool {
        let (bounds, team) = match target {
            Target::Mob(id) => {
                let Some(other) = self.common.mobs.iter().find(|m| m.id == id) else {
                    return false;
                };
                let Some(bounds) = warden_mob_target_bounds(other) else {
                    return false;
                };
                (bounds, self.mob_team_of(other))
            }
            Target::Dragon(id) => {
                let Some((_, target)) = self
                    .dragon_target()
                    .filter(|(target_id, _)| *target_id == id)
                else {
                    return false;
                };
                let (flag, window) = self.saved_invulnerability(id);
                if flag || window > 0 {
                    return false;
                }
                (target.bounds, self.mob_team(id))
            }
            Target::Guest(conn) => {
                let Some(g) = self.command_guest(conn) else {
                    return false;
                };
                if g.dimension != self.common.dim.id()
                    || g.dead
                    || g.health <= 0.0
                    || matches!(g.game_mode, 1 | 3)
                    || g.motion.as_ref().is_some_and(|p| {
                        p.health.permanently_invulnerable || p.health.invulnerable_time > 0
                    })
                {
                    return false;
                }
                (g.bounds(), self.common.score.board.players_team(&g.name))
            }
            Target::Player => {
                let Some(player) = &self.common.player else {
                    return false;
                };
                if matches!(self.mode_id(), 1 | 3)
                    || player.health.dead_or_dying()
                    || player.health.permanently_invulnerable
                    || player.health.invulnerable_time > 0
                {
                    return false;
                }
                (player.bounds(), self.player_team())
            }
            _ => return false,
        };
        self.warden_target_in_bounds(bounds, own_team, team)
    }

    pub(in crate::game::server_side) fn warden_target_in_bounds(
        &self,
        bounds: Aabb,
        own: Option<&Team>,
        other: Option<&Team>,
    ) -> bool {
        let border = self.common.border.geometry();
        !sunrise_game_data::scoreboard::teams_allied(own, other)
            && border.is_within_bounds(bounds.min(Axis::X), bounds.min(Axis::Z), 0.0)
            && border.is_within_bounds(
                bounds.max(Axis::X) - f64::from(1.0e-5_f32),
                bounds.max(Axis::Z) - f64::from(1.0e-5_f32),
                0.0,
            )
    }

    /// `Mob.getTarget`. Brain-backed mobs read their attack memory; piglins
    /// and hoglins use Sunrise's `ai.target` stand-in. Invalid targets, an
    /// unavailable mob or player binding, or a target without a living handle
    /// read as None. The dragon reads what a mod set, while it stays a valid
    /// target (`EnderDragon.canAttack`); its flight never sets one.
    pub fn mob_target(&self, mob: MobRef) -> Option<LivingRef> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return self.native_dragon_target(mob.entity());
        };
        let mob = self.common.mobs.iter().find(|m| m.id == id)?;
        if mob.species() == Species::Warden {
            let (id, _) = self.warden_attack_target(mob, None)?;
            return Some(LivingRef::new(EntityRef::new(
                self.common.native_lifecycle.epoch()?,
                id,
            )));
        }
        let target = brain_target(mob).unwrap_or(mob.ai.target)?;
        let target_id = match target {
            Who::Mob(id) => id,
            Who::Player => {
                let host = self.server.lan.as_ref()?;
                let conn = host.mob_players.get(&id)?;
                u32::try_from(host.hub.guest(*conn)?.entity_id).ok()?
            }
        };
        let entity = self.entity(target_id)?;
        self.native_mob_valid_target(mob, self.native_entity_target(entity)?)
            .then(|| self.living(entity))?
    }

    /// `Mob.setTarget`; None clears it. Only mobs and players whose binding
    /// can be stored in the current level are supported; armor stands are
    /// refused. The dragon stores a mob or a player, which its flight ignores
    /// and its save does not keep (vanilla keeps no target either). Brain-backed mobs refuse writes: vanilla's setter
    /// does not update their brain memory (#934). A fox, an enderman or a
    /// zombified piglin stores the target with its own `setTarget` side
    /// effects (`Mob::set_target`).
    /// Invalid targets are refused (false, mob unchanged); vanilla instead
    /// stores `asValidTarget(target)`, clearing the target when it is invalid.
    /// False also means a stale handle, client, or an unavailable mob/player.
    pub fn mob_set_target(&mut self, mob: MobRef, target: Option<LivingRef>) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return self.native_dragon_set_target(mob.entity(), target);
        };
        let Some(mob_state) = self.common.mobs.iter().find(|m| m.id == id) else {
            return false;
        };
        if mob_state.refuses_set_target() {
            return false;
        }
        if let Some(target) = target {
            let Some(target_kind) = self.native_entity_target(target.entity()) else {
                return false;
            };
            if !self.native_mob_valid_target(mob_state, target_kind) {
                return false;
            }
        }
        let Some(target) = self.native_mob_target_event(mob, target, false) else {
            return false;
        };
        if let Some(target) = target {
            let Some(kind) = self.native_entity_target(target.entity()) else {
                return false;
            };
            let Some(state) = self.common.mobs.iter().find(|m| m.id == id) else {
                return false;
            };
            if !self.native_mob_valid_target(state, kind) {
                return false;
            }
        }
        let Some(who) = self.native_mob_bind_target(id, target) else {
            return false;
        };
        let Some(mob) = self.common.mobs.iter_mut().find(|m| m.id == id) else {
            return false;
        };
        mob.set_target(who);
        true
    }

    /// Dispatch a mod-initiated change while the mob remains in the game.
    pub(in crate::game) fn native_mob_target_event(
        &self,
        mob: MobRef,
        original_target: Option<LivingRef>,
        is_brain: bool,
    ) -> Option<Option<LivingRef>> {
        if !mod_api::__private::listened(mod_api::HookId::LivingChangeTarget) {
            return Some(original_target);
        }
        let mut new_target = original_target;
        let mut cancel = false;
        sunrise_gameplay_common::game::native_hooks::NativeHookHost { host: self }
            .native_living_change_target(mod_api::__private::LivingChangeTargetCall {
                mob,
                original_target,
                new_target: &mut new_target,
                is_brain,
                cancel: &mut cancel,
            });
        (!cancel).then_some(new_target)
    }

    /// The `Who` mob `id` stores for `target`, for either target setter: a
    /// mob, or a player in the level in place. A player target binds to the
    /// mob (`mob_players`), so the mob's tick sees that player. None for a
    /// fake player (the root rule for every mob target setter), a player in
    /// another level, or anything else (an armor stand, the dragon, a stale
    /// handle).
    pub(in crate::game) fn native_mob_bind_target(
        &mut self,
        id: u32,
        target: Option<LivingRef>,
    ) -> Option<Option<Who>> {
        let (who, player) = match target {
            None => (None, None),
            Some(target) => match self.native_entity_target(target.entity())? {
                Target::Mob(id) => (Some(Who::Mob(id)), None),
                t @ Target::Guest(conn) if self.server.lan_acting_uuid.is_none() => {
                    self.native_player_facts(t)?;
                    let guest = self.server.lan.as_ref()?.hub.guest(conn)?;
                    if guest.dimension != self.common.dim.id() {
                        return None;
                    }
                    (Some(Who::Player), Some(conn))
                }
                t @ Target::Player => {
                    self.native_player_facts(t)?;
                    match self.server.lan_acting {
                        Some(conn) if self.server.lan.is_some() => (Some(Who::Player), Some(conn)),
                        _ => return None,
                    }
                }
                _ => return None,
            },
        };
        if let Some(conn) = player {
            self.server.lan.as_mut()?.mob_players.insert(id, conn);
        }
        Some(who)
    }

    /// `Brain.setMemory(ATTACK_TARGET, target)`, or `eraseMemory` for None:
    /// the target a brain-driven mob — an axolotl, a breeze, a creaking, a
    /// hoglin, a piglin or piglin brute, a warden or a zoglin — reads instead
    /// of `Mob.setTarget`'s (#934). A plain, non-expiring memory write: the
    /// brain's own behaviours act on it from the next tick (a target the
    /// species may not attack is dropped there, as vanilla's
    /// `StopAttackingIfTargetInvalid` drops it), and none of a species' own
    /// setter effects run — a warden's `setAttackTarget` roar, reach and
    /// sonic-boom bookkeeping stays with its own sensing. A frog refuses: its
    /// target belongs to the running tongue attack, which starts from the
    /// nearest attackable, so a written one would not be pursued.
    /// False for a fake player, a player in another level, a mob without an
    /// attack-target brain (`MobRef::set_target` writes those), a stale or
    /// unavailable mob, the dragon, or on a client.
    pub(in crate::game) fn mob_set_brain_target(
        &mut self,
        mob: MobRef,
        target: Option<LivingRef>,
    ) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        let Some(m) = self.common.mobs.iter().find(|m| m.id == id && !m.removed) else {
            return false;
        };
        if !sunrise_sim::mobs::ai::writes_brain_target(m) {
            return false;
        }
        let Some(target) = self.native_mob_target_event(mob, target, true) else {
            return false;
        };
        let Some(who) = self.native_mob_bind_target(id, target) else {
            return false;
        };
        self.common
            .mobs
            .iter_mut()
            .find(|m| m.id == id)
            .is_some_and(|m| {
                sunrise_sim::mobs::ai::set_brain_target(m, who, target.map(|t| t.entity().id()))
            })
    }

    /// The mob flag a handle names: a mob's, or the dragon's own.
    pub(in crate::game::server_side) fn native_mob_flag(
        &self,
        mob: MobRef,
        flag: MobFlag,
    ) -> Option<bool> {
        match self.native_entity_target(mob.entity())? {
            Target::Mob(id) => {
                let mob = self.common.mobs.iter().find(|m| m.id == id)?;
                Some(match flag {
                    MobFlag::Aggressive => mob.ai.aggressive,
                    MobFlag::Persistent => mob.persistent,
                })
            }
            Target::Dragon(_) => {
                let dragon = self.native_dragon(mob.entity())?;
                Some(match flag {
                    MobFlag::Aggressive => dragon.aggressive,
                    MobFlag::Persistent => dragon.persistent,
                })
            }
            _ => None,
        }
    }
    pub(in crate::game::server_side) fn native_mob_set_flag(
        &mut self,
        mob: MobRef,
        flag: MobFlag,
        value: bool,
    ) -> bool {
        let slot = match self.native_entity_target(mob.entity()) {
            Some(Target::Mob(id)) => {
                self.common
                    .mobs
                    .iter_mut()
                    .find(|m| m.id == id)
                    .map(|mob| match flag {
                        MobFlag::Aggressive => &mut mob.ai.aggressive,
                        MobFlag::Persistent => &mut mob.persistent,
                    })
            }
            Some(Target::Dragon(_)) => self.native_dragon_mut(mob.entity()).map(|d| match flag {
                MobFlag::Aggressive => &mut d.aggressive,
                MobFlag::Persistent => &mut d.persistent,
            }),
            _ => None,
        };
        slot.map(|slot| *slot = value).is_some()
    }

    /// `Mob.isAggressive`, the synced mob flag (the dragon's reaches its
    /// clients as `DATA_MOB_FLAGS_ID`).
    pub fn mob_is_aggressive(&self, mob: MobRef) -> Option<bool> {
        self.native_mob_flag(mob, MobFlag::Aggressive)
    }

    /// `Mob.setAggressive`. False for an unavailable mob.
    pub fn mob_set_aggressive(&mut self, mob: MobRef, value: bool) -> bool {
        self.native_mob_set_flag(mob, MobFlag::Aggressive, value)
    }

    /// `TamableAnimal.isInSittingPose`: the synced pose, which only a sit goal
    /// or this write changes. None for a horse, another mob kind, the dragon,
    /// a stale or removed handle, or a client.
    pub(in crate::game) fn mob_is_in_sitting_pose(&self, mob: MobRef) -> Option<bool> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        let mob = self.common.mobs.iter().find(|m| m.id == id && !m.removed)?;
        mob.is_tamable_animal().then_some(mob.extra.sitting_pose)
    }

    /// `TamableAnimal.setInSittingPose`: writes the synced pose and marks the
    /// mob for sync, leaving the saved order alone. False for a horse,
    /// another mob kind, the dragon, a stale or removed handle, or a client.
    pub(in crate::game) fn mob_set_in_sitting_pose(&mut self, mob: MobRef, sitting: bool) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        let Some(mob) = self
            .common
            .mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed)
        else {
            return false;
        };
        if !mob.is_tamable_animal() {
            return false;
        }
        mob.extra.sitting_pose = sitting;
        mob.needs_sync = true;
        true
    }

    /// `Mob.isPersistenceRequired`. The dragon never despawns
    /// (`EnderDragon.checkDespawn` is empty), but keeps and saves the flag
    /// (`PersistenceRequired`).
    pub fn mob_persistence_required(&self, mob: MobRef) -> Option<bool> {
        self.native_mob_flag(mob, MobFlag::Persistent)
    }

    /// `Mob.setPersistenceRequired`, which sets the flag and cannot clear it.
    /// False for an unavailable mob.
    pub fn mob_set_persistence_required(&mut self, mob: MobRef) -> bool {
        self.native_mob_set_flag(mob, MobFlag::Persistent, true)
    }

    /// `Mob.requiresCustomPersistence` and its species overrides: passengers,
    /// leashed mobs and species-specific reasons to stay, such as an enderman
    /// carrying a block. Independent of `PersistenceRequired` and `NoAI`.
    /// None for an unavailable mob or the separately modeled dragon.
    pub fn mob_requires_custom_persistence(&self, mob: MobRef) -> Option<bool> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        self.common
            .mobs
            .iter()
            .find(|m| m.id == id)
            .map(mobs::ai::requires_custom_persistence)
    }

    /// `Mob.isNoAi`: the mob stands where it is and its goals do not run,
    /// while what NoAI keeps running in vanilla (the daylight burn of the
    /// undead, a horse's tail) still does. Saved with it as `NoAI`. None
    /// for anything but a mob.
    pub fn mob_no_ai(&self, mob: MobRef) -> Option<bool> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        super::native_living_body::body_no_ai(self.native_living_body(Target::Mob(id))?)
    }

    /// `Mob.setNoAi`: whether the mob's own AI runs. With no AI the mob
    /// stands (what NoAI keeps in vanilla still runs), and the flag is
    /// saved with it as `NoAI`. False for anything but a mob.
    pub fn mob_set_no_ai(&mut self, mob: MobRef, no_ai: bool) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        self.common
            .mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed)
            .map(|m| m.no_ai = no_ai)
            .is_some()
    }

    /// `LivingEntity.getNoActionTime`: the ticks since the mob last had a
    /// reason to stay (a player near, a hit). Despawning and a raid's
    /// recruiting read it. None for a stale or unavailable mob, the dragon,
    /// or on a client.
    pub(in crate::game) fn mob_no_action_time(&self, mob: MobRef) -> Option<i32> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        Some(
            self.common
                .mobs
                .iter()
                .find(|m| m.id == id && !m.removed)?
                .ai
                .no_action_time,
        )
    }

    /// `LivingEntity.setNoActionTime`: the count the mob's own tick carries
    /// on from (a far-away mob's despawn chance starts past 600). Not saved,
    /// as in vanilla. False for a stale or unavailable mob, the dragon, or
    /// on a client.
    pub(in crate::game) fn mob_set_no_action_time(&mut self, mob: MobRef, ticks: i32) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        self.common
            .mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed)
            .map(|m| m.ai.no_action_time = ticks)
            .is_some()
    }

    /// `Mob.setBaby` through the species' setter: ageable mobs use their
    /// baby start age (`-48000` for sniffers, otherwise `-24000`) or `0`,
    /// including age-boundary attribute and AI setup changes. Non-ageable
    /// babies, such as zombies, keep their non-growing baby state instead.
    /// Leaves `NoAI` and `PersistenceRequired` unchanged. False, with no
    /// changes, for an unavailable mob or a species with no baby form.
    pub fn mob_set_baby(&mut self, mob: MobRef, baby: bool) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        let Some(mob) = self.common.mobs.iter_mut().find(|m| m.id == id) else {
            return false;
        };
        if !mobs::species::can_be_a_baby(mob.species()) {
            return false;
        }
        mobs::finalize::set_baby(mob, baby);
        true
    }

    /// `LookControl.setLookAt`, by the mob's `lookAt` path: the wanted
    /// position its look control rotates the head toward over the next
    /// ticks. As `move_to`'s path, the mob's own AI — a goal's look, or a
    /// brain species' `LookAtTargetSink`, which writes through this same
    /// control — may issue its own and replace this one; no species refuses.
    /// A creaking being looked at, an angry bee and an axolotl playing dead
    /// hold their look control, as in vanilla: the wanted position waits
    /// until it runs again. False for non-finite coordinates, a stale or
    /// unavailable mob, a client, and the dragon.
    pub fn mob_look_at(&mut self, mob: MobRef, at: [f64; 3]) -> bool {
        if at.iter().any(|v| !v.is_finite()) {
            return false;
        }
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        self.common
            .mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed)
            .map(|m| m.look_at(at))
            .is_some()
    }

    /// `JumpControl.jump`: the mob jumps on its next own tick by its own
    /// jump path — no velocity is written, and the control consumes the
    /// wish on that tick (a rabbit's jump control reads it too). The mob's
    /// own wishes (a slime's hop, a swimming brain's) are the same flag; no
    /// species refuses. A creaking being looked at holds its jump control,
    /// as in vanilla: the wish waits until it runs again. False for a stale
    /// or unavailable mob, a client, and the dragon.
    pub fn mob_jump(&mut self, mob: MobRef) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        self.common
            .mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed)
            .map(|m| m.ai.jump = true)
            .is_some()
    }

    /// `Mob.restrictTo(pos, radius)`: the home area the mob keeps to — its
    /// own stroll and wander goals pick their positions inside it, as a
    /// lead's and a guardian's home do. Radius `-1` clears the restriction
    /// while keeping `pos` as the center, `clear_restriction`'s shape. The
    /// area is saved with the mob (`home_pos`, `home_radius`). False for a
    /// stale or unavailable mob, a client, and the dragon.
    pub fn mob_restrict_to(&mut self, mob: MobRef, at: [i32; 3], radius: i32) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        self.common
            .mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed)
            .map(|m| m.ai.restore_home(at, radius))
            .is_some()
    }

    /// `Mob.getRestrictCenter`: the restriction's block position, kept
    /// after `clear_restriction` and after a load (`[0, 0, 0]` for a mob
    /// that never had one, vanilla's `ZERO`). None for a stale or
    /// unavailable mob, a client, and the dragon.
    pub fn mob_restrict_center(&self, mob: MobRef) -> Option<[i32; 3]> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        Some(
            self.common
                .mobs
                .iter()
                .find(|m| m.id == id && !m.removed)?
                .ai
                .home_position(),
        )
    }

    /// `Mob.getRestrictRadius`: `-1` while the mob has no restriction.
    /// None for a stale or unavailable mob, a client, and the dragon.
    pub fn mob_restrict_radius(&self, mob: MobRef) -> Option<i32> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        Some(
            self.common
                .mobs
                .iter()
                .find(|m| m.id == id && !m.removed)?
                .ai
                .home
                .map_or(-1, |(_, radius)| radius),
        )
    }

    /// `Mob.hasRestriction`. None for a stale or unavailable mob, a client,
    /// and the dragon.
    pub fn mob_has_restriction(&self, mob: MobRef) -> Option<bool> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        Some(
            self.common
                .mobs
                .iter()
                .find(|m| m.id == id && !m.removed)?
                .ai
                .home
                .is_some(),
        )
    }

    /// `Mob.clearRestriction`: the restriction goes, the center stays (its
    /// own goals read no restriction, as vanilla's `-1` radius). False for
    /// a stale or unavailable mob, a client, and the dragon.
    pub fn mob_clear_restriction(&mut self, mob: MobRef) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        self.common
            .mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed)
            .map(|m| m.ai.clear_home())
            .is_some()
    }

    /// `Mob.isWithinRestriction(pos)`: whether the block position is inside
    /// the home area; a mob without a restriction is within everywhere, as
    /// in vanilla. None for a stale or unavailable mob, a client, and the
    /// dragon.
    pub fn mob_is_within_restriction(&self, mob: MobRef, at: [i32; 3]) -> Option<bool> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        Some(
            self.common
                .mobs
                .iter()
                .find(|m| m.id == id && !m.removed)?
                .ai
                .is_within_home(at),
        )
    }

    /// `Mob.isLeftHanded`: whether the mob holds things in its left hand
    /// (the mob flags in its entity data). None for anything but a mob.
    pub fn mob_left_handed(&self, mob: MobRef) -> Option<bool> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        Some(
            self.common
                .mobs
                .iter()
                .find(|m| m.id == id && !m.removed)?
                .extra
                .left_handed,
        )
    }

    /// `Mob.setLeftHanded`: sets the mob flag the players around get with
    /// the mob's next entity data and see in its swing and held items.
    /// Saved with the mob as `LeftHanded`. False for anything but a mob.
    pub fn mob_set_left_handed(&mut self, mob: MobRef, left_handed: bool) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        self.common
            .mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed)
            .map(|m| m.extra.left_handed = left_handed)
            .is_some()
    }

    /// `Mob.canPickUpLoot`, with the overrides of an allay (holding
    /// something) and a sulfur cube (with no block). None for anything but
    /// a mob.
    pub fn mob_can_pick_up_loot(&self, mob: MobRef) -> Option<bool> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        Some(
            self.common
                .mobs
                .iter()
                .find(|m| m.id == id && !m.removed)?
                .can_pick_up_loot(),
        )
    }

    /// `Mob.setCanPickUpLoot`: whether the mob picks up the items it sees
    /// (saved with it as `CanPickUpLoot`; clients are not told, as in
    /// vanilla). An allay and a sulfur cube decide for themselves. False
    /// for anything but a mob.
    pub fn mob_set_can_pick_up_loot(&mut self, mob: MobRef, can_pick_up: bool) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        self.common
            .mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed)
            .map(|m| m.extra.pick_up_loot = can_pick_up)
            .is_some()
    }
}

use sunrise_gameplay_common::game::creatures::world_view::WorldViewExt as _;

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt6 as _;

use sunrise_gameplay_common::game::native_living_body::NativeLivingBodyGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;

use sunrise_gameplay_common::game::scoreboard_common::CommonGameExt1 as _;
