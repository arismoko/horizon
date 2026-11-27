//! Projectile handles for native server mods: owner, deflection, and an
//! arrow's pickup, damage and critical flag. They cover the projectile list
//! and the player-thrown tridents and firework rockets, which Sunrise keeps
//! in lists of their own.
use super::commands::Target;
use mod_api::{ArrowPickup, Deflection, EntityRef, PierceLevelError, ProjectileRef};
use sunrise_core::geometry::Vec3;
use sunrise_sim::{
    entities::arrows::Pickup,
    lan::playerdata::{ints_uuid, uuid_ints},
    mobs::Who,
    projectiles::{OwnerFields, Projectile, ProjectileKind, Turn},
};

/// Where a projectile handle's entity lives.
#[derive(Clone, Copy)]
enum Backing {
    /// `creatures.projectiles`.
    List(usize),
    /// `tridents`: a trident a player threw.
    Trident(usize),
    /// `rockets`: a firework rocket.
    Rocket(usize),
}

impl crate::game::ServerGame<'_> {
    fn native_projectile_backing(&self, projectile: ProjectileRef) -> Option<Backing> {
        match self.native_entity_target(projectile.entity())? {
            Target::Projectile(id) => (self.common.creatures.projectiles.iter())
                .position(|p| p.id == id)
                .map(Backing::List),
            Target::Trident(id) => (self.common.tridents.iter())
                .position(|t| t.id == id)
                .map(Backing::Trident),
            Target::Rocket(id) => (self.common.rockets.iter())
                .position(|r| r.id == id)
                .map(Backing::Rocket),
            _ => None,
        }
    }

    /// The index in the projectile list: None for a trident a player threw,
    /// a rocket or a stale handle, which have no arrow state, kind or acceleration.
    fn native_projectile_index(&self, projectile: ProjectileRef) -> Option<usize> {
        match self.native_projectile_backing(projectile)? {
            Backing::List(i) => Some(i),
            _ => None,
        }
    }

    fn native_projectile(&self, projectile: ProjectileRef) -> Option<&Projectile> {
        self.common
            .creatures
            .projectiles
            .get(self.native_projectile_index(projectile)?)
    }

    /// Narrows an entity to a projectile: one in the projectile list, a trident
    /// a player threw or a firework rocket. None for another kind, a stale
    /// handle or a client.
    pub fn projectile(&self, entity: EntityRef) -> Option<ProjectileRef> {
        let projectile = ProjectileRef::new(entity);
        self.native_projectile_backing(projectile)
            .map(|_| projectile)
    }

    /// The owner a target names, as a resolved entity in the level in place.
    fn native_owner_entity(&self, target: Target) -> Option<EntityRef> {
        if self.target_level(target) != self.common.dim {
            return None;
        }
        let id = u32::try_from(self.network_entity_id(target)?).ok()?;
        self.entity(id)
    }

    /// `Projectile.getOwner`: the owner as a resolved entity. None for no
    /// owner, one that is gone or in another level, or a stale handle.
    pub fn projectile_owner(&self, projectile: ProjectileRef) -> Option<EntityRef> {
        self.native_projectile_backing(projectile)?;
        self.entity_owner(projectile.entity())
    }

    /// `TraceableEntity.getOwner`: a projectile's owner (`Projectile.getOwner`)
    /// or a general row's, as a resolved entity. None for no owner or a kind
    /// with none, one that is gone or in another level, or a stale handle.
    pub fn entity_owner(&self, entity: EntityRef) -> Option<EntityRef> {
        let Some(backing) = self.native_projectile_backing(ProjectileRef::new(entity)) else {
            let Target::General(id) = self.native_entity_target(entity)? else {
                return None;
            };
            let rows = &self.common.resident_entities.general.rows;
            let owner = rows.iter().find(|row| row.id == id)?.owner?;
            return self.native_owner_entity(self.target_by_uuid(ints_uuid(owner))?);
        };
        let target = match backing {
            Backing::List(i) => {
                let uuid = self.common.creatures.projectiles[i].owner_uuid?;
                self.target_by_uuid(ints_uuid(uuid))?
            }
            // A trident belongs to the guest who threw it, else the host.
            Backing::Trident(i) => self.common.tridents[i]
                .guest
                .map_or(Target::Player, Target::Guest),
            Backing::Rocket(i) => {
                let r = &self.common.rockets[i];
                match r.owner {
                    Some(Who::Player) => r.guest.map_or(Target::Player, Target::Guest),
                    Some(Who::Mob(id)) => Target::Mob(id),
                    None => self.target_by_uuid(ints_uuid(r.owner_uuid?))?,
                }
            }
        };
        self.native_owner_entity(target)
    }

    /// Resolves hit credit, persistent identity, and a separate non-living
    /// vehicle identity for collision exclusion. None for a stale owner or hook.
    pub(in crate::game::server_side) fn native_projectile_owner_fields(
        &mut self,
        owner: Option<EntityRef>,
    ) -> Option<(OwnerFields, Option<u32>)> {
        let Some(owner) = owner else {
            return Some(((None, None, None), None));
        };
        let target = self.native_entity_target(owner)?;
        let vehicle = match target {
            Target::Boat(id) | Target::Cart(id) => Some(id),
            _ => None,
        };
        let (who, guest) = match target {
            Target::Player => (Some(Who::Player), self.server.lan_acting),
            Target::Guest(conn) => (Some(Who::Player), Some(conn)),
            Target::Mob(id) => (Some(Who::Mob(id)), None),
            Target::Hook(_) => return None,
            _ => (None, None),
        };
        let uuid = uuid_ints(self.entity_uuid(owner)?);
        Some(((who, Some(uuid), guest), vehicle))
    }

    /// `Projectile.setOwner` on the thing a handle names. False when the
    /// owner does not fit: a trident a player threw always has a player owner.
    fn native_projectile_apply_owner(
        &mut self,
        backing: Backing,
        owner: (OwnerFields, Option<u32>),
    ) -> bool {
        let (fields, vehicle) = owner;
        let (who, uuid, guest) = fields;
        match backing {
            Backing::List(i) => {
                self.common.creatures.projectiles.update(i, |p| {
                    (p.owner, p.owner_uuid, p.guest) = fields;
                    p.owner_vehicle = vehicle;
                    // `AbstractArrow.setOwner(Entity)`: a player's arrow may be picked up.
                    if let Some(a) = &mut p.arrow {
                        a.set_owner(who);
                    }
                });
            }
            Backing::Trident(i) => {
                if who != Some(Who::Player) {
                    return false;
                }
                self.common.tridents[i].guest = guest;
            }
            Backing::Rocket(i) => {
                self.common.rockets[i].set_owner(who, uuid, guest);
            }
        }
        true
    }

    /// `Projectile.setOwner`. False for a stale projectile or owner, or an
    /// owner that is a fishing hook (a trident a player threw takes a player
    /// only). Does not touch `leftOwner`.
    pub fn projectile_set_owner(
        &mut self,
        projectile: ProjectileRef,
        owner: Option<EntityRef>,
    ) -> bool {
        let Some(backing) = self.native_projectile_backing(projectile) else {
            return false;
        };
        let Some(fields) = self.native_projectile_owner_fields(owner) else {
            return false;
        };
        self.native_projectile_apply_owner(backing, fields)
    }

    /// `Projectile.deflect(deflection, by, newOwner, byAttack, power)`, with
    /// `power` per axis, through `Projectile::deflect` for the projectile list
    /// (the engine path gameplay uses). `REVERSE` draws one float from the
    /// projectile's own random; `AIM_DEFLECT` uses `by`'s look and
    /// `MOMENTUM_DEFLECT` its normalized known speed, and both change nothing
    /// without `by`. The new owner is set, then `onDeflection`: a hurting
    /// projectile's acceleration power is 0.1 by an attack, else halved. A
    /// wind charge refuses for its first five ticks, with no change and no
    /// draw. A trident a player threw and a rocket turn the same way (no
    /// acceleration power). False, and nothing changed, for a stale projectile,
    /// `by` or owner, a non-finite power or a refusal.
    pub(in crate::game) fn projectile_deflect(
        &mut self,
        projectile: ProjectileRef,
        deflection: Deflection,
        by: Option<EntityRef>,
        new_owner: Option<EntityRef>,
        by_attack: bool,
        power: [f64; 3],
    ) -> bool {
        let Some(backing) = self.native_projectile_backing(projectile) else {
            return false;
        };
        let Some((owner, vehicle)) = self.native_projectile_owner_fields(new_owner) else {
            return false;
        };
        if !power.iter().all(|v| v.is_finite()) {
            return false;
        }
        let turn = match (deflection, by) {
            (Deflection::Reverse, _) => Turn::Reverse,
            (_, None) => Turn::Direction(None),
            (Deflection::AimDeflect, Some(by)) => {
                let Some([x, y, z]) = self.entity_view_vector(by) else {
                    return false;
                };
                Turn::Direction(Some(Vec3::new(x, y, z)))
            }
            (Deflection::MomentumDeflect, Some(by)) => {
                let Some(speed) = self.native_entity_known_speed(by) else {
                    return false;
                };
                Turn::Direction(Some(speed.normalize()))
            }
            _ => unreachable!("(Deflection, Option<EntityRef>): unhandled variant"),
        };
        match backing {
            // `deflect` takes the `EntityReference` overload of `setOwner`: an
            // arrow's pickup does not change, unlike `set_owner`.
            Backing::List(i) => self.common.creatures.projectiles.update(i, |p| {
                let applied = p.deflect(turn, power, owner, by_attack);
                if applied {
                    p.owner_vehicle = vehicle;
                }
                applied
            }),
            Backing::Trident(i) => {
                if owner.0 != Some(Who::Player) {
                    return false;
                }
                let t = &mut self.common.tridents[i];
                if let Some((velocity, rotation)) = turn.apply(t.velocity, power, &mut t.random) {
                    t.velocity = velocity;
                    t.yaw += rotation;
                }
                t.guest = owner.2;
                true
            }
            Backing::Rocket(i) => {
                let r = &mut self.common.rockets[i];
                if let Some((velocity, _)) = turn.apply(r.velocity, power, &mut r.random) {
                    r.velocity = velocity;
                }
                r.set_owner(owner.0, owner.1, owner.2);
                true
            }
        }
    }

    /// `AbstractArrow.pickup`, for an arrow or a mob's trident.
    pub(in crate::game) fn projectile_pickup(
        &self,
        projectile: ProjectileRef,
    ) -> Option<ArrowPickup> {
        Some(
            match self.native_projectile(projectile)?.arrow.as_ref()?.pickup {
                Pickup::Disallowed => ArrowPickup::Disallowed,
                Pickup::Allowed => ArrowPickup::Allowed,
                Pickup::CreativeOnly => ArrowPickup::CreativeOnly,
            },
        )
    }

    /// Sets `AbstractArrow.pickup`. False for a stale handle or a projectile
    /// that is not an arrow or a mob's trident.
    pub(in crate::game) fn projectile_set_pickup(
        &mut self,
        projectile: ProjectileRef,
        pickup: ArrowPickup,
    ) -> bool {
        let Some(index) = self.native_projectile_index(projectile) else {
            return false;
        };
        self.common.creatures.projectiles.update(index, |p| {
            let Some(arrow) = p.arrow.as_mut() else {
                return false;
            };
            arrow.pickup = match pickup {
                ArrowPickup::Disallowed => Pickup::Disallowed,
                ArrowPickup::Allowed => Pickup::Allowed,
                ArrowPickup::CreativeOnly => Pickup::CreativeOnly,
                _ => unreachable!("ArrowPickup: unhandled variant"),
            };
            true
        })
    }

    fn native_arrow_index(&self, projectile: ProjectileRef) -> Result<usize, PierceLevelError> {
        if !self.common.is_logical_server() {
            return Err(PierceLevelError::WrongSide);
        }
        let backing = self
            .native_projectile_backing(projectile)
            .ok_or(PierceLevelError::Stale)?;
        match backing {
            Backing::List(index) if self.common.creatures.projectiles[index].arrow.is_some() => {
                Ok(index)
            }
            _ => Err(PierceLevelError::NotArrow),
        }
    }

    /// `AbstractArrow.getPierceLevel`: the signed byte stored by vanilla.
    pub(in crate::game) fn projectile_pierce_level(
        &self,
        projectile: ProjectileRef,
    ) -> Result<i8, PierceLevelError> {
        let index = self.native_arrow_index(projectile)?;
        Ok(self.common.creatures.projectiles[index]
            .arrow
            .as_ref()
            .unwrap()
            .pierce as i8)
    }

    /// `AbstractArrow.setPierceLevel`: stores the signed byte without clamping.
    pub(in crate::game) fn projectile_set_pierce_level(
        &mut self,
        projectile: ProjectileRef,
        level: i8,
    ) -> Result<(), PierceLevelError> {
        let index = self.native_arrow_index(projectile)?;
        self.common
            .creatures
            .projectiles
            .update(index, |p| p.arrow.as_mut().unwrap().pierce = level as u8);
        Ok(())
    }

    /// `AbstractArrow.getBaseDamage`, for an arrow (a trident's is fixed).
    pub(in crate::game) fn projectile_base_damage(&self, projectile: ProjectileRef) -> Option<f64> {
        match self.native_projectile(projectile)?.kind {
            ProjectileKind::Arrow { base_damage, .. } => Some(base_damage),
            _ => None,
        }
    }

    /// `AbstractArrow.setBaseDamage`: stores `damage` as supplied. False for a stale
    /// handle or a projectile without an arrow's base damage.
    pub(in crate::game) fn projectile_set_base_damage(
        &mut self,
        projectile: ProjectileRef,
        damage: f64,
    ) -> bool {
        let Some(index) = self.native_projectile_index(projectile) else {
            return false;
        };
        self.common.creatures.projectiles.update(index, |p| {
            let ProjectileKind::Arrow { base_damage, .. } = &mut p.kind else {
                return false;
            };
            *base_damage = damage;
            true
        })
    }

    /// `AbstractArrow.isCritArrow`, for an arrow.
    pub(in crate::game) fn projectile_critical(&self, projectile: ProjectileRef) -> Option<bool> {
        let p = self.native_projectile(projectile)?;
        matches!(p.kind, ProjectileKind::Arrow { .. }).then(|| p.critical())
    }

    /// `AbstractArrow.setCritArrow`; clients see the flag at once through the
    /// entity data. False for a stale handle or a projectile that is not an arrow.
    pub(in crate::game) fn projectile_set_critical(
        &mut self,
        projectile: ProjectileRef,
        critical: bool,
    ) -> bool {
        let Some(index) = self.native_projectile_index(projectile) else {
            return false;
        };
        self.common.creatures.projectiles.update(index, |p| {
            if !matches!(p.kind, ProjectileKind::Arrow { .. }) {
                return false;
            }
            p.set_critical(critical);
            true
        })
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
