//! One section meshed on a pool worker: the `Job::Mesh` of the player's level and a viewed
//! level's mesh job (`viewed_meshing`) run the same mesher with the same settings.
use super::{MeshSettings, MeshStreamer, WorkerContext};
use crate::mesher::{
    AttachedTintScratch, MeshScratch, MeshTables, Mesher, Region, SectionMesh, borrow,
};
use std::sync::Arc;
use sunrise_world::block_render::FaceVisibility;

/// What a worker keeps between its mesh jobs: the face visibility table and its occlusion cache
/// (like vanilla's per-thread cache: cloning it per job copied every state and face shape), for
/// the tables it was made from.
pub(super) type Faces = Option<(Arc<MeshTables>, FaceVisibility)>;

/// Section `sy` of `region`'s centre column, into `recycled`'s buffers.
pub(super) fn mesh_section(
    context: &WorkerContext,
    settings: &MeshSettings,
    faces: &mut Faces,
    scratch: &mut MeshScratch,
    region: &Region,
    sy: i32,
    recycled: SectionMesh,
) -> SectionMesh {
    let visibility = match faces.take() {
        Some((tables, visibility)) if Arc::ptr_eq(&tables, &context.mesh.tables) => visibility,
        _ => context.mesh.tables.visibility.clone(),
    };
    let mut mesher = section_mesher(context, settings, visibility);
    let region = borrow(region);
    scratch.attached_tints = settings
        .attached_tints
        .as_ref()
        .map(|(_, colors)| colors.clone());
    let mut mesh = if let Some(prepared) = &settings.visibility {
        mesher.mesh_planned(
            &region,
            sy,
            recycled,
            scratch,
            prepared.plan.as_ref(),
            super::section_visibility::REQUEST_BYTES,
        )
    } else {
        mesher.mesh_reusing(&region, sy, recycled, scratch)
    };
    mesh.sort_solid();
    *faces = Some((context.mesh.tables.clone(), mesher.visibility));
    mesh
}

fn section_mesher<'a>(
    context: &'a WorkerContext,
    settings: &'a MeshSettings,
    visibility: FaceVisibility,
) -> Mesher<'a> {
    Mesher {
        nether_shading: settings.nether,
        shader_pack_lighting: settings.shader_pack_lighting,
        separate_ao: settings.separate_ao,
        shader_pack: settings.shader_pack,
        zoom: settings.zoom,
        blend_radius: settings.blend_radius,
        smooth_lighting: settings.smooth_lighting,
        cutout_leaves: settings.cutout_leaves,
        opaque_ice: settings.opaque_ice,
        translucent_layers: &settings.translucent_layers,
        layer_overrides: &settings.layer_overrides,
        leaf_culling: settings.leaf_culling,
        hidden_states: &context.mesh.hidden_states,
        part_tables: &context.mesh.part_tables,
        assets: &context.chunk.assets,
        render: &context.mesh.render,
        tables: &context.mesh.tables,
        biomes: &context.chunk.biomes,
        visibility,
    }
}

/// Buffers and face-visibility cache used only for attachment tint preparation.
#[derive(Default)]
pub(super) struct TintPreparation {
    buffers: AttachedTintScratch,
    faces: Faces,
}

impl TintPreparation {
    pub(super) fn prepare(
        &mut self,
        context: &Arc<WorkerContext>,
        settings: &mut MeshSettings,
        region: &Region,
        sy: i32,
    ) {
        if !context.mesh.tables.has_custom_tint_sources()
            || !region[1][1].as_ref().is_some_and(|chunk| {
                chunk
                    .extra_models
                    .keys()
                    .any(|cell| (cell >> 12) as i32 == sy)
            })
        {
            return;
        }
        let plan = settings.visibility.as_ref().and_then(|v| v.plan.as_ref());
        if plan.is_some_and(|plan| plan.hides_terrain()) {
            return;
        }
        let visibility = match self.faces.take() {
            Some((tables, visibility)) if Arc::ptr_eq(&tables, &context.mesh.tables) => visibility,
            _ => context.mesh.tables.visibility.clone(),
        };
        let mut mesher = section_mesher(context, settings, visibility);
        let colors = mesher.prepare_attached_tints(
            &borrow(region),
            sy,
            settings
                .visibility
                .as_ref()
                .map(|_| super::section_visibility::REQUEST_BYTES),
            plan,
            &mut self.buffers,
        );
        self.faces = Some((context.mesh.tables.clone(), mesher.visibility));
        settings.attached_tints = colors.map(|colors| (context.clone(), colors));
    }
}

impl MeshStreamer {
    /// The settings the player's sections mesh with now.
    pub(super) fn mesh_settings(&self) -> MeshSettings {
        MeshSettings {
            nether: self.nether_shading,
            shader_pack_lighting: self.shader_pack_lighting,
            separate_ao: self.separate_ao,
            shader_pack: self.shader_pack,
            zoom: self.chunks.biome_zoom,
            blend_radius: self.biome_blend,
            smooth_lighting: self.smooth_lighting,
            cutout_leaves: self.cutout_leaves,
            opaque_ice: self.opaque_ice,
            translucent_layers: self.translucent_layers.clone(),
            layer_overrides: self.layer_overrides.clone(),
            leaf_culling: self.leaf_culling,
            visibility: None,
            attached_tints: None,
        }
    }
}
