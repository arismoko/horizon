//! The client ticks of what a server's client tracks as projectiles and area effect clouds.
use sunrise_client::items::ItemStack;
use sunrise_client::join_seq::JoinList;
use sunrise_client::particles::{ParticleEngine, ParticleLevel, ParticleOptions, ParticleType};
use sunrise_client::projectiles::{Hit, Projectile, ProjectileKind, ProjectileWorld};
use sunrise_core::{
    geometry::{Vec3, trig},
    random::{LegacyRandom, RandomSource},
};
use sunrise_sim::entities::arrows::{ArrowEvent, ArrowScene, tick_client};

/// An `AreaEffectCloud` a server tracks: what its client tick reads.
pub struct RemoteCloud {
    pub id: u32,
    pub at: [f64; 3],
    pub radius: f32,
    pub waiting: bool,
    pub particle: ParticleOptions,
    pub random: LegacyRandom,
}

/// `AreaEffectCloud.DEFAULT_PARTICLE`: a white entity effect.
pub fn default_cloud_particle() -> ParticleOptions {
    ParticleOptions::colored(ParticleType::EntityEffect, [1.0; 4])
}

/// `AreaEffectCloud.clientTick`: while waiting, now and then two particles
/// near the middle; then about one per square block of the disc, drifting
/// (an entity effect stays still).
pub fn cloud_client_tick(
    particles: &mut ParticleEngine,
    level: &ParticleLevel<'_>,
    at: [f64; 3],
    radius: f32,
    waiting: bool,
    particle: &ParticleOptions,
    random: &mut LegacyRandom,
) {
    if waiting && random.next_boolean() {
        return;
    }
    let (count, spread) = if waiting {
        (2, 0.2)
    } else {
        (
            (std::f32::consts::PI * radius * radius).ceil() as i32,
            radius,
        )
    };
    for _ in 0..count {
        let r = &mut *random;
        let angle = r.next_float() * std::f32::consts::TAU;
        let distance = r.next_float().sqrt() * spread;
        let at = [
            at[0] + f64::from(trig::cos(f64::from(angle)) * distance),
            at[1],
            at[2] + f64::from(trig::sin(f64::from(angle)) * distance),
        ];
        let default;
        let (options, velocity) = if particle.ty == ParticleType::EntityEffect {
            let options = if waiting && r.next_boolean() {
                default = default_cloud_particle();
                &default
            } else {
                particle
            };
            (options, [0.0; 3])
        } else if waiting {
            (particle, [0.0; 3])
        } else {
            let v = [
                (0.5 - r.next_double()) * 0.15,
                f64::from(0.01f32),
                (0.5 - r.next_double()) * 0.15,
            ];
            (particle, v)
        };
        // The no-flag overload: always-show thinning, without a limiter override.
        particles.add_always_visible_with_level(level, options, at, velocity);
    }
}

/// The projectile a type id names, as Sunrise models it.
pub fn projectile_kind(kind: &str) -> Option<ProjectileKind> {
    Some(match kind {
        "minecraft:arrow" | "minecraft:spectral_arrow" => ProjectileKind::Arrow {
            base_damage: 2.0,
            critical: false,
            effect: None,
        },
        "minecraft:snowball" => ProjectileKind::Snowball,
        "minecraft:egg" => ProjectileKind::Egg,
        "minecraft:ender_pearl" => ProjectileKind::EnderPearl,
        "minecraft:small_fireball" => ProjectileKind::SmallFireball,
        "minecraft:fireball" => ProjectileKind::LargeFireball { power: 1 },
        "minecraft:dragon_fireball" => ProjectileKind::DragonFireball,
        "minecraft:llama_spit" => ProjectileKind::LlamaSpit,
        "minecraft:wind_charge" => ProjectileKind::WindCharge,
        "minecraft:breeze_wind_charge" => ProjectileKind::BreezeWindCharge,
        // Homing state stays on the server; the client interpolates its positions.
        "minecraft:shulker_bullet" => ProjectileKind::ShulkerBullet {
            target: Some(sunrise_client::mobs::Who::Player),
            dir: None,
            steps: 0,
            delta: [0.0; 3],
            seed: 0,
        },
        "minecraft:wither_skull" => ProjectileKind::WitherSkull { dangerous: false },
        _ => ProjectileKind::Thrown(sunrise_client::projectiles::thrown_type(kind)?),
    })
}

/// `ShulkerBullet.tick` on a server's client: only the server steers it, so
/// the client moves it by its motion and applies the effects of the blocks it
/// went through (`applyEffectsFromBlocks`): a bubble column pushes it. The
/// surface's particles are the server's to send (`sendBubbleColumnParticles`).
pub fn remote_bullet_tick(p: &mut Projectile, w: &impl ProjectileWorld) {
    p.set_old_pos_and_rot();
    p.age += 1;
    let from = p.position;
    p.position = p.position.add_vector(p.velocity);
    p.bubble_columns_along(w, from);
    p.bubble_surfaces.clear();
}

/// A server's client ticks a projectile itself: it flies on between the
/// tracker's updates (10 to 20 ticks apart) with its own gravity and drag,
/// and a block stops it there; only the server decides what it hits.
/// The tridents' type, as the tick hooks name it.
pub const TRIDENT: &str = "minecraft:trident";

/// One projectile's tick on a client: an arrow flies on through the scene (its events kept for
/// after the family), a shulker bullet and the others through the world view.
pub fn remote_projectile_step(
    p: &mut Projectile,
    arrow_scene: &ArrowScene<'_, '_>,
    world: &impl ProjectileWorld,
    arrow_events: &mut Vec<(Vec3, ArrowEvent)>,
) {
    match p.kind {
        ProjectileKind::Arrow { .. } => {
            let _ = tick_client(p, arrow_scene);
            if let Some(a) = p.arrow.as_mut() {
                arrow_events.extend(
                    std::mem::take(&mut a.events)
                        .into_iter()
                        .map(|event| (p.position, event)),
                );
            }
        }
        ProjectileKind::ShulkerBullet { .. } => remote_bullet_tick(p, world),
        _ => remote_projectile_tick(p, world),
    }
}

pub fn remote_projectile_tick(p: &mut Projectile, w: &impl ProjectileWorld) {
    if !matches!(p.tick_remote(w).1, Hit::None) {
        p.velocity = Vec3::ZERO;
    }
}

/// `Entity.getRandomX(0.5)` / `getRandomY` / `getRandomZ(0.5)` for a shulker
/// bullet (`EntityType.SHULKER_BULLET` is 0.3125 x 0.3125).
pub fn shulker_bullet_trail_at(at: Vec3, random: &mut impl RandomSource) -> [f64; 3] {
    const SIZE: f64 = 0.3125;
    [
        at.x + SIZE * (2.0 * random.next_double() - 1.0) * 0.5,
        at.y + SIZE * random.next_double(),
        at.z + SIZE * (2.0 * random.next_double() - 1.0) * 0.5,
    ]
}

/// `ClientPacketListener.handleSetEntityData`'s `Fireball.DATA_ITEM_STACK`:
/// the item the guest draws the fireball as, through `Fireball.setItem`;
/// a kind that is no `Fireball` has none.
pub fn remote_projectile_item(
    projectiles: &mut JoinList<Projectile, false>,
    id: u32,
    stack: ItemStack,
) {
    if let Some(i) = projectiles
        .iter()
        .position(|p| p.id == id && p.kind.is_item_fireball())
    {
        projectiles.update(i, |p| p.set_item(stack));
    }
}

/// `ClientPacketListener.handleProjectilePowerPacket`: the power a hurting
/// projectile (`AbstractHurtingProjectile`) accelerates by, found by id;
/// anything else is ignored.
pub fn remote_projectile_power(projectiles: &mut JoinList<Projectile, false>, id: i32, power: f64) {
    if let Some(i) = projectiles.iter().position(|p| p.id == id as u32)
        && matches!(
            projectiles[i].kind,
            ProjectileKind::SmallFireball
                | ProjectileKind::LargeFireball { .. }
                | ProjectileKind::DragonFireball
                | ProjectileKind::WitherSkull { .. }
                | ProjectileKind::WindCharge
                | ProjectileKind::BreezeWindCharge
        )
    {
        projectiles.update(i, |p| p.acceleration = power);
    }
}
