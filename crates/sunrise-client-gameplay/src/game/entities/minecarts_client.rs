use crate::game::ClientGame;
use std::collections::HashMap;
use sunrise_client::{
    mesher::Vertex,
    mobs::Part,
    sound_engine::{CartSound, PlayResult, SoundInstance},
};
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::minecarts::CartKind;
use sunrise_gameplay_common::game::minecarts::HEIGHT;
use sunrise_gameplay_common::game::minecarts::TNT_PRIME_EVENT;
use sunrise_gameplay_common::game::minecarts::WIDTH;
use sunrise_gameplay_common::game::minecarts::cart_render_bounds;
use sunrise_gameplay_common::game::minecarts::display_offset;
use sunrise_gameplay_common::game::minecarts::shown_unless_air;
use sunrise_gameplay_common::game::minecarts::{Minecart, Shape};
use sunrise_gameplay_common::game::{
    api_values::TryIntoApi, cart_rails::dispatch_cart_rail_direction,
    native_level_signals::ReadLevel,
};
use sunrise_level::random_source;
use sunrise_sim::mobs::anim::Poses;
use sunrise_world::registry::BlockStateId;

#[cfg_attr(test, derive(Clone))]
pub(in crate::game) struct CartModels {
    pub models: HashMap<CartKind, Part>,
}
impl CartModels {
    pub fn load(document: Option<&sunrise_client::mobs::EntityModelDocument>) -> Self {
        let kinds = [
            CartKind::Rideable,
            CartKind::Chest,
            CartKind::Hopper,
            CartKind::Furnace,
            CartKind::Tnt,
            CartKind::CommandBlock,
            CartKind::Spawner,
        ];
        let models = kinds
            .into_iter()
            .filter_map(|k| document?.model(&format!("{}#main", k.id())).map(|m| (k, m)))
            .collect();
        Self { models }
    }
}
/// Local geometry with atlas coordinates resolved for both CPU and GPU consumers.
#[derive(Clone, Copy)]
pub(in crate::game) struct PreparedModelFace {
    pub(in crate::game) positions: [[f32; 3]; 4],
    pub(in crate::game) uv: [[f32; 2]; 4],
    pub(in crate::game) facing: sunrise_assets::model::Direction,
    pub(in crate::game) tint_index: i32,
    /// The side that hides this face when a neighbour covers it (`ModelQuad::cull`).
    pub(in crate::game) cull: Option<sunrise_assets::model::Direction>,
    pub(in crate::game) light_emission: u8,
}

impl PreparedModelFace {
    pub(in crate::game) fn new(
        quad: &sunrise_assets::bake::ModelQuad,
        sprite: sunrise_client::hud::Rect,
    ) -> Self {
        Self {
            positions: quad.positions,
            uv: quad.uv.map(|uv| {
                [
                    sprite.u0 + uv[0] * (sprite.u1 - sprite.u0),
                    sprite.v0 + uv[1] * (sprite.v1 - sprite.v0),
                ]
            }),
            facing: quad.facing,
            tint_index: quad.tint_index,
            cull: quad.cull,
            light_emission: quad.light_emission,
        }
    }

    pub(in crate::game) fn positions(&self) -> [[f32; 3]; 4] {
        self.positions
    }
}

/// Shared transform, shading and light path for block-state and prepared standalone faces.
pub(in crate::game) fn emit_model_faces(
    faces: impl IntoIterator<Item = (PreparedModelFace, u32)>,
    m: glam::Mat4,
    diffuse: bool,
    tint: impl Fn(i32) -> Option<u32>,
    out: &mut Vec<Vertex>,
) {
    use sunrise_assets::model::Direction as D;
    let mut normals = sunrise_client::item_render::FaceNormals::new(m);
    for (quad, light) in faces {
        let shade: u32 = match quad.facing {
            D::Up => 255,
            D::Down => 127,
            D::North | D::South => 204,
            D::West | D::East => 153,
        };
        let mut color = if diffuse {
            u32::MAX
        } else {
            0xFF00_0000 | shade << 16 | shade << 8 | shade
        };
        if let Some(t) = tint(quad.tint_index) {
            color =
                sunrise_client::hud_layout::multiply(color, sunrise_client::mobs::render::abgr(t));
        }
        let normal = if diffuse {
            let n = normals
                .face(quad.facing)
                .to_array()
                .map(|v| (v.clamp(-1.0, 1.0) * 127.0) as i8);
            [n[0], n[1], n[2], 127]
        } else {
            [0; 4]
        };
        let light = if diffuse {
            let emission = u32::from(quad.light_emission) * 16;
            (light & 0xFF) | (light >> 8 & 0xFF).max(emission) << 8
        } else {
            light
        };
        for k in 0..4 {
            let uv = quad.uv[k];
            out.push(Vertex {
                normal,
                pos: m
                    .transform_point3(glam::Vec3::from_array(quad.positions[k]))
                    .to_array(),
                uv,
                layer: 0,
                color,
                light,
                glint_uv: [0.0, 0.0],
            });
        }
    }
}

/// Vanilla multiplies tint, alpha included, only when `tintIndex != -1`.
pub(in crate::game) fn scale_tint_alpha<'f>(
    vertices: &mut [Vertex],
    faces: impl IntoIterator<Item = &'f PreparedModelFace>,
    tint: u32,
) {
    let alpha = tint >> 24;
    if alpha == 255 {
        return;
    }
    let (quads, _) = vertices.as_chunks_mut::<4>();
    for (face, quad) in faces.into_iter().zip(quads) {
        if face.tint_index < 0 {
            continue;
        }
        for vertex in quad {
            let scaled = (vertex.color >> 24) * alpha / 255;
            vertex.color = vertex.color & 0x00FF_FFFF | scaled << 24;
        }
    }
}

/// `Minecart.tick`: stationary rotation does not turn the passenger.
pub(in crate::game) fn accumulate_rotation(c: &mut Minecart) {
    c.passenger_yaw_delta = 0.0;
    if c.kind == CartKind::Rideable && c.position.subtract(c.previous).length() > 0.01 {
        c.rotation_offset += ((f64::from(c.yaw) - f64::from(c.previous_yaw)) % 360.0) as f32;
        c.rotation_offset %= 360.0;
    }
}
/// `Minecart.positionRider`: double rotLerp, then round back to float.
/// Only yaw follows; the player's pitch and manual look remain their own.
pub(in crate::game) fn rotate_passenger(c: &mut Minecart, yaw: &mut f32) {
    let mut delta = (f64::from(c.rotation_offset) - f64::from(c.player_rotation_offset)) % 360.0;
    if delta >= 180.0 {
        delta -= 360.0;
    }
    if delta < -180.0 {
        delta += 360.0;
    }
    let next = (f64::from(c.player_rotation_offset) + 0.5 * delta) as f32;
    let previous = *yaw;
    *yaw -= next - c.player_rotation_offset;
    c.passenger_yaw_delta = *yaw - previous;
    c.player_rotation_offset = next;
}

impl ClientGame<'_> {
    pub(in crate::game) fn client_cart_rail_direction(
        &self,
        cart: Option<u32>,
        state: BlockStateId,
        pos: sunrise_core::position::BlockPos,
        text: &str,
        vanilla: Shape,
    ) -> Shape {
        let (Some(level), Some(epoch), Ok(api_state)) = (
            self.level(self.common.dim.id()),
            self.common.native_lifecycle.epoch(),
            state.try_into_api(),
        ) else {
            return vanilla;
        };
        let read = ReadLevel::new(self, level, &self.common.world);
        let side = mod_api::LevelSide::Client(&read);
        let cart = cart.map(|id| mod_api::EntityRef::new(epoch, id));
        dispatch_cart_rail_direction(side, self.common.dim.level(), api_state, pos, cart, text)
    }

    /// `tinted_block_model_vertices` for baked models given as they are (a
    /// state's, or a standalone model).
    pub(in crate::game) fn baked_model_vertices<'m>(
        &self,
        models: impl IntoIterator<Item = &'m sunrise_client::models::catalog::BakedModel>,
        m: glam::Mat4,
        light: impl Fn(&sunrise_assets::bake::ModelQuad) -> u32,
        diffuse: bool,
        tint: impl Fn(i32) -> Option<u32>,
        out: &mut Vec<Vertex>,
    ) {
        let faces = models.into_iter().flat_map(|model| {
            model.faces.iter().filter_map(|face| {
                let sprite = self
                    .client
                    .client_pack
                    .layer_names
                    .get(&face.layer)
                    .and_then(|name| self.client.gui.atlas.sprites.get(name))?;
                Some((
                    PreparedModelFace::new(&face.quad, *sprite),
                    light(&face.quad),
                ))
            })
        });
        emit_model_faces(faces, m, diffuse, tint, out);
    }
    /// `BlockModelResolver.update` + `BlockModelRenderState.submit`: combines external light
    /// with the block state's own light component-wise (`emissiveRendering` gives full sky+block
    /// light `(15, 15)`, otherwise `(0, state.getLightEmission())`).
    pub(in crate::game::client_side) fn block_display_light(
        &self,
        state: BlockStateId,
        external_light: u32,
    ) -> u32 {
        let emissive = self
            .common
            .pack
            .assets
            .registry
            .state(state)
            .is_some_and(|st| {
                let name = st.name.strip_prefix("minecraft:").unwrap_or(&st.name);
                name == "magma_block"
                    || (matches!(name, "sculk_sensor" | "calibrated_sculk_sensor")
                        && st.properties.get("sculk_sensor_phase").map(String::as_str)
                            == Some("active"))
            });
        let (block_sky, block_em) = if emissive {
            (15 * 16, 15 * 16)
        } else {
            (
                0,
                u32::from(self.common.res.facts().get(state).emission) * 16,
            )
        };
        let sky = (external_light & 0xFF).max(block_sky);
        let block = ((external_light >> 8) & 0xFF).max(block_em);
        sky | (block << 8)
    }
    /// `BlockModelRenderState` / `BlockDisplayRenderer`: baked block-state faces with back-face
    /// culling (`SHADED_CULL_W`), one resolved sheet (`translucentBlockItemSheet` if any face is
    /// translucent, else `cutoutBlockItemSheet`), component-wise light max, JOML vertex transform,
    /// and reusable slot scratch. Returns whether the model routed to the translucent sheet.
    pub(in crate::game::client_side) fn block_display_model_vertices(
        &self,
        state: BlockStateId,
        seed: i64,
        pose: sunrise_client::camera::joml::Mat4,
        light: u32,
        slots: &mut Vec<u32>,
        out: &mut Vec<Vertex>,
    ) -> bool {
        if self
            .client
            .client_pack
            .render
            .model_catalog
            .slots_seeded(state, seed, slots)
            .is_err()
        {
            return false;
        }
        let effective_light = self.block_display_light(state, light);
        let has_translucency = slots.iter().any(|&slot| {
            self.client
                .client_pack
                .render
                .model_catalog
                .baked_model(slot)
                .faces
                .iter()
                .any(|face| {
                    let Some(name) = self.client.client_pack.layer_names.get(&face.layer) else {
                        return false;
                    };
                    if !self.client.gui.atlas.sprites.contains_key(name) {
                        return false;
                    }
                    face.translucent
                        || self.client.gui.atlas.translucent.contains(name)
                        || face.quad.render_type
                            == Some(sunrise_assets::bake::RenderType::Translucent)
                })
        });
        let layer = if has_translucency {
            sunrise_client::gfx::TRANSLUCENT_ITEM_OVERLAY
        } else {
            0
        };
        let tint = self.default_block_tint(state);
        let mut normals = sunrise_client::item_render::FaceNormals::new(pose.to_glam());
        for &slot in slots.iter() {
            let model = self
                .client
                .client_pack
                .render
                .model_catalog
                .baked_model(slot);
            for face in &model.faces {
                let Some(name) = self.client.client_pack.layer_names.get(&face.layer) else {
                    continue;
                };
                let Some(sprite) = self.client.gui.atlas.sprites.get(name) else {
                    continue;
                };
                let quad = PreparedModelFace::new(&face.quad, *sprite);
                let mut color = u32::MAX;
                if let Some(t) = tint(quad.tint_index) {
                    color = sunrise_client::hud_layout::multiply(
                        color,
                        sunrise_client::mobs::render::abgr(t),
                    );
                }
                let n = normals
                    .face(quad.facing)
                    .to_array()
                    .map(|v| (v.clamp(-1.0, 1.0) * 127.0) as i8);
                let normal = [n[0], n[1], n[2], sunrise_client::mesher::SHADED_CULL_W];
                let quad_light = (effective_light & 0xFF)
                    | ((effective_light >> 8) & 0xFF).max(u32::from(quad.light_emission) * 16) << 8;
                for k in 0..4 {
                    out.push(Vertex {
                        normal,
                        pos: pose.transform_position(quad.positions[k]),
                        uv: quad.uv[k],
                        layer,
                        color,
                        light: quad_light,
                        glint_uv: [0.0, 0.0],
                    });
                }
            }
        }
        has_translucency
    }
    /// Baked block-state faces with default block tints and terrain shade or diffuse lighting.
    pub fn block_model_vertices(
        &self,
        state: BlockStateId,
        seed: i64,
        m: glam::Mat4,
        light: u32,
        diffuse: bool,
        out: &mut Vec<Vertex>,
    ) {
        let tint = self.default_block_tint(state);
        self.tinted_block_model_vertices(state, seed, m, |_| light, diffuse, tint, out);
    }
    /// `AbstractMinecartRenderer` (old render): on the rail between the
    /// front and back points, pitched on slopes, with the display block.
    pub(in crate::game::client_side) fn cart_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
        run: std::ops::Range<usize>,
    ) {
        let a = alpha as f32;
        let Some(uv) = self
            .client
            .gui
            .atlas
            .sprites
            .get("entity/minecart/minecart")
            .map(|r| [r.u0, r.v0, r.u1, r.v1])
        else {
            return;
        };
        // Entity.shouldRenderAtSqrDistance: 64 times the bounding box's mean
        // edge, scaled by the view scale.
        let range = (WIDTH + HEIGHT + WIDTH) / 3.0 * 64.0 * self.view_scale();
        for c in &self.common.carts[run] {
            if self.hides_as_camera(c.id) {
                continue;
            }
            let Some(model) = self.client.cart_models.models.get(&c.kind) else {
                continue;
            };
            let lerp = self.remote_minecart_lerp(c.id, a);
            let new_render = self.minecart_improvements() || lerp.is_some();
            let (x, y, z) = if let Some((p, _, _)) = lerp {
                (p[0], p[1], p[2])
            } else {
                (
                    c.previous.x + (c.position.x - c.previous.x) * alpha,
                    c.previous.y + (c.position.y - c.previous.y) * alpha,
                    c.previous.z + (c.position.z - c.previous.z) * alpha,
                )
            };
            // LevelExtractor.isEntityVisible: the cart the player rides always draws.
            let mut shown = None;
            if self.common.riding != Some(c.id) {
                let d = glam::DVec3::new(c.position.x, c.position.y, c.position.z) - camera;
                if d.length_squared() >= range * range {
                    continue;
                }
                // Looked up once, past the distance cut: the cull and the draw both use it.
                shown = Some(self.display_state(c));
                // EntityRenderer.shouldRender: the interpolated box grown by 0.5;
                // AbstractMinecartRenderer.getBoundingBoxForCulling stretches it
                // up by the display block's offset.
                if let Some(view) = &self.client.view_frustum {
                    let top = HEIGHT
                        + if shown.flatten().is_none() {
                            0.0
                        } else {
                            display_offset(c) * 0.75 / 16.0
                        };
                    let p = glam::DVec3::new(x, y, z) - camera;
                    let lo = p - glam::DVec3::new(WIDTH / 2.0 + 0.5, 0.5, WIDTH / 2.0 + 0.5);
                    let hi = p + glam::DVec3::new(WIDTH / 2.0 + 0.5, top + 0.5, WIDTH / 2.0 + 0.5);
                    if !view.intersects(lo.as_vec3(), hi.as_vec3()) {
                        continue;
                    }
                }
            }
            let mut yaw =
                c.previous_yaw + ((c.yaw - c.previous_yaw + 180.0).rem_euclid(360.0) - 180.0) * a;
            let mut pitch = 0.0f32;
            let mut offset = glam::DVec3::ZERO;
            if new_render {
                (yaw, pitch) = lerp.map_or((c.yaw, c.pitch), |(_, yaw, pitch)| (yaw, pitch));
            } else if let Some(on) = self.rail_pos(Some(c.id), x, y, z) {
                // getPosOffs(..., 0.3F): the float widened.
                let half = f64::from(0.3_f32);
                let front = self.rail_pos_offs(Some(c.id), x, y, z, half).unwrap_or(on);
                let back = self.rail_pos_offs(Some(c.id), x, y, z, -half).unwrap_or(on);
                offset = glam::DVec3::new(on[0] - x, (front[1] + back[1]) / 2.0 - y, on[2] - z);
                let d =
                    glam::DVec3::new(back[0] - front[0], back[1] - front[1], back[2] - front[2]);
                if d.length() != 0.0 {
                    let d = d.normalize();
                    yaw = d.z.atan2(d.x).to_degrees() as f32;
                    pitch = (d.y.atan() * 73.0) as f32;
                }
            }
            let render_position = Vec3::new(x + offset.x, y + offset.y, z + offset.z);
            let has_display = shown.as_ref().is_some_and(Option::is_some)
                || (shown.is_none() && self.display_state(c).is_some());
            let culling = self.entity_cull_decision_in_bounds(
                c,
                cart_render_bounds(render_position, display_offset(c), has_display),
            );
            if culling.hides_entity() {
                continue;
            }
            let entity_start = out.len();
            self.mark_motion(
                entity_start,
                sunrise_gameplay_common::game::MOTION_OBJECT | u64::from(c.id),
            );
            // The per-entity jitter (offsetSeed) that keeps stacked carts from z-fighting.
            let seed = i64::from(c.id).wrapping_mul(493_286_711);
            let seed = seed
                .wrapping_mul(seed)
                .wrapping_mul(4_392_167_121)
                .wrapping_add(seed.wrapping_mul(98_761));
            let jitter = |shift: i64| (((seed >> shift & 7) as f32 + 0.5) / 8.0 - 0.5) * 0.004;
            let p = glam::DVec3::new(x, y, z) + offset - camera;
            let jittered = p.as_vec3() + glam::Vec3::new(jitter(16), jitter(20), jitter(24));
            let mut base = if new_render {
                // AbstractMinecartRenderer.newRender translates the model
                // after rotating, so the seat follows the pitched cart.
                glam::Mat4::from_translation(jittered)
                    * glam::Mat4::from_rotation_y(yaw.to_radians())
                    * glam::Mat4::from_rotation_z((-pitch).to_radians())
                    * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.375, 0.0))
            } else {
                glam::Mat4::from_translation(jittered + glam::Vec3::new(0.0, 0.375, 0.0))
                    * glam::Mat4::from_rotation_y((180.0 - yaw).to_radians())
                    * glam::Mat4::from_rotation_z((-pitch).to_radians())
            };
            let hurt = c.hurt_time as f32 - a;
            if hurt > 0.0 {
                let damage = (c.damage - a).max(0.0);
                base *= glam::Mat4::from_rotation_x(
                    (hurt.sin() * hurt * damage / 10.0 * c.hurt_dir as f32).to_radians(),
                );
            }
            let l = self.common.world.light(
                c.position.x.floor() as i32,
                (c.position.y + 0.35).floor() as i32,
                c.position.z.floor() as i32,
            );
            let packed = (u32::from(l >> 4) * 16) | ((u32::from(l & 15) * 16) << 8);
            if let Some(state) = shown.unwrap_or_else(|| self.display_state(c)) {
                let display = base
                    * glam::Mat4::from_scale(glam::Vec3::splat(0.75))
                    * glam::Mat4::from_translation(glam::Vec3::new(
                        -0.5,
                        (display_offset(c) as f32 - 8.0) / 16.0,
                        0.5,
                    ))
                    * glam::Mat4::from_rotation_y(90f32.to_radians());
                if c.kind == CartKind::Chest {
                    self.special_block_model(state, display, l, out);
                } else {
                    // TNT minecarts flash white as the fuse runs out (TntMinecartRenderer).
                    let swell = if c.kind == CartKind::Tnt && c.fuse > -1 {
                        let fuse = c.fuse as f32 - a + 1.0;
                        if fuse < 10.0 {
                            let g = (1.0 - fuse / 10.0).clamp(0.0, 1.0);
                            1.0 + g * g * g * g * 0.3
                        } else {
                            1.0
                        }
                    } else {
                        1.0
                    };
                    let m = display
                        * glam::Mat4::from_translation(glam::Vec3::splat(0.5))
                        * glam::Mat4::from_scale(glam::Vec3::splat(swell))
                        * glam::Mat4::from_translation(glam::Vec3::splat(-0.5));
                    let flash = c.kind == CartKind::Tnt && c.fuse > -1 && (c.fuse / 5) % 2 == 0;
                    let start = out.len();
                    self.block_model_vertices(state, 0, m, packed, false, out);
                    if flash {
                        // submitWhiteSolidBlock: OverlayTexture.u(1.0) on the cart's own light.
                        for v in &mut out[start..] {
                            v.layer = 5;
                        }
                    }
                }
            }
            let m = base * glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0));
            let poses = Poses::new(model);
            sunrise_client::mobs::render::draw(model, m, &poses, uv, 0xFFFF_FFFF, packed, 0, out);
            self.finish_entity_occlusion(culling, c, out, entity_start);
        }
    }
    /// `BlockTintSource.color(state)` with no level: each tinted face's default colour.
    pub(in crate::game) fn default_block_tint(
        &self,
        state: BlockStateId,
    ) -> impl Fn(i32) -> Option<u32> + '_ {
        let grass = self.client.client_pack.item_renderer.grass_color(0.5, 1.0) as u32;
        let tables = &self.client.client_pack.context.mesh.tables;
        move |index| tables.default_tint(state, index, grass)
    }
    /// The block the cart shows: `None` for air (`AbstractMinecartRenderer`
    /// draws and stretches its culling box only for a non-air state).
    pub(in crate::game::client_side) fn display_state(&self, c: &Minecart) -> Option<BlockStateId> {
        shown_unless_air(self.display_state_unfiltered(c), |s| {
            self.common.res.facts().is_air(s)
        })
    }
    pub(in crate::game::client_side) fn display_state_unfiltered(
        &self,
        c: &Minecart,
    ) -> Option<BlockStateId> {
        let registry = &self.common.pack.assets.registry;
        let with = |name: &str, props: &[(&str, &str)]| {
            let mut s = registry.state(registry.default_state(name)?)?.clone();
            for (k, v) in props {
                s.properties.insert((*k).to_owned(), (*v).to_owned());
            }
            registry.id(&s)
        };
        // AbstractMinecart.getDisplayBlockState: a custom state wins.
        if let Some(state) = c
            .custom_display
            .as_ref()
            .and_then(|tag| sunrise_sim::entity_nbt::state_from_nbt(registry, tag))
        {
            return Some(state);
        }
        match c.kind {
            CartKind::Rideable => None,
            CartKind::Chest => with("minecraft:chest", &[("facing", "north")]),
            CartKind::Hopper => registry.default_state("minecraft:hopper"),
            CartKind::Furnace => with(
                "minecraft:furnace",
                &[
                    ("facing", "north"),
                    ("lit", if c.fuel > 0 { "true" } else { "false" }),
                ],
            ),
            CartKind::Tnt => registry.default_state("minecraft:tnt"),
            CartKind::CommandBlock => registry.default_state("minecraft:command_block"),
            // MinecartSpawner.getDefaultDisplayBlockState.
            CartKind::Spawner => registry.default_state("minecraft:spawner"),
        }
    }
    /// `OldMinecartBehavior.getPosOffs`.
    pub(in crate::game::client_side) fn rail_pos_offs(
        &self,
        cart: Option<u32>,
        mut x: f64,
        y: f64,
        mut z: f64,
        offs: f64,
    ) -> Option<[f64; 3]> {
        let [xt, yt, zt] = self.rail_block(Vec3::new(x, y, z), self.minecart_improvements());
        let rail = self.rail(cart, xt, yt, zt)?;
        let mut y = f64::from(yt) + if rail.shape.is_slope() { 1.0 } else { 0.0 };
        let (e0, e1) = rail.shape.exits();
        let (mut xd, mut zd) = (f64::from(e1[0] - e0[0]), f64::from(e1[2] - e0[2]));
        let dd = sunrise_core::geometry::length(xd, zd);
        xd /= dd;
        zd /= dd;
        x += xd * offs;
        z += zd * offs;
        let at = |e: [i32; 3]| x.floor() as i32 - xt == e[0] && z.floor() as i32 - zt == e[2];
        if e0[1] != 0 && at(e0) {
            y += f64::from(e0[1]);
        } else if e1[1] != 0 && at(e1) {
            y += f64::from(e1[1]);
        }
        self.rail_pos(cart, x, y, z)
    }
    /// `MinecartTNT.handleEntityEvent`: event 70 calls `primeFuse`, which on a
    /// client only sets the fuse to 80. The window then counts it down
    /// (`MinecartTNT.tick`) for the smoke, the flash and the swell.
    pub(in crate::game::client_side) fn remote_tnt_cart_event(&mut self, id: i32, event: i8) {
        if event != TNT_PRIME_EVENT {
            return;
        }
        if let Some(i) = self
            .common
            .carts
            .iter()
            .position(|c| c.id == id as u32 && c.kind == CartKind::Tnt)
        {
            self.common.carts.update(i, |c| c.fuse = 80);
        }
    }
    /// The model of a block drawn by its block-entity renderer, as a chest
    /// minecart shows its chest and a block display its special block.
    pub(in crate::game::client_side) fn special_block_model(
        &self,
        state: BlockStateId,
        m: glam::Mat4,
        light: u8,
        out: &mut Vec<Vertex>,
    ) {
        with_default_scene(
            &self.client.gui,
            &self.common.world,
            self.nether_lighting(),
            |scene| {
                self.client_res()
                    .block_entity_render()
                    .render_state(scene, state.0, m, light, out);
            },
        );
    }
    /// `ClientPacketListener.postAddEntitySoundInstance` for the carts that
    /// joined the level since the last tick, and `LocalPlayer.startRiding`'s
    /// two inside sounds when the player got in one; then what the sound
    /// instances read this tick (the ridden cart and every cart).
    pub fn start_cart_sounds(&mut self) -> (Option<u32>, Vec<CartSound>) {
        let improved = self.minecart_improvements();
        let carts: Vec<CartSound> = self
            .common
            .carts
            .iter()
            .map(|c| CartSound {
                id: c.id,
                position: [c.position.x, c.position.y, c.position.z],
                // Vec3.horizontalDistance.
                speed: (c.velocity.x * c.velocity.x + c.velocity.z * c.velocity.z).sqrt(),
                new_behavior: improved,
                on_rails: c.on_rails,
            })
            .collect();
        self.client
            .cart_sounds
            .retain(|id| carts.iter().any(|c| c.id == *id));
        for cart in &carts {
            if !self.client.cart_sounds.contains(&cart.id) {
                let instance = SoundInstance::minecart(cart, random_source::create());
                let (_, res) = self.play_audio_instance(instance, false);
                if res != PlayResult::NotStarted {
                    self.client.cart_sounds.insert(cart.id);
                }
            }
        }
        let vehicle = self
            .common
            .riding
            .filter(|id| carts.iter().any(|c| c.id == *id));
        if vehicle != self.client.riding_cart_sounds
            && let Some(cart) = carts.iter().find(|c| Some(c.id) == vehicle)
        {
            let mut started = true;
            for underwater in [true, false] {
                let instance =
                    SoundInstance::riding_minecart(cart, underwater, random_source::create());
                let (_, res) = self.play_audio_instance(instance, false);
                if res == PlayResult::NotStarted {
                    started = false;
                }
            }
            if started {
                self.client.riding_cart_sounds = vehicle;
            }
        } else if vehicle.is_none() {
            self.client.riding_cart_sounds = None;
        }
        (vehicle, carts)
    }
    /// `block_model_vertices` with each face multiplied by `tint` of its
    /// tint index (RGB, `None` for untinted faces) and lit by `light` of
    /// each quad.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game::client_side) fn tinted_block_model_vertices(
        &self,
        state: BlockStateId,
        seed: i64,
        m: glam::Mat4,
        light: impl Fn(&sunrise_assets::bake::ModelQuad) -> u32,
        diffuse: bool,
        tint: impl Fn(i32) -> Option<u32>,
        out: &mut Vec<Vertex>,
    ) {
        let Ok(models) = self
            .client
            .client_pack
            .render
            .model_catalog
            .collect_seeded(state, seed)
        else {
            return;
        };
        let models = models.into_iter().map(|(_, model)| model);
        self.baked_model_vertices(models, m, light, diffuse, tint, out);
    }
}

/// Calls `render` with the scene of a block drawn with no block entity data: `world` at its own
/// game time, no sign, banner or skull data and every chest lid shut. Shared by
/// [`ClientGame::special_block_model`] and `StandaloneGeometry`, which sizes the special part in
/// an empty world before any level is loaded.
pub(in crate::game) fn with_default_scene(
    gui: &crate::game::UiResources,
    world: &sunrise_level::world::World,
    nether: bool,
    render: impl FnOnce(&sunrise_client::block_entity_render::Scene<'_>),
) {
    let lid = |_: [i32; 3]| 0.0;
    let scene = sunrise_client::block_entity_render::Scene {
        camera: glam::DVec3::ZERO,
        sign_outline: sunrise_client::block_entity_render::SignOutline::None,
        orientation: glam::Quat::IDENTITY,
        anim: &|_| sunrise_client::block_entity_render::BlockAnim::default(),
        texts: None,
        partial: 0.0,
        game_time: world.game_time,
        max_y: world.shape.max_y() - 1,
        world,
        sprites: &gui.atlas.sprites,
        emissive: &gui.atlas.emissive,
        translucent_sprites: &gui.atlas.translucent,
        atlas_revision: gui.atlas.revision,
        nether,
        lid: &lid,
        sign: &|_| None,
        banner: &|_| Vec::new(),
        skull: &|_| None,
        glyph: &|c| gui.atlas.glyph_cell(c),
        glyphs: gui.atlas.font.epoch(),
        font: Some(&gui.atlas.font),
        inline_images: None,
        dynamic_visible: None,
    };
    render(&scene);
}

/// Packed vertex light as the level's `sky << 4 | block` byte that
/// [`ClientGame::special_block_model`] takes: the inverse of `block_entity_render::vertex_light`.
pub fn special_light(light: u32) -> u8 {
    ((((light & 0xFF) / 16) << 4) | (((light >> 8) & 0xFF) / 16)) as u8
}

impl PreparedModelFace {
    pub(in crate::game) fn gpu_vertex(
        &self,
        corner: usize,
    ) -> sunrise_client::gfx::PreparedModelVertex {
        let normal = sunrise_client::item_render::direction_vector(self.facing);
        sunrise_client::gfx::PreparedModelVertex {
            pos: [
                self.positions[corner][0],
                self.positions[corner][1],
                self.positions[corner][2],
                1.0,
            ],
            uv: [self.uv[corner][0], self.uv[corner][1], 0.0, 0.0],
            normal: [normal.x, normal.y, normal.z, 0.0],
            flags: [
                u32::from(self.tint_index >= 0),
                u32::from(self.light_emission),
                0,
                0,
            ],
        }
    }
    /// Whether `hidden` (`ModelInstance::hidden_faces`) leaves this face out.
    pub(in crate::game) fn hidden_by(&self, hidden: u8) -> bool {
        self.cull.is_some_and(|side| hidden >> side as u8 & 1 != 0)
    }
}

use sunrise_gameplay_common::game::cart_rails::CartRailsGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::minecarts::MinecartRailGameExt as _;
