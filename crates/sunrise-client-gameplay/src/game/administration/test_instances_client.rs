use crate::game::ClientGame;
use sunrise_client::{block_entity_render::vertex_light, hud::HudBatch, mesher::Vertex};
use sunrise_client_render::block_entity_reach::Reach;
use sunrise_game_data::{component::name_text, text::Text};
use sunrise_level::containers::BlockEntity;

/// `GameTestBlockHighlightRenderer.highlightPos`'s colour, 1610678016: ARGB 0x6000FF00.
const HIGHLIGHT_FILL: u32 = 0x6000_FF00;

impl ClientGame<'_> {
    /// `ClientPacketListener.handleGameTestHighlightPos`:
    /// `GameTestBlockHighlightRenderer.highlightPos` shows the block for ten
    /// seconds with its position relative to the test.
    pub(in crate::game) fn highlight_test_pos(&mut self, absolute: [i32; 3], relative: [i32; 3]) {
        let text = format!("{}, {}, {}", relative[0], relative[1], relative[2]);
        let until = std::time::Instant::now() + std::time::Duration::from_millis(10_000);
        self.client
            .test_highlights
            .insert((absolute[0], absolute[1], absolute[2]), (text, until));
    }
    /// `TestInstanceRenderer.submit`'s error markers, for any player within
    /// the beacon's view distance of the block: each marked block in a
    /// translucent red box (`out`, blending) and its text as a billboard over
    /// everything (`see_through`).
    pub(in crate::game::client_side) fn test_instance_marker_geometry(
        &mut self,
        camera: glam::DVec3,
        out: &mut Vec<Vertex>,
        see_through: &mut super::text_displays::SeeThroughCollector,
    ) {
        use sunrise_client::gizmos as g;
        // `GameTestBlockHighlightRenderer.emitGizmos`: expired highlights go,
        // the rest are a green box with the relative position over it.
        let now = std::time::Instant::now();
        self.client
            .test_highlights
            .retain(|_, (_, until)| now <= *until);
        let mut markers: Vec<(_, _, u32)> = self
            .client
            .test_highlights
            .iter()
            .map(|(&pos, (text, _))| ([pos.0, pos.1, pos.2], text.clone(), HIGHLIGHT_FILL))
            .collect();
        let render_distance = self.effective_render_distance();
        // Test instance blocks show a box: `boxes` lists them without a scan of every block entity.
        for (pos, entity) in self.common.world.block_entities.boxes() {
            let BlockEntity::TestInstanceBlock(b) = entity else {
                continue;
            };
            if !Reach::TestInstance.should_render([pos.0, pos.1, pos.2], camera, render_distance) {
                continue;
            }
            for marker in &b.error_markers {
                markers.push((
                    marker.pos,
                    name_text(&marker.text).plain(),
                    g::ERROR_MARKER_FILL,
                ));
            }
        }
        if markers.is_empty() {
            return;
        }
        let light = vertex_light(0xFF);
        self.mark_translucent_entity(out.len(), "minecraft:test_instance_block");
        let mut patches = Vec::new();
        for (pos, text, color) in markers {
            // A plain colour: the atlas's white pixel at the fill's colour.
            let mut fill = HudBatch::scratch([1, 1]);
            fill.fill(&self.client.gui.atlas, 0.0, 0.0, 1.0, 1.0, color);
            let Some(v) = fill.vertices.first().copied() else {
                continue;
            };
            let (min, max, text_at) = g::error_marker_shape(pos);
            for quad in g::cuboid_fill_quads(min, max) {
                for corner in quad {
                    out.push(Vertex {
                        normal: [0; 4],
                        pos: (glam::DVec3::from_array(corner) - camera)
                            .as_vec3()
                            .to_array(),
                        uv: v.uv,
                        layer: 0,
                        color: v.color,
                        light,
                        glint_uv: [0.0, 0.0],
                    });
                }
            }
            // `Gizmos.billboardText(..).setAlwaysOnTop()`: white, centred,
            // without a background.
            let o = glam::DVec3::from_array(text_at) - camera;
            let st_start = see_through.start();
            patches.extend(self.name_tag_quads(
                o,
                &[(Text::literal(text), 0.0)],
                0xFF,
                false,
                2,
                0,
                g::billboard_text_unit(g::ERROR_MARKER_TEXT_SCALE),
                see_through.buffer_mut(),
            ));
            let dist_sq =
                super::text_displays::submit_distance_sq(o.x as f32, o.y as f32, o.z as f32);
            see_through.finish(st_start, dist_sq);
        }
        self.client.map_patches.extend(patches);
    }
}
