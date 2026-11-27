//! A few blocks drawn in a screen as a small scene seen from an angle: what
//! Create's Ponder shows its machines in (docs/native-mods.md, "Scenes in
//! screens"). The blocks' models and their mod block entities' drawing are
//! projected into the GUI with their own depth, as the smithing screen draws
//! its armour stand.
use super::native_block_entity_draw::BlockEntityDraw;
use crate::game::ClientGame;
use glam::{Mat4, Vec3};
use mod_api::{BlockEntityDataView, SceneView};
use sunrise_client::{block_entity_render::diffuse_lights, hud::HudBatch};
use sunrise_world::level::data::state_flags;
/// Prepared scene data. Entity fields remain borrowed for the duration of the draw.
pub struct SceneBlock<'a> {
    pub pos: [i32; 3],
    pub state: sunrise_world::registry::BlockStateId,
    pub entity: Option<&'a mod_api::SceneBlockEntity>,
}

/// A quad in the scene's blocks, before it is seen: its corners, UVs in the
/// atlas, ABGR colour, and its outward normal.
pub(in crate::game) struct ProjectedQuad {
    depth: f32,
    pub(in crate::game) corners: [[f32; 3]; 4],
    pub(in crate::game) uv: [[f32; 2]; 4],
    pub(in crate::game) color: u32,
}

pub(in crate::game) struct SceneQuad {
    pub(in crate::game) corners: [Vec3; 4],
    pub(in crate::game) uv: [[f32; 2]; 4],
    pub(in crate::game) color: u32,
    pub(in crate::game) normal: Vec3,
    /// Dropped when it faces away: block model faces, and the quads a block
    /// entity marks for culling as `entity.wgsl` does (`CULL_BACK`,
    /// `SHADED_CULL_W`). Other quads draw both sides, as in the world.
    pub(in crate::game) cull: bool,
}

/// The GUI transform of `view` in the box `rect` (x0, y0, x1, y1): the view's
/// centre at the box's middle, a block `scale` pixels across. Unturned, the
/// scene is seen from the north looking south, as a player at yaw 0 sees it,
/// up the screen's up; `yaw` turns it counterclockwise seen from above, and
/// `pitch` tilts its top towards the viewer. The GUI's y runs down and its z
/// towards the viewer, so the point reflection keeps the picture unmirrored.
pub fn scene_matrix(rect: [f32; 4], view: SceneView) -> Mat4 {
    let middle = Vec3::new((rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0, 0.0);
    Mat4::from_translation(middle)
        * Mat4::from_scale(Vec3::splat(-view.scale))
        * Mat4::from_rotation_x(-view.pitch.to_radians())
        * Mat4::from_rotation_y(view.yaw.to_radians())
        * Mat4::from_translation(-Vec3::from_array(view.center))
}

/// The step a model face's direction points: the model's `Direction` lists
/// its sides in the order the game's does.
fn step(direction: sunrise_assets::model::Direction) -> [i32; 3] {
    sunrise_core::geometry::Direction::ALL[direction as usize].normal()
}

/// How bright a face is with `normal` (world space, unit), as blocks are
/// shaded: 1 for the top, 0.5 underneath, 0.8 north and south, 0.6 east and
/// west, in between for a slanted face.
pub(in crate::game::client_side) fn face_shade(normal: Vec3) -> f32 {
    let up = if normal.y > 0.0 { 1.0 } else { 0.5 };
    (normal.x * normal.x * 0.6 + normal.y * normal.y * up + normal.z * normal.z * 0.8).min(1.0)
}

/// `color` (ABGR) with its red, green and blue times `shade`.
pub(in crate::game::client_side) fn shaded(color: u32, shade: f32) -> u32 {
    let ch = |s: u32| (((color >> s & 255) as f32 * shade) as u32).min(255) << s;
    (color & 0xFF00_0000) | ch(0) | ch(8) | ch(16)
}

/// Build the read-only camera snapshot for a screen scene view (`DrawContext::draw_scene`).
pub(in crate::game::client_side) fn scene_camera(view: SceneView, partial: f32) -> mod_api::Camera {
    mod_api::Camera {
        position: view.center.map(f64::from),
        eye_height: 0.0,
        partial_tick: partial,
        yaw: view.yaw,
        pitch: view.pitch,
        roll: 0.0,
        fov_degrees: 0.0,
        near: 0.0,
        far: 0.0,
        view: sunrise_client::camera::view_matrix(view.yaw, view.pitch).to_cols_array(),
        projection: Mat4::IDENTITY.to_cols_array(),
        dimension: String::new(),
    }
}

impl ClientGame<'_> {
    /// Draws `blocks`, a scene of block states at positions of its own (not
    /// the world's), into the box `rect` (x0, y0, x1, y1, GUI pixels) of a
    /// screen's `batch`, seen as `view` says: Create's Ponder scenes. Each
    /// block is drawn with its model, full bright and shaded by face, a face
    /// against a solid neighbour in the scene left out; a block with an
    /// `entity` (a mod's) is drawn by `native_block_entity_submit` as in the
    /// world too, its `spin` parts turning with the game time. What lies
    /// outside the box is cut off, and the scene has a depth of its own, so
    /// it never covers the screen's other drawing or is covered by it but in
    /// the order drawn. `extra` adds quads in the scene's coordinates before
    /// the scene is seen and sorted (a living scene's entities and particles,
    /// `native_scene_live`); a scene of blocks alone passes none. Nothing is
    /// drawn when a quad seen through the view isn't finite. API 422.
    pub(in crate::game::client_side) fn draw_scene_quads(
        &self,
        batch: &mut HudBatch,
        rect: [f32; 4],
        blocks: &[SceneBlock<'_>],
        view: SceneView,
        clock: Option<(i64, f32, f64)>,
        extra: impl FnOnce(&mut Vec<SceneQuad>, &mod_api::WorldRenderView<'_>),
    ) -> Result<(), mod_api::SceneError> {
        if !self.common.native_window() {
            return Err(mod_api::SceneError::Unavailable);
        }
        if !view.scale.is_finite() || view.scale <= 0.0 {
            return Err(mod_api::SceneError::BadView);
        }
        let (game_time, partial, animation_time) = clock.unwrap_or_else(|| {
            (
                self.common.world.game_time,
                self.common.clock.fraction() as f32,
                self.animation_time(),
            )
        });
        let camera = scene_camera(view, partial);
        let render_view = mod_api::WorldRenderView::new(
            &camera,
            None,
            partial,
            game_time,
            animation_time,
            false,
            &[],
        );
        // The Game boundary prepared every state before any drawing began.
        let occupied: std::collections::HashMap<[i32; 3], _> =
            blocks.iter().map(|b| (b.pos, b.state)).collect();
        let solid = |pos: [i32; 3]| {
            occupied.get(&pos).is_some_and(|&state| {
                self.common.res.behaviours().facts(state).flags & state_flags::SOLID_RENDER != 0
            })
        };
        let grass = self.client.client_pack.item_renderer.grass_color(0.5, 1.0) as u32;
        let tables = &self.client.client_pack.context.mesh.tables;
        let mut quads = Vec::new();
        for block in blocks {
            let at = Vec3::new(
                block.pos[0] as f32,
                block.pos[1] as f32,
                block.pos[2] as f32,
            );
            let Ok(models) = self
                .client
                .client_pack
                .render
                .model_catalog
                .collect_seeded(block.state, 42)
            else {
                continue;
            };
            for face in models.iter().flat_map(|(_, model)| &model.faces) {
                if let Some(cull) = face.quad.cull {
                    let [dx, dy, dz] = step(cull);
                    if solid([block.pos[0] + dx, block.pos[1] + dy, block.pos[2] + dz]) {
                        continue;
                    }
                }
                let Some(sprite) = self
                    .client
                    .client_pack
                    .layer_names
                    .get(&face.layer)
                    .and_then(|name| self.client.gui.atlas.sprites.get(name))
                else {
                    continue;
                };
                let [nx, ny, nz] = step(face.quad.facing);
                let normal = Vec3::new(nx as f32, ny as f32, nz as f32);
                let mut color = shaded(0xFFFF_FFFF, face_shade(normal));
                if let Some(t) = tables.default_tint(block.state, face.quad.tint_index, grass) {
                    color = sunrise_client::hud_layout::multiply(
                        color,
                        sunrise_client::mobs::render::abgr(t),
                    );
                }
                quads.push(SceneQuad {
                    corners: face.quad.positions.map(|p| at + Vec3::from_array(p)),
                    uv: face.quad.uv.map(|uv| {
                        [
                            sprite.u0 + uv[0] * (sprite.u1 - sprite.u0),
                            sprite.v0 + uv[1] * (sprite.v1 - sprite.v0),
                        ]
                    }),
                    color,
                    normal,
                    cull: true,
                });
            }
            if let Some(entity) = block.entity {
                let entity = BlockEntityDataView::new(entity);
                let name = &*self.common.res.facts().get(block.state).name;
                let mut draw = BlockEntityDraw::posed_with_view(
                    render_view,
                    240 | 240 << 8,
                    block.pos,
                    Mat4::IDENTITY,
                    diffuse_lights(false),
                    Vec::new(),
                )
                .with_quad_budget(self.entity_quad_capacity(), 0);
                self.native_block_entity_submit(block.pos, name, block.state, entity, &mut draw);
                // Its text's new glyphs upload with this screen's batch.
                batch
                    .patches
                    .append(&mut self.client.text_patches.borrow_mut());
                let placed = draw.into_placed(at);
                for q in placed.as_chunks::<4>().0 {
                    let corners = q.each_ref().map(|v| Vec3::from_array(v.pos));
                    let [nx, ny, nz, w] = q[0].normal;
                    let mut normal = (corners[1] - corners[0])
                        .cross(corners[2] - corners[0])
                        .normalize_or_zero();
                    if normal == Vec3::ZERO {
                        normal = Vec3::new(f32::from(nx), f32::from(ny), f32::from(nz))
                            .normalize_or_zero();
                    }
                    // `entity.wgsl`: |w| >= 0.5 is shaded by the normal; smaller
                    // keeps the colour baked in (quads, cubes, items, text).
                    let unbaked = w.unsigned_abs() >= 64;
                    quads.push(SceneQuad {
                        corners,
                        uv: q.each_ref().map(|v| v.uv),
                        color: if unbaked {
                            shaded(q[0].color, face_shade(normal))
                        } else {
                            q[0].color
                        },
                        normal,
                        cull: sunrise_client_render::mesher::culls_back_faces(w),
                    });
                }
            }
        }
        extra(&mut quads, &render_view);
        let seen =
            project_quads(quads, scene_matrix(rect, view)).ok_or(mod_api::SceneError::BadView)?;
        let start = batch.vertices.len();
        for face in seen {
            batch.quad_3d(face.corners, face.uv, face.color);
        }
        batch.depth_since(start, rect);
        Ok(())
    }
}

/// `quads` seen through `m`: the faces turned away dropped (those that cull), the rest far to
/// near, for the faces that let others show through. None when a quad does not stay finite:
/// a mod's view or position may overflow, and nothing may be drawn then.
pub(in crate::game::client_side) fn project_quads(
    quads: Vec<SceneQuad>,
    m: Mat4,
) -> Option<Vec<ProjectedQuad>> {
    let mut seen: Vec<ProjectedQuad> = quads
        .into_iter()
        .filter(|q| !q.cull || m.transform_vector3(q.normal).z > 0.0)
        .map(|q| {
            let p = q.corners.map(|c| m.transform_point3(c));
            let depth = p.iter().map(|v| v.z).sum::<f32>() / 4.0;
            ProjectedQuad {
                depth,
                corners: p.map(|v| v.to_array()),
                uv: q.uv,
                color: q.color,
            }
        })
        .collect();
    let finite = |q: &ProjectedQuad| {
        q.depth.is_finite() && q.corners.iter().flatten().all(|v| v.is_finite())
    };
    if !seen.iter().all(finite) {
        return None;
    }
    seen.sort_by(|a, b| a.depth.total_cmp(&b.depth));
    Some(seen)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(yaw: f32, pitch: f32) -> SceneView {
        SceneView::new([0.5, 0.5, 0.5], yaw, pitch, 20.0)
    }

    /// Unturned, the scene is seen from the north as a player at yaw 0 sees
    /// it: the centre in the box's middle, up up the screen, east to the
    /// viewer's left, south away and its north face towards the viewer.
    #[test]
    fn an_unturned_scene_is_seen_from_the_north() {
        let m = scene_matrix([0.0, 0.0, 100.0, 60.0], view(0.0, 0.0));
        let at = |p: [f32; 3]| m.transform_point3(Vec3::from_array(p));
        assert_eq!(at([0.5, 0.5, 0.5]), Vec3::new(50.0, 30.0, 0.0));
        assert!(at([0.5, 1.5, 0.5]).y < 30.0, "up is up the screen");
        assert!(at([1.5, 0.5, 0.5]).x < 50.0, "east is on the left");
        assert!(at([0.5, 0.5, 1.5]).z < 0.0, "south is away");
        assert!(
            m.transform_vector3(Vec3::NEG_Z).z > 0.0,
            "the north face is seen"
        );
        assert!(
            m.transform_vector3(Vec3::Y).z.abs() < 1e-5,
            "the top is edge on"
        );
    }

    /// A tilt shows the top and hides the bottom; a quarter turn brings the
    /// east face round to the viewer (counterclockwise seen from above: east
    /// goes to the north, the side the viewer is on).
    #[test]
    fn pitch_shows_the_top_and_yaw_turns_the_scene() {
        let tilted = scene_matrix([0.0, 0.0, 100.0, 100.0], view(0.0, 30.0));
        assert!(tilted.transform_vector3(Vec3::Y).z > 0.0);
        assert!(tilted.transform_vector3(Vec3::NEG_Y).z < 0.0);
        let turned = scene_matrix([0.0, 0.0, 100.0, 100.0], view(90.0, 0.0));
        assert!(turned.transform_vector3(Vec3::X).z > 0.99);
    }

    fn quad(z: f32, color: u32, normal: Vec3, cull: bool) -> SceneQuad {
        SceneQuad {
            corners: [Vec3::new(0.0, 0.0, z); 4],
            uv: [[0.0; 2]; 4],
            color,
            normal,
            cull,
        }
    }

    /// Faces turned away are dropped when they cull, the rest come far to near, and quads at one
    /// depth keep the order they came in.
    #[test]
    fn project_quads_drops_back_faces_and_sorts_far_to_near_stably() {
        let towards = Vec3::Z; // faces the viewer under the identity
        let away = Vec3::NEG_Z;
        let quads = vec![
            quad(5.0, 1, towards, true),
            quad(9.0, 2, towards, true),
            quad(5.0, 3, towards, true),
            quad(1.0, 4, away, true),
            quad(2.0, 5, away, false),
        ];
        let seen = project_quads(quads, Mat4::IDENTITY).unwrap();
        // Depth is z: the smaller z is farther. Culled quad 4 is gone; 5 does not cull.
        let colors: Vec<_> = seen.iter().map(|q| q.color).collect();
        assert_eq!(colors, [5, 1, 3, 2]);
    }

    #[test]
    fn project_quads_refuses_a_quad_that_does_not_stay_finite() {
        let mut turned = Mat4::IDENTITY;
        turned.x_axis.x = f32::MAX;
        let quads = vec![quad(0.0, 1, Vec3::Z, false), {
            let mut far = quad(0.0, 2, Vec3::Z, false);
            far.corners[0].x = f32::MAX;
            far
        }];
        assert!(project_quads(quads, turned).is_none());
    }

    #[test]
    fn faces_are_shaded_as_blocks_are() {
        assert_eq!(face_shade(Vec3::Y), 1.0);
        assert_eq!(face_shade(Vec3::NEG_Y), 0.5);
        assert!((face_shade(Vec3::Z) - 0.8).abs() < 1e-6);
        assert!((face_shade(Vec3::X) - 0.6).abs() < 1e-6);
        assert_eq!(shaded(0xFF80_4020, 0.5), 0xFF40_2010);
    }
}
