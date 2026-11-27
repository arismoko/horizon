use sunrise_sim::projectiles::{Projectile, ProjectileKind};

/// Wither skulls drawn as the skull model, blue when dangerous.
pub fn skull_kind(p: &Projectile) -> Option<bool> {
    match p.kind {
        ProjectileKind::WitherSkull { dangerous } => Some(dangerous),
        _ => None,
    }
}
