//! An item or a standalone model drawn in a screen in 3D under a caller's turn
//! (`DrawContext::model_3d`): what a symmetry wand's screen shows its wand and its mirror with.
//! The item is the slot's own item (`item_render::gui::draw_posed`) under a pose with the turn in
//! it; the model is projected as a scene's blocks are. Both have a depth of their own
//! (`HudBatch::depth_glint_over` for items, `HudBatch::depth_over` for models).
use super::native_scene::{SceneQuad, face_shade, project_quads, shaded};
use crate::game::ClientGame;
use glam::{Mat4, Vec3};
use mod_api::{Gui3dError, Gui3dSubject, Gui3dView, ModelError};
use sunrise_client::{hud::HudBatch, world_geometry::row_major_3x4};

/// The pose of a subject drawn as `view` says: its centre at `view.at`, a block `view.size`
/// pixels across, the caller's `transform` applied in the frame `x` right, `y` up, `z` towards
/// the viewer. With the identity at size 16 it is a slot's pose (`translate(x + 8, y + 8)`,
/// `scale(16, -16, 16)`).
fn view_pose(view: Gui3dView) -> Mat4 {
    Mat4::from_translation(Vec3::new(view.at[0], view.at[1], 0.0))
        * Mat4::from_scale(Vec3::new(view.size, -view.size, view.size))
        * row_major_3x4(view.transform)
}

impl ClientGame<'_> {
    /// Draws `subject` as `view` says into `batch`. See `DrawContext::model_3d`.
    pub(in crate::game) fn native_model_3d(
        &self,
        batch: &mut HudBatch,
        subject: Gui3dSubject<'_>,
        view: Gui3dView,
    ) -> Result<(), Gui3dError> {
        if !self.common.native_window() {
            return Err(Gui3dError::Unavailable);
        }
        let pose = || {
            if !view.is_drawable() {
                return Err(Gui3dError::NotFinite);
            }
            let pose = view_pose(view);
            pose.is_finite()
                .then_some(pose)
                .ok_or(Gui3dError::NotFinite)
        };
        let (start, blends) = (batch.vertices.len(), batch.blends.len());
        match subject {
            Gui3dSubject::Item(stack) => {
                let item =
                    mod_api::__private::item_key_id(stack.item()).ok_or(Gui3dError::UnknownItem)?;
                // GuiGraphicsExtractor.item checks for emptiness before inspecting components.
                // ItemStack already guarantees a nonzero count.
                if item == "minecraft:air" {
                    return Ok(());
                }
                let pose = pose()?;
                if self.common.res.items().get(item).is_none() {
                    return Err(Gui3dError::UnknownItem);
                }
                let stack =
                    sunrise_gameplay_common::game::api_values::encode_stack(stack, item.into())
                        .map_err(|_| Gui3dError::InvalidStack)?;
                if !self.draw_item_model_posed(batch, &stack, pose) {
                    return Err(Gui3dError::NotFinite);
                }
            }
            Gui3dSubject::Model(model) => self.draw_model_posed(batch, model, pose()?)?,
        }
        // A turn can overflow a corner: draw nothing rather than something broken.
        if !batch.vertices[start..]
            .iter()
            .all(|v| v.pos.iter().all(|c| c.is_finite()) && v.depth.is_finite())
        {
            batch.vertices.truncate(start);
            batch.blends.truncate(blends);
            return Err(Gui3dError::NotFinite);
        }
        Ok(())
    }

    /// A standalone model's faces under `pose` about the centre of its 0 to 1 cube, full bright
    /// and shaded by face, back faces dropped, far to near, with a depth of its own.
    fn draw_model_posed(
        &self,
        batch: &mut HudBatch,
        model: mod_api::ModelRef,
        pose: Mat4,
    ) -> Result<(), Gui3dError> {
        use mod_api::__private::game::Fields;
        let fields = model.fields();
        if fields.generation != self.client.standalone_geometry.generation() {
            return Err(Gui3dError::Model(ModelError::StaleModel));
        }
        let model = self
            .client
            .standalone_geometry
            .instance_slot(fields.slot)
            .and_then(|_| {
                self.client
                    .client_pack
                    .render
                    .model_catalog
                    .model_by_slot(fields.slot)
            })
            .ok_or(Gui3dError::Model(ModelError::UnknownModel))?;
        let mut quads = Vec::with_capacity(model.faces.len());
        for face in &model.faces {
            let Some(sprite) = self
                .client
                .client_pack
                .layer_names
                .get(&face.layer)
                .and_then(|name| self.client.gui.atlas.sprites.get(name))
            else {
                continue;
            };
            let [nx, ny, nz] =
                sunrise_core::geometry::Direction::ALL[face.quad.facing as usize].normal();
            let normal = Vec3::new(nx as f32, ny as f32, nz as f32);
            quads.push(SceneQuad {
                corners: face.quad.positions.map(Vec3::from_array),
                uv: face.quad.uv.map(|uv| {
                    [
                        sprite.u0 + uv[0] * (sprite.u1 - sprite.u0),
                        sprite.v0 + uv[1] * (sprite.v1 - sprite.v0),
                    ]
                }),
                color: shaded(0xFFFF_FFFF, face_shade(normal)),
                normal,
                cull: true,
            });
        }
        // The scene's own projection, about the centre of the model's 0 to 1 cube.
        let seen = project_quads(quads, pose * Mat4::from_translation(Vec3::splat(-0.5)))
            .ok_or(Gui3dError::NotFinite)?;
        let start = batch.vertices.len();
        let (mut lo, mut hi) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
        for face in seen {
            for c in face.corners {
                for axis in 0..2 {
                    lo[axis] = lo[axis].min(c[axis]);
                    hi[axis] = hi[axis].max(c[axis]);
                }
            }
            batch.quad_3d(face.corners, face.uv, face.color);
        }
        if batch.vertices.len() > start {
            batch.depth_over(start, lo, hi);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::host::draw::GameDrawContext;
    use mod_api::{DrawContext, IDENTITY_TRANSFORM};

    #[test]
    fn the_identity_at_size_16_is_the_slots_pose() {
        let pose = view_pose(Gui3dView::new([40.0, 30.0], 16.0, IDENTITY_TRANSFORM));
        let slot = Mat4::from_cols_array(&[
            16.0, 0.0, 0.0, 0.0, // x
            0.0, -16.0, 0.0, 0.0, // y
            0.0, 0.0, 16.0, 0.0, // z
            40.0, 30.0, 0.0, 1.0, // translation
        ]);
        assert_eq!(pose, slot);
    }

    #[test]
    fn a_quarter_turn_about_z_moves_the_right_edge_to_the_top() {
        // Row-major 3x4: x' = -y, y' = x, z' = z (counterclockwise seen from the viewer).
        let turn = [0.0, -1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let pose = view_pose(Gui3dView::new([100.0, 100.0], 10.0, turn));
        // The model's +x point lands above the centre on the screen (y down).
        let right = pose.transform_point3(Vec3::X);
        assert_eq!(right, Vec3::new(100.0, 90.0, 0.0));
    }

    /// A borrowed stack is reusable, and a refused second draw leaves the first draw intact.
    #[test]
    #[ignore = "daily"]
    fn an_overflowing_item_is_refused_and_leaves_the_batch_as_it_was() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let stack = mod_api::ItemStack::new(
                mod_api::__private::item_key("minecraft:diamond_sword").unwrap(),
                1,
            )
            .unwrap();
            let mut batch = HudBatch::scratch([320, 240]);
            let mut draw = GameDrawContext::new(game, &mut batch);
            let view = Gui3dView::new([40.0, 30.0], 16.0, IDENTITY_TRANSFORM);
            draw.model_3d(Gui3dSubject::Item(&stack), view).unwrap();
            let (vertices, blends) = (draw.surface.vertices.len(), draw.surface.blends.len());
            assert!(vertices > 0, "the first draw submitted the sword");

            // Twice 3e38 pixels a block is not finite: the pose overflows.
            let mut turn = IDENTITY_TRANSFORM;
            turn[0] = 2.0;
            let view = Gui3dView::new([40.0, 30.0], 3.0e38, turn);
            assert_eq!(
                draw.model_3d(Gui3dSubject::Item(&stack), view),
                Err(Gui3dError::NotFinite)
            );
            assert_eq!(
                (draw.surface.vertices.len(), draw.surface.blends.len()),
                (vertices, blends)
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn air_stack_is_noop_and_unknown_item_is_refused() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let mut batch = HudBatch::scratch([320, 240]);
            let mut draw = GameDrawContext::new(game, &mut batch);
            let view = Gui3dView::new([40.0, 30.0], 16.0, IDENTITY_TRANSFORM);

            let air =
                mod_api::ItemStack::new(mod_api::__private::item_key("minecraft:air").unwrap(), 1)
                    .unwrap();
            let mut sword = mod_api::ItemStack::of(
                mod_api::__private::item_key("minecraft:diamond_sword").unwrap(),
            );
            let invalid = mod_api::Tag::compound([(
                "on_consume_effects",
                mod_api::Tag::List(vec![mod_api::Tag::compound([
                    ("type", mod_api::Tag::String("example:charge".into())),
                    ("bad", mod_api::Tag::Double(f64::NAN.to_bits())),
                ])]),
            )]);
            let mut air = air;
            for stack in [&mut air, &mut sword] {
                stack
                    .set_components([("minecraft:consumable".into(), invalid.clone())].into())
                    .unwrap();
            }
            let invalid_view = Gui3dView::new([f32::NAN, 30.0], 16.0, IDENTITY_TRANSFORM);
            let mut turn = IDENTITY_TRANSFORM;
            turn[0] = 2.0;
            let overflowing_view = Gui3dView::new([40.0, 30.0], 3.0e38, turn);
            for view in [view, invalid_view, overflowing_view] {
                assert_eq!(draw.model_3d(Gui3dSubject::Item(&air), view), Ok(()));
            }
            assert_eq!(
                draw.model_3d(Gui3dSubject::Item(&sword), view),
                Err(Gui3dError::InvalidStack)
            );
            assert!(draw.surface.vertices.is_empty());
            assert!(draw.surface.blends.is_empty());

            let unknown = mod_api::ItemStack::new(
                mod_api::__private::parts_installation::item_key(u32::MAX),
                1,
            )
            .unwrap();
            assert_eq!(
                draw.model_3d(Gui3dSubject::Item(&unknown), view),
                Err(Gui3dError::UnknownItem)
            );
            assert!(draw.surface.vertices.is_empty());
            assert!(draw.surface.blends.is_empty());
        });
    }
}
