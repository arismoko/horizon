//! Standalone models under a block entity's pose with the terrain's own materials
//! (`BlockEntityDrawContext::terrain_model`): explicit solid/cutout/cutout-mipped selection,
//! directional shading, UV shift and shrink, and tint.
use super::native_block_entity_draw::BlockEntityDraw;
use mod_api::{ModelError, ModelRef, TerrainMaterial, TerrainModelOptions, TerrainShade};
use sunrise_client::{
    models::catalog::BakedFace,
    terrain_model::{
        NativeModelMaterial, NativeModelRenderOptions, NativeModelShade, NativeModelUvShift,
        draw_model,
    },
};

impl crate::game::ClientGame<'_> {
    /// Draws the resolved standalone `model` under `draw`'s pose from the block sprite arrays.
    /// See `BlockEntityDrawContext::terrain_model`.
    pub(in crate::game) fn native_draw_terrain_model(
        &self,
        draw: &mut BlockEntityDraw,
        model: ModelRef,
        options: TerrainModelOptions<'_>,
    ) -> Result<(), ModelError> {
        use mod_api::__private::game::Fields;
        let fields = model.fields();
        if fields.generation != self.client.standalone_geometry.generation() {
            return Err(ModelError::StaleModel);
        }
        let (faces, special) = self
            .client
            .standalone_geometry
            .instance_slot(fields.slot)
            .and_then(|slot| Some((self.model_faces(fields.slot)?, slot.special)))
            .ok_or(ModelError::UnknownModel)?;
        let layers = &self.client.client_pack.render.sprite_layers;
        let color = options.color;
        draw_terrain_model(
            draw,
            faces,
            |name| {
                layers
                    .get(name)
                    .or_else(|| layers.get(name.strip_prefix("minecraft:")?))
                    .copied()
            },
            options,
            |pose, light, scope| {
                if let Some(state) = special {
                    self.special_model_part(state, pose, light, color, scope);
                }
            },
        );
        Ok(())
    }
}

/// The renderer's options for the API's.
fn engine_options(options: TerrainModelOptions<'_>) -> NativeModelRenderOptions<'_> {
    NativeModelRenderOptions {
        material: match options.material {
            TerrainMaterial::Model => NativeModelMaterial::Model,
            TerrainMaterial::Solid => NativeModelMaterial::Solid,
            TerrainMaterial::Cutout => NativeModelMaterial::Cutout,
            TerrainMaterial::CutoutMipped => NativeModelMaterial::CutoutMipped,
            _ => unreachable!("TerrainMaterial: unhandled variant"),
        },
        shade: match options.shade {
            TerrainShade::Model => NativeModelShade::Model,
            TerrainShade::Off => NativeModelShade::Off,
            TerrainShade::Entity => NativeModelShade::Entity,
            _ => unreachable!("TerrainShade: unhandled variant"),
        },
        uv_shift: options.uv_shift.map(|shift| NativeModelUvShift {
            source_sprite: shift.source_sprite,
            target_sprite: shift.target_sprite,
            scale: shift.scale,
            offset: shift.offset,
        }),
        uv_shrink_ratio: options.uv_shrink_ratio,
        tint: options.tint,
        color: options.color,
        ..Default::default()
    }
}

/// Appends each valid quad of `faces` under `draw`'s pose, then `special`'s vertices under the
/// same pose and light, within `draw`'s device-derived batch capacity.
pub(super) fn draw_terrain_model(
    draw: &mut BlockEntityDraw,
    faces: &[BakedFace],
    sprite_layer: impl Fn(&str) -> Option<u32>,
    options: TerrainModelOptions<'_>,
    special: impl FnOnce(glam::Mat4, u32, &mut sunrise_client::gfx::EntityQuadScope<'_>),
) {
    let pose = draw.pose();
    let light = draw.element_ambient();
    let start = draw.vertices_len();
    {
        let mut scope = draw.quad_scope();
        if !scope.has_room() {
            return;
        }
        draw_model(
            faces,
            pose,
            light,
            engine_options(options),
            sprite_layer,
            |quad| scope.submit_quad(quad),
        );
        special(pose, light, &mut scope);
    }
    draw.lit_since(start, BlockEntityDraw::cell_centre, draw.light() & 0xffff);
    draw.crumbling_since(start);
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat4, Vec3};
    use mod_api::TerrainUvShift;
    use sunrise_assets::{
        bake::{RenderType, bake_flat_face},
        model::{Direction, Material},
    };
    use sunrise_client::{
        block_entity_render::diffuse_lights,
        gfx::{
            EntityQuadCapacity, TERRAIN_MODEL_CUTOUT_MIPPED_OVERLAY, TERRAIN_MODEL_CUTOUT_OVERLAY,
            TERRAIN_MODEL_SOLID_OVERLAY,
        },
        mesher::Vertex,
    };

    /// A full south face of `sprite` (layer 7) in `render_type`, tinted when `tint_index` >= 0.
    fn face(render_type: Option<RenderType>, tint_index: i32) -> BakedFace {
        let mut quad = bake_flat_face(
            [0.0; 3],
            [16.0; 3],
            [0.0, 0.0, 16.0, 16.0],
            Direction::South,
            Material {
                sprite: "test:casing".into(),
                force_translucent: false,
            },
            tint_index,
        )
        .unwrap();
        quad.render_type = render_type;
        BakedFace::new(quad, 7, true, false, None)
    }

    fn sprite(name: &str) -> Option<u32> {
        match name {
            "test:casing" => Some(7),
            "test:belt" => Some(11),
            "minecraft:missingno" => Some(1),
            _ => None,
        }
    }

    /// Full bright, a block east of the camera.
    fn drawn(faces: &[BakedFace], options: TerrainModelOptions<'_>) -> Vec<Vertex> {
        drawn_with_budget(faces, options, EntityQuadCapacity::default_device(), 0)
    }

    fn drawn_with_budget(
        faces: &[BakedFace],
        options: TerrainModelOptions<'_>,
        capacity: EntityQuadCapacity,
        reserved_quads: usize,
    ) -> Vec<Vertex> {
        let mut draw = BlockEntityDraw::posed(
            0.0,
            0,
            0xF0F0,
            [0; 3],
            Mat4::from_translation(Vec3::X),
            diffuse_lights(false),
            Vec::new(),
        )
        .with_quad_budget(capacity, reserved_quads);
        draw_terrain_model(&mut draw, faces, sprite, options, |_, _, _| {});
        draw.into_vertices()
    }

    fn materials(vertices: &[Vertex]) -> Vec<u32> {
        vertices
            .iter()
            .step_by(4)
            .map(|v| v.layer & 0xFFFF)
            .collect()
    }

    #[test]
    fn each_quad_keeps_its_cutout_material_unless_one_is_chosen_for_all() {
        let faces = [
            face(None, -1),
            face(Some(RenderType::Cutout), -1),
            face(Some(RenderType::CutoutMipped), -1),
        ];
        let vertices = drawn(&faces, TerrainModelOptions::default());
        assert_eq!(
            materials(&vertices),
            [
                TERRAIN_MODEL_SOLID_OVERLAY,
                TERRAIN_MODEL_CUTOUT_OVERLAY,
                TERRAIN_MODEL_CUTOUT_MIPPED_OVERLAY
            ]
        );
        assert!(vertices.iter().all(|v| v.layer >> 16 == 7));
        assert_eq!(vertices[0].pos[0], 1.0, "drawn under the pose");
        let vertices = drawn(
            &faces,
            TerrainModelOptions::default().with_material(TerrainMaterial::CutoutMipped),
        );
        assert_eq!(
            materials(&vertices),
            [TERRAIN_MODEL_CUTOUT_MIPPED_OVERLAY; 3]
        );
    }

    #[test]
    fn a_translucent_quad_takes_the_translucent_material_under_the_models_and_a_chosen_one_wins() {
        let faces = [face(None, -1), face(Some(RenderType::Translucent), -1)];
        let vertices = drawn(&faces, TerrainModelOptions::default());
        assert_eq!(
            materials(&vertices),
            [
                TERRAIN_MODEL_SOLID_OVERLAY,
                sunrise_client::gfx::TERRAIN_MODEL_TRANSLUCENT_OVERLAY
            ]
        );
        let vertices = drawn(
            &faces,
            TerrainModelOptions::default().with_material(TerrainMaterial::Cutout),
        );
        assert_eq!(materials(&vertices), [TERRAIN_MODEL_CUTOUT_OVERLAY; 2]);
    }

    #[test]
    fn shading_is_chosen_apart_from_full_bright_light() {
        let shaded =
            |vertices: &[Vertex]| vertices[0].glint_uv != [0.0; 2] || vertices[0].normal != [0; 4];
        // A loader quad that takes no directional shading.
        let flat = || {
            let mut face = face(None, -1);
            face.quad.shade = false;
            face
        };
        for (faces, shade, expected) in [
            ([face(None, -1)], TerrainShade::Model, true),
            ([flat()], TerrainShade::Model, false),
            ([face(None, -1)], TerrainShade::Off, false),
            ([flat()], TerrainShade::Entity, true),
        ] {
            let vertices = drawn(&faces, TerrainModelOptions::default().with_shade(shade));
            assert_eq!(shaded(&vertices), expected, "{shade:?}");
            assert_eq!(vertices[0].light, 0xF0F0);
        }
    }

    /// `unlit` covers the helpers' draws, not `terrain_model`: its shading and light stay as
    /// `TerrainModelOptions` and the draw's light give them.
    #[test]
    fn unlit_leaves_a_terrain_models_shading_and_light() {
        let faces = [face(None, -1)];
        let drawn = |unlit: bool| {
            let mut draw = BlockEntityDraw::posed(
                0.0,
                0,
                0x3070,
                [0; 3],
                Mat4::IDENTITY,
                diffuse_lights(false),
                Vec::new(),
            );
            draw.unlit(unlit);
            let options = TerrainModelOptions::default().with_shade(TerrainShade::Model);
            draw_terrain_model(&mut draw, &faces, sprite, options, |_, _, _| {});
            draw.into_vertices()
        };
        let unlit = drawn(true);
        let bytes = |v: &[Vertex]| bytemuck::cast_slice::<_, u8>(v).to_vec();
        assert_eq!(bytes(&unlit), bytes(&drawn(false)));
        assert!(
            unlit[0].glint_uv != [0.0; 2] || unlit[0].normal != [0; 4],
            "still shaded"
        );
        assert!(unlit.iter().all(|v| v.light == 0x3070));
    }

    #[test]
    fn the_tint_multiplies_only_quads_with_a_tint_index() {
        let vertices = drawn(
            &[face(None, 0), face(None, -1)],
            TerrainModelOptions::default().with_tint(0x80_4020),
        );
        // Vertex colours are ABGR.
        assert_eq!(vertices[0].color, 0xFF20_4080);
        assert_eq!(vertices[4].color, u32::MAX);
    }

    #[test]
    fn shift_fallback_per_quad_isolation_and_vertex_limit() {
        let belt = TerrainUvShift::new("test:casing", "test:belt", [0.5; 2], [0.5, 0.0]);
        let vertices = drawn(
            &[face(None, -1)],
            TerrainModelOptions::default().with_uv_shift(belt),
        );
        assert_eq!(vertices.len(), 4);
        assert!(vertices.iter().all(|v| v.layer >> 16 == 11));
        assert!(vertices.iter().all(|v| v.uv[0] >= 0.5 && v.uv[1] <= 0.5));

        // Unmatched source sprite is a no-op and keeps the quad's own sprite.
        let vertices = drawn(
            &[face(None, -1)],
            TerrainModelOptions::default().with_uv_shift(TerrainUvShift::new(
                "test:belt",
                "test:casing",
                [1.0; 2],
                [0.0; 2],
            )),
        );
        assert_eq!(vertices.len(), 4);
        assert!(vertices.iter().all(|v| v.layer >> 16 == 7));

        // Missing target sprite resolves to `minecraft:missingno` (layer 1).
        let vertices = drawn(
            &[face(None, -1)],
            TerrainModelOptions::default().with_uv_shift(TerrainUvShift::new(
                "test:casing",
                "test:missing",
                [1.0; 2],
                [0.0; 2],
            )),
        );
        assert_eq!(vertices.len(), 4);
        assert!(vertices.iter().all(|v| v.layer >> 16 == 1));

        // Non-finite shift skips the affected quad while leaving non-matching quads intact.
        let mut belt_face = face(None, -1);
        belt_face.quad.material.sprite = "test:belt".into();
        belt_face.layer = 11;
        let vertices = drawn(
            &[face(None, -1), belt_face],
            TerrainModelOptions::default().with_uv_shift(TerrainUvShift::new(
                "test:casing",
                "test:belt",
                [f32::NAN; 2],
                [0.0; 2],
            )),
        );
        assert_eq!(vertices.len(), 4);
        assert!(vertices.iter().all(|v| v.layer >> 16 == 11));

        // Finite shrink ratio outside 0..=1 is accepted directly.
        let vertices = drawn(
            &[face(None, -1)],
            TerrainModelOptions::default().with_uv_shrink_ratio(1.5),
        );
        assert_eq!(vertices.len(), 4);

        // Device-derived combined batch budget stops cleanly at whole quads without partial
        // vertices, accounting for quads already reserved by other draw sources.
        let capacity =
            EntityQuadCapacity::from_max_buffer_size(EntityQuadCapacity::VERTEX_QUAD_BYTES * 5);
        let vertices = drawn_with_budget(
            &[face(None, -1), face(None, -1), face(None, -1)],
            TerrainModelOptions::default(),
            capacity,
            3,
        );
        assert_eq!(vertices.len(), 8);
    }
}
