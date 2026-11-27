//! A mod's general entity rows on the window (#792, stage 5): each row the server shows
//! (`game/network/remote/general.rs`), drawn by the renderer a mod registered for its type
//! (`register_entity_renderers`), with the row's shared, frozen synced attachments. A general
//! type has no look of its own: without a renderer, when its renderer returns an `Err` for
//! that draw, or when its renderer has been disabled for the session after a panic, its rows
//! draw nothing.
use super::entity_culling;
use super::native_entity_renderers::EntityRendererTable;
use crate::game::client_side::remote_client::general::RemoteGeneral;
use glam::DVec3;
use std::{sync::Arc, time::Instant};
use sunrise_client::mesher::Vertex;

/// `EntityRenderState` for a general row: its identity, age and pose at the frame's `partial`
/// tick, interpolated from the tick before as `Entity.getYRot(partial)` does, with no family's
/// fields. The frame clones only the attachment generation's Arc; it never consults the live holder.
fn render_state(
    id: u32,
    row: &RemoteGeneral,
    partial: f32,
    at: DVec3,
    camera: DVec3,
    dimensions: (f64, f64),
) -> mod_api::EntityRenderState {
    let lerp = |from: f32, to: f32| from + partial * (to - from);
    let (width, height) = dimensions;
    mod_api::__private::built::EntityRenderState {
        id,
        partial,
        age: row.age as f32 + partial,
        yaw: lerp(row.previous_rotation[0], row.rotation[0]),
        pitch: lerp(row.previous_rotation[1], row.rotation[1]),
        position: at.to_array(),
        size: [width as f32, height as f32],
        eye_height: (height * 0.85) as f32,
        distance_to_camera_sq: (at - camera).length_squared(),
        living: None,
        tnt_fuse: None,
        orb_value: None,
        painting: None,
        item_frame: None,
        attachments: Some(row.render_attachments.clone()),
    }
    .into()
}

pub(in crate::game) struct GeneralDrawFrame<'a> {
    rows: &'a indexmap::map::Slice<i32, RemoteGeneral>,
    renderers: Arc<EntityRendererTable>,
    camera: DVec3,
    alpha: f64,
    view_scale: f64,
    now: Instant,
}

impl crate::game::ClientGame<'_> {
    pub(in crate::game::client_side) fn begin_general_geometry(
        &self,
        camera: DVec3,
        alpha: f64,
    ) -> Option<GeneralDrawFrame<'_>> {
        let rows = self.client.remote.as_ref()?.general.rows.as_slice();
        if rows.is_empty() {
            return None;
        }
        Some(GeneralDrawFrame {
            rows,
            renderers: super::native_entity_renderers::general_renderers()?,
            camera,
            alpha,
            view_scale: self.view_scale(),
            now: Instant::now(),
        })
    }

    pub(in crate::game::client_side) fn general_run_geometry(
        &self,
        frame: &GeneralDrawFrame<'_>,
        range: std::ops::Range<usize>,
        out: &mut Vec<Vertex>,
    ) {
        for (&id, row) in frame.rows[range].iter() {
            if self.hides_as_camera(id as u32) {
                continue;
            }
            self.general_entity_geometry(frame, id as u32, row, out);
        }
    }

    /// Extracts and submits one shown row from the client's immutable frame state, lit at its
    /// interpolated eye (`EntityRenderer.getPackedLightCoords`, at 0.85 of declared height).
    fn general_entity_geometry(
        &self,
        frame: &GeneralDrawFrame<'_>,
        id: u32,
        row: &RemoteGeneral,
        out: &mut Vec<Vertex>,
    ) {
        let Some(resolved) = frame.renderers.general(row.type_slot) else {
            return;
        };
        if !resolved.is_enabled() {
            return;
        }
        let position = DVec3::from_array(row.position);
        let previous = DVec3::from_array(row.previous);
        let at = previous + (position - previous) * frame.alpha;
        let bounds = row.bounds();
        let range =
            entity_culling::default_range(entity_culling::at(bounds.size()), frame.view_scale);
        let offset = at - position;
        if !self.entity_in_view(
            frame.camera,
            position,
            range,
            entity_culling::at(bounds.min_position()) + offset,
            entity_culling::at(bounds.max_position()) + offset,
        ) {
            return;
        }
        let section_block = position.floor().as_ivec3().to_array();
        if self.common.world.shape.contains_y(section_block[1])
            && !self.client.renderer.0.as_ref().is_some_and(|renderer| {
                renderer.entity_section_visible(
                    section_block,
                    self.client.settings.chunk_fade as f32,
                    frame.now,
                )
            })
        {
            return;
        }
        let block = at.floor().as_ivec3().to_array();
        let catalog = sunrise_gameplay_common::game::general::general_type_catalog();
        let dimensions = catalog.dimensions(row.type_slot);
        let (_, height) = dimensions;
        let entity = render_state(id, row, frame.alpha as f32, at, frame.camera, dimensions);
        let eye = (at.y + height * 0.85).floor() as i32;
        let light = self.common.world.light(block[0], eye, block[2]);
        let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
        let key = catalog.key(row.type_slot);
        let start = out.len();
        self.mark_entity(start, key);
        self.mark_motion(start, 0);
        self.mod_entity_renderer_geometry(
            resolved,
            &entity,
            key,
            at - frame.camera,
            Some(at),
            block,
            packed,
            out,
        );
        #[cfg(any(test, feature = "test-support"))]
        super::native_render_pair_tests::witness_body(id, start, out.len());
    }
}

#[cfg(test)]
mod tests {
    use super::render_state;

    /// The frame between two ticks: a quarter of the way, in age, yaw and pitch alike, with no
    /// family's fields.
    #[test]
    fn a_rows_state_is_interpolated_to_the_frame() {
        let mut row = super::RemoteGeneral::new(
            0,
            1,
            [0.0; 3],
            [90.0, 20.0],
            mod_api::EntityRenderAttachments::default(),
        );
        row.previous_rotation = [0.0, -20.0];
        row.age = 40;
        let state = render_state(
            9,
            &row,
            0.25,
            glam::DVec3::new(1.0, 2.0, 3.0),
            glam::DVec3::new(4.0, 6.0, 3.0),
            (0.6, 1.2),
        );
        assert_eq!(state.id, 9);
        assert_eq!(state.age, 40.25);
        assert_eq!((state.yaw, state.pitch), (22.5, -10.0));
        assert_eq!(state.position, [1.0, 2.0, 3.0]);
        assert_eq!(state.size, [0.6, 1.2]);
        assert!((state.eye_height - 1.02).abs() < 1e-6);
        assert_eq!(state.distance_to_camera_sq, 25.0);
        assert!(state.living.is_none() && state.tnt_fuse.is_none() && state.painting.is_none());
    }
}
