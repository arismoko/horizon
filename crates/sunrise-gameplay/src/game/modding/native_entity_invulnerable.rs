//! An entity's invulnerability for native server mods, through an
//! `EntityRef` (docs/modding/src/how-to/native-mods.md, "Entity and level handles"): the
//! `Invulnerable` flag (`Entity.permanentlyInvulnerable`) and
//! `LivingEntity.damageCooldownTime`, the ticks after a hit in which a
//! weaker one does only the difference. Sunrise has no counterpart yet of
//! 26.3's own `Entity.invulnerableTime` timer, so neither is offered. Reads
//! return None and writes false for a stale handle, on a client, and for a
//! kind Sunrise keeps no such state for.
use super::commands::Target;
use mod_api::EntityRef;
use sunrise_sim::mobs::Damage;

impl crate::game::ServerGame<'_> {
    /// Whether a named causing entity is a creative player.
    pub(in crate::game::server_side) fn creative_damage_source_id(&self, id: i32) -> bool {
        id != 0
            && ((id == self.own_entity_id() && self.common.creative)
                || self.server.lan.as_ref().is_some_and(|host| {
                    host.hub
                        .guests
                        .iter()
                        .any(|g| g.entity_id == id && g.creative())
                }))
    }
    /// Ask listeners about a nonliving entity damage source.
    pub(in crate::game::server_side) fn native_nonliving_invulnerable(
        &mut self,
        id: u32,
        kind: &str,
        source: sunrise_sim::health::HurtSource<'_>,
        vanilla: bool,
    ) -> bool {
        crate::game::server_side::entity_server::damage_level_of!(
            self,
            self.server.combat.0.attack(),
            no_push,
            None
        )
        .invulnerability_check(
            sunrise_sim::health::Hurting::Entity(id as i32, kind, source),
            vanilla,
        )
    }
    /// `Entity.isInvulnerableToBase` for lava's `lavaHurt` of a non-living
    /// entity: `vanilla` (its saved invulnerability, which lava does not
    /// bypass), as the mods' check answers it.
    pub(in crate::game) fn refuses_lava_hurt(
        &mut self,
        id: u32,
        kind: &str,
        vanilla: bool,
    ) -> bool {
        let source = sunrise_sim::health::HurtSource {
            damage_type: "minecraft:lava",
            causing: 0,
            direct: 0,
            position: None,
            direct_type: None,
        };
        self.native_nonliving_invulnerable(id, kind, source, vanilla)
    }
    /// A player's body, where it is kept now.
    pub(in crate::game) fn native_player_body(
        &self,
        t: Target,
    ) -> Option<&sunrise_sim::motion::PlayerMotion> {
        match t {
            Target::Player => self.common.player.as_ref(),
            Target::Guest(conn) => self.server.lan.as_ref()?.hub.guest(conn)?.motion.as_deref(),
            _ => None,
        }
    }
    pub(in crate::game) fn native_player_body_mut(
        &mut self,
        t: Target,
    ) -> Option<&mut sunrise_sim::motion::PlayerMotion> {
        match t {
            Target::Player => self.common.player.as_mut(),
            Target::Guest(conn) => self.server.lan.as_mut()?.hub.guests.motion_mut(conn),
            _ => None,
        }
    }

    /// `Entity.isPermanentlyInvulnerable` reads the flag from the entity's base state.
    pub fn entity_permanently_invulnerable(&self, entity: EntityRef) -> Option<bool> {
        let target = self.native_entity_target(entity)?;
        Some(self.read_entity_base(target)?.base.invulnerable)
    }
    /// `Entity.setInvulnerable` sets the flag on the entity's base state.
    pub fn entity_set_invulnerable(&mut self, entity: EntityRef, invulnerable: bool) -> bool {
        self.native_entity_target(entity).is_some_and(|target| {
            self.update_entity_base(target, |b| b.invulnerable = invulnerable)
        })
    }

    fn timer_read_target(&self, entity: EntityRef) -> Option<Target> {
        self.native_entity_read_target(entity)
    }

    /// `Entity.invulnerableTime` (#916): the entity-level invulnerability
    /// window, its own field on a mob and a player — never the living damage
    /// cooldown — and the saved `invulnerable_time` tag for the other kinds.
    /// Counted down by the base tick; `isTemporarilyInvulnerable` is it above
    /// zero. None for a stale handle or a kind that keeps none.
    pub fn entity_invulnerable_time(&self, entity: EntityRef) -> Option<i32> {
        match self.timer_read_target(entity)? {
            Target::Mob(id) => Some(
                self.common
                    .mobs
                    .iter()
                    .find(|m| m.id == id)?
                    .invulnerable_time,
            ),
            t @ (Target::Player | Target::Guest(_)) => {
                Some(self.native_player_body(t)?.health.invulnerable_time)
            }
            t => self.saved_invulnerable_time(t),
        }
    }
    /// `Entity.setInvulnerableTime` (#916): the write side of
    /// [`native_entity_invulnerable_time`](Self::native_entity_invulnerable_time).
    /// False where the entity is gone or keeps no such field.
    pub fn entity_set_invulnerable_time(&mut self, entity: EntityRef, ticks: i32) -> bool {
        let ticks = ticks.max(0);
        match self.native_entity_target(entity) {
            Some(Target::Mob(id)) => match self.common.mobs.iter_mut().find(|m| m.id == id) {
                Some(m) => {
                    m.invulnerable_time = ticks;
                    true
                }
                None => false,
            },
            Some(t @ (Target::Player | Target::Guest(_))) => {
                self.native_player_body_mut(t).is_some_and(|p| {
                    p.health.invulnerable_time = ticks;
                    true
                })
            }
            Some(t) => self.set_saved_invulnerable_time(t, ticks),
            None => false,
        }
    }

    /// A non-mob kind's saved invulnerability (#916), the one parse of its
    /// entity data: the `Invulnerable` flag and the `invulnerable_time`
    /// window. `(false, 0)` where its data has neither.
    pub(in crate::game::server_side) fn saved_invulnerability(&self, id: u32) -> (bool, i32) {
        use sunrise_core::nbt::Tag;
        match self.common.persist.raw.get(&id) {
            Some(Tag::Compound(map)) => (
                matches!(map.get(&"Invulnerable".into()), Some(Tag::Byte(1))),
                match map.get(&"invulnerable_time".into()) {
                    Some(Tag::Int(ticks)) => *ticks,
                    _ => 0,
                },
            ),
            _ => (false, 0),
        }
    }
    /// The saved `invulnerable_time` tag of a non-mob entity's data (#916):
    /// 0 where its entity data has none. None for a kind that keeps none.
    fn saved_invulnerable_time(&self, t: Target) -> Option<i32> {
        let id = match t {
            Target::Stand(id) | Target::Dragon(id) | Target::Item(id) => id,
            _ => return None,
        };
        Some(self.saved_invulnerability(id).1)
    }
    /// `Entity.isInvulnerableToBase` over a non-mob kind's saved flag and
    /// window (#916) — the one place that refusal is decided: past only for
    /// `#bypasses_invulnerability` damage and a creative player's hit
    /// (`isCreativePlayer`). False where the saved state has neither.
    pub(in crate::game) fn saved_refuses_damage(
        &self,
        id: u32,
        damage_type: &str,
        creative_source: bool,
    ) -> bool {
        let (flag, window) = self.saved_invulnerability(id);
        if !flag && window <= 0 {
            return false;
        }
        let registry = &self.common.health_resources.damage;
        let bypass = registry
            .id(damage_type)
            .is_some_and(|kind| registry.in_tag(kind, "minecraft:bypasses_invulnerability"));
        !(bypass || creative_source)
    }
    /// `Entity.isInvulnerableToBase` for resident items and orbs. These bodies
    /// have no intrinsic fire immunity; item-stack resistance is checked separately.
    pub(in crate::game::server_side) fn resident_item_or_orb_immune(
        &self,
        id: u32,
        entity_type: &str,
        source: sunrise_world::damage_source::DamageTypeId,
        creative_source: bool,
    ) -> bool {
        let (flag, window) = self.saved_invulnerability(id);
        sunrise_sim::health::BodyImmunity {
            removed: false,
            invulnerable: flag || window > 0,
            creative_source,
            fire_immune: false,
            fall_immune: world_items!(self).entity_type_tags.has_flag(
                entity_type,
                sunrise_items::items::EntityTypeTags::FALL_DAMAGE_IMMUNE,
            ),
            source_has_tag: |tag| {
                self.common
                    .health_resources
                    .damage
                    .immunity_tag(source, tag)
            },
        }
        .vanilla()
    }

    /// `DamageSource.isCreativePlayer` (#916): the causing player's creative
    /// mode, the host's own or a guest's; false for anything else. The one
    /// decision every invulnerability gate shares with `/damage`'s and the
    /// dragon's.
    pub(in crate::game::server_side) fn creative_source_of(&self, cause: Option<Target>) -> bool {
        match cause {
            Some(Target::Player) => self.common.creative,
            Some(Target::Guest(conn)) => self.command_guest(conn).is_some_and(|g| g.creative()),
            _ => false,
        }
    }
    /// Writes the `invulnerable_time` tag into a non-mob entity's saved data
    /// (#916).
    fn set_saved_invulnerable_time(&mut self, t: Target, ticks: i32) -> bool {
        use sunrise_core::nbt::Tag;
        let id = match t {
            Target::Stand(id) | Target::Dragon(id) | Target::Item(id) => id,
            _ => return false,
        };
        let mut compound = match self.common.persist.raw.get(&id) {
            Some(Tag::Compound(map)) => map.clone(),
            _ => sunrise_core::nbt::Compound::new(),
        };
        compound.insert("invulnerable_time".into(), Tag::Int(ticks.max(0)));
        self.common.persist.raw.insert(id, Tag::Compound(compound));
        true
    }
    /// `Entity.baseTick`'s countdown of a non-mob kind's saved
    /// `invulnerable_time` (#916): one less each tick while it runs, so the
    /// window an entity carries in its saved data closes like a mob's and a
    /// player's do.
    pub fn tick_saved_invulnerable_time(&mut self, id: u32) {
        use sunrise_core::nbt::Tag;
        let ticks = self.saved_invulnerability(id).1;
        if ticks <= 0 {
            return;
        }
        let ticks = ticks - 1;
        let mut compound = match self.common.persist.raw.get(&id) {
            Some(Tag::Compound(map)) => map.clone(),
            _ => sunrise_core::nbt::Compound::new(),
        };
        compound.insert("invulnerable_time".into(), Tag::Int(ticks));
        self.common.persist.raw.insert(id, Tag::Compound(compound));
    }

    /// `Entity.hurtServer` of `amount` from a source of `damage_type`
    /// (`minecraft:generic`'s shape), `direct` the entity that dealt it (a
    /// shot arrow) and `causing` the one responsible for it (its shooter):
    /// the engine's own hurt paths take it — the damage type, the attacker,
    /// armor, the cooldown and every hurt hook kept, the damage event the
    /// hit already broadcasts — by kind: the player's and mobs' for the
    /// living, the item, dragon and crystal branches for the damageable
    /// rest. A return value of `true` means the hurt was accepted, not
    /// necessarily that the entity lost health. `None` means the target handle
    /// is stale; `false` means the hurt was refused, including for a kind that
    /// cannot be hurt or an unregistered damage type. Stale direct/causing
    /// handles are treated as absent sources.
    pub(in crate::game) fn entity_hurt(
        &mut self,
        entity: EntityRef,
        damage_type: &str,
        amount: f32,
        direct: Option<EntityRef>,
        causing: Option<EntityRef>,
    ) -> Option<bool> {
        let target = self.native_entity_target(entity)?;
        let direct = direct.and_then(|e| self.native_entity_target(e));
        let causing = causing.and_then(|e| self.native_entity_target(e));
        // `DamageSource(type, cause, at)`: the source position the knockback
        // pushes away from is the direct entity's.
        let from = direct.and_then(|t| self.target_position(t));
        Some(self.hurt_target(
            target,
            amount,
            damage_type,
            direct,
            causing.or(direct),
            from,
        ))
    }

    /// `Entity.isInvulnerableTo` for a source of `damage_type`: #842's own
    /// decision, the guard every mob damage hook runs after — dead or
    /// removed, fire immunity, or the `Invulnerable` flag nothing but
    /// `#bypasses_invulnerability` gets past. None for a stale handle or a
    /// kind with no such decision (the player's paths answer their refusals
    /// at hurt time).
    pub(in crate::game) fn entity_invulnerable_to_damage(
        &self,
        entity: EntityRef,
        damage_type: &str,
    ) -> Option<bool> {
        let Target::Mob(id) = self.native_entity_target(entity)? else {
            return None;
        };
        let m = self.common.mobs.iter().find(|m| m.id == id)?;
        Some(self.mob_query_invulnerable(m, damage_type))
    }

    /// `Entity.isInvulnerableTo` asked of `mob` outside a hit, for a source of `damage_type`:
    /// the vanilla answer, which `entity_invulnerability_check` listeners may change.
    pub fn mob_query_invulnerable(&self, mob: &sunrise_sim::mobs::Mob, damage_type: &str) -> bool {
        let source = Damage::of(Self::damage_kind(
            &self.common.health_resources.damage,
            damage_type,
        ));
        let vanilla = mob.refuses_damage(&source);
        sunrise_sim::health::query_invulnerability(
            &self.common.world,
            self.common.pack.assets.registry,
            Some(world_items!(self)),
            Some(&self.common.pack.block_registries),
            &self.common.damage_hooks,
            sunrise_sim::health::Hurting::Mob(mob, &source),
            vanilla,
        )
    }

    /// `isInvulnerableToBase` of a non-living entity the acting player strikes in melee
    /// (`player_attack`), asked of listeners.
    pub(in crate::game::server_side) fn native_player_hit_invulnerable(
        &mut self,
        id: u32,
        kind: &str,
    ) -> bool {
        let attacker = self.own_entity_id();
        let source = sunrise_sim::health::HurtSource {
            damage_type: "minecraft:player_attack",
            causing: attacker,
            direct: attacker,
            position: None,
            direct_type: None,
        };
        let vanilla =
            self.saved_refuses_damage(id, "minecraft:player_attack", self.common.creative);
        self.native_nonliving_invulnerable(id, kind, source, vanilla)
    }

    /// Whether `source`'s damage may hit `entity` through their teams
    /// (`Player.canHarmPlayer`, `Team.allowFriendlyFire`,
    /// `DamageSource.doTeamsAllowDamage`): allies without friendly fire
    /// cannot. Entities without a team always can. None for a stale handle.
    pub(in crate::game) fn entity_entities_team_damage_allowed(
        &self,
        entity: EntityRef,
        source: EntityRef,
    ) -> Option<bool> {
        let own = self.native_target_team(self.native_entity_target(entity)?)?;
        let their = self.native_target_team(self.native_entity_target(source)?)?;
        Some(sunrise_game_data::scoreboard::teams_allow_damage(
            own, their,
        ))
    }

    /// The team an entity is on: a mob's own or its tamed owner's, the
    /// acting player's, a guest's by name.
    pub(in crate::game::server_side) fn native_target_team(
        &self,
        t: Target,
    ) -> Option<Option<&sunrise_game_data::scoreboard::Team>> {
        Some(match t {
            Target::Player => self.player_team(),
            Target::Guest(conn) => self
                .common
                .score
                .board
                .players_team(&self.command_guest(conn)?.name),
            Target::Mob(id) => self.mob_team(id),
            _ => None,
        })
    }

    /// `Entity.isInvulnerableToPiercingWeapon` (`isInvulnerable`): the
    /// `Entity.isInvulnerable()`: the `Invulnerable` flag or the window
    /// `Entity.invulnerableTime` keeps open — a mob's own fields, a player's
    /// health, the saved tags for the other kinds (#916). None for a stale
    /// handle.
    pub(in crate::game) fn entity_invulnerable_to_piercing_weapon(
        &self,
        entity: EntityRef,
    ) -> Option<bool> {
        match self.native_entity_target(entity)? {
            Target::Mob(id) => Some(
                self.common
                    .mobs
                    .iter()
                    .find(|m| m.id == id)?
                    .is_invulnerable(),
            ),
            t @ (Target::Player | Target::Guest(_)) => {
                let p = self.native_player_body(t)?;
                Some(p.health.permanently_invulnerable || p.health.invulnerable_time > 0)
            }
            t => {
                let id = match t {
                    Target::Stand(id) | Target::Dragon(id) | Target::Item(id) => id,
                    _ => return None,
                };
                let (flag, window) = self.saved_invulnerability(id);
                Some(flag || window > 0)
            }
        }
    }

    /// `LivingEntity.damageCooldownTime`: 20 after a hit, counting down;
    /// while it is above 10 a hit does only what it deals beyond the last
    /// one. Mobs and players.
    pub fn entity_damage_cooldown_time(&self, entity: EntityRef) -> Option<i32> {
        match self.timer_read_target(entity)? {
            Target::Mob(id) => Some(self.common.mobs.iter().find(|m| m.id == id)?.invulnerable),
            t => Some(self.native_player_body(t)?.health.hurt.cooldown),
        }
    }
    /// Sets `LivingEntity.damageCooldownTime`, as mods do to let rapid hits
    /// through (0). Mobs and players.
    pub fn entity_set_damage_cooldown_time(&mut self, entity: EntityRef, ticks: i32) -> bool {
        let Some(t) = self.native_entity_target(entity) else {
            return false;
        };
        match t {
            Target::Mob(id) => match self.common.mobs.iter_mut().find(|m| m.id == id) {
                Some(m) => {
                    m.invulnerable = ticks;
                    true
                }
                None => false,
            },
            t => match self.native_player_body_mut(t) {
                Some(p) => {
                    p.health.hurt.cooldown = ticks;
                    true
                }
                None => false,
            },
        }
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::{
    NativeEntityRefsCommonGameExt1 as _, NativeEntityRefsCommonGameExt5 as _,
};

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
