//! Independent 26.3 Projectile.shootFromRotation sequence fixtures, and
//! how a wind charge's tick turns it (`AbstractHurtingProjectile.tick`),
//! from a saved rotation too.
use super::{Projectile, ProjectileKind, ProjectileWorld};
use crate::entity_nbt::fireball_from_nbt;
use crate::mobs::Who;
use crate::save::{doubles, floats};
use sunrise_core::{
    geometry::{Aabb, Vec3},
    nbt::Tag,
    random::{LegacyRandom, RandomSource},
};

/// Open air: nothing to hit, no water.
struct Air;
impl ProjectileWorld for Air {
    fn clip(&self, _: Vec3, _: Vec3) -> Option<(Vec3, [i32; 3], [i32; 3])> {
        None
    }
    fn in_water(&self, _: Aabb) -> bool {
        false
    }
    fn targets(&self) -> &[(Who, Aabb)] {
        &[]
    }
}

#[test]
fn rotation_shot_consumes_six_doubles_in_axis_order_and_inherits_motion() {
    // java.util.Random(1234).nextDouble():
    // .6465821602909256, .9513577109193919, .8575884598068334,
    // .45823330506267057, .3359524025416939, .20387478195313158.
    // Vanilla Projectile.shoot: normalize [0,0,1], add x/y/z triangles
    // scaled by .0172275, multiply by 1.5, then add known movement.
    for grounded in [false, true] {
        let mut shot = Projectile::new(ProjectileKind::Snowball, Vec3::ZERO, None)
            .with_entity_random(LegacyRandom::new(1234));
        shot.shoot_from_rotation(0.0, 0.0, 0.0, 1.5, 1.0, Vec3::new(0.2, 0.3, -0.4), grounded);
        let expected_y = if grounded {
            0.0103198363925326
        } else {
            0.3103198363925326
        };
        for (actual, expected) in [
            (shot.velocity.x, 0.19212421880232217),
            (shot.velocity.y, expected_y),
            (shot.velocity.z, 1.1034130508130342),
        ] {
            assert!((actual - expected).abs() < 1e-14, "{actual} != {expected}");
        }
        assert_eq!(shot.entity_random().next_double(), 0.34690742873967684);
        assert!(shot.needs_sync);
    }
}

#[test]
fn rotation_shot_applies_vanilla_near_zero_normalization_without_skipping_draws() {
    let mut shot = Projectile::new(ProjectileKind::Snowball, Vec3::ZERO, None)
        .with_entity_random(LegacyRandom::new(1234));
    // At pitch 90 with a -90 vertical offset all three direction components
    // are below Vec3.normalize's widened 1.0E-5F threshold. Spread is zero,
    // but Projectile.getMovementToShoot still takes all six double draws.
    shot.shoot_from_rotation(90.0, 0.0, -90.0, 1.5, 0.0, Vec3::new(0.2, 0.3, -0.4), false);
    assert_eq!(shot.velocity, Vec3::new(0.2, 0.3, -0.4));
    assert_eq!(shot.entity_random().next_double(), 0.34690742873967684);
    assert!(shot.needs_sync);
}

/// `AbstractWindCharge.tick` runs `AbstractHurtingProjectile.tick`, which
/// calls `ProjectileUtil.rotateTowardsMovement(this, 0.2F)` before it moves:
/// a fifth of the way from the tick's old rotation towards the motion.
/// Vanilla 26.3, `/summon wind_charge ~ ~ ~ {Motion:[0.6d,0.0d,0.0d]}` then
/// `/data get entity .. Rotation` after each `tick step 1` (#765).
#[test]
fn a_wind_charge_turns_a_fifth_of_the_way_to_its_motion_each_tick() {
    for kind in [ProjectileKind::WindCharge, ProjectileKind::BreezeWindCharge] {
        let mut p = Projectile::new(kind.clone(), Vec3::new(0.5, 100.0, 0.5), None);
        p.velocity = Vec3::new(0.6, 0.0, 0.0);
        let mut seen = Vec::new();
        for _ in 0..3 {
            p.tick(&Air);
            seen.push([p.yaw, p.pitch]);
        }
        assert_eq!(
            seen,
            [
                [18.0_f32, -1.525_878_9e-6],
                [32.4, -2.746_582_1e-6],
                [43.920_002, -3.723_144_8e-6],
            ],
            "{kind:?}"
        );
        // The charge's motion is untouched by the turn (inertia 1.0, no power).
        assert_eq!(p.velocity, Vec3::new(0.6, 0.0, 0.0), "{kind:?}");
    }
}

/// `rotateTowardsMovement` returns at once for a zero motion: a still wind
/// charge keeps the rotation it was summoned with. Vanilla 26.3,
/// `/summon wind_charge .. {Rotation:[-170.5f,-90.0f]}` reads back
/// `[-170.5f, -90.0f]` after its ticks; it never snaps to `[0f, 0f]`.
#[test]
fn a_still_wind_charge_keeps_its_rotation() {
    for kind in [ProjectileKind::WindCharge, ProjectileKind::BreezeWindCharge] {
        let mut p = Projectile::new(kind.clone(), Vec3::new(0.5, 100.0, 0.5), None);
        p.yaw = -170.5;
        p.pitch = -90.0;
        for _ in 0..3 {
            p.tick(&Air);
        }
        assert_eq!([p.yaw, p.pitch], [-170.5, -90.0], "{kind:?}");
    }
}

/// A moving wind charge restored with a huge finite rotation (`Rotation`
/// is loaded as saved; `Entity.setYRot` keeps any finite yaw) must tick
/// and turn. Vanilla's `rotateTowardsMovement` loops would never end here:
/// `1.0E30F - 360.0F == 1.0E30F`. Sunrise first takes the angle within one
/// turn with `%`: `1.0E30F % 360.0F` is 120.0 and `Float.MAX_VALUE %
/// 360.0F` is 0.0 (Java `frem`). From there the 26.3 loops and
/// `Mth.lerp(0.2F, ..)` give the rotations below (computed with the jar's
/// `Mth`), and the same three ticks as from a sane start. Fireballs,
/// dragon fireballs and wither skulls load through the same
/// `fireball_from_nbt` and turn through the same function.
#[test]
fn a_restored_wind_charge_with_a_huge_rotation_turns_back_into_range() {
    for (id, rotation) in [
        ("minecraft:wind_charge", [1.0e30_f32, f32::MAX]),
        ("minecraft:breeze_wind_charge", [1.0e30, f32::MAX]),
        ("minecraft:small_fireball", [1.0e30, f32::MAX]),
    ] {
        let tag = Tag::compound([
            ("id", Tag::String(id.into())),
            ("Pos", doubles(&[0.5, 100.0, 0.5])),
            ("Motion", doubles(&[0.6, 0.0, 0.0])),
            ("Rotation", floats(&rotation)),
            ("acceleration_power", Tag::Double(0.0f64.to_bits())),
        ]);
        let mut p = fireball_from_nbt(&tag, None).unwrap();
        // Entity.load runs setYRot, setXRot, then setRot(yRot % 360, xRot % 360):
        // 1.0e30 % 360 == 120.0 and clamp(f32::MAX % 360, -90, 90) == 0.0 (26.3 witness).
        assert_eq!([p.yaw, p.pitch], [120.0, 0.0], "{id}");
        let mut seen = Vec::new();
        for _ in 0..3 {
            p.tick(&Air);
            seen.push([p.yaw, p.pitch]);
        }
        assert_eq!(
            seen,
            [
                [114.0_f32, -1.525_878_9e-6],
                [109.2, -2.746_582_1e-6],
                [105.36, -3.723_144_8e-6],
            ],
            "{id}"
        );
        // Direct huge yaw via Entity.setYRot also stays finite and turns cleanly via shift_by_turns.
        let mut huge = fireball_from_nbt(&tag, None).unwrap();
        huge.set_yaw(1.0e30);
        huge.tick(&Air);
        assert_eq!([huge.yaw, huge.pitch], [114.0_f32, -1.525_878_9e-6], "{id}");
    }
}

/// A non-finite saved angle, which `Entity.setYRot` / `setXRot` refuse on
/// load (the rotation stays 0), turns as from `[0f, 0f]`: the #765 values.
#[test]
fn a_restored_wind_charge_with_an_infinite_rotation_turns_as_from_zero() {
    let tag = Tag::compound([
        ("id", Tag::String("minecraft:wind_charge".into())),
        ("Pos", doubles(&[0.5, 100.0, 0.5])),
        ("Motion", doubles(&[0.6, 0.0, 0.0])),
        ("Rotation", floats(&[f32::INFINITY, f32::NEG_INFINITY])),
    ]);
    let mut p = fireball_from_nbt(&tag, None).unwrap();
    let mut seen = Vec::new();
    for _ in 0..3 {
        p.tick(&Air);
        seen.push([p.yaw, p.pitch]);
    }
    assert_eq!(
        seen,
        [
            [18.0_f32, -1.525_878_9e-6],
            [32.4, -2.746_582_1e-6],
            [43.920_002, -3.723_144_8e-6],
        ]
    );
}

/// `Projectile.shoot(DDDFF)` sets the rotation from the new motion as
/// `(float)(Mth.atan2(x, z) * 57.2957763671875D)` and
/// `(float)(Mth.atan2(y, horizontalDistance) * 57.2957763671875D)`: one
/// multiply by the folded `180.0F / (float)Math.PI` (offsets 43-50, 62-69).
/// The motions and rotations are vanilla 26.3's own, read right after a
/// dispenser shot them (`tools/parity_fuzz.py spawn`, #920).
#[test]
fn a_shot_faces_its_motion_with_the_folded_float_degree_constant() {
    for (kind, motion, rotation) in [
        (
            ProjectileKind::Egg,
            [1.0921503583127599, 0.07908806208644255, 0.05642147364871476],
            [87.04267_f32, 4.1363444],
        ),
        (
            ProjectileKind::SmallFireball,
            [
                0.958190841689071,
                0.060279224934985304,
                -0.017876257827707385,
            ],
            [91.068855, 3.5990546],
        ),
        (
            ProjectileKind::SmallFireball,
            [
                0.010748615966384488,
                1.0573762104847477,
                -0.09487713089403285,
            ],
            [173.53645, 84.84002],
        ),
        (
            ProjectileKind::Snowball,
            [
                -1.1570431167174582,
                -0.04080260176556583,
                0.08084624878829741,
            ],
            [-86.00303, -2.0147643],
        ),
        (
            ProjectileKind::ExperienceBottle,
            [
                1.388066520989399,
                -0.010784563767071194,
                0.05055930210554001,
            ],
            [87.914, -0.44485638],
        ),
    ] {
        let mut p = Projectile::new(kind.clone(), Vec3::ZERO, None);
        p.velocity = Vec3::new(motion[0], motion[1], motion[2]);
        p.update_rotation();
        assert_eq!([p.yaw, p.pitch], rotation, "{kind:?}");
        assert_eq!([p.previous_yaw, p.previous_pitch], rotation, "{kind:?}");
    }
}

/// Through `shoot`: no spread, so the motion is the normalised direction
/// times the speed (`getMovementToShoot`), and the rotation follows it.
/// Expected values from the 26.3 jar's own `Vec3.normalize().scale(1.1F)`
/// and `Mth.atan2(..) * 57.2957763671875D`; main's multiply by 180 and
/// divide by the float pi gave `[87.04268f, 4.136343f]`.
#[test]
fn shoot_sets_the_vanilla_rotation_of_its_motion() {
    let mut p = Projectile::new(ProjectileKind::Egg, Vec3::ZERO, None);
    p.shoot(
        &mut LegacyRandom::new(1),
        [1.0921503583127599, 0.07908806208644255, 0.05642147364871476],
        1.1,
        0.0,
    );
    assert_eq!(
        p.velocity,
        Vec3::new(
            1.0956736529323259,
            0.07934320144660496,
            0.056603490230059485
        )
    );
    assert_eq!([p.yaw, p.pitch], [87.04267_f32, 4.1363425]);
    assert_eq!(
        [p.previous_yaw, p.previous_pitch],
        [87.04267_f32, 4.1363425]
    );
}

/// A motion so large that its squared lengths overflow gives no finite target
/// angle (`Mth.atan2` of an overflowed length). The turn is skipped: the
/// rotation and the previous rotation it would interpolate from stay as they
/// were, rather than becoming NaN or looping by whole turns towards an
/// infinite angle. `[0, 1e200, 0]` leaves the yaw target finite (90) and only
/// the pitch target NaN; the others make both non-finite.
#[test]
fn a_turn_towards_a_non_finite_angle_keeps_the_rotation() {
    for v in [
        Vec3::new(f64::MAX, f64::MAX, f64::MAX),
        Vec3::new(f64::MAX, -f64::MAX, 0.0),
        Vec3::new(1e160, 1e160, 1e160),
        Vec3::new(0.0, 1e200, 0.0),
    ] {
        let mut p = Projectile::new(ProjectileKind::WindCharge, Vec3::new(0.5, 100.0, 0.5), None);
        p.velocity = v;
        p.yaw = 30.0;
        p.pitch = -10.0;
        p.previous_yaw = 25.0;
        p.previous_pitch = -5.0;
        p.rotate_towards_movement(0.2);
        assert_eq!(
            [p.yaw, p.pitch, p.previous_yaw, p.previous_pitch],
            [30.0, -10.0, 25.0, -5.0],
            "{v:?}"
        );
    }
}
