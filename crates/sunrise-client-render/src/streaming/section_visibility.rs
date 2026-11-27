//! Admission and world-thread preparation for terrain visibility in the player's level.
use super::{MeshJob, MeshSettings, MeshStreamer, MeshUpload, WorkerContext};
use crate::fast_hash::{FxHashMap, FxHashSet};
use crate::mesher::{
    ANSWER_BYTES, Lease, MAX_RESIDENT_VISIBILITY_BYTES, PLAN_BYTES, Region, SectionAnswer,
    SectionVisibilityPlan, capture_input, plan_section,
};
use crate::world::World;
use mod_api::{
    BlockState, CellVisibility, MAX_CELL_RECORDS, SECTION_INPUT_CELLS, SectionCell,
    SectionVisibilityInput,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use sunrise_level::chunk_streamer::dirty_visibility_columns;

// The registry belongs to the already shared worker context. Capture allocates only these
// three arrays and retains a reference to the existing keys, with no palette scratch.
const SNAPSHOT_BYTES: usize = size_of::<SectionVisibilityInput>()
    + SECTION_INPUT_CELLS * (size_of::<SectionCell>() + size_of::<u8>() + size_of::<Option<u16>>());
const WORKER_BYTES: usize = PLAN_BYTES + size_of::<Region>() + size_of::<FrozenPlan>();
const HALO_BYTES: usize = size_of::<HaloAnswers>();
const CACHE_BYTES: usize = ANSWER_BYTES + size_of::<HeldAnswer>() - size_of::<SectionAnswer>();
pub(super) const REQUEST_BYTES: usize = SNAPSHOT_BYTES
    + MAX_CELL_RECORDS * size_of::<CellVisibility>()
    + size_of::<VisibilityRequest>()
    + WORKER_BYTES
    + HALO_BYTES
    + CACHE_BYTES;
static RESIDENT_BYTES: AtomicUsize = AtomicUsize::new(0);

/// Section visibility state for the player's current level.
#[derive(Default)]
pub struct SectionPreparation {
    pub(super) listening: bool,
    /// Also validate results left over from when a listener was active, or just became active.
    pub(super) guard_uploads: bool,
    pending: Vec<VisibilityRequest>,
    answers: FxHashMap<(i32, i32, i32), HeldAnswer>,
    capacity_waiting: FxHashSet<(i32, i32, i32)>,
    /// The mods and refusal codes already logged since the last refresh.
    pub refusals: FxHashSet<(&'static str, &'static str)>,
    logged_capacity: bool,
}

struct HeldAnswer {
    answer: SectionAnswer,
    border_seq: u64,
    current: bool,
    _lease: Lease,
    resident: &'static AtomicUsize,
}

impl Drop for HeldAnswer {
    fn drop(&mut self) {
        self.resident.fetch_sub(CACHE_BYTES, Ordering::AcqRel);
    }
}

/// The versions of the neighbour answers used by a mesh. Zero means no cached answer was read.
pub(super) struct HaloAnswers {
    versions: [u64; 27],
    _lease: Lease,
}

/// Prepared alongside the hook input; keeps the same resources and budget until the worker ends.
pub struct FrozenPlan {
    pub(crate) plan: Option<SectionVisibilityPlan>,
    pub(super) context: Arc<WorkerContext>,
    pub(super) halo: Option<Arc<HaloAnswers>>,
    _lease: Lease,
}

/// A section chosen to mesh, with admission already held for the input, answer and worker plan.
pub struct VisibilityRequest {
    /// The section: column x, section index, column z.
    pub key: (i32, i32, i32),
    /// The same immutable blocks, light and biomes that the worker will read.
    pub region: Arc<Region>,
    seq: u64,
    first: bool,
    settings: MeshSettings,
    context: Arc<WorkerContext>,
    lease: Lease,
}

impl VisibilityRequest {
    /// Capture input using this request's immutable region and the worker's own biome table.
    pub fn capture_input(&self, state_of: impl FnMut(u32) -> BlockState) -> SectionVisibilityInput {
        capture_input(
            &self.region,
            self.key.1,
            state_of,
            &self.context.chunk.biomes,
        )
    }
}

impl SectionPreparation {
    /// Admit before copying the snapshot. A job that cannot fit stays dirty, not in a second
    /// queue holding an uncharged snapshot or answers.
    pub(super) fn admit() -> Option<Lease> {
        Lease::charge(REQUEST_BYTES)
    }

    fn replace_answer(
        &mut self,
        key: (i32, i32, i32),
        seq: u64,
        answer: Option<SectionAnswer>,
        lease: &mut Lease,
        resident: &'static AtomicUsize,
    ) -> bool {
        self.capacity_waiting.remove(&key);
        let before = self.answers.get(&key);
        let border_changed =
            SectionAnswer::border_differs(before.map(|held| &held.answer), answer.as_ref());
        // A reader may already have dropped its completed mesh while this answer was stale;
        // let it compile again even if the newly checked border is identical.
        let remesh_readers =
            border_changed || before.is_some_and(|held| !held.current || answer.is_none());
        if let Some(answer) = answer {
            if let Some(old) = self.answers.get_mut(&key) {
                // Reuse the old resident charge. The new answer was staged with the request's
                // charge and replacing it cannot fail, even when the resident pool is full.
                old.answer = answer;
                old.current = true;
                if border_changed {
                    old.border_seq = seq;
                }
            } else if resident
                .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                    (used + CACHE_BYTES <= MAX_RESIDENT_VISIBILITY_BYTES)
                        .then_some(used + CACHE_BYTES)
                })
                .is_ok()
            {
                self.answers.insert(
                    key,
                    HeldAnswer {
                        answer,
                        border_seq: seq,
                        current: true,
                        _lease: lease.split(CACHE_BYTES),
                        resident,
                    },
                );
            } else {
                self.capacity_waiting.insert(key);
                if !self.logged_capacity {
                    log::warn!(
                        "Sunrise: section_visibility: resident visibility budget exhausted; new masks wait for room (E_SECTION_VISIBILITY_CAPACITY)"
                    );
                    self.logged_capacity = true;
                }
                return false;
            }
        } else {
            self.answers.remove(&key);
        }
        remesh_readers
    }

    fn retry_masks(&mut self, resident: &AtomicUsize, dirty: &mut crate::world::DirtySections) {
        if self.capacity_waiting.is_empty() {
            return;
        }
        let slots =
            (MAX_RESIDENT_VISIBILITY_BYTES - resident.load(Ordering::Acquire)) / CACHE_BYTES;
        for key in self.capacity_waiting.extract_if(|_| true).take(slots) {
            dirty.insert_mesh(key);
        }
    }

    pub(super) fn begin_meshing(&mut self, world: &mut World) {
        for ((x, z), sections) in world.dirty_sections.visibility_edits() {
            for sy in sections.iter() {
                if let Some(held) = self.answers.get_mut(&(x, sy as i32, z)) {
                    held.current = false;
                }
            }
        }
        self.retry_masks(&RESIDENT_BYTES, &mut world.dirty_sections);
    }

    pub(super) fn has_answer(&self, key: (i32, i32, i32)) -> bool {
        self.answers.contains_key(&key)
    }

    pub(super) fn defer_mesh(&mut self, world: &mut World, key: (i32, i32, i32)) {
        self.capacity_waiting.remove(&key);
        // An out-of-view section won't re-prepare until it re-enters the mesh range. Do not
        // leave its stale answer in the way of a still visible reader at the edge.
        if self.answers.get(&key).is_some_and(|answer| !answer.current) {
            self.answers.remove(&key);
            dirty_neighbours(world, key);
        }
    }

    pub(super) fn halo_current(&self, world: &World, upload: &MeshUpload) -> bool {
        let Some(halo) = &upload.halo else {
            return true;
        };
        let (x, y, z) = upload.key;
        halo.versions.iter().enumerate().all(|(i, &seq)| {
            if seq == 0 {
                return true;
            }
            let i = i as i32;
            let key = (x + i % 3 - 1, y + i / 9 - 1, z + i / 3 % 3 - 1);
            !world.dirty_sections.visibility_edited(key)
                && self
                    .answers
                    .get(&key)
                    .is_some_and(|held| held.current && held.border_seq == seq)
        })
    }
}

impl MeshStreamer {
    pub(super) fn submit_mesh(
        &mut self,
        key: (i32, i32, i32),
        region: &Arc<Region>,
        first: bool,
        visibility: Option<Lease>,
    ) {
        let mut settings = self.mesh_settings();
        if let Some(lease) = visibility {
            let context = self.mesh_shared.context.read().unwrap().clone();
            self.section_visibility.pending.push(VisibilityRequest {
                key,
                region: region.clone(),
                seq: self.chunks.sequence(),
                first,
                settings,
                context,
                lease,
            });
            return;
        }
        let (x, sy, z) = key;
        if region[1][1]
            .as_ref()
            .is_some_and(|c| !c.extra_models.is_empty())
        {
            let context = self.mesh_shared.context.read().unwrap().clone();
            self.attached_tint_preparation
                .prepare(&context, &mut settings, region, sy);
        }
        let job = MeshJob::mesh(x, sy, z, region.clone(), self.chunks.sequence(), settings);
        self.submit_mesh_job(Some((x, z)), if first { 2 } else { 0 }, job);
    }

    /// The admitted sections whose raw inputs are ready to lend to mods on the world thread.
    /// Empty while no mod listens.
    pub fn take_visibility_requests(&mut self) -> Vec<VisibilityRequest> {
        std::mem::take(&mut self.section_visibility.pending)
    }

    /// Accept a completed decision only while its selection is still current. Freeze its own
    /// answer and the cached halo answers before handing the immutable input to a worker.
    pub fn submit_visibility_answer(
        &mut self,
        world: &mut World,
        request: VisibilityRequest,
        answer: Option<SectionAnswer>,
    ) {
        let VisibilityRequest {
            key,
            region,
            seq,
            first,
            mut settings,
            context,
            mut lease,
        } = request;
        let (x, sy, z) = key;
        if self.meshing.get(&key) != Some(&seq) || !world.chunks.contains_key(&(x, z)) {
            return;
        }
        if self
            .section_visibility
            .replace_answer(key, seq, answer, &mut lease, &RESIDENT_BYTES)
        {
            dirty_neighbours(world, key);
        }
        let answers = &self.section_visibility.answers;
        let mut versions = [0; 27];
        let plan = plan_section(
            answers.get(&key).map(|held| &held.answer),
            |[dx, dy, dz]| {
                let held = answers.get(&(x + dx, sy + dy, z + dz))?;
                versions[((dy + 1) * 9 + (dz + 1) * 3 + dx + 1) as usize] = held.border_seq;
                Some(&held.answer)
            },
        );
        let halo = versions.iter().any(|&seq| seq != 0).then(|| {
            Arc::new(HaloAnswers {
                versions,
                _lease: lease.split(HALO_BYTES),
            })
        });
        settings.visibility = Some(Arc::new(FrozenPlan {
            plan,
            context: context.clone(),
            halo,
            _lease: lease.split(WORKER_BYTES),
        }));
        self.attached_tint_preparation
            .prepare(&context, &mut settings, &region, sy);
        let job = MeshJob::mesh(x, sy, z, region, seq, settings);
        self.submit_mesh_job(Some((x, z)), if first { 2 } else { 0 }, job);
    }

    /// A visibility policy change, or the join. Invalidate answers, jobs and queued uploads,
    /// including results accepted earlier in this frame. No work when there was no listener.
    pub fn refresh_section_visibility(&mut self, world: &mut World, listening: bool) {
        let state = &mut self.section_visibility;
        if !listening && !state.listening {
            return;
        }
        state.listening = listening;
        state.guard_uploads = true;
        self.chunks.set_dirty_visibility_on_load(listening);
        world.dirty_sections.track_visibility_edits(listening);
        self.reload_sections(world);
    }

    pub(super) fn invalidate_visibility(&mut self) {
        let state = &mut self.section_visibility;
        state.pending.clear();
        state.answers.clear();
        state.capacity_waiting.clear();
        state.refusals.clear();
        state.logged_capacity = false;
    }

    /// Rebuild surviving halo readers even if the unloaded column never had a mod answer:
    /// the raw input changes from present to missing independently of its visibility mask.
    pub(super) fn forget_visibility(
        &mut self,
        world: &mut World,
        columns: impl IntoIterator<Item = (i32, i32)>,
    ) {
        if !self.section_visibility.listening {
            return;
        }
        for (x, z) in columns {
            for sy in 0..world.layout().sections() as i32 {
                self.section_visibility.answers.remove(&(x, sy, z));
                self.section_visibility.capacity_waiting.remove(&(x, sy, z));
            }
            dirty_visibility_columns(world, x, z);
        }
    }
}

/// The 26 loaded halo readers of a section whose border answer just changed.
fn dirty_neighbours(world: &mut World, (x, sy, z): (i32, i32, i32)) {
    let sections = world.layout().sections() as i32;
    for dx in -1..=1 {
        for dz in -1..=1 {
            if !world.chunks.contains_key(&(x + dx, z + dz)) {
                continue;
            }
            for dy in -1..=1 {
                let ny = sy + dy;
                if (dx, dy, dz) != (0, 0, 0) && (0..sections).contains(&ny) {
                    world.dirty_sections.insert_mesh((x + dx, ny, z + dz));
                }
            }
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::streaming::{ChunkGenerator, ChunkWorkerContext, MeshWorkerContext, Proto};
    use crate::{
        assets::Assets, biome::Biomes, chunk::Chunk, light::LightTables, mesher::MeshTables, models,
    };
    use std::sync::OnceLock;
    use std::time::Duration;
    use sunrise_assets::resource_source::ResourceRoot;
    use sunrise_items::blocks::BlockFacts;
    use sunrise_world::registry::BlockStateId;

    pub(in crate::streaming) fn fixture() -> (World, MeshStreamer, u32) {
        struct RemoteOnly;
        impl ChunkGenerator for RemoteOnly {
            type Worker = ();
            fn worker(&self) {}
            fn proto(&self, _: &mut (), _: i32, _: i32) -> Result<Proto, String> {
                panic!("a remote level never generates locally")
            }
            fn finish(&self, _: Proto) -> Chunk {
                panic!("a remote level never generates locally")
            }
        }
        static CONTEXT: OnceLock<Arc<WorkerContext>> = OnceLock::new();
        let context = CONTEXT.get_or_init(|| {
            let root = std::path::PathBuf::from(
                std::env::var_os("SUNRISE_MESH_TEST_RESOURCES")
                    .or_else(|| std::env::var_os("SUNRISE_TEST_RESOURCES"))
                    .unwrap(),
            );
            let source = ResourceRoot::directory(root.clone());
            let assets = Arc::new(Assets::load(&root).unwrap());
            let render = Arc::new(crate::assets::RenderAssets::load(&source, &assets).unwrap());
            let facts = BlockFacts::load(&root, assets.registry).unwrap();
            let visibility = models::load_visibility(&root, assets.registry).unwrap();
            let tables =
                Arc::new(MeshTables::new(&assets, &render, &facts, visibility, None).unwrap());
            let light = Arc::new(LightTables::load(&root, assets.registry, &facts).unwrap());
            let biomes = Arc::new(Biomes::load(&source).unwrap());
            Arc::new(WorkerContext {
                chunk: ChunkWorkerContext {
                    assets,
                    biomes,
                    light,
                },
                mesh: MeshWorkerContext {
                    render,
                    tables,
                    hidden_states: Arc::from([]),
                    part_tables: Default::default(),
                },
            })
        });
        let world = World::new(BlockStateId(0), 0);
        let mut streaming = MeshStreamer::new(Arc::new(RemoteOnly), context.clone(), 1);
        streaming.remote = true;
        streaming.center_y = world.bottom_y();
        let stone = context
            .chunk
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap()
            .0;
        (world, streaming, stone)
    }

    const CENTRE: (i32, i32, i32) = (0, 0, 0);

    #[test]
    fn resident_capacity_does_not_stall_replacement_or_clear() {
        use crate::mesher::SectionPlanBuilder;
        static RESIDENT: AtomicUsize =
            AtomicUsize::new(MAX_RESIDENT_VISIBILITY_BYTES - CACHE_BYTES);
        static SHARED: AtomicUsize = AtomicUsize::new(MAX_RESIDENT_VISIBILITY_BYTES - CACHE_BYTES);
        let mut state = SectionPreparation::default();
        let mut lease = Lease::charge_in(&SHARED, REQUEST_BYTES).unwrap();
        let mut builder = SectionPlanBuilder::new();
        builder.hide_terrain();
        assert!(state.replace_answer(CENTRE, 1, builder.finish(), &mut lease, &RESIDENT));
        drop(lease);
        assert_eq!(
            RESIDENT.load(Ordering::Acquire),
            MAX_RESIDENT_VISIBILITY_BYTES
        );
        let mut lease =
            Lease::charge_in(&SHARED, REQUEST_BYTES).expect("cache leaves room before capture");
        let mut builder = SectionPlanBuilder::new();
        builder.restrict([0, 0, 0], true, true, 61);
        assert!(state.replace_answer(CENTRE, 2, builder.finish(), &mut lease, &RESIDENT));
        let stored = &state.answers[&CENTRE].answer;
        assert!(!stored.hides_terrain());
        assert_eq!(
            RESIDENT.load(Ordering::Acquire),
            MAX_RESIDENT_VISIBILITY_BYTES
        );
        let waiting = (1, 0, 0);
        let mut builder = SectionPlanBuilder::new();
        builder.hide_terrain();
        assert!(!state.replace_answer(waiting, 3, builder.finish(), &mut lease, &RESIDENT));
        let mut dirty = crate::world::DirtySections::default();
        state.retry_masks(&RESIDENT, &mut dirty);
        assert!(
            !dirty.contains(&waiting),
            "a full cache does not re-ask every frame"
        );
        assert!(state.replace_answer(CENTRE, 4, None, &mut lease, &RESIDENT));
        assert!(!state.answers.contains_key(&CENTRE));
        state.retry_masks(&RESIDENT, &mut dirty);
        assert!(
            dirty.contains(&waiting),
            "a freed slot schedules a fresh answer"
        );
        let mut builder = SectionPlanBuilder::new();
        builder.hide_terrain();
        assert!(state.replace_answer(waiting, 5, builder.finish(), &mut lease, &RESIDENT));
        assert!(state.answers[&waiting].answer.hides_terrain());
        state.answers.clear();
        drop(lease);
        assert_eq!(
            RESIDENT.load(Ordering::Acquire),
            MAX_RESIDENT_VISIBILITY_BYTES - CACHE_BYTES
        );
        assert_eq!(
            SHARED.load(Ordering::Acquire),
            MAX_RESIDENT_VISIBILITY_BYTES - CACHE_BYTES
        );
    }

    fn test_block_key() -> mod_api::BlockKey {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        mod_api::__private::resolve_block_keys(["minecraft:air"], |_| {
            sunrise_mod_runtime::block_definitions::BlockDefinition::new(
                &[&std::collections::BTreeMap::new()],
                0,
            )
            .map_err(|error| format!("{error:?}"))
        })
        .unwrap();
        mod_api::__private::block_key("minecraft:air").unwrap()
    }

    /// `Streaming::meshed_columns` (#5911): a column is listed once its sections are meshed and
    /// leaves the list when it unloads.
    #[test]
    #[ignore = "daily"]
    fn a_meshed_column_is_listed_until_it_unloads() {
        use crate::mesher::SectionPlanBuilder;

        mod_platform::testing::with_mods(Vec::new().into(), || {
            let (mut world, mut streaming, stone) = fixture();
            let base = world.bottom_y();
            let mut centre = Chunk::empty(0, 0);
            centre.set(8, base + 4, 8, stone);
            streaming.insert_remote(&mut world, centre);
            assert!(streaming.meshed_columns().next().is_none());
            streaming.refresh_section_visibility(&mut world, true);
            let mut meshed = false;
            for _ in 0..100 {
                let (uploads, _) = streaming.update(&mut world);
                for request in streaming.take_visibility_requests() {
                    let plan = SectionPlanBuilder::new().finish();
                    streaming.submit_visibility_answer(&mut world, request, plan);
                }
                meshed |= uploads.iter().any(|upload| upload.key == CENTRE);
                streaming.free(uploads);
                if meshed {
                    break;
                }
                streaming.wait(Duration::from_millis(20));
            }
            assert!(meshed, "the centre column was meshed");
            assert!(streaming.meshed_columns().any(|column| column == (0, 0)));
            let gone = [(0, 0)].into_iter().collect();
            MeshStreamer::take_remote_columns(&mut world, &gone);
            streaming.forget_columns(&mut world, &gone);
            assert!(!streaming.meshed_columns().any(|column| column == (0, 0)));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn remote_column_arrival_and_unload_remesh_the_actual_halo() {
        use crate::mesher::SectionPlanBuilder;
        use mod_api::SectionCell;

        mod_platform::testing::with_mods(Vec::new().into(), || {
            let block = test_block_key();
            let state = |raw: u32| mod_api::__private::block_state_from_ordinal(block, raw as u16);
            let (mut world, mut streaming, stone) = fixture();
            let base = world.bottom_y();
            let mut centre = Chunk::empty(0, 0);
            centre.set(15, base + 4, 8, stone);
            streaming.insert_remote(&mut world, centre);
            assert!(streaming.update(&mut world).0.is_empty());
            assert!(streaming.take_visibility_requests().is_empty());
            assert!(
                world.dirty_sections.contains(&CENTRE),
                "the no-listener route still waits for a complete region"
            );
            streaming.refresh_section_visibility(&mut world, true);

            let compile =
                |world: &mut World, streaming: &mut MeshStreamer, expected: SectionCell| {
                    let mut captured = false;
                    for _ in 0..100 {
                        let (uploads, _) = streaming.update(world);
                        for request in streaming.take_visibility_requests() {
                            let mut builder = SectionPlanBuilder::new();
                            if request.key == CENTRE {
                                let input = request.capture_input(state);
                                assert_eq!(input.cell([16, 4, 8]), Ok(expected));
                                captured = true;
                            } else if request.key == (1, 0, 0) {
                                builder.restrict([1, 4, 8], false, false, 0);
                            }
                            streaming.submit_visibility_answer(world, request, builder.finish());
                        }
                        let faces = uploads
                            .iter()
                            .find(|upload| {
                                upload.key == CENTRE && streaming.can_publish_mesh(world, upload)
                            })
                            .map(|upload| upload.mesh.solid.len() / 4);
                        streaming.free(uploads);
                        if let Some(faces) = faces {
                            assert!(captured);
                            return faces;
                        }
                        streaming.wait(Duration::from_millis(20));
                    }
                    panic!("the requested centre mesh did not complete");
                };
            assert_eq!(compile(&mut world, &mut streaming, SectionCell::Missing), 6);
            let mut east = Chunk::empty(1, 0);
            east.set(0, base + 4, 8, stone);
            streaming.insert_remote(&mut world, east);
            assert_eq!(
                compile(&mut world, &mut streaming, SectionCell::State(state(stone))),
                5
            );
            assert!(
                streaming
                    .section_visibility
                    .answers
                    .contains_key(&(1, 0, 0))
            );
            let gone = [(1, 0)].into_iter().collect();
            assert_eq!(
                MeshStreamer::take_remote_columns(&mut world, &gone).len(),
                1
            );
            streaming.forget_columns(&mut world, &gone);
            assert!(
                !streaming
                    .section_visibility
                    .answers
                    .contains_key(&(1, 0, 0)),
                "unload drops the answer and its lease"
            );
            assert_eq!(compile(&mut world, &mut streaming, SectionCell::Missing), 6);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_neighbours_interior_edit_stops_a_mesh_using_its_old_border_answer() {
        use crate::mesher::SectionPlanBuilder;
        use mod_api::SectionCell;

        const EAST: (i32, i32, i32) = (1, 0, 0);
        mod_platform::testing::with_mods(Vec::new().into(), || {
            let block = test_block_key();
            let state = |raw: u32| mod_api::__private::block_state_from_ordinal(block, raw as u16);
            let (mut world, mut streaming, stone) = fixture();
            let base = world.bottom_y();
            let marker = streaming
                .mesh_shared
                .context
                .read()
                .unwrap()
                .chunk
                .assets
                .registry
                .default_state("minecraft:cobblestone")
                .unwrap();
            let mut centre = Chunk::empty(0, 0);
            centre.set(15, base + 4, 8, stone);
            let mut east = Chunk::empty(1, 0);
            east.set(0, base + 4, 8, stone);
            east.set(8, base + 4, 8, marker.0);
            streaming.insert_remote(&mut world, centre);
            streaming.insert_remote(&mut world, east);
            streaming.refresh_section_visibility(&mut world, true);

            let centre_faces = |world: &mut World, streaming: &mut MeshStreamer| {
                for _ in 0..200 {
                    let (uploads, _) = streaming.update(world);
                    for request in streaming.take_visibility_requests() {
                        let mut answer = SectionPlanBuilder::new();
                        if request.key == EAST
                            && request.capture_input(state).cell([8, 4, 8])
                                == Ok(SectionCell::State(state(marker.0)))
                        {
                            answer.restrict([0, 4, 8], false, false, 0);
                        }
                        streaming.submit_visibility_answer(world, request, answer.finish());
                    }
                    let faces = uploads
                        .iter()
                        .find(|upload| {
                            upload.key == CENTRE && streaming.can_publish_mesh(world, upload)
                        })
                        .map(|upload| upload.mesh.solid.len() / 4);
                    streaming.free(uploads);
                    if let Some(faces) = faces {
                        return faces;
                    }
                    streaming.wait(Duration::from_millis(20));
                }
                panic!("the centre never had a current mesh");
            };
            assert_eq!(centre_faces(&mut world, &mut streaming), 6);

            world.dirty_sections.insert_mesh(CENTRE);
            streaming.update(&mut world);
            let request = streaming
                .take_visibility_requests()
                .into_iter()
                .find(|request| request.key == CENTRE)
                .expect("centre selected");
            let seq = request.seq;
            streaming.submit_visibility_answer(&mut world, request, None);
            // Same light behaviour and away from all borders: only the east section's raw input
            // changes. Its listener will no longer hide the stone on the border.
            world.set(24, base + 4, 8, BlockStateId(stone)).unwrap();
            assert!(!world.dirty_sections.contains(&CENTRE));
            assert!(world.dirty_sections.contains(&EAST));

            let completed = (0..200)
                .find_map(|_| {
                    let (uploads, _) = streaming.update(&mut world);
                    let completed = uploads
                        .iter()
                        .find(|upload| upload.key == CENTRE && upload.seq == seq)
                        .map(|upload| {
                            (
                                upload.mesh.solid.len() / 4,
                                streaming.can_publish_mesh(&world, upload),
                            )
                        });
                    streaming.free(uploads);
                    if completed.is_none() {
                        streaming.wait(Duration::from_millis(20));
                    }
                    completed
                })
                .expect("the centre worker finished before the east answer was collected");
            assert_eq!(
                completed,
                (6, false),
                "the old six-face mesh must not publish"
            );
            let east = streaming
                .take_visibility_requests()
                .into_iter()
                .find(|request| request.key == EAST)
                .expect("the east answer is still outstanding");
            streaming.submit_visibility_answer(&mut world, east, None);
            assert_eq!(centre_faces(&mut world, &mut streaming), 5);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_accepted_mesh_cannot_publish_after_policy_resource_or_column_changes() {
        use crate::mesher::SectionPlanBuilder;

        let (mut world, mut streaming, stone) = fixture();
        let base = world.bottom_y();
        let column = || {
            let mut chunk = Chunk::empty(0, 0);
            chunk.set(8, base + 4, 8, stone);
            chunk
        };
        let request = |world: &mut World, streaming: &mut MeshStreamer| {
            streaming.update(world);
            streaming
                .take_visibility_requests()
                .into_iter()
                .find(|request| request.key == CENTRE)
                .unwrap()
        };
        // Drive the mailbox in a fixed order rather than depending on which worker finishes first.
        let completed = |streaming: &MeshStreamer, seq| {
            streaming.chunks().send_auxiliary_done_for_test(MeshUpload {
                key: CENTRE,
                mesh: Default::default(),
                seq,
                halo: None,
            });
        };
        let upload = |world: &mut World, streaming: &mut MeshStreamer| -> MeshUpload {
            let uploads = streaming.update(world).0;
            let mut centre = uploads.into_iter().filter(|upload| upload.key == CENTRE);
            let upload = centre.next().expect("one accepted centre mesh");
            assert!(
                centre.next().is_none(),
                "a second, stale compile was accepted"
            );
            upload
        };
        streaming.insert_remote(&mut world, column());
        streaming.refresh_section_visibility(&mut world, true);
        let first = request(&mut world, &mut streaming);
        completed(&streaming, first.seq);
        let before_panic = upload(&mut world, &mut streaming);
        assert!(streaming.can_publish_mesh(&world, &before_panic));
        world.dirty_sections.insert(CENTRE);
        assert!(
            !streaming.can_publish_mesh(&world, &before_panic),
            "a changed dependency invalidates an accepted upload before its next compile"
        );
        world.dirty_sections.take_column(0, 0);
        // This is precisely the invalidation used when a listener panics during the same update.
        streaming.refresh_section_visibility(&mut world, true);
        assert!(!streaming.can_publish_mesh(&world, &before_panic));
        let mut builder = SectionPlanBuilder::new();
        builder.hide_terrain();
        let resource_a = request(&mut world, &mut streaming);
        let seq_a = resource_a.seq;
        // Install a cached answer under A without sending live work into the synthetic mailbox.
        let mut resource_a = resource_a;
        streaming.section_visibility.replace_answer(
            CENTRE,
            seq_a,
            builder.finish(),
            &mut resource_a.lease,
            &RESIDENT_BYTES,
        );
        let a = &resource_a.context;
        let context_b = Arc::new(WorkerContext {
            chunk: a.chunk.clone(),
            mesh: MeshWorkerContext {
                hidden_states: vec![true; a.chunk.assets.registry.len()].into(),
                ..a.mesh.clone()
            },
        });
        streaming.reload_resources(&mut world, context_b.clone());
        assert!(streaming.section_visibility.answers.is_empty());
        let resource_b = request(&mut world, &mut streaming);
        assert!(Arc::ptr_eq(&resource_b.context, &context_b));
        assert!(resource_a.context.mesh.hidden_states.is_empty());
        completed(&streaming, resource_b.seq);
        completed(&streaming, seq_a);
        let before_unload = upload(&mut world, &mut streaming);
        assert_eq!(before_unload.seq, resource_b.seq);
        assert!(streaming.can_publish_mesh(&world, &before_unload));
        let gone = [(0, 0)].into_iter().collect();
        MeshStreamer::take_remote_columns(&mut world, &gone);
        streaming.forget_columns(&mut world, &gone);
        streaming.insert_remote(&mut world, column());
        let reloaded = request(&mut world, &mut streaming);
        completed(&streaming, reloaded.seq);
        let current = upload(&mut world, &mut streaming);
        assert!(streaming.can_publish_mesh(&world, &current));
        assert!(!streaming.can_publish_mesh(&world, &before_unload));
    }
}
