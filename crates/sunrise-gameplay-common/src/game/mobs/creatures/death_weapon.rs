//! Recorded projectile weapons and the death-loot producer.
use super::*;

impl crate::game::CommonGame<'_> {
    /// `DamageSources.arrow` / `trident`: type and tags follow the actual projectile.
    pub fn projectile_damage_kind(&self, shot: &Projectile) -> DamageKind {
        match shot.kind {
            ProjectileKind::Arrow { .. } => DamageKind::Arrow,
            ProjectileKind::Trident => DamageKind::Named(
                sunrise_sim::mobs::NamedDamage::registered(
                    &self.health_resources.damage,
                    "minecraft:trident",
                )
                .expect("registered vanilla trident damage"),
            ),
            ProjectileKind::SmallFireball | ProjectileKind::LargeFireball { .. } => {
                DamageKind::Fireball
            }
            _ => DamageKind::Thrown,
        }
    }
    pub fn set_projectile_source(attack: &mut Attack, shot: &Projectile) {
        attack.projectile = Some(shot.id as i32);
        attack.enchantment_direct = Some(sunrise_sim::health::EnchantmentActor {
            id: shot.id as i32,
            kind: shot.entity_type().into(),
            motion: None,
        });
        attack.projectile_kind = Self::projectile_direct_entity(&shot.kind);
        attack.projectile_type = Some(shot.entity_type());
    }
    /// Classify a projectile's vanilla `DamageSource.getDirectEntity()` family.
    pub fn projectile_direct_entity(kind: &ProjectileKind) -> sunrise_sim::health::DirectEntity {
        match kind {
            ProjectileKind::Arrow { .. } | ProjectileKind::Trident => {
                sunrise_sim::health::DirectEntity::Arrow
            }
            ProjectileKind::LargeFireball { .. } => {
                sunrise_sim::health::DirectEntity::LargeFireball
            }
            _ => sunrise_sim::health::DirectEntity::Projectile,
        }
    }
}
