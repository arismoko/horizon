//! Entity mesh occlusion and view-distance checks.
#[cfg(feature = "client")]
use super::particle_culling::section_scene;
use super::{
    Vertex,
    entity_client::{RenderEntity, RenderEntityBounds},
    occlusion_culling::{AsyncOcclusionCuller, opaque},
};
use sunrise_client::graphics::EntityCullingMode;
use sunrise_client::voxel_occlusion::Culler as OcclusionCuller;
pub use sunrise_client_gameplay::entity_occlusion::{EntityCullDecision, at};
pub(in crate::game) use sunrise_client_gameplay::entity_occlusion::{
    EntityOcclusion, cull_box, default_range, entity_view_scale, orb_in_view,
};
use sunrise_client_gameplay::occlusion_culling::QuerySource;
use sunrise_core::geometry::{Aabb, Vec3};

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub fn prepare_entity_culling(&mut self) {
        // The mods' render hooks: whether any listens this frame, and no entity canceled yet.
        super::native_mob_draw::begin_entity_frame();
        self.extract_local_player_vehicle();
        // Spans tell the pack's passes which entities are hidden; without them, nothing changes.
        self.client.occlusion.0.shadow_casters =
            match (&self.client.renderer.0, &self.client.pack_ids) {
                (Some(r), Some(_)) => r.pack_shadow_casters(),
                _ => (false, false),
            };
    }

    pub(in crate::game::client_side) fn cull_entity_vertices(
        &self,
        vertices: &mut Vec<Vertex>,
        start: usize,
    ) -> bool {
        let hidden = self.occlusion_hidden(Some(vertices), start, None);
        if hidden
            && !(self.client.occlusion.0.shadow_casters.0
                && self.hide_spans_from(start, vertices.len()))
        {
            if self.client.settings.sunrise.entity_culling == EntityCullingMode::Asynchronous
                && let Some(culler) = &self.client.occlusion.0.async_culler
            {
                culler.record_entity_vertices_culled(vertices.len().saturating_sub(start));
            }
            vertices.truncate(start);
        }
        hidden
    }

    pub(in crate::game::client_side) fn cull_entity_camera_bounds(
        &self,
        lo: glam::Vec3,
        hi: glam::Vec3,
    ) -> bool {
        if self.client.occlusion.0.occlusion_hidden_camera_bounds(
            &section_scene!(self),
            self.client.settings.sunrise.entity_culling,
            lo,
            hi,
        ) {
            if self.client.settings.sunrise.entity_culling == EntityCullingMode::Asynchronous
                && let Some(culler) = &self.client.occlusion.0.async_culler
            {
                culler.record_entity_vertices_culled(2);
            }
            return true;
        }
        false
    }

    /// Pre-model visibility hook for dynamic block-entity geometry.
    pub(in crate::game::client_side) fn block_entity_occluded(&self, bounds: Aabb) -> bool {
        self.occlusion_hidden(None, 0, Some(bounds))
    }

    /// Cull from the shared entity AABB before generating its model vertices.
    pub(in crate::game) fn entity_cull_decision<T: RenderEntityBounds>(
        &self,
        entity: &T,
        alpha: f64,
    ) -> EntityCullDecision {
        self.entity_cull_decision_in_bounds(entity, entity.render_bounds(alpha))
    }

    /// Cull from a renderer-computed envelope when the rendered transform has
    /// extra offsets or model geometry that the entity type alone cannot see.
    pub(in crate::game::client_side) fn entity_cull_decision_in_bounds<T: RenderEntity>(
        &self,
        entity: &T,
        render_bounds: Aabb,
    ) -> EntityCullDecision {
        if self.client.settings.sunrise.entity_culling == EntityCullingMode::Off
            || self.client.occlusion.0.shadow_casters.0
            || entity.render_glowing(self)
        {
            return EntityCullDecision::Bypass;
        }
        // `render_bounds` is already the conservative model and animation
        // envelope. Only cover f32 vertex rounding here; another half-block
        // would make coarse walls falsely visible and force mesh generation.
        let margin = Vec3::new(1.0e-4, 1.0e-4, 1.0e-4);
        let bounds = render_bounds.inflate(margin);
        let scene = section_scene!(self);
        if self
            .client
            .occlusion
            .0
            .terrain_sections_occlude(&scene, bounds)
        {
            if let Some(culler) = &self.client.occlusion.0.async_culler {
                culler.record_entity_culled();
            }
            return EntityCullDecision::Hidden;
        }
        match self.client.settings.sunrise.entity_culling {
            EntityCullingMode::Off => EntityCullDecision::Bypass,
            EntityCullingMode::Synchronous => {
                if self
                    .client
                    .occlusion
                    .0
                    .entity_hidden_sync(&scene, None, 0, Some(bounds))
                {
                    EntityCullDecision::Hidden
                } else {
                    EntityCullDecision::CheckVertices
                }
            }
            EntityCullingMode::Asynchronous => {
                let Some(culler) = &self.client.occlusion.0.async_culler else {
                    return if self.client.occlusion.0.entity_hidden_sync(
                        &scene,
                        None,
                        0,
                        Some(bounds),
                    ) {
                        EntityCullDecision::Hidden
                    } else {
                        EntityCullDecision::CheckVertices
                    };
                };
                if !culler.camera_is_stationary() {
                    return EntityCullDecision::CheckVertices;
                }
                let Some(query) = culler.query_entity_bounds(bounds) else {
                    return EntityCullDecision::CheckVertices;
                };
                culler.sample_entity_aabb(entity.render_id(), &query);
                match culler.query_result(&query, QuerySource::EntityAabb) {
                    Some((false, _)) => EntityCullDecision::CheckVertices,
                    Some((true, Some(proof)))
                        if culler.proof_is_current(
                            scene.world,
                            scene.renderer,
                            scene.streaming,
                            scene.pending_uploads,
                            proof,
                        ) =>
                    {
                        culler.record_entity_culled();
                        EntityCullDecision::Hidden
                    }
                    Some(_) | None => {
                        culler.queue(query);
                        EntityCullDecision::Pending
                    }
                }
            }
        }
    }

    /// Refine an AABB-visible result with the exact bounds of the rendered mesh.
    pub(in crate::game) fn finish_entity_occlusion<T: RenderEntity>(
        &self,
        decision: EntityCullDecision,
        entity: &T,
        vertices: &mut Vec<Vertex>,
        start: usize,
    ) {
        if decision.checks_vertices()
            && !self.client.occlusion.0.shadow_casters.0
            && !entity.render_glowing(self)
        {
            self.cull_entity_vertices(vertices, start);
        }
    }

    fn occlusion_hidden(
        &self,
        vertices: Option<&[Vertex]>,
        start: usize,
        bounds: Option<Aabb>,
    ) -> bool {
        self.client.occlusion.0.occlusion_hidden(
            &section_scene!(self),
            self.client.settings.sunrise.entity_culling,
            vertices,
            start,
            bounds,
        )
    }
}

/// `Options.getEffectiveRenderDistance`: client setting capped by positive server view distance.
pub(in crate::game::client_side) fn effective_render_distance_for(
    client_render_distance: i32,
    server_view_distance: Option<i32>,
) -> i32 {
    match server_view_distance {
        Some(server) if server > 0 => client_render_distance.min(server),
        _ => client_render_distance,
    }
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Options.getEffectiveRenderDistance: the client setting, capped by the server's view distance.
    pub fn effective_render_distance(&self) -> i32 {
        effective_render_distance_for(
            self.client.settings.render_distance,
            self.client.remote.as_ref().map(|r| r.view_distance),
        )
    }
    /// Entity.setViewScale: entity distance times the render distance's boost (1 at 8 chunks, 2.5 at 20+).
    pub(in crate::game) fn view_scale(&self) -> f64 {
        entity_view_scale(
            self.effective_render_distance(),
            self.client.settings.entity_distance,
        )
    }
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `LevelRenderer.isSectionCompiledAndVisible(BlockPos)`: entities outside build height
    /// always render; inside build height they wait for a compiled section past the fade gate.
    pub(in crate::game::client_side) fn entity_section_visible(
        &self,
        position: glam::DVec3,
        now: std::time::Instant,
    ) -> bool {
        let block = position.floor().as_ivec3().to_array();
        if !self.common.world.shape.contains_y(block[1]) {
            return true;
        }
        self.client.renderer.0.as_ref().is_none_or(|renderer| {
            renderer.entity_section_visible(block, self.client.settings.chunk_fade as f32, now)
        })
    }
    /// EntityRenderer.shouldRender: the entity within `range` blocks of the
    /// camera, measured from its current `position`, and its culling box
    /// (`lo`..`hi`, interpolated, in world space) grown by 0.5 in the view.
    pub(in crate::game::client_side) fn entity_in_view(
        &self,
        camera: glam::DVec3,
        position: glam::DVec3,
        range: f64,
        lo: glam::DVec3,
        hi: glam::DVec3,
    ) -> bool {
        if (position - camera).length_squared() >= range * range {
            return false;
        }
        self.client.view_frustum.as_ref().is_none_or(|view| {
            view.intersects(
                (lo - camera - glam::DVec3::splat(0.5)).as_vec3(),
                (hi - camera + glam::DVec3::splat(0.5)).as_vec3(),
            )
        })
    }
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `entity_in_view` for an entity whose box stands on its position,
    /// `width` wide and `height` tall, interpolated by `alpha` and grown by
    /// `grow` (a renderer's getBoundingBoxForCulling).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game) fn standing_entity_in_view(
        &self,
        camera: glam::DVec3,
        previous: glam::DVec3,
        position: glam::DVec3,
        alpha: f64,
        width: f64,
        height: f64,
        range: f64,
        grow: f64,
    ) -> bool {
        let p = previous.lerp(position, alpha);
        let half = glam::DVec3::new(width / 2.0 + grow, grow, width / 2.0 + grow);
        let lo = p - half;
        let hi = p + glam::DVec3::new(half.x, height + grow, half.z);
        self.entity_in_view(camera, position, range, lo, hi)
    }
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(in crate::game::client_side) fn prepare_occlusion_culling(&mut self, camera: glam::DVec3) {
        self.client.occlusion.0.section_camera = None;
        self.client.occlusion.0.section_visibility_valid = false;
        let entity_mode = self.client.settings.sunrise.entity_culling;
        let requested = entity_mode != EntityCullingMode::Off;
        let eligible = requested
            // Packs can replace opaque models, so their terrain is not a safe occluder.
            && self.model_packs_are_vanilla()
            && self.common.player.is_some()
            && !self.common.spectator();
        if !eligible {
            self.client.occlusion.0.sync_culler = None;
            self.client.occlusion.0.async_culler = None;
            self.client.occlusion.0.async_failed = false;
            return;
        }
        let at = camera.to_array().map(sunrise_core::mth::floor);
        self.client.occlusion.0.section_camera = Some((
            at[0] >> 4,
            (at[1] >> 4) - (self.common.world.bottom_y() >> 4),
            at[2] >> 4,
        ));
        let camera_facts = self
            .common
            .res
            .facts()
            .get(self.common.world.block(at[0], at[1], at[2]));
        if opaque(camera_facts) {
            self.client.occlusion.0.sync_culler = None;
            self.client.occlusion.0.async_culler = None;
            self.client.occlusion.0.async_failed = false;
            return;
        }
        self.client.occlusion.0.section_visibility_valid =
            self.client.settings.sunrise.cave_culling
                && !camera_facts.solid_render
                && self.client.occlusion.0.section_camera.is_some();

        let wants_async = entity_mode == EntityCullingMode::Asynchronous;
        let wants_sync = entity_mode == EntityCullingMode::Synchronous;
        if wants_async {
            if self.client.occlusion.0.async_culler.is_none()
                && !self.client.occlusion.0.async_failed
            {
                match AsyncOcclusionCuller::new() {
                    Ok(culler) => self.client.occlusion.0.async_culler = Some(Box::new(culler)),
                    Err(error) => {
                        log::warn!("Sunrise: cannot start world-occlusion-culling worker: {error}");
                        self.client.occlusion.0.async_failed = true;
                    }
                }
            }
            if let Some(culler) = &mut self.client.occlusion.0.async_culler {
                culler.begin_frame(camera, &self.common.world);
            }
        } else {
            self.client.occlusion.0.async_culler = None;
            self.client.occlusion.0.async_failed = false;
        }

        let async_unavailable = wants_async && self.client.occlusion.0.async_culler.is_none();
        self.client.occlusion.0.sync_culler = (wants_sync || async_unavailable)
            .then(|| OcclusionCuller::new(camera.to_array(), 8_192));
    }

    pub(in crate::game::client_side) fn finish_async_occlusion_culling(&mut self) {
        if let Some(culler) = &mut self.client.occlusion.0.async_culler {
            culler.finish_frame(&self.common.world, self.common.res.facts());
        }
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
mod precheck_envelope_tests {
    use super::super::entity_client::mob_render_bounds;
    use std::{path::Path, sync::Arc};
    use sunrise_client::mobs::render;
    use sunrise_client::mobs_render::{ClientMobData, ClientMobModels, ClientMobRender};
    use sunrise_core::geometry::Vec3;
    use sunrise_sim::mobs::{Mob, MobTypes, Species};

    #[test]
    fn vanilla_models_and_animations_stay_inside_the_precheck_envelope() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/sunrise-client/26.3");
        if !root.join("entity-models.json").exists() {
            eprintln!("no prepared resources: skipped model coverage");
            return;
        }
        let types = MobTypes::load(&root).unwrap();
        let models = ClientMobModels::load(&root).unwrap();
        let data = ClientMobData {
            animations: ClientMobData::load_animations(&root).unwrap(),
            ..Default::default()
        };
        let source = ClientMobRender {
            models: &models,
            data: &data,
        };
        let camera = glam::DVec3::new(3.0, 62.0, -4.0);
        let mut checked = 0;
        for kind in types.types.values() {
            for sample in 0..12 {
                let mut mob = Mob::new(1, Arc::new(kind.clone()), Vec3::new(1.5, 64.0, 3.25), 37.0);
                mob.previous = Vec3::new(1.25, 64.25, 3.0);
                mob.age = sample * 37;
                mob.limb_speed = 0.8;
                mob.limb_position = sample as f32 * 1.7;
                mob.head_pitch = if sample % 2 == 0 { 85.0 } else { -85.0 };
                mob.extra.age = if sample >= 6 { -100 } else { 0 };
                mob.death_time = if sample % 3 == 0 { 10 } else { 0 };
                mob.extra.flag = sample % 2 == 0;
                mob.extra.size = match mob.species() {
                    Species::Slime
                    | Species::MagmaCube
                    | Species::SulfurCube
                    | Species::Phantom => {
                        if sample % 2 == 0 {
                            1
                        } else {
                            4
                        }
                    }
                    _ => mob.extra.size,
                };
                if sample % 3 == 1 {
                    mob.custom_name = Some("Dinnerbone".into());
                }
                for partial in [0.0, 0.4, 1.0] {
                    let bounds = mob_render_bounds(&mob, &models, f64::from(partial))
                        .unwrap_or_else(|| panic!("missing visual for {}", kind.id));
                    let lo = bounds.min_position();
                    let hi = bounds.max_position();
                    let lo = [lo.x, lo.y, lo.z];
                    let hi = [hi.x, hi.y, hi.z];
                    let mut vertices = Vec::new();
                    render::render_with(
                        &mob,
                        partial,
                        camera,
                        &|_| Some([0.0, 0.0, 1.0, 1.0]),
                        0xF0F0,
                        &source,
                        &[],
                        &mut vertices,
                    );
                    for vertex in &vertices {
                        for axis in 0..3 {
                            let world = f64::from(vertex.pos[axis]) + camera[axis];
                            assert!(
                                world >= lo[axis] - 1.0e-4 && world <= hi[axis] + 1.0e-4,
                                "{} sample {sample} partial {partial}: {world} outside {lo:?}..{hi:?}",
                                kind.id,
                            );
                        }
                    }
                    checked += usize::from(!vertices.is_empty());
                }
            }
        }
        assert!(
            checked > 1_000,
            "insufficient real model coverage: {checked}"
        );
    }

    #[test]
    fn effective_render_distance_tracks_login_radius_updates_and_local_settings() {
        use super::effective_render_distance_for;
        use sunrise_gameplay_common::game::streaming_radius::streaming_radius_for;

        // Login with client=12, server chunk_radius=6: effective is 6, streaming keeps +1 buffer (7).
        assert_eq!(effective_render_distance_for(12, Some(6)), 6);
        assert_eq!(streaming_radius_for(12, Some(6)), 7);

        // Live SetChunkCacheRadius changes: lower to 4, raise to 10, zero falls back to client.
        assert_eq!(effective_render_distance_for(12, Some(4)), 4);
        assert_eq!(streaming_radius_for(12, Some(4)), 5);
        assert_eq!(effective_render_distance_for(12, Some(10)), 10);
        assert_eq!(effective_render_distance_for(12, Some(0)), 12);
        assert_eq!(streaming_radius_for(12, Some(0)), 12);

        // Local settings changes while connected to server=6: lowering wins, raising stays capped.
        assert_eq!(effective_render_distance_for(3, Some(6)), 3);
        assert_eq!(streaming_radius_for(3, Some(6)), 3);
        assert_eq!(effective_render_distance_for(16, Some(6)), 6);
        assert_eq!(streaming_radius_for(16, Some(6)), 7);

        // Without a remote server, effective and streaming match local client render distance.
        assert_eq!(effective_render_distance_for(12, None), 12);
        assert_eq!(streaming_radius_for(12, None), 12);
    }
}
