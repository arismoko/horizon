//! The ender dragon fight (`EnderDragonFight`, `EnderDragon`, its phases,
//! `EndCrystal`, `DragonFireball`): the fight state saved in the End's
//! `ender_dragon_fight.dat`, the dragon spawned over the island, the
//! holding pattern over the 24-node path graph, strafing with fireballs,
//! landing on the podium to breathe and charge, healing from the crystals on
//! the spikes, its parts (head and neck take full damage, the rest a quarter
//! plus one), wing knockback, block breaking, the death flight and rays, the
//! exit portal, the egg and the next gateway; and its rendering and boss bar.
#[path = "dragon/death.rs"]
pub(in crate::game) mod death;
#[cfg(test)]
#[path = "dragon/fight_hooks_tests.rs"]
mod fight_hooks_tests;
#[path = "dragon/residency.rs"]
pub(in crate::game) mod residency;
#[path = "dragon/save_lending.rs"]
pub(in crate::game) mod save_lending;
#[path = "dragon/walls.rs"]
pub(in crate::game) mod walls;
use super::commands::Target;
use mod_api::DragonRespawnStage;
use sunrise_core::nbt::Tag;
use sunrise_core::position::BlockPos;
use sunrise_core::{
    geometry::{Aabb, Vec3, trig},
    jmath,
    random::{LegacyRandom, RandomSource},
};
use sunrise_gameplay_common::game::creatures::SpawnSite;
use sunrise_gameplay_common::game::dragon_common::{
    BREATH_WAIT, Breath, Crystal, Dragon, FightWho, MAX_HEALTH, Path, PathBox, Phase, SPECIAL,
    breath_expired, dragon_part_id, wrap,
};
use {
    crate::game::server_side::potions_server::mob_can_be_affected,
    sunrise_gameplay_common::game::potions_common::Victim,
};
#[allow(unused_imports)]
use {
    sunrise_game_data::text::Text, sunrise_level::particle_options::ParticleType,
    sunrise_level::random_source, sunrise_sim::health::DamageContainer,
    sunrise_sim::health::DamageLevel, sunrise_sim::health::HurtSource,
    sunrise_sim::health::Hurting, sunrise_sim::health::Knock, sunrise_sim::health::Reduction,
    sunrise_sim::mobs::Damage, sunrise_sim::mobs::DamageKind, sunrise_sim::mobs::Who,
    sunrise_sim::projectiles::Projectile, sunrise_sim::projectiles::ProjectileKind,
};

/// `HealOrHarmMobEffect.applyInstantaneousEffect` at an `AreaEffectCloud`'s
/// 0.5 scale: `(int) (scale * (6 << amplifier) + 0.5)`.
pub(in crate::game::server_side) fn harm_amount(amplifier: u32) -> f32 {
    (0.5 * f64::from(6u32 << amplifier) + 0.5) as i32 as f32
}

/// Whether a cloud at `at` (its floor) reaches a living entity: its box is
/// `radius * 2` wide and 0.5 tall, and the entity's position must lie within
/// the radius horizontally.
pub(in crate::game::server_side) fn cloud_hits(
    at: [f64; 3],
    radius: f32,
    bounds: Aabb,
    pos: Vec3,
) -> bool {
    let r = f64::from(radius);
    let area = Aabb::new(
        Vec3::new(at[0] - r, at[1], at[2] - r),
        Vec3::new(at[0] + r, at[1] + 0.5, at[2] + r),
    );
    let (dx, dz) = (pos.x - at[0], pos.z - at[2]);
    bounds.intersects(area) && dx * dx + dz * dz <= f64::from(radius * radius)
}

/// `AreaEffectCloud.serverTick` victims: every 5 ticks after the wait, drop
/// entries whose delay (`reapplicationDelay` 20) has passed, then return the
/// candidates not in the map, recording them until `age + 20`.
pub(in crate::game::server_side) fn breath_victims<T: Copy + PartialEq>(
    age: i32,
    victims: &mut Vec<(T, i32)>,
    candidates: impl IntoIterator<Item = T>,
) -> Vec<T> {
    if age < BREATH_WAIT || age % 5 != 0 {
        return Vec::new();
    }
    victims.retain(|&(_, until)| age < until);
    let mut hit = Vec::new();
    for who in candidates {
        if !victims.iter().any(|&(v, _)| v == who) {
            victims.push((who, age + 20));
            hit.push(who);
        }
    }
    hit
}

/// `DragonFireball.onHit`: the cloud moves to the first living entity in the
/// fireball's box inflated by (4, 2, 4) that is within 4 blocks of it.
/// `feet` is the fireball's position (the bottom of its 1x1x1 box).
pub(in crate::game::server_side) fn fireball_cloud_origin(
    feet: [f64; 3],
    targets: &[(Aabb, Vec3)],
) -> [f64; 3] {
    let area = Aabb::new(
        Vec3::new(feet[0] - 4.5, feet[1] - 2.0, feet[2] - 4.5),
        Vec3::new(feet[0] + 4.5, feet[1] + 3.0, feet[2] + 4.5),
    );
    for (bounds, pos) in targets {
        let (dx, dy, dz) = (pos.x - feet[0], pos.y - feet[1], pos.z - feet[2]);
        if bounds.intersects(area) && dx * dx + dy * dy + dz * dz < 16.0 {
            return [pos.x, pos.y, pos.z];
        }
    }
    feet
}

/// Direct projectile metadata for `hit_special_full` (both shared projectiles and `ThrownTrident`).
pub(in crate::game) struct DirectHit<'a> {
    pub id: u32,
    pub position: Vec3,
    pub kind: Option<&'a sunrise_sim::projectiles::ProjectileKind>,
    projectile: Option<&'a Projectile>,
    entity_type: Option<&'static str>,
    pub attack: Option<&'a sunrise_sim::health::Attack>,
}
impl<'a> DirectHit<'a> {
    pub(in crate::game) fn from_projectile(p: &'a Projectile) -> Self {
        Self {
            id: p.id,
            position: p.position,
            kind: Some(&p.kind),
            projectile: Some(p),
            entity_type: None,
            attack: None,
        }
    }
    pub(in crate::game) fn trident(
        id: u32,
        position: Vec3,
        attack: Option<&'a sunrise_sim::health::Attack>,
    ) -> Self {
        Self {
            id,
            position,
            kind: Some(&ProjectileKind::Trident),
            projectile: None,
            entity_type: Some("minecraft:trident"),
            attack,
        }
    }
    fn resolve_entity_type(&self) -> Option<&'static str> {
        self.entity_type
            .or_else(|| self.projectile.map(Projectile::entity_type))
    }
}

pub(in crate::game) const FIGHT_FILE: &str = "data/minecraft/ender_dragon_fight.dat";
/// `Mob.getAmbientSoundInterval`.
pub(in crate::game) const AMBIENT_INTERVAL: i32 = 80;

/// A full hit's default knockback (`dealDefaultKnockback`): where it comes from, and the
/// dragon's knockback resistance when it lands, which the hurt alone cannot read.
#[derive(Clone, Copy, Debug)]
pub(in crate::game) struct Push {
    pub(in crate::game) from: Vec3,
    pub(in crate::game) resistance: f64,
}

#[derive(Default)]
pub struct Fight {
    pub loaded: bool,
    pub(in crate::game) needs_scan: bool,
    pub killed: bool,
    pub previously_killed: bool,
    /// `dragonUUID`: the fight's own dragon. Only its death wins the fight
    /// (`setDragonKilled`); a dragon summoned after the fight is won is not it.
    pub dragon_uuid: Option<[i32; 4]>,
    /// `ticksSinceDragonSeen`: ticks since the fight's dragon last ticked
    /// (`updateDragon`); at 1200 the fight looks for another one.
    pub(in crate::game) dragon_unseen: i32,
    pub(in crate::game) gateways: Vec<i32>,
    pub(in crate::game) exit_portal: Option<[i32; 3]>,
    pub alive_crystals: i32,
    pub(in crate::game) crystal_scan: i32,
    /// `ticksSinceLastPlayerScan`, and whether the last scan found a player
    /// for the fight (`dragonEvent.getPlayers()` is not empty).
    pub(in crate::game) player_scan: i32,
    pub(in crate::game) players: bool,
    /// The spikes' crystals have been placed (worldgen does not place entities here).
    pub(in crate::game) crystals_placed: bool,
    /// The respawn sequence (`DragonRespawnStage`) and its tick count.
    pub(in crate::game) respawn: Option<(Respawn, i32)>,
    /// The four crystals on the exit portal driving the respawn.
    pub(in crate::game) respawn_crystals: Vec<Vec3>,
    /// The player behind the blast under way, whose `explosion` damage
    /// source names them: a crystal's (`EndCrystal.hurtServer`) or a large
    /// fireball's (`Explosion.getIndirectSourceEntity`, its owner).
    pub(in crate::game) blast_by: Option<FightWho>,
    /// The last hit the dragon took (`die(source)`'s source, for the
    /// `player_killed_entity` trigger's `killing_blow`): its damage type and
    /// whether a player was behind it.
    pub(in crate::game) last_blow: Option<(DamageKind, bool)>,
    pub(in crate::game) dirty: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) enum Respawn {
    Start,
    Preparing,
    Pillars,
    Summoning,
}
impl Fight {
    /// `EnderDragonFight.tick`'s ticket condition: the fight's boss bar has a
    /// player (`dragonEvent.getPlayers()` is not empty), which holds the
    /// arena's `TicketType.DRAGON` ticket. The bar is scanned every 20 ticks,
    /// as `updatePlayers` is.
    pub(in crate::game::server_side) fn arena_watched(&self) -> bool {
        self.players
    }
    /// The fight as mods read it.
    pub(super) fn api(&self) -> mod_api::DragonFightState {
        mod_api::DragonFightState {
            won: self.killed,
            previously_won: self.previously_killed,
            respawn_stage: self.respawn.map(|(stage, _)| stage.api()),
            alive_crystals: self.alive_crystals,
        }
    }
}
impl Respawn {
    pub(in crate::game::server_side) fn name(self) -> &'static str {
        match self {
            Respawn::Start => "start",
            Respawn::Preparing => "preparing_to_summon_pillars",
            Respawn::Pillars => "summoning_pillars",
            Respawn::Summoning => "summoning_dragon",
        }
    }
    pub(in crate::game::server_side) fn from_name(name: &str) -> Option<Self> {
        [
            Respawn::Start,
            Respawn::Preparing,
            Respawn::Pillars,
            Respawn::Summoning,
        ]
        .into_iter()
        .find(|r| r.name() == name)
    }
    pub(super) fn api(self) -> DragonRespawnStage {
        match self {
            Respawn::Start => DragonRespawnStage::Start,
            Respawn::Preparing => DragonRespawnStage::PreparingToSummonPillars,
            Respawn::Pillars => DragonRespawnStage::SummoningPillars,
            Respawn::Summoning => DragonRespawnStage::SummoningDragon,
        }
    }
}
/// A player in the End as the dragon sees it.
#[derive(Clone, Copy, Debug)]
pub(in crate::game) struct FightPlayer {
    pub(in crate::game) who: FightWho,
    pub(in crate::game) position: Vec3,
    pub(in crate::game) bounds: Aabb,
    pub(in crate::game) alive: bool,
    pub(in crate::game) creative: bool,
    pub(in crate::game) spectator: bool,
}
impl FightPlayer {
    /// `Player.canBeSeenAsEnemy`: alive, neither creative nor spectator.
    pub(in crate::game::server_side) fn enemy(&self) -> bool {
        self.alive && !self.creative && !self.spectator
    }
}
/// `TargetingConditions.forCombat` (visibility aside) in
/// `Level.getNearestPlayer`: the enemy nearest `at` among `players`, within
/// `range` of the dragon at `from` and passing `selector`; none on Peaceful.
pub(in crate::game::server_side) fn nearest_enemy(
    players: &[FightPlayer],
    peaceful: bool,
    at: Vec3,
    range: Option<(Vec3, f64)>,
    selector: impl Fn(&FightPlayer) -> bool,
) -> Option<FightPlayer> {
    if peaceful {
        return None;
    }
    players
        .iter()
        .filter(|p| p.enemy() && selector(p))
        .filter(|p| range.is_none_or(|(from, r)| p.position.distance_squared(from) <= r * r))
        .min_by(|a, b| {
            a.position
                .distance_squared(at)
                .total_cmp(&b.position.distance_squared(at))
        })
        .copied()
}
/// Whether `EndCrystal.hurtServer` creates a blast before notifying the fight.
#[derive(Clone, Copy, PartialEq)]
pub(in crate::game) enum CrystalDamage {
    Direct,
    Explosion,
}
/// What `EnderDragon.hurt` made of a hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hurt {
    /// It returned true; `full` when it was a new hit, not just the excess
    /// over the last one (the hurt sound and animation).
    Landed { full: bool },
    /// It returned false (dying, too little damage, fire on a part).
    Refused,
    /// A perched dragon's `onHurt` took a direct arrow's or wind charge's
    /// damage away (and set it alight for a second).
    Shrugged,
}
/// The dragon's body among its parts (head, neck, body, ...).
pub(in crate::game) const BODY: usize = 2;
pub struct DragonHitOrigin<'a> {
    pub direct: Option<&'a ProjectileKind>,
    pub entity_id: Option<i32>,
    pub from: Option<Vec3>,
    pub invulnerability_checked: bool,
    pub hook_source: Option<HurtSource<'a>>,
    pub attack: Option<&'a sunrise_sim::health::Attack>,
    pub hooks: DragonHitHooks,
}
struct DragonLethalHit {
    before_health: f32,
    phase_id: i32,
    post: Option<std::sync::Arc<sunrise_sim::health::DamagePost>>,
}
enum DragonLethal {
    None,
    Completed,
    Pending(DragonLethalHit),
}
struct DragonHurt {
    hurt: Hurt,
    lethal: DragonLethal,
}
impl DragonHurt {
    const fn plain(hurt: Hurt) -> Self {
        Self {
            hurt,
            lethal: DragonLethal::None,
        }
    }
}
#[derive(Clone, Copy, Default)]
pub struct DragonHitHooks {
    invulnerability: bool,
    death: bool,
    death_post: bool,
    any: bool,
}
impl DragonHitHooks {
    pub fn capture() -> Self {
        if mod_api::__private::mods().is_empty() {
            return Self::default();
        }
        let invulnerability =
            mod_api::__private::enabled_listened(mod_api::HookId::EntityInvulnerabilityCheck);
        let death = mod_api::__private::enabled_listened(mod_api::HookId::LivingDeath);
        let death_post = mod_api::__private::enabled_listened(mod_api::HookId::LivingDeathPost);
        let any = invulnerability
            || death
            || death_post
            || [
                mod_api::HookId::LivingIncomingDamage,
                mod_api::HookId::LivingShieldBlock,
                mod_api::HookId::LivingDamagePre,
                mod_api::HookId::LivingDamagePost,
                mod_api::HookId::LivingKnockback,
            ]
            .into_iter()
            .any(mod_api::__private::enabled_listened);
        Self {
            invulnerability,
            death,
            death_post,
            any,
        }
    }
}
/// A direct `AbstractArrow` or `WindCharge`, which a perched dragon shrugs off
/// (`AbstractDragonSittingPhase.onHurt`); a breeze's `BreezeWindCharge` is neither.
pub(in crate::game::server_side) fn shrugs(direct: Option<&ProjectileKind>) -> bool {
    matches!(
        direct,
        Some(ProjectileKind::Arrow { .. } | ProjectileKind::Trident | ProjectileKind::WindCharge)
    )
}
/// `Vec3.normalize`: zero under a length of `1.0E-5F`.
pub(in crate::game::server_side) fn vanilla_normalize(v: glam::DVec3) -> glam::DVec3 {
    let len = v.length();
    if len < f64::from(1.0e-5_f32) {
        glam::DVec3::ZERO
    } else {
        v / len
    }
}

/// `DragonStrafePlayerPhase`'s firing angle: `Math.acos` of the float dot, one
/// `dmul` by `Mth.RAD_TO_DEG`, narrowed to float, then `+ 0.5F`.
pub(in crate::game::server_side) fn strafe_angle(dot: f32) -> f32 {
    (jmath::acos(f64::from(dot)) * f64::from(trig::RAD_TO_DEG)) as f32 + 0.5
}
/// `#minecraft:is_fire`, which the dragon's parts and the crystals refuse
/// (`isInvulnerableToBase`: both types are fire immune).
pub(in crate::game::server_side) fn is_fire(kind: DamageKind) -> bool {
    let source = Damage {
        kind,
        attacker: None,
        from: None,
        direct: None,
        attacker_species: None,
    };
    source.is_fire()
}
pub(in crate::game) const ADJACENCY: [u32; 24] = [
    6146, 8197, 8202, 16404, 32808, 32848, 65696, 131392, 131712, 263424, 526848, 525313, 1581057,
    3166214, 2138120, 6373424, 4358208, 12910976, 9044480, 9706496, 15216640, 13688832, 11763712,
    8257536,
];
/// A* over the dragon's 24 fixed nodes, in Minecraft's open-set order.
pub(in crate::game::server_side) fn search_dragon_path(
    nodes: &[[i32; 3]; 24],
    from: usize,
    to: usize,
    min: usize,
    last: Option<[i32; 3]>,
) -> Option<Path> {
    use sunrise_sim::mobs::path::heap::{Key, OpenSet};
    let dist = |a: [i32; 3], b: [i32; 3]| {
        (((a[0] - b[0]).pow(2) + (a[1] - b[1]).pow(2) + (a[2] - b[2]).pow(2)) as f32).sqrt()
    };
    let mut g = [f32::MAX; 24];
    let mut came = [usize::MAX; 24];
    let mut closed = [false; 24];
    let mut keys = [Key::default(); 24];
    let mut open = OpenSet::with_capacity(keys.len());
    open.insert(&mut keys, from as u32, dist(nodes[from], nodes[to]));
    g[from] = 0.0;
    let mut closest = from;
    while let Some(cur) = open.pop(&mut keys) {
        let cur = cur as usize;
        if cur == to {
            closest = to;
            break;
        }
        if dist(nodes[cur], nodes[to]) < dist(nodes[closest], nodes[to]) {
            closest = cur;
        }
        closed[cur] = true;
        for i in min..24 {
            if ADJACENCY[cur] & (1 << i) != 0 && !closed[i] {
                let tentative = g[cur] + dist(nodes[cur], nodes[i]);
                if tentative < g[i] {
                    g[i] = tentative;
                    came[i] = cur;
                    let f = tentative + dist(nodes[i], nodes[to]);
                    if keys[i].is_open() {
                        open.change_cost(&mut keys, i as u32, f);
                    } else {
                        open.insert(&mut keys, i as u32, f);
                    }
                }
            }
        }
    }
    if closest == from && from != to {
        return None;
    }
    let mut list = vec![nodes[closest]];
    let mut n = closest;
    while came[n] != usize::MAX {
        n = came[n];
        list.insert(0, nodes[n]);
    }
    if let Some(l) = last {
        list.push(l);
    }
    Some(Path {
        nodes: list,
        next: 0,
    })
}
/// `EnderDragonPart.hurtServer`: `isInvulnerableToBase` refuses fire
/// (the dragon's type is fire immune) before `EnderDragon.hurt`.
/// `direct` is the projectile that hit: a direct `AbstractArrow` or a
/// `WindCharge` is the arrow a perched dragon shrugs off, a breeze's
/// `BreezeWindCharge` is neither. Gameplay calls `hurt_part_from`; this
/// is the no-attacker form the tests use.
#[cfg(test)]
pub(in crate::game::server_side) fn hurt_part(
    d: &mut Dragon,
    level: &mut DamageLevel<'_>,
    part: usize,
    damage: f32,
    counts: Option<HurtSource<'_>>,
    kind: DamageKind,
    direct: Option<&ProjectileKind>,
) -> Hurt {
    hurt_part_from(d, level, part, damage, counts, kind, direct, None, false)
}

/// `hurt_part` with where a full hit's default knockback comes from: the melee attacker's
/// position (`DamageSource.getSourcePosition`, the direct entity's feet) and the dragon's
/// knockback resistance at the hit. `None` for a hit that is not a player's melee.
#[cfg(test)]
pub(in crate::game::server_side) fn hurt_part_from(
    d: &mut Dragon,
    level: &mut DamageLevel<'_>,
    part: usize,
    damage: f32,
    counts: Option<HurtSource<'_>>,
    kind: DamageKind,
    direct: Option<&ProjectileKind>,
    push: Option<Push>,
    invulnerability_checked: bool,
) -> Hurt {
    hurt_part_from_outcome(
        d,
        level,
        part,
        damage,
        counts,
        kind,
        direct,
        push,
        invulnerability_checked,
        false,
    )
    .hurt
}

fn hurt_part_from_outcome(
    d: &mut Dragon,
    level: &mut DamageLevel<'_>,
    part: usize,
    damage: f32,
    counts: Option<HurtSource<'_>>,
    kind: DamageKind,
    direct: Option<&ProjectileKind>,
    push: Option<Push>,
    invulnerability_checked: bool,
    defer_lethal: bool,
) -> DragonHurt {
    // The game-level invulnerability hook includes the part's fire immunity in
    // vanilla's answer. Only the standalone test adapter still needs this guard.
    if is_fire(kind) && !invulnerability_checked {
        return DragonHurt::plain(Hurt::Refused);
    }
    hurt_from_outcome(
        d,
        level,
        part,
        damage,
        counts,
        shrugs(direct),
        push,
        invulnerability_checked,
        defer_lethal,
    )
}

/// `EnderDragon.hurt(part, source, damage)`: whether the hit landed
/// (an arrow is spent, else it turns back). `counts` is the source
/// `reallyHurt` takes when `source.getEntity() instanceof Player` or its
/// type always hurts dragons (explosions), and none otherwise; `arrow` a
/// direct arrow or wind charge, which a perched dragon shrugs off
/// (`AbstractDragonSittingPhase.onHurt`).
#[cfg(test)]
pub(in crate::game::server_side) fn hurt(
    d: &mut Dragon,
    level: &mut DamageLevel<'_>,
    part: usize,
    damage: f32,
    counts: Option<HurtSource>,
    arrow: bool,
) -> Hurt {
    hurt_from(d, level, part, damage, counts, arrow, None, false)
}

/// EnderDragon.handleKillingBlow starts the dying flight with one health.
fn apply_killing_blow(d: &mut Dragon) {
    if !d.phase.sitting() {
        d.health = 1.0;
        d.set_phase(Phase::Dying {
            target: None,
            time: 0,
        });
    }
}

/// Apply the takeoff threshold for damage received while perched.
fn finish_sitting_damage(d: &mut Dragon, before: f32) {
    if d.phase.sitting() {
        d.sitting_damage += before - d.health.max(0.0);
        if d.sitting_damage > 0.25 * MAX_HEALTH {
            d.sitting_damage = 0.0;
            d.set_phase(Phase::Takeoff {
                first: true,
                path: None,
                target: None,
            });
        }
    }
}

pub(in crate::game::server_side) fn hurt_from(
    d: &mut Dragon,
    level: &mut DamageLevel<'_>,
    part: usize,
    damage: f32,
    counts: Option<HurtSource<'_>>,
    arrow: bool,
    push: Option<Push>,
    invulnerability_checked: bool,
) -> Hurt {
    hurt_from_outcome(
        d,
        level,
        part,
        damage,
        counts,
        arrow,
        push,
        invulnerability_checked,
        false,
    )
    .hurt
}

fn hurt_from_outcome(
    d: &mut Dragon,
    level: &mut DamageLevel<'_>,
    part: usize,
    damage: f32,
    counts: Option<HurtSource<'_>>,
    arrow: bool,
    push: Option<Push>,
    invulnerability_checked: bool,
    defer_lethal: bool,
) -> DragonHurt {
    if matches!(d.phase, Phase::Dying { .. }) || !d.alive() {
        return DragonHurt::plain(Hurt::Refused);
    }
    if arrow && d.phase.sitting() {
        return DragonHurt::plain(Hurt::Shrugged);
    }
    let mut damage = damage;
    if part != 0 && part != 1 {
        damage = damage / 4.0 + damage.min(1.0);
    }
    if damage < 0.01 {
        return DragonHurt::plain(Hurt::Refused);
    }
    let Some(source) = counts else {
        return DragonHurt::plain(Hurt::Landed { full: false });
    };
    if !invulnerability_checked
        && level.invulnerability_check(
            sunrise_sim::health::Hurting::Dragon(d.id as i32, source),
            false,
        )
    {
        return DragonHurt::plain(Hurt::Landed { full: false });
    }
    let before = d.health;
    // reallyHurt: LivingEntity.hurtServer, past its guards: the hit's damage sequence
    // starts, and `LivingIncomingDamageEvent` may change or cancel it. A canceled hit
    // still returns true.
    let mut container = DamageContainer::new(damage);
    let hurting = sunrise_sim::health::Hurting::Dragon(d.id as i32, source);
    // The dragon's head turns with its body and never pitches.
    let view = sunrise_world::view_direction::view_vector(0.0, d.yaw);
    let victim = mod_api::entity::health::Victim {
        position: [d.position.x, d.position.y, d.position.z],
        view: [view.x, view.y, view.z],
    };
    let body = sunrise_gameplay_common::game::native_living_body::dragon_body(d);
    if level
        .reborrow()
        .with_victim(body)
        .incoming_canceled(hurting, victim, &mut container)
    {
        return DragonHurt::plain(Hurt::Landed { full: false });
    }
    damage = container.new_damage();
    if damage < 0.0 {
        damage = 0.0;
    }
    // LivingEntity.applyItemBlocking: the dragon raises nothing, but a native
    // mod may block for it.
    if damage > 0.0 {
        let hurting = sunrise_sim::health::Hurting::Dragon(d.id as i32, source);
        let block = sunrise_sim::health::no_block(damage);
        let blocked = level.shield_block(hurting, damage, block).map_or(0.0, |b| {
            container.record_shield_damage(b.shield_damage.max(0) as f32);
            b.blocked_damage
        });
        damage -= blocked;
        container.record_block(blocked);
    }
    if !damage.is_finite() {
        damage = f32::MAX;
    }
    container.record_new_damage(damage);
    // LivingEntity.hurtServer: within the cooldown only what exceeds the last hit.
    let window = d.invulnerable > 10;
    container.record_should_cause_side_effects(!window);
    let mut dealt = if window {
        if damage <= d.last_hurt {
            return DragonHurt::plain(Hurt::Landed { full: false });
        }
        container.record_post_attack_invulnerability_ticks(d.invulnerable);
        container.reduce(
            Reduction::Invulnerability,
            damage,
            d.last_hurt,
            damage - d.last_hurt,
        )
    } else {
        damage
    };
    // LivingEntity.actuallyHurt: the dragon wears no armor and takes no effects.
    // NeoForge still subtracts the armor-stage answer from the live container amount before Pre.
    let before_armor = container.new_damage();
    dealt = container.reduce(Reduction::Armor, before_armor, before_armor - dealt, dealt);
    let hurting = sunrise_sim::health::Hurting::Dragon(d.id as i32, source);
    container.record_new_damage(dealt);
    let body = sunrise_gameplay_common::game::native_living_body::dragon_body(d);
    level
        .reborrow()
        .with_victim(body)
        .damage_pre(hurting, &mut container);
    dealt = container.capture_inflicted();
    // LivingEntity.actuallyHurt: absorption runs after Pre even with 0.0 absorption hearts.
    let (remaining, _, _) = container.absorb(dealt, 0.0);
    if remaining != 0.0 {
        d.health -= remaining;
        container.record_health_damage(remaining);
    }
    d.last_hurt = damage;
    d.invulnerable = container.post_attack_invulnerability_ticks();
    if !window {
        d.hurt_time = 10;
    }
    let full = !window;
    if !defer_lethal {
        let lethal = d.health <= 0.0;
        if lethal {
            if d.phase.sitting() {
                d.health = 0.0;
            } else {
                apply_killing_blow(d);
            }
        }
        if full && let Some(Push { from, resistance }) = push {
            let knock = Knock {
                strength: f64::from(0.4_f32),
                x: from.x - d.position.x,
                z: from.z - d.position.z,
            };
            knockback(d, level, knock, source, resistance);
        }
        let hurting = sunrise_sim::health::Hurting::Dragon(d.id as i32, source);
        let body = sunrise_gameplay_common::game::native_living_body::dragon_body(d);
        level
            .reborrow()
            .with_victim(body)
            .damage_post(hurting, &container);
        finish_sitting_damage(d, before);
        return DragonHurt {
            hurt: Hurt::Landed { full },
            lethal: if lethal {
                DragonLethal::Completed
            } else {
                DragonLethal::None
            },
        };
    }
    // Default knockback precedes death and sitting damage; explosions apply their own push.
    if full && let Some(Push { from, resistance }) = push {
        let knock = Knock {
            strength: f64::from(0.4_f32),
            x: from.x - d.position.x,
            z: from.z - d.position.z,
        };
        knockback(d, level, knock, source, resistance);
    }
    let lethal = d.health <= 0.0;
    let phase_id = d.phase.id();
    if lethal {
        d.health = 0.0;
    }
    // Defer the damage post hook on a lethal hit until the death hooks finish.
    let hurting = sunrise_sim::health::Hurting::Dragon(d.id as i32, source);
    let body = sunrise_gameplay_common::game::native_living_body::dragon_body(d);
    let completion = level
        .reborrow()
        .with_victim(body)
        .finish_damage(hurting, container, lethal);
    if lethal {
        return DragonHurt {
            hurt: Hurt::Landed { full },
            lethal: DragonLethal::Pending(DragonLethalHit {
                before_health: before,
                phase_id,
                post: completion.and_then(|c| c.post),
            }),
        };
    }
    finish_sitting_damage(d, before);
    DragonHurt::plain(Hurt::Landed { full })
}

/// `EnderDragon.knockback`: `LivingEntity.knockback` only when the phase
/// is not sitting. `noPhysics` never sets `onGround`, so the vertical component stays.
pub(in crate::game::server_side) fn knockback(
    d: &mut Dragon,
    level: &mut DamageLevel<'_>,
    mut knock: Knock,
    source: HurtSource<'_>,
    resistance: f64,
) -> bool {
    !d.phase.sitting()
        && !level.knockback_canceled(Hurting::Dragon(d.id as i32, source), &mut knock)
        && knock.apply(&mut d.velocity, resistance, false, &mut d.random)
}

/// `DragonHoldingPatternPhase.onCrystalDestroyed`: strafe `player` if
/// the dragon can attack them (`EnderDragon.canAttack`, on Peaceful too).
/// The first such crystal wins: the dragon strafes from then on, and
/// `onCrystalDestroyed` does nothing in that phase.
pub(in crate::game::server_side) fn strafe_after_crystal(
    d: &mut Dragon,
    player: Option<&FightPlayer>,
) {
    if let Some(p) = player
        && p.enemy()
        && matches!(d.phase, Phase::Holding { .. })
        && d.strafe_request.is_none()
    {
        d.strafe_request = Some(p.who);
    }
}

/// The source `reallyHurt` takes from a hit `by` the player of that entity
/// id: `DamageSources.explosion(crystal, player)` is `player_explosion` with
/// a player behind it, who is the causing entity, and the direct one of a
/// melee hit. A projectile is the direct entity only when the damage type
/// names one; `magic()` (an ownerless wither skull) does not.
pub(in crate::game::server_side) fn hurt_source(
    kind: DamageKind,
    by: Option<i32>,
    direct: Option<i32>,
) -> HurtSource<'static> {
    let causing = by.unwrap_or(0);
    let damage_type = sunrise_sim::health::damage_type(kind, by.is_some());
    let projectile = direct.filter(|_| sunrise_sim::health::names_its_projectile(damage_type));
    HurtSource {
        damage_type,
        causing,
        direct: projectile.unwrap_or(if sunrise_sim::health::melee(kind) {
            causing
        } else {
            0
        }),
        position: None,
        direct_type: None,
    }
}
/// `Mth.wrapDegrees(double)`.
pub(in crate::game::server_side) fn wrap64(d: f64) -> f64 {
    let mut r = d % 360.0;
    if r >= 180.0 {
        r -= 360.0;
    }
    if r < -180.0 {
        r += 360.0;
    }
    r
}
impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn fight_path(&self) -> std::path::PathBuf {
        sunrise_sim::save::dimension_dir(&self.server.world_root, "minecraft:the_end")
            .join(FIGHT_FILE)
    }
    /// The saved fight, or `createDefault` with the gateways shuffled by the seed.
    pub(in crate::game::server_side) fn load_fight(&mut self) {
        if self.server.fight.loaded {
            return;
        }
        let tag = sunrise_sim::save::read_dat(&self.fight_path());
        let data = tag.as_ref().and_then(|t| t.get("data"));
        let flag = |k: &str, default: bool| match data.and_then(|d| d.get(k)) {
            Some(Tag::Byte(v)) => *v != 0,
            _ => default,
        };
        let mut fight = Fight {
            loaded: true,
            needs_scan: flag("needs_state_scanning", true),
            killed: flag("dragon_killed", false),
            previously_killed: flag("previously_killed", false),
            crystals_placed: flag("sunrise_crystals_placed", false),
            gateways: match data.and_then(|d| d.get("gateways")) {
                Some(Tag::IntArray(v)) => v.clone(),
                Some(Tag::List(v)) => v
                    .iter()
                    .filter_map(|t| match t {
                        Tag::Int(i) => Some(*i),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            },
            respawn: match data.and_then(|d| d.get("respawn_stage")) {
                Some(Tag::String(name)) => {
                    Respawn::from_name(&name.to_string().unwrap_or_default()).map(|r| {
                        let time = match data.and_then(|d| d.get("respawn_time")) {
                            Some(Tag::Int(t)) => *t,
                            _ => 0,
                        };
                        (r, time)
                    })
                }
                _ => None,
            },
            respawn_crystals: match data.and_then(|d| d.get("sunrise_respawn_crystals")) {
                Some(Tag::IntArray(v)) => v
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|b| {
                        Vec3::new(
                            f64::from(b[0]) + 0.5,
                            f64::from(b[1]),
                            f64::from(b[2]) + 0.5,
                        )
                    })
                    .collect(),
                _ => Vec::new(),
            },
            exit_portal: match data.and_then(|d| d.get("exit_portal_location")) {
                Some(Tag::IntArray(v)) if v.len() == 3 => Some([v[0], v[1], v[2]]),
                _ => None,
            },
            dragon_uuid: match data.and_then(|d| d.get("dragon_uuid")) {
                Some(Tag::IntArray(v)) if v.len() == 4 => Some([v[0], v[1], v[2], v[3]]),
                _ => None,
            },
            // The first tick scans for players.
            player_scan: 21,
            ..Default::default()
        };
        if fight.gateways.is_empty() {
            // Util.shuffle(0..20, RandomSource.create(seed)).
            let mut gateways: Vec<i32> = (0..20).collect();
            let mut random = LegacyRandom::new(self.common.level.seed);
            for i in (2..=gateways.len()).rev() {
                let j = random.next_int_bound(i as i32).unwrap_or(0) as usize;
                gateways.swap(i - 1, j);
            }
            fight.gateways = gateways;
            fight.dirty = true;
        }
        // A world from before this fight existed: one with an active portal
        // already won it.
        if tag.is_none() && self.end_portal_active() {
            fight.killed = true;
            fight.previously_killed = true;
            fight.crystals_placed = true;
            fight.needs_scan = false;
        }
        self.server.fight = fight;
    }
    pub(in crate::game::server_side) fn end_portal_active(&self) -> bool {
        (0..256).any(|y| {
            &*self
                .common
                .res
                .facts()
                .get(self.common.world.block(0, y, 0))
                .name
                == "minecraft:bedrock"
                && (-3..=3).any(|d| {
                    &*self
                        .common
                        .res
                        .facts()
                        .get(self.common.world.block(d, y, 0))
                        .name
                        == "minecraft:end_portal"
                })
        })
    }
    pub(in crate::game::server_side) fn save_fight(&mut self) {
        if !self.server.fight.loaded || !self.server.fight.dirty {
            return;
        }

        let f = &self.server.fight;
        let b = |v: bool| Tag::Byte(i8::from(v));
        let mut data = vec![
            ("needs_state_scanning", b(f.needs_scan)),
            ("dragon_killed", b(f.killed)),
            ("previously_killed", b(f.previously_killed)),
            ("respawn_time", Tag::Int(f.respawn.map_or(0, |r| r.1))),
            ("gateways", Tag::IntArray(f.gateways.clone())),
            ("sunrise_crystals_placed", b(f.crystals_placed)),
        ];
        if let Some(p) = f.exit_portal {
            data.push(("exit_portal_location", Tag::IntArray(p.to_vec())));
        }
        if let Some(u) = f.dragon_uuid {
            data.push(("dragon_uuid", Tag::IntArray(u.to_vec())));
        }
        if let Some((stage, _)) = f.respawn {
            data.push(("respawn_stage", Tag::String(stage.name().into())));
            data.push((
                "sunrise_respawn_crystals",
                Tag::IntArray(
                    f.respawn_crystals
                        .iter()
                        .flat_map(|p| [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32])
                        .collect(),
                ),
            ));
        }
        let tag = Tag::compound([
            ("data", Tag::compound(data)),
            ("DataVersion", Tag::Int(sunrise_world::DATA_VERSION)),
        ]);
        if sunrise_sim::save::write_dat(&self.fight_path(), tag).is_ok() {
            self.server.fight.dirty = false;
        }
    }
    /// The island's centre chunks are lit (the arena is "loaded").
    pub(in crate::game::server_side) fn arena_loaded(&self) -> bool {
        (-2..=1).all(|cz| (-2..=1).all(|cx| self.server.streaming.is_lit(cx, cz)))
    }
    /// A player's entity id: the acting player's body has it, another
    /// player's comes from the hub (0 when no such player is available).
    pub(in crate::game::server_side) fn fight_entity(&self, who: FightWho) -> i32 {
        if who == self.acting_player() {
            return self.common.player.as_ref().map_or(0, |p| p.entity_id);
        }
        let host = self.server.lan.as_ref();
        match who {
            FightWho::Host => 0,
            FightWho::Guest(conn) => host
                .and_then(|h| h.hub.guest(conn))
                .map_or(0, |g| g.entity_id),
        }
    }
    /// The player acting now: the guest whose turn it is, else the host.
    pub fn acting_player(&self) -> FightWho {
        self.server
            .lan_acting
            .map_or(FightWho::Host, FightWho::Guest)
    }
    /// The causing player shared by the crystal/deflected-fireball blast.
    pub(in crate::game::server_side) fn crystal_blast_player(
        &self,
    ) -> Option<super::commands::Target> {
        self.server.fight.blast_by.map(|who| match who {
            FightWho::Host => super::commands::Target::Player,
            FightWho::Guest(conn) => super::commands::Target::Guest(conn),
        })
    }
    /// Names the acting player behind the blast about to go off (a large
    /// fireball they shot), or no one, for the dragon's kill credit.
    pub(in crate::game::server_side) fn set_blast_by_player(&mut self, by_player: bool) {
        self.server.fight.blast_by = by_player.then(|| self.acting_player());
    }
    /// The players in the level the dragon sees (`level.players()`): the
    /// window's own, then every guest in the level.
    pub(in crate::game) fn fight_players(&self) -> Vec<FightPlayer> {
        let mut out = Vec::new();
        if let Some(p) = self.common.player.as_ref() {
            out.push(FightPlayer {
                who: FightWho::Host,
                position: p.position,
                bounds: p.bounds(),
                alive: !p.health.dead_or_dying(),
                creative: self.common.creative,
                spectator: self.common.spectator(),
            });
        }
        if let Some(h) = self.server.lan.as_ref() {
            for g in h.hub.guests.iter().filter(|g| h.hub.here(g) && g.loaded) {
                let [x, y, z] = g.position;
                out.push(FightPlayer {
                    who: FightWho::Guest(g.conn),
                    position: Vec3::new(x, y, z),
                    bounds: super::lan_mobs::guest_bounds(g),
                    alive: !g.dead,
                    creative: g.creative(),
                    spectator: g.spectator(),
                });
            }
        }
        out
    }
    /// Run `f` as the fight's player `who` (a guest's turn, its knockback
    /// sent to its client).
    pub(in crate::game::server_side) fn as_fight_player(
        &mut self,
        who: FightWho,
        f: impl FnOnce(&mut Self),
    ) {
        match who {
            FightWho::Host => f(self),
            FightWho::Guest(conn) => {
                self.guest_hit_by(conn, f);
            }
        }
    }
    /// `EnderDragonFight.validPlayer`: the players alive within 192 blocks of
    /// (0, 128, 0), the host and the guests in the level alike.
    pub(in crate::game::server_side) fn fight_has_players(&self) -> bool {
        let near =
            |p: [f64; 3]| p[0].powi(2) + (p[1] - 128.0).powi(2) + p[2].powi(2) <= 192.0 * 192.0;
        let host = self.common.player.as_ref().is_some_and(|p| {
            !p.health.dead_or_dying() && near([p.position.x, p.position.y, p.position.z])
        });
        host || self.server.lan.as_ref().is_some_and(|h| {
            h.hub
                .guests
                .iter()
                .any(|g| h.hub.here(g) && g.loaded && !g.dead && near(g.position))
        })
    }
    /// `EnderDragonFight.tick`, before the level ticks its entities.
    pub fn tick_fight(&mut self) {
        self.load_fight();
        let mut canceled = false;
        if mod_api::__private::listened(mod_api::HookId::DragonFightTick) {
            self.native_dragon_fight_tick(self.fight_level(), &mut canceled);
        }
        // updatePlayers, every 20 ticks.
        if !canceled {
            self.server.fight.player_scan += 1;
            if self.server.fight.player_scan >= 20 {
                self.server.fight.player_scan = 0;
                self.server.fight.players = self.fight_has_players();
            }
        }
        if !self.arena_loaded() {
            return;
        }
        // World generation's, not the fight's: placed whatever a mod does.
        if !self.server.fight.crystals_placed {
            self.place_spike_crystals();
            self.server.fight.crystals_placed = true;
            self.server.fight.dirty = true;
        }
        if !canceled && self.server.fight.players {
            if self.server.fight.needs_scan {
                self.scan_state();
                self.server.fight.needs_scan = false;
                self.server.fight.dirty = true;
            }
            if self.server.fight.respawn.is_some() {
                self.tick_respawn();
            }
            if !self.server.fight.killed {
                // Without a dragon of its own, or 1200 ticks after it last
                // ticked, the fight takes the End's dragon or makes one
                // (`findOrCreateDragon`).
                let look = match self.server.fight.dragon_uuid {
                    None => true,
                    Some(_) => {
                        self.server.fight.dragon_unseen += 1;
                        self.server.fight.dragon_unseen >= 1200
                    }
                };
                if look {
                    self.find_or_create_dragon();
                    self.server.fight.dragon_unseen = 0;
                }
                self.server.fight.crystal_scan += 1;
                if self.server.fight.crystal_scan >= 100 {
                    self.count_crystals();
                }
            }
        }
        self.assign_dragon_and_crystal_ids();
    }

    fn fight_level(&self) -> mod_api::LevelRef {
        self.level_ref(self.lend_epoch(), self.common.dim.key())
    }

    pub fn assign_dragon_and_crystal_ids(&mut self) {
        // EnderDragon's constructor takes its id and its parts' from the
        // entity counter (a dragon loaded from the save gets them here too).
        if let Some(d) = self.common.dragon.as_mut()
            && d.id == 0
        {
            super::entity_server::assign_stored_id(&mut self.server.next_entity, &mut d.id, None);
            self.server.next_entity += 8;
            self.common.dragon.id_assigned();
        }
        // The spikes' crystals too (EndCrystal's constructor): an id whether
        // or not a client has tracked them yet, to be aimed at by it.
        self.assign_resident_crystal_ids();
    }
    /// EndSpikeFeature's crystals (with fire under them on the bedrock).
    pub(in crate::game::server_side) fn place_spike_crystals(&mut self) {
        for spike in sunrise_world::worldgen::end_spike::spikes_for_seed(self.common.level.seed) {
            let at = Vec3::new(
                f64::from(spike.center_x) + 0.5,
                f64::from(spike.height + 1),
                f64::from(spike.center_z) + 0.5,
            );
            if self
                .common
                .crystals
                .iter()
                .any(|c| (c.position.x - at.x).abs() < 1.0 && (c.position.z - at.z).abs() < 1.0)
            {
                continue;
            }
            let (drawn, uuid) = Crystal::own_draws();
            let time = Crystal::start_time(at, drawn, self.common.clock.still());
            // EndCrystal's constructor takes an id from the entity counter.
            self.server.next_entity += 1;
            self.common
                .persist
                .uuids
                .insert(self.server.next_entity, uuid);
            self.common
                .crystals
                .push(Crystal::new(self.server.next_entity, at, true, time));
            self.common
                .world
                .modified
                .insert((at.x.floor() as i32 >> 4, at.z.floor() as i32 >> 4));
        }
    }
    /// `updateCrystalCount`: crystals on the spike tops.
    pub(in crate::game::server_side) fn count_crystals(&mut self) {
        self.server.fight.crystal_scan = 0;
        let spikes = sunrise_world::worldgen::end_spike::spikes_for_seed(self.common.level.seed);
        self.server.fight.alive_crystals = self
            .common
            .crystals
            .iter()
            .filter(|c| {
                spikes.iter().any(|s| {
                    (c.position.x - f64::from(s.center_x) - 0.5).abs() <= f64::from(s.radius)
                        && (c.position.z - f64::from(s.center_z) - 0.5).abs() <= f64::from(s.radius)
                        && c.position.y >= f64::from(s.height)
                })
            })
            .count() as i32;
    }
    /// `EndCrystal.tick` in every dimension: the time (the spin and bob),
    /// and fire at its block where there is a dragon fight (the End).
    pub(in crate::game::server_side) fn tick_crystals(&mut self) {
        let fire = (self.dimension_type().has_ender_dragon_fight)
            .then(|| {
                self.common
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:fire")
            })
            .flatten();
        for i in 0..self.common.crystals.len() {
            self.common.crystals[i].previous = self.common.crystals[i].position;
            self.common.crystals[i].time += 1;
            self.crystal_block_effects(i);
            let p = self.common.crystals[i].position;
            let b = [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32];
            if let Some(fire) = fire
                && self
                    .common
                    .res
                    .facts()
                    .is_air(self.common.world.block(b[0], b[1], b[2]))
                && self.common.world.loaded(b[0], b[2])
            {
                let _ = self.level_set(b, fire);
            }
        }
    }
    /// `EndCrystal.hurtServer`: remove it, blast only for non-explosion damage,
    /// then notify the dragon fight even when no blast was created.
    pub(in crate::game::server_side) fn destroy_crystal(
        &mut self,
        index: usize,
        by: Option<FightWho>,
        damage: CrystalDamage,
    ) {
        if index >= self.common.crystals.len() || self.common.crystals[index].invulnerable {
            return;
        }
        let c = self.common.crystals.remove(index);
        // Read before the blast, which can take other respawn crystals.
        let respawn_crystal = self.server.fight.respawn.is_some()
            && self
                .server
                .fight
                .respawn_crystals
                .iter()
                .any(|p| p.distance_squared(c.position) < 0.01);
        if damage == CrystalDamage::Direct {
            let at = [c.position.x, c.position.y, c.position.z];
            // The blast's damage source carries the player, so the dragon
            // remembers them before the blast can kill it.
            self.server.fight.blast_by = by;
            // EndCrystal.hurtServer: ExplosionInteraction.BLOCK.
            let interaction = self
                .destroy_type(sunrise_game_data::gamerules::vanilla::BLOCK_EXPLOSION_DROP_DECAY);
            // EndCrystal is 2 by 2 blocks: its box decides `isInWater`.
            let (x, y, z) = (c.position.x, c.position.y, c.position.z);
            self.server.combat.0.explosion_in_water = self.bounds_in_water(Aabb::new(
                Vec3::new(x - 1.0, y, z - 1.0),
                Vec3::new(x + 1.0, y + 2.0, z + 1.0),
            ));
            self.server.combat.0.explosion_direct = Some((c.id, "minecraft:end_crystal"));
            let _ = self.server_explode_from(
                at,
                6.0,
                interaction,
                false,
                super::explosion::ExplosionSource::default().with_damage_direct(
                    c.id,
                    Some("minecraft:end_crystal"),
                    sunrise_sim::health::DirectEntity::Other,
                ),
            );
            self.server.combat.0.explosion_direct = None;
            self.server.combat.0.explosion_in_water = false;
            self.server.fight.blast_by = None;
        }
        self.crystal_destroyed(index, &c, by, respawn_crystal);
    }
    /// `EndCrystal.onDestroyedBy`: also called before removal by EndCrystal.kill.
    pub fn crystal_destroyed(
        &mut self,
        index: usize,
        c: &Crystal,
        by: Option<FightWho>,
        respawn_crystal: bool,
    ) {
        // `level.getDragonFight()`: only a level with a fight hears of it.
        if !self.dimension_type().has_ender_dragon_fight {
            return;
        }
        if respawn_crystal {
            self.abort_respawn();
        } else {
            // EnderDragonFight.onCrystalDestroyed.
            self.count_crystals();
            // EnderDragon.onCrystalDestroyed: the player behind the damage, else
            // the one nearest the crystal's block (`CRYSTAL_DESTROY_TARGETING`,
            // whose range `TargetingConditions.test` skips without a targeter).
            let player = by.or_else(|| {
                let p = c.position;
                let at = Vec3::new(p.x.floor(), p.y.floor(), p.z.floor());
                let peaceful = self.common.ui.difficulty == 0;
                nearest_enemy(&self.fight_players(), peaceful, at, None, |_| true).map(|p| p.who)
            });
            if let Some(d) = self.common.dragon.as_mut() {
                if d.nearest_crystal == Some(index) {
                    d.nearest_crystal = None;
                    let hooks = DragonHitHooks::capture();
                    let hook_source = hooks.any.then(|| HurtSource {
                        damage_type: if player.is_some() {
                            "minecraft:player_explosion"
                        } else {
                            "minecraft:explosion"
                        },
                        causing: player.map_or(0, |who| self.fight_entity(who)),
                        direct: c.id as i32,
                        position: Some([c.position.x, c.position.y, c.position.z]),
                        direct_type: Some("minecraft:end_crystal"),
                    });
                    self.hurt_dragon_part_at(
                        0,
                        10.0,
                        true,
                        player,
                        DamageKind::Explosion,
                        DragonHitOrigin {
                            direct: None,
                            entity_id: None,
                            from: None,
                            invulnerability_checked: false,
                            hook_source,
                            attack: None,
                            hooks,
                        },
                    );
                } else if let Some(n) = d.nearest_crystal.as_mut()
                    && *n > index
                {
                    *n -= 1;
                }
                let target =
                    player.and_then(|who| self.fight_players().into_iter().find(|p| p.who == who));
                if let Some(d) = self.common.dragon.as_mut() {
                    strafe_after_crystal(d, target.as_ref());
                }
            }
        }
        if mod_api::__private::listened(mod_api::HookId::EndCrystalDestroyed) {
            let p = c.position;
            let pos = BlockPos::new(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
            self.native_end_crystal_destroyed(self.fight_level(), pos, respawn_crystal);
        }
    }
    /// Whether `id` is a dragon part (`SPECIAL..SPECIAL + 8`) or a live End crystal.
    pub(in crate::game::server_side) fn is_special_target(&self, id: u32) -> bool {
        id >= SPECIAL || self.common.is_crystal_id(id)
    }
    /// `ServerLevel.dragonParts`: where the part with that entity id is
    /// and its box.
    pub fn dragon_part(&self, id: u32) -> Option<(Vec3, Aabb)> {
        let (d, i) = self.resident_dragon_part(id)?;
        d.alive().then(|| d.part_geometry(i))
    }
    /// Damage to a special id (from projectiles and melee): what the
    /// dragon part or crystal made of it. `direct` is the projectile that hit.
    pub fn hit_special(
        &mut self,
        id: u32,
        amount: f32,
        by_player: bool,
        kind: DamageKind,
        direct: Option<&Projectile>,
    ) -> Hurt {
        let owner = by_player.then_some(Who::Player);
        let guest = self.server.lan_acting.filter(|_| by_player);
        let hit = |game: &mut Self| {
            game.hit_special_full(
                id,
                amount,
                owner,
                guest,
                kind,
                None,
                direct.map(DirectHit::from_projectile),
            )
        };
        if id >= SPECIAL
            && mod_api::__private::enabled_listened(mod_api::HookId::LivingDeath)
            && let Some(shot) = direct
            && self.server.combat.0.attack().projectile != Some(shot.id as i32)
        {
            let attack = self.projectile_attack(shot);
            self.with_attack(attack, hit)
        } else {
            hit(self)
        }
    }
    /// Full-source damage dispatch for a crystal (real entity id) or dragon part.
    pub(in crate::game) fn hit_special_full(
        &mut self,
        id: u32,
        amount: f32,
        owner: Option<Who>,
        guest: Option<u32>,
        kind: DamageKind,
        damage_type_override: Option<&str>,
        direct: Option<DirectHit<'_>>,
    ) -> Hurt {
        if let Some(index) = self.common.crystals.iter().position(|c| c.id == id) {
            // EndCrystal.hurtServer: false while invulnerable and for fire
            // (its type is fire immune), else it blows up.
            let crystal = &self.common.crystals[index];
            let crystal_id = crystal.id;
            let by_player = owner == Some(Who::Player);
            let creative = match owner {
                Some(Who::Player) => match guest {
                    Some(conn) if self.server.lan_acting == Some(conn) => self.common.creative,
                    Some(conn) => self.command_guest(conn).is_some_and(|g| g.creative()),
                    None => self.common.creative,
                },
                _ => false,
            };
            let vanilla = is_fire(kind) || (crystal.invulnerable && !(by_player && creative));
            if !mod_api::__private::mods().is_empty()
                && mod_api::__private::enabled_listened(mod_api::HookId::EntityInvulnerabilityCheck)
            {
                let causing = match owner {
                    Some(Who::Player) => match guest {
                        Some(conn) => self.command_guest(conn).map_or(0, |g| g.entity_id),
                        None => self.own_entity_id(),
                    },
                    Some(Who::Mob(oid)) => oid as i32,
                    None => 0,
                };
                let damage_type = damage_type_override
                    .unwrap_or_else(|| sunrise_sim::health::damage_type(kind, by_player));
                let source = HurtSource {
                    damage_type,
                    causing,
                    direct: direct.as_ref().map_or(causing, |d| d.id as i32),
                    position: direct
                        .as_ref()
                        .map(|d| [d.position.x, d.position.y, d.position.z]),
                    direct_type: direct.as_ref().and_then(DirectHit::resolve_entity_type),
                };
                if self.native_nonliving_invulnerable(
                    crystal_id,
                    "minecraft:end_crystal",
                    source,
                    vanilla,
                ) {
                    return Hurt::Refused;
                }
            } else if vanilla {
                return Hurt::Refused;
            }
            let Some(index) = self.common.crystals.iter().position(|c| c.id == id) else {
                return Hurt::Refused;
            };
            let by = by_player.then(|| guest.map_or_else(|| self.acting_player(), FightWho::Guest));
            // A listener may allow a hit on an otherwise invulnerable crystal.
            self.common.crystals[index].invulnerable = false;
            self.destroy_crystal(index, by, CrystalDamage::Direct);
            Hurt::Landed { full: true }
        } else if id >= SPECIAL {
            let by_player = owner == Some(Who::Player);
            let by = by_player.then(|| guest.map_or_else(|| self.acting_player(), FightWho::Guest));
            let hooks = DragonHitHooks::capture();
            let hook_source = hooks.any.then(|| {
                let causing = match owner {
                    Some(Who::Player) => by.map_or(0, |who| self.fight_entity(who)),
                    Some(Who::Mob(oid)) => oid as i32,
                    None => 0,
                };
                let damage_type = damage_type_override
                    .unwrap_or_else(|| sunrise_sim::health::damage_type(kind, by_player));
                let (direct_id, direct_type) = match direct
                    .as_ref()
                    .filter(|_| sunrise_sim::health::names_its_projectile(damage_type))
                {
                    Some(d) => (d.id as i32, d.resolve_entity_type()),
                    None if sunrise_sim::health::melee(kind) => (causing, None),
                    None => (0, None),
                };
                HurtSource {
                    damage_type,
                    causing,
                    direct: direct_id,
                    position: direct
                        .as_ref()
                        .map(|d| [d.position.x, d.position.y, d.position.z]),
                    direct_type,
                }
            });
            let part = (id - SPECIAL) as usize;
            let from = (matches!(kind, DamageKind::PlayerAttack | DamageKind::Thorns))
                .then(|| self.common.player.as_ref().map(|p| p.position))
                .flatten();
            self.hurt_dragon_part_at(
                part,
                amount,
                by_player,
                by,
                kind,
                DragonHitOrigin {
                    direct: direct.as_ref().and_then(|d| d.kind),
                    entity_id: direct.as_ref().map(|d| d.id as i32),
                    from,
                    invulnerability_checked: false,
                    hook_source,
                    attack: direct.as_ref().and_then(|d| d.attack),
                    hooks,
                },
            )
        } else {
            Hurt::Refused
        }
    }
    /// The player behind a command's damage source (`source.getEntity() instanceof Player`).
    pub(in crate::game::server_side) fn command_player(
        &self,
        cause: Option<Target>,
    ) -> Option<FightWho> {
        match cause {
            Some(Target::Player) => Some(self.acting_player()),
            Some(Target::Guest(conn)) => Some(FightWho::Guest(conn)),
            _ => None,
        }
    }
    /// `DamageCommand` on the dragon: `EnderDragon.hurtServer` is `hurt(body, ...)`, so a
    /// quarter plus up to 1. `reallyHurt` runs only with a player behind the source or an
    /// `#is_explosion` type, and its `LivingEntity.hurtServer` refuses fire (the dragon is fire
    /// immune); `hurt` succeeds either way unless the dragon is dying or the hit is under 0.01.
    /// `by` is the direct entity, `cause` the causing one (`by` without `from`), and `from`
    /// the source position (`at`, else the direct entity's), which `dealDefaultKnockback`
    /// pushes away from for a type not in `#no_knockback`.
    pub fn command_damage_dragon(
        &mut self,
        id: u32,
        amount: f32,
        ty: &str,
        kind: DamageKind,
        (by, cause, from): (Option<Target>, Option<Target>, Option<Vec3>),
    ) -> bool {
        let player = self.command_player(cause);
        // A perched dragon shrugs off a direct arrow or wind charge (`onHurt`).
        let direct = match by {
            Some(Target::Projectile(p)) => self
                .common
                .creatures
                .projectiles
                .iter()
                .find(|q| q.id == p)
                .map(|q| q.kind.clone()),
            Some(Target::Trident(_)) => Some(ProjectileKind::Trident),
            _ => None,
        };
        // `Entity.isInvulnerableToBase` over the saved flag and window
        // (#916), with the attacker's creative mode as `isCreativePlayer`,
        // before the dragon's own borrow of the game.
        let creative = self.creative_source_of(cause.or(by));
        let saved = self.saved_refuses_damage(id, ty, creative);
        let vanilla = saved || is_fire(kind);
        if self.common.dragon.as_ref().is_none_or(|d| d.id != id) {
            return false;
        }
        let hooks = DragonHitHooks::capture();
        let (owned_direct_type, hook_source_meta) = if hooks.any {
            let attack = self.server.combat.0.attack();
            if let Some(direct_entity) = attack.direct {
                (
                    attack.direct_type.clone(),
                    Some((attack.attacker, direct_entity)),
                )
            } else {
                let causing = cause.and_then(|t| self.network_entity_id(t)).unwrap_or(0);
                let direct_entity = by.and_then(|t| self.network_entity_id(t)).unwrap_or(0);
                (
                    by.map(|t| self.target_type(t)),
                    Some((causing, direct_entity)),
                )
            }
        } else {
            (None, None)
        };
        let hook_source = hook_source_meta.map(|(causing, direct_entity)| HurtSource {
            damage_type: ty,
            causing,
            direct: direct_entity,
            position: from.map(|p| [p.x, p.y, p.z]),
            direct_type: owned_direct_type.as_deref(),
        });
        // The world's tags, so a datapack type is classed as vanilla's are.
        let explosion = self.damage_in_tag(ty, "minecraft:is_explosion");
        let no_knockback = self.damage_in_tag(ty, "minecraft:no_knockback");
        let reaches_base = self.common.dragon.as_ref().is_some_and(|d| {
            d.reaches_base_check(
                BODY,
                amount,
                player.is_some() || explosion,
                shrugs(direct.as_ref()),
            )
        });
        let invulnerable = if reaches_base {
            if hooks.invulnerability
                && let Some(source) = hook_source
            {
                let level = super::entity_server::damage_level_of!(
                    self,
                    self.server.combat.0.attack(),
                    no_push,
                    None
                );
                level.invulnerability_check(
                    sunrise_sim::health::Hurting::Dragon(id as i32, source),
                    vanilla,
                )
            } else {
                vanilla
            }
        } else {
            false
        };
        if invulnerable {
            // Before the hook, this command treated an otherwise valid fire
            // hit as accepted while the dragon's fire immunity absorbed it.
            if is_fire(kind) && !saved {
                let Some(d) = self.common.dragon.as_mut().filter(|d| d.id == id) else {
                    return false;
                };
                let level = super::entity_server::damage_level_of!(
                    self,
                    self.server.combat.0.attack(),
                    no_push,
                    None
                );
                return matches!(
                    hurt_from(
                        d,
                        level,
                        BODY,
                        amount,
                        None,
                        shrugs(direct.as_ref()),
                        None,
                        true
                    ),
                    Hurt::Landed { .. }
                );
            }
            return false;
        }
        let Some(d) = self.common.dragon.as_mut().filter(|d| d.id == id) else {
            return false;
        };
        let counts = player.is_some() || explosion;
        // With no source position the push has no direction (knockback(0.4, 0, 0)).
        let knock = (!no_knockback).then(|| from.unwrap_or(d.position));
        let entity_id = match by {
            Some(Target::Projectile(id) | Target::Trident(id)) => Some(id as i32),
            _ => None,
        };
        let hurt = self.hurt_dragon_part_at(
            BODY,
            amount,
            counts,
            player,
            kind,
            DragonHitOrigin {
                direct: direct.as_ref(),
                entity_id,
                from: knock,
                invulnerability_checked: true,
                hook_source,
                attack: None,
                hooks,
            },
        );
        // A shrugged-off arrow is `onHurt`'s 0: under 0.01, refused.
        matches!(hurt, Hurt::Landed { .. })
    }
    /// `DamageCommand` on an End crystal: `EndCrystal.hurtServer`. `isInvulnerableToBase`
    /// refuses an `Invulnerable` crystal unless the type bypasses it or a creative player is
    /// behind it, and fire; the dragon's own damage is refused too. Else the crystal is removed,
    /// and blows up unless the damage is an explosion.
    pub(in crate::game::server_side) fn command_damage_crystal(
        &mut self,
        id: u32,
        ty: &str,
        kind: DamageKind,
        direct: Option<Target>,
        cause: Option<Target>,
        position: Option<[f64; 3]>,
    ) -> bool {
        let Some(index) = self.common.crystals.iter().position(|c| c.id == id) else {
            return false;
        };
        let creative = cause
            .filter(|t| matches!(t, Target::Player | Target::Guest(_)))
            .and_then(|t| self.for_player(t, |g| g.creative))
            .unwrap_or(false);
        let bypass = self.damage_in_tag(ty, "minecraft:bypasses_invulnerability") || creative;
        let vanilla = (self.common.crystals[index].invulnerable && !bypass) || is_fire(kind);
        let entity_id = |target| match target {
            Some(Target::Player) => self.own_entity_id(),
            Some(Target::Guest(conn)) => self.command_guest(conn).map_or(0, |g| g.entity_id),
            Some(target) => sunrise_gameplay_common::game::commands_common::target_id(target)
                .map_or(0, |id| id as i32),
            None => 0,
        };
        let source = HurtSource {
            damage_type: ty,
            causing: entity_id(cause),
            direct: entity_id(direct),
            position,
            direct_type: None,
        };
        if self.native_nonliving_invulnerable(id, "minecraft:end_crystal", source, vanilla)
            || matches!(cause, Some(Target::Dragon(_)))
        {
            return false;
        }
        // destroy_crystal keeps a crystal invulnerable to everything else.
        self.common.crystals[index].invulnerable = false;
        let damage = if self.damage_in_tag(ty, "minecraft:is_explosion") {
            CrystalDamage::Explosion
        } else {
            CrystalDamage::Direct
        };
        let player = self.command_player(cause);
        self.destroy_crystal(index, player, damage);
        true
    }
    /// `hurt_dragon_part` with no projectile behind the hit.
    #[cfg(any(test, feature = "test-support"))]
    pub fn hurt_dragon(
        &mut self,
        part: usize,
        damage: f32,
        counts: bool,
        by: Option<FightWho>,
        kind: DamageKind,
    ) -> Hurt {
        self.hurt_dragon_part(part, damage, counts, by, kind, None)
    }
    /// `EnderDragonPart.hurtServer` on the dragon in the level. `by` is the
    /// player behind the hit, remembered for the kill credit.
    #[cfg(any(test, feature = "test-support"))]
    fn hurt_dragon_part(
        &mut self,
        part: usize,
        damage: f32,
        counts: bool,
        by: Option<FightWho>,
        kind: DamageKind,
        direct: Option<&Projectile>,
    ) -> Hurt {
        // `dealDefaultKnockback` for a player's melee: away from the attacker's
        // feet. The scope names that player to the knockback hook.
        let from = (matches!(kind, DamageKind::PlayerAttack | DamageKind::Thorns))
            .then(|| self.common.player.as_ref().map(|p| p.position))
            .flatten();
        let hooks = DragonHitHooks::capture();
        let hit = |game: &mut Self| {
            game.hurt_dragon_part_at(
                part,
                damage,
                counts,
                by,
                kind,
                DragonHitOrigin {
                    direct: direct.map(|p| &p.kind),
                    entity_id: direct.map(|p| p.id as i32),
                    from,
                    invulnerability_checked: false,
                    hook_source: None,
                    attack: None,
                    hooks,
                },
            )
        };
        if hooks.death
            && let Some(shot) = direct
            && self.server.combat.0.attack().projectile != Some(shot.id as i32)
        {
            let attack = self.projectile_attack(shot);
            self.with_attack(attack, hit)
        } else {
            hit(self)
        }
    }
    /// The dragon's `getAttributeValue(KNOCKBACK_RESISTANCE)` now, which both knockback entries
    /// read: its saved attributes over the type's default. 0 without a dragon.
    pub fn dragon_knockback_resistance(&self) -> f64 {
        self.common
            .dragon
            .as_ref()
            .and_then(|d| {
                d.live.read(
                    &self.common.motion.attributes,
                    "minecraft:ender_dragon",
                    "minecraft:knockback_resistance",
                    sunrise_world::attribute_map::ValueKind::Computed,
                    Vec::new,
                )
            })
            .unwrap_or(0.0)
    }
    /// `hurt_dragon_part` with the position a full hit's knockback pushes away from.
    pub fn hurt_dragon_part_at(
        &mut self,
        part: usize,
        damage: f32,
        counts: bool,
        by: Option<FightWho>,
        kind: DamageKind,
        origin: DragonHitOrigin<'_>,
    ) -> Hurt {
        let by_id = by.map(|who| self.fight_entity(who));
        let base_source = hurt_source(kind, by_id, origin.entity_id);
        let hook_source = origin.hook_source.unwrap_or(base_source);
        let base_count_source = counts.then_some(base_source);
        let hook_count_source = counts.then_some(hook_source);
        // Part fire immunity is independent of LivingEntity's base check.
        // Commands call the dragon itself and have already checked that base.
        if is_fire(kind) && !origin.invulnerability_checked {
            return Hurt::Refused;
        }
        let death_listened = origin.hooks.death;
        let post_listened = origin.hooks.death_post;
        let defer_lethal = death_listened || post_listened;
        let hit_attack = if death_listened {
            let base_attack = origin
                .attack
                .unwrap_or_else(|| self.server.combat.0.attack());
            let has_attack = origin.attack.is_some()
                || base_attack.direct.is_some()
                || (hook_source.direct != 0 && base_attack.projectile == Some(hook_source.direct))
                || (hook_source.causing != 0 && base_attack.attacker == hook_source.causing);
            (!has_attack
                && counts
                && hook_source.causing != 0
                && hook_source.direct == hook_source.causing)
                .then(|| self.attack_of(hook_source.causing))
        } else {
            None
        };
        if !origin.invulnerability_checked
            && self
                .common
                .dragon
                .as_ref()
                .is_some_and(|d| d.reaches_base_check(part, damage, counts, shrugs(origin.direct)))
        {
            let Some(id) = self.common.dragon.as_ref().map(|d| d.id) else {
                return Hurt::Refused;
            };
            let attacker = by.map(|who| match who {
                FightWho::Host => Target::Player,
                FightWho::Guest(conn) => Target::Guest(conn),
            });
            let creative = self.creative_source_of(attacker);
            let vanilla =
                self.saved_refuses_damage(id, base_source.damage_type, creative) || is_fire(kind);
            if origin.hooks.invulnerability {
                let attack = hit_attack
                    .as_ref()
                    .or(origin.attack)
                    .unwrap_or_else(|| self.server.combat.0.attack());
                let level = super::entity_server::damage_level_of!(self, attack, no_push, None);
                if level.invulnerability_check(
                    sunrise_sim::health::Hurting::Dragon(id as i32, hook_source),
                    vanilla,
                ) {
                    return Hurt::Refused;
                }
            } else if vanilla {
                return Hurt::Refused;
            }
        }
        let weapon = death_listened
            .then(|| {
                hit_attack
                    .as_ref()
                    .or(origin.attack)
                    .unwrap_or_else(|| self.server.combat.0.attack())
                    .weapon_of(hook_source.direct)
                    .cloned()
            })
            .flatten();
        let plan = {
            let attack = hit_attack
                .as_ref()
                .or(origin.attack)
                .unwrap_or_else(|| self.server.combat.0.attack());
            self.prepare_knockback_credit(
                self.common.dragon.as_ref().map_or(0, |d| d.id) as i32,
                None,
                attack,
            )
        };
        // `getAttributeValue(KNOCKBACK_RESISTANCE)` as the push reads it, before the hit.
        let push = origin.from.map(|from| Push {
            from,
            resistance: self.dragon_knockback_resistance(),
        });
        let dim = self.common.dim;
        let (id, join, DragonHurt { hurt, lethal }) = {
            let mut credit = plan.lend(&mut self.server.last_hurt_by_player);
            let Some(d) = self.common.dragon.as_mut() else {
                return Hurt::Refused;
            };
            let (id, join) = (d.id, d.join);
            let (cooldown, last_hurt) = (d.invulnerable, d.last_hurt);
            let attack = hit_attack
                .as_ref()
                .or(origin.attack)
                .unwrap_or_else(|| self.server.combat.0.attack());
            let level = super::entity_server::damage_level_of!(self, attack, &mut credit, None);
            let outcome = hurt_part_from_outcome(
                d,
                level,
                part,
                damage,
                hook_count_source,
                kind,
                origin.direct,
                push,
                true,
                defer_lethal,
            );
            // LivingEntity.hurtServer: damage that is applied sets lastHurtByPlayer
            // (100 ticks); a hit the damage cooldown absorbs returns before that.
            if counts
                && matches!(outcome.hurt, Hurt::Landed { .. })
                && (d.invulnerable != cooldown || d.last_hurt != last_hurt)
                && let Some(who) = by
            {
                d.hurt_by_player = Some((who, d.ticks));
            }
            (id, join, outcome)
        };
        let same_dragon = |game: &Self| {
            game.common.dim == dim
                && !game.target_removed(&Target::Dragon(id))
                && game
                    .common
                    .dragon
                    .as_ref()
                    .is_some_and(|d| d.id == id && d.join == join)
        };
        let revalidated = match lethal {
            DragonLethal::None => false,
            DragonLethal::Completed => {
                self.server.fight.last_blow = Some((kind, by.is_some()));
                self.credit_dragon_kill(false);
                false
            }
            DragonLethal::Pending(lethal) => {
                // A death veto runs before the killing blow and kill credit; damage post runs last.
                let apply_death = |game: &mut Self| -> Option<Option<u128>> {
                    if !same_dragon(game)
                        || game.common.dragon.as_ref().is_none_or(|d| {
                            d.alive()
                                || d.death_canceled
                                || d.phase.id() != lethal.phase_id
                                || matches!(d.phase, Phase::Dying { .. })
                        })
                    {
                        return None;
                    }
                    if let Some(d) = game.common.dragon.as_mut() {
                        apply_killing_blow(d);
                    }
                    game.server.fight.last_blow = Some((kind, by.is_some()));
                    Some(game.credit_dragon_kill(post_listened))
                };
                let credited = if death_listened {
                    self.storage_scope(|game, _| {
                        let credited =
                            if game.dragon_death_canceled(id, &hook_source, weapon.as_ref()) {
                                if same_dragon(game)
                                    && let Some(d) = game.common.dragon.as_mut()
                                {
                                    d.death_canceled = !d.alive();
                                    if !d.death_canceled {
                                        d.death_time = 0;
                                    }
                                }
                                None
                            } else {
                                apply_death(game)
                            };
                        (credited, true)
                    })
                } else {
                    apply_death(self)
                };
                if let Some(credited_player) = credited
                    && post_listened
                {
                    // The attacker is the mob that dealt it, 0 for none or a player.
                    let attacker = if by.is_some() {
                        0
                    } else {
                        u32::try_from(hook_source.causing).unwrap_or(0)
                    };
                    self.native_living_death_post(mod_api::__private::LivingDeathPostCall {
                        entity: id,
                        kind: "minecraft:ender_dragon",
                        damage_type: hook_source.damage_type,
                        attacker,
                        player: None,
                        credited_player,
                        message_key: None,
                    });
                }
                if let Some(post) = lethal.post {
                    self.finish_damage_post(id as i32, "minecraft:ender_dragon", post);
                }
                if same_dragon(self)
                    && let Some(d) = self.common.dragon.as_mut()
                    && d.phase.id() == lethal.phase_id
                {
                    finish_sitting_damage(d, lethal.before_health);
                }
                true
            }
        };
        if revalidated && !same_dragon(self) {
            return hurt;
        }
        let Some(d) = self.common.dragon.as_mut() else {
            return hurt;
        };
        if hurt == (Hurt::Landed { full: true }) {
            // Mob.playHurtSound: the ambient sound clock starts again.
            d.ambient_time = -AMBIENT_INTERVAL;
            let at = [d.position.x, d.position.y, d.position.z];
            if let Some(s) = base_count_source {
                d.last_source = Some((
                    s.damage_type,
                    (s.causing != 0).then_some(s.causing),
                    (s.direct != 0).then_some(s.direct),
                ));
            } else {
                d.last_source = Some(("minecraft:generic", None, None));
            }
            let pitch = 0.8 + self.common.world.random.next_float() * 0.3;
            self.server_play_sound("minecraft:entity.ender_dragon.hurt", at, 5.0, pitch);
            if kind.is("minecraft:thorns") {
                self.server_play_sound("minecraft:enchant.thorns.hit", at, 1.0, 1.0);
            }
        }
        hurt
    }
    /// `LivingEntity.die`: `getKillCredit().awardKillScore`, for the player
    /// who hurt the dragon within the last 100 ticks. A guest's kill is
    /// theirs: their statistics, scores and advancements.
    fn credit_dragon_kill(&mut self, resolve_uuid: bool) -> Option<u128> {
        let d = self.common.dragon.as_ref()?;
        let (who, _) = d.hurt_by_player.filter(|(_, hit)| d.ticks - hit <= 100)?;
        let view = sunrise_sim::advancements::EntityView {
            kind: "minecraft:ender_dragon".to_owned(),
            position: [d.position.x, d.position.y, d.position.z],
            ..Default::default()
        };
        let id = d.id;
        let blow = self.server.fight.last_blow.take();
        let guest = match who {
            // Another guest than the one acting: their state, as for a mob's kill.
            FightWho::Guest(conn) if self.server.lan_acting != Some(conn) => Some(conn),
            _ => None,
        };
        if let Some(conn) = guest
            && let Some(uuid) = self.as_guest(conn, |g| {
                g.award_dragon_kill(id, &view, blow);
                resolve_uuid.then(|| g.server.lan_acting_uuid.unwrap_or_else(|| g.own_uuid()))
            })
        {
            return uuid;
        }
        self.award_dragon_kill(id, &view, blow);
        resolve_uuid.then(|| {
            self.server
                .lan_acting_uuid
                .unwrap_or_else(|| self.own_uuid())
        })
    }
    /// `ServerPlayer.awardKillScore` for the dragon, on the killer's state.
    pub(in crate::game::server_side) fn award_dragon_kill(
        &mut self,
        id: u32,
        view: &sunrise_sim::advancements::EntityView,
        blow: Option<(DamageKind, bool)>,
    ) {
        self.award_stat("killed", "minecraft:ender_dragon", 1);
        self.award_custom_stat("mob_kills", 1);
        self.score_player_killed(super::commands::Target::Mob(id));
        // KilledTrigger: the damage source `die` was given.
        let blow = blow.map(|(kind, by_player)| {
            let mut view = self.damage_view(Damage {
                kind,
                attacker: by_player.then_some(Who::Player),
                from: None,
                direct: None,
                attacker_species: None,
            });
            // `damageSources().explosion(crystal, player)`: the crystal is the direct entity.
            if kind == DamageKind::Explosion {
                view.direct = Some(sunrise_sim::advancements::EntityView {
                    kind: "minecraft:end_crystal".to_owned(),
                    ..Default::default()
                });
                view.is_direct = false;
            }
            view
        });
        self.trigger_killed(view, blow.as_ref());
    }

    /// The 24 nodes (`findClosestNode` builds them on first use).
    pub(in crate::game::server_side) fn dragon_nodes(&mut self) -> [[i32; 3]; 24] {
        if let Some(n) = self.common.dragon.as_ref().and_then(|d| d.nodes) {
            return n;
        }
        let mut nodes = [[0; 3]; 24];
        for (i, node) in nodes.iter_mut().enumerate() {
            let (r, step, base, add) = if i < 12 {
                (60.0f32, std::f32::consts::PI / 12.0, i, 5)
            } else if i < 20 {
                (40.0, std::f32::consts::PI / 8.0, i - 12, 15)
            } else {
                (20.0, std::f32::consts::PI / 4.0, i - 20, 5)
            };
            let a = 2.0 * (-std::f32::consts::PI + step * base as f32);
            let (x, z) = ((r * a.cos()).floor() as i32, (r * a.sin()).floor() as i32);
            let y = (self.common.motion_blocking_no_leaves(x, z) + add).max(73);
            *node = [x, y, z];
        }
        if let Some(d) = self.common.dragon.as_mut() {
            d.nodes = Some(nodes);
        }
        nodes
    }
    pub(in crate::game::server_side) fn closest_node(&mut self, x: f64, y: f64, z: f64) -> usize {
        let nodes = self.dragon_nodes();
        let start = if self.server.fight.alive_crystals == 0 {
            12
        } else {
            0
        };
        let p = [x.floor() as i32, y.floor() as i32, z.floor() as i32];
        let mut best = (10_000.0f32, 0);
        for (i, n) in nodes.iter().enumerate().skip(start) {
            let d = ((n[0] - p[0]).pow(2) + (n[1] - p[1]).pow(2) + (n[2] - p[2]).pow(2)) as f32;
            if d < best.0 {
                best = (d, i);
            }
        }
        best.1
    }
    /// `findPath` (A* over the adjacency masks), optionally ending at `last`.
    pub(in crate::game::server_side) fn dragon_path(
        &mut self,
        from: usize,
        to: usize,
        last: Option<[i32; 3]>,
    ) -> Option<Path> {
        let nodes = self.dragon_nodes();
        let min = if self.server.fight.alive_crystals == 0 {
            12
        } else {
            0
        };
        search_dragon_path(&nodes, from, to, min, last)
    }
    pub(in crate::game::server_side) fn next_target(
        &mut self,
        path: &mut Option<PathBox>,
    ) -> Option<Vec3> {
        let p = path.as_mut()?;
        if p.0.done() {
            return None;
        }
        let c = p.0.next_pos();
        p.0.advance();
        let d = self.common.dragon.as_mut()?;
        let y = loop {
            let y = f64::from(c[1]) + f64::from(d.random.next_float() * 20.0);
            if y >= f64::from(c[1]) {
                break y;
            }
        };
        Some(Vec3::new(f64::from(c[0]), y, f64::from(c[2])))
    }
    /// The holding-pattern style path step: around the outer or inner ring.
    pub(in crate::game::server_side) fn ring_path(&mut self, clockwise: &mut bool) -> Option<Path> {
        let d = self.common.dragon.as_mut()?;
        let flip = d.random.next_int_bound(8).unwrap_or(1) == 0;
        let current = {
            let p = d.position;
            self.closest_node(p.x, p.y, p.z)
        };
        let mut target = current as i32;
        if flip {
            *clockwise = !*clockwise;
            target += 6;
        }
        target += if *clockwise { 1 } else { -1 };
        if self.server.fight.alive_crystals > 0 {
            target = target.rem_euclid(12);
        } else {
            target = ((target - 12) & 7) + 12;
        }
        let mut path = self.dragon_path(current, target as usize, None)?;
        path.advance();
        Some(path)
    }
    /// `DragonStrafePlayerPhase.setTarget`: a path to the player at `p`.
    pub(in crate::game::server_side) fn strafe_begin(&mut self, p: Vec3) {
        let Some(d) = self.common.dragon.as_ref() else {
            return;
        };
        let dp = d.position;
        let current = self.closest_node(dp.x, dp.y, dp.z);
        let target = self.closest_node(p.x, p.y, p.z);
        let (fx, fz) = (p.x.floor() as i32, p.z.floor() as i32);
        let sd = sunrise_core::geometry::length(f64::from(fx) - dp.x, f64::from(fz) - dp.z);
        let ho = (f64::from(0.4_f32) + sd / 80.0 - 1.0).min(10.0);
        let last = [fx, (p.y + ho).floor() as i32, fz];
        let mut path = self.dragon_path(current, target, Some(last));
        if let Some(path) = path.as_mut() {
            path.advance();
        }
        let mut boxed = path.map(PathBox);
        let target = self.next_target(&mut boxed);
        if let Some(Dragon {
            phase: Phase::Strafe {
                path, target: t, ..
            },
            ..
        }) = self.common.dragon.as_mut()
        {
            *path = boxed;
            *t = target;
        }
    }
    /// One phase server tick (`doServerTick`).
    pub(in crate::game::server_side) fn tick_phase(&mut self) {
        let players = self.fight_players();
        let peaceful = self.common.ui.difficulty == 0;
        // A crystal destroyed since: strafe the player it named.
        if let Some(d) = self.common.dragon.as_mut()
            && let Some(who) = d.strafe_request.take()
            && matches!(d.phase, Phase::Holding { .. })
        {
            let target = players.iter().find(|p| p.who == who).copied();
            if let Some(t) = target {
                d.set_phase(Phase::Strafe {
                    charge: 0,
                    path: None,
                    target: None,
                    clockwise: false,
                    player: Some(t.who),
                });
                self.strafe_begin(t.position);
            }
        }
        let Some(d) = self.common.dragon.as_ref() else {
            return;
        };
        let pos = d.position;
        let mut phase = d.phase.clone();
        let collided = false;
        let far = |t: Option<Vec3>| {
            t.map_or(0.0, |t| {
                (t.x - pos.x).powi(2) + (t.y - pos.y).powi(2) + (t.z - pos.z).powi(2)
            })
        };
        let podium = self.common.podium_top();
        // The egg's place (`getHeightmapPos` at the podium) and its centre.
        let egg = Vec3::new(0.0, podium.y, 0.0);
        let egg_centre = Vec3::new(0.5, podium.y + 0.5, 0.5);
        let mut next: Option<Phase> = None;
        // Where the player a new strafe goes after stands.
        let mut strafe_at: Option<Vec3> = None;
        match &mut phase {
            Phase::Holding {
                path,
                target,
                clockwise,
            } => {
                let dt = far(*target);
                if dt < 100.0 || dt > 22500.0 || collided {
                    let done = path.as_ref().is_some_and(|p| p.0.done());
                    let mut chosen = false;
                    if done {
                        let crystals = self.server.fight.alive_crystals;
                        let r = &mut self.common.dragon.as_mut().expect("dragon").random;
                        if r.next_int_bound(crystals + 3).unwrap_or(1) == 0 {
                            next = Some(Phase::LandingApproach {
                                path: None,
                                target: None,
                            });
                            chosen = true;
                        } else if let Some(nearest) =
                            nearest_enemy(&players, peaceful, egg, None, |_| true)
                        {
                            let dist = nearest.position.distance_squared(egg_centre) / 512.0;
                            let r = &mut self.common.dragon.as_mut().expect("dragon").random;
                            if r.next_int_bound((dist + 2.0) as i32).unwrap_or(1) == 0
                                || r.next_int_bound(crystals + 2).unwrap_or(1) == 0
                            {
                                next = Some(Phase::Strafe {
                                    charge: 0,
                                    path: None,
                                    target: None,
                                    clockwise: false,
                                    player: Some(nearest.who),
                                });
                                strafe_at = Some(nearest.position);
                                chosen = true;
                            }
                        }
                    }
                    if !chosen {
                        if path.as_ref().is_none_or(|p| p.0.done()) {
                            *path = self.ring_path(clockwise).map(PathBox);
                        }
                        if let Some(t) = self.next_target(path) {
                            *target = Some(t);
                        }
                    }
                }
            }
            Phase::Strafe {
                charge,
                path,
                target,
                clockwise,
                player,
            } => {
                // Its target as it is now, whatever became of it; gone from
                // the level, or never found: back to the holding pattern.
                let tracked = player.and_then(|who| players.iter().find(|p| p.who == who));
                let Some(pp) = tracked.map(|p| p.position) else {
                    next = Some(Phase::Holding {
                        path: None,
                        target: None,
                        clockwise: false,
                    });
                    self.common.dragon.as_mut().expect("dragon").phase = phase;
                    if let Some(n) = next {
                        self.common.dragon.as_mut().expect("dragon").set_phase(n);
                    }
                    return;
                };
                if path.as_ref().is_some_and(|p| p.0.done()) {
                    let dist = sunrise_core::geometry::length(pp.x - pos.x, pp.z - pos.z);
                    let ho = (f64::from(0.4_f32) + dist / 80.0 - 1.0).min(10.0);
                    *target = Some(Vec3::new(pp.x, pp.y + ho, pp.z));
                }
                let dt = far(*target);
                if dt < 100.0 || dt > 22500.0 {
                    if path.as_ref().is_none_or(|p| p.0.done()) {
                        *path = self.ring_path(clockwise).map(PathBox);
                    }
                    if let Some(t) = self.next_target(path) {
                        *target = Some(t);
                    }
                }
                let d2 = (pp.x - pos.x).powi(2) + (pp.y - pos.y).powi(2) + (pp.z - pos.z).powi(2);
                // Entity.hasLineOfSight: eye to eye (the dragon's eye is 0.85 of
                // its 8 block height), blocks only.
                let sees = d2 < 4096.0 && {
                    let eye = Vec3::new(pos.x, pos.y + 6.8, pos.z);
                    let target_eye =
                        tracked.map_or(pp.y + 1.62, |p| p.bounds.max_position().y - 0.18);
                    self.mob_world_base(&[])
                        .line_of_sight(eye, Vec3::new(pp.x, target_eye, pp.z))
                };
                if sees {
                    *charge += 1;
                    let d = self.common.dragon.as_ref().expect("dragon");
                    let r = f64::from(d.yaw * trig::DEG_TO_RAD);
                    let dir = vanilla_normalize(glam::DVec3::new(
                        f64::from(trig::sin(r)),
                        0.0,
                        f64::from(-trig::cos(r)),
                    ));
                    let aim = vanilla_normalize(glam::DVec3::new(pp.x - pos.x, 0.0, pp.z - pos.z));
                    let angle = strafe_angle(dir.dot(aim) as f32);
                    if *charge >= 5 && (0.0..10.0).contains(&angle) {
                        // From the head, one block back along the view vector.
                        let head = d.parts[0].0;
                        let view = super::view_vector(d.yaw, 0.0);
                        let start =
                            Vec3::new(head.x - view[0], head.y + 0.5 + 0.5, head.z - view[2]);
                        let to = Vec3::new(pp.x - start.x, pp.y + 0.9 - start.y, pp.z - start.z);
                        self.spawn_dragon_fireball(start, to);
                        *charge = 0;
                        next = Some(Phase::Holding {
                            path: None,
                            target: None,
                            clockwise: false,
                        });
                    }
                } else if *charge > 0 {
                    *charge -= 1;
                }
            }
            Phase::LandingApproach { path, target } => {
                let dt = far(*target);
                if dt < 100.0 || dt > 22500.0 || collided {
                    if path.as_ref().is_none_or(|p| p.0.done()) {
                        let current = self.closest_node(pos.x, pos.y, pos.z);
                        // NEAR_EGG_TARGETING: the enemy nearest the egg.
                        let near_egg = nearest_enemy(&players, peaceful, egg, None, |_| true);
                        let target_node = match near_egg.map(|p| p.position) {
                            Some(pp) => {
                                let aim = glam::DVec2::new(pp.x, pp.z).normalize_or_zero();
                                self.closest_node(-aim.x * 40.0, 105.0, -aim.y * 40.0)
                            }
                            None => self.closest_node(40.0, podium.y, 0.0),
                        };
                        let last = [
                            podium.x.floor() as i32,
                            podium.y as i32,
                            podium.z.floor() as i32,
                        ];
                        let mut p = self.dragon_path(current, target_node, Some(last));
                        if let Some(p) = p.as_mut() {
                            p.advance();
                        }
                        *path = p.map(PathBox);
                    }
                    if let Some(t) = self.next_target(path) {
                        *target = Some(t);
                    }
                    if path.as_ref().is_some_and(|p| p.0.done()) {
                        next = Some(Phase::Landing { target: None });
                    }
                }
            }
            Phase::Landing { target } => {
                let t = *target.get_or_insert(podium);
                if (t.x - pos.x).powi(2) + (t.y - pos.y).powi(2) + (t.z - pos.z).powi(2) < 1.0 {
                    self.common.dragon.as_mut().expect("dragon").flame_count = 0;
                    next = Some(Phase::Scanning { time: 0 });
                }
            }
            Phase::Takeoff {
                first,
                path,
                target,
            } => {
                if !*first && path.is_some() {
                    if (podium.x - pos.x).powi(2)
                        + (podium.y - pos.y).powi(2)
                        + (podium.z - pos.z).powi(2)
                        >= 100.0
                    {
                        next = Some(Phase::Holding {
                            path: None,
                            target: None,
                            clockwise: false,
                        });
                    }
                } else {
                    *first = false;
                    let current = self.closest_node(pos.x, pos.y, pos.z);
                    let look = self.common.dragon_look();
                    let mut t = self.closest_node(-look[0] * 40.0, 105.0, -look[2] * 40.0) as i32;
                    if self.server.fight.alive_crystals > 0 {
                        t = t.rem_euclid(12);
                    } else {
                        t = ((t - 12) & 7) + 12;
                    }
                    let mut p = self.dragon_path(current, t as usize, None).map(PathBox);
                    if let Some(pb) = p.as_mut() {
                        pb.0.advance();
                    }
                    if let Some(t) = self.next_target(&mut p) {
                        *target = Some(t);
                    }
                    *path = p.or(Some(PathBox(Path::default())));
                }
            }
            Phase::Scanning { time } => {
                *time += 1;
                // scanTargeting: within 20 blocks, at most 10 above or below.
                let near = nearest_enemy(&players, peaceful, pos, Some((pos, 20.0)), |p| {
                    (p.position.y - pos.y).abs() <= 10.0
                })
                .map(|p| p.position);
                if let Some(pp) = near {
                    if *time > 25 {
                        next = Some(Phase::Attacking { ticks: 0 });
                    } else {
                        let d = self.common.dragon.as_mut().expect("dragon");
                        let head = d.parts[0].0;
                        scanning_turn(&mut d.yaw, &mut d.yaw_accel, d.position, head, pp);
                    }
                } else if *time >= 100 {
                    // CHARGE_TARGETING: within 150 blocks.
                    let charge =
                        nearest_enemy(&players, peaceful, pos, Some((pos, 150.0)), |_| true);
                    next = Some(match charge {
                        Some(p) => Phase::Charging {
                            target: Some(p.position),
                            since: 0,
                        },
                        None => Phase::Takeoff {
                            first: true,
                            path: None,
                            target: None,
                        },
                    });
                }
            }
            Phase::Attacking { ticks } => {
                // The growl is `doClientTick`'s (`remote_dragon_phase_tick`).
                *ticks += 1;
                if *ticks > 40 {
                    next = Some(Phase::Flaming { ticks: 0 });
                }
            }
            Phase::Flaming { ticks } => {
                *ticks += 1;
                if *ticks >= 200 {
                    let count = self.common.dragon.as_ref().map_or(0, |d| d.flame_count);
                    next = Some(if count >= 4 {
                        Phase::Takeoff {
                            first: true,
                            path: None,
                            target: None,
                        }
                    } else {
                        Phase::Scanning { time: 0 }
                    });
                } else if *ticks == 10 {
                    let d = self.common.dragon.as_ref().expect("dragon");
                    let head = d.parts[0].0;
                    let look = glam::DVec2::new(head.x - pos.x, head.z - pos.z).normalize_or_zero();
                    let (x, z) = (head.x + look.x * 2.5, head.z + look.y * 2.5);
                    let mut y = head.y + 0.5;
                    let (xi, zi) = (x.floor() as i32, z.floor() as i32);
                    while y > 0.0
                        && self.common.res.facts().is_air(self.common.world.block(
                            xi,
                            y.floor() as i32,
                            zi,
                        ))
                    {
                        y -= 1.0;
                    }
                    let id = self.take_entity_id();
                    self.common.breath_clouds.push(Breath {
                        attachment_lifetime: Default::default(),
                        join: Default::default(),
                        id,
                        at: [x, y.floor() + 1.0, z],
                        motion: [0.0; 3],
                        fall_distance: 0.0,
                        fire_ticks: 0,
                        first_tick: true,
                        radius: 5.0,
                        age: 0,
                        duration: 200,
                        from_dragon: true,
                        victims: Vec::new(),
                        potion: None,
                    });
                }
            }
            Phase::Charging { target, since } => {
                if target.is_none() {
                    next = Some(Phase::Holding {
                        path: None,
                        target: None,
                        clockwise: false,
                    });
                } else if *since > 0 {
                    *since += 1;
                    if *since >= 10 {
                        next = Some(Phase::Holding {
                            path: None,
                            target: None,
                            clockwise: false,
                        });
                    }
                } else {
                    let dt = far(*target);
                    if dt < 100.0 || dt > 22500.0 {
                        *since += 1;
                    }
                }
            }
            Phase::Dying { target, time } => {
                *time += 1;
                let t = *target.get_or_insert(Vec3::new(
                    0.5,
                    f64::from(self.common.motion_blocking_no_leaves(0, 0)),
                    0.5,
                ));
                let dt = (t.x - pos.x).powi(2) + (t.y - pos.y).powi(2) + (t.z - pos.z).powi(2);
                let d = self.common.dragon.as_mut().expect("dragon");
                d.health = if !(100.0..=22500.0).contains(&dt) {
                    0.0
                } else {
                    1.0
                };
            }
            Phase::Hover { target } => {
                target.get_or_insert(pos);
            }
        }
        let d = self.common.dragon.as_mut().expect("dragon");
        d.phase = phase;
        if let Some(n) = next {
            d.set_phase(n);
            if let Some(at) = strafe_at {
                self.strafe_begin(at);
            }
        }
    }
    /// The inherited LivingEntity.tick runs Mob.tickHeadTurn after aiStep.
    pub fn tick_dragon(&mut self) {
        self.dragon_ai_step();
        if let Some(d) = self.common.dragon.as_mut() {
            let (dx, dz) = (d.position.x - d.previous.x, d.position.z - d.previous.z);
            // EnderDragon inherits Mob.getMaxHeadYRot (75 degrees).
            d.body_rotation.tick(
                dx * dx + dz * dz,
                d.yaw,
                &mut d.body_yaw,
                &mut d.head_yaw,
                75.0,
            );
        }
    }

    /// `EnderDragon.aiStep` (and `tickDeath`) for the dragon's own entity tick.
    fn dragon_ai_step(&mut self) {
        // updateDragon runs when the fight dragon ticks or dies, but not with NoAI.
        let ticks = self
            .common
            .dragon
            .as_ref()
            .is_some_and(|d| !d.alive() || !d.no_ai);
        if ticks && self.is_fight_dragon() {
            self.server.fight.dragon_unseen = 0;
        }
        // Entity.baseTick: the saved invulnerability window counts down
        // (#916), before the dragon's own borrow of the game.
        if let Some(dragon) = self.common.dragon.as_ref() {
            let id = dragon.id;
            if !dragon.equipment.is_empty() {
                self.tick_equipment_enchantments(super::commands::Target::Dragon(id), id);
            }
            self.tick_saved_invulnerable_time(id);
        }
        let target_dead = self
            .common
            .dragon
            .as_ref()
            .and_then(|d| d.last_hurt_mob)
            .is_some_and(|(id, _)| !self.last_hurt_target_alive(id));
        let Some(d) = self.common.dragon.as_mut() else {
            return;
        };
        d.previous = d.position;
        d.previous_yaw = d.yaw;
        d.old_flap = d.flap;
        d.ticks = d.ticks.wrapping_add(1);
        if target_dead {
            d.last_hurt_mob = None;
        }
        d.hurt_time = (d.hurt_time - 1).max(0);
        d.invulnerable = (d.invulnerable - 1).max(0);
        for part in &mut d.parts {
            part.1 = part.0;
        }
        if !d.alive() {
            self.tick_dragon_death();
            return;
        }
        // Mob.baseTick: the ambient sound, rarer right after one.
        let roll = d.random.next_int_bound(1000).unwrap_or(999);
        let waited = d.ambient_time;
        d.ambient_time += 1;
        if roll < waited {
            d.ambient_time = -AMBIENT_INTERVAL;
            let at = [d.position.x, d.position.y, d.position.z];
            // LivingEntity.getVoicePitch, from the dragon's own random.
            let r = &mut d.random;
            let pitch = (r.next_float() - r.next_float()) * 0.2 + 1.0;
            self.server_play_sound("minecraft:entity.ender_dragon.ambient", at, 5.0, pitch);
        }
        let d = self.common.dragon.as_mut().expect("dragon");
        // Flapping, growls and the flap sound.
        // Mth.cos(flapTime * 6.2831855F); flapSpeed's `(float) Math.pow(2.0, movement.y)`.
        let flap_was = trig::cos(f64::from(d.old_flap * std::f32::consts::TAU));
        let v = d.velocity;
        let mut flap_speed = 0.2 / (((v.x * v.x + v.z * v.z).sqrt() as f32) * 10.0 + 1.0);
        flap_speed *= 2f64.powf(v.y) as f32;
        d.flap += if d.phase.sitting() {
            0.1
        } else if d.in_wall {
            flap_speed * 0.5
        } else {
            flap_speed
        };
        let flap_now = trig::cos(f64::from(d.flap * std::f32::consts::TAU));
        let at = [d.position.x, d.position.y, d.position.z];
        if flap_was <= -0.3 && flap_now >= -0.3 {
            let pitch = 0.8 + self.common.world.random.next_float() * 0.3;
            self.server_play_sound("minecraft:entity.ender_dragon.flap", at, 5.0, pitch);
        }
        let d = self.common.dragon.as_mut().expect("dragon");
        if !d.phase.sitting() {
            d.growl -= 1;
            if d.growl < 0 {
                d.growl = 200 + d.random.next_int_bound(200).unwrap_or(0);
                let pitch = 0.8 + self.common.world.random.next_float() * 0.3;
                self.server_play_sound("minecraft:entity.ender_dragon.growl", at, 2.5, pitch);
            }
        }
        // checkCrystals: heal 1 every 10 ticks from the nearest crystal, found
        // among those touching the dragon's box grown by 32.
        let d = self.common.dragon.as_mut().expect("dragon");
        if d.nearest_crystal.is_some() && d.ticks % 10 == 0 && d.health < MAX_HEALTH {
            // LivingEntity.setHealth clamps to the maximum.
            d.health = (d.health + 1.0).min(MAX_HEALTH);
        }
        if d.random.next_int_bound(10).unwrap_or(1) == 0 {
            let p = d.position;
            let nearest = self.common.nearest_crystal_to(p);
            self.common.dragon.as_mut().expect("dragon").nearest_crystal = nearest;
        }
        let d = self.common.dragon.as_mut().expect("dragon");
        d.yaw = wrap(d.yaw);
        // EnderDragon.aiStep with isNoAi: the flap stops half way, and nothing
        // else moves it (no flight history, phase, steering or parts).
        if d.no_ai {
            d.flap = 0.5;
            return;
        }
        d.record();
        self.tick_phase();
        // Steering towards the phase's fly target.
        let Some(d) = self.common.dragon.as_mut() else {
            return;
        };
        if let Some(t) = d.phase.target() {
            let (xd, mut yd, zd) = (t.x - d.position.x, t.y - d.position.y, t.z - d.position.z);
            let dist2 = xd * xd + yd * yd + zd * zd;
            let max = f64::from(d.phase.fly_speed());
            let horizontal = (xd * xd + zd * zd).sqrt();
            if horizontal > 0.0 {
                yd = (yd / horizontal).clamp(-max, max);
            }
            d.velocity.y += yd * 0.01;
            d.yaw = wrap(d.yaw);
            // Mth.sin / Mth.cos (the table) and Mth.atan2, as vanilla computes them: a hovering
            // dragon's target is a hair away, where the exact functions turn it another way.
            let aim = vanilla_normalize(glam::DVec3::new(xd, t.y - d.position.y, zd));
            let r = f64::from(d.yaw * trig::DEG_TO_RAD);
            let dir = vanilla_normalize(glam::DVec3::new(
                f64::from(trig::sin(r)),
                d.velocity.y,
                f64::from(-trig::cos(r)),
            ));
            let dot = ((dir.dot(aim) as f32 + 0.5) / 1.5).max(0.0);
            if xd.abs() > f64::from(1.0e-5_f32) || zd.abs() > f64::from(1.0e-5_f32) {
                let rot =
                    (d.velocity.x * d.velocity.x + d.velocity.z * d.velocity.z).sqrt() as f32 + 1.0;
                let turn = match d.phase {
                    Phase::Landing { .. } => rot.min(40.0) / rot,
                    _ => 0.7 / rot.min(40.0) / rot,
                };
                let delta = wrap(180.0 - trig::atan2(xd, zd) as f32 * trig::RAD_TO_DEG - d.yaw)
                    .clamp(-50.0, 50.0);
                d.yaw_accel *= 0.8;
                d.yaw_accel += delta * turn;
                d.yaw += d.yaw_accel * 0.1;
            }
            let span = (2.0 / (dist2 + 1.0)) as f32;
            let speed = 0.06 * (dot * span + (1.0 - span));
            // moveRelative(speed, (0, 0, -1)): getInputVector in doubles, Mth's sine.
            let r = f64::from(d.yaw * trig::DEG_TO_RAD);
            d.velocity.x += f64::from(speed) * f64::from(trig::sin(r));
            d.velocity.z += -f64::from(speed) * f64::from(trig::cos(r));
            let step = if d.in_wall { 0.800000011920929 } else { 1.0 };
            // The dragon's Entity.move is this physics-less step (#2414): it
            // goes through the move gate like every other move, self cause,
            // before the position is written.
            let gate = sunrise_sim::move_gate::armed(
                &self.common.loop_context,
                d.id,
                "minecraft:ender_dragon",
                sunrise_world::entity_move::MoverType::SelfMovement,
                Vec3::new(
                    d.velocity.x * step,
                    d.velocity.y * step,
                    d.velocity.z * step,
                ),
            );
            if !gate.skipped() {
                let moved = gate.delta();
                d.position = Vec3::new(
                    d.position.x + moved.x,
                    d.position.y + moved.y,
                    d.position.z + moved.z,
                );
            }
            let actual =
                vanilla_normalize(glam::DVec3::new(d.velocity.x, d.velocity.y, d.velocity.z));
            let slide = 0.8 + 0.15 * (actual.dot(dir) + 1.0) / 2.0;
            d.velocity = Vec3::new(
                d.velocity.x * slide,
                d.velocity.y * 0.9100000262260437,
                d.velocity.z * slide,
            );
        }
        // EnderDragon.aiStep assigns the body's angle after flight, before placing parts.
        d.body_yaw = d.yaw;
        self.common.place_dragon_parts();
        self.dragon_contacts();
        // The chunks the dragon passes through re-save their entity lists.
        if let Some(d) = self.common.dragon.as_ref() {
            for p in [d.previous, d.position] {
                self.common
                    .world
                    .modified
                    .insert((p.x.floor() as i32 >> 4, p.z.floor() as i32 >> 4));
            }
        }
    }
    /// Wing knockback, head and neck damage, and breaking blocks in the way.
    pub(in crate::game::server_side) fn dragon_contacts(&mut self) {
        let Some(d) = self.common.dragon.as_ref() else {
            return;
        };
        let sitting = d.phase.sitting();
        let recently_hurt = d.hurt_time > 0;
        let body = d.part_box(2);
        let (bc, bz) = (
            (body.min_position().x + body.max_position().x) / 2.0,
            (body.min_position().z + body.max_position().z) / 2.0,
        );
        let wings = [d.part_box(6), d.part_box(7)];
        let heads = [d.part_box(0), d.part_box(1)];
        if !recently_hurt {
            // EntitySelector.NO_CREATIVE_OR_SPECTATOR, every player alike.
            let players: Vec<FightPlayer> = self
                .fight_players()
                .into_iter()
                .filter(|p| p.alive && !p.creative && !p.spectator)
                .collect();
            for p in players {
                // knockBack, once for each wing whose zone holds them.
                for w in wings {
                    let zone = w
                        .inflate(Vec3::new(4.0, 2.0, 4.0))
                        .move_by(Vec3::new(0.0, -2.0, 0.0));
                    if zone.intersects(p.bounds) {
                        let (xd, zd) = (p.position.x - bc, p.position.z - bz);
                        let dd = (xd * xd + zd * zd).max(0.1);
                        let push = Vec3::new(xd / dd * 4.0, f64::from(0.2_f32), zd / dd * 4.0);
                        self.as_fight_player(p.who, |g| g.dragon_wing_knock_back(push, sitting));
                    }
                }
                // hurt: the head and the neck.
                for h in heads {
                    if h.inflate(Vec3::new(1.0, 1.0, 1.0)).intersects(p.bounds) {
                        self.as_fight_player(p.who, |g| {
                            g.dragon_hurts_player("minecraft:mob_attack", 10.0);
                        });
                    }
                }
            }
        }
        // EnderDragon.aiStep queries living entities, not only players.
        if let Some(d) = self.common.dragon.as_ref() {
            let frame = sunrise_gameplay::dragon_contacts::Frame {
                dragon_id: d.id,
                origin: d.position,
                center: (bc, bz),
                wings: wings.map(|w| {
                    w.inflate(Vec3::new(4.0, 2.0, 4.0))
                        .move_by(Vec3::new(0.0, -2.0, 0.0))
                }),
                heads: heads.map(|h| h.inflate(Vec3::new(1.0, 1.0, 1.0))),
                sitting,
                recently_hurt,
            };
            // Keep team lookup in Game (including tamed pets' owner teams).
            let allowed: Vec<bool> = self
                .common
                .mobs
                .iter()
                .map(|m| {
                    sunrise_game_data::scoreboard::teams_allow_damage(
                        self.mob_team(d.id),
                        self.mob_team_of(m),
                    )
                })
                .collect();
            // knockBack(wing1), knockBack(wing2), hurt(head), hurt(neck): each
            // part's list in turn, so Thorns' level and dragon draws follow it.
            for part in 0..4 {
                for (index, &allowed) in allowed.iter().enumerate() {
                    let id = self.common.mobs[index].id;
                    let plan = self.prepare_knockback_credit(
                        id as i32,
                        Some(Who::Mob(frame.dragon_id)),
                        self.server.combat.0.attack(),
                    );
                    let mut credit = plan.lend(&mut self.server.last_hurt_by_player);
                    if sunrise_gameplay::dragon_contacts::contact_part(
                        crate::game::entity_server::damage_level_of!(
                            self,
                            &sunrise_sim::health::NO_ATTACK,
                            &mut credit,
                            None
                        ),
                        &mut self.common.mobs[index],
                        &frame,
                        allowed,
                        part,
                    ) {
                        self.finish_mob_damage_at(index);
                        self.mob_post_attack(
                            id,
                            Some(sunrise_sim::mobs::Who::Mob(frame.dragon_id)),
                            None,
                            Some("minecraft:mob_attack"),
                        );
                    }
                }
            }
        }
        self.dragon_check_walls();
    }
    /// `Entity.isSilent` of the dragon: its `Silent`, kept with the fields
    /// Sunrise models nowhere (`remember_tags`: `/summon`, a load).
    pub fn dragon_silent(&self) -> bool {
        let raw = self
            .common
            .dragon
            .as_ref()
            .filter(|d| d.id != 0)
            .and_then(|d| self.common.persist.raw.get(&d.id));
        saved_silent(raw)
    }

    /// `EnderDragonFight.setDragonKilled`.
    pub(in crate::game::server_side) fn dragon_killed(&mut self) {
        let _ = self.activate_exit_portal();
        if let Some(g) = self.server.fight.gateways.pop() {
            let a = 2.0 * (-std::f64::consts::PI + std::f64::consts::PI / 20.0 * f64::from(g));
            let pos = [
                (96.0 * a.cos()).floor() as i32,
                75,
                (96.0 * a.sin()).floor() as i32,
            ];
            self.server_level_event_at(3000, pos, 0);
            let _ = self.place_gateway_at(pos);
        }
        // getHeightmapPos(MOTION_BLOCKING, getPodiumLocation(origin)).
        let y = {
            use sunrise_world::{heightmap::HeightmapType, worldgen::terrain::TerrainBlockStates};
            let properties = &self.common.pack.assets.properties;
            self.common.world.motion_blocking(0, 0, |s| {
                HeightmapType::MotionBlocking.is_opaque(properties.heightmap_state(s))
            })
        };
        let previously_won = self.server.fight.previously_killed;
        let mut place_egg = None;
        if mod_api::__private::listened(mod_api::HookId::DragonFightWon) {
            let pos = BlockPos::new(0, y, 0);
            self.native_dragon_fight_won(self.fight_level(), pos, previously_won, &mut place_egg);
        }
        if place_egg.unwrap_or(!previously_won)
            && let Some(egg) = self
                .common
                .pack
                .assets
                .registry
                .default_state("minecraft:dragon_egg")
        {
            let _ = self.level_set([0, y, 0], egg);
        }
        self.server.fight.previously_killed = true;
        self.server.fight.killed = true;
        self.server.fight.dirty = true;
        self.save_fight();
    }
    /// `DragonFireball.onHit` where it burst (`next`): the dragon's breath
    /// cloud and level event 2006, as the fireballs of the dragon's own list
    /// burst before they joined the other projectiles.
    pub(in crate::game::server_side) fn dragon_fireball_on_hit(&mut self, next: Vec3) {
        let snap_targets: Vec<(Aabb, Vec3)> = self
            .fight_players()
            .iter()
            .filter(|p| p.alive)
            .map(|p| (p.bounds, p.position))
            .chain(
                self.common
                    .mobs
                    .iter()
                    .filter(|m| m.alive())
                    .map(|m| (m.bounds(), m.position)),
            )
            .collect();
        let id = self.take_entity_id();
        let feet = [next.x, next.y - 0.5, next.z];
        let at = fireball_cloud_origin(feet, &snap_targets);
        self.common.breath_clouds.push(Breath {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id,
            at,
            motion: [0.0; 3],
            fall_distance: 0.0,
            fire_ticks: 0,
            first_tick: true,
            radius: 3.0,
            age: 0,
            duration: 600,
            from_dragon: false,
            victims: Vec::new(),
            potion: None,
        });
        self.server_level_event_at(
            2006,
            [
                next.x.floor() as i32,
                next.y.floor() as i32,
                next.z.floor() as i32,
            ],
            1,
        );
    }
    /// The dragon's breath clouds (`AreaEffectCloud` with instant damage at
    /// a quarter duration scale): shrinking, puffing particles, harming players and mobs.
    /// `DragonSittingFlamingPhase.end`: the flame cloud goes once the
    /// phase is left.
    pub(in crate::game::server_side) fn end_dragon_flame(&mut self) {
        if self
            .common
            .dragon
            .as_mut()
            .is_some_and(|d| std::mem::take(&mut d.flame_ended))
        {
            self.retain_breath_clouds(|c| !c.from_dragon);
        }
    }
    pub fn tick_breath(&mut self) {
        // What instant damage reaches: neither creative nor spectator players.
        let players: Vec<FightPlayer> = self
            .fight_players()
            .into_iter()
            .filter(|p| p.alive && !p.creative && !p.spectator)
            .collect();
        // AreaEffectCloud.serverTick: every living entity, the mobs that
        // instant damage can affect (`canBeAffected`) too; only while a
        // breath cloud is there (lingering potions' clouds tick elsewhere).
        let no_breath = self.common.breath_clouds.iter().all(|c| c.potion.is_some());
        let mobs: Vec<(u32, Aabb, Vec3)> = if no_breath {
            Vec::new()
        } else {
            let tags = &self.items().entity_type_tags;
            self.common
                .mobs
                .iter()
                .filter(|m| {
                    m.alive()
                        && mob_can_be_affected(
                            tags,
                            &m.kind.id,
                            m.kind.flags.entity_tag_slot,
                            "instant_damage",
                        )
                })
                .map(|m| (m.id, m.bounds(), m.position))
                .collect()
        };
        let mut harmed: Vec<(FightWho, f32)> = Vec::new();
        self.end_dragon_flame();
        let mut harmed_mobs: Vec<(u32, i32, [f64; 3], u32)> = Vec::new();
        let ticking = self.entity_ticking_area();
        let through = self.server.ticked_entities_through;
        let mut clouds = self.common.breath_clouds.lift();
        #[cfg(any(test, feature = "test-support"))]
        crate::game::server_side::entity_server::taken_pass_tests::after_take(self);
        clouds.retain_mut(|c| {
            // Lingering potions' clouds tick with the potions.
            if c.potion.is_some() {
                return true;
            }
            if c.id > through || !ticking(c.at[0].floor() as i32, c.at[2].floor() as i32) {
                return true;
            }
            self.cloud_fluid_step(c);
            c.age += 1;
            if breath_expired(c.age, c.duration) {
                return self.forget_expired_cloud(c.id);
            }
            if !c.from_dragon && c.age >= BREATH_WAIT {
                // DragonFireball's cloud: radius per tick (7 - 3) / duration.
                c.radius += (7.0 - 3.0) / c.duration as f32;
            }
            let r = c.radius;
            for _ in 0..((std::f32::consts::PI * r * r) as i32).min(40) {
                let _a = self.common.world.random.next_float() * std::f32::consts::TAU;
                let _d = self.common.world.random.next_float().sqrt() * r;
            }
            // AreaEffectCloud: after its wait, each entity every 20 ticks.
            // Instant damage I from the flame cloud, II from a fireball's,
            // both at the cloud's 0.5 scale.
            let amount = harm_amount(u32::from(!c.from_dragon));
            let inside = players
                .iter()
                .filter(|p| cloud_hits(c.at, r, p.bounds, p.position))
                .map(|p| Victim::Player(p.who))
                .chain(
                    mobs.iter()
                        .filter(|m| cloud_hits(c.at, r, m.1, m.2))
                        .map(|m| Victim::Mob(m.0)),
                );
            for victim in breath_victims(c.age, &mut c.victims, inside) {
                match victim {
                    Victim::Player(who) => harmed.push((who, amount)),
                    Victim::Mob(id) => {
                        harmed_mobs.push((id, i32::from(!c.from_dragon), c.at, c.id));
                    }
                }
            }
            true
        });
        self.common
            .breath_clouds
            .restore(clouds, sunrise_sim::join_seq::RestoreOrder::SpawnsFirst);
        for (who, amount) in harmed {
            self.as_fight_player(who, |g| {
                g.dragon_hurts_player("minecraft:indirect_magic", amount);
            });
        }
        for (id, amplifier, at, cloud_id) in harmed_mobs {
            self.breath_affect_mob(id, amplifier, at, Some(cloud_id));
        }
    }
    /// `EnderDragon.knockBack` on the player in a wing's zone: pushed away
    /// from the body, and hurt for 5 unless the dragon is perched
    /// (`isSitting`) or a mob hurt the player within the last 2 ticks
    /// (`getLastHurtByMobTimestamp() < tickCount - 2`).
    pub fn dragon_wing_knock_back(&mut self, push: Vec3, sitting: bool) {
        let now = self.common.world.game_time;
        let hurt_lately = self.player_hurt_by().is_some_and(|(_, at)| at >= now - 2);
        let hurts = !sitting && !hurt_lately && !self.common.creative;
        if hurts {
            self.note_dragon_attacker("minecraft:mob_attack");
        }
        let before = self.common.player.as_mut().map(|p| {
            let before = (p.velocity, std::mem::take(&mut p.health.sync_velocity));
            p.velocity = p.velocity.add_vector(push);
            before
        });
        if hurts {
            let origin = self
                .common
                .dragon
                .as_ref()
                .map(|d| [d.position.x, d.position.y, d.position.z]);
            let blocking = self.player_blocking(origin, "minecraft:mob_attack", false);
            let attack = self.current_attack();
            let causing = self.common.dragon.as_ref().map_or(0, |d| d.id as i32);
            if let Some(id) = self
                .common
                .health_resources
                .damage
                .id("minecraft:mob_attack")
                && let Ok(outcome) = self
                    .hurt_player_equipment(id, 5.0, blocking, false, causing, false, &attack, None)
            {
                self.server.combat.0.forget_noted(outcome.canceled);
                self.after_block(
                    outcome,
                    (self.common.dragon.as_ref().map(|d| d.id), false),
                    HurtSource {
                        damage_type: "minecraft:mob_attack",
                        causing,
                        direct: causing,
                        position: origin,
                        direct_type: None,
                    },
                    &attack,
                );
            }
        }
        if let Some((velocity, marked)) = before
            && let Some(p) = self.common.player.as_mut()
        {
            if !p.health.sync_velocity {
                p.velocity = velocity;
            }
            p.health.sync_velocity |= marked;
        }
        self.finish_guest_damage();
        if !sitting
            && !hurt_lately
            && let Some(id) = self.common.dragon.as_ref().map(|d| d.id)
        {
            self.thorns(id);
        }
    }
    /// Damage from the dragon, which is its causing entity
    /// (`DamageSources.mobAttack(dragon)`, `indirectMagic(cloud, dragon)`): the
    /// death message names it, or the cloud once the dragon is gone.
    pub fn dragon_hurts_player(&mut self, damage_type: &str, amount: f32) {
        self.note_dragon_attacker(damage_type);
        self.hurt_player_by(damage_type, amount);
        // EnderDragon.hurt(List) runs effects even when hurtServer refused.
        // The breath cloud's indirect magic never runs them.
        if damage_type == "minecraft:mob_attack"
            && let Some(id) = self.common.dragon.as_ref().map(|d| d.id)
        {
            self.thorns(id);
        }
    }
    /// The causing entity of the dragon's next `damage_type` damage, for the
    /// death message.
    pub(in crate::game::server_side) fn note_dragon_attacker(&mut self, damage_type: &str) {
        if let Some(id) = self.common.dragon.as_ref().map(|d| d.id) {
            let name = Text::translate("entity.minecraft.ender_dragon", &[]);
            self.note_named_attacker(damage_type, id, name);
        } else if damage_type == "minecraft:indirect_magic" {
            let name = Text::translate("entity.minecraft.area_effect_cloud", &[]);
            self.note_direct_entity(damage_type, name);
        }
    }
    /// The dragon fight's bar for a player at `at`
    /// (`EnderDragonFight.dragonEvent`, visible while the fight is not won
    /// and the player is alive within 192 blocks of (0, 128, 0)).
    pub(in crate::game::server_side) fn dragon_boss_progress(
        &self,
        at: [f64; 3],
        alive: bool,
    ) -> Option<f32> {
        if !self.dimension_type().has_ender_dragon_fight
            || !alive
            || !self.server.fight.loaded
            || self.server.fight.killed
        {
            return None;
        }
        let near = at[0].powi(2) + (at[1] - 128.0).powi(2) + at[2].powi(2) <= 192.0 * 192.0;
        if !near {
            return None;
        }
        Some(
            self.common
                .dragon
                .as_ref()
                .map_or(1.0, |d| (d.health / MAX_HEALTH).clamp(0.0, 1.0)),
        )
    }
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn fight_ready(&mut self) -> bool {
        self.load_fight();
        // spawnExitPortal runs from scanState and the dragon's death, both
        // only with players in the fight.
        self.server.fight.loaded && self.server.fight.players
    }
    pub(in crate::game::server_side) fn exit_portal(&self) -> Option<[i32; 3]> {
        self.server.fight.exit_portal
    }
    pub(in crate::game::server_side) fn set_exit_portal(&mut self, origin: [i32; 3]) {
        if self.server.fight.exit_portal.is_none() {
            self.server.fight.exit_portal = Some(origin);
            self.server.fight.dirty = true;
        }
    }
}
/// `Entity.isSilent` from an entity's saved compound: `Silent`, written only
/// when set.
pub(in crate::game::server_side) fn saved_silent(raw: Option<&Tag>) -> bool {
    matches!(raw.and_then(|t| t.get("Silent")), Some(Tag::Byte(b)) if *b != 0)
}
impl crate::game::ServerGame<'_> {
    /// `FireworkRocketEntity.dealExplosionDamage` on the dragon: a living
    /// entity in the rocket's `inflate(5)` box and within 5 blocks, seen at its
    /// feet or middle, takes `EnderDragon.hurtServer` (`hurt(body, ...)`).
    pub fn rocket_blast_dragon(
        &mut self,
        from: Vec3,
        damage: f32,
        reach: Aabb,
        by: Option<FightWho>,
        rocket_id: u32,
        owner: Option<Who>,
    ) {
        let Some(d) = self.common.dragon.as_ref() else {
            return;
        };
        let p = d.position;
        let d2 = (p.x - from.x).powi(2) + (p.y - from.y).powi(2) + (p.z - from.z).powi(2);
        if d2 > 25.0 || !d.whole_box().intersects(reach) {
            return;
        }
        let world = self.mob_world_base(&[]);
        let sees = (0..2).any(|step| {
            let to = Vec3::new(p.x, p.y + 8.0 * 0.5 * f64::from(step), p.z);
            world.clear_between(from, to, false)
        });
        if !sees {
            return;
        }
        let amount = super::fireworks::blast_damage(damage, from, p);
        let hooks = DragonHitHooks::capture();
        let hook_source = hooks.any.then(|| {
            let causing = match owner {
                Some(Who::Player) => by.map_or(0, |who| self.fight_entity(who)),
                Some(Who::Mob(id)) => id as i32,
                None => 0,
            };
            HurtSource {
                damage_type: "minecraft:fireworks",
                causing,
                direct: rocket_id as i32,
                position: Some([from.x, from.y, from.z]),
                direct_type: Some("minecraft:firework_rocket"),
            }
        });
        self.hurt_dragon_part_at(
            BODY,
            amount,
            true,
            by,
            DamageKind::Fireworks,
            DragonHitOrigin {
                direct: None,
                entity_id: None,
                from: None,
                invulnerability_checked: false,
                hook_source,
                attack: None,
                hooks,
            },
        );
    }
    /// `ServerExplosion.hurtEntities`: crystals in reach are removed without another
    /// blast; explosions always hurt dragons, except one `explosion_detonate` spared.
    pub fn explosion_hits_special(
        &mut self,
        center: [f64; 3],
        radius: f32,
        blocks: &[[i32; 3]],
        damage_type: &str,
        blast_direct: Option<(u32, Option<&'static str>)>,
    ) {
        let diameter = f64::from(radius * 2.0);
        let in_reach = |p: Vec3| {
            ((p.x - center[0]).powi(2) + (p.y - center[1]).powi(2) + (p.z - center[2]).powi(2))
                .sqrt()
                / diameter
                <= 1.0
        };
        let (direct, direct_type) = match blast_direct {
            Some((id, kind)) => (
                id as i32,
                kind.or_else(|| {
                    self.server
                        .combat
                        .0
                        .explosion_direct
                        .filter(|&(d, _)| d == id)
                        .map(|(_, ty)| ty)
                }),
            ),
            None => match self.server.combat.0.explosion_direct {
                Some((id, ty)) => (id as i32, Some(ty)),
                None => (
                    self.server
                        .combat
                        .0
                        .explosion_source
                        .map_or(0, |id| id as i32),
                    None,
                ),
            },
        };
        let crystals: Vec<Vec3> = self
            .common
            .crystals
            .iter()
            .filter(|c| in_reach(c.position) && self.server.combat.0.visits(c.id))
            .map(|c| c.position)
            .collect();
        // The blast's source carries the player who set it off, if any.
        let by = self.server.fight.blast_by;
        for at in crystals {
            // Look up the current index after each removal.
            if let Some(i) = self.common.crystals.iter().position(|c| c.position == at) {
                let id = self.common.crystals[i].id;
                let causing = self.blast_attacker().map_or(0, |(id, _)| id as i32);
                let vanilla = self.common.crystals[i].invulnerable
                    && !self.creative_damage_source_id(causing);
                let source = HurtSource {
                    damage_type,
                    causing,
                    direct,
                    position: Some(center),
                    direct_type,
                };
                if self.native_nonliving_invulnerable(id, "minecraft:end_crystal", source, vanilla)
                {
                    continue;
                }
                self.common.crystals[i].invulnerable = false;
                self.destroy_crystal(i, by, CrystalDamage::Explosion);
            }
        }
        let Some(d) = self
            .common
            .dragon
            .as_ref()
            .filter(|d| self.server.combat.0.visits(d.id))
        else {
            return;
        };
        let hits = d.explosion_hits(center, radius, |b| {
            let (lo, hi) = (b.min_position(), b.max_position());
            self.seen_percent(center, [lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z])
        });
        let whole = d.whole_box();
        let (lo, hi) = (whole.min_position(), whole.max_position());
        let exposure = self.seen_percent(center, [lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]);
        let hooks = DragonHitHooks::capture();
        let blast_source = hooks.any.then(|| HurtSource {
            damage_type,
            causing: self.blast_attacker().map_or(0, |(id, _)| id as i32),
            direct,
            position: Some(center),
            direct_type,
        });
        for (part, amount) in hits {
            self.hurt_dragon_part_at(
                part,
                amount,
                true,
                by,
                DamageKind::Explosion,
                DragonHitOrigin {
                    direct: None,
                    entity_id: None,
                    from: None,
                    invulnerability_checked: false,
                    hook_source: blast_source,
                    attack: None,
                    hooks,
                },
            );
        }
        let Some((id, mut push)) = self
            .common
            .dragon
            .as_ref()
            .and_then(|d| Some((d.id, d.explosion_knockback(center, radius, exposure)?)))
        else {
            return;
        };
        self.native_explosion_knockback(self.common.dim.id(), center, blocks, Some(id), &mut push);
        if let Some(d) = self.common.dragon.as_mut() {
            d.velocity = Vec3::new(
                d.velocity.x + push[0],
                d.velocity.y + push[1],
                d.velocity.z + push[2],
            );
        }
    }
}
impl crate::game::ServerGame<'_> {
    /// The dragon as guests track it (`EnderDragon.getAddEntityPacket` and
    /// the synched data that differs from the defaults: its health, defined
    /// as 1, and `DATA_PHASE`, defined as hovering). Vanilla pairs it with no
    /// attribute instances.
    pub fn lan_dragon_state(&self) -> Option<sunrise_sim::lan::entity::EntityState> {
        use sunrise_protocol::entity_data::{Value, serializer};
        let d = self.common.dragon.as_ref().filter(|d| d.id != 0)?;
        let mut meta = Vec::new();
        let health = d.health.max(0.0);
        if health != 1.0 {
            meta.push((9, serializer::FLOAT, Value::Float(health)));
        }
        // Mob.DATA_MOB_FLAGS_ID: aggressive is 4.
        if d.aggressive {
            meta.push((15, serializer::BYTE, Value::Byte(4)));
        }
        let phase = d.phase.id();
        if phase != 10 {
            meta.push((16, serializer::INT, Value::Int(phase)));
        }
        // Mob.DATA_MOB_FLAGS_ID: NoAI is its first bit.
        if d.no_ai {
            meta.push((15, serializer::BYTE, Value::Byte(1)));
        }
        Some(sunrise_sim::lan::entity::EntityState {
            id: d.id as i32,
            uuid: super::lan::entity_uuid(d.id),
            kind: "minecraft:ender_dragon".to_owned(),
            position: [d.position.x, d.position.y, d.position.z],
            yaw: d.yaw,
            pitch: 0.0,
            head_yaw: d.yaw,
            on_ground: false,
            velocity: [d.velocity.x, d.velocity.y, d.velocity.z],
            minecart_steps: None,
            data: 0,
            meta,
            display: None,
            forced_meta: [0; 4],
            // Back at their defaults the values leave `meta`, and the clients
            // still hold the old ones.
            defaults: vec![
                (9, serializer::FLOAT, Value::Float(1.0)),
                (15, serializer::BYTE, Value::Byte(0)),
                (16, serializer::INT, Value::Int(10)),
            ],
            equipment: Vec::new(),
            attributes: Vec::new(),
            precise: false,
            vertical_collision: false,
            horizontal_collision: false,
            needs_sync: false,
            sync_position: false,
            sync_velocity: false,
            stepped: true,
            idle: false,
            bare: false,
            leash: None,
            spectator: false,
            watching: false,
            hidden: None,
            general: None,
        })
    }
}
impl crate::game::ServerGame<'_> {
    /// The dragon and fight for `sunrise-ctl status`.
    pub(in crate::game) fn dragon_status(&self) -> serde_json::Value {
        serde_json::json!({
            "dragon": self.dragon.as_ref().map(|d| serde_json::json!({
                "id": d.id,
                "position": [d.position.x, d.position.y, d.position.z],
                "health": d.health,
                "phase": d.phase.id(),
                "death_time": d.death_time,
                // The player it strafes: "host", or the guest's entity id.
                "strafing": match &d.phase {
                    Phase::Strafe { player: Some(FightWho::Host), .. } => serde_json::json!("host"),
                    Phase::Strafe { player: Some(FightWho::Guest(conn)), .. } => serde_json::json!(
                        self.server.lan.as_ref().and_then(|h| h.hub.guest(*conn)).map(|g| g.entity_id)
                    ),
                    _ => serde_json::Value::Null,
                },
            })),
            "players": self.server.fight.players,
            "crystals": self.crystals.len(),
            // The tracked id (0 until tracked) and position of each crystal.
            "crystal_list": self.crystals.iter().map(|c| serde_json::json!({
                "id": c.id,
                "position": [c.position.x, c.position.y, c.position.z],
            })).collect::<Vec<_>>(),
            "alive_crystals": self.server.fight.alive_crystals,
            "killed": self.server.fight.killed,
            "exit_portal": self.server.fight.exit_portal,
            "respawn": self.server.fight.respawn.map(|(s, t)| format!("{} {t}", s.name())),
        })
    }
}
/// Respawning the dragon with end crystals on the exit portal
/// (`EndCrystalItem.useOn`, `EnderDragonFight.tryRespawn`, `DragonRespawnStage`).
impl crate::game::ServerGame<'_> {
    /// `EndCrystalItem.useOn`: on obsidian or bedrock with room above.
    pub(in crate::game::server_side) fn place_end_crystal(
        &mut self,
        hit: &super::Hit,
        stack: &sunrise_items::items::ItemStack,
    ) -> bool {
        if &*stack.item != "minecraft:end_crystal" || !self.end_crystal_fits(hit.pos) {
            return false;
        }
        let above = [hit.pos[0], hit.pos[1] + 1, hit.pos[2]];
        let (x, y, z) = (
            f64::from(above[0]),
            f64::from(above[1]),
            f64::from(above[2]),
        );
        // EndCrystal's constructor: `this.random.nextInt(100000)`, the crystal's own random.
        let drawn = random_source::entity().next_int_bound(100_000).unwrap_or(0);
        let at = Vec3::new(x + 0.5, y, z + 0.5);
        let time = Crystal::start_time(at, drawn, self.common.clock.still());
        // EndCrystal's constructor takes an id from the entity counter, so
        // the crystal is an entity a command can name (`@e[type=end_crystal]`).
        self.server.next_entity += 1;
        self.common
            .crystals
            .push(Crystal::new(self.server.next_entity, at, false, time));
        // EndCrystalItem.useOn: ENTITY_PLACE at the block above, by the player.
        self.post_block_game_event("minecraft:entity_place", above, self.sculk_actor(), None);
        self.common
            .world
            .modified
            .insert((above[0] >> 4, above[2] >> 4));
        if self.dimension_type().has_ender_dragon_fight {
            self.try_respawn();
        }
        if !self.common.creative {
            self.consume_held();
        }
        // ItemStack.useOn counts the use.
        self.award_stat("used", &stack.item, 1);
        self.swing_arm();
        true
    }
    /// `EndCrystal.kill` (`/kill`): the crystal is removed, without a blast
    /// (`Entity.kill` does not hurt it).
    pub(in crate::game::server_side) fn kill_crystal(&mut self, id: u32) {
        if let Some(i) = self.common.crystals.iter().position(|c| c.id == id) {
            let c = self.common.crystals[i].clone();
            let respawn = self.server.fight.respawn.is_some()
                && self
                    .server
                    .fight
                    .respawn_crystals
                    .iter()
                    .any(|p| p.distance_squared(c.position) < 0.01);
            self.crystal_destroyed(i, &c, None, respawn);
            self.common.crystals.retain(|crystal| crystal.id != id);
            self.entity_killed_event([c.position.x, c.position.y, c.position.z], false);
        }
    }
    /// Any entity touching `room` (`Level.getEntities(null, box)`).
    pub(in crate::game) fn server_entity_in(&self, room: Aabb) -> bool {
        let touches = |lo: Vec3, hi: Vec3| {
            let (a, b) = (room.min_position(), room.max_position());
            lo.x < b.x && hi.x > a.x && lo.y < b.y && hi.y > a.y && lo.z < b.z && hi.z > a.z
        };
        // Level.getEntities: the host's player and every guest in the level.
        let player = self
            .fight_players()
            .iter()
            .any(|p| !p.spectator && touches(p.bounds.min_position(), p.bounds.max_position()));
        player
            || self.common.crystals.iter().any(|c| {
                touches(
                    Vec3::new(c.position.x - 1.0, c.position.y, c.position.z - 1.0),
                    Vec3::new(c.position.x + 1.0, c.position.y + 2.0, c.position.z + 1.0),
                )
            })
            || self.common.mobs.iter().any(|m| {
                let w = f64::from(m.width()) / 2.0;
                let h = f64::from(m.height());
                touches(
                    Vec3::new(m.position.x - w, m.position.y, m.position.z - w),
                    Vec3::new(m.position.x + w, m.position.y + h, m.position.z + w),
                )
            })
    }
    /// `EnderDragonFight.tryRespawn`: a crystal on each side of the portal.
    pub(in crate::game::server_side) fn try_respawn(&mut self) -> bool {
        self.load_fight();
        if !self.server.fight.killed || self.server.fight.respawn.is_some() {
            return false;
        }
        let origin = match self.server.fight.exit_portal {
            Some(o) => o,
            None => {
                let _ = self.activate_exit_portal();
                match self.server.fight.exit_portal {
                    Some(o) => o,
                    None => return false,
                }
            }
        };
        let mut found = Vec::new();
        for d in [[0, -1], [0, 1], [-1, 0], [1, 0]] {
            // new AABB(center.relative(direction, 3)): the block cube.
            let cell = [origin[0] + d[0] * 3, origin[1] + 1, origin[2] + d[1] * 3];
            let lo = Vec3::new(f64::from(cell[0]), f64::from(cell[1]), f64::from(cell[2]));
            let hits: Vec<Vec3> = self
                .common
                .crystals
                .iter()
                .filter(|c| {
                    let p = c.position;
                    p.x + 1.0 > lo.x
                        && p.x - 1.0 < lo.x + 1.0
                        && p.y + 2.0 > lo.y
                        && p.y < lo.y + 1.0
                        && p.z + 1.0 > lo.z
                        && p.z - 1.0 < lo.z + 1.0
                })
                .map(|c| c.position)
                .collect();
            if hits.is_empty() {
                return false;
            }
            found.extend(hits);
        }
        // respawnDragon: the podium goes back to end stone, then inactive.
        for y in origin[1] - 1..=origin[1] + 3 {
            for z in origin[2] - 3..=origin[2] + 3 {
                for x in origin[0] - 3..=origin[0] + 3 {
                    let name = self.block_name_at([x, y, z]);
                    if matches!(&*name, "minecraft:bedrock" | "minecraft:end_portal") {
                        let _ = self.set_state_named([x, y, z], "minecraft:end_stone", &[]);
                    }
                }
            }
        }
        let _ = self.place_podium(origin, false);
        self.server.fight.respawn = Some((Respawn::Start, 0));
        self.server.fight.respawn_crystals = found;
        self.server.fight.dirty = true;
        self.native_dragon_respawn_stage(self.fight_level(), DragonRespawnStage::Start);
        true
    }
    pub(in crate::game::server_side) fn block_name_at(&self, p: [i32; 3]) -> std::sync::Arc<str> {
        self.common
            .res
            .facts()
            .get(self.common.world.block(p[0], p[1], p[2]))
            .name
            .clone()
    }
    pub(in crate::game::server_side) fn respawn_beams(&mut self, target: Option<[i32; 3]>) {
        let list = self.server.fight.respawn_crystals.clone();
        for c in &mut self.common.crystals {
            if list.iter().any(|p| p.distance_squared(c.position) < 0.01) {
                c.beam = target;
            }
        }
    }
    pub(in crate::game::server_side) fn set_respawn(&mut self, stage: Respawn) {
        self.server.fight.respawn = Some((stage, 0));
        self.server.fight.dirty = true;
        self.native_dragon_respawn_stage(self.fight_level(), stage.api());
    }
    /// `EnderDragonFight.tick` with a respawn in progress.
    pub(in crate::game::server_side) fn tick_respawn(&mut self) {
        let Some((stage, time)) = self.server.fight.respawn else {
            return;
        };
        let alive = self.server.fight.respawn_crystals.iter().any(|p| {
            self.common
                .crystals
                .iter()
                .any(|c| p.distance_squared(c.position) < 0.01)
        });
        if !alive {
            self.abort_respawn();
            return;
        }
        if let Some(r) = self.server.fight.respawn.as_mut() {
            r.1 += 1;
        }
        const CENTER: [i32; 3] = [0, 128, 0];
        match stage {
            Respawn::Start => {
                self.respawn_beams(Some(CENTER));
                self.set_respawn(Respawn::Preparing);
            }
            Respawn::Preparing => {
                if time < 100 {
                    if matches!(time, 0 | 50..=52) || time >= 95 {
                        self.respawn_growl();
                    }
                } else {
                    self.set_respawn(Respawn::Pillars);
                }
            }
            Respawn::Pillars => {
                let (start, end) = (time % 40 == 0, time % 40 == 39);
                if !(start || end) {
                    return;
                }
                let spikes =
                    sunrise_world::worldgen::end_spike::spikes_for_seed(self.common.level.seed);
                let index = (time / 40) as usize;
                if let Some(spike) = spikes.get(index) {
                    if start {
                        self.respawn_beams(Some([
                            spike.center_x,
                            spike.height + 1,
                            spike.center_z,
                        ]));
                    } else {
                        self.rebuild_spike(spike);
                    }
                } else if start {
                    self.set_respawn(Respawn::Summoning);
                }
            }
            Respawn::Summoning => {
                if time >= 100 {
                    self.finish_respawn();
                } else if time >= 80 || (1..5).contains(&time) {
                    self.respawn_growl();
                } else if time == 0 {
                    self.respawn_beams(Some(CENTER));
                }
            }
        }
    }
    /// SUMMONING_PILLARS at the end of a beam: the spike is cleared, blown up
    /// and grown again with a guarded crystal.
    pub(in crate::game::server_side) fn rebuild_spike(
        &mut self,
        spike: &sunrise_world::worldgen::end_spike::EndSpike,
    ) {
        let (cx, cz, h) = (spike.center_x, spike.center_z, spike.height);
        let air = self
            .common
            .pack
            .assets
            .registry
            .default_state("minecraft:air");
        let Some(air) = air else {
            return;
        };
        for y in h - 10..=h + 10 {
            for z in cz - 10..=cz + 10 {
                for x in cx - 10..=cx + 10 {
                    if !self
                        .common
                        .res
                        .facts()
                        .is_air(self.common.world.block(x, y, z))
                    {
                        let _ = self.level_set([x, y, z], air);
                    }
                }
            }
        }
        // DragonRespawnStage.SUMMONING_PILLARS: ExplosionInteraction.BLOCK.
        let interaction =
            self.destroy_type(sunrise_game_data::gamerules::vanilla::BLOCK_EXPLOSION_DROP_DECAY);
        let _ = self.server_explode(
            [f64::from(cx) + 0.5, f64::from(h), f64::from(cz) + 0.5],
            5.0,
            interaction,
            false,
        );
        // EndSpikeFeature.placeSpike with crystal_invulnerable and the beam.
        let r = spike.radius;
        for z in cz - r..=cz + r {
            for y in 0..=h + 10 {
                for x in cx - r..=cx + r {
                    let (dx, dz) = (f64::from(x - cx), f64::from(z - cz));
                    if dx * dx + dz * dz <= f64::from(r * r + 1) && y < h {
                        if &*self.block_name_at([x, y, z]) != "minecraft:obsidian" {
                            let _ = self.set_state_named([x, y, z], "minecraft:obsidian", &[]);
                        }
                    } else if y > 65
                        && !self
                            .common
                            .res
                            .facts()
                            .is_air(self.common.world.block(x, y, z))
                    {
                        let _ = self.level_set([x, y, z], air);
                    }
                }
            }
        }
        if spike.guarded {
            for dx in -2i32..=2 {
                for dz in -2i32..=2 {
                    for dy in 0..=3 {
                        let top = dy == 3;
                        if !(dx.abs() == 2 || dz.abs() == 2 || top) {
                            continue;
                        }
                        let x_edge = dx.abs() == 2 || top;
                        let z_edge = dz.abs() == 2 || top;
                        self.set_bars(
                            [cx + dx, h + dy, cz + dz],
                            [
                                x_edge && dz != -2,
                                x_edge && dz != 2,
                                z_edge && dx != -2,
                                z_edge && dx != 2,
                            ],
                        );
                    }
                }
            }
        }
        let at = Vec3::new(f64::from(cx) + 0.5, f64::from(h + 1), f64::from(cz) + 0.5);
        self.common
            .crystals
            .retain(|c| c.position.distance_squared(at) > 0.01);
        let (drawn, _) = Crystal::own_draws();
        let time = Crystal::start_time(at, drawn, self.common.clock.still());
        self.server.next_entity += 1;
        let crystal_id = self.server.next_entity;
        self.common.crystals.push(Crystal {
            beam: Some([0, 128, 0]),
            invulnerable: true,
            ..Crystal::new(crystal_id, at, true, time)
        });
        let _ = self.set_state_named([cx, h, cz], "minecraft:bedrock", &[]);
        let _ = self.set_state_named([cx, h + 1, cz], "minecraft:fire", &[]);
        self.common.world.modified.insert((cx >> 4, cz >> 4));
    }
    /// Iron bars connected north, south, west and east.
    pub(in crate::game::server_side) fn set_bars(&mut self, pos: [i32; 3], sides: [bool; 4]) {
        let registry = self.common.pack.assets.registry;
        let Some(mut state) = registry
            .default_state("minecraft:iron_bars")
            .and_then(|d| registry.state(d).cloned())
        else {
            return;
        };
        for (name, on) in ["north", "south", "west", "east"].into_iter().zip(sides) {
            state
                .properties
                .insert(name.into(), if on { "true" } else { "false" }.into());
        }
        if let Ok(id) = registry.resolve(&state) {
            let _ = self.level_set(pos, id);
        }
    }
    /// `resetSpikeCrystals`: spike crystals lose their beams and protection.
    pub(in crate::game::server_side) fn reset_spike_crystals(&mut self) {
        let spikes = sunrise_world::worldgen::end_spike::spikes_for_seed(self.common.level.seed);
        for c in &mut self.common.crystals {
            let on_spike = spikes.iter().any(|s| {
                (c.position.x - f64::from(s.center_x) - 0.5).abs() <= f64::from(s.radius)
                    && (c.position.z - f64::from(s.center_z) - 0.5).abs() <= f64::from(s.radius)
                    && c.position.y >= f64::from(s.height)
            });
            if on_spike {
                c.invulnerable = false;
                c.beam = None;
            }
        }
    }
    /// `EnderDragonFight.createNewDragon`: over the podium, holding pattern, a random yaw.
    /// It becomes the fight's dragon (`dragonUUID = dragon.getUUID()`).
    pub fn create_new_dragon(&mut self) {
        let yaw = self.common.world.random.next_float() * 360.0;
        let seed = random_source::unique_seed();
        let mut dragon = Dragon::new(Vec3::new(0.0, 128.0, 0.0), yaw, seed);
        // The fight's snapTo sets only the view yaw, not the new living angles.
        dragon.body_yaw = 0.0;
        dragon.head_yaw = 0.0;
        dragon.join = sunrise_sim::join_seq::JoinSeq::next();
        dragon.id = self.server.next_entity + 1;
        dragon.phase = Phase::Holding {
            path: None,
            target: None,
            clockwise: false,
        };
        self.server.next_entity += 9;
        let id = dragon.id;
        assert!(self.dragon.install(dragon).is_ok());
        self.server.fight.dragon_uuid = Some(self.uuid_for(id));
        self.server.fight.dirty = true;
    }
    /// `EnderDragonFight.findOrCreateDragon`: the End's dragon becomes the
    /// fight's, or a new one is made.
    pub(in crate::game::server_side) fn find_or_create_dragon(&mut self) {
        match self.common.dragon.as_ref().map(|d| (d.id, d.alive())) {
            None => self.create_new_dragon(),
            Some((id, true)) => {
                self.server.fight.dragon_uuid = Some(self.uuid_for(id));
                self.server.fight.dirty = true;
            }
            // `ServerLevel.getDragons` takes only living dragons, so vanilla
            // makes a new one beside a dying one; with one dragon to a level,
            // the new one comes once the dying one is gone.
            Some((_, false)) => {}
        }
    }
    /// `EnderDragonFight.scanState`, for a world from before the fight was
    /// saved: an active exit portal means the dragon was killed before. A
    /// living dragon there is the fight's, unless there is no active portal:
    /// then it is removed for a new one.
    pub fn scan_state(&mut self) {
        let active = self.end_portal_active();
        self.server.fight.previously_killed = active;
        match self
            .common
            .dragon
            .as_ref()
            .filter(|d| d.alive())
            .map(|d| d.id)
        {
            None => self.server.fight.killed = true,
            Some(id) => {
                self.server.fight.dragon_uuid = Some(self.uuid_for(id));
                self.server.fight.killed = false;
                if !active {
                    self.discard_resident_dragon();
                    self.server.fight.dragon_uuid = None;
                }
            }
        }
        if !self.server.fight.previously_killed && self.server.fight.killed {
            self.server.fight.killed = false;
        }
    }
    /// The dragon is the End fight's own (`EnderDragon.dragonFight`, set
    /// only for the dragon whose UUID is the fight's `dragonUUID`).
    pub(in crate::game::server_side) fn is_fight_dragon(&mut self) -> bool {
        if !self.dimension_type().has_ender_dragon_fight {
            return false;
        }
        match (
            self.common.dragon.as_ref().map(|d| d.id),
            self.server.fight.dragon_uuid,
        ) {
            (Some(id), Some(uuid)) if id != 0 => self.uuid_for(id) == uuid,
            _ => false,
        }
    }
    /// The END stage: the portal crystals blow up (no block damage) and a
    /// new dragon takes over.
    pub(in crate::game::server_side) fn finish_respawn(&mut self) {
        // Portal crystals are not on spikes; clear their revival beams before
        // the spike reset and before they are removed (vanilla resetEndCrystals).
        self.respawn_beams(None);
        self.reset_spike_crystals();
        self.server.fight.respawn = None;
        let list = std::mem::take(&mut self.server.fight.respawn_crystals);
        for p in list {
            if let Some(i) = self
                .common
                .crystals
                .iter()
                .position(|c| c.position.distance_squared(p) < 0.01)
            {
                let c = self.common.crystals.remove(i);
                self.common.world.modified.insert((
                    c.position.x.floor() as i32 >> 4,
                    c.position.z.floor() as i32 >> 4,
                ));
                let _ = self.server_explode_from(
                    [c.position.x, c.position.y, c.position.z],
                    6.0,
                    super::explosion::BlockInteraction::Keep,
                    false,
                    super::explosion::ExplosionSource::default()
                        .with_silence(self.entity_silent(c.id))
                        .with_entity(
                            c.id,
                            "minecraft:end_crystal",
                            sunrise_sim::health::DirectEntity::Other,
                        ),
                );
            }
        }
        self.server.fight.killed = false;
        // setRespawnStage(END): the new dragon is summoned for every player
        // of the fight (the boss bar's players: alive within 192 blocks).
        self.create_new_dragon();
        let view = self
            .common
            .dragon
            .as_ref()
            .map(|d| sunrise_sim::advancements::EntityView {
                kind: "minecraft:ender_dragon".to_owned(),
                position: [d.position.x, d.position.y, d.position.z],
                ..Default::default()
            });
        if let Some(view) = view {
            self.for_each_player(|g| {
                let near = g.player.as_ref().is_some_and(|p| {
                    !p.health.dead_or_dying()
                        && p.position.x.powi(2)
                            + (p.position.y - 128.0).powi(2)
                            + p.position.z.powi(2)
                            <= 192.0 * 192.0
                });
                if near {
                    g.trigger_entity("minecraft:summoned_entity", "entity", &view);
                }
            });
        }
        self.server.fight.dirty = true;
        self.count_crystals();
        self.native_dragon_respawn_stage(self.fight_level(), DragonRespawnStage::End);
    }
    /// `abortRespawnSequence`.
    pub(in crate::game::server_side) fn abort_respawn(&mut self) {
        self.server.fight.respawn = None;
        self.respawn_beams(None);
        self.server.fight.respawn_crystals.clear();
        self.reset_spike_crystals();
        let _ = self.activate_exit_portal();
        self.server.fight.dirty = true;
    }
}

/// The dragon on a server's client (`ClientPacketListener.handleAddEntity`
/// and the client side of `EnderDragon.aiStep`): the server flies it and
/// sends its moves, health and phase; the client flaps its wings, keeps
/// its flight history and places its parts, so it can be drawn and hit.
impl crate::game::ServerGame<'_> {
    /// `EntityType.create` for an egg dragon, or `SummonCommand.createEntity` without NBT.
    /// Unlike the End fight's dragon, this
    /// one is not the fight's (`EnderDragon.aiStep` adopts it only when its UUID is the fight's),
    /// so it can be spawned in any dimension. Sunrise keeps one dragon per level, as `/summon`
    /// and loading do: with one already there, nothing spawns and the egg stays.
    pub fn spawn_dragon_spawned(&mut self, at: [f64; 3], site: SpawnSite) -> Option<u32> {
        self.spawn_dragon_facing(at, site, None)
    }
    /// [`spawn_dragon_spawned`](Self::spawn_dragon_spawned) snapped to `fixed_yaw` when
    /// given (`snapTo(pos, yaw, 0)`), which takes no draw from the level.
    pub fn spawn_dragon_facing(
        &mut self,
        at: [f64; 3],
        site: SpawnSite,
        fixed_yaw: Option<f32>,
    ) -> Option<u32> {
        if self.common.dragon.is_some() {
            return None;
        }
        let position = Vec3::new(at[0], at[1], at[2]);
        // SummonCommand loads absent Rotation as zero before finalization. Only
        // EntityType.create's egg placement draws a random snapTo yaw.
        // Diverges from upstream: shared mod finalization also admits command dragons.
        let yaw = match (fixed_yaw, site) {
            (Some(yaw), _) => yaw,
            (None, SpawnSite::Finalizing(sunrise_sim::mobs::spawn_rules::Reason::Command)) => 0.0,
            _ => sunrise_level::camera_math::wrap_degrees(
                self.common.world.random.next_float() * 360.0,
            ),
        };
        let finalized = match site {
            SpawnSite::Finalizing(reason) => {
                self.finalize_spawn_at("minecraft:ender_dragon", at, reason, None)
                    .0
            }
            SpawnSite::Plain | SpawnSite::Raw => Default::default(),
        };
        if finalized.veto_admission {
            return None;
        }
        if !finalized.skip_initialization {
            // Mob.finalizeSpawn (EnderDragon keeps it): the follow range's
            // triangle and left-handed roll, neither of which changes how the dragon flies.
            let _ = self
                .common
                .world
                .random
                .triangle(0.0, 0.114_850_000_000_000_01);
            let _ = self.common.world.random.next_float();
        }
        // The dragon's own random (`Entity.random`), not the level's.
        let seed = random_source::unique_seed();
        let mut dragon = Dragon::new(position, yaw, seed);
        dragon.join = sunrise_sim::join_seq::JoinSeq::next();
        dragon.id = self.server.next_entity + 1;
        self.server.next_entity += 9;
        let id = dragon.id;
        // Finalization listeners can install another dragon after the initial
        // refusal check. Keep the constructor's existing replacement order.
        self.replace_resident_dragon(dragon);
        // Entity's constructor assigns the identity before any join callbacks or hits.
        self.uuid_for(id);
        self.common.place_dragon_parts();
        self.native_joins_flush(false, id).then_some(id)
    }
}

#[path = "dragon/crystal.rs"]
pub(in crate::game) mod crystal;

/// `DragonSittingScanningPhase.doServerTick` with a player at `target` and 25 ticks or fewer of
/// scanning: the dragon turns only while that player is more than 10 degrees off its heading
/// (`Math.acos` of the float dot, in degrees, plus 0.5). `yRotA` then eases by 0.8 and gains the
/// delta (`180 - Mth.atan2` in degrees less the yaw, wrapped, held within 100) over the distance
/// from the head plus one capped at 40 and over that distance uncapped, and the yaw takes it.
pub(in crate::game::server_side) fn scanning_turn(
    yaw: &mut f32,
    yaw_accel: &mut f32,
    body: Vec3,
    head: Vec3,
    target: Vec3,
) {
    let aim = vanilla_normalize(glam::DVec3::new(target.x - body.x, 0.0, target.z - body.z));
    let r = f64::from(*yaw * trig::DEG_TO_RAD);
    let dir = vanilla_normalize(glam::DVec3::new(
        f64::from(trig::sin(r)),
        0.0,
        f64::from(-trig::cos(r)),
    ));
    let dot = dir.dot(aim) as f32;
    let angle =
        (sunrise_core::jmath::acos(f64::from(dot)) * f64::from(trig::RAD_TO_DEG)) as f32 + 0.5;
    if angle < 0.0 || angle > 10.0 {
        let (xd, zd) = (target.x - head.x, target.z - head.z);
        let delta =
            wrap64(180.0 - trig::atan2(xd, zd) * f64::from(trig::RAD_TO_DEG) - f64::from(*yaw))
                .clamp(-100.0, 100.0);
        *yaw_accel *= 0.8;
        let rot_speed = (xd * xd + zd * zd).sqrt() as f32 + 1.0;
        let dist = if rot_speed > 40.0 { 40.0 } else { rot_speed };
        *yaw_accel += delta as f32 * (0.7 / dist / rot_speed);
        *yaw += *yaw_accel;
    }
}

impl crate::game::ServerGame<'_> {
    /// `Mob.playAmbientSound` when EntityType.spawn places a dragon.
    pub(in crate::game::server_side) fn dragon_spawn_ambient(&mut self, id: u32) -> bool {
        let Some(d) = self.common.dragon.as_mut().filter(|d| d.id == id) else {
            return false;
        };
        let at = [d.position.x, d.position.y, d.position.z];
        let pitch = (d.random.next_float() - d.random.next_float()) * 0.2 + 1.0;
        self.server_play_sound("minecraft:entity.ender_dragon.ambient", at, 5.0, pitch);
        true
    }
    /// `LevelEvent 3001`: the dragon's growl over the island.
    pub(in crate::game) fn respawn_growl(&mut self) {
        let pitch = 0.8 + self.common.world.random.next_float() * 0.3;
        self.server_play_sound(
            "minecraft:entity.ender_dragon.growl",
            [0.5, 128.5, 0.5],
            64.0,
            pitch,
        );
    }
    /// `DragonStrafePlayerPhase`'s shot: `new DragonFireball(level, dragon,
    /// direction.normalize())`, whose `AbstractHurtingProjectile` constructor
    /// takes the dragon as its owner and `assignDirectionalMovement`s the
    /// direction normalized again times `accelerationPower` (0.1), then
    /// `snapTo` the start. It joins the level's other projectiles.
    pub fn spawn_dragon_fireball(&mut self, at: Vec3, dir: Vec3) {
        // Vec3.normalize: a vector shorter than 1e-5 is zero.
        let normalize = |v: Vec3| {
            let l = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
            if l < 1.0e-5 {
                Vec3::new(0.0, 0.0, 0.0)
            } else {
                Vec3::new(v.x / l, v.y / l, v.z / l)
            }
        };
        let n = normalize(normalize(dir));
        self.server_play_sound(
            "minecraft:entity.ender_dragon.shoot",
            [at.x, at.y, at.z],
            10.0,
            1.0,
        );
        let owner = self.common.dragon.as_ref().map(|d| Who::Mob(d.id));
        let mut p = sunrise_sim::projectiles::Projectile::new(
            sunrise_sim::projectiles::ProjectileKind::DragonFireball,
            at,
            owner,
        );
        p.acceleration = 0.1;
        p.velocity = Vec3::new(n.x * 0.1, n.y * 0.1, n.z * 0.1);
        self.add_fresh_projectile(p);
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::tables::GameItems as _;
use sunrise_gameplay_common::game::transactions::{
    TransactionsGameExt as _, entities::EntityRemovalGameExt as _,
};

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::dragon_common::DragonCommonGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;

use sunrise_gameplay_common::game::scoreboard_common::CommonGameExt1 as _;

#[cfg(test)]
pub(in crate::game) mod tests {
    use sunrise_gameplay_common::game::server_side::dragon::tests::HIT;

    use sunrise_gameplay_common::game::server_side::dragon::tests::dragon_as;

    use sunrise_gameplay_common::game::server_side::dragon::tests::flying;

    use sunrise_gameplay_common::game::server_side::dragon::tests::hooks;

    pub fn player(who: FightWho, x: f64, creative: bool, spectator: bool) -> FightPlayer {
        let position = Vec3::new(x, 64.0, 0.0);
        FightPlayer {
            who,
            position,
            bounds: Aabb::new(position, Vec3::new(x + 0.6, 65.8, 0.6)),
            alive: true,
            creative,
            spectator,
        }
    }

    use sunrise_gameplay_common::game::server_side::dragon::tests::player_box;

    use sunrise_gameplay_common::game::server_side::dragon::tests::dragon;

    use sunrise_gameplay_common::game::server_side::dragon::tests::seen;

    use super::*;
    use sunrise_gameplay_common::game::dragon_common::{dragon_part_index, dragon_part_wire_id};

    /// The pinned vanilla 26.3 side is `VanillaTurnOracle`'s `dragon.txt`
    /// (`tools/test_mob_turn_parity.py`, recorded into `.local/witness/26.3/mob_turn/dragon.txt`):
    /// `DragonSittingScanningPhase.doServerTick` with its scan's player at each place, from each
    /// yaw, `yRotA` and head, as the yaw and `yRotA` bits it leaves.
    #[test]
    #[ignore = "run by tools/test_mob_turn_parity.py"]
    fn the_scanning_turn_matches_the_pinned_oracle() {
        run_the_scanning_turn_matches_the_pinned_oracle();
    }

    pub(in crate::game::server_side) fn run_the_scanning_turn_matches_the_pinned_oracle() {
        let witness_dir = std::env::var_os("SUNRISE_MOB_TURN_WITNESS_DIR").expect(
            "SUNRISE_MOB_TURN_WITNESS_DIR must be set; run through tools/test_mob_turn_parity.py, which records the witness",
        );
        let witness_path = std::path::Path::new(&witness_dir).join("dragon.txt");
        let witness = std::fs::read_to_string(&witness_path).unwrap_or_else(|err| {
            panic!(
                "failed to read {} ({err}); run through tools/test_mob_turn_parity.py, which records the witness",
                witness_path.display()
            )
        });
        let bits = |v: f64| format!("{:016x}", v.to_bits());
        let mut rows = Vec::new();
        let places = [
            [4.0, -3.0],
            [0.0, 30.0],
            [-35.0, 25.0],
            [50.0, 0.0],
            [0.5, -0.5],
            [12.0, 12.0],
            [-1.0, -60.0],
        ];
        for [px, pz] in places {
            for yaw in [45.0f32, 0.0, -170.0, 359.5, 180.0] {
                for accel in [0.0f32, 2.5, -7.25] {
                    for [hx, hz] in [[0.0, 0.0], [6.0, -4.0]] {
                        let (mut y, mut a) = (yaw, accel);
                        super::scanning_turn(
                            &mut y,
                            &mut a,
                            Vec3::new(0.0, 70.0, 0.0),
                            Vec3::new(hx, 72.0, hz),
                            Vec3::new(px, 70.0, pz),
                        );
                        rows.push(format!(
                            "{} {} {:08x} {:08x} {} {} {:08x} {:08x}",
                            bits(px),
                            bits(pz),
                            yaw.to_bits(),
                            accel.to_bits(),
                            bits(hx),
                            bits(hz),
                            y.to_bits(),
                            a.to_bits()
                        ));
                    }
                }
            }
        }
        let actual = rows.join("\n") + "\n";
        if let Some(dir) = std::env::var_os("SUNRISE_TURN_OUTPUT_DIR") {
            std::fs::write(std::path::Path::new(&dir).join("dragon.txt"), &actual).unwrap();
        }
        assert_eq!(rows.len(), 7 * 5 * 3 * 2);
        assert_eq!(actual, witness);
    }

    #[test]
    fn dragon_part_wire_ids_wrap_like_vanilla_i32() {
        let neg_one = (-1i32) as u32;
        for part in 0..8 {
            let wire = dragon_part_wire_id(neg_one, part);
            assert_eq!(wire, part as i32);
            assert_eq!(dragon_part_index(neg_one, wire as u32), Some(part));
        }
        assert_eq!(dragon_part_index(neg_one, (-1i32) as u32), None);
        assert_eq!(dragon_part_index(neg_one, 8), None);

        let max = i32::MAX as u32;
        assert_eq!(dragon_part_wire_id(max, 0), i32::MIN);
        assert_eq!(dragon_part_index(max, i32::MIN as u32), Some(0));
        assert_eq!(dragon_part_wire_id(max, 7), i32::MIN + 7);
        assert_eq!(dragon_part_index(max, (i32::MIN + 7) as u32), Some(7));
    }

    #[test]
    fn dragon_search_preserves_equal_cost_heap_order() {
        let xz = [
            [60, -1],
            [51, 29],
            [29, 51],
            [0, 60],
            [-30, 51],
            [-52, 30],
            [-60, 0],
            [-52, -31],
            [-31, -52],
            [-1, -60],
            [29, -52],
            [51, -30],
            [40, -1],
            [28, 28],
            [0, 40],
            [-29, 28],
            [-40, 0],
            [-29, -29],
            [-1, -40],
            [28, -29],
            [20, -1],
            [0, 20],
            [-20, 0],
            [-1, -20],
        ];
        let heights = [
            83, 88, 75, 82, 86, 82, 87, 88, 83, 75, 78, 74, 80, 77, 79, 73, 74, 82, 74, 73, 85, 75,
            75, 77,
        ];
        let nodes = std::array::from_fn(|i| [xz[i][0], heights[i], xz[i][1]]);
        let path = search_dragon_path(&nodes, 18, 6, 0, None).unwrap();
        // With crystals alive, the ruled array heap visits node 16 before 7 at a tie.
        assert_eq!(path.nodes, [nodes[18], nodes[17], nodes[16], nodes[6]]);
    }

    #[test]
    fn a_dragon_save_keeps_equipment_only_while_something_is_equipped() {
        use sunrise_items::equipment::{BODY, SADDLE};
        use sunrise_items::items::ItemStack;
        let mut d = Dragon::new(Vec3::ZERO, 0.0, 0);
        // An old save has no `equipment` key: the dragon loads empty and saves none.
        let old = d.to_nbt([0; 4], None, &Default::default());
        assert!(old.get("equipment").is_none());
        assert!(Dragon::from_nbt(&old, 0).unwrap().equipment.is_empty());
        // Body and saddle round-trip with their name and damage.
        let mut body = ItemStack::new("minecraft:diamond_horse_armor", 1);
        body.damage = 6;
        body.data_mut().custom_name = Some(Tag::String("Plate".into()));
        d.equipment[BODY] = Some(body);
        d.equipment[SADDLE] = Some(ItemStack::new("minecraft:saddle", 1));
        let saved = d.to_nbt([0; 4], None, &Default::default());
        assert_eq!(Dragon::from_nbt(&saved, 0).unwrap().equipment, d.equipment);
        // Saving over a raw save that holds a stale key removes it once emptied.
        d.equipment.take_all().for_each(drop);
        assert!(
            d.to_nbt([0; 4], Some(&saved), &Default::default())
                .get("equipment")
                .is_none()
        );
    }

    /// `Entity.load` and `EndCrystal.readAdditionalSaveData` read their booleans with
    /// `getBooleanOr`: any number, `ShowBottom` true when absent.
    #[test]
    fn a_crystal_reads_its_flags_from_any_number() {
        let crystal = |fields: Vec<(&str, Tag)>| {
            let pos = [0.5f64, 64.0, 0.5].map(|v| Tag::Double(v.to_bits()));
            let mut all = vec![("Pos", Tag::List(pos.to_vec()))];
            all.extend(fields);
            Crystal::from_nbt(&Tag::compound(all), 0).unwrap()
        };
        let c = crystal(vec![
            ("Invulnerable", Tag::Byte(2)),
            ("ShowBottom", Tag::Int(0)),
        ]);
        assert!(c.invulnerable && !c.show_bottom);
        let c = crystal(Vec::new());
        assert!(!c.invulnerable && c.show_bottom);
    }

    /// Under `SUNRISE_STILL` a crystal's spin and bob start from its position, whichever order
    /// the crystals load in; without it each takes the next draw, as vanilla's do.
    #[test]
    fn a_still_crystal_starts_from_its_position_in_any_load_order() {
        let spikes = [Vec3::new(42.5, 77.0, 0.5), Vec3::new(-33.5, 95.0, 24.5)];
        let starts = |order: [usize; 2], still: bool| {
            let mut random = LegacyRandom::new(5);
            let mut times = [0; 2];
            for i in order {
                let drawn = random.next_int_bound(100_000).unwrap();
                times[i] = Crystal::start_time(spikes[i], drawn, still);
            }
            times
        };
        let still = starts([0, 1], true);
        assert_eq!(still, starts([1, 0], true));
        assert_ne!(still[0], still[1], "each crystal keeps its own phase");
        let first = LegacyRandom::new(5).next_int_bound(100_000).unwrap();
        assert_eq!(starts([0, 1], false)[0], first);
        assert_eq!(
            starts([1, 0], false)[1],
            first,
            "the first loaded takes the first draw"
        );
    }
    /// `Mth.wrapDegrees`: `% 360` then one step of 360, exact for a negative angle
    /// (a remainder made positive and back again loses the float's low bits).
    #[test]
    fn wrap_is_mth_wrap_degrees() {
        for (d, wrapped) in [
            (-10.123456_f32, -10.123456_f32),
            (190.0, -170.0),
            (-190.0, 170.0),
            (540.25, 180.25 - 360.0),
            (-0.0001, -0.0001),
        ] {
            assert_eq!(wrap(d), wrapped, "{d}");
        }
        assert_eq!(wrap64(-10.123456789), -10.123456789);
    }
    /// `EnderDragon.tickDeath` roars (`globalLevelEvent(1028)`) only when
    /// `!isSilent()`; `Silent` is saved only when set, as a byte.
    #[test]
    fn a_dragon_is_silent_only_with_a_set_silent_byte() {
        use sunrise_core::nbt::{Compound, Tag};
        let with = |v: Option<i8>| {
            let mut map = Compound::new();
            if let Some(b) = v {
                map.insert("Silent".into(), Tag::Byte(b));
            }
            Tag::Compound(map)
        };
        assert!(!super::saved_silent(None));
        assert!(!super::saved_silent(Some(&with(None))));
        assert!(!super::saved_silent(Some(&with(Some(0)))));
        assert!(super::saved_silent(Some(&with(Some(1)))));
    }

    #[test]
    fn breath_lifetime_excludes_the_wait() {
        assert!(!breath_expired(219, 200));
        assert!(breath_expired(220, 200));
        assert!(!breath_expired(619, 600));
        assert!(breath_expired(620, 600));
    }

    #[test]
    fn breath_victims_cadence() {
        let (a, b) = (FightWho::Host, FightWho::Guest(1));
        let mut v = Vec::new();
        assert!(breath_victims(15, &mut v, [a]).is_empty());
        assert_eq!(breath_victims(20, &mut v, [a]), vec![a]);
        assert!(breath_victims(25, &mut v, [a]).is_empty());
        // A newcomer is hit at the next 5-tick check, not the next 20.
        assert_eq!(breath_victims(30, &mut v, [a, b]), vec![b]);
        assert!(breath_victims(35, &mut v, [a, b]).is_empty());
        assert_eq!(breath_victims(40, &mut v, [a, b]), vec![a]);
        assert_eq!(breath_victims(50, &mut v, [a, b]), vec![b]);
    }

    #[test]
    fn instant_damage_at_cloud_scale() {
        assert_eq!(harm_amount(0), 3.0);
        assert_eq!(harm_amount(1), 6.0);
    }

    #[test]
    fn cloud_box_is_half_a_block_tall_from_its_floor() {
        let at = [0.0, 64.0, 0.0];
        let hit = |feet: [f64; 3], r: f32| {
            let (b, p) = player_box(feet);
            cloud_hits(at, r, b, p)
        };
        assert!(hit([0.0, 64.0, 0.0], 3.0));
        // Feet above the 0.5 box: the old |dy| < 1 test hit here.
        assert!(!hit([0.0, 64.6, 0.0], 3.0));
        // Feet below, head reaching into the box.
        assert!(hit([0.0, 63.0, 0.0], 3.0));
        // Box overlaps but the centre is outside the radius.
        assert!(!hit([3.2, 64.0, 0.0], 3.0));
        assert!(hit([3.0, 64.0, 0.0], 3.0));
    }

    #[test]
    fn fireball_cloud_snaps_to_a_player_within_four() {
        let (b, p) = player_box([2.0, 70.0, 1.0]);
        assert_eq!(
            fireball_cloud_origin([0.0, 70.0, 0.0], &[(b, p)]),
            [2.0, 70.0, 1.0]
        );
        let (b, p) = player_box([4.5, 70.0, 0.0]);
        assert_eq!(
            fireball_cloud_origin([0.0, 70.0, 0.0], &[(b, p)]),
            [0.0, 70.0, 0.0]
        );
    }

    #[test]
    fn dying_dragons_keep_their_flight_pose_between_frames() {
        let mut dragon = Dragon::new(Vec3::new(0.0, 80.0, 0.0), 0.0, 432);
        dragon.history.fill((70.0, 170.0));
        dragon.position.y = 90.0;
        dragon.yaw = -170.0;
        dragon.record();
        assert_eq!(dragon.sample_lerp(0, 0.5), (80.0, 180.0));
        dragon.health = 0.0;
        for partial in [0.0, 0.25, 0.5, 0.75, 1.0] {
            assert_eq!(dragon.sample_lerp(0, partial), (70.0, 170.0));
        }
    }

    /// `EnderDragon.readAdditionalSaveData` reads `sitting_damage_received`
    /// (`getFloatOr`, 0 when absent); the writer already saved it. Sunrise
    /// only wrote it, so a perched dragon that took a quarter of its health
    /// but was saved before taking off came back with the damage forgotten.
    #[test]
    fn a_perched_dragons_damage_survives_the_chunk_save() {
        let mut d = dragon(Phase::Flaming { ticks: 0 });
        d.sitting_damage = 12.5;
        let tag = d.to_nbt([1, 2, 3, 4], None, &Default::default());
        let back = Dragon::from_nbt(&tag, 0).unwrap();
        assert_eq!(back.sitting_damage, 12.5);
        // No key at all reads as no damage accumulated.
        let mut bare = tag.clone();
        if let sunrise_core::nbt::Tag::Compound(fields) = &mut bare {
            fields.remove(&"sitting_damage_received".into());
        }
        assert_eq!(Dragon::from_nbt(&bare, 0).unwrap().sitting_damage, 0.0);
    }

    #[test]
    fn a_perched_dragon_shrugs_arrows_off() {
        let mut test_level = sunrise_sim::health::TestLevel::new(0);
        let level = &mut test_level.level();
        // AbstractDragonSittingPhase.onHurt: an arrow does nothing, melee does.
        let mut d = dragon(Phase::Scanning { time: 0 });
        assert_eq!(hurt(&mut d, level, 0, 9.0, HIT, true), Hurt::Shrugged);
        assert_eq!(d.health, MAX_HEALTH);
        assert_eq!(
            hurt(&mut d, level, 0, 9.0, HIT, false),
            Hurt::Landed { full: true }
        );
        assert_eq!(d.health, MAX_HEALTH - 9.0);
        // In flight an arrow lands.
        let mut d = dragon(flying());
        assert_eq!(
            hurt(&mut d, level, 0, 9.0, HIT, true),
            Hurt::Landed { full: true }
        );
        assert_eq!(d.health, MAX_HEALTH - 9.0);
    }

    /// `EnderDragon.knockback` calls `LivingEntity.knockback` only when the
    /// phase is not sitting. `dealDefaultKnockback` uses `0.4F` away from the
    /// attacker's feet. Resistance is 0 and `onGround` stays false
    /// (`noPhysics`), so from velocity `(1, -0.3, 0.5)` and a source 3 by 4
    /// blocks away the push is `(0.5 - 0.6p, -0.3, 0.25 - 0.8p)` with
    /// `p = (double) 0.4F`. A perched hit, including one that makes it take
    /// off, leaves the velocity alone. A hit inside the damage cooldown is
    /// not a full hit, so it does not push either.
    #[test]
    fn a_melee_hit_knocks_a_flying_dragon_and_not_a_perched_one() {
        let mut test_level = sunrise_sim::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let from = Vec3::new(3.0, 70.0, 4.0);
        let velocity = Vec3::new(1.0, -0.3, 0.5);
        let power = f64::from(0.4_f32);
        let pushed = Vec3::new(0.5 - 0.6 * power, -0.3, 0.25 - 0.8 * power);

        let mut perched = dragon(Phase::Scanning { time: 0 });
        perched.velocity = velocity;
        assert_eq!(
            hurt_from(
                &mut perched,
                level,
                0,
                9.0,
                HIT,
                false,
                Some(Push {
                    from,
                    resistance: 0.0
                }),
                false
            ),
            Hurt::Landed { full: true }
        );
        assert_eq!(perched.velocity, velocity);

        let mut taking_off = dragon(Phase::Flaming { ticks: 0 });
        taking_off.velocity = velocity;
        hurt_from(
            &mut taking_off,
            level,
            0,
            60.0,
            HIT,
            false,
            Some(Push {
                from,
                resistance: 0.0,
            }),
            false,
        );
        assert!(matches!(taking_off.phase, Phase::Takeoff { .. }));
        assert_eq!(taking_off.velocity, velocity);

        let mut flying = dragon(flying());
        flying.velocity = velocity;
        assert_eq!(
            hurt_from(
                &mut flying,
                level,
                0,
                9.0,
                HIT,
                false,
                Some(Push {
                    from,
                    resistance: 0.0
                }),
                false
            ),
            Hurt::Landed { full: true }
        );
        assert_eq!(flying.velocity, pushed);
        assert_eq!(
            hurt_from(
                &mut flying,
                level,
                0,
                20.0,
                HIT,
                false,
                Some(Push {
                    from,
                    resistance: 0.0
                }),
                false
            ),
            Hurt::Landed { full: false }
        );
        assert_eq!(flying.velocity, pushed);
    }

    #[test]
    fn a_part_refuses_fire() {
        let mut test_level = sunrise_sim::health::TestLevel::new(0);
        let level = &mut test_level.level();
        // EnderDragonPart.hurtServer: the dragon is fire immune, so a
        // fireball's hit is refused on the head and on the other parts.
        let large = ProjectileKind::LargeFireball { power: 1 };
        for part in [0, 2] {
            let mut d = dragon(flying());
            let hurt = hurt_part(
                &mut d,
                level,
                part,
                6.0,
                HIT,
                DamageKind::Fireball,
                Some(&large),
            );
            assert_eq!(hurt, Hurt::Refused);
            assert_eq!(d.health, MAX_HEALTH);
        }
        let small = ProjectileKind::SmallFireball;
        let mut d = dragon(flying());
        let hurt = hurt_part(
            &mut d,
            level,
            0,
            5.0,
            HIT,
            DamageKind::Fireball,
            Some(&small),
        );
        assert_eq!(hurt, Hurt::Refused);
        assert_eq!(d.health, MAX_HEALTH);
    }

    #[test]
    fn a_perched_dragon_shrugs_a_players_wind_charge_off() {
        let mut test_level = sunrise_sim::health::TestLevel::new(0);
        let level = &mut test_level.level();
        // AbstractDragonSittingPhase.onHurt: a direct WindCharge does
        // nothing, a breeze's BreezeWindCharge is not one.
        let charge = ProjectileKind::WindCharge;
        let mut d = dragon(Phase::Scanning { time: 0 });
        let hurt = hurt_part(
            &mut d,
            level,
            0,
            1.0,
            HIT,
            DamageKind::WindCharge,
            Some(&charge),
        );
        assert_eq!(hurt, Hurt::Shrugged);
        assert_eq!(d.health, MAX_HEALTH);
        let breeze = ProjectileKind::BreezeWindCharge;
        let hurt = hurt_part(
            &mut d,
            level,
            0,
            1.0,
            HIT,
            DamageKind::WindCharge,
            Some(&breeze),
        );
        assert_eq!(hurt, Hurt::Landed { full: true });
        assert_eq!(d.health, MAX_HEALTH - 1.0);
        // In flight the player's charge lands too.
        let mut d = dragon(flying());
        let hurt = hurt_part(
            &mut d,
            level,
            0,
            1.0,
            HIT,
            DamageKind::WindCharge,
            Some(&charge),
        );
        assert_eq!(hurt, Hurt::Landed { full: true });
        assert_eq!(d.health, MAX_HEALTH - 1.0);
    }

    #[test]
    fn only_a_player_owned_wither_skull_takes_the_dragons_health() {
        let mut test_level = sunrise_sim::health::TestLevel::new(0);
        let level = &mut test_level.level();
        // WitherSkull.onHitEntity: EnderDragon.hurt takes the wither's skull
        // (not a player, not an explosion) but takes no health, even perched.
        let skull = ProjectileKind::WitherSkull { dangerous: false };
        let wither = super::super::wither::skull_source(Some(Who::Mob(7)), [0.0; 3]);
        assert_eq!((wither.amount, wither.kind), (8.0, DamageKind::WitherSkull));
        let mut d = dragon(Phase::Scanning { time: 0 });
        let hurt = hurt_part(
            &mut d,
            level,
            2,
            wither.amount,
            None,
            wither.kind,
            Some(&skull),
        );
        assert_eq!(hurt, Hurt::Landed { full: false });
        assert_eq!(d.health, MAX_HEALTH);
        // A player's skull: 8 on the head, 8 / 4 + 1 on the body.
        let player = super::super::wither::skull_source(Some(Who::Player), [0.0; 3]);
        let mut d = dragon(Phase::Scanning { time: 0 });
        let hurt = hurt_part(
            &mut d,
            level,
            0,
            player.amount,
            HIT,
            player.kind,
            Some(&skull),
        );
        assert_eq!(hurt, Hurt::Landed { full: true });
        assert_eq!(d.health, MAX_HEALTH - 8.0);
        let mut d = dragon(flying());
        hurt_part(
            &mut d,
            level,
            2,
            player.amount,
            HIT,
            player.kind,
            Some(&skull),
        );
        assert_eq!(d.health, MAX_HEALTH - 3.0);
    }

    #[test]
    fn the_cooldown_lets_only_the_excess_through() {
        let mut test_level = sunrise_sim::health::TestLevel::new(0);
        let level = &mut test_level.level();
        // LivingEntity.hurtServer: damageCooldownTime > 10 keeps lastHurt.
        let mut d = dragon(flying());
        assert_eq!(
            hurt(&mut d, level, 0, 6.0, HIT, false),
            Hurt::Landed { full: true }
        );
        assert_eq!(
            hurt(&mut d, level, 0, 4.0, HIT, false),
            Hurt::Landed { full: false }
        );
        assert_eq!(d.health, MAX_HEALTH - 6.0);
        assert_eq!(
            hurt(&mut d, level, 0, 10.0, HIT, false),
            Hurt::Landed { full: false }
        );
        assert_eq!(d.health, MAX_HEALTH - 10.0);
        // Ten ticks on, the window is over: a full hit again.
        d.invulnerable = 10;
        assert_eq!(
            hurt(&mut d, level, 0, 3.0, HIT, false),
            Hurt::Landed { full: true }
        );
        assert_eq!(d.health, MAX_HEALTH - 13.0);
    }

    #[test]
    fn parts_and_sources() {
        let mut test_level = sunrise_sim::health::TestLevel::new(0);
        let level = &mut test_level.level();
        // Neither head nor neck: a quarter plus one.
        let mut d = dragon(flying());
        assert_eq!(
            hurt(&mut d, level, 2, 20.0, HIT, false),
            Hurt::Landed { full: true }
        );
        assert_eq!(d.health, MAX_HEALTH - 6.0);
        // Not a player's and not an explosion: true, but no damage.
        let mut d = dragon(flying());
        assert_eq!(
            hurt(&mut d, level, 0, 20.0, None, false),
            Hurt::Landed { full: false }
        );
        assert_eq!(d.health, MAX_HEALTH);
        // Too little after the part's cut, and nothing while dying.
        assert_eq!(hurt(&mut d, level, 3, 0.001, HIT, false), Hurt::Refused);
        d.phase = Phase::Dying {
            target: None,
            time: 0,
        };
        assert_eq!(hurt(&mut d, level, 0, 20.0, HIT, false), Hurt::Refused);
    }

    #[test]
    fn base_check_only_receives_qualifying_dragon_hits() {
        let mut d = dragon(flying());
        assert!(d.reaches_base_check(0, 1.0, true, true));
        assert!(!d.reaches_base_check(0, 1.0, false, false));
        assert!(!d.reaches_base_check(BODY, 0.001, true, false));
        d.set_phase(Phase::Dying {
            target: None,
            time: 0,
        });
        assert!(!d.reaches_base_check(0, 1.0, true, false));
    }

    #[test]
    fn dragon_invulnerability_hook_blocks_the_matching_damage_type() {
        use sunrise_sim::health::{DamageHooks, DamageLevel, Hurting};
        fn hook(_: DamageLevel<'_>, hurting: Hurting<'_>, vanilla: bool) -> bool {
            match hurting {
                Hurting::Dragon(9100, source) => source.damage_type == "minecraft:arrow",
                _ => vanilla,
            }
        }
        let hooks = DamageHooks {
            invulnerable: Some(hook),
            ..DamageHooks::NONE
        };
        let mut test_level = sunrise_sim::health::TestLevel::with_hooks(0, hooks);
        let level = &mut test_level.level();
        let mut d = dragon_as(9100);
        let arrow = HurtSource {
            damage_type: "minecraft:arrow",
            ..HIT.unwrap()
        };
        let generic = HurtSource {
            damage_type: "minecraft:generic",
            ..HIT.unwrap()
        };
        assert_eq!(
            hurt(&mut d, level, 0, 4.0, Some(arrow), false),
            Hurt::Landed { full: false }
        );
        assert_eq!(d.health, MAX_HEALTH);
        assert_eq!(
            hurt(&mut d, level, 0, 4.0, Some(generic), false),
            Hurt::Landed { full: true }
        );
        assert_eq!(d.health, MAX_HEALTH - 4.0);
    }

    #[test]
    fn dragon_part_respects_a_hook_override_of_fire_immunity() {
        let mut test_level = sunrise_sim::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut d = dragon(flying());
        assert_eq!(
            hurt_part_from(
                &mut d,
                level,
                0,
                4.0,
                HIT,
                DamageKind::OnFire,
                None,
                None,
                false
            ),
            Hurt::Refused
        );
        assert_eq!(
            hurt_part_from(
                &mut d,
                level,
                0,
                4.0,
                HIT,
                DamageKind::OnFire,
                None,
                None,
                true
            ),
            Hurt::Landed { full: true }
        );
        assert_eq!(d.health, MAX_HEALTH - 4.0);
    }

    #[test]
    fn the_native_damage_hook_changes_or_cancels_a_dragons_hit() {
        let mut test_level = sunrise_sim::health::TestLevel::with_hooks(0, hooks());
        let level = &mut test_level.level();
        // EnderDragon.hurt returns true whatever reallyHurt did. A canceled
        // hit takes no health, starts no cooldown and isn't full: what
        // hurt_dragon_part reads to credit the player and play the hurt sound.
        let mut d = dragon_as(9001);
        let arrow = HurtSource {
            damage_type: "minecraft:arrow",
            causing: 12,
            direct: 0,
            position: None,
            direct_type: None,
        };
        let hurt = hurt(&mut d, level, 0, 9.0, Some(arrow), false);
        assert_eq!(hurt, Hurt::Landed { full: false });
        assert_eq!(seen(9001), [("minecraft:arrow".to_owned(), 12, 0, 9.0)]);
        assert_eq!(d.health, MAX_HEALTH);
        assert_eq!((d.invulnerable, d.last_hurt, d.hurt_time), (0, 0.0, 0));
        // The hook sees the amount after the part's cut (20 / 4 + 1).
        let mut d = dragon_as(9002);
        assert_eq!(
            super::hurt(&mut d, level, 2, 20.0, HIT, false),
            Hurt::Landed { full: true }
        );
        assert_eq!(
            seen(9002),
            [("minecraft:player_attack".to_owned(), 0, 0, 6.0)]
        );
        assert_eq!(d.health, MAX_HEALTH - 12.0);
        // Neither a hit that doesn't count nor one cut below 0.01 reaches
        // reallyHurt.
        let mut d = dragon_as(9005);
        assert_eq!(
            super::hurt(&mut d, level, 0, 9.0, None, false),
            Hurt::Landed { full: false }
        );
        assert_eq!(
            super::hurt(&mut d, level, 3, 0.001, HIT, false),
            Hurt::Refused
        );
        assert!(seen(9005).is_empty());
    }

    #[test]
    fn dragon_records_shield_damage_and_runs_armor_and_absorption_modifiers() {
        use std::sync::Mutex;
        use sunrise_sim::health::{
            DamageContainer, DamageHooks, DamageLevel, Hurting, Reduction, ShieldBlock, Victim,
        };
        static ARMOR_SEEN: Mutex<Vec<f32>> = Mutex::new(Vec::new());
        static ABSORPTION_SEEN: Mutex<Vec<f32>> = Mutex::new(Vec::new());
        static POST_SEEN: Mutex<Vec<(f32, f32, f32, f32, f32, f32)>> = Mutex::new(Vec::new());
        fn on_shield(
            _: DamageLevel<'_>,
            hurting: Hurting<'_>,
            _: f32,
            _: bool,
            block: &mut ShieldBlock,
        ) {
            if matches!(hurting, Hurting::Dragon(9201 | 9202, _)) {
                block.blocked = true;
                block.blocked_damage = 2.0;
                block.shield_damage = 5;
            }
        }
        fn on_incoming(
            _: DamageLevel<'_>,
            hurting: Hurting<'_>,
            _: Victim,
            container: &mut DamageContainer,
        ) -> bool {
            if matches!(hurting, Hurting::Dragon(9201 | 9202, _)) {
                container.add_reduction_modifier(
                    Reduction::Armor,
                    Box::new(|_, taken| {
                        ARMOR_SEEN.lock().unwrap().push(taken);
                        1.0
                    }),
                );
            }
            false
        }
        fn on_pre(_: DamageLevel<'_>, hurting: Hurting<'_>, container: &mut DamageContainer) {
            let absorb_to = match hurting {
                Hurting::Dragon(9201, _) => 3.0,
                Hurting::Dragon(9202, _) => 10.0,
                _ => return,
            };
            container.add_reduction_modifier(
                Reduction::Absorption,
                Box::new(move |_, taken| {
                    ABSORPTION_SEEN.lock().unwrap().push(taken);
                    absorb_to
                }),
            );
        }
        fn on_post(_: DamageLevel<'_>, hurting: Hurting<'_>, container: &DamageContainer) {
            if matches!(hurting, Hurting::Dragon(9201 | 9202, _)) {
                POST_SEEN.lock().unwrap().push((
                    container.blocked(),
                    container.shield_damage(),
                    container.reduction(Reduction::Armor),
                    container.reduction(Reduction::Absorption),
                    container.inflicted(),
                    container.health_damage(),
                ));
            }
        }
        ARMOR_SEEN.lock().unwrap().clear();
        ABSORPTION_SEEN.lock().unwrap().clear();
        POST_SEEN.lock().unwrap().clear();
        let hooks = DamageHooks {
            incoming: Some(on_incoming),
            pre: Some(on_pre),
            post: Some(on_post),
            shield: Some(on_shield),
            ..DamageHooks::NONE
        };
        let mut test_level = sunrise_sim::health::TestLevel::with_hooks(0, hooks);
        let level = &mut test_level.level();

        // Hit 1: 10.0 - 2.0 shield = 8.0, Armor modifier takes 1.0 -> 7.0 inflicted,
        // Pre Absorption modifier receives 0.0 and takes 3.0 -> 4.0 health damage.
        let mut d1 = dragon_as(9201);
        assert_eq!(
            hurt(&mut d1, level, 0, 10.0, HIT, false),
            Hurt::Landed { full: true }
        );
        assert_eq!(d1.health, MAX_HEALTH - 4.0);

        // Hit 2: Pre Absorption modifier absorbs all 7.0 -> 0.0 health damage.
        let mut d2 = dragon_as(9202);
        assert_eq!(
            hurt(&mut d2, level, 0, 10.0, HIT, false),
            Hurt::Landed { full: true }
        );
        assert_eq!(d2.health, MAX_HEALTH);

        assert_eq!(*ARMOR_SEEN.lock().unwrap(), [0.0, 0.0]);
        assert_eq!(*ABSORPTION_SEEN.lock().unwrap(), [0.0, 0.0]);
        assert_eq!(
            *POST_SEEN.lock().unwrap(),
            [
                (2.0, 5.0, 1.0, 3.0, 7.0, 4.0),
                (2.0, 5.0, 1.0, 10.0, 7.0, 0.0),
            ]
        );
    }

    #[test]
    fn a_hit_names_the_player_behind_it() {
        let source = |kind, by, direct| {
            let s = super::hurt_source(kind, by, direct);
            (s.damage_type, s.causing, s.direct)
        };
        let melee = source(DamageKind::PlayerAttack, Some(12), None);
        assert_eq!(melee, ("minecraft:player_attack", 12, 12));
        let mut arrow = Projectile::new(ProjectileKind::Trident, Vec3::ZERO, None);
        arrow.id = 37;
        let shot = source(DamageKind::Arrow, Some(12), Some(arrow.id as i32));
        assert_eq!(shot, ("minecraft:arrow", 12, 37));
        let skull = source(DamageKind::WitherSkull, Some(12), Some(arrow.id as i32));
        assert_eq!(skull, ("minecraft:wither_skull", 12, 37));
        // DamageSources.magic(): an ownerless skull names no direct entity.
        let magic = source(DamageKind::Magic, None, Some(arrow.id as i32));
        assert_eq!(magic, ("minecraft:magic", 0, 0));
        let blast = source(DamageKind::Explosion, Some(12), None);
        assert_eq!(blast, ("minecraft:player_explosion", 12, 0));
        let blast = source(DamageKind::Explosion, None, None);
        assert_eq!(blast, ("minecraft:explosion", 0, 0));
    }

    #[test]
    fn a_quarter_of_its_health_while_perched_makes_it_take_off() {
        let mut test_level = sunrise_sim::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut d = dragon(Phase::Flaming { ticks: 0 });
        hurt(&mut d, level, 0, 30.0, HIT, false);
        assert!(d.phase.sitting());
        d.invulnerable = 0;
        hurt(&mut d, level, 0, 21.0, HIT, false);
        assert!(matches!(d.phase, Phase::Takeoff { .. }));
        assert_eq!(d.sitting_damage, 0.0);
    }

    #[test]
    fn an_explosion_pushes_the_dragon_along_the_blast() {
        let mut d = dragon(flying());
        d.position = Vec3::new(0.0, 70.0, 0.0);
        d.velocity = Vec3::ZERO;
        // Power 5 right under the dragon: it is thrown up, away from the blast.
        let push = d.explosion_knockback([0.0, 66.0, 0.0], 5.0, 1.0).unwrap();
        assert!(push[1] > 0.0 && push[0] == 0.0 && push[2] == 0.0);
        // Out of reach or fully covered: no push.
        assert_eq!(d.explosion_knockback([0.0, 40.0, 0.0], 5.0, 1.0), None);
        assert_eq!(
            d.explosion_knockback([0.0, 66.0, 0.0], 5.0, 0.0),
            Some([0.0; 3])
        );
    }

    #[test]
    fn explosions_hit_each_part_by_its_exposure() {
        let mut test_level = sunrise_sim::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut d = dragon(flying());
        for (i, part) in d.parts.iter_mut().enumerate() {
            part.0 = Vec3::new(i as f64 * 4.0, 70.0, 0.0);
        }
        // A power-6 blast at the dragon's feet: diameter 12.
        let center = [0.0, 70.0, 0.0];
        let open = d.explosion_hits(center, 6.0, |_| 1.0);
        // The dragon itself (its body), then the parts within 12 blocks.
        let parts: Vec<usize> = open.iter().map(|h| h.0).collect();
        assert_eq!(parts, vec![2, 0, 1, 2, 3]);
        // At the centre: (1 + 1) / 2 * 7 * 12 + 1.
        assert_eq!(open[0].1, 85.0);
        // Behind cover (no ray reaches the box) only the 1 is left.
        let covered = d.explosion_hits(center, 6.0, |_| 0.0);
        assert!(covered.iter().all(|h| h.1 == 1.0));
        // Through the cooldown the strongest hit after the part's cut wins.
        for (part, amount) in open {
            hurt(&mut d, level, part, amount, HIT, false);
        }
        assert_eq!(d.health, MAX_HEALTH - 85.0);
    }

    #[test]
    fn the_first_crystal_destroyed_in_a_tick_names_the_strafed_player() {
        // DragonHoldingPatternPhase.onCrystalDestroyed strafes at once, and
        // the strafing phase ignores the next crystal.
        let (host, guest) = (
            player(FightWho::Host, 1.0, false, false),
            player(FightWho::Guest(8), 2.0, false, false),
        );
        let mut d = dragon(flying());
        strafe_after_crystal(&mut d, Some(&host));
        strafe_after_crystal(&mut d, Some(&guest));
        assert_eq!(d.strafe_request, Some(FightWho::Host));
        // Nobody, or a player it can't attack, leaves the next crystal free.
        let creative = player(FightWho::Host, 1.0, true, false);
        let mut d = dragon(flying());
        strafe_after_crystal(&mut d, None);
        strafe_after_crystal(&mut d, Some(&creative));
        strafe_after_crystal(&mut d, Some(&guest));
        assert_eq!(d.strafe_request, Some(FightWho::Guest(8)));
        // Only while it holds its pattern.
        let mut d = dragon(Phase::Scanning { time: 0 });
        strafe_after_crystal(&mut d, Some(&guest));
        assert_eq!(d.strafe_request, None);
    }

    #[test]
    fn the_dragon_targets_any_player_it_can_attack() {
        let at = Vec3::new(0.0, 64.0, 0.0);
        // A spectating host and a creative guest are never targets: the
        // survival guest further away is.
        let players = [
            player(FightWho::Host, 1.0, false, true),
            player(FightWho::Guest(7), 2.0, true, false),
            player(FightWho::Guest(8), 30.0, false, false),
        ];
        let t = nearest_enemy(&players, false, at, None, |_| true);
        assert_eq!(t.map(|p| p.who), Some(FightWho::Guest(8)));
        // Out of a 20-block range, or filtered out, or on Peaceful: none.
        assert!(nearest_enemy(&players, false, at, Some((at, 20.0)), |_| true).is_none());
        assert!(nearest_enemy(&players, false, at, None, |p| p.position.x < 10.0).is_none());
        assert!(nearest_enemy(&players, true, at, None, |_| true).is_none());
        // The nearest of several, measured from where it looks (the egg).
        let players = [
            player(FightWho::Host, 5.0, false, false),
            player(FightWho::Guest(8), -3.0, false, false),
        ];
        let t = nearest_enemy(&players, false, at, None, |_| true);
        assert_eq!(t.map(|p| p.who), Some(FightWho::Guest(8)));
        let t = nearest_enemy(&players, false, Vec3::new(9.0, 64.0, 0.0), None, |_| true);
        assert_eq!(t.map(|p| p.who), Some(FightWho::Host));
    }
    #[test]
    fn a_landed_hurt_sets_the_dragon_hurt_flash_time() {
        let mut test_level = sunrise_sim::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut d = dragon_as(42);
        assert_eq!(d.hurt_time, 0);
        let hurt = hurt(&mut d, level, 0, 5.0, HIT, false);
        assert_eq!(hurt, Hurt::Landed { full: true });
        assert_eq!(d.hurt_time, 10);
    }

    #[test]
    fn the_strafe_firing_angle_matches_vanillas_floats() {
        // `(float)(Math.acos(dot) * (180.0F / (float)Math.PI)) + 0.5F` on a JDK: dots at
        // and around cos(10°) cross the [0, 10) gate on single ulps.
        for (dot, angle) in [
            (0x3f80_0000, 0x3f00_0000), // 1.0 -> 0.5
            (0x3f7f_be77, 0x4044_00b1), // 0.999 -> 3.0625422
            (0x3f7c_1c5f, 0x4127_ffca), // over the gate -> 10.4999485
            (0x3f7c_1c5d, 0x4127_fff3), // under it -> 10.499988
            (0x3f7c_1c5c, 0x4128_0007), // -> 10.500007
            (0x3f7c_1bda, 0x4128_0a80), // -> 10.502563
            (0x3f7c_28f6, 0x4126_fb56), // -> 10.436361
            (0x3f7a_e148, 0x413f_a742), // 0.98 -> 11.978334
            (0x3f00_0000, 0x4271_ffff), // 0.5 -> 60.499996
            (0x0000_0000, 0x42b4_ffff), // 0.0 -> 90.49999
            (0xbf80_0000, 0x4334_7fff), // -1.0 -> 180.49998
        ] {
            assert_eq!(
                strafe_angle(f32::from_bits(dot)),
                f32::from_bits(angle),
                "dot {dot:#010x}"
            );
        }
        // Past 1.0 or NaN, `Math.acos` is NaN and the gate refuses, as vanilla's does.
        assert!(strafe_angle(f32::from_bits(0x3f80_0001)).is_nan());
        assert!(strafe_angle(f32::NAN).is_nan());
    }
}
