//! Portable spear simulation: `KineticWeapon` charge conditions, reach, and
//! stab hit scanning (`ProjectileUtil.getHitEntitiesAlong`).
use crate::first_person_sim::SwingAnimation;
use std::f32::consts::PI;
use sunrise_core::{
    geometry::{Aabb, Vec3 as Vec3d, trig::Mth},
    jmath::pow,
};
use sunrise_items::kinetic_weapon::UseDuration;
pub use sunrise_items::kinetic_weapon::UseTiming;

/// `KineticWeapon.Condition`.
#[derive(Clone, Copy, Debug)]
pub struct Condition {
    pub max_duration_ticks: i32,
    pub min_speed: f32,
    pub min_relative_speed: f32,
}
impl Condition {
    pub fn test(
        self,
        ticks_used: i32,
        attacker_speed: f64,
        relative_speed: f64,
        entity_factor: f64,
    ) -> bool {
        ticks_used <= self.max_duration_ticks
            && attacker_speed >= f64::from(self.min_speed) * entity_factor
            && relative_speed >= f64::from(self.min_relative_speed) * entity_factor
    }
}

/// `KineticWeapon`: a spear's charge.
#[derive(Clone, Copy, Debug)]
pub struct Kinetic {
    pub contact_cooldown_ticks: i32,
    pub delay_ticks: i32,
    pub dismount: Condition,
    pub knockback: Condition,
    pub damage: Condition,
    pub forward_movement: f32,
    pub damage_multiplier: f32,
    pub sound: &'static str,
    pub hit_sound: &'static str,
    /// The `attack_animation` duration, and the `piercing_weapon` sound of a
    /// jab.
    pub attack_ticks: i32,
    pub attack_sound: &'static str,
}

impl From<&Kinetic> for UseTiming {
    fn from(k: &Kinetic) -> Self {
        Self {
            delay_ticks: k.delay_ticks,
            dismount_conditions: Some(UseDuration {
                max_duration_ticks: k.dismount.max_duration_ticks,
            }),
            knockback_conditions: Some(UseDuration {
                max_duration_ticks: k.knockback.max_duration_ticks,
            }),
            damage_conditions: Some(UseDuration {
                max_duration_ticks: k.damage.max_duration_ticks,
            }),
        }
    }
}

/// `AttackRange` of a spear: the reach of its stabs.
pub struct Reach {
    pub min: f32,
    pub max: f32,
    pub min_creative: f32,
    pub max_creative: f32,
    pub hitbox_margin: f32,
    pub mob_factor: f32,
}
pub const REACH: Reach = Reach {
    min: 2.0,
    max: 4.5,
    min_creative: 2.0,
    max_creative: 6.5,
    hitbox_margin: 0.125,
    mob_factor: 0.5,
};

impl Kinetic {
    /// The `kinetic_weapon` of a spear item (`Item.Properties.spear`).
    pub fn of(item: &str) -> Option<Self> {
        // (attack duration, damage multiplier, delay, dismount time and
        // speed, knockback time and speed, damage time and speed): seconds
        // and blocks per second.
        let (attack, multiplier, delay, dismount, knockback, damage) = match item {
            "minecraft:wooden_spear" => (0.65, 0.7, 0.75, (5.0, 14.0), (10.0, 5.1), (15.0, 4.6)),
            "minecraft:stone_spear" => (0.75, 0.82, 0.7, (4.5, 13.0), (9.0, 5.1), (13.75, 4.6)),
            "minecraft:copper_spear" => (0.85, 0.82, 0.65, (4.0, 12.0), (8.25, 5.1), (12.5, 4.6)),
            "minecraft:iron_spear" => (0.95, 0.95, 0.6, (2.5, 11.0), (6.75, 5.1), (11.25, 4.6)),
            "minecraft:golden_spear" => (0.95, 0.7, 0.7, (3.5, 13.0), (8.5, 5.1), (13.75, 4.6)),
            "minecraft:diamond_spear" => (1.05, 1.075, 0.5, (3.0, 10.0), (6.5, 5.1), (10.0, 4.6)),
            "minecraft:netherite_spear" => (1.15, 1.2, 0.4, (2.5, 9.0), (5.5, 5.1), (8.75, 4.6)),
            _ => return None,
        };
        let wood = item == "minecraft:wooden_spear";
        let ticks = |seconds: f32| (seconds * 20.0) as i32;
        Some(Self {
            contact_cooldown_ticks: 10,
            delay_ticks: ticks(delay),
            // `Condition.ofAttackerSpeed`, and `ofRelativeSpeed` for the damage.
            dismount: Condition {
                max_duration_ticks: ticks(dismount.0),
                min_speed: dismount.1,
                min_relative_speed: 0.0,
            },
            knockback: Condition {
                max_duration_ticks: ticks(knockback.0),
                min_speed: knockback.1,
                min_relative_speed: 0.0,
            },
            damage: Condition {
                max_duration_ticks: ticks(damage.0),
                min_speed: 0.0,
                min_relative_speed: damage.1,
            },
            forward_movement: 0.38,
            damage_multiplier: multiplier,
            sound: if wood {
                "minecraft:item.spear_wood.use"
            } else {
                "minecraft:item.spear.use"
            },
            hit_sound: if wood {
                "minecraft:item.spear_wood.hit"
            } else {
                "minecraft:item.spear.hit"
            },
            attack_ticks: ticks(attack),
            attack_sound: if wood {
                "minecraft:item.spear_wood.attack"
            } else {
                "minecraft:item.spear.attack"
            },
        })
    }

    /// `KineticWeapon.computeDamageUseDuration`: the delay and the time the
    /// damage condition lasts.
    pub fn damage_use_duration(&self) -> i32 {
        self.delay_ticks + self.damage.max_duration_ticks
    }

    /// The spear's `attack_animation`: a STAB (`SwingAnimationType` 2) as long
    /// as its attack duration.
    pub fn attack_animation(&self) -> SwingAnimation {
        SwingAnimation {
            kind: 2,
            duration: self.attack_ticks,
        }
    }
}

/// `KineticWeapon.HIT_FEEDBACK_TICKS`.
pub const HIT_FEEDBACK_TICKS: i64 = 10;

/// `ProjectileUtil.getManyEntityHitResult` over `boxes`: those the segment
/// `from`..`to` meets, or comes within the margin of with no block between
/// (`clip`: where a segment first meets a block), each with where it is hit.
fn visit_hits_along(
    boxes: impl IntoIterator<Item = (usize, Aabb)>,
    from: Vec3d,
    to: Vec3d,
    margin: f64,
    clip: &impl Fn(Vec3d, Vec3d) -> Option<Vec3d>,
    mut visit: impl FnMut(usize, Vec3d),
) {
    let grow = Vec3d::new(margin, margin, margin);
    for (i, bounds) in boxes {
        let hit = if bounds.contains(from) {
            Some(from)
        } else if let Some(at) = bounds.clip(from, to) {
            Some(at)
        } else if margin > 0.0 {
            bounds.inflate(grow).clip(from, to).and_then(|outside| {
                let centre = bounds.center();
                let toward = clip(outside, centre).unwrap_or(centre);
                bounds.clip(outside, toward)
            })
        } else {
            None
        };
        if let Some(at) = hit {
            visit(i, at);
        }
    }
}

/// `ProjectileUtil.getHitEntitiesAlong` with an `AttackRange` of `reach` (its
/// effective min and max range and hitbox margin): the boxes along `look`
/// from the minimum reach to the maximum plus the attacker's own movement,
/// none when a block (`clip`) is closer than the minimum.
pub fn hits_in_reach(
    boxes: &[(usize, Aabb)],
    eye: Vec3d,
    look: Vec3d,
    movement: Vec3d,
    (min, max, margin): (f32, f32, f32),
    clip: impl Fn(Vec3d, Vec3d) -> Option<Vec3d>,
) -> Vec<(usize, Vec3d)> {
    let mut hits = Vec::new();
    for_each_hit_in_reach(
        boxes.iter().copied(),
        eye,
        look,
        movement,
        (min, max, margin),
        clip,
        |i, at| hits.push((i, at)),
    );
    hits
}

/// Visit the same ordered hits as hits_in_reach without allocating candidate or result lists.
/// Client picks reduce these to one nearest hit; multi-target attacks may collect them.
pub fn for_each_hit_in_reach(
    boxes: impl IntoIterator<Item = (usize, Aabb)>,
    eye: Vec3d,
    look: Vec3d,
    movement: Vec3d,
    (min, max, margin): (f32, f32, f32),
    clip: impl Fn(Vec3d, Vec3d) -> Option<Vec3d>,
    visit: impl FnMut(usize, Vec3d),
) {
    let from = eye.add_vector(look.scale(f64::from(min)));
    let mut to = eye.add_vector(look.scale(f64::from(max) + movement.dot(look).max(0.0)));
    // A block closer than the minimum reach takes the whole stab.
    if let Some(block) = clip(eye, to) {
        to = block;
        if eye.distance_squared(to) < eye.distance_squared(from) {
            return;
        }
    }
    visit_hits_along(boxes, from, to, f64::from(margin), &clip, visit)
}

/// `Mth.clamp` on floats (a NaN stays one).
fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}
/// `SpearAnimations.progress`.
#[doc(hidden)]
pub fn progress(time: f32, start: f32, end: f32) -> f32 {
    clamp((time - start) / (end - start), 0.0, 1.0)
}
fn square(x: f32) -> f32 {
    x * x
}
fn cube(x: f32) -> f32 {
    x * x * x
}
fn p(x: f32, y: f64) -> f64 {
    pow(f64::from(x), y)
}

// `net.minecraft.util.Ease`.
#[doc(hidden)]
pub fn in_quad(x: f32) -> f32 {
    x * x
}
#[doc(hidden)]
pub fn out_quart(x: f32) -> f32 {
    1.0 - square(square(1.0 - x))
}
fn out_cubic(x: f32) -> f32 {
    1.0 - cube(1.0 - x)
}
#[doc(hidden)]
pub fn out_back(x: f32) -> f32 {
    1.0 + 2.70158 * cube(x - 1.0) + 1.70158 * square(x - 1.0)
}
#[doc(hidden)]
pub fn in_out_sine(x: f32) -> f32 {
    -((PI * x).mcos() - 1.0) / 2.0
}
#[doc(hidden)]
pub fn in_out_expo(x: f32) -> f32 {
    if x < 0.5 {
        if x == 0.0 {
            0.0
        } else {
            (p(2.0, f64::from(20.0 * x - 10.0)) / 2.0) as f32
        }
    } else if x == 1.0 {
        1.0
    } else {
        ((2.0 - p(2.0, f64::from(-20.0 * x + 10.0))) / 2.0) as f32
    }
}
#[doc(hidden)]
pub fn in_out_back(x: f32) -> f32 {
    if x < 0.5 {
        return 4.0 * x * x * (7.189_819 * x - 2.594_909_4) / 2.0;
    }
    let dt = 2.0 * x - 2.0;
    (dt * dt * (3.594_909_4 * dt + 2.594_909_4) + 2.0) / 2.0
}
fn in_out_elastic(x: f32) -> f32 {
    if x == 0.0 {
        return 0.0;
    }
    if x == 1.0 {
        return 1.0;
    }
    let sin = (f64::from(20.0 * x - 11.125) * f64::from(PI * 4.0 / 9.0)).sin();
    if x < 0.5 {
        (-(p(2.0, f64::from(20.0 * x - 10.0)) * sin) / 2.0) as f32
    } else {
        (p(2.0, f64::from(-20.0 * x + 10.0)) * sin / 2.0 + 1.0) as f32
    }
}
fn out_circ(x: f32) -> f32 {
    f64::from(1.0 - square(x - 1.0)).sqrt() as f32
}
fn in_circ(x: f32) -> f32 {
    (-f64::from(1.0 - x * x).sqrt()) as f32 + 1.0
}

/// `SpearAnimations.UseParams`.
#[doc(hidden)]
pub struct UseParams {
    pub raise: f32,
    pub raise_start: f32,
    pub raise_middle: f32,
    pub raise_end: f32,
    pub sway: f32,
    pub lower: f32,
    pub raise_back: f32,
    pub sway_intensity: f32,
    pub sway_slow: f32,
    pub sway_fast: f32,
}
impl UseParams {
    #[doc(hidden)]
    pub fn of(k: &UseTiming, time: f32) -> Self {
        let finish_raising = k.delay_ticks;
        let duration = |c: Option<UseDuration>| c.map_or(0, |c| c.max_duration_ticks);
        let finish_swaying = duration(k.dismount_conditions).wrapping_add(finish_raising);
        let start_swaying = finish_swaying.wrapping_sub(20);
        let finish_lowering = duration(k.knockback_conditions).wrapping_add(finish_raising);
        let start_lowering = finish_lowering.wrapping_sub(40);
        let finish_raising_back = duration(k.damage_conditions).wrapping_add(finish_raising);
        let raise = progress(time, 0.0, finish_raising as f32);
        let sway = progress(time, start_swaying as f32, finish_swaying as f32);
        let lower = out_cubic(in_out_elastic(progress(
            time - 20.0,
            start_lowering as f32,
            finish_lowering as f32,
        )));
        let raise_back = progress(
            time,
            finish_raising_back.wrapping_sub(5) as f32,
            finish_raising_back as f32,
        );
        let sway_intensity = 2.0 * out_circ(sway) - 2.0 * in_circ(raise_back);
        let degrees = PI / 180.0;
        Self {
            raise,
            raise_start: progress(raise, 0.0, 0.5),
            raise_middle: progress(raise, 0.5, 0.8),
            raise_end: progress(raise, 0.8, 1.0),
            sway,
            lower,
            raise_back,
            sway_intensity,
            sway_slow: (time * 19.0 * degrees).msin() * sway_intensity,
            sway_fast: (time * 30.0 * degrees).msin() * sway_intensity,
        }
    }
}

/// `SpearAnimations.thirdPersonHandUse`: the angles (x, y and the z added to
/// the arm's own) of the arm holding a spear. `ticks_using` is the time the
/// item has been in use by this arm (0 when it is not).
pub fn third_person_arm(
    k: &Kinetic,
    ticks_using: f32,
    right: bool,
    head: (f32, f32),
    gliding_or_swimming: bool,
) -> (f32, f32, f32) {
    let invert = if right { 1.0 } else { -1.0 };
    let degrees = PI / 180.0;
    let mut y = -0.1 * invert + head.1;
    let mut x = -PI / 2.0 + head.0 + 0.8;
    if gliding_or_swimming {
        x -= 0.959_931_1;
    }
    y = degrees * clamp(180.0 / PI * y, -60.0, 60.0);
    x = degrees * clamp(180.0 / PI * x, -120.0, 30.0);
    let mut z = 0.0;
    if ticks_using > 0.0 {
        let u = UseParams::of(&UseTiming::from(k), ticks_using);
        y += -invert * u.sway_fast * degrees * u.sway_intensity;
        z = -invert * u.sway_slow * degrees * u.sway_intensity * 0.5;
        x += degrees
            * (-40.0 * u.raise_start
                + 30.0 * u.raise_middle
                + -20.0 * u.raise_end
                + 20.0 * u.lower
                + 10.0 * u.raise_back
                + 0.6 * u.sway_slow * u.sway_intensity);
    }
    (x, y, z)
}

/// `SpearAnimations.thirdPersonAttackHand`: what a STAB swing adds to the
/// swinging arm's `xRot`, `swing` into the swing.
pub fn third_person_attack_arm(swing: f32) -> f32 {
    let prepare = in_out_sine(progress(swing, 0.0, 0.05));
    let attack = in_quad(progress(swing, 0.05, 0.2));
    let retract = in_out_expo(progress(swing, 0.4, 1.0));
    (90.0 * prepare - 120.0 * attack + 30.0 * retract).to_radians()
}
