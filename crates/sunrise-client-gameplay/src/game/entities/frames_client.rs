use crate::game::ClientGame;
use sunrise_assets::{bake::bake_model, item_model::OwnerState, resolve::ModelLibrary};
use sunrise_client::mesher::Vertex;
use sunrise_gameplay_common::game::{frames_common::center, maps::map_id};
use sunrise_items::items::ItemStack;
use sunrise_sim::entities::ItemFrame;

/// A baked frame model: (corners in block units, unit UVs, sprite) per quad.
pub(in crate::game) type FrameModel = Vec<([[f32; 3]; 4], [[f32; 2]; 4], String)>;

/// Item-frame variants prepared together for one resource generation.
pub(in crate::game) fn bake_frames(library: Option<&ModelLibrary>) -> [FrameModel; 4] {
    [
        "item_frame",
        "glow_item_frame",
        "item_frame_map",
        "glow_item_frame_map",
    ]
    .map(|name| library.map_or_else(Vec::new, |library| bake_frame(library, name)))
}

/// Bake `block/<name>` (item_frame / glow_item_frame) from the resources.
fn bake_frame(library: &ModelLibrary, name: &str) -> FrameModel {
    let Ok(resolved) = library.resolve(&format!("minecraft:block/{name}")) else {
        return Vec::new();
    };
    let Ok(quads) = bake_model(&resolved) else {
        return Vec::new();
    };
    let unit = if quads
        .iter()
        .flat_map(|q| q.positions)
        .any(|p| p.iter().any(|v| *v > 1.5))
    {
        16.0
    } else {
        1.0
    };
    quads
        .into_iter()
        .map(|q| {
            (
                q.positions.map(|p| p.map(|v| v / unit)),
                q.uv,
                q.material
                    .sprite
                    .trim_start_matches("minecraft:")
                    .to_owned(),
            )
        })
        .collect()
}
/// A frame's item, waiting for its model: the frames' borrow of `self` ends
/// before `resolve_item` reads the pack.
pub(in crate::game) struct FramedItem {
    pub(in crate::game) stack: ItemStack,
    pub(in crate::game) id: u32,
    pub(in crate::game) matrix: glam::Mat4,
    pub(in crate::game) light: u8,
    pub(in crate::game) cull: bool,
    pub(in crate::game) kind: &'static str,
    pub(in crate::game) owner: OwnerState,
}
/// `ItemFrameRenderer.extractRenderState`'s `ItemOwner` (`updateForNonLiving`
/// with the frame entity): `ItemFrame.position` and
/// `ItemFrame.getVisualRotationYInDegrees`.
pub(in crate::game) fn frame_owner(f: &ItemFrame) -> OwnerState {
    super::item_draw::owner_at(center(f), visual_rotation_y_degrees(f), f.kind())
}
/// `ItemFrame.getVisualRotationYInDegrees`: the frame's own yaw, as an item
/// model's owner reads it (a framed compass's needle turns with it).
fn visual_rotation_y_degrees(f: &ItemFrame) -> f32 {
    // `Direction.get2DDataValue` of a wall frame, and the axis direction's
    // step for the two flat ones.
    let s = f.step();
    let (two_d, correction) = match (s[0], s[1], s[2]) {
        (0, 0, 1) => (0, 0),
        (-1, 0, 0) => (1, 0),
        (0, 0, -1) => (2, 0),
        (1, 0, 0) => (3, 0),
        (0, y, _) => (-1, 90 * y),
        _ => (0, 0),
    };
    sunrise_level::camera_math::wrap_degrees(
        180.0 + (two_d * 90) as f32 + f32::from(f.rotation) * 45.0 + correction as f32,
    )
}

impl ClientGame<'_> {
    /// ItemFrameRenderer: the frame model, then the item half size on its face.
    pub fn frame_geometry(&mut self, camera: glam::DVec3, alpha: f64, out: &mut Vec<Vertex>) {
        let sprites = &self.client.gui.atlas.sprites;
        // Each with its frame's type: a pack tells a glow frame and what it holds apart by its
        // `entityId` (`glow_item_frame` in entity.properties), the item drawn within its frame.
        let mut maps: Vec<(i32, glam::Mat4, u32, bool, &'static str)> = Vec::new();
        let mut items: Vec<FramedItem> = Vec::new();
        // ItemFrame.shouldRenderAtSqrDistance: 1024 blocks at a view scale
        // of 1.
        let range = 16.0 * 64.0 * self.view_scale();
        let partial = alpha as f32;
        for f in &self.common.item_frames {
            // LevelExtractor: the camera entity is not drawn from its own eyes.
            if self.hides_as_camera(f.id) {
                continue;
            }
            let entity_start = out.len();
            self.mark_entity(entity_start, f.kind());
            let map = f.item.as_ref().and_then(map_id);
            // EntityRenderer.shouldRender: its box (a block wide with a map),
            // measured from the box's centre.
            let (lo, hi) = f.bounds();
            let (mut lo, mut hi) = (glam::DVec3::from(lo), glam::DVec3::from(hi));
            if map.is_some() {
                let s = f.step();
                for i in 0..3 {
                    if s[i] == 0 {
                        let c = (lo[i] + hi[i]) / 2.0;
                        (lo[i], hi[i]) = (c - 0.5, c + 0.5);
                    }
                }
            }
            if !self.entity_in_view(camera, (lo + hi) / 2.0, range, lo, hi) {
                continue;
            }
            let culling = self.entity_cull_decision(f, 1.0);
            if culling.hides_entity() {
                continue;
            }
            let entity_start = out.len();
            self.mark_motion(
                entity_start,
                sunrise_gameplay_common::game::MOTION_OBJECT | u64::from(f.id),
            );
            if let Some(resolved) = super::native_entity_renderers::item_frame_renderer(f.glow) {
                let step = f.step().map(f64::from);
                let at = glam::DVec3::new(
                    f64::from(f.pos[0]) + 0.5 - step[0] * 0.46875,
                    f64::from(f.pos[1]) + 0.5 - step[1] * 0.46875,
                    f64::from(f.pos[2]) + 0.5 - step[2] * 0.46875,
                );
                let (yaw, pitch) = match f.facing {
                    0 => (0.0, 90.0),
                    1 => (0.0, -90.0),
                    2 => (180.0, 0.0),
                    4 => (90.0, 0.0),
                    5 => (270.0, 0.0),
                    _ => (0.0, 0.0),
                };
                let facing = mod_api::Direction::ALL[usize::from(f.facing.min(5))];
                let entity: mod_api::EntityRenderState =
                    mod_api::__private::built::EntityRenderState {
                        id: f.id,
                        partial,
                        age: f.ticks as f32 + partial,
                        yaw,
                        pitch,
                        position: at.to_array(),
                        // EntityTypes.ITEM_FRAME / GLOW_ITEM_FRAME: sized(0.5F, 0.5F).eyeHeight(0.0F).
                        size: [0.5, 0.5],
                        eye_height: 0.0,
                        distance_to_camera_sq: (at - camera).length_squared(),
                        living: None,
                        tnt_fuse: None,
                        orb_value: None,
                        painting: None,
                        item_frame: Some(mod_api::__private::built::ItemFrameRenderState {
                            facing,
                            glow: f.glow,
                            invisible: f.invisible,
                            rotation: f.rotation,
                            has_item: f.item.is_some(),
                            has_map: map.is_some(),
                        }),
                        attachments: None,
                    }
                    .into();
                let light = self.common.world.light(f.pos[0], f.pos[1], f.pos[2]);
                let frame_light = if f.glow {
                    (light & 0xF0) | (light & 15).max(5)
                } else {
                    light
                };
                let packed =
                    (u32::from(frame_light >> 4) * 16) | ((u32::from(frame_light & 15) * 16) << 8);
                if self.mod_entity_renderer_geometry(
                    &resolved,
                    &entity,
                    f.kind(),
                    at - camera,
                    Some(at),
                    f.pos,
                    packed,
                    out,
                ) {
                    self.finish_entity_occlusion(culling, f, out, entity_start);
                    continue;
                }
            }
            let s = f.step();
            // Pose: to the block centre, then X/Y rotation by facing.
            let (x_rot, y_rot) = if s[1] == 0 {
                let to_y = match f.facing {
                    3 => 0.0,
                    4 => 90.0,
                    2 => 180.0,
                    _ => 270.0,
                };
                (0.0f64, 180.0f64 - to_y)
            } else {
                (-90.0 * f64::from(s[1]), 180.0)
            };
            let pose = glam::DMat4::from_translation(
                glam::DVec3::new(
                    f64::from(f.pos[0]) + 0.5,
                    f64::from(f.pos[1]) + 0.5,
                    f64::from(f.pos[2]) + 0.5,
                ) - camera,
            ) * glam::DMat4::from_rotation_x(x_rot.to_radians())
                * glam::DMat4::from_rotation_y(y_rot.to_radians());
            let light = self.common.world.light(f.pos[0], f.pos[1], f.pos[2]);
            let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            // ItemFrameRenderer.getBlockLightLevel: a glow frame's model is at least 5 bright.
            let frame_light = if f.glow {
                (light & 0xF0) | (light & 15).max(5)
            } else {
                light
            };
            let frame_packed =
                (u32::from(frame_light >> 4) * 16) | ((u32::from(frame_light & 15) * 16) << 8);
            let frame_start = out.len();
            let mut quad =
                |m: &glam::DMat4, corners: [[f64; 3]; 4], uv: [[f32; 2]; 4], light: u32| {
                    // The frame model is drawn with the entity shader, lit by its face normal.
                    let at = corners.map(|c| m.transform_point3(glam::DVec3::from_array(c)));
                    let n = (at[1] - at[0]).cross(at[2] - at[0]).normalize_or_zero();
                    let normal = [0, 1, 2].map(|k| (n[k].clamp(-1.0, 1.0) * 127.0) as i8);
                    for (p, uv) in at.into_iter().zip(uv) {
                        out.push(Vertex {
                            normal: [normal[0], normal[1], normal[2], -127],
                            pos: p.as_vec3().to_array(),
                            uv,
                            layer: 0,
                            color: 0xFFFF_FFFF,
                            light,

                            glint_uv: [0.0, 0.0],
                        });
                    }
                };
            let map_uv = |r: &sunrise_client::hud::Rect, uv: [[f32; 2]; 4]| {
                uv.map(|[u, v]| [r.u0 + u * (r.u1 - r.u0), r.v0 + v * (r.v1 - r.v0)])
            };
            if !f.invisible {
                let model = match (f.glow, map.is_some()) {
                    (true, true) => &self.client.glow_map_frame_model,
                    (false, true) => &self.client.map_frame_model,
                    (true, false) => &self.client.glow_frame_model,
                    (false, false) => &self.client.frame_model,
                };
                for (corners, uv, sprite) in model {
                    let Some(r) = sprites.get(sprite) else {
                        continue;
                    };
                    quad(
                        &pose,
                        corners.map(|c| {
                            [
                                f64::from(c[0]) - 0.5,
                                f64::from(c[1]) - 0.5,
                                f64::from(c[2]) - 0.5,
                            ]
                        }),
                        map_uv(r, *uv),
                        frame_packed,
                    );
                }
            }
            // HDR output: a glow frame shines softly (entity.wgsl fs_hdr_mark); the item in it
            // is only lit, as vanilla draws it.
            if f.glow {
                for v in &mut out[frame_start..] {
                    v.layer |= u32::from(sunrise_client::mesher::INCANDESCENT) << 16;
                }
            }
            self.finish_entity_occlusion(culling, f, out, entity_start);
            let cull_contents = culling.checks_vertices();
            if let Some(stack) = &f.item {
                let mut canceled = false;
                self.native_render_item_in_frame(f.id, f.pos, stack, &mut canceled);
                if canceled {
                    continue;
                }
            }
            if let Some(id) = map {
                // ItemFrameRenderer: the map fills the face, in 90-degree steps.
                let depth = if f.invisible { 0.5 } else { 0.4375 };
                // A glow frame's map is 15728850 = 0xF000D2 (sky coordinate 240,
                // block coordinate 210), not full bright.
                let item_light = if f.glow { 240 | 210 << 8 } else { packed };
                let m = pose
                    * glam::DMat4::from_translation(glam::DVec3::new(0.0, 0.0, depth))
                    * glam::DMat4::from_rotation_z(
                        (f64::from(f.rotation % 4 * 2) * 45.0).to_radians(),
                    )
                    * glam::DMat4::from_rotation_z(std::f64::consts::PI)
                    * glam::DMat4::from_scale(glam::DVec3::splat(0.0078125))
                    * glam::DMat4::from_translation(glam::DVec3::new(-64.0, -64.0, -1.0));
                maps.push((id, m.as_mat4(), item_light, cull_contents, f.kind()));
                continue;
            }
            let Some(stack) = f.item.clone() else {
                continue;
            };
            // `GlowItemFrame` items render full bright (15728880).
            let item_light = if f.glow { 0xFF } else { light };
            let depth = if f.invisible { 0.5 } else { 0.4375 };
            let m = pose
                * glam::DMat4::from_translation(glam::DVec3::new(0.0, 0.0, depth))
                * glam::DMat4::from_rotation_z((f64::from(f.rotation) * 45.0).to_radians())
                * glam::DMat4::from_scale(glam::DVec3::splat(0.5));
            items.push(FramedItem {
                stack,
                id: f.id,
                matrix: m.as_mat4(),
                light: item_light,
                cull: cull_contents,
                kind: f.kind(),
                // `ItemFrameRenderer.extractRenderState` hands the frame entity
                // itself to the item model (`updateForNonLiving`), so a framed
                // compass's needle reads the frame's position and yaw.
                owner: frame_owner(f),
            });
        }
        // `extractRenderState`: FIXED, seeded by the frame entity.
        let skinning = self.skinning();
        let lights = sunrise_client::block_entity_render::diffuse_lights(self.nether_lighting());
        for item in items {
            let entity_start = out.len();
            self.mark_entity(entity_start, item.kind);
            self.mark_motion(entity_start, Self::MOTION_FRAME_ITEM | u64::from(item.id));
            let state = self.resolve_item(
                &item.stack,
                sunrise_assets::item_model::DisplayContext::Fixed,
                Some(&item.owner),
                item.id as i32,
            );
            let light = sunrise_client::block_entity_render::vertex_light(item.light);
            // Posed on the GPU when every layer can be.
            let skinned = skinning
                .then(|| self.submit_item_skinned(&state, &[item.matrix], light, lights))
                .flatten();
            if skinned.is_none() {
                self.submit_item_world(&state, item.matrix, light, out);
            }
            if item.cull {
                self.cull_skinned(out, entity_start, skinned);
            }
        }
        // Maps: keyed by their map id, which several frames can show, so they stay unmarked.
        self.mark_motion(out.len(), 0);
        // Nearest first: past the atlas's map slots, the far ones go without.
        maps.sort_by(|a, b| {
            let d = |m: &glam::Mat4| {
                m.transform_point3(glam::Vec3::new(64.0, 64.0, 0.0))
                    .length_squared()
            };
            d(&a.1).total_cmp(&d(&b.1))
        });
        for (id, m, light, cull, kind) in maps {
            let entity_start = out.len();
            self.mark_entity(entity_start, kind);
            // Its outline first: a map out of sight takes no slot in the atlas, or with
            // more maps around than slots, the ones in view lost theirs to it.
            if cull {
                for c in [[0.0, 0.0], [128.0, 0.0], [128.0, 128.0], [0.0, 128.0]] {
                    out.push(Vertex {
                        pos: m
                            .transform_point3(glam::Vec3::new(c[0], c[1], 0.0))
                            .to_array(),
                        ..bytemuck::Zeroable::zeroed()
                    });
                }
                self.cull_entity_vertices(out, entity_start);
                if out.len() == entity_start {
                    continue;
                }
                out.truncate(entity_start);
            }
            self.map_vertices(id, true, m, light, out);
            if cull {
                self.cull_entity_vertices(out, entity_start);
            }
        }
    }
}
