//! Who may hurt a mob: `LivingEntity.isInvulnerableTo`'s `Entity.isInvulnerableToBase` term,
//! and the dead-or-removed guard every damage hook runs after (`LivingIncomingDamageEvent`).
use super::{Damage, Mob, Who};

impl Mob {
    /// `Ghast.isReflectedFireball`: a player returning a `LargeFireball` gets
    /// past a ghast's fire immunity, regardless of the damage type.
    pub(super) fn reflected_fireball(
        &self,
        source: &Damage,
        attack: &crate::health::Attack,
    ) -> bool {
        &*self.kind.id == "minecraft:ghast"
            && source.attacker == Some(Who::Player)
            && attack.direct_is_large_fireball(source)
    }

    /// `Entity.isInvulnerableToBase`'s invulnerability term: `Invulnerable` or
    /// `invulnerable_time` left, which only #bypasses_invulnerability or a blow a creative
    /// player caused (`DamageSource.isCreativePlayer`) gets past.
    pub fn is_invulnerable_to(&self, source: &Damage) -> bool {
        self.is_invulnerable() && !self.creative_attacker && !source.kind.bypasses_invulnerability()
    }
    /// `LivingEntity.isInvulnerableTo` and dead or removed: only #bypasses_invulnerability or a
    /// creative player's blow gets past the flag. The guard every damage hook runs after
    /// (`LivingIncomingDamageEvent`).
    pub fn refuses_damage(&self, source: &Damage) -> bool {
        self.extra.client_side
            || !self.alive()
            || self.removed
            || self.is_invulnerable_to(source)
            || (source.is_fire() && self.fire_immune())
    }
}
