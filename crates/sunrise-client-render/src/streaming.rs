//! Client render streaming: section meshing and visibility preparation layered over
//! [`sunrise_level::chunk_streamer::ChunkStreamer`], sharing one pool registration, priority
//! queue, and completion mailbox.
use crate::fast_hash::{FxHashMap, FxHashSet};
use crate::{
    chunk::Chunk,
    graphics::LeafCullingMode,
    mesher::{MeshScratch, MeshTables, Region, SectionMesh},
    world::World,
};
use std::collections::HashSet;
use std::sync::{Arc, Mutex, RwLock};
use sunrise_core::biome::BiomeZoom;
use sunrise_level::chunk_streamer::{
    AuxiliaryJobs, ChunkCommit, ChunkPlanResume, ChunkPlanStep, ChunkUpdate,
    dirty_visibility_columns, nearest_first,
};

pub use sunrise_level::chunk_streamer::{
    Band, ChunkGenerator, ChunkState, ChunkStreamer, ChunkWorkerContext, FinalWrites, FullColumn,
    IDLE_RELEASE, LodDetail, LodSampler, LodSource, Proto, ProtoCensus, ProtoSave, QueueCensus,
    SaveStep, ViewArrival, chunk_pos_is_valid, full_column_ready, in_view_distance, job_stats,
    take_resent_column,
};

/// Render-side resources needed by section meshing workers.
#[derive(Clone)]
pub struct MeshWorkerContext {
    pub render: Arc<crate::assets::RenderAssets>,
    pub tables: Arc<MeshTables>,
    /// The block states a mod hides from the mesher, by state id (`Mesher::hidden_states`); empty
    /// for none.
    pub hidden_states: Arc<[bool]>,
    /// The client `BlockPart` answers the mesher reads (`Mesher::part_tables`); empty for none.
    pub part_tables: Arc<crate::mesher::part_tables::PartTables>,
}

/// Combined chunk and mesh worker resources.
#[derive(Clone)]
pub struct WorkerContext {
    pub chunk: ChunkWorkerContext,
    pub mesh: MeshWorkerContext,
}
impl WorkerContext {
    pub fn chunk_context(&self) -> ChunkWorkerContext {
        self.chunk.clone()
    }
    pub fn mesh_context(&self) -> MeshWorkerContext {
        self.mesh.clone()
    }
    pub fn from_parts(chunk: ChunkWorkerContext, mesh: MeshWorkerContext) -> Self {
        Self { chunk, mesh }
    }
}

/// Per-job mesher inputs that change with the dimension or the options.
#[derive(Clone)]
pub struct MeshSettings {
    pub(super) nether: bool,
    pub(super) shader_pack_lighting: bool,
    pub(super) separate_ao: bool,
    pub(super) shader_pack: bool,
    pub(super) zoom: BiomeZoom,
    pub(super) blend_radius: i32,
    pub(super) smooth_lighting: bool,
    pub(super) cutout_leaves: bool,
    pub(super) opaque_ice: bool,
    pub(super) translucent_layers: Arc<[bool]>,
    pub(super) layer_overrides: Arc<[Option<sunrise_shaders::RenderLayer>]>,
    pub(super) leaf_culling: LeafCullingMode,
    /// The section's frozen `section_visibility` plan; `None` on the no-mod path.
    pub(super) visibility: Option<Arc<section_visibility::FrozenPlan>>,
    /// The colours mods' tint sources gave the section's extra models, asked when the job was
    /// scheduled (`Mesher::prepare_attached_tints`), with the resources used to prepare them.
    pub(super) attached_tints: Option<(Arc<WorkerContext>, Arc<crate::mesher::AttachedTints>)>,
}

/// Opaque inline mesh-worker job payload stored directly in the shared priority queue.
pub struct MeshJob(MeshJobKind);

enum MeshJobKind {
    Mesh(i32, i32, i32, Arc<Region>, u64, MeshSettings),
    Free(Vec<MeshUpload>),
}

impl MeshJob {
    pub(super) fn mesh(
        x: i32,
        sy: i32,
        z: i32,
        region: Arc<Region>,
        seq: u64,
        settings: MeshSettings,
    ) -> Self {
        Self(MeshJobKind::Mesh(x, sy, z, region, seq, settings))
    }
    pub(super) fn free(meshes: Vec<MeshUpload>) -> Self {
        Self(MeshJobKind::Free(meshes))
    }
}

/// A completed mesh and the compile sequence that produced it, kept through GPU publication.
pub struct MeshUpload {
    pub key: (i32, i32, i32),
    pub mesh: SectionMesh,
    seq: u64,
    halo: Option<Arc<section_visibility::HaloAnswers>>,
}

/// Render-side completion marker.
pub enum Done {
    Meshed(MeshUpload),
}

struct MeshShared {
    /// What meshing reads (block models and sprites), swapped when resource
    /// packs reload; each mesh job takes the current one.
    context: RwLock<Arc<WorkerContext>>,
    /// Only workers use this pool; frames never wait for its lock. Retaining
    /// cleared buffers avoids racing the renderer for the process heap.
    meshes: Mutex<MeshPool>,
}

#[derive(Default)]
struct MeshPool {
    meshes: Vec<SectionMesh>,
    bytes: usize,
}
impl MeshPool {
    const BYTES: usize = 16 << 20;
    const COUNT: usize = 128;
    fn take(&mut self) -> SectionMesh {
        let mesh = self.meshes.pop().unwrap_or_default();
        self.bytes -= mesh.capacity_bytes();
        mesh
    }
    /// A rejected mesh is dropped by the caller, after releasing the lock.
    fn put(&mut self, mut mesh: SectionMesh) -> Option<SectionMesh> {
        let bytes = mesh.capacity_bytes();
        if bytes == 0 || self.meshes.len() == Self::COUNT || bytes > Self::BYTES - self.bytes {
            return Some(mesh);
        }
        mesh.clear();
        self.bytes += bytes;
        self.meshes.push(mesh);
        None
    }
}

/// Opaque per-worker meshing cache and scratch retained between jobs on each pool worker.
pub struct MeshWorkerLocal {
    faces: Faces,
    mesh_scratch: MeshScratch,
}

/// Companion job executor running section meshing and mesh buffer recycling on `ChunkStreamer`'s pool source.
pub struct MeshJobs {
    shared: Arc<MeshShared>,
}

impl AuxiliaryJobs for MeshJobs {
    type Payload = MeshJob;
    type Output = MeshUpload;
    type Worker = MeshWorkerLocal;

    fn worker(&self) -> Self::Worker {
        MeshWorkerLocal {
            faces: None,
            mesh_scratch: MeshScratch::default(),
        }
    }
    fn run(&self, payload: Self::Payload, local: &mut Self::Worker) -> Option<Self::Output> {
        let MeshWorkerLocal {
            faces,
            mesh_scratch,
        } = local;
        match payload.0 {
            MeshJobKind::Free(meshes) => {
                for MeshUpload { mesh, .. } in meshes {
                    let rejected = self.shared.meshes.lock().unwrap().put(mesh);
                    drop(rejected);
                }
                None
            }
            MeshJobKind::Mesh(x, sy, z, region, seq, settings) => {
                let context = if let Some(prepared) = &settings.visibility {
                    prepared.context.clone()
                } else if let Some((context, _)) = &settings.attached_tints {
                    context.clone()
                } else {
                    self.shared.context.read().unwrap().clone()
                };
                let recycled = self.shared.meshes.lock().unwrap().take();
                let mesh = mesh_section(
                    &context,
                    &settings,
                    faces,
                    mesh_scratch,
                    &region,
                    sy,
                    recycled,
                );
                Some(MeshUpload {
                    key: (x, sy, z),
                    mesh,
                    seq,
                    halo: settings
                        .visibility
                        .as_ref()
                        .and_then(|prepared| prepared.halo.clone()),
                })
            }
        }
    }
    fn idle(&self, local: &mut Self::Worker) {
        local.faces = None;
        local.mesh_scratch = MeshScratch::default();
    }
    fn all_idle(&self) {
        let meshes = std::mem::take(&mut *self.shared.meshes.lock().unwrap());
        drop(meshes);
    }
    fn classify(payload: &Self::Payload) -> (usize, &'static str) {
        match &payload.0 {
            MeshJobKind::Mesh(..) => (5, "stream.mesh"),
            MeshJobKind::Free(..) => (6, "stream.other"),
        }
    }
}

/// An update paused before removing its out-of-range live chunks.
pub struct StreamingUpdate {
    uploads: Vec<MeshUpload>,
    chunk: ChunkUpdate,
}

impl StreamingUpdate {
    /// Columns selected for removal. After `prepare_commit`, only the columns actually removed.
    pub fn unloading_columns(&self) -> &HashSet<(i32, i32)> {
        self.chunk.unloading_columns()
    }

    /// Leave the live columns alone when their save could not be submitted.
    pub fn keep_columns(&mut self) {
        self.chunk.keep_columns();
    }

    /// Keep only columns that may be committed by this sweep.
    pub fn retain_unloading_columns(&mut self, can_unload: impl FnMut(&(i32, i32)) -> bool) {
        self.chunk.retain_unloading_columns(can_unload);
    }
}

/// Result of processing generation results, pausing before a late full save.
pub enum PlanStep {
    Ready(StreamingUpdate),
    Save { chunk: Chunk, resume: PlanResume },
}

/// Work retained while a late full-chunk save callback runs.
pub struct PlanResume {
    uploads: Vec<MeshUpload>,
    stale: Vec<MeshUpload>,
    chunk: ChunkPlanResume<MeshUpload>,
}

/// A streaming update paused before scheduling generation after proto checkout.
pub struct StreamingCommit {
    uploads: Vec<MeshUpload>,
    chunk: ChunkCommit,
}

struct MeshDrain<'a> {
    meshing: &'a mut FxHashMap<(i32, i32, i32), u64>,
    mesh_jobs: &'a mut usize,
    compile_started: &'a mut FxHashMap<(i32, i32, i32), u64>,
    meshed_total: &'a mut u64,
    uploads: &'a mut Vec<MeshUpload>,
    stale: &'a mut Vec<MeshUpload>,
}
impl MeshDrain<'_> {
    fn receive(&mut self, world: &World, upload: MeshUpload) {
        *self.mesh_jobs = self.mesh_jobs.saturating_sub(1);
        let MeshUpload { key, seq, .. } = upload;
        let (x, _, z) = key;
        if self.meshing.get(&key) != Some(&seq) {
            self.stale.push(upload);
            return;
        }
        self.meshing.remove(&key);
        // Its column unloaded while it was in flight: nothing would ever remove it.
        if !world.chunks.contains_key(&(x, z)) {
            self.stale.push(upload);
            return;
        }
        self.compile_started.insert(key, seq);
        *self.meshed_total += 1;
        self.uploads.push(upload);
    }
}

pub struct MeshStreamer {
    mesh_shared: Arc<MeshShared>,
    meshing: FxHashMap<(i32, i32, i32), u64>,
    /// Mesh jobs actually in flight. `meshing` can hide more than one job for a
    /// section when an edit replaces its entry before the old job lands, so the
    /// room is counted from this, not from the map's length.
    mesh_jobs: usize,
    /// The sequence each section's latest accepted mesh was dispatched at
    /// (`CompiledSectionMesh.compileTaskStartTimeNs`).
    compile_started: FxHashMap<(i32, i32, i32), u64>,
    /// Columns whose sections have been meshed at least once.
    meshed: FxHashSet<(i32, i32)>,
    /// Dirty sections of loaded columns past the render distance, meshed once
    /// the player comes closer, and the area they were set aside for.
    waiting: FxHashSet<(i32, i32, i32)>,
    waiting_area: ((i32, i32), i32, usize, (usize, u64)),
    /// Per-update meshing scratch retained to avoid allocator churn.
    mesh_candidates_scratch: Vec<(i64, i32, i32, crate::section_mask::SectionMask)>,
    mesh_columns_scratch: Vec<((i32, i32), crate::section_mask::SectionMask)>,
    /// The centre's height: a column admitted part-way spends its slots on the
    /// sections nearest the camera, not on the lowest.
    pub center_y: i32,
    /// Finished meshes the renderer has not taken yet (the game's pending
    /// uploads): they count against the meshing room as the jobs do.
    pub upload_backlog: usize,
    /// Sections waiting for the mods' `section_visibility` answers, and the answers kept.
    pub section_visibility: section_visibility::SectionPreparation,
    attached_tint_preparation: mesh_job::TintPreparation,
    /// Dimension flag: Nether face shading.
    pub nether_shading: bool,
    /// A shader pack lights the faces: sections mesh without face shading
    /// (`Mesher::shader_pack_lighting`).
    pub shader_pack_lighting: bool,
    /// OptiFine/Iris separate ambient occlusion: vertex RGB has pure tint and alpha holds AO factor
    /// (`Mesher::separate_ao`).
    pub separate_ao: bool,
    /// A `shaders/` pack draws the terrain: fluid tops split as Iris splits them
    /// (`Mesher::shader_pack`).
    pub shader_pack: bool,
    /// `biomeBlendRadius`.
    pub biome_blend: i32,
    /// The `ao` and `cutoutLeaves` options.
    pub smooth_lighting: bool,
    pub cutout_leaves: bool,
    /// Draw regular ice opaque in the solid pass (`Mesher::opaque_ice`).
    pub opaque_ice: bool,
    /// Sprite layers a shader pack draws in the translucent pass (`[[materials]]
    /// translucent`), by layer; empty without one.
    pub translucent_layers: Arc<[bool]>,
    /// By state id, the terrain layer a `shaders/` pack's `layer.*` rules move the state to
    /// (`Mesher::layer_overrides`); empty without a pack or a state it moves.
    pub layer_overrides: Arc<[Option<sunrise_shaders::RenderLayer>]>,
    /// What the mesher does with leaf faces inside a canopy (`LeafCullingMode`).
    pub leaf_culling: LeafCullingMode,
    pub meshed_total: u64,
    /// Mesh the sections for drawing (false on a server thread).
    pub mesh: bool,
    /// Portable chunk streamer; must remain LAST so its Drop save-drain and Registration drop run last.
    chunks: ChunkStreamer<MeshJobs>,
}

impl std::ops::Deref for MeshStreamer {
    type Target = ChunkStreamer<MeshJobs>;
    fn deref(&self) -> &Self::Target {
        &self.chunks
    }
}
impl std::ops::DerefMut for MeshStreamer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.chunks
    }
}

mod mesh_job;
mod section_visibility;
pub use section_visibility::{SectionPreparation, VisibilityRequest};
mod viewed_meshing;
use mesh_job::{Faces, mesh_section};
pub use viewed_meshing::{
    MAX_VIEWED_SECTIONS, MeshSource, VIEWED_LINGER, VIEWED_MESH_WORKERS, VIEWED_UPLOADS_PER_FRAME,
    ViewedLevel, ViewedMeshing,
};

impl MeshStreamer {
    pub fn new<G: ChunkGenerator>(
        generator: Arc<G>,
        context: Arc<WorkerContext>,
        threads: usize,
    ) -> Self {
        let mesh_shared = Arc::new(MeshShared {
            context: RwLock::new(context.clone()),
            meshes: Mutex::new(MeshPool::default()),
        });
        let chunks = ChunkStreamer::with_auxiliary(
            generator,
            Arc::new(context.chunk.clone()),
            MeshJobs {
                shared: mesh_shared.clone(),
            },
            threads,
        );
        Self {
            mesh_shared,
            meshing: Default::default(),
            mesh_jobs: 0,
            compile_started: Default::default(),
            meshed: Default::default(),
            waiting: Default::default(),
            waiting_area: ((0, 0), 0, 0, (0, 0)),
            mesh_candidates_scratch: Vec::new(),
            mesh_columns_scratch: Vec::new(),
            center_y: 0,
            upload_backlog: 0,
            section_visibility: Default::default(),
            attached_tint_preparation: Default::default(),
            nether_shading: false,
            shader_pack_lighting: false,
            separate_ao: false,
            shader_pack: false,
            biome_blend: 2,
            smooth_lighting: true,
            cutout_leaves: true,
            opaque_ice: false,
            translucent_layers: Arc::from([]),
            layer_overrides: Arc::from([]),
            leaf_culling: LeafCullingMode::Off,
            meshed_total: 0,
            mesh: true,
            chunks,
        }
    }
    pub fn chunks(&self) -> &ChunkStreamer<MeshJobs> {
        &self.chunks
    }
    pub fn chunks_mut(&mut self) -> &mut ChunkStreamer<MeshJobs> {
        &mut self.chunks
    }
    pub(super) fn submit_mesh_job(&mut self, at: Option<(i32, i32)>, bias: i64, job: MeshJob) {
        self.chunks.submit_auxiliary(at, bias, job);
    }
    /// Resource packs reloaded (`LevelExtractor.allChanged`): mesh with the new
    /// models and sprites, and rebuild every loaded section. Meshes in flight
    /// were made with the old resources; their results are dropped.
    pub fn reload_resources(&mut self, world: &mut World, context: Arc<WorkerContext>) {
        *self.mesh_shared.context.write().unwrap() = context;
        self.attached_tint_preparation = Default::default();
        self.reload_sections(world);
    }
    /// `LevelExtractor.allChanged`: discard mesh work made before the reload and
    /// rebuild every loaded, lit section.
    pub fn reload_sections(&mut self, world: &mut World) {
        self.meshing.clear();
        self.mesh_jobs = 0;
        self.compile_started.clear();
        if self.section_visibility.guard_uploads {
            self.invalidate_visibility();
        }
        self.chunks.mark_loaded_sections_dirty(world);
    }
    #[cfg(test)]
    fn mark_loaded_sections_dirty(world: &mut World, lit: &FxHashSet<(i32, i32)>) {
        ChunkState::mark_loaded_sections_dirty_for(world, lit);
    }
    /// A cache-centre change invalidates mesh inputs but keeps the hidden columns lit.
    pub fn remote_chunk_visibility_changed(
        &mut self,
        world: &mut World,
        hidden: &[(i32, i32)],
        shown: &[(i32, i32)],
    ) {
        self.forget_visibility(world, hidden.iter().copied());
        for &(x, z) in hidden.iter().chain(shown) {
            dirty_visibility_columns(world, x, z);
        }
    }
    /// Give back meshes the renderer has copied: a worker frees them, ahead of
    /// other jobs, as vanilla's compile task closes its mesh data on its own
    /// thread once copied (`SectionRenderDispatcher.RenderSection.CompileTask.doTask`).
    /// Freed in the frame, the hundreds of meshes a chunk border brings waited
    /// on the allocator behind every busy worker: up to 20 ms of a frame each
    /// time while flying at render distance 32.
    pub fn free(&mut self, meshes: Vec<MeshUpload>) {
        if !meshes.is_empty() {
            self.submit_mesh_job(None, 0, MeshJob::free(meshes));
        }
    }
    /// Remove resident columns before clearing their streaming and level state.
    pub fn take_remote_columns(world: &mut World, gone: &FxHashSet<(i32, i32)>) -> Vec<Arc<Chunk>> {
        ChunkStreamer::take_remote_columns(world, gone)
    }
    /// The columns whose sections have been queued for meshing since they loaded, `(x, z)` in
    /// chunk coordinates, in no order. A column leaves it when it unloads or drops past the
    /// render distance.
    pub fn meshed_columns(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.meshed.iter().copied()
    }
    /// Clear streaming state after resident remote columns have been removed.
    pub fn forget_columns(&mut self, world: &mut World, gone: &FxHashSet<(i32, i32)>) {
        // Every column held (a far teleport): no pending section is left to keep, and a
        // pass over them all (80,000 at render distance 32) took 1.7 ms of the frame.
        self.forget_visibility(world, gone.iter().copied());
        if world.chunks.is_empty() {
            self.meshing.clear();
            self.mesh_jobs = 0;
            self.compile_started.clear();
            self.waiting.clear();
        } else {
            forget_pending_sections(
                &mut self.meshing,
                &mut self.compile_started,
                &mut self.waiting,
                gone,
                world.layout().sections() as i32,
            );
        }
        for &(x, z) in gone {
            self.meshed.remove(&(x, z));
        }
        self.chunks.forget_columns(world, gone);
    }
    fn region(world: &World, x: i32, z: i32, allow_missing: bool) -> Option<Arc<Region>> {
        let mut region: Region = Default::default();
        for dz in 0..3 {
            for dx in 0..3 {
                let chunk = world.chunks.get(&(x + dx as i32 - 1, z + dz as i32 - 1));
                if chunk.is_none() && (!allow_missing || (dx, dz) == (1, 1)) {
                    return None;
                }
                region[dz][dx] = chunk.cloned();
            }
        }
        Some(Arc::new(region))
    }
    /// Revalidate at publication: the accepted sequence identifies the section's lifetime
    /// (unload removes it) and policy/resource generation (reload clears it). A newer compile,
    /// or dirtiness waiting to start one, invalidates all earlier halo decisions.
    pub fn can_publish_mesh(&self, world: &World, upload: &MeshUpload) -> bool {
        !self.section_visibility.guard_uploads
            || (self.compile_started.get(&upload.key) == Some(&upload.seq)
                && world.chunks.contains_key(&(upload.key.0, upload.key.2))
                && !self.meshing.contains_key(&upload.key)
                && !self.waiting.contains(&upload.key)
                && !world.dirty_sections.contains(&upload.key)
                && self.section_visibility.halo_current(world, upload))
    }
    /// When the section's latest accepted mesh started compiling, on the
    /// `sequence` clock (`CompiledSectionMesh.getCompileTaskStartTime`).
    pub fn compile_start(&self, section: (i32, i32, i32)) -> Option<u64> {
        self.compile_started.get(&section).copied()
    }
    pub fn is_meshing(&self, section: (i32, i32, i32)) -> bool {
        self.meshing.contains_key(&section)
    }
    /// Whether any chunk is queued or on its way from generation to a mesh.
    pub fn busy(&self) -> bool {
        self.chunks.chunks_in_flight() || !self.meshing.is_empty() || self.chunks.pending() != 0
    }
    /// Whether any queued, in-flight, cached proto or extra-area work remains
    /// for an idle parked server level to drain.
    pub fn has_idle_work(&self) -> bool {
        !self.meshing.is_empty() || self.chunks.has_idle_work()
    }
    /// Whether the near terrain is still coming in: chunks generating,
    /// decorating, finishing or being lit (or their neighbours' light views),
    /// or sections meshing for the first time. Each of those is counted from
    /// when its job is queued, so queued jobs need no separate count.
    /// Remeshing sections already drawn (block, light and fluid updates) is
    /// not loading: it kept small machines' far terrain held back
    /// (`Lod::set_near_loading`) for as long as anything nearby changed.
    pub fn terrain_loading(&self) -> bool {
        self.chunks.chunks_in_flight()
            || self
                .meshing
                .keys()
                .any(|section| !self.compile_started.contains_key(section))
    }
    /// How much work each stage holds: queued jobs, then what is in flight or
    /// waiting at each step from generation to meshing.
    pub fn stages(&self) -> [(&'static str, usize); 7] {
        let [a, b, c, d, e, f] = self.chunks.stages();
        [a, b, c, d, e, f, ("meshing", self.meshing.len())]
    }
    /// Advance streaming: collect finished work, then schedule more.
    /// Returns meshes ready for upload and columns that were unloaded.
    pub fn update(&mut self, world: &mut World) -> (Vec<MeshUpload>, Vec<Arc<Chunk>>) {
        let plan = self.plan_update(world);
        self.finish_update(world, plan)
    }
    /// Collect completed work and plan removals without removing live chunks.
    pub fn plan_update(&mut self, world: &mut World) -> StreamingUpdate {
        let mut step = self.begin_plan_update(world);
        loop {
            match step {
                PlanStep::Ready(plan) => return plan,
                PlanStep::Save { chunk, resume } => {
                    self.save_finished(chunk);
                    step = self.resume_plan_update(world, resume);
                }
            }
        }
    }
    fn finish_plan_step(
        &mut self,
        step: ChunkPlanStep<MeshUpload>,
        uploads: Vec<MeshUpload>,
        stale: Vec<MeshUpload>,
    ) -> PlanStep {
        match step {
            ChunkPlanStep::Save { chunk, resume } => PlanStep::Save {
                chunk,
                resume: PlanResume {
                    uploads,
                    stale,
                    chunk: resume,
                },
            },
            ChunkPlanStep::Ready(chunk) => {
                self.free(stale);
                PlanStep::Ready(StreamingUpdate { uploads, chunk })
            }
        }
    }
    /// Start planning, yielding before any late-generated column is saved.
    pub fn begin_plan_update(&mut self, world: &mut World) -> PlanStep {
        if self.chunks.paused {
            return PlanStep::Ready(StreamingUpdate {
                uploads: Vec::new(),
                chunk: ChunkUpdate::paused(),
            });
        }
        // A section dirtied while its mesh was in flight must not upload that
        // mesh: drop its entry so the result is stale, and leave the section
        // dirty for the next job. The alternative is drawing the pre-edit
        // border face the edit was meant to hide.
        for ((x, z), sections) in world.dirty_sections.columns() {
            for sy in sections.iter() {
                self.meshing.remove(&(x, sy as i32, z));
            }
        }
        let mut uploads = Vec::new();
        let mut stale = Vec::new();
        let mut drain = MeshDrain {
            meshing: &mut self.meshing,
            mesh_jobs: &mut self.mesh_jobs,
            compile_started: &mut self.compile_started,
            meshed_total: &mut self.meshed_total,
            uploads: &mut uploads,
            stale: &mut stale,
        };
        let step = self
            .chunks
            .begin_plan_update_with(world, |world, upload| drain.receive(world, upload));
        self.finish_plan_step(step, uploads, stale)
    }
    /// Resume processing the same batch after the caller saves a late column.
    pub fn resume_plan_update(&mut self, world: &mut World, resume: PlanResume) -> PlanStep {
        let PlanResume {
            mut uploads,
            mut stale,
            chunk,
        } = resume;
        let mut drain = MeshDrain {
            meshing: &mut self.meshing,
            mesh_jobs: &mut self.mesh_jobs,
            compile_started: &mut self.compile_started,
            meshed_total: &mut self.meshed_total,
            uploads: &mut uploads,
            stale: &mut stale,
        };
        let step = self
            .chunks
            .resume_plan_update_with(world, chunk, |world, upload| drain.receive(world, upload));
        self.finish_plan_step(step, uploads, stale)
    }
    /// Commit a planned update after the caller has finished with live chunks.
    pub fn finish_update(
        &mut self,
        world: &mut World,
        mut plan: StreamingUpdate,
    ) -> (Vec<MeshUpload>, Vec<Arc<Chunk>>) {
        let (commit, protos) = self.prepare_commit(world, &mut plan);
        self.submit_protos(protos);
        self.complete_commit(world, commit)
    }
    /// Remove planned columns and check out proto saves before further scheduling.
    pub fn prepare_commit(
        &mut self,
        world: &mut World,
        plan: &mut StreamingUpdate,
    ) -> (StreamingCommit, Vec<ProtoSave>) {
        let uploads = std::mem::take(&mut plan.uploads);
        if plan.chunk.is_paused() {
            return (
                StreamingCommit {
                    uploads,
                    chunk: ChunkCommit::paused(),
                },
                Vec::new(),
            );
        }
        let (commit, proto_saves) = self.chunks.prepare_commit(world, &mut plan.chunk);
        if !commit.sweep_far().is_empty() {
            let far = commit.sweep_far();
            self.forget_visibility(world, far.iter().copied());
            for key in far {
                self.meshed.remove(key);
            }
            self.meshing.retain(|k, _| !far.contains(&(k.0, k.2)));
            self.compile_started
                .retain(|k, _| !far.contains(&(k.0, k.2)));
            self.waiting.retain(|k| !far.contains(&(k.0, k.2)));
        }
        (
            StreamingCommit {
                uploads,
                chunk: commit,
            },
            proto_saves,
        )
    }
    /// Resume an update after submitting its proto saves.
    pub fn complete_commit(
        &mut self,
        world: &mut World,
        commit: StreamingCommit,
    ) -> (Vec<MeshUpload>, Vec<Arc<Chunk>>) {
        let StreamingCommit { mut uploads, chunk } = commit;
        if chunk.is_paused() {
            return (uploads, chunk.into_removed());
        }
        let removed = self
            .chunks
            .complete_commit_without_clearing_dirty(world, chunk);
        // A server draws nothing: its sections are never meshed.
        if !self.mesh {
            world.dirty_sections.clear();
            self.chunks.finish_update_cover();
            return (uploads, removed);
        }
        let mut candidates = std::mem::take(&mut self.mesh_candidates_scratch);
        candidates.clear();
        let mut columns = std::mem::take(&mut self.mesh_columns_scratch);
        columns.clear();
        let mut hz_laps = crate::spans::laps();
        if self.section_visibility.listening {
            self.section_visibility.begin_meshing(world);
        }
        // Meshing once the column and its neighbours are lit, and the column
        // is within the render distance (`in_view_distance`, the columns
        // vanilla draws). Sections set aside past it come back
        // when the player moves: a column that loads with its light gets no
        // other request (#228). A server's far column (`ChunkMap::is_far`) is
        // drawn wherever it is, as a forced one.
        let area = (
            self.chunks.center,
            self.chunks.radius,
            self.chunks.forced.len(),
            world.chunks.far_state(),
        );
        if self.waiting_area != area {
            self.waiting_area = area;
            let chunks = &self.chunks;
            self.waiting.retain(|&(x, sy, z)| {
                let back = chunks.in_view(x, z) || world.chunks.is_far((x, z));
                if back {
                    world.dirty_sections.insert_mesh((x, sy, z));
                }
                !back
            });
        }
        hz_laps.lap("st_waiting");
        // Column by column: the checks below hold for all of a column's
        // sections, and columns waiting for light cost one look per frame.
        columns.extend(world.dirty_sections.columns());
        for ((x, z), sections) in columns.iter().copied() {
            if !world.chunks.contains_key(&(x, z)) {
                world.dirty_sections.take_column(x, z);
                continue;
            }
            if !self.chunks.in_view(x, z) && !world.chunks.is_far((x, z)) {
                world.dirty_sections.take_column(x, z);
                for sy in sections.iter().map(|sy| sy as i32) {
                    let key = (x, sy, z);
                    if self.section_visibility.listening {
                        self.section_visibility.defer_mesh(world, key);
                    }
                    self.waiting.insert(key);
                }
                continue;
            }
            // A neighbour being lit dirties this column again when its light
            // job finishes (new chunks relight their eight neighbours), so
            // wait for it rather than mesh once per neighbour. Columns still
            // in `dirty_light` here can't be lit yet (a neighbour of theirs is
            // missing): waiting for them could last until the player comes
            // back.
            let ready = (-1..=1).all(|dz| {
                (-1..=1).all(|dx| {
                    let n = (x + dx, z + dz);
                    (self.chunks.is_lit(n.0, n.1) && !self.chunks.is_lighting(n.0, n.1))
                        || (self.section_visibility.listening
                            && n != (x, z)
                            && !world.chunks.contains_key(&n))
                })
            });
            if !ready {
                continue;
            }
            candidates.push((self.chunks.column_distance(x, z), x, z, sections));
        }
        hz_laps.lap("st_candidates");
        // Nearest first, and only as many sections as the workers can chew.
        // Every ready column used to be submitted at once, so at a wide
        // render distance the queue — and `meshing`, and each job's 3x3
        // region snapshot — grew with the number of loaded columns and the
        // jobs went stale long before a worker reached them (#3629). What
        // stays dirty is asked for again, from the current centre, as
        // capacity frees; `requeue` then has a small queue to keep close.
        // Meshes waiting for upload count too: when uploads fall behind (a
        // low frame rate at a wide distance), meshing on held gigabytes of
        // them until the system swapped.
        // `room` counts sections, but a fresh column brings every one of its
        // sections dirty at once: choosing `room` columns could submit 24
        // times the room. Columns are chosen nearest first, then the budget
        // is spent section by section, lowest first; a column it runs out in
        // keeps the rest of its sections dirty.
        let room =
            (self.chunks.workers() * 32).saturating_sub(self.meshing.len() + self.upload_backlog);
        nearest_first(&mut candidates, room);
        let mut budget = room;
        // An all-air section meshes to nothing (`Mesher::mesh_reusing`) unless
        // a mod adds section geometry, and most of a tall column is air: the
        // first mesh of one goes straight to the uploads rather than through
        // a worker. Registration is set once, so with none now the workers'
        // tables have none either. Only a first mesh: a section that has one
        // (`compile_started`) may have had it accepted in this very update,
        // and two meshes of one section in `uploads` can be uploaded in
        // either order once the game sorts its upload queue.
        let air_meshes_empty = !crate::mesher::has_section_geometry();
        for (_, x, z, _) in candidates.drain(..) {
            if budget == 0 {
                break;
            }
            let listening = self.section_visibility.listening;
            let mut region = if listening {
                None
            } else {
                Self::region(world, x, z, false)
            };
            if !listening && region.is_none() {
                continue;
            }
            let mut sections = world.dirty_sections.take_column(x, z);
            let center = (self.center_y - world.layout().min_y()).div_euclid(16);
            let taken = sections.take_near(&mut budget, center);
            if !sections.is_empty() {
                world
                    .dirty_sections
                    .insert_mesh_column_mask((x, z), sections);
            }
            let mut taken = taken.iter().map(|sy| sy as i32);
            while let Some(sy) = taken.next() {
                let key = (x, sy, z);
                let air = air_meshes_empty
                    && !self.compile_started.contains_key(&key)
                    // Even a still-in-flight first compile may have published an answer
                    // for its neighbours; a raw edit to air must retire or replace it.
                    && (!listening || !self.section_visibility.has_answer(key))
                    && region
                        .as_deref()
                        .and_then(|region| region[1][1].as_deref())
                        .or_else(|| world.chunks.get(&(x, z)).map(Arc::as_ref))
                        .and_then(|centre| centre.sections.get(sy as usize))
                        .is_some_and(|section| section.is_uniform(0));
                let visibility = if listening && !air {
                    let Some(lease) = SectionPreparation::admit() else {
                        // Do not capture a region or ask a mod until the whole preparation fits.
                        for sy in std::iter::once(sy).chain(taken) {
                            world.dirty_sections.insert_mesh((x, sy, z));
                        }
                        budget = 0;
                        break;
                    };
                    Some(lease)
                } else {
                    None
                };
                let first = self.meshed.insert((x, z));
                let seq = self.chunks.next_sequence();
                if air {
                    // What `Done::Meshed` does for an accepted mesh; dropping
                    // `meshing`'s entry turns an older job still out for this
                    // section stale, as a newer submission would.
                    self.meshing.remove(&(x, sy, z));
                    self.compile_started.insert((x, sy, z), seq);
                    self.meshed_total += 1;
                    let mut mesh = SectionMesh::default();
                    mesh.sort_solid();
                    uploads.push(MeshUpload {
                        key,
                        mesh,
                        seq,
                        halo: None,
                    });
                    continue;
                }
                self.meshing.insert(key, seq);
                self.mesh_jobs += 1;
                let region = region
                    .get_or_insert_with(|| Self::region(world, x, z, true).expect("centre chunk"));
                self.submit_mesh(key, region, first, visibility);
            }
        }
        self.chunks.finish_update_cover();
        self.mesh_candidates_scratch = candidates;
        self.mesh_columns_scratch = columns;
        (uploads, removed)
    }
    /// Advance light work only, without streaming generation, unloads or meshing.
    /// Used for parked server levels so relights queued on them run and batch on
    /// their own worker pool without waiting for that dimension to be ticked.
    pub fn update_light(&mut self, world: &mut World) {
        self.chunks.update_light(world);
        if !self.mesh {
            world.dirty_sections.clear();
        }
    }
    /// Abort unvisited background chunk generation, decoration, and meshing on world exit.
    pub fn abort_generation(&mut self) {
        self.waiting.clear();
        self.meshing.clear();
        self.mesh_jobs = 0;
        self.chunks.abort_generation();
    }
    /// `ServerLevel.setChunkForced`: load a newly forced local column to
    /// ticking readiness before returning. `ChunkMap.prepareTickingChunk`
    /// waits for the lit column and its 3×3 so saved ticks can resume (#3341).
    /// While paused, record the ticket to take effect when its level starts
    /// streaming. Remote streamers can only accept resident columns. Refuse adds
    /// after generation stops. Repeated adds and removals do not wait.
    pub fn set_forced_loaded(&mut self, world: &mut World, x: i32, z: i32, forced: bool) -> bool {
        if !forced {
            return self.set_forced(x, z, false);
        }
        if self.chunks.is_aborted() {
            return false;
        }
        if self.chunks.remote {
            // Remote columns arrive via packets, not the generation queue.
            return world.chunks.contains_key(&(x, z)) && self.set_forced(x, z, true);
        }
        if self.chunks.paused {
            return self.set_forced(x, z, true);
        }
        if !self.set_forced(x, z, true) {
            return false;
        }
        self.wait_for_column(world, |world| world.chunks.is_ticking_ready((x, z)));
        true
    }
    fn wait_for_column(&mut self, world: &mut World, ready: impl Fn(&World) -> bool) -> bool {
        while !ready(world) {
            if !self.pump_column(world) {
                return false;
            }
            if !ready(world) {
                self.wait(std::time::Duration::from_millis(5));
            }
        }
        true
    }
    /// One turn of a synchronous wait: take the workers' results and give them more of the
    /// existing generation and light queue. Unloads wait for the normal owner update, which
    /// saves entities and notifies mods. False, doing nothing, once generation is aborted.
    pub fn pump_column(&mut self, world: &mut World) -> bool {
        if self.chunks.is_aborted() {
            return false;
        }
        let mut plan = self.plan_update(world);
        plan.chunk.suppress_unloads();
        let (meshes, unloaded) = self.finish_update(world, plan);
        debug_assert!(unloaded.is_empty());
        self.free(meshes);
        true
    }
}

impl Drop for MeshStreamer {
    fn drop(&mut self) {
        self.abort_generation();
    }
}

/// Drops the pending sections (`meshing`, `compile_started`, `waiting`) of
/// the `gone` columns: one pass over each store when that is cheaper than a
/// lookup per section of every column (a far teleport at render distance 32
/// forgets ~3,700 columns, 1.7 ms of lookups), else by key.
fn forget_pending_sections(
    meshing: &mut FxHashMap<(i32, i32, i32), u64>,
    compile_started: &mut FxHashMap<(i32, i32, i32), u64>,
    waiting: &mut FxHashSet<(i32, i32, i32)>,
    gone: &FxHashSet<(i32, i32)>,
    sections: i32,
) {
    let lookups = gone.len().saturating_mul(sections.max(0) as usize);
    if lookups > meshing.len() + compile_started.len() + waiting.len() {
        let kept = |&(x, _, z): &(i32, i32, i32)| !gone.contains(&(x, z));
        meshing.retain(|k, _| kept(k));
        compile_started.retain(|k, _| kept(k));
        waiting.retain(kept);
    } else {
        for &(x, z) in gone {
            for sy in 0..sections {
                meshing.remove(&(x, sy, z));
                compile_started.remove(&(x, sy, z));
                waiting.remove(&(x, sy, z));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::section_visibility::tests::fixture;
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn a_far_column_past_the_render_distance_is_not_set_aside() {
        let (mut world, mut streaming, _) = fixture();
        streaming.center = (0, 0);
        streaming.radius = 2;
        for x in [300, 400] {
            world.chunks.insert((x, 0), Arc::new(Chunk::empty(x, 0)));
            world.dirty_sections.insert_mesh((x, 0, 0));
        }
        assert!(
            world.chunks.keep_far(&[(300, 0)]).is_empty(),
            "nothing was hidden"
        );
        streaming.update(&mut world);
        assert!(
            world.dirty_sections.contains(&(300, 0, 0)),
            "waits for its light"
        );
        assert!(!world.dirty_sections.contains(&(400, 0, 0)));
        assert!(streaming.waiting.contains(&(400, 0, 0)));
        assert!(!streaming.waiting.contains(&(300, 0, 0)));
    }

    #[test]
    fn mesh_pool_reuses_capacity_with_clean_geometry_state() {
        let mut mesh = SectionMesh::default();
        mesh.solid.resize(64, bytemuck::Zeroable::zeroed());
        mesh.translucent.resize(16, bytemuck::Zeroable::zeroed());
        mesh.block_entities.push(([1, 2, 3], 7));
        mesh.lights.push(([1, 2, 3], 7));
        mesh.visibility = 0;
        mesh.sides = crate::mesher::Sides::unsorted(16);
        let capacities = mesh.capacity_bytes();
        let pointers = (mesh.solid.as_ptr(), mesh.translucent.as_ptr());
        let mut pool = MeshPool::default();
        assert!(pool.put(mesh).is_none());
        assert_eq!(pool.bytes, capacities);
        let reused = pool.take();
        assert_eq!(reused.capacity_bytes(), capacities);
        assert_eq!(
            pointers,
            (reused.solid.as_ptr(), reused.translucent.as_ptr())
        );
        assert!(reused.solid.is_empty());
        assert!(reused.translucent.is_empty());
        assert!(reused.block_entities.is_empty());
        assert!(reused.lights.is_empty());
        assert_eq!(reused.visibility, crate::mesher::ALL_VISIBLE);
        assert_eq!(reused.sides, crate::mesher::Sides::default());
        assert_eq!(pool.bytes, 0);
    }

    #[test]
    fn mesh_pool_caps_retained_count_and_bytes() {
        let mut pool = MeshPool::default();
        for _ in 0..MeshPool::COUNT {
            let mut mesh = SectionMesh::default();
            mesh.solid.reserve_exact(1);
            assert!(pool.put(mesh).is_none());
        }
        let mut extra = SectionMesh::default();
        extra.solid.reserve_exact(1);
        assert!(pool.put(extra).is_some());
        for _ in 0..MeshPool::COUNT {
            pool.take();
        }
        assert_eq!(pool.bytes, 0);
        for i in 0..2 {
            let mut mesh = SectionMesh::default();
            mesh.solid.reserve_exact(
                MeshPool::BYTES / std::mem::size_of::<crate::mesher::TerrainVertex>() / 2 + 1,
            );
            assert_eq!(pool.put(mesh).is_some(), i == 1);
            assert!(pool.bytes <= MeshPool::BYTES);
            assert_eq!(pool.meshes.len(), 1);
        }
        pool.take();
        assert_eq!(pool.bytes, 0);
        let mut huge = SectionMesh::default();
        huge.solid.reserve_exact(
            MeshPool::BYTES / std::mem::size_of::<crate::mesher::TerrainVertex>() + 1,
        );
        assert!(pool.put(huge).is_some());
        assert!(pool.meshes.is_empty());
        assert_eq!(pool.bytes, 0);
    }

    /// Whether the forgotten columns are few (looked up by key) or many
    /// (one pass over each store), exactly their pending sections go.
    #[test]
    fn forgotten_columns_lose_their_pending_sections_either_way() {
        let pending: Vec<(i32, i32, i32)> = (0..4)
            .flat_map(|x| (0..3).map(move |sy| (x, sy, -x)))
            .collect();
        for gone in [
            vec![(1, -1)],
            (0..400)
                .map(|x| (x, -x))
                .filter(|&c| c != (2, -2))
                .collect(),
        ] {
            let gone: FxHashSet<(i32, i32)> = gone.into_iter().collect();
            let mut meshing: FxHashMap<_, u64> = pending.iter().map(|&k| (k, 1)).collect();
            let mut started = meshing.clone();
            let mut waiting: FxHashSet<_> = pending.iter().copied().collect();
            forget_pending_sections(&mut meshing, &mut started, &mut waiting, &gone, 3);
            let mut left: Vec<_> = waiting.into_iter().collect();
            left.sort();
            let expected: Vec<_> = pending
                .iter()
                .copied()
                .filter(|&(x, _, z)| !gone.contains(&(x, z)))
                .collect();
            assert_eq!(left, expected, "{} gone", gone.len());
            let mut left: Vec<_> = meshing.into_keys().collect();
            left.sort();
            assert_eq!(left, expected);
            let mut left: Vec<_> = started.into_keys().collect();
            left.sort();
            assert_eq!(left, expected);
        }
    }

    #[test]
    #[ignore = "daily"]
    fn a_resend_keeps_models_at_unchanged_resident_cells() {
        if !crate::test_process::isolated(concat!(
            "streaming::tests::",
            stringify!(a_resend_keeps_models_at_unchanged_resident_cells)
        )) {
            return;
        }
        for lit in [false, true] {
            let (mut world, mut streaming, stone) = fixture();
            let resend = |first| {
                let mut chunk = Chunk::empty(2, -3);
                chunk.set(1, 10, 1, first);
                chunk.set(2, 10, 1, stone);
                chunk
            };
            let first = mod_api::__private::built::model_ref(1, 7);
            let second = mod_api::__private::built::model_ref(1, 8);
            let (x, y, z) = (33, 10, -47);
            streaming.insert_remote(&mut world, resend(stone));
            world.set_extra_models(x, y, z, &[first]).unwrap();
            world.set_extra_models(x + 1, y, z, &[second]).unwrap();
            for (state, expected) in [(stone, &[first][..]), (0, &[][..])] {
                if !lit {
                    streaming.chunks.mark_unlit_for_test((2, -3));
                }
                streaming.replace_remote(&mut world, resend(state));
                assert_eq!(world.extra_models(x, y, z), expected, "lit={lit}");
                assert_eq!(world.extra_models(x + 1, y, z), [second], "lit={lit}");
            }
            world.unload_chunk((2, -3));
            streaming.replace_remote(&mut world, resend(stone));
            assert!(world.extra_models(x + 1, y, z).is_empty());
        }
    }
}

#[cfg(test)]
mod reload_tests {
    use super::*;

    #[test]
    fn reload_marks_every_section_of_loaded_lit_columns() {
        let mut world = World::new(sunrise_world::registry::BlockStateId(0), 0);
        world.chunks.insert((2, -3), Arc::new(Chunk::empty(2, -3)));
        world.chunks.insert((4, 5), Arc::new(Chunk::empty(4, 5)));
        let lit = FxHashSet::from_iter([(2, -3), (99, 99)]);

        MeshStreamer::mark_loaded_sections_dirty(&mut world, &lit);

        let sections = world.layout().sections() as i32;
        assert_eq!(world.dirty_sections.len(), sections as usize);
        for sy in 0..sections {
            assert!(world.dirty_sections.contains(&(2, sy, -3)));
            assert!(!world.dirty_sections.contains(&(4, sy, 5)));
        }
    }

    #[test]
    fn reload_keeps_existing_work_and_ignores_unloaded_lit_columns() {
        let mut world = World::new(sunrise_world::registry::BlockStateId(0), 0);
        world.chunks.insert((4, 5), Arc::new(Chunk::empty(4, 5)));
        world.dirty_sections.insert((4, 3, 5));
        let lit = FxHashSet::from_iter([(99, 99)]);

        MeshStreamer::mark_loaded_sections_dirty(&mut world, &lit);

        assert_eq!(world.dirty_sections.len(), 1);
        assert!(world.dirty_sections.contains(&(4, 3, 5)));
    }
}
