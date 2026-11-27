use mod_api::LivingRef;
use sunrise_gameplay_common::game::creatures::{Hit, Mob, Projectile, ProjectileKind, Vec3, Who};

pub const PLAYER: u32 = 20;
pub const PIG: u32 = 71;
pub const MOB: u32 = 72;
pub const WOLF: u32 = 73;
pub const PLAYER_AGE: i32 = 39;
pub const MOB_AGE: u32 = 117;

pub fn living(game: &crate::game::ServerGame<'_>, id: u32) -> LivingRef {
    game.living(game.entity(id).expect("admitted entity"))
        .expect("living entity")
}

pub fn add_mob(game: &mut crate::game::ServerGame<'_>, id: u32, kind: &str, x: f64) {
    let kind = game.common.res.mob_types().get(kind).unwrap().clone();
    let mut mob = Mob::new(id, kind, Vec3::new(x, 80.0, 0.5), 0.0);
    mob.no_ai = true;
    mob.no_gravity = true;
    mob.ground.on_ground = true;
    game.common.mobs.push(mob);
}

pub fn shoot(game: &mut crate::game::ServerGame<'_>, owner: Who, kind: ProjectileKind) {
    let at = game.common.mobs[0].position;
    let mut shot = Projectile::new(kind, at, Some(owner));
    shot.velocity = Vec3::new(0.0, 0.0, 2.0);
    shot.id = 77;
    game.apply_projectile_hit(
        &shot,
        Hit::Entity {
            who: Who::Mob(PIG),
            at,
        },
    )
    .unwrap();
}

pub fn arrow() -> ProjectileKind {
    ProjectileKind::Arrow {
        base_damage: 2.0,
        critical: false,
        effect: None,
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
