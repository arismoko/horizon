//! The dying ender dragon's rays and dissolve (`EnderDragonRenderer`).
use sunrise_client::mesher::Vertex;
use sunrise_core::random::{LegacyRandom, RandomSource};

/// `submitRays`: the death rays from a fixed seed, fading in white to magenta.
pub fn dragon_rays(
    base: glam::Mat4,
    t: f32,
    white: sunrise_client::hud::Rect,
    out: &mut Vec<Vertex>,
) {
    // Vanilla's dragonRays has no texture. Sample inside the white sprite:
    // its border can round into transparent neighbours during interpolation.
    let uv = [(white.u0 + white.u1) / 2.0, (white.v0 + white.v1) / 2.0];
    let over = if t > 0.8 {
        ((t - 0.8) / 0.2).min(1.0)
    } else {
        0.0
    };
    let inner = (((1.0 - over) * 255.0) as u32) << 24 | 0x00FF_FFFF;
    let outer = 0x00FF_00FF;
    let mut random = LegacyRandom::new(432);
    let mut m = base;
    let count = ((t + t * t) / 2.0 * 60.0).floor() as i32;
    let half_sqrt3 = 3f32.sqrt() / 2.0;
    let tau = std::f32::consts::TAU;
    for _ in 0..count {
        let mut f = || random.next_float() * tau;
        let q1 = glam::Quat::from_rotation_x(f())
            * glam::Quat::from_rotation_y(f())
            * glam::Quat::from_rotation_z(f());
        let (a, b, c) = (f(), f(), f());
        let q2 = glam::Quat::from_rotation_x(a)
            * glam::Quat::from_rotation_y(b)
            * glam::Quat::from_rotation_z(c + t * std::f32::consts::FRAC_PI_2);
        m *= glam::Mat4::from_quat(q1 * q2);
        let length = random.next_float() * 20.0 + 5.0 + over * 10.0;
        let width = random.next_float() * 2.0 + 1.0 + over * 2.0;
        let o = glam::Vec3::ZERO;
        let left = glam::Vec3::new(-half_sqrt3 * width, length, -0.5 * width);
        let right = glam::Vec3::new(half_sqrt3 * width, length, -0.5 * width);
        let bottom = glam::Vec3::new(0.0, length, width);
        for (p1, p2) in [(left, right), (right, bottom), (bottom, left)] {
            // A triangle as a quad with its last corner repeated.
            for (p, c) in [(o, inner), (p1, outer), (p2, outer), (p2, outer)] {
                out.push(Vertex {
                    normal: [0; 4],
                    pos: m.transform_point3(p).to_array(),
                    uv,
                    layer: sunrise_client::gfx::DRAGON_RAYS_OVERLAY,
                    color: c,
                    light: 0,

                    glint_uv: [0.0, 0.0],
                });
            }
        }
    }
}

/// The dying model's colour, `ARGB.white(1 - deathTime / 200)`: `Mth.floor` of the alpha
/// shifted as a Java int, so past 200 it wraps as vanilla's does.
pub fn dissolve_color(death: f32) -> u32 {
    let alpha = ((1.0 - death / 200.0) * 255.0).floor() as i32;
    (alpha << 24) as u32 | 0x00FF_FFFF
}
/// `DissolveMaskSampler` at the vertex's `UV0`: the same place in the mask's sprite as in the
/// skin's, as the dissolve overlay reads it from `glint_uv` (entity.wgsl).
pub fn dissolve_mask(quads: &mut [Vertex], skin: [f32; 4], mask: [f32; 4]) {
    for v in quads {
        let u = (v.uv[0] - skin[0]) / (skin[2] - skin[0]);
        let w = (v.uv[1] - skin[1]) / (skin[3] - skin[1]);
        v.glint_uv = [
            mask[0] + (mask[2] - mask[0]) * u,
            mask[1] + (mask[3] - mask[1]) * w,
        ];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn run_dragon_render_parity_fixture() {
        let path = std::env::var("SUNRISE_DRAGON_RENDER_OUTPUT").expect("harness output path");
        let mut cases = Vec::new();
        for t in [0.0, 0.05, 0.25, 0.5, 0.8, 0.9, 0.95, 1.0] {
            let mut vertices = Vec::new();
            dragon_rays(
                glam::Mat4::IDENTITY,
                t,
                sunrise_client::hud::Rect {
                    u0: 0.0,
                    v0: 0.0,
                    u1: 1.0,
                    v1: 1.0,
                    w: 1,
                    h: 1,
                },
                &mut vertices,
            );
            // The fourth quad vertex only pads a triangle for our index buffer.
            let triangles: Vec<_> = vertices
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|q| &q[..3])
                .map(|v| serde_json::json!([v.pos[0], v.pos[1], v.pos[2], v.color]))
                .collect();
            cases.push(triangles);
        }
        std::fs::write(path, serde_json::to_vec(&cases).unwrap()).unwrap();
    }

    /// Driven by `tools/test_dragon_render_parity.py`; compares the actual
    /// ray vertices with EnderDragonRenderer's captured VertexConsumer output.
    #[test]
    #[ignore]
    fn dragon_render_parity_fixture() {
        run_dragon_render_parity_fixture();
    }

    #[test]
    fn the_dying_model_reads_the_mask_where_it_reads_the_skin() {
        // The skin in the atlas's top-left quarter, the mask in its bottom-right one.
        let mut quad = [Vertex {
            pos: [0.0; 3],
            uv: [0.25, 0.125],
            layer: 0,
            color: 0,
            light: 0,
            normal: [0; 4],
            glint_uv: [0.0; 2],
        }];
        dissolve_mask(&mut quad, [0.0, 0.0, 0.5, 0.5], [0.5, 0.5, 1.0, 1.0]);
        assert_eq!(quad[0].glint_uv, [0.75, 0.625]);
        // ARGB.white(1 - deathTime / 200): floor(127.5), and past 200 the int shift wraps.
        assert_eq!(dissolve_color(100.0), 0x7FFF_FFFF);
        assert_eq!(dissolve_color(200.5), 0xFFFF_FFFF);
    }

    #[test]
    fn death_rays_sample_inside_the_white_sprite() {
        // A four-texel white sprite surrounded by transparent atlas pixels.
        // A border UV can round outside it during perspective interpolation.
        let size = 1024.0;
        let white = sunrise_client::hud::Rect {
            u0: 20.0 / size,
            v0: 60.0 / size,
            u1: 24.0 / size,
            v1: 64.0 / size,
            w: 4,
            h: 4,
        };
        for t in [0.25, 0.8, 0.95, 1.0] {
            let mut vertices = Vec::new();
            dragon_rays(glam::Mat4::IDENTITY, t, white, &mut vertices);
            assert!(!vertices.is_empty());
            for v in &vertices {
                assert!(
                    v.uv[0] > white.u0 + 0.5 / size && v.uv[0] < white.u1 - 0.5 / size,
                    "death-ray U touches the atlas border: {}",
                    v.uv[0]
                );
                assert!(
                    v.uv[1] > white.v0 + 0.5 / size && v.uv[1] < white.v1 - 0.5 / size,
                    "death-ray V touches the atlas border: {}",
                    v.uv[1]
                );
            }
        }
    }
}
