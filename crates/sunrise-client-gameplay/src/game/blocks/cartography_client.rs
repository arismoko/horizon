//! Client-side cartography table preview rendering (`CartographyTableScreen`).
use crate::game::ClientGame;
use sunrise_client::hud::HudBatch;
use sunrise_gameplay_common::game::maps::{
    Decoration, decoration_pose, decoration_pose_at, map_id, shown_on_frame,
};

impl ClientGame<'_> {
    /// `CartographyTableScreen.extractBackground` after the background.
    pub(in crate::game) fn draw_cartography(&mut self, batch: &mut HudBatch, ox: f32, oy: f32) {
        let Some((map, extra)) = self
            .common
            .station
            .as_ref()
            .map(|s| (s.inputs[0].clone(), s.inputs[1].clone()))
        else {
            return;
        };
        let extra = extra.map(|e| e.item.to_string()).unwrap_or_default();
        let (duplicate, scaling, locking) = (
            extra == "minecraft:map",
            extra == "minecraft:paper",
            extra == "minecraft:glass_pane",
        );
        let id = map.as_ref().and_then(map_id);
        let state = id.and_then(|id| self.common.maps.data.get(&id).map(|m| (m.locked, m.scale)));
        let mut locked = false;
        let error = "container/cartography_table/error";
        if let Some((is_locked, scale)) = state {
            if is_locked {
                locked = true;
                if scaling || locking {
                    batch.sprite(&self.client.gui.atlas, error, ox + 35.0, oy + 31.0);
                }
            }
            if scaling && scale >= 4 {
                locked = true;
                batch.sprite(&self.client.gui.atlas, error, ox + 35.0, oy + 31.0);
            }
        }
        let sprite = |batch: &mut HudBatch, game: &Self, name: &str, x: f32, y: f32| {
            batch.sprite(
                &game.client.gui.atlas,
                &format!("container/cartography_table/{name}"),
                ox + x,
                oy + y,
            );
        };
        let preview = |batch: &mut HudBatch, game: &mut Self, x: f32, y: f32, scale: f32| {
            if let Some((id, rect)) = id
                .filter(|_| state.is_some())
                .and_then(|id| game.map_texture(id).map(|rect| (id, rect)))
            {
                let size = 128.0 * scale;
                batch.rect_uv(ox + x, oy + y, size, size, rect, u32::MAX);
                game.draw_map_decorations(batch, id, ox + x, oy + y, scale);
            }
        };
        if scaling && !locked {
            sprite(batch, self, "scaled_map", 67.0, 13.0);
            preview(batch, self, 85.0, 31.0, 0.226);
        } else if duplicate {
            sprite(batch, self, "duplicated_map", 83.0, 13.0);
            preview(batch, self, 86.0, 16.0, 0.34);
            sprite(batch, self, "duplicated_map", 67.0, 29.0);
            preview(batch, self, 70.0, 32.0, 0.34);
        } else if locking {
            sprite(batch, self, "map", 67.0, 13.0);
            preview(batch, self, 71.0, 17.0, 0.45);
            sprite(batch, self, "locked", 118.0, 60.0);
        } else {
            sprite(batch, self, "map", 67.0, 13.0);
            preview(batch, self, 71.0, 17.0, 0.45);
        }
    }

    /// `GuiGraphicsExtractor.map`: map `id`'s decorations over its image at
    /// `(x, y)` drawn `scale` GUI pixels per map pixel. Only those with
    /// `renderOnFrame` (`MapDecorationType.showOnItemFrame`) are drawn: the
    /// screen's `map` skips the player arrows and the two markers.
    fn draw_map_decorations(&self, batch: &mut HudBatch, id: i32, x: f32, y: f32, scale: f32) {
        let Some(map) = self.common.maps.data.get(&id) else {
            return;
        };
        for d in map.decorations.iter().filter(|d| shown_on_frame(d.sprite)) {
            let Some(s) = self.client.gui.atlas.gui_rect(d.sprite).copied() else {
                continue;
            };
            batch.quad(
                decoration_corners(d, x, y, scale),
                [[s.u0, s.v0], [s.u1, s.v0], [s.u1, s.v1], [s.u0, s.v1]],
                u32::MAX,
            );
            if let Some(name) = &d.name {
                let text = sunrise_game_data::text::Text::literal(name);
                let f = self.client.gui.atlas.font.width_text(&text);
                if f > 0.0 {
                    let f1 = (25.0 / f).clamp(0.0, 6.0 / 9.0);
                    let mut temp = HudBatch::scratch([1, 1]);
                    temp.name_tag(&self.client.gui.atlas, &text, 0.0, 0xFFFF_FFFF, 0x8000_0000);
                    batch.patches.extend(temp.patches);
                    let tm = glam::Mat4::from_translation(glam::Vec3::new(
                        x + (f32::from(d.x) / 2.0 + 64.0) * scale,
                        y + (f32::from(d.y) / 2.0 + 64.0 + 4.0) * scale,
                        0.0,
                    )) * glam::Mat4::from_scale(glam::Vec3::splat(scale * f1));
                    for quad in temp.vertices.as_chunks::<6>().0 {
                        let corners = [quad[0], quad[1], quad[2], quad[5]].map(|v| {
                            let p = tm.transform_point3(glam::Vec3::new(v.pos[0], v.pos[1], 0.0));
                            [p.x, p.y]
                        });
                        let uvs = [quad[0].uv, quad[1].uv, quad[2].uv, quad[5].uv];
                        batch.quad(corners, uvs, quad[0].color);
                    }
                }
            }
        }
        // A native mod's own icons (`native_map_decorations`).
        for d in self.mod_map_decorations(id, true) {
            let Some(s) = self.client.gui.atlas.gui_rect(d.sprite.as_str()).copied() else {
                continue;
            };
            batch.quad(
                pose_corners(decoration_pose_at(d.x, d.y, d.rot, 0.0), x, y, scale),
                [[s.u0, s.v0], [s.u1, s.v0], [s.u1, s.v1], [s.u0, s.v1]],
                u32::MAX,
            );
        }
    }
}

/// Decoration `d`'s corners on screen, for a map drawn at `(x, y)` with
/// `scale` GUI pixels per map pixel: (-1, 1), (1, 1), (1, -1), (-1, -1)
/// under `MapRenderer.render`'s pose.
fn decoration_corners(d: &Decoration, x: f32, y: f32, scale: f32) -> [[f32; 2]; 4] {
    pose_corners(decoration_pose(d, 0.0), x, y, scale)
}

/// The corners of a decoration drawn under `pose` (`decoration_pose`), like
/// [`decoration_corners`].
fn pose_corners(pose: glam::Mat4, x: f32, y: f32, scale: f32) -> [[f32; 2]; 4] {
    let m = glam::Mat4::from_translation(glam::Vec3::new(x, y, 0.0))
        * glam::Mat4::from_scale(glam::Vec3::new(scale, scale, 1.0))
        * pose;
    [(-1.0, 1.0), (1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)].map(|(cx, cy)| {
        let p = m.transform_point3(glam::Vec3::new(cx, cy, 0.0));
        [p.x, p.y]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_gameplay_common::game::maps::{Decoration, Key};

    fn close(a: [[f32; 2]; 4], b: [[f32; 2]; 4]) {
        for (a, b) in a.iter().zip(&b) {
            assert!(
                (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4,
                "{a:?} {b:?}"
            );
        }
    }

    #[test]
    fn the_preview_skips_what_an_item_frame_skips() {
        // GuiGraphicsExtractor.map: `if (decoration.renderOnFrame)`.
        assert!(!shown_on_frame("map/decorations/player"));
        assert!(!shown_on_frame("map/decorations/red_marker"));
        assert!(shown_on_frame("map/decorations/frame"));
        assert!(shown_on_frame("map/decorations/red_x"));
    }

    #[test]
    fn map_markers_sit_on_the_preview_like_on_a_held_map() {
        let player = |x, y, rot| Decoration {
            sprite: "map/decorations/player",
            x,
            y,
            rot,
            name: None,
            key: Key::Remote,
        };
        // The centre, facing north: 8 map pixels wide, nudged by (-0.5, 0.5).
        close(
            decoration_corners(&player(0, 0, 0), 0.0, 0.0, 1.0),
            [[59.5, 68.5], [67.5, 68.5], [67.5, 60.5], [59.5, 60.5]],
        );
        // A quarter turn (rotation 4 of 16) turns the quad about its marker.
        close(
            decoration_corners(&player(0, 0, 4), 0.0, 0.0, 1.0),
            [[59.5, 59.5], [59.5, 67.5], [67.5, 67.5], [67.5, 59.5]],
        );
        // The preview's offset and scale apply after: the top-left corner
        // marker on the 0.45 preview at (71, 17).
        close(
            decoration_corners(&player(-128, -128, 0), 71.0, 17.0, 0.45),
            [
                [71.0 - 4.5 * 0.45, 17.0 + 4.5 * 0.45],
                [71.0 + 3.5 * 0.45, 17.0 + 4.5 * 0.45],
                [71.0 + 3.5 * 0.45, 17.0 - 3.5 * 0.45],
                [71.0 - 4.5 * 0.45, 17.0 - 3.5 * 0.45],
            ],
        );
    }
}
