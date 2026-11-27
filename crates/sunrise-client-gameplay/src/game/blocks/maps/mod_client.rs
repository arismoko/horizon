use crate::game::ClientGame;
use sunrise_client::{
    hud::{HudBatch, Rect, map_slot_sprite},
    mesher::Vertex,
};
use sunrise_gameplay_common::game::maps::{
    Decoration, Key, MapData, decoration_pose, decoration_pose_at, decoration_sprite, map_id,
    packed_rgba, shown_on_frame,
};

impl ClientGame<'_> {
    /// `renderMap`: map `id` at pose `m`, its background (with a 7-pixel
    /// border) then the map.
    pub(in crate::game::client_side) fn held_map(
        &mut self,
        m: glam::Mat4,
        id: i32,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        let deg = |d: f32| d.to_radians();
        let m = m
            * glam::Mat4::from_rotation_y(deg(180.0))
            * glam::Mat4::from_rotation_z(deg(180.0))
            * glam::Mat4::from_scale(glam::Vec3::splat(0.38))
            * glam::Mat4::from_translation(glam::Vec3::new(-0.5, -0.5, 0.0))
            * glam::Mat4::from_scale(glam::Vec3::splat(0.0078125));
        let has_data = self.common.maps.data.contains_key(&id);
        let background = if has_data {
            "map/map_background_checkerboard"
        } else {
            "map/map_background"
        };
        if let Some(r) = self.client.gui.atlas.sprites.get(background).copied() {
            for (c, uv) in [
                ([-7.0, 135.0, 0.0], [r.u0, r.v1]),
                ([135.0, 135.0, 0.0], [r.u1, r.v1]),
                ([135.0, -7.0, 0.0], [r.u1, r.v0]),
                ([-7.0, -7.0, 0.0], [r.u0, r.v0]),
            ] {
                out.push(Vertex {
                    normal: [0; 4],
                    pos: m.transform_point3(glam::Vec3::from_array(c)).to_array(),
                    uv,
                    layer: 0,
                    color: 0xFFFF_FFFF,
                    light,

                    glint_uv: [0.0, 0.0],
                });
            }
        }
        if has_data {
            self.map_vertices(id, false, m, light, out);
        }
    }
    /// `ClientboundMapItemDataPacket.handle`: the server's colours and
    /// decorations for a map (one carried or framed on a server).
    pub(in crate::game) fn remote_map_item_data(
        &mut self,
        p: &sunrise_protocol::packets::play::MapItemData,
    ) {
        let decorations = p.decorations.as_ref().map(|list| {
            let types = self
                .client
                .remote
                .as_ref()
                .and_then(|r| r.builtin.get("minecraft:map_decoration_type"));
            list.iter()
                .filter_map(|d| {
                    Some(Decoration {
                        sprite: decoration_sprite(types?.name(d.kind)?)?,
                        x: d.x,
                        y: d.y,
                        rot: (d.rot & 15) as u8,
                        name: d
                            .name
                            .as_ref()
                            .map(|t| sunrise_sim::entity_nbt::name_text(t).plain()),
                        key: Key::Remote,
                    })
                })
                .collect()
        });
        let dimension = self.common.dim.id();
        let map = self
            .common
            .maps
            .data
            .entry(p.map_id)
            .or_insert_with(|| MapData::fresh(0.0, 0.0, p.scale.max(0) as u8, dimension));
        map.locked = p.locked;
        if let Some(decorations) = decorations {
            map.decorations = decorations;
        }
        let Some(patch) = &p.patch else {
            map.dirty = false;
            return;
        };
        for x in 0..patch.width {
            for y in 0..patch.height {
                let Some(&color) = patch.colors.get((x + y * patch.width) as usize) else {
                    continue;
                };
                let (px, py) = (patch.start_x + x, patch.start_y + y);
                if (0..128).contains(&px) && (0..128).contains(&py) {
                    map.update_color(px, py, color);
                }
            }
        }
        // The server owns a remote map: only the texture has to change.
        map.dirty = false;
    }
    /// The atlas slot showing map `id`, its texture uploaded when it changed.
    pub(in crate::game) fn map_texture(&mut self, id: i32) -> Option<Rect> {
        if !self.common.maps.data.contains_key(&id) {
            return None;
        }
        let slot = match self.client.map_slots.show(&id) {
            Some(slot) => slot,
            None => {
                let (slot, _) = self.client.map_slots.claim(id)?;
                if let Some(m) = self.common.maps.data.get_mut(&id) {
                    m.texture_dirty = true;
                }
                slot
            }
        };
        let rect = self
            .client
            .gui
            .atlas
            .sprites
            .get(map_slot_sprite(slot)?)
            .copied()?;
        let m = self.common.maps.data.get_mut(&id)?;
        if m.texture_dirty {
            m.texture_dirty = false;
            let rgba: Vec<u8> = m.colors.iter().flat_map(|&c| packed_rgba(c)).collect();
            let (w, h) = (
                self.client.gui.atlas.width as f32,
                self.client.gui.atlas.height as f32,
            );
            self.client.map_patches.push(sunrise_client::font::Patch {
                x: (rect.u0 * w).round() as u32,
                y: (rect.v0 * h).round() as u32,
                width: 128,
                height: 128,
                rgba,
            });
        }
        Some(rect)
    }
    /// `MapRenderer.render`: the map texture and its decorations under `m`
    /// (map pixels 0..128, y down), drawn at depth offsets toward the viewer.
    /// `show_only_frame` (a map in an item frame) leaves out the decorations that
    /// do not show on frames.
    pub(in crate::game::client_side) fn map_vertices(
        &mut self,
        id: i32,
        show_only_frame: bool,
        m: glam::Mat4,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        let Some(rect) = self.map_texture(id) else {
            return;
        };
        let quad = |corners: [[f32; 3]; 4], uv: [[f32; 2]; 4], out: &mut Vec<Vertex>| {
            for (c, uv) in corners.into_iter().zip(uv) {
                out.push(Vertex {
                    normal: [0; 4],
                    pos: m.transform_point3(glam::Vec3::from_array(c)).to_array(),
                    uv,
                    layer: 0,
                    color: 0xFFFF_FFFF,
                    light,

                    glint_uv: [0.0, 0.0],
                });
            }
        };
        let z = -0.01;
        quad(
            [
                [0.0, 128.0, z],
                [128.0, 128.0, z],
                [128.0, 0.0, z],
                [0.0, 0.0, z],
            ],
            [
                [rect.u0, rect.v1],
                [rect.u1, rect.v1],
                [rect.u1, rect.v0],
                [rect.u0, rect.v0],
            ],
            out,
        );
        let decorations = self
            .common
            .maps
            .data
            .get(&id)
            .map_or(&[][..], |m| &m.decorations);
        let mut count = 0;
        for d in decorations {
            if show_only_frame && !shown_on_frame(d.sprite) {
                continue;
            }
            let Some(s) = self.client.gui.atlas.gui_rect(d.sprite).copied() else {
                continue;
            };
            let dm = decoration_pose(d, -0.02 - count as f32 * 0.001);
            let p = |x: f32, y: f32| dm.transform_point3(glam::Vec3::new(x, y, 0.0)).to_array();
            let mut q = |corners: [[f32; 3]; 4], uv: [[f32; 2]; 4]| {
                for (c, uv) in corners.into_iter().zip(uv) {
                    out.push(Vertex {
                        normal: [0; 4],
                        pos: m.transform_point3(glam::Vec3::from_array(c)).to_array(),
                        uv,
                        layer: 0,
                        color: 0xFFFF_FFFF,
                        light,

                        glint_uv: [0.0, 0.0],
                    });
                }
            };
            q(
                [p(-1.0, 1.0), p(1.0, 1.0), p(1.0, -1.0), p(-1.0, -1.0)],
                [[s.u0, s.v0], [s.u1, s.v0], [s.u1, s.v1], [s.u0, s.v1]],
            );
            if let Some(name) = &d.name {
                let text = sunrise_game_data::text::Text::literal(name);
                let f = self.client.gui.atlas.font.width_text(&text);
                if f > 0.0 {
                    let f1 = (25.0 / f).clamp(0.0, 6.0 / 9.0);
                    let mut text_batch = HudBatch::scratch([1, 1]);
                    let behind = text_batch.name_tag(
                        &self.client.gui.atlas,
                        &text,
                        0.0,
                        0xFFFF_FFFF,
                        0x8000_0000,
                    );
                    self.client.map_patches.append(&mut text_batch.patches);
                    let tm = glam::Mat4::from_translation(glam::Vec3::new(
                        f32::from(d.x) / 2.0 + 64.0,
                        f32::from(d.y) / 2.0 + 64.0 + 4.0,
                        // MapRenderer translates by -0.025, then flips Z and
                        // translates by 0.1 in text space: -0.025 - 0.1.
                        -0.125,
                    )) * glam::Mat4::from_scale(glam::Vec3::new(f1, f1, 1.0));
                    for (i, quad) in text_batch.vertices.as_chunks::<6>().0.iter().enumerate() {
                        let z_shift = if i * 6 < behind { 0.0005 } else { -0.0005 };
                        for v in [quad[5], quad[2], quad[1], quad[0]] {
                            out.push(Vertex {
                                normal: [0; 4],
                                pos: m
                                    .transform_point3(tm.transform_point3(glam::Vec3::new(
                                        v.pos[0], v.pos[1], z_shift,
                                    )))
                                    .to_array(),
                                uv: v.uv,
                                layer: 0,
                                color: v.color,
                                light,
                                glint_uv: [0.0, 0.0],
                            });
                        }
                    }
                }
            }
            count += 1;
        }
        // A native mod's own icons, over the vanilla ones.
        for d in self.mod_map_decorations(id, show_only_frame) {
            let Some(s) = self.client.gui.atlas.gui_rect(d.sprite.as_str()).copied() else {
                continue;
            };
            let dm = decoration_pose_at(d.x, d.y, d.rot, -0.02 - count as f32 * 0.001);
            let p = |x: f32, y: f32| dm.transform_point3(glam::Vec3::new(x, y, 0.0)).to_array();
            let corners = [p(-1.0, 1.0), p(1.0, 1.0), p(1.0, -1.0), p(-1.0, -1.0)];
            let uvs = [[s.u0, s.v0], [s.u1, s.v0], [s.u1, s.v1], [s.u0, s.v1]];
            for (c, uv) in corners.into_iter().zip(uvs) {
                out.push(Vertex {
                    normal: [0; 4],
                    pos: m.transform_point3(glam::Vec3::from_array(c)).to_array(),
                    uv,
                    layer: 0,
                    color: 0xFFFF_FFFF,
                    light,
                    glint_uv: [0.0, 0.0],
                });
            }
            count += 1;
        }
    }
    /// The maps the player carries, for `status`: how many of their pixels
    /// are drawn, and their decorations.
    pub(in crate::game::client_side) fn maps_status(&self) -> Vec<serde_json::Value> {
        self.common
            .inventory
            .iter()
            .chain(std::iter::once(&self.common.offhand))
            .flatten()
            .filter_map(map_id)
            .filter_map(|id| {
                let m = self.common.maps.data.get(&id)?;
                Some(serde_json::json!({
                    "id": id,
                    "pixels": m.colors.iter().filter(|&&c| c != 0).count(),
                    "decorations": m.decorations.iter().map(|d| d.sprite).collect::<Vec<_>>(),
                }))
            })
            .collect()
    }
    /// The icons `native_map_decorations` adds to map `id`, as it is drawn
    /// (`frame`: in an item frame or on a screen, which show only what
    /// `shown_on_frame` allows).
    pub(in crate::game) fn mod_map_decorations(
        &self,
        id: i32,
        frame: bool,
    ) -> Vec<mod_api::MapDecoration> {
        let Some(map) = self.common.maps.data.get(&id) else {
            return Vec::new();
        };
        let info = mod_api::MapInfo {
            id,
            center: map.center,
            scale: map.scale,
            dimension: &map.dimension,
            locked: map.locked,
            in_frame: frame,
        };
        let mut decorations = Vec::new();
        self.native_map_decorations(&info, &mut decorations);
        decorations
            .into_iter()
            .filter(|d| !frame || d.shown_on_frame)
            .map(|d| mod_api::MapDecoration {
                rot: d.rot & 15,
                ..d
            })
            .collect()
    }
    /// `renderTwoHandedMap`: map `id` held in both hands, in view space (the
    /// arms under it: `first_person::map_arm_poses`).
    pub(in crate::game::client_side) fn two_handed_map_geometry(
        &mut self,
        to_world: glam::Mat4,
        attack: f32,
        inverse_height: f32,
        id: i32,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        use sunrise_core::geometry::trig::Mth;
        let deg = |d: f32| d.to_radians();
        let base = to_world
            * sunrise_client::first_person::two_handed_map_base(
                self.common.pitch,
                inverse_height,
                attack,
            );
        let swing = (attack.sqrt() * std::f32::consts::PI).msin();
        let m = base
            * glam::Mat4::from_rotation_x(deg(swing * 20.0))
            * glam::Mat4::from_scale(glam::Vec3::splat(2.0));
        self.held_map(m, id, light, out);
    }
}
