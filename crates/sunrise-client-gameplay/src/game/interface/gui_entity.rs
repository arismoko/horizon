//! A living entity drawn in a mod's screen (`DrawContext::entity`): vanilla's
//! `InventoryScreen.renderEntityInInventoryFollowsMouse` and
//! `renderEntityInInventory`. The entity is drawn by the same route the world
//! draws it with (a mod's renderer for its type, else its model, then the
//! mods' layers), then seen as the inventory's player is (`draw_player_preview`):
//! the box's centre, `size` pixels a block, turned, depth-sorted, back faces
//! culled only for render types that cull them, and lit by `Lighting.ENTITY_IN_UI`.
//!
//! Nothing here runs unless a mod calls `DrawContext::entity`. The buffers and
//! the entities made for `GuiEntity::Kind` are kept per thread and reused, so a
//! screen drawing the same entity every frame makes it once and allocates
//! nothing once the buffers have grown.
use super::stock_layer_mask::StockLayerMask;
use crate::game::ClientGame;
use crate::game::client_side::dragon_client::{DragonPreview, GuiDragonLook};
use glam::{DVec3, Mat4, Vec3};
use mod_api::{GuiEntity, GuiEntityError, GuiEntityLook, GuiEntityView};
use std::{cell::RefCell, sync::Arc};
use sunrise_client::{hud::HudBatch, mesher::Vertex, mobs::render::FrameDraw};
use sunrise_core::{
    geometry::Vec3 as Position,
    nbt::{Compound, JavaString, Tag},
};
use sunrise_gameplay_common::game::commands_common::Target;
use sunrise_sim::mobs::Mob;

/// How many entities made for `GuiEntity::Kind` are kept: a mob page shows a
/// few at once, and a learner's four models.
const KEPT: usize = 8;

/// The dragon lives outside the mob list and draws through `EnderDragonRenderer`.
const DRAGON: &str = "minecraft:ender_dragon";
/// `EntityTypes.ENDER_DRAGON`'s height, by which the box centres it.
const DRAGON_HEIGHT: f32 = 8.0;

/// An entity made for the screen from a type and its data.
struct Made {
    kind: String,
    data: Compound,
    mob: Mob,
}

/// A face of the entity as seen: its depth, corners, UVs, ABGR colour and emission ordinal.
type SeenQuad = (f32, [[f32; 3]; 4], [[f32; 2]; 4], u32, usize);

#[derive(Default)]
struct Scratch {
    made: Vec<Made>,
    verts: Vec<Vertex>,
    eyes: Vec<Vertex>,
    quads: Vec<SeenQuad>,
    eye_quads: Vec<SeenQuad>,
}

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::default();
}

#[cfg(any(test, feature = "test-support"))]
pub fn test_cached_entity_count() -> usize {
    SCRATCH.with_borrow(|scratch| scratch.made.len())
}

/// Full bright, as `renderEntityInInventory` draws (`LightTexture.FULL_BRIGHT`).
const FULL_BRIGHT: u32 = 240 | 240 << 8;

/// The turn `look` gives an entity standing in the box centred at `centre`:
/// its body yaw, head yaw and head pitch (degrees, as the mob keeps them), and
/// the pitch the whole entity is tilted by (`cameraOrientation`).
fn turn(look: GuiEntityLook, centre: [f32; 2]) -> (f32, f32, f32, f32) {
    match look {
        GuiEntityLook::At([x, y]) => {
            let x_angle = ((centre[0] - x) / 40.0).atan();
            let y_angle = ((centre[1] - y) / 40.0).atan();
            let body = 180.0 + x_angle * 20.0;
            (body, body + x_angle * 20.0, -y_angle * 20.0, y_angle * 20.0)
        }
        GuiEntityLook::Turned { yaw, pitch } => (180.0 + yaw, 180.0 + yaw, pitch, 0.0),
        // A look a later API adds is drawn facing the viewer until the game knows it.
        _ => (180.0, 180.0, 0.0, 0.0),
    }
}

/// `mob` posed still for the screen with the turn `turn` gives it.
fn pose(mob: &mut Mob, (body, head, pitch, _): (f32, f32, f32, f32)) {
    mob.previous = mob.position;
    mob.body_yaw = body;
    mob.previous_body_yaw = body;
    mob.head_yaw = head;
    mob.previous_head_yaw = head;
    mob.head_pitch = pitch;
    mob.previous_head_pitch = pitch;
    // `extractEntityInInventory` divides the size by the entity's scale.
    mob.extra.model_scale = 0.0;
}

/// `EnderDragonRenderer` turns by `-yRot` where a living renderer turns by `180 - yBodyRot`.
fn dragon_look((body, head, pitch, _): (f32, f32, f32, f32)) -> GuiDragonLook {
    GuiDragonLook {
        body_yaw: body - 180.0,
        head_yaw: head - body,
        head_pitch: pitch,
    }
}

fn dragon_preview_from_data(data: &Compound) -> Result<DragonPreview, GuiEntityError> {
    let named = JavaString::from(DRAGON);
    if data
        .get(&JavaString::from("id"))
        .is_some_and(|id| !matches!(id, Tag::String(s) if *s == named))
    {
        return Err(GuiEntityError::BadData);
    }
    Ok(DragonPreview::from_nbt(data))
}

impl ClientGame<'_> {
    /// `DrawContext::entity` on a screen's `batch`.
    pub fn native_draw_entity(
        &self,
        batch: &mut HudBatch,
        entity: &GuiEntity,
        view: GuiEntityView,
    ) -> Result<(), GuiEntityError> {
        // A client's call: a logical server (the integrated server thread too) has no screen.
        if self.common.is_logical_server() {
            return Err(GuiEntityError::Unavailable);
        }
        if !view.is_drawable() {
            return Err(GuiEntityError::BadView);
        }
        let mut scratch = SCRATCH.with_borrow_mut(std::mem::take);
        let result = self.draw_entity_with(batch, entity, view, &mut scratch);
        SCRATCH.with_borrow_mut(|kept| *kept = scratch);
        result
    }

    fn draw_entity_with(
        &self,
        batch: &mut HudBatch,
        entity: &GuiEntity,
        view: GuiEntityView,
        scratch: &mut Scratch,
    ) -> Result<(), GuiEntityError> {
        let rect = view.rect;
        let centre = [(rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0];
        // Finite view values can still overflow once combined: refused before any vertex.
        if !centre.iter().all(|v| v.is_finite()) {
            return Err(GuiEntityError::BadView);
        }
        let turned = turn(view.look, centre);
        scratch.verts.clear();
        scratch.eyes.clear();
        let height = match entity {
            GuiEntity::Kind { kind, data } if kind.as_str() == DRAGON => {
                let preview = dragon_preview_from_data(data)?;
                self.gui_dragon_geometry(
                    preview,
                    dragon_look(turned),
                    &mut scratch.verts,
                    &mut scratch.eyes,
                );
                DRAGON_HEIGHT
            }
            GuiEntity::Kind { kind, data } => {
                let at = self.made_entity(&mut scratch.made, kind, data)?;
                let mob = &mut scratch.made[at].mob;
                pose(mob, turned);
                self.gui_entity_geometry(mob, &mut scratch.verts);
                mob.height()
            }
            GuiEntity::Existing(handle) => match self.native_entity_read_target(*handle) {
                Some(Target::Dragon(id)) => {
                    let dragon = self
                        .common
                        .dragon
                        .as_ref()
                        .filter(|d| d.id == id)
                        .ok_or(GuiEntityError::NoEntity)?;
                    self.gui_dragon_geometry(
                        DragonPreview::from_dragon(dragon),
                        dragon_look(turned),
                        &mut scratch.verts,
                        &mut scratch.eyes,
                    );
                    DRAGON_HEIGHT
                }
                Some(target) => {
                    let mut existing = self.existing_gui_mob(target)?;
                    pose(&mut existing, turned);
                    self.gui_entity_geometry(&existing, &mut scratch.verts);
                    existing.height()
                }
                None => return Err(GuiEntityError::NoEntity),
            },
            // An entity a later API adds is one this game can't name.
            _ => return Err(GuiEntityError::UnknownType),
        };
        // PictureInPictureRenderer: the box centre, (size, size, -size); then
        // the translation and Rz(pi) * Rx(the look's pitch).
        let gui = Mat4::from_translation(Vec3::new(centre[0], centre[1], 0.0))
            * Mat4::from_scale(Vec3::new(view.size, view.size, -view.size))
            * Mat4::from_translation(Vec3::new(0.0, height / 2.0 + view.offset_y, 0.0))
            * Mat4::from_rotation_z(std::f32::consts::PI)
            * Mat4::from_rotation_x(turned.3.to_radians());
        if !gui.is_finite() {
            return Err(GuiEntityError::BadView);
        }
        seen_quads::<true>(&scratch.verts, gui, &mut scratch.quads);
        seen_quads::<false>(&scratch.eyes, gui, &mut scratch.eye_quads);
        let placed = |q: &SeenQuad| q.0.is_finite() && q.1.iter().flatten().all(|v| v.is_finite());
        if !scratch.quads.iter().all(placed) || !scratch.eye_quads.iter().all(placed) {
            return Err(GuiEntityError::BadView);
        }
        let start = batch.vertices.len();
        for &(_, p, uv, color, _) in &scratch.quads {
            batch.quad_3d(p, uv, color);
        }
        let additive = batch.vertices.len();
        for &(_, p, uv, color, _) in &scratch.eye_quads {
            batch.quad_3d(p, uv, color);
        }
        batch.depth_glint_since(start, additive, rect);
        Ok(())
    }

    /// The index in `made` of the entity of `kind` loaded from `data`, made
    /// now when it isn't kept (or its type was replaced by a resource reload).
    fn made_entity(
        &self,
        made: &mut Vec<Made>,
        kind: &str,
        data: &Compound,
    ) -> Result<usize, GuiEntityError> {
        let types = self.common.res.mob_types();
        let ty = types.get(kind).ok_or(GuiEntityError::UnknownType)?;
        let same = |m: &Made| m.kind == kind && m.data == *data && Arc::ptr_eq(&m.mob.kind, ty);
        if let Some(at) = made.iter().position(same) {
            return Ok(at);
        }
        let mob = self.new_gui_entity(kind, data)?;
        if made.len() == KEPT {
            made.remove(0);
        }
        made.push(Made {
            kind: kind.into(),
            data: data.clone(),
            mob,
        });
        Ok(made.len() - 1)
    }

    /// `entity` prepared for drawing, as `DrawContext::entity` prepares it (a screen scene's
    /// entities too): a new entity of a type made for drawing only, or a copy of the
    /// client's entity as it is this frame. Refused by the same named causes.
    pub(in crate::game::client_side) fn prepare_gui_entity(
        &self,
        entity: &GuiEntity,
    ) -> Result<Mob, GuiEntityError> {
        match entity {
            GuiEntity::Kind { kind, data } => self.new_gui_entity(kind, data),
            GuiEntity::Existing(handle) => self.existing_gui_entity(*handle),
            // An entity a later API adds is one this game can't name.
            _ => Err(GuiEntityError::UnknownType),
        }
    }

    /// One copy of the client's entity `handle`, as vanilla's `createRenderState` makes one a
    /// frame.
    fn existing_gui_entity(&self, handle: mod_api::EntityRef) -> Result<Mob, GuiEntityError> {
        let target = self
            .native_entity_read_target(handle)
            .ok_or(GuiEntityError::NoEntity)?;
        self.existing_gui_mob(target)
    }

    #[inline(always)]
    fn existing_gui_mob(&self, target: Target) -> Result<Mob, GuiEntityError> {
        let found = match target {
            Target::Mob(id) => self.common.mobs.iter().find(|m| m.id == id && !m.removed),
            Target::Player => self.client.player_model.as_ref(),
            _ => None,
        };
        Ok(found.ok_or(GuiEntityError::NoEntity)?.clone())
    }

    /// A new entity of living type `kind` loaded from `data`, made for drawing only.
    fn new_gui_entity(&self, kind: &str, data: &Compound) -> Result<Mob, GuiEntityError> {
        let types = self.common.res.mob_types();
        let ty = types.get(kind).ok_or(GuiEntityError::UnknownType)?;
        Ok(if data.is_empty() {
            // Built for drawing only, as a client's copy of a server's mob: no mod part's
            // `register_goals` or behaviour registration runs for a screen's entity.
            Mob::new_remote(u32::MAX, ty.clone(), Position::ZERO, 0.0)
        } else {
            let named = JavaString::from(kind);
            if data
                .get(&JavaString::from("id"))
                .is_some_and(|id| !matches!(id, Tag::String(s) if *s == named))
            {
                return Err(GuiEntityError::BadData);
            }
            let mut tag = data.clone();
            tag.insert("id".into(), Tag::String(named));
            tag.remove(&JavaString::from("Pos"));
            sunrise_sim::entity_nbt::mob_from_nbt_remote(
                &Tag::Compound(tag),
                types,
                u32::MAX,
                Some(self.common.pack.assets.registry),
            )
            .filter(|m| Arc::ptr_eq(&m.kind, ty))
            .ok_or(GuiEntityError::BadData)?
        })
    }

    /// `mob`'s geometry around its feet, as the world draws it: a mod's
    /// renderer for its type in place of its model, else the model; then the
    /// mods' layers. Full bright, seen from its own position.
    pub fn gui_entity_geometry(&self, mob: &Mob, out: &mut Vec<Vertex>) {
        let p = mob.position;
        let origin = DVec3::new(p.x, p.y, p.z);
        let camera = mod_api::Camera {
            position: origin.to_array(),
            eye_height: 0.0,
            partial_tick: 1.0,
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            fov_degrees: 0.0,
            near: 0.0,
            far: 0.0,
            view: Mat4::IDENTITY.to_cols_array(),
            projection: Mat4::IDENTITY.to_cols_array(),
            dimension: String::new(),
        };
        let view = mod_api::WorldRenderView::new(
            &camera,
            None,
            1.0,
            self.world.game_time,
            self.animation_time(),
            false,
            &[],
        );
        self.gui_entity_geometry_at(mob, 1.0, origin, Some(&view), out);
    }

    /// `gui_entity_geometry` at `partial` into the tick, around `camera`: what a screen
    /// scene's entities submit, in the scene's own coordinates.
    pub(in crate::game::client_side) fn gui_entity_geometry_at(
        &self,
        mob: &Mob,
        partial: f32,
        camera: DVec3,
        explicit_view: Option<&mod_api::WorldRenderView<'_>>,
        out: &mut Vec<Vertex>,
    ) {
        let is_player = &*mob.kind.id == "minecraft:player";
        let resolved = self.entity_layers(mob);
        let added = resolved.layers;
        // A mod's renderer for the type draws in place of the model only: the mods' layers
        // still follow, as in the world.
        let modded = self.mod_living_renderer_geometry_with_view(
            mob,
            partial,
            camera,
            explicit_view,
            FULL_BRIGHT,
            false,
            out,
        );
        let hidden = if modded {
            StockLayerMask::EMPTY
        } else {
            resolved.hidden
        };
        let mut frame_draw = FrameDraw::new(!modded && added.is_some());
        let atlas = &self.client.gui.atlas;
        let lookup = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        let data = super::humanoid_setup_anim::HiddenLayersData {
            game: self,
            hidden_layers: hidden,
        };
        // A screen's entity: custom entity model animations see is_in_gui = 1.
        sunrise_client::mobs::cem_anim::in_gui(|| {
            if modded {
                return;
            }
            sunrise_client::mobs::render::render_with_frame(
                mob,
                partial,
                camera,
                &lookup,
                FULL_BRIGHT,
                &data,
                &[],
                &mut frame_draw,
                out,
            );
        });
        let stock_ctx = super::native_entity_layers::StockLayerContext {
            is_player,
            local_player: false,
            invisible: mob.is_invisible(),
            in_world: false,
            player_name: None,
            alpha: f64::from(partial),
            camera,
            light: FULL_BRIGHT,
            supported: if modded {
                StockLayerMask::EMPTY
            } else {
                resolved.supported
            },
            hidden,
        };
        self.mod_layer_geometry_with_view(
            mob,
            None,
            frame_draw.model(),
            added,
            &stock_ctx,
            partial,
            camera,
            explicit_view,
            FULL_BRIGHT,
            out,
        );
    }
}

/// `verts` seen through `gui`, into `quads` far to near, each lit by
/// `Lighting.ENTITY_IN_UI` (`entity_in_ui_shade`). A face that looks away is
/// left out only when its render type culls (`culls_back_faces`): living
/// models draw as `entityCutoutNoCull`, as in the world, and their art can sit
/// on such a face (a chicken's legs are painted on their back and bottom
/// faces, seen through the transparent front ones).
fn seen_quads<const SHADE: bool>(verts: &[Vertex], gui: Mat4, quads: &mut Vec<SeenQuad>) {
    quads.clear();
    for q in verts.as_chunks::<4>().0 {
        if (q[0].layer & 0xffff) == sunrise_client_render::gfx::ARMOR_GLINT_ONLY_OVERLAY {
            continue;
        }
        let p: [Vec3; 4] =
            std::array::from_fn(|i| gui.transform_point3(Vec3::from_array(q[i].pos)));
        let n = (p[1] - p[0]).cross(p[2] - p[0]);
        // The winding gives inward normals: faces with n.z >= 0 point away.
        let back = n.z >= 0.0;
        if back && sunrise_client_render::mesher::culls_back_faces(q[0].normal[3]) {
            continue;
        }
        let color = if SHADE {
            // Lit by its outward normal, whichever way it faces, as vanilla's vertex normals.
            let shade = super::perspective::entity_in_ui_shade(-n.normalize_or_zero());
            let c = q[0].color;
            let ch = |s: u32| ((((c >> s) & 255) as f32 * shade) as u32) << s;
            (c & 0xFF00_0000) | ch(0) | ch(8) | ch(16)
        } else {
            q[0].color
        };
        let depth = p.iter().map(|v| v.z).sum::<f32>() / 4.0;
        let (mut corners, mut uv) = (p.map(|v| v.to_array()), std::array::from_fn(|i| q[i].uv));
        if back {
            // Seen from behind: the same quad wound towards the viewer, as the player preview's.
            corners.reverse();
            uv.reverse();
        }
        quads.push((depth, corners, uv, color, quads.len()));
    }
    // Far to near for translucent faces. Equal depths keep their emission order (the ordinal),
    // so a layer the model draws again over the same faces (a villager's type and profession
    // clothes over its body) stays after them and the depth test (LessEqual) shows the layer,
    // as the world's draw order does. In place, so the sort allocates nothing each frame.
    quads.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.4.cmp(&b.4)));
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(test)]
mod tests {
    fn vertex(pos: [f32; 3], w: i8, u: f32) -> Vertex {
        Vertex {
            pos,
            uv: [u, 0.0],
            layer: 0,
            color: 0xFFFF_FFFF,
            light: 0,
            normal: [0, 0, 0, w],
            glint_uv: [0.0; 2],
        }
    }
    /// A face at depth `z` looking towards the viewer (or away, reversed), its
    /// vertices' normal w `w` and its UVs marked `u`.
    pub fn face(z: f32, w: i8, u: f32, away: bool) -> [Vertex; 4] {
        let mut f = [
            vertex([0.0, 0.0, z], w, u),
            vertex([0.0, 1.0, z], w, u),
            vertex([1.0, 1.0, z], w, u),
            vertex([1.0, 0.0, z], w, u),
        ];
        if away {
            f.reverse();
        }
        f
    }

    use super::*;

    #[test]
    fn the_cursor_turns_the_entity_as_follows_mouse_does() {
        // The cursor on the centre: facing the viewer, untilted.
        assert_eq!(
            turn(GuiEntityLook::At([50.0, 60.0]), [50.0, 60.0]),
            (180.0, 180.0, 0.0, 0.0)
        );
        // Left of the centre: atan(40 / 40) = pi/4, body 20 of it, head 40.
        let (body, head, pitch, tilt) = turn(GuiEntityLook::At([10.0, 60.0]), [50.0, 60.0]);
        let quarter = std::f32::consts::FRAC_PI_4;
        assert_eq!(
            (body, head),
            (180.0 + quarter * 20.0, 180.0 + quarter * 40.0)
        );
        assert_eq!((pitch, tilt), (0.0, 0.0));
        // Above: the head pitches up (negative) and the whole entity tilts.
        let (_, _, pitch, tilt) = turn(GuiEntityLook::At([50.0, 20.0]), [50.0, 60.0]);
        assert_eq!((pitch, tilt), (-quarter * 20.0, quarter * 20.0));
        let fixed = turn(
            GuiEntityLook::Turned {
                yaw: 30.0,
                pitch: 10.0,
            },
            [0.0; 2],
        );
        assert_eq!(fixed, (210.0, 210.0, 10.0, 0.0));
    }

    #[test]
    fn faces_looking_away_are_culled_and_the_rest_sorted_far_to_near() {
        // Two faces towards the viewer at z 1 and 2, and one facing away whose
        // render type culls (an `entityCutoutCull` model's, `SHADED_CULL_W`).
        let cull = sunrise_client_render::mesher::SHADED_CULL_W;
        let mut verts = Vec::new();
        verts.extend(face(2.0, cull, 0.0, false));
        verts.extend(face(1.0, cull, 0.0, false));
        verts.extend(face(0.0, cull, 0.0, true));
        let mut quads = Vec::new();
        seen_quads::<true>(&verts, Mat4::IDENTITY, &mut quads);
        let depths: Vec<f32> = quads.iter().map(|q| q.0).collect();
        assert_eq!(depths.len(), 2, "{depths:?}");
        assert!(depths[0] < depths[1]);
    }

    /// A living model's faces draw both sides (`entityCutoutNoCull`, normal w
    /// -127), as in the world: a chicken's legs are painted on faces that look
    /// away. Such a face is kept and wound towards the viewer.
    #[test]
    fn no_cull_faces_looking_away_are_kept_and_turned_to_the_viewer() {
        let mut quads = Vec::new();
        seen_quads::<true>(&face(0.0, -127, 0.5, true), Mat4::IDENTITY, &mut quads);
        assert_eq!(quads.len(), 1);
        let p = quads[0].1.map(Vec3::from_array);
        assert!(
            (p[1] - p[0]).cross(p[2] - p[0]).z < 0.0,
            "wound towards the viewer"
        );
        // A face the model culls still goes.
        let cull = sunrise_client_render::mesher::SHADED_CULL_W;
        seen_quads::<true>(&face(0.0, cull, 0.5, true), Mat4::IDENTITY, &mut quads);
        assert!(quads.is_empty());
    }

    /// A layer drawn again over the same faces (a villager's clothes over its
    /// body) has the same depth: it must stay after them so the depth test
    /// (LessEqual) shows it, whatever the sort meets.
    #[test]
    fn coplanar_layers_keep_their_draw_order() {
        let mut verts = Vec::new();
        for layer in 0..48 {
            verts.extend(face(1.0, -127, layer as f32, false));
        }
        let mut quads = Vec::new();
        seen_quads::<true>(&verts, Mat4::IDENTITY, &mut quads);
        let order: Vec<f32> = quads.iter().map(|q| q.2[0][0]).collect();
        assert_eq!(order, (0..48).map(|l| l as f32).collect::<Vec<_>>());
    }

    #[test]
    fn unshaded_seen_quads_retain_vertex_color() {
        let mut shaded = Vec::new();
        let mut unshaded = Vec::new();
        let quad = face(0.0, -127, 0.5, false);
        seen_quads::<true>(&quad, Mat4::IDENTITY, &mut shaded);
        seen_quads::<false>(&quad, Mat4::IDENTITY, &mut unshaded);
        assert_eq!(unshaded.len(), 1);
        assert_eq!(unshaded[0].3, 0xFFFF_FFFF);
        assert_ne!(shaded[0].3, 0xFFFF_FFFF);
    }

    #[test]
    fn views_that_cant_be_drawn_are_refused() {
        let look = GuiEntityLook::At([0.0; 2]);
        assert!(GuiEntityView::new([0.0, 0.0, 50.0, 70.0], 30.0, 0.0625, look).is_drawable());
        assert!(!GuiEntityView::new([0.0, 0.0, 0.0, 70.0], 30.0, 0.0, look).is_drawable());
        assert!(!GuiEntityView::new([0.0, 0.0, 50.0, 70.0], 0.0, 0.0, look).is_drawable());
        assert!(!GuiEntityView::new([0.0, 0.0, 50.0, 70.0], 30.0, f32::NAN, look).is_drawable());
        let nan = GuiEntityLook::Turned {
            yaw: f32::INFINITY,
            pitch: 0.0,
        };
        assert!(!GuiEntityView::new([0.0, 0.0, 50.0, 70.0], 30.0, 0.0, nan).is_drawable());
    }

    fn part_vertex_range(root: &sunrise_client::mobs::Part, name: &str) -> std::ops::Range<usize> {
        fn walk(
            part: &sunrise_client::mobs::Part,
            name: &str,
            cursor: &mut usize,
        ) -> Option<std::ops::Range<usize>> {
            let start = *cursor;
            *cursor += part.polygons.len() * 4;
            let mut found = None;
            for child in &part.children {
                if let Some(range) = walk(child, name, cursor) {
                    found = Some(range);
                }
            }
            if part.name == name {
                return Some(start..*cursor);
            }
            found
        }
        walk(root, name, &mut 0).expect("dragon model part")
    }

    /// The dragon uses a separate render path from the client's mob list (#6050).
    #[test]
    #[ignore = "daily"]
    fn a_screen_draws_the_ender_dragon_by_type() {
        use sunrise_client::hud::Blend;
        use sunrise_level::random_source::{record_seeds, replay_seeds};
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            let look = GuiEntityLook::At([50.0, 40.0]);
            let view = GuiEntityView::new([26.0, 8.0, 75.0, 78.0], 4.0, 0.0, look);
            let mut batch = HudBatch::scratch([320, 240]);
            let dragon = GuiEntity::kind(DRAGON);
            assert_eq!(game.native_draw_entity(&mut batch, &dragon, view), Ok(()));
            // Body draws in the depth pass; eyes follow in the depth-tested additive run unshaded.
            let runs = batch.runs();
            assert_eq!(
                runs.len(),
                2,
                "expected body depth + additive eyes runs: {runs:?}"
            );
            let (body_range, Blend::Depth(body_clip)) = runs[0].clone() else {
                panic!("expected Depth body run, got {:?}", runs[0]);
            };
            let (eyes_range, Blend::DepthGlint(eyes_clip)) = runs[1].clone() else {
                panic!("expected DepthGlint additive eyes run, got {:?}", runs[1]);
            };
            assert_eq!(body_clip, eyes_clip);
            assert!(!body_range.is_empty());
            assert!(!eyes_range.is_empty());
            assert_eq!(body_range.end, eyes_range.start);
            let body_verts = &batch.vertices[body_range.start as usize..body_range.end as usize];
            let eye_verts = &batch.vertices[eyes_range.start as usize..eyes_range.end as usize];
            assert!(
                eye_verts
                    .iter()
                    .all(|v| v.color == 0xFFFF_FFFF && v.uv[0] < 2.0),
                "eyes keep unshaded full-bright color and normal atlas UVs"
            );
            assert!(
                body_verts.iter().any(|v| v.color != 0xFFFF_FFFF),
                "body keeps ENTITY_IN_UI directional shading"
            );

            // Repeated by-type draws consume no process-wide entity seeds.
            let ((), seeds) = record_seeds(|| {
                for _ in 0..3 {
                    assert_eq!(game.native_draw_entity(&mut batch, &dragon, view), Ok(()));
                }
            });
            replay_seeds(seeds.clone(), || {});
            assert!(
                seeds.complete(),
                "by-type dragon draws must not allocate entity seeds"
            );

            let mut other = Compound::new();
            other.insert("id".into(), Tag::String("minecraft:zombie".into()));
            let mut refused = HudBatch::scratch([320, 240]);
            assert_eq!(
                game.native_draw_entity(&mut refused, &GuiEntity::with_data(DRAGON, other), view),
                Err(GuiEntityError::BadData)
            );
            assert!(refused.vertices.is_empty());

            // Vanilla's default Hover phase sits, and the preview discards the saved position.
            let model = game.client.dragon_model.as_ref().expect("dragon model");
            let head_range = part_vertex_range(model, "head");
            let neck_range = part_vertex_range(model, "neck2");
            let body_part_range = part_vertex_range(model, "body");
            let mut flying_nbt = Compound::new();
            flying_nbt.insert("DragonPhase".into(), Tag::Int(0));
            let mut sitting_nbt = Compound::new();
            sitting_nbt.insert("DragonPhase".into(), Tag::Int(5));
            let mut sitting_pos_nbt = sitting_nbt.clone();
            sitting_pos_nbt.insert(
                "Pos".into(),
                Tag::List(vec![
                    Tag::Double(100.0_f64.to_bits()),
                    Tag::Double(200.0_f64.to_bits()),
                    Tag::Double(300.0_f64.to_bits()),
                ]),
            );
            let straight = GuiDragonLook::default();
            let (mut fly_verts, mut fly_eyes) = (Vec::new(), Vec::new());
            let (mut sit_verts, mut sit_eyes) = (Vec::new(), Vec::new());
            game.gui_dragon_geometry(
                DragonPreview::from_nbt(&flying_nbt),
                straight,
                &mut fly_verts,
                &mut fly_eyes,
            );
            game.gui_dragon_geometry(
                DragonPreview::from_nbt(&sitting_nbt),
                straight,
                &mut sit_verts,
                &mut sit_eyes,
            );
            let pos_slice = |v: &[Vertex], r: &std::ops::Range<usize>| -> Vec<[u32; 3]> {
                v[r.clone()]
                    .iter()
                    .map(|v| v.pos.map(f32::to_bits))
                    .collect()
            };
            assert_ne!(
                pos_slice(&fly_verts, &neck_range),
                pos_slice(&sit_verts, &neck_range),
                "sitting DragonPhase:5 changes neck geometry relative to flying DragonPhase:0"
            );
            assert_eq!(
                pos_slice(&fly_verts, &body_part_range),
                pos_slice(&sit_verts, &body_part_range),
                "sitting phase leaves body vertices unchanged"
            );
            let flat_view = GuiEntityView::new(
                view.rect,
                4.0,
                0.0,
                GuiEntityLook::Turned {
                    yaw: 0.0,
                    pitch: 0.0,
                },
            );
            let batch_positions = |entity: &GuiEntity, v: GuiEntityView| -> Vec<[u32; 2]> {
                let mut b = HudBatch::scratch([320, 240]);
                assert_eq!(game.native_draw_entity(&mut b, entity, v), Ok(()));
                b.vertices
                    .iter()
                    .map(|vt| vt.pos.map(f32::to_bits))
                    .collect()
            };
            let fly_batch = batch_positions(&GuiEntity::with_data(DRAGON, flying_nbt), flat_view);
            let sit_batch = batch_positions(&GuiEntity::with_data(DRAGON, sitting_nbt), flat_view);
            let sit_pos_batch =
                batch_positions(&GuiEntity::with_data(DRAGON, sitting_pos_nbt), flat_view);
            let hover_batch = batch_positions(&dragon, flat_view);
            assert_ne!(fly_batch, sit_batch);
            assert_eq!(sit_batch, sit_pos_batch, "saved Pos is discarded");
            assert_eq!(
                hover_batch, sit_batch,
                "default Hover phase is sitting in vanilla DragonHoverPhase"
            );

            // Changing only Turned.pitch moves the head vertices and leaves body vertices in place.
            let pitched_view = GuiEntityView::new(
                view.rect,
                4.0,
                0.0,
                GuiEntityLook::Turned {
                    yaw: 0.0,
                    pitch: 30.0,
                },
            );
            let (mut pitched_verts, mut pitched_eyes) = (Vec::new(), Vec::new());
            game.gui_dragon_geometry(
                DragonPreview::from_nbt(&Compound::new()),
                GuiDragonLook {
                    body_yaw: 0.0,
                    head_yaw: 0.0,
                    head_pitch: 30.0,
                },
                &mut pitched_verts,
                &mut pitched_eyes,
            );
            assert_ne!(
                pos_slice(&sit_verts, &head_range),
                pos_slice(&pitched_verts, &head_range),
                "head pitch moves head vertices"
            );
            assert_eq!(
                pos_slice(&sit_verts, &body_part_range),
                pos_slice(&pitched_verts, &body_part_range),
                "head pitch leaves body vertices in place"
            );
            let pitched_batch = batch_positions(&dragon, pitched_view);
            assert_ne!(hover_batch, pitched_batch);

            // Logged-in client fixture: parent dragon draws, part returns NoEntity, removed parent returns NoEntity.
            let (client, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(client, "GuiDragonTest").unwrap();
            let remote = game.client.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            remote.profile = 42;
            game.common.player = Some(
                sunrise_sim::motion::PlayerMotion::new(game.common.motion, Position::ZERO).unwrap(),
            );
            game.spawn_entity(15, DRAGON, [0.0; 3], [0.0; 3]);
            let tracked = game.entity(15).expect("tracked dragon handle");
            let part = game.entity(16).expect("dragon part handle");
            let mut existing_batch = HudBatch::scratch([320, 240]);
            assert_eq!(
                game.native_draw_entity(&mut existing_batch, &GuiEntity::existing(tracked), view),
                Ok(())
            );
            assert!(!existing_batch.vertices.is_empty());
            let mut refused_part = HudBatch::scratch([320, 240]);
            assert_eq!(
                game.native_draw_entity(&mut refused_part, &GuiEntity::existing(part), view),
                Err(GuiEntityError::NoEntity)
            );
            assert!(refused_part.vertices.is_empty());
            game.remove_entity(15);
            let mut refused_gone = HudBatch::scratch([320, 240]);
            assert_eq!(
                game.native_draw_entity(&mut refused_gone, &GuiEntity::existing(tracked), view),
                Err(GuiEntityError::NoEntity)
            );
            assert!(refused_gone.vertices.is_empty());
        });
    }
}
