//! Client-side native block-entity rendering, using the existing entity batch.
use std::collections::HashMap;

use horizon_client::{
    block_entity_render::vertex_light, containers::BlockEntity, hud::Rect, mesher::Vertex,
    world::World,
};
use horizon_world::nbt::Compound;

/// Inputs for one native block-entity draw. The data is the client's copy,
/// not the server's saved compound. It is borrowed only during the call.
#[allow(dead_code)]
pub(crate) struct NativeBlockEntityRender<'a> {
    pub(crate) pos: [i32; 3],
    pub(crate) state: u32,
    pub(crate) id: &'a str,
    pub(crate) data: &'a Compound,
    /// Block origin relative to the camera, in blocks.
    pub(crate) origin: [f32; 3],
    /// Fraction between the preceding and current client tick.
    pub(crate) partial: f32,
    pub(crate) game_time: i64,
    /// Packed light for `Vertex::light`.
    pub(crate) light: u32,
    /// Named sprites in the entity/GUI atlas, used with vertex layer 0.
    /// This atlas currently packs Minecraft textures only.
    pub(crate) sprites: &'a HashMap<String, Rect>,
}

pub(super) struct Frame<'a> {
    pub(super) world: &'a World,
    pub(super) sprites: &'a HashMap<String, Rect>,
    pub(super) camera: glam::DVec3,
    pub(super) partial: f32,
    pub(super) visible: &'a dyn Fn([i32; 3]) -> bool,
}

/// Draw only client mod entities within vanilla's default renderer range.
pub(super) fn draw(
    frame: &Frame<'_>,
    pos: [i32; 3],
    state: u32,
    out: &mut Vec<Vertex>,
    hook: impl FnOnce(&NativeBlockEntityRender<'_>, &mut Vec<Vertex>),
) {
    let Some(BlockEntity::Mod(entity)) = frame.world.block_entities.get(&(pos[0], pos[1], pos[2]))
    else {
        return;
    };
    let position = glam::DVec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]));
    if (position + glam::DVec3::splat(0.5)).distance_squared(frame.camera) > 64.0 * 64.0
        || !(frame.visible)(pos)
    {
        return;
    }
    let context = NativeBlockEntityRender {
        pos,
        state,
        id: &entity.id,
        data: &entity.nbt,
        origin: (position - frame.camera).as_vec3().to_array(),
        partial: frame.partial,
        game_time: frame.world.game_time,
        light: vertex_light(frame.world.light(pos[0], pos[1], pos[2])),
        sprites: frame.sprites,
    };
    hook(&context, out);
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::{
        chunk::{Chunk, Light, MIN_Y},
        containers::ModBlockEntity,
    };
    use horizon_world::{nbt::Tag, registry::BlockStateId};
    use std::sync::Arc;

    fn fixture() -> World {
        let mut world = World::new(BlockStateId(0), 0);
        let mut chunk = Chunk::empty(0, 0);
        Arc::make_mut(&mut chunk.sections[((0 - MIN_Y) >> 4) as usize]).light =
            Light::Uniform(0xA3);
        world.chunks.insert((0, 0), Arc::new(chunk));
        let mut entity = ModBlockEntity {
            id: "example:machine".into(),
            ..Default::default()
        };
        entity.nbt.insert("angle".into(), Tag::Int(8));
        world
            .block_entities
            .insert((1, 0, 2), BlockEntity::Mod(Box::new(entity)));
        world.game_time = 12;
        world
    }

    #[test]
    fn empty_output_preserves_batch_and_missing_entity_never_calls_hook() {
        let world = fixture();
        let sprites = HashMap::new();
        let frame = Frame {
            world: &world,
            sprites: &sprites,
            camera: glam::DVec3::ZERO,
            partial: 0.5,
            visible: &|_| true,
        };
        let mut out = vec![Vertex {
            pos: [9.0; 3],
            uv: [0.0; 2],
            layer: 0,
            color: 0,
            light: 0,
            normal: [0; 4],
            glint_uv: [0.0; 2],
        }];
        draw(&frame, [1, 0, 2], 7, &mut out, |_, _| {});
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].pos, [9.0; 3]);
        draw(&frame, [2, 0, 2], 7, &mut out, |_, _| {
            panic!("missing entity")
        });
    }

    #[test]
    fn culls_outside_view_and_default_distance() {
        let world = fixture();
        let sprites = HashMap::new();
        let mut out = Vec::new();
        let frame = Frame {
            world: &world,
            sprites: &sprites,
            camera: glam::DVec3::ZERO,
            partial: 0.5,
            visible: &|_| false,
        };
        draw(&frame, [1, 0, 2], 7, &mut out, |_, _| {
            panic!("outside view")
        });
        let frame = Frame {
            camera: glam::DVec3::splat(100.0),
            visible: &|_| true,
            ..frame
        };
        draw(&frame, [1, 0, 2], 7, &mut out, |_, _| {
            panic!("outside distance")
        });
    }

    #[test]
    fn supplies_interpolation_data_camera_origin_and_local_light() {
        let world = fixture();
        let sprites = HashMap::new();
        let frame = Frame {
            world: &world,
            sprites: &sprites,
            camera: glam::DVec3::new(0.25, 0.5, 1.0),
            partial: 0.25,
            visible: &|_| true,
        };
        let mut called = false;
        let mut out = Vec::new();
        draw(&frame, [1, 0, 2], 7, &mut out, |ctx, out| {
            called = true;
            assert_eq!(ctx.pos, [1, 0, 2]);
            assert_eq!(ctx.state, 7);
            assert_eq!(ctx.id, "example:machine");
            assert_eq!(ctx.data.get(&"angle".into()), Some(&Tag::Int(8)));
            assert_eq!(ctx.origin, [0.75, -0.5, 1.0]);
            assert_eq!(ctx.partial, 0.25);
            assert_eq!(ctx.game_time, 12);
            assert_eq!(ctx.light, 0x30a0);
            let interpolated = 2.0 + (6.0 - 2.0) * ctx.partial;
            for _ in 0..4 {
                out.push(Vertex {
                    pos: [ctx.origin[0] + interpolated, ctx.origin[1], ctx.origin[2]],
                    uv: [0.0; 2],
                    layer: 0,
                    color: 0,
                    light: ctx.light,
                    normal: [0; 4],
                    glint_uv: [0.0; 2],
                });
            }
        });
        assert!(called);
        assert_eq!(out.len(), 4);
        assert!(
            out.iter()
                .all(|v| v.pos == [3.75, -0.5, 1.0] && v.light == 0x30a0)
        );
    }
}
