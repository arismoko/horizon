use super::{Attack, DirectEntity, HurtSource, names_its_projectile};
use crate::mobs::{Damage, DamageKind, Who};

impl Attack {
    /// `Enchantment.damageContext`: type predicates see the entities named by
    /// this damage source, not the last attacker or another hit's projectile.
    pub fn enchantment_context<'a>(
        &'a self,
        source: HurtSource<'a>,
        victim: &'a str,
    ) -> crate::enchant::EffectContext<'a> {
        let actor = |id| {
            [
                self.enchantment_attacker.as_ref(),
                self.enchantment_direct.as_ref(),
            ]
            .into_iter()
            .flatten()
            .find(|actor| actor.id == id)
        };
        let causing = actor(source.causing);
        let direct = actor(source.direct);
        crate::enchant::EffectContext {
            this: Some(victim),
            damage_type: Some(source.damage_type),
            is_direct: source.causing == source.direct,
            attacker: causing.map(|actor| actor.kind.as_ref()),
            attacker_motion: causing.and_then(|actor| actor.motion),
            direct_attacker: direct.map(|actor| actor.kind.as_ref()),
            direct_motion: direct.and_then(|actor| actor.motion),
            ..Default::default()
        }
    }

    /// The vanilla class family of this hit's resolved `DamageSource.getDirectEntity()`.
    pub fn direct_entity(&self, damage: &Damage) -> DirectEntity {
        let source = mob_source(damage, self);
        if let Some(direct) = self.direct {
            if direct == 0 {
                return DirectEntity::Other;
            }
            if let Some(kind) = self.direct_kind {
                return kind;
            }
            if self.projectile == Some(direct) {
                return self.projectile_kind;
            }
            return DirectEntity::Other;
        }
        if melee(damage.kind) {
            return if damage.attacker.is_some() {
                DirectEntity::Living
            } else {
                DirectEntity::Other
            };
        }
        if self
            .projectile
            .is_some_and(|entity| entity != 0 && source.direct == entity)
        {
            return self.projectile_kind;
        }
        DirectEntity::Other
    }
    /// `WitherBoss.hurtServer` / `Shulker.hurtServer`: test the direct entity's
    /// `AbstractArrow` family, independently of the damage type's name or tags.
    pub fn direct_is_arrow(&self, damage: &Damage) -> bool {
        self.direct_entity(damage).is_arrow()
    }
    /// `Ghast.isReflectedFireball`: test whether the direct entity is a
    /// `LargeFireball`, independently of the damage type's name or tags.
    pub fn direct_is_large_fireball(&self, damage: &Damage) -> bool {
        self.direct_entity(damage).is_large_fireball()
    }
    /// `DamageSource.isDirect`: whether the cause and direct entity are identical (including absent).
    pub fn is_direct(&self, damage: &Damage) -> bool {
        let source = mob_source(damage, self);
        source.causing == source.direct
    }
    /// `DamageSource.getWeaponItem`: melee's direct living entity or a projectile's saved weapon.
    pub fn weapon_of(&self, direct: i32) -> Option<&crate::items::ItemStack> {
        if direct == 0 {
            None
        } else if self.projectile == Some(direct) {
            self.projectile_weapon.as_ref()
        } else if direct == self.attacker {
            self.held[0].as_ref()
        } else {
            None
        }
    }
    /// The direct entity passed by the damage factory, not whichever entity is nearby.
    pub fn projectile_direct(&self, damage_type: &str) -> i32 {
        if let Some(direct) = self.direct {
            direct
        } else if names_its_projectile(damage_type) {
            self.projectile.unwrap_or(0)
        } else {
            0
        }
    }
    /// The current player's source, shared by every phase and equipment continuation.
    pub fn player_source<'a>(&'a self, damage_type: &'a str, causing: i32) -> HurtSource<'a> {
        let melee = direct_entity_source(damage_type);
        let direct = self.direct.unwrap_or_else(|| {
            if melee {
                causing
            } else {
                self.projectile_direct(damage_type)
            }
        });
        HurtSource {
            damage_type,
            causing: self.arrow_cause(damage_type, causing),
            direct,
            direct_type: self.direct_entity_type(direct),
            position: self.position,
        }
    }

    fn direct_entity_type(&self, direct: i32) -> Option<&str> {
        self.direct_type.as_deref().or_else(|| {
            (direct != 0 && self.projectile == Some(direct))
                .then_some(self.projectile_type)
                .flatten()
        })
    }

    fn arrow_cause(&self, damage_type: &str, causing: i32) -> i32 {
        if causing == 0
            && self.direct.is_none()
            && matches!(damage_type, "minecraft:arrow" | "minecraft:trident")
        {
            // AbstractArrow/ThrownTrident name themselves as cause if their owner is gone.
            self.projectile.unwrap_or(0)
        } else {
            causing
        }
    }
}
/// The damage type id of a Sunrise damage kind. `caused`: the source has a
/// causing entity (only a blast asks).
pub fn damage_type(kind: DamageKind, caused: bool) -> &'static str {
    use crate::mobs::DamageKind as K;
    match kind {
        K::Named(named) => named.name,
        K::PlayerAttack => "minecraft:player_attack",
        K::MaceSmash => "minecraft:mace_smash",
        K::Spear => "minecraft:spear",
        K::MobAttack => "minecraft:mob_attack",
        K::MobAttackNoAggro => "minecraft:mob_attack_no_aggro",
        K::Arrow => "minecraft:arrow",
        K::Thrown => "minecraft:thrown",
        K::Fireball => "minecraft:fireball",
        K::Explosion if caused => "minecraft:player_explosion",
        K::Explosion => "minecraft:explosion",
        K::Fall => "minecraft:fall",
        K::FlyIntoWall => "minecraft:fly_into_wall",
        K::FallingBlock => "minecraft:falling_block",
        K::FallingStalactite => "minecraft:falling_stalactite",
        K::OnFire => "minecraft:on_fire",
        K::InFire => "minecraft:in_fire",
        K::Campfire => "minecraft:campfire",
        K::Lava => "minecraft:lava",
        K::Drown => "minecraft:drown",
        K::DryOut => "minecraft:dry_out",
        K::Magic => "minecraft:magic",
        K::IndirectMagic => "minecraft:indirect_magic",
        K::Sting => "minecraft:sting",
        K::Cactus => "minecraft:cactus",
        K::SweetBerryBush => "minecraft:sweet_berry_bush",
        K::Generic => "minecraft:generic",
        K::InWall => "minecraft:in_wall",
        K::Thorns => "minecraft:thorns",
        K::SonicBoom => "minecraft:sonic_boom",
        K::Freeze => "minecraft:freeze",
        K::OutOfWorld => "minecraft:out_of_world",
        K::Cramming => "minecraft:cramming",
        K::FallingAnvil => "minecraft:falling_anvil",
        K::Fireworks => "minecraft:fireworks",
        K::GenericKill => "minecraft:generic_kill",
        K::LightningBolt => "minecraft:lightning_bolt",
        K::OutsideBorder => "minecraft:outside_border",
        K::Spit => "minecraft:spit",
        K::Stalagmite => "minecraft:stalagmite",
        K::Starve => "minecraft:starve",
        K::HotFloor => "minecraft:hot_floor",
        K::SulfurCubeHot => "minecraft:sulfur_cube_hot",
        K::WindCharge => "minecraft:wind_charge",
        K::Wither => "minecraft:wither",
        K::WitherSkull => "minecraft:wither_skull",
    }
}
/// The damage type id of a mob's damage source (`DamageSources.explosion`: a
/// blast with a causing entity is `player_explosion` whoever it is, an
/// exploding creeper too).
pub fn source_damage_type(d: Damage) -> &'static str {
    damage_type(d.kind, d.attacker.is_some())
}
/// Whether the causing entity deals `kind` itself, so it is the source's
/// direct entity too: a melee hit (`DamageSources.mobAttack`, `playerAttack`,
/// a mace's or a spear's), a sting, a sonic boom, thorns.
pub fn melee(kind: DamageKind) -> bool {
    direct_entity_source(damage_type(kind, false))
}
/// A mob's damage as its `DamageSource`: the causing entity is known when it
/// is a mob, or the player whose attack the level lends. The direct entity is
/// the attacker in melee, or the projectile when this damage type names one
/// (`arrow`, `wither_skull`). `magic()` does not, even during the impact.
pub fn mob_source<'a>(damage: &Damage, attack: &'a Attack) -> HurtSource<'a> {
    let causing = match damage.attacker {
        Some(Who::Mob(id)) => id as i32,
        Some(Who::Player) => attack.attacker,
        None if matches!(
            damage.kind,
            DamageKind::FallingBlock | DamageKind::FallingAnvil | DamageKind::FallingStalactite
        ) =>
        {
            attack.attacker
        }
        None => 0,
    };
    let damage_type = source_damage_type(*damage);
    let direct = attack.direct.unwrap_or_else(|| {
        if melee(damage.kind) {
            causing
        } else {
            attack.projectile_direct(damage_type)
        }
    });
    HurtSource {
        damage_type,
        causing: attack.arrow_cause(damage_type, causing),
        direct,
        direct_type: attack.direct_entity_type(direct),
        position: damage.from,
    }
}

/// `DamageSources`: these factories pass their single living entity as both cause and direct.
fn direct_entity_source(damage_type: &str) -> bool {
    matches!(
        damage_type,
        "minecraft:player_attack"
            | "minecraft:mob_attack"
            | "minecraft:mob_attack_no_aggro"
            | "minecraft:mace_smash"
            | "minecraft:spear"
            | "minecraft:sting"
            | "minecraft:sonic_boom"
            | "minecraft:thorns"
    )
}

#[cfg(test)]
#[path = "attack_tests.rs"]
mod tests;
