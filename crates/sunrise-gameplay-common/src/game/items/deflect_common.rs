use sunrise_sim::projectiles::{Projectile, ProjectileKind};

/// `AbstractHurtingProjectile.getPickRadius`: the crosshair finds it a block
/// round its box.
pub const PICK_RADIUS: f64 = 1.0;

/// `#minecraft:redirectable_projectile`: a ghast's fireball and both wind charges.
pub fn redirectable(p: &Projectile) -> bool {
    !p.stuck
        && matches!(
            p.kind,
            ProjectileKind::LargeFireball { .. }
                | ProjectileKind::WindCharge
                | ProjectileKind::BreezeWindCharge
        )
}

/// A projectile a player's blow reaches under the vanilla default tag: a redirectable
/// projectile or a shulker bullet (`ShulkerBullet.isPickable` is `true`).
pub fn attackable(p: &Projectile) -> bool {
    redirectable(p) || matches!(p.kind, ProjectileKind::ShulkerBullet { .. })
}
