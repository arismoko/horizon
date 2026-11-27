//! Spears: `KineticWeapon` (what holding the use button does: the speed
//! conditions of a stab, `Item.Properties.spear`'s numbers) and
//! `SpearAnimations` (how the spear is held in first and third person while
//! it charges).
use glam::{Mat4, Vec3};
pub use sunrise_sim::spear_sim::{
    Condition, HIT_FEEDBACK_TICKS, Kinetic, REACH, Reach, UseTiming, for_each_hit_in_reach,
    hits_in_reach, third_person_arm, third_person_attack_arm,
};
use sunrise_sim::spear_sim::{
    UseParams, in_out_back, in_out_expo, in_out_sine, in_quad, out_back, out_quart, progress,
};

/// `SpearAnimations.hitFeedbackAmount`.
fn hit_feedback(ticks_since: f32) -> f32 {
    0.4 * (out_quart(progress(ticks_since, 1.0, 3.0))
        - in_out_sine(progress(ticks_since, 3.0, 10.0)))
}

/// `PoseStack.rotateAround`.
fn rotate_around(rotation: Mat4, about: Vec3) -> Mat4 {
    Mat4::from_translation(about) * rotation * Mat4::from_translation(-about)
}

/// `SpearAnimations.firstPersonUse`, after the hand's own
/// `translate(invert * 0.56, -0.52, -0.72)`.
pub fn first_person_use(k: &UseTiming, ticks_since_hit: f32, time_held: f32, right: bool) -> Mat4 {
    let u = UseParams::of(k, time_held);
    let invert = if right { 1.0 } else { -1.0 };
    let degrees = |d: f32| d.to_radians();
    let mut m = Mat4::from_translation(Vec3::new(
        invert * (u.raise * 0.15 + u.raise_end * -0.05 + u.sway * -0.1 + u.sway_slow * 0.005),
        u.raise * -0.075 + u.raise_middle * 0.075 + u.sway_fast * 0.01,
        u.raise_start * 0.05 + u.raise_end * -0.05 + u.sway_slow * 0.005,
    ));
    m *= rotate_around(
        Mat4::from_rotation_x(degrees(
            -65.0 * in_out_back(u.raise) - 35.0 * u.lower
                + 100.0 * u.raise_back
                + -0.5 * u.sway_fast,
        )),
        Vec3::new(0.0, 0.1, 0.0),
    );
    // Axis.YN: a turn the other way round.
    m *= rotate_around(
        Mat4::from_rotation_y(-degrees(
            invert * (-90.0 * progress(u.raise, 0.5, 0.55) + 90.0 * u.sway + 2.0 * u.sway_slow),
        )),
        Vec3::new(invert * 0.15, 0.0, 0.0),
    );
    m * Mat4::from_translation(Vec3::new(0.0, -hit_feedback(ticks_since_hit), 0.0))
}

/// `SpearAnimations.thirdPersonUseItem`: the spear's own pose in the hand
/// held for `time_held` ticks, `swing` into the current swing.
pub fn third_person_use_item(
    k: &Kinetic,
    time_held: f32,
    swing: f32,
    ticks_since_hit: f32,
    right: bool,
) -> Mat4 {
    if time_held == 0.0 {
        return Mat4::IDENTITY;
    }
    let attack = in_quad(progress(swing, 0.05, 0.2));
    let retract = in_out_expo(progress(swing, 0.4, 1.0));
    let u = UseParams::of(&UseTiming::from(k), time_held);
    let invert = if right { 1.0 } else { -1.0 };
    let raise_modified = 1.0 - out_back(1.0 - u.raise);
    let hit = hit_feedback(ticks_since_hit);
    let degrees = |d: f32| d.to_radians();
    Mat4::from_translation(Vec3::new(
        0.0,
        -hit * 0.4,
        -k.forward_movement * (raise_modified - u.raise_back) + hit,
    )) * rotate_around(
        Mat4::from_rotation_x(-degrees(
            70.0 * (u.raise - u.raise_back) - 40.0 * (attack - retract),
        )),
        Vec3::new(0.0, -0.031_25, 0.125),
    ) * rotate_around(
        Mat4::from_rotation_y(degrees(
            invert * 90.0 * (u.raise - u.sway + 3.0 * retract + attack),
        )),
        Vec3::new(0.0, 0.0, 0.125),
    )
}

/// `SpearAnimations.firstPersonAttack`, after `applyItemArmTransform`: the
/// jab (a STAB swing), `attack` into the swing.
pub fn first_person_attack(attack: f32, right: bool) -> Mat4 {
    let invert = if right { 1.0 } else { -1.0 };
    let start = in_out_sine(progress(attack, 0.0, 0.05));
    let middle = out_back(progress(attack, 0.05, 0.2));
    let end = in_out_expo(progress(attack, 0.4, 1.0));
    Mat4::from_translation(Vec3::new(
        invert * 0.1 * (start - middle),
        -0.075 * (start - end),
        0.65 * (start - middle),
    )) * Mat4::from_rotation_x((-70.0 * (start - end)).to_radians())
        * Mat4::from_translation(Vec3::new(0.0, 0.0, -0.25 * (end - middle)))
}

/// `SpearAnimations.thirdPersonAttackItem`: the item in the swinging hand
/// thrust out by a STAB swing, `forward` the main hand's `forward_movement`
/// (0 without a kinetic weapon there).
pub fn third_person_attack_item(swing: f32, forward: f32) -> Mat4 {
    if swing <= 0.0 {
        return Mat4::IDENTITY;
    }
    let attack = in_quad(progress(swing, 0.05, 0.2));
    let retract = in_out_expo(progress(swing, 0.4, 1.0));
    rotate_around(
        Mat4::from_rotation_x((-70.0 * (attack - retract)).to_radians()),
        Vec3::new(0.0, -0.125, 0.125),
    ) * Mat4::from_translation(Vec3::new(0.0, forward * (attack - retract), 0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_kinetic_components_drive_the_first_person_pose() {
        use crate::{first_person::ItemPose, items::ItemStack};
        use serde_json::json;
        use std::{collections::BTreeMap, sync::Arc};
        use sunrise_assets::item_model::ItemModels;
        use sunrise_core::nbt::Tag;

        let kinetic = json!({
            "delay_ticks": 20,
            "dismount_conditions": {"max_duration_ticks": 200, "min_speed": 14.0},
            "knockback_conditions": {"max_duration_ticks": 400, "min_speed": 5.1},
            "damage_conditions": {"max_duration_ticks": 600, "min_relative_speed": 4.6},
            "forward_movement": 0.38
        });
        let models = ItemModels {
            prototypes: Arc::new(BTreeMap::from([
                (
                    "minecraft:iron_spear".into(),
                    BTreeMap::from([("minecraft:kinetic_weapon".into(), kinetic.clone())]),
                ),
                (
                    "test:lance".into(),
                    BTreeMap::from([("minecraft:kinetic_weapon".into(), kinetic)]),
                ),
            ])),
            ..Default::default()
        };
        let read = |stack: &ItemStack| {
            let prototype = models
                .prototypes
                .get(&*stack.item)
                .and_then(|c| c.get("minecraft:kinetic_weapon"))
                .and_then(UseTiming::from_json);
            UseTiming::of_stack(stack, prototype)
        };
        let mut spear = ItemStack::new("minecraft:iron_spear", 1);
        let timing = read(&spear).unwrap();
        // Half raised, before any condition expires. These are the resulting
        // vanilla first-person matrix columns, not a name-based spear preset.
        for left in [false, true] {
            let pose = crate::first_person::item_pose(
                left,
                0.8,
                0.0,
                ItemPose::Spear {
                    timing,
                    time_held: 10.0,
                    ticks_since_hit: 100.0,
                },
            );
            let expected = Mat4::from_cols_array(&[
                1.0,
                0.0,
                0.0,
                0.0,
                0.0,
                0.843_391_4,
                -0.537_299_63,
                0.0,
                0.0,
                0.537_299_63,
                0.843_391_4,
                0.0,
                if left { -0.635 } else { 0.635 },
                -0.541_839_1,
                -0.616_270_07,
                1.0,
            ]);
            assert!(pose.abs_diff_eq(expected, 1e-5));
        }
        let lance = read(&ItemStack::new("test:lance", 1)).unwrap();
        assert_eq!(UseParams::of(&lance, 210.0).sway, 0.5);
        assert_eq!(UseParams::of(&lance, 420.0).lower, 0.875);
        assert_eq!(UseParams::of(&lance, 617.5).raise_back, 0.5);
        assert!(
            first_person_use(&lance, 100.0, 410.0, true)
                .abs_diff_eq(first_person_use(&timing, 100.0, 410.0, true), 1e-6)
        );

        spear.data_mut().set_other(
            "minecraft:kinetic_weapon",
            Some(Tag::compound([("delay_ticks", Tag::Int(40))])),
        );
        // Saved/command decode and later mutations keep the raw full component
        // and its typed timing together through the same owning boundary.
        let saved = crate::item_data::components_tag(0, spear.data()).unwrap();
        let (_, extra) = crate::item_data::decode_components(Some(&saved));
        spear.extra = extra;
        assert_eq!(
            spear.data().unwrap().other("minecraft:kinetic_weapon"),
            saved.get("minecraft:kinetic_weapon")
        );
        let patched = read(&spear).unwrap();
        assert_eq!(UseParams::of(&patched, 10.0).raise, 0.25);
        // A replacement component does not inherit the prototype's conditions.
        assert!(patched.dismount_conditions.is_none());
        assert!(patched.knockback_conditions.is_none());
        assert!(patched.damage_conditions.is_none());
        spear.data_mut().set_other(
            "minecraft:kinetic_weapon",
            Some(Tag::String("invalid".into())),
        );
        assert!(read(&spear).is_none()); // An invalid replacement is not a prototype fallback.
        assert!(
            !first_person_use(&patched, 100.0, 10.0, true)
                .abs_diff_eq(first_person_use(&timing, 100.0, 10.0, true), 1e-3)
        );

        spear.data_mut().set_other("minecraft:kinetic_weapon", None);
        spear.data_mut().set_other(
            "!minecraft:kinetic_weapon",
            Some(Tag::Compound(Default::default())),
        );
        assert!(read(&spear).is_none());
        let mut stick = ItemStack::new("minecraft:stick", 1);
        assert!(read(&stick).is_none());
        stick.data_mut().set_other(
            "minecraft:kinetic_weapon",
            Some(Tag::Compound(Default::default())),
        );
        let defaulted = read(&stick).unwrap();
        assert_eq!(defaulted.delay_ticks, 0);
        assert!(defaulted.dismount_conditions.is_none());
        assert!(defaulted.knockback_conditions.is_none());
        assert!(defaulted.damage_conditions.is_none());
        // Java Mth.clamp preserves NaN when zero delay meets zero held time.
        assert!(UseParams::of(&defaulted, 0.0).raise.is_nan());
        assert!(!first_person_use(&defaulted, 100.0, 0.0, true).is_finite());
        assert!(first_person_use(&defaulted, 100.0, 1.0, true).is_finite());
    }

    #[test]
    fn kinetic_numeric_codecs_normalize_before_rendering() {
        use serde_json::json;
        for (value, ticks) in [
            (json!(40.0), 40),
            (json!(40.75), 40),
            (json!(-0.5), 0),
            (json!(4_294_967_336_i64), 40),
            (json!(4_294_967_336.75), 40),
        ] {
            let timing = UseTiming::from_json(&json!({
                "delay_ticks": value,
                "damage_conditions": {"max_duration_ticks": value}
            }))
            .unwrap();
            assert_eq!(timing.delay_ticks, ticks);
            assert_eq!(timing.damage_conditions.unwrap().max_duration_ticks, ticks);
        }
        for value in [json!(-1), json!("40"), json!(true), json!(2_147_483_648.0)] {
            assert!(UseTiming::from_json(&json!({"delay_ticks": value})).is_none());
        }
        assert_eq!(
            UseTiming::from_json(&json!({"delay_ticks": null, "damage_conditions": null})),
            UseTiming::from_json(&json!({}))
        );
        let mut stack = crate::items::ItemStack::new("minecraft:stick", 1);
        for (number, ticks) in [
            (sunrise_core::nbt::Tag::Double(40.75f64.to_bits()), 40),
            (sunrise_core::nbt::Tag::Float(f32::NAN.to_bits()), 0),
        ] {
            stack.data_mut().set_other(
                "minecraft:kinetic_weapon",
                Some(sunrise_core::nbt::Tag::compound([("delay_ticks", number)])),
            );
            assert_eq!(
                UseTiming::of_stack(&stack, None).unwrap().delay_ticks,
                ticks
            );
        }
    }

    #[test]
    fn kinetic_pose_timing_uses_java_integer_arithmetic() {
        let components = sunrise_assets::item_model::Components::from([(
            "minecraft:kinetic_weapon".into(),
            serde_json::json!({
                "delay_ticks": 2_147_483_647,
                "dismount_conditions": {"max_duration_ticks": 2_147_483_647}
            }),
        )]);
        let timing = UseTiming::from_json(&components["minecraft:kinetic_weapon"]).unwrap();
        // finishSwayingTick wraps to -2, startSwayingTick to -22.
        assert_eq!(UseParams::of(&timing, 10.0).sway, 1.0);
    }

    #[test]
    fn the_jab_starts_and_ends_at_rest() {
        assert_eq!(first_person_attack(0.0, true), Mat4::IDENTITY);
        assert!(first_person_attack(1.0, true).abs_diff_eq(Mat4::IDENTITY, 1e-5));
        // Thrust out and turned mid-jab, mirrored in the left hand.
        let right = first_person_attack(0.2, true);
        assert!(!right.abs_diff_eq(Mat4::IDENTITY, 1e-3));
        let left = first_person_attack(0.2, false);
        assert!((left.w_axis.x + right.w_axis.x).abs() < 1e-6);
        assert_eq!(third_person_attack_item(0.0, 0.38), Mat4::IDENTITY);
        assert!(third_person_attack_item(1.0, 0.38).abs_diff_eq(Mat4::IDENTITY, 1e-5));
        // The arm draws back, thrusts forward, then returns.
        assert_eq!(third_person_attack_arm(0.0), 0.0);
        assert!((third_person_attack_arm(0.05) - 90f32.to_radians()).abs() < 1e-5);
        assert!(third_person_attack_arm(0.2) < 0.0);
        assert!(third_person_attack_arm(1.0).abs() < 1e-5);
    }
}
