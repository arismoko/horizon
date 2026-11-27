//! Single-source-of-truth live pickable target query and non-mob projectile hit dispatch.
use crate::game::deflect_common::redirectable;
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_sim::mobs::Who;
use sunrise_sim::projectiles::{Projectile, ProjectileKind};

/// Per-projectile target filters (`canHitEntity` overrides).
#[derive(Clone, Copy, Debug, Default)]
pub struct TargetQuery {
    pub skip_no_physics: bool,
    pub skip_crystals: bool,
    pub skip_wind_charges: bool,
    pub skip_self_id: Option<u32>,
    pub include_guests: bool,
    pub include_items: bool,
    /// `AbstractArrow.canHitEntity` player-vs-player team / `pvp` rule check.
    pub arrow_friendly_fire: Option<(Who, Option<u32>)>,
}

impl TargetQuery {
    /// Build a live target query for a shared-list projectile.
    pub fn for_projectile(p: &Projectile, include_guests: bool) -> Self {
        let is_wind = matches!(
            p.kind,
            ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
        );
        Self {
            skip_no_physics: projectile_skips_no_physics(&p.kind),
            skip_crystals: is_wind,
            skip_wind_charges: is_wind,
            skip_self_id: (p.id != 0).then_some(p.id),
            include_guests,
            include_items: false,
            arrow_friendly_fire: p
                .arrow
                .is_some()
                .then_some(p.owner)
                .flatten()
                .map(|o| (o, p.guest)),
        }
    }
}

/// `AbstractHurtingProjectile.canHitEntity` / `ShulkerBullet.canHitEntity`: skip `noPhysics`.
pub(in crate::game) fn projectile_skips_no_physics(kind: &ProjectileKind) -> bool {
    matches!(
        kind,
        ProjectileKind::SmallFireball
            | ProjectileKind::LargeFireball { .. }
            | ProjectileKind::DragonFireball
            | ProjectileKind::WitherSkull { .. }
            | ProjectileKind::WindCharge
            | ProjectileKind::BreezeWindCharge
            | ProjectileKind::ShulkerBullet { .. }
    )
}

/// Whether `p` is pickable by other projectiles (`#redirectable_projectile` or `ShulkerBullet`).
pub fn is_pickable_projectile(p: &Projectile) -> bool {
    !p.stuck && (redirectable(p) || matches!(p.kind, ProjectileKind::ShulkerBullet { .. }))
}

/// `LeashFenceKnotEntity.recalculateBoundingBox` around `position` (`(x+0.5, y+0.375, z+0.5)`).
pub fn knot_bounds(position: Vec3) -> Aabb {
    let half = 0.375 / 2.0;
    Aabb::new(
        Vec3::new(position.x - half, position.y, position.z - half),
        Vec3::new(position.x + half, position.y + 0.5, position.z + half),
    )
}

pub fn extents((lo, hi): ([f64; 3], [f64; 3])) -> Aabb {
    Aabb::new(
        Vec3::new(lo[0], lo[1], lo[2]),
        Vec3::new(hi[0], hi[1], hi[2]),
    )
}

/// Context for hurting a non-mob pickable entity from any projectile.
pub struct ProjectileHitInfo {
    pub direct_id: u32,
    pub owner: Option<Who>,
    pub guest: Option<u32>,
    pub damage_type: &'static str,
    pub amount: f32,
    pub velocity: Vec3,
    pub position: Vec3,
    pub is_arrow: bool,
    pub burning_arrow: bool,
    pub ignite_seconds: Option<f32>,
}

impl ProjectileHitInfo {
    pub fn from_projectile(
        p: &Projectile,
        damage_type: &'static str,
        amount: f32,
        ignite_seconds: Option<f32>,
    ) -> Self {
        let burning_arrow = p.arrow.as_ref().is_some_and(|a| a.on_fire());
        Self {
            direct_id: p.id,
            owner: p.owner,
            guest: p.guest,
            damage_type,
            amount,
            velocity: p.velocity,
            position: p.position,
            is_arrow: p.arrow.is_some(),
            burning_arrow,
            ignite_seconds,
        }
    }
}

/// Outcome of `Game::projectile_hurts_non_mob`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NonMobHit {
    /// Target id does not name a non-mob pickable object (caller handles mob/player/special).
    NotObject,
    /// Non-mob object's `hurtServer` returned true.
    Hurt,
    /// Non-mob object's `hurtServer` returned false (refused, e.g. falling block, fixed frame, TNT).
    Refused,
}
