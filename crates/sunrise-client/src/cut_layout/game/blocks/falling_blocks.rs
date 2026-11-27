#![cfg(test)]
#[cfg(feature = "client")]
use sunrise_core::position::BlockPos;
#[cfg(test)]
#[path = "falling_blocks/moving_piston_tests.rs"]
pub(crate) mod moving_piston_tests;

#[cfg(test)]
pub(crate) mod geometry_tests {
    #[cfg(feature = "client")]
    use super::*;

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn falling_model_cache_matches_direct_vertices() {
        // Geometry belongs to the client resource graph; server packs have no models.
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            for frame in 0..2 {
                for (name, pos, origin, light) in [
                    (
                        "minecraft:sand",
                        BlockPos::new(1, 80, 3),
                        glam::Vec3::new(-3.25, 12.5, 0.0),
                        0x7010,
                    ),
                    (
                        "minecraft:sand",
                        BlockPos::new(9, 90, -4),
                        glam::Vec3::new(8.125, -0.0, -9.5),
                        0xf020,
                    ),
                    (
                        "minecraft:gravel",
                        BlockPos::new(-2, 70, 5),
                        glam::Vec3::new(-0.0, -17.75, 2.5),
                        0x5080,
                    ),
                ] {
                    let state = game.pack.assets.registry.default_state(name).unwrap();
                    let seed = sunrise_core::random::position_seed(pos);
                    let mut expected = Vec::new();
                    game.block_model_vertices(
                        state,
                        seed,
                        glam::Mat4::from_translation(origin),
                        light,
                        false,
                        &mut expected,
                    );
                    let mut actual = Vec::new();
                    game.falling_model_vertices(state, pos, origin, light, &mut actual);
                    let (_, tint) = game.falling_block_face_sprite(state).unwrap();
                    let tint = sunrise_client::mobs::render::abgr(tint);
                    for vertices in [&mut expected, &mut actual] {
                        for v in vertices {
                            v.color = sunrise_client::hud_layout::multiply(v.color, tint);
                        }
                    }
                    assert!(!expected.is_empty());
                    assert_eq!(actual.len(), expected.len());
                    for (a, b) in actual.iter().zip(&expected) {
                        assert_eq!(
                            a.pos.map(f32::to_bits),
                            b.pos.map(f32::to_bits),
                            "frame {frame}"
                        );
                        assert_eq!(a.uv.map(f32::to_bits), b.uv.map(f32::to_bits));
                        assert_eq!(a.glint_uv.map(f32::to_bits), b.glint_uv.map(f32::to_bits));
                        assert_eq!(
                            (a.color, a.light, a.normal, a.layer),
                            (b.color, b.light, b.normal, b.layer)
                        );
                    }
                }
            }
        });
    }
}
