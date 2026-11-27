use super::mob_effect_instance;
use super::thrown_item_axes;
use super::thrown_item_quad;
use super::{ProjectileKind, is_item_billboard};
use sunrise_level::camera_math::CameraType;

/// The component-wise lowest and highest corner of the quad.
pub(in crate::game) fn bounds(corners: &[glam::Vec3; 4]) -> (glam::Vec3, glam::Vec3) {
    (
        corners.iter().copied().reduce(glam::Vec3::min).unwrap(),
        corners.iter().copied().reduce(glam::Vec3::max).unwrap(),
    )
}

#[test]
fn nautilus_breath_lands_in_creative_as_ambient() {
    // AbstractNautilus.applyEffects: MobEffectInstance(BREATH_OF_THE_NAUTILUS,
    // 60, 0, ambient true, visible true, showIcon true), no game-mode gate.
    let e = mob_effect_instance("minecraft:breath_of_the_nautilus", 60, 0, true, true)
        .expect("applies to a creative player");
    assert_eq!(
        (e.id, e.duration, e.amplifier),
        ("breath_of_the_nautilus", 60, 0)
    );
    assert!(e.ambient && e.visible);
    // Survival gets the same ambient instance.
    let e = mob_effect_instance("minecraft:breath_of_the_nautilus", 60, 0, true, false).unwrap();
    assert!(e.ambient);
}

#[test]
fn other_mob_effects_stay_blocked_in_creative() {
    assert!(mob_effect_instance("minecraft:poison", 140, 0, false, true).is_none());
    let e = mob_effect_instance("minecraft:poison", 140, 0, false, false).unwrap();
    assert!(!e.ambient);
}

#[test]
fn thrown_item_billboard_lifts_by_the_generated_ground_transform() {
    // The kinds vanilla draws with `ThrownItemRenderer` as a billboard: the
    // only lifted ones. A fireball hands `getItem` to the item renderer
    // instead, which carries the ground transform itself.
    assert!(is_item_billboard(&ProjectileKind::Snowball));
    assert!(is_item_billboard(&ProjectileKind::Egg));
    assert!(is_item_billboard(&ProjectileKind::EnderPearl));
    assert!(!is_item_billboard(&ProjectileKind::SmallFireball));
    let large = ProjectileKind::LargeFireball { power: 1 };
    assert!(!is_item_billboard(&large));
    // A projectile at y = 100 with right = +X, up = +Y: the sprite sits
    // 0.125 blocks above the entity (2 of `item/generated`'s 16 units) and
    // stays 0.5 blocks wide, so the quad's y runs 99.875 .. 100.375.
    let at = glam::Vec3::Y * 100.0;
    let (right, up) = (glam::Vec3::X, glam::Vec3::Y);
    let lifted = is_item_billboard(&ProjectileKind::Snowball);
    let (lo, hi) = bounds(&thrown_item_quad(at, right, up, 1.0, lifted));
    assert_eq!((lo.y, hi.y), (99.875, 100.375));
    assert_eq!((lo.x, hi.x), (-0.25, 0.25));
    // Llama spit shares the quad here but is a model in vanilla
    // (`LlamaSpitRenderer`): no lift at all.
    let lifted = is_item_billboard(&ProjectileKind::LlamaSpit);
    assert!(!lifted);
    let (lo, hi) = bounds(&thrown_item_quad(at, right, up, 1.0, lifted));
    assert_eq!((lo.y, hi.y), (99.75, 100.25));
    // The rise follows the billboard's own up axis, not world up.
    let (right, up) = (glam::Vec3::Z, glam::Vec3::X);
    let lifted = is_item_billboard(&ProjectileKind::Egg);
    let (lo, hi) = bounds(&thrown_item_quad(at, right, up, 1.0, lifted));
    assert_eq!((lo.x, hi.x), (-0.125, 0.375));
    assert_eq!((lo.y, hi.y), (100.0, 100.0));
}

#[test]
fn thrown_item_billboard_does_not_flip_at_straight_up_or_down() {
    for yaw in [0.0f32, 37.5, 90.0, 180.0, -135.0] {
        for (edge, near) in [(-90.0f32, -89.9f32), (90.0, 89.9)] {
            let (right_edge, up_edge) = thrown_item_axes(CameraType::FirstPerson, yaw, edge);
            let (right_near, up_near) = thrown_item_axes(CameraType::FirstPerson, yaw, near);
            assert!(
                right_edge.dot(right_near) > 0.99 && up_edge.dot(up_near) > 0.99,
                "yaw {yaw}, pitch {edge}: {right_edge}/{up_edge} vs {right_near}/{up_near} at {near}"
            );
        }
    }
}

#[test]
fn thrown_item_billboard_faces_the_front_camera() {
    // Camera.setup turns the front camera round (yaw + 180, -pitch): its
    // right is the player's left.
    let (right, up) = thrown_item_axes(CameraType::FirstPerson, 0.0, 0.0);
    assert!(right.abs_diff_eq(glam::Vec3::NEG_X, 1e-5), "{right}");
    assert!(up.abs_diff_eq(glam::Vec3::Y, 1e-5), "{up}");
    let (right, up) = thrown_item_axes(CameraType::ThirdPersonFront, 0.0, 0.0);
    assert!(right.abs_diff_eq(glam::Vec3::X, 1e-5), "{right}");
    assert!(up.abs_diff_eq(glam::Vec3::Y, 1e-5), "{up}");
    let (right, _) = thrown_item_axes(CameraType::ThirdPersonBack, 0.0, 0.0);
    assert!(right.abs_diff_eq(glam::Vec3::NEG_X, 1e-5), "{right}");
}
