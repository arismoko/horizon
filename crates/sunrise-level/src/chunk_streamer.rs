//! Portable chunk streaming: a worker pool generates, decorates, finishes and lights chunk columns
//! around the player, closest first. The game thread owns the world and only hands immutable `Arc`
//! snapshots to workers; results carry a sequence number so stale work is dropped.
use crate::fast_hash::{FxHashMap, FxHashSet};
use crate::mod_chunk_tickets::{ModTicket, ModTickets};
use crate::section_mask::SectionMask;
use crate::{
    chunk::Chunk,
    chunk_map::ChunkMembershipDelta,
    light::{LightChanges, LightTables, light_chunk},
    world::{ChunkResend, World},
};
use std::{
    collections::{BTreeSet, BinaryHeap, HashSet, VecDeque},
    sync::{Arc, Condvar, Mutex, MutexGuard, TryLockError, atomic::Ordering},
    time::{Duration, Instant},
};
use sunrise_core::{biome::BiomeZoom, position::ChunkPos};
use sunrise_threads::jobs::{Class, PerWorker, Rank, Registration, Source};
use sunrise_world::worldgen::decoration_order::DecorationOrder;

type Region = [[Option<Arc<Chunk>>; 3]; 3];

fn borrow(region: &Region) -> [[Option<&Chunk>; 3]; 3] {
    region
        .each_ref()
        .map(|row| row.each_ref().map(|chunk| chunk.as_deref()))
}

/// How long a worker waits with nothing to do before it gives back its
/// generator's scratch buffers ([`ChunkGenerator::idle`]).
pub const IDLE_RELEASE: std::time::Duration = std::time::Duration::from_secs(5);

/// Worker CPU time and job counts by kind, for benchmarks
/// (`examples/gen_light_bench.rs`). Off unless enabled: a relaxed load per
/// job then.
pub mod job_stats;
/// A chunk under generation, opaque to the streamer.
pub type Proto = Box<dyn std::any::Any + Send>;

/// A proto checked out for saving, before its worker takes ownership.
pub struct ProtoSave {
    pub pos: (i32, i32),
    pub proto: Proto,
}

/// Chunk generation in vanilla's stages: a neighbour-independent proto chunk
/// (biomes, noise, surface, carvers), decoration of a 3x3 neighbourhood
/// (FEATURES, which may write into neighbours), then conversion of a chunk
/// whose eight neighbours are decorated into a final column. Implementations
/// must be thread-safe; each worker of the shared pool
/// (`sunrise_threads::jobs`) keeps one `Worker` per streamer.
pub trait ChunkGenerator: Send + Sync + 'static {
    type Worker: Send + 'static;
    fn worker(&self) -> Self::Worker;
    /// `ChunkGenerator.getSpawnHeight` (specialized by flat generators).
    fn spawn_height(&self) -> i32 {
        64
    }
    /// The persistent wrapper supplies starts from their owning chunk, in every dimension.
    fn set_saved_starts(
        &self,
        _source: Arc<sunrise_world::worldgen::generator::SavedStructureStarts>,
    ) {
    }
    fn lod_sampler(self: Arc<Self>) -> Box<dyn LodSampler>
    where
        Self: Sized,
    {
        let worker = self.worker();
        Box::new(GeneratorSampler {
            generator: self,
            worker,
        })
    }
    /// An undecorated far-terrain column, without allocating gameplay chunks.
    fn lod_surface(
        &self,
        _worker: &mut Self::Worker,
        _x: i32,
        _z: i32,
    ) -> Option<crate::columns::Surface> {
        None
    }
    /// A previously saved final chunk, bypassing generation.
    fn load(&self, _x: i32, _z: i32) -> Option<Chunk> {
        None
    }
    /// What a loaded chunk needs from the generator before the game uses it,
    /// on the worker that loaded it.
    fn prepare_loaded(&self, _worker: &mut Self::Worker, _chunk: &Chunk) {}
    /// Finish a decoded full record on the owner before it enters the world.
    fn accept_loaded_chunk(&self, _chunk: &mut Chunk) {}
    /// The worker has had nothing to do for [`IDLE_RELEASE`]: give back what
    /// it keeps between jobs only to go faster.
    fn idle(&self, _worker: &mut Self::Worker) {}
    /// Every worker has had nothing to do for [`IDLE_RELEASE`]: give back the
    /// caches the workers share, which only spare work for chunks generated
    /// next to each other in a run. Results do not change.
    fn all_idle(&self) {}
    /// The neighbour-independent proto chunk of column `(x, z)`. An `Err` is a generation that
    /// cannot finish (a mod's code failed): the column is not published and is not asked for
    /// again.
    fn proto(&self, worker: &mut Self::Worker, x: i32, z: i32) -> Result<Proto, String>;
    /// Build the proto and, when needed, return its decoded disk record to the owner.
    fn worker_proto(
        &self,
        worker: &mut Self::Worker,
        x: i32,
        z: i32,
    ) -> Result<(Proto, Option<Box<crate::chunk::SavedProto>>), String> {
        Ok((self.proto(worker, x, z)?, None))
    }
    fn accept_loaded_proto(&self, proto: &mut Proto, saved: crate::chunk::SavedProto) -> bool {
        self.restore_proto(proto, saved);
        self.is_decorated(proto)
    }
    fn decorates(&self) -> bool {
        false
    }
    /// Decorate `region[4]`; the region is row-major (dz, dx) around it.
    fn decorate(&self, _worker: &mut Self::Worker, _region: &mut [Proto; 9]) {}
    /// Decorate `region[4]` with read-only resident columns from the `FEATURES`
    /// `STRUCTURE_STARTS` dependency ring (Chebyshev distance `2..=8`). Columns
    /// absent from `outer` have no resident proto or full column yet and read as
    /// empty `STRUCTURE_STARTS` proto chunks (`ChunkPyramid.GENERATION_PYRAMID`).
    fn decorate_with_outer(
        &self,
        worker: &mut Self::Worker,
        region: &mut [Proto; 9],
        _outer: &[Arc<Chunk>],
    ) {
        self.decorate(worker, region);
    }
    /// Whether a chunk next to final ones (loaded from the save, as the
    /// chunks of a world an older version made) is decorated, its features
    /// spilling into them (`proto_beside_final`, `final_writes`), as vanilla's
    /// are; otherwise it is left undecorated.
    fn decorates_beside_final(&self) -> bool {
        false
    }
    /// The proto chunk of a final chunk, for a decoration region to hold: its
    /// blocks are `snapshot`'s.
    fn proto_beside_final(&self, _worker: &mut Self::Worker, _snapshot: &Chunk) -> Option<Proto> {
        None
    }
    /// What decoration wrote into the proto chunk of a final chunk: the blocks
    /// that differ from `snapshot`, as `([x, y, z], state)`.
    fn final_writes(&self, _snapshot: &Chunk, _proto: &Proto) -> Vec<([i32; 3], u32)> {
        Vec::new()
    }
    /// The block entities decoration made at the blocks `writes` put into a
    /// final chunk (a chest of a structure piece, a spawner), with their
    /// contents.
    fn final_block_entities(
        &self,
        _entries: &[(&Proto, &[([i32; 3], u32)])],
    ) -> Vec<((i32, i32, i32), crate::chunk::block_entity::BlockEntity)> {
        Vec::new()
    }
    fn finish(&self, proto: Proto) -> Chunk;
    /// Whether `proto` has been decorated (a saved one can come back decorated).
    fn is_decorated(&self, _proto: &Proto) -> bool {
        false
    }
    /// A decorated proto chunk's blocks as a column, to light a finished
    /// neighbour against (vanilla lights with neighbours at FEATURES).
    fn light_view(&self, _proto: &Proto) -> Option<Chunk> {
        None
    }
    /// A decorated proto chunk's column sections (`Chunk::sections`
    /// indexes) holding a block other than air: what its neighbours' light
    /// layers count once it is at INITIALIZE_LIGHT.
    fn non_air_sections(&self, _proto: &Proto) -> Option<SectionMask> {
        None
    }
    /// What the generator's shared caches hold: (name, entries, bytes, if
    /// known), for `sunrise-ctl memory`.
    fn cache_census(&self) -> Vec<(&'static str, usize, Option<usize>)> {
        Vec::new()
    }
    /// A proto chunk's heap bytes by part, and its sections by bits per
    /// block, for `sunrise-ctl memory` (`ProtoChunk::heap_bytes`).
    fn proto_heap_bytes(&self, _proto: &Proto) -> ([usize; 6], [usize; 16]) {
        ([0; 6], [0; 16])
    }
    fn saved_proto(&self, _proto: &Proto) -> Option<crate::chunk::SavedProto> {
        None
    }
    /// Put a saved proto chunk's state back onto `proto`, generated again.
    fn restore_proto(&self, _proto: &mut Proto, _saved: crate::chunk::SavedProto) {}
    /// Save proto chunks decoration wrote to, so that they come back as they
    /// were: generating them again would lose those writes.
    fn save_protos(&self, _protos: &[&Proto]) {}
    /// Whether proto snapshots must go through the owner before background encoding.
    fn save_protos_on_owner(&self) -> bool {
        false
    }
    /// Save a column that finished after the player moved away: its
    /// neighbours' decoration is done, so it can't be generated again.
    fn save_finished(&self, _chunk: Chunk) {}
}
pub type LodDetail = ((i32, i32), Arc<[crate::lod_data::Column; 256]>);

/// Independent, thread-owned far-terrain sampling state.
pub trait LodSampler: Send {
    /// Independent sampling state; native detail generation stays with the owner.
    fn fork(&self) -> Option<Box<dyn LodSampler>> {
        None
    }
    fn request_detail(&mut self, _chunks: &[(i32, i32)], _center: [i32; 2]) {}
    fn drain_details(&mut self) -> Vec<LodDetail> {
        Vec::new()
    }
    /// A loaded chunk's summary, made on the far-terrain worker.
    fn observed(&mut self, _key: (i32, i32), _columns: &Arc<[crate::lod_data::Column; 256]>) {}
    /// Reduced far detail (`graphics::FarDetail::Reduced`): `grid` may then
    /// approximate wide cells.
    fn set_approximate(&mut self, _approximate: bool) {}
    fn sample(&mut self, x: i32, z: i32) -> Option<crate::columns::Surface>;
    fn column(&mut self, x: i32, z: i32) -> crate::lod_data::Column {
        crate::lod_data::Column::from_surface(self.sample(x, z))
    }
    fn reduced_column(&mut self, x: i32, z: i32, _width: i32) -> crate::lod_data::Column {
        self.column(x, z)
    }
    /// Whether a column since the last call was left empty because its data
    /// is still on its way (a BlueMap tile downloading, nothing coarser to
    /// stand in): that sample isn't drawn, what it would replace is
    /// (`Lod::finish_any_sample`), and it is taken again later.
    fn waited(&mut self) -> bool {
        false
    }
    /// `side` x `side` columns from (`x`, `z`), `stride` blocks apart; none
    /// when sampling failed and should be tried again later.
    fn grid(&mut self, x: i32, z: i32, stride: i32, side: usize) -> Vec<crate::lod_data::Column> {
        let mut columns = Vec::with_capacity(side * side);
        for dz in 0..side {
            for dx in 0..side {
                columns.push(self.reduced_column(
                    x + dx as i32 * stride,
                    z + dz as i32 * stride,
                    stride,
                ));
            }
        }
        columns
    }
}
pub trait LodSource: Send + Sync {
    fn sampler(self: Arc<Self>) -> Box<dyn LodSampler>;
    /// Whether its samplers' `observed` chunks become its own (a server's
    /// kept terrain): tiles sampled without one are sampled again.
    fn keeps_observed(&self) -> bool {
        false
    }
}
struct GeneratorSampler<G: ChunkGenerator> {
    generator: Arc<G>,
    worker: G::Worker,
}
impl<G: ChunkGenerator> LodSampler for GeneratorSampler<G> {
    fn sample(&mut self, x: i32, z: i32) -> Option<crate::columns::Surface> {
        self.generator.lod_surface(&mut self.worker, x, z)
    }
}
impl<G: ChunkGenerator> LodSource for G {
    fn sampler(self: Arc<Self>) -> Box<dyn LodSampler> {
        ChunkGenerator::lod_sampler(self)
    }
}
/// The generator's saving, for the streamer (which doesn't know its type).
trait Saves: LodSource {
    fn accept_loaded_chunk(&self, chunk: &mut Chunk);
    fn accept_loaded_proto(&self, proto: &mut Proto, saved: crate::chunk::SavedProto) -> bool;
    fn save_protos_on_owner(&self) -> bool;
    fn save_protos(&self, protos: &[&Proto]);
    fn save_finished(&self, chunk: Chunk);
}
impl<G: ChunkGenerator> Saves for G {
    fn save_protos(&self, protos: &[&Proto]) {
        ChunkGenerator::save_protos(self, protos);
    }
    fn save_finished(&self, chunk: Chunk) {
        ChunkGenerator::save_finished(self, chunk);
    }
    fn accept_loaded_chunk(&self, chunk: &mut Chunk) {
        ChunkGenerator::accept_loaded_chunk(self, chunk);
    }
    fn accept_loaded_proto(&self, proto: &mut Proto, saved: crate::chunk::SavedProto) -> bool {
        ChunkGenerator::accept_loaded_proto(self, proto, saved)
    }
    fn save_protos_on_owner(&self) -> bool {
        ChunkGenerator::save_protos_on_owner(self)
    }
}

/// Portable worker resources needed for chunk generation and lighting.
#[derive(Clone)]
pub struct ChunkWorkerContext {
    pub assets: Arc<crate::assets::Assets>,
    pub biomes: Arc<crate::biome::Biomes>,
    pub light: Arc<LightTables>,
}

/// Uninhabited payload for a streamer with no companion job half.
pub enum NoPayload {}
/// Uninhabited completion for a streamer with no companion job half.
pub enum NoOutput {}
/// Companion executor for standalone portable streamers.
pub struct NoAuxiliary;

/// Companion job executor sharing `ChunkStreamer<A>`'s single pool source, queue, and mailbox.
pub trait AuxiliaryJobs: Send + Sync + 'static {
    type Payload: Send + 'static;
    type Output: Send + 'static;
    type Worker: Send + 'static;
    fn worker(&self) -> Self::Worker;
    fn run(&self, payload: Self::Payload, worker: &mut Self::Worker) -> Option<Self::Output>;
    fn idle(&self, worker: &mut Self::Worker);
    fn all_idle(&self);
    fn classify(payload: &Self::Payload) -> (usize, &'static str);
}

impl AuxiliaryJobs for NoAuxiliary {
    type Payload = NoPayload;
    type Output = NoOutput;
    type Worker = ();
    fn worker(&self) {}
    fn run(&self, payload: NoPayload, _: &mut ()) -> Option<NoOutput> {
        match payload {}
    }
    fn idle(&self, _: &mut ()) {}
    fn all_idle(&self) {}
    fn classify(payload: &NoPayload) -> (usize, &'static str) {
        match *payload {}
    }
}

/// Tracks a light job until it leaves the queue or its result is received.
struct LightJob {
    seq: u64,
    /// Edit sequence when submitted, or `None` if the column was unloaded or replaced.
    snapshot: Option<u64>,
}

enum Job<P = NoPayload> {
    Generate(i32, i32),
    /// A region to decorate, with the final chunks in it (region index and
    /// the chunk as it is): their slots hold nothing until the worker makes
    /// their proto chunks, plus read-only resident columns in the `FEATURES`
    /// `STRUCTURE_STARTS` dependency ring (Chebyshev distance `2..=8`).
    Decorate(
        i32,
        i32,
        Box<[Proto; 9]>,
        Vec<(usize, Chunk)>,
        Vec<Arc<Chunk>>,
    ),
    Finish(Proto),
    Light(i32, i32, Box<Region>, u64, bool),
    /// Proto chunks checked out to be saved (`ChunkGenerator::save_protos`),
    /// off the main thread like vanilla's IOWorker.
    Save(Vec<ProtoSave>),
    /// A proto chunk checked out for its light view (`light_region`).
    LightView(i32, i32, Proto),
    /// Companion job payload stored inline in the priority queue.
    Auxiliary(P),
}
/// What decoration wrote into the final chunks of its region.
#[derive(Default)]
pub struct FinalWrites {
    blocks: Vec<([i32; 3], u32)>,
    block_entities: Vec<((i32, i32, i32), crate::chunk::block_entity::BlockEntity)>,
}
pub enum Done {
    Generated(Chunk),
    /// A proto chunk, its decoration flag, and any record deferred to the owner.
    Proto(i32, i32, Proto, bool, Option<Box<crate::chunk::SavedProto>>),
    /// A column whose generation failed, and why.
    Failed(i32, i32, String),
    /// A decorated region, the region indices of its final chunks and what
    /// decoration wrote into them.
    Decorated(i32, i32, Box<[Proto; 9]>, Vec<usize>, FinalWrites),
    Lit(i32, i32, Vec<crate::chunk::Light>, u64),
    Saved(Vec<((i32, i32), Proto)>),
    /// A proto chunk back from `Job::LightView`, with its view.
    LightView(i32, i32, Proto, Option<Chunk>),
}

/// Single-mailbox completion carrying either a chunk result or an inline companion output.
pub enum Mail<O = NoOutput> {
    Chunk(Done),
    Auxiliary(O),
}

struct Queued<P = NoPayload> {
    priority: i64,
    /// The column the job is for and what it adds to the column's distance
    /// (`ChunkStreamer::submit`): the priority is computed again from them when
    /// the centre moves (`ChunkStreamer::requeue`). None for saves, which go first.
    at: Option<(i32, i32)>,
    bias: i64,
    order: u64,
    job: Job<P>,
}
impl<P> PartialEq for Queued<P> {
    fn eq(&self, o: &Self) -> bool {
        self.priority == o.priority && self.order == o.order
    }
}
impl<P> Eq for Queued<P> {}
impl<P> PartialOrd for Queued<P> {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl<P> Ord for Queued<P> {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        // BinaryHeap is a max-heap: lower priority value = more urgent.
        o.priority
            .cmp(&self.priority)
            .then(o.order.cmp(&self.order))
    }
}
struct Shared<P, O> {
    queue: Mutex<BinaryHeap<Queued<P>>>,
    done: Mailbox<O>,
    aborted: std::sync::atomic::AtomicBool,
}

/// The chunks `wanted` in the order they decorate: by rank, lowest first. No
/// two share a rank, so only the chunks without one yet need their distance:
/// those no ticket reached (forced and extra areas) are ranked after every
/// other, nearest first. The pass that pops them stops once the workers are
/// busy, usually long before the last, so a heap orders only what it takes. A
/// server with players spread over the map holds tens of thousands of proto
/// chunks; measuring and sorting them all every tick took a fifth of its tick.
fn decoration_queue(
    order: &mut DecorationOrder,
    wanted: impl Iterator<Item = (i32, i32)>,
    distance: impl Fn(i32, i32) -> i64,
) -> std::collections::BinaryHeap<std::cmp::Reverse<(u64, i32, i32)>> {
    let mut ranked = Vec::new();
    let mut fresh = Vec::new();
    for (x, z) in wanted {
        match order.rank(x, z) {
            Some(rank) => ranked.push(std::cmp::Reverse((rank, x, z))),
            None => fresh.push((distance(x, z), x, z)),
        }
    }
    fresh.sort_unstable();
    for (_, x, z) in fresh {
        ranked.push(std::cmp::Reverse((order.rank_or_append(x, z), x, z)));
    }
    std::collections::BinaryHeap::from(ranked)
}

/// Finished jobs, from the workers to the game thread, in the order they
/// finished. Not an `mpsc` channel: its receiver waits for a sender it finds
/// half way through a send, and a worker descheduled there on a busy machine
/// held the frame for a scheduler quantum (10 to 30 ms, the RD 32 ocean
/// flight's 1% lows). The frame takes what is there without waiting; while a
/// worker holds the lock, it takes it the next frame.
struct Mailbox<O = NoOutput> {
    done: Mutex<VecDeque<Mail<O>>>,
    arrived: Condvar,
}
impl<O> Default for Mailbox<O> {
    fn default() -> Self {
        Self {
            done: Mutex::new(VecDeque::new()),
            arrived: Condvar::new(),
        }
    }
}
impl<O> Mailbox<O> {
    fn send(&self, done: Mail<O>) {
        self.done.lock().unwrap().push_back(done);
        self.arrived.notify_one();
    }
    fn try_lock(&self) -> Option<MutexGuard<'_, VecDeque<Mail<O>>>> {
        match self.done.try_lock() {
            Ok(done) => Some(done),
            Err(TryLockError::WouldBlock) => None,
            Err(TryLockError::Poisoned(e)) => panic!("{e}"),
        }
    }
    /// Every finished job, or none while a worker is sending.
    fn take(&self) -> VecDeque<Mail<O>> {
        self.try_lock()
            .map(|mut done| std::mem::take(&mut *done))
            .unwrap_or_default()
    }
    fn try_recv(&self) -> Option<Mail<O>> {
        self.try_lock()?.pop_front()
    }
    fn wait(
        &self,
        timeout: Duration,
        mut now: impl FnMut() -> Instant,
    ) -> (MutexGuard<'_, VecDeque<Mail<O>>>, Duration) {
        let done = self.done.lock().unwrap();
        if !done.is_empty() || timeout.is_zero() {
            return (done, Duration::ZERO);
        }
        // With the queue locked and empty, time only the condition-variable wait.
        let start = now();
        let (done, _) = self
            .arrived
            .wait_timeout_while(done, timeout, |done| done.is_empty())
            .unwrap();
        (done, now() - start)
    }
    fn recv_timeout(&self, timeout: Duration) -> Option<Mail<O>> {
        self.wait(timeout, Instant::now).0.pop_front()
    }
}
/// [`ChunkState::proto_census`].
#[derive(Debug, Default)]
pub struct ProtoCensus {
    pub protos: usize,
    /// Packed blocks, section palettes and lists, heightmaps, other terrain
    /// data, biomes, the box and its structure lists.
    pub bytes: [usize; 6],
    pub sections_by_bits: [usize; 16],
    pub light_views: usize,
    pub light_view_bytes: usize,
    /// `ChunkGenerator::cache_census`.
    pub caches: Vec<(&'static str, usize, Option<usize>)>,
}
fn plan_unloads(world: &World, unload: impl Fn(i32, i32) -> bool) -> HashSet<(i32, i32)> {
    world
        .chunks
        .keys()
        .copied()
        .filter(|&(x, z)| unload(x, z))
        .collect()
}

fn commit_unloads(world: &mut World, keys: &mut HashSet<(i32, i32)>) -> Vec<Arc<Chunk>> {
    let mut removed = Vec::with_capacity(keys.len());
    keys.retain(|&key| {
        let Some(chunk) = world.unload_chunk(key) else {
            return false;
        };
        removed.push(chunk);
        true
    });
    removed
}

/// A column arrived or disappeared; every resident reader's raw halo changed.
pub fn dirty_visibility_columns(world: &mut World, x: i32, z: i32) {
    let mask = SectionMask::below(world.layout().sections());
    for dz in -1..=1 {
        for dx in -1..=1 {
            let n = (x + dx, z + dz);
            if world.chunks.contains_key(&n) {
                world.dirty_sections.insert_column_mask(n, mask);
            }
        }
    }
}

/// An update paused before removing its out-of-range live chunks.
pub struct ChunkUpdate {
    sweep: bool,
    far: HashSet<(i32, i32)>,
    paused: bool,
}
impl ChunkUpdate {
    pub fn paused() -> Self {
        Self {
            sweep: false,
            far: HashSet::new(),
            paused: true,
        }
    }
    pub fn is_paused(&self) -> bool {
        self.paused
    }
    pub fn is_sweep(&self) -> bool {
        self.sweep
    }
    /// Columns selected for removal. After `prepare_commit`, only the columns actually removed.
    pub fn unloading_columns(&self) -> &HashSet<(i32, i32)> {
        &self.far
    }
    /// Leave the live columns alone when their save could not be submitted.
    pub fn keep_columns(&mut self) {
        self.far.clear();
    }
    /// Keep only columns that may be committed by this sweep.
    pub fn retain_unloading_columns(&mut self, can_unload: impl FnMut(&(i32, i32)) -> bool) {
        self.far.retain(can_unload);
    }
    pub fn suppress_unloads(&mut self) {
        self.sweep = false;
        self.far.clear();
    }
}

/// Result of processing generation results, pausing before a late full save.
pub enum ChunkPlanStep<O = NoOutput> {
    Ready(ChunkUpdate),
    Save {
        chunk: Chunk,
        resume: ChunkPlanResume<O>,
    },
}

/// A save step is pending, ready for proto submission, or paused at a late full chunk.
pub enum SaveStep {
    Pending,
    Full(Chunk),
    Protos(Vec<ProtoSave>),
}

/// Planning work retained while the caller submits a deferred full-chunk save.
pub struct ChunkPlanResume<O = NoOutput> {
    sweep: bool,
    deferred: std::vec::IntoIter<Mail<O>>,
    received: VecDeque<Mail<O>>,
}

/// A streaming update paused before scheduling generation after proto checkout.
pub struct ChunkCommit {
    removed: Vec<Arc<Chunk>>,
    sweep_far: HashSet<(i32, i32)>,
    paused: bool,
}
impl ChunkCommit {
    pub fn paused() -> Self {
        Self {
            removed: Vec::new(),
            sweep_far: HashSet::new(),
            paused: true,
        }
    }
    pub fn is_paused(&self) -> bool {
        self.paused
    }
    pub fn sweep_far(&self) -> &HashSet<(i32, i32)> {
        &self.sweep_far
    }
    pub fn into_removed(self) -> Vec<Arc<Chunk>> {
        self.removed
    }
}

/// Concrete portable chunk-streaming state independent of any companion job type.
pub struct ChunkState {
    spawn_height: i32,
    /// Columns generated and saved as they were left behind: new when they
    /// next arrive, though read from the save (`World::loaded_chunks`).
    dropped_new: FxHashSet<(i32, i32)>,
    non_air_sections: Arc<dyn Fn(&Proto) -> Option<SectionMask> + Send + Sync>,
    /// `ChunkGenerator::proto_heap_bytes`, for the memory census.
    proto_bytes: Arc<dyn Fn(&Proto) -> ([usize; 6], [usize; 16]) + Send + Sync>,
    /// `ChunkGenerator::cache_census`.
    caches: Arc<dyn Fn() -> Vec<(&'static str, usize, Option<usize>)> + Send + Sync>,
    /// The workers its in-flight work is sized for.
    workers: usize,
    order: u64,
    /// Ticks since the columns and protos that fell out of range were swept.
    scan_age: u8,
    /// The centre and radius the sweep was last reset for.
    scan_area: ((i32, i32), i32),
    generating: FxHashSet<(i32, i32)>,
    /// Columns whose generation failed or panicked (`Done::Failed`), contained by the engine where
    /// vanilla would crash: never published, never generated again while this streaming lives, so
    /// they and the columns that need them around them are never FULL (`full_column_failed`).
    failed: FxHashSet<(i32, i32)>,
    generation_scan: GenerationScan,
    /// Proto chunks: `None` while checked out to a decoration or save job.
    protos: FxHashMap<(i32, i32), Option<Proto>>,
    /// The light jobs' views of proto chunks past the area (`light_region`),
    /// built once per proto: each is a neighbour of up to three columns and
    /// waits in failed regions until the last one loads. Dropped whenever
    /// the proto is checked out, replaced or removed.
    light_views: FxHashMap<(i32, i32), Arc<Chunk>>,
    decorated: FxHashSet<(i32, i32)>,
    decorating: usize,
    /// The final chunks a running decoration writes into: another one that
    /// reaches them waits, as vanilla's features run one after the other and
    /// each sees what the one before placed.
    decorating_finals: HashSet<(i32, i32)>,
    /// Save jobs in flight.
    saving: usize,
    /// Proto chunks checked out to a worker for their light view.
    viewing: FxHashSet<(i32, i32)>,
    /// Vanilla's decoration order around the player, and the centre and
    /// radius it was last ranked for.
    decoration_order: DecorationOrder,
    order_area: Option<((i32, i32), i32)>,
    /// Set when async chunk requests are added/removed so `decoration_order` prunes entries
    /// whose requests have completed or been cancelled even while the player is stationary or on
    /// a parked level.
    decoration_order_dirty: bool,
    /// Something decoration reads may have changed since the last pass, so the
    /// finalize and decoration passes are worth running: a proto chunk, a
    /// decorated key, a finished column, the swept area, or a proto save came
    /// or went. Light and mesh results do not set it.
    decoration_changed: bool,
    /// `World::chunks` membership when the last decoration pass ran: a column
    /// that arrives or leaves outside streaming (a server's packet, a far
    /// teleport) can make a proto ready too.
    decoration_revision: u64,
    /// The ranges the last finalize/decoration pass ran for. Tracked apart
    /// from `GenerationScan`, whose remembered ranges can lag while a scan
    /// returns at capacity: an extra area moving away and back must still
    /// wake the pass.
    decoration_basis: Option<DecorationBasis>,
    /// What the queued jobs' priorities were computed from (`requeue`): the
    /// centre, the band, the extra areas, and the forced chunks.
    queue_basis: forced_ring::Basis,
    finishing: FxHashSet<(i32, i32)>,
    decorates: bool,
    /// `ChunkGenerator::decorates_beside_final`.
    saves_beside_final: bool,
    lighting: FxHashMap<(i32, i32), LightJob>,
    /// Chunks whose light is current.
    lit: FxHashSet<(i32, i32)>,
    /// Proto chunks decoration wrote to since they were generated or saved.
    written: FxHashSet<(i32, i32)>,
    saves: Arc<dyn Saves>,
    /// Per-update collection scratch, retained to avoid allocator churn while
    /// the view is moving or many columns are dirty.
    light_scratch: Vec<(i64, i32, i32)>,
    gone_scratch: Vec<(i32, i32)>,
    finalize_scratch: Vec<(i32, i32)>,
    sequence: u64,
    pub center: (i32, i32),
    /// The view distance around `center`; below 0, no area there (a dedicated
    /// server nobody is on).
    pub radius: i32,
    /// Chunks kept loaded whatever the distance: the `/forceload`ed ones and
    /// those a mod holds a ticket for. Changed only through `set_forced_area`,
    /// `clear_forced`, `set_mod_forced` and `load_forced`, through `settle`.
    pub forced: HashSet<(i32, i32)>,
    /// The `/forceload`ed chunks, the ones saved in `chunk_tickets.dat`.
    commanded: HashSet<(i32, i32)>,
    /// The same membership in `ForceLoadCommand.listForceLoad`'s signed packed order.
    ordered_commanded: BTreeSet<i64>,
    mod_tickets: ModTickets,
    /// No work is scheduled while paused (title screen).
    pub paused: bool,
    /// Dimension flag: sky light.
    pub sky_light: bool,
    /// `BiomeManager`'s zoom seed (the hashed world seed).
    pub biome_zoom: BiomeZoom,
    /// Incremental relighting of block edits on lit chunks (see `light::relight`).
    light_tables: Arc<LightTables>,
    /// Applied edits by sequence, replayed onto light jobs snapshotted before them.
    light_log: Vec<(u64, (i32, i32, i32))>,
    light_edit_sequence: u64,
    pub generated_total: u64,
    /// Chunks come from a server (`insert_remote`/`forget`): nothing is
    /// generated and columns are only unloaded when the server says so.
    pub remote: bool,
    /// This server sends bulk block edits as whole columns whose light is
    /// from before its relight (a Sunrise server): a resent resident column
    /// is lit here. Any other server's packet carries the light vanilla
    /// applies, so it is not lit again (a proxy's next server resends its
    /// whole world at the same coordinates).
    pub relight_resent: bool,
    /// More areas to keep loaded (centre chunk, radius), e.g. around the
    /// players of a server. Each is a square, like vanilla's ticket levels:
    /// chunks finish out to `radius + 1` (Chebyshev) and generation reaches
    /// past that as around the player. They are generated and kept but not meshed.
    pub extra: Vec<((i32, i32), i32)>,
    /// `TicketType.UNKNOWN` tickets at FULL's level (`ServerChunkCache.getChunkFutureMainThread`),
    /// each with its ticks left: `begin_full_column` places them, `purge_unknown_tickets` expires
    /// them, and `hold_unknown_tickets` keeps their columns through an owner update.
    unknown: Vec<((i32, i32), i8)>,
    /// `ServerChunkCache`'s four most recent FULL lookups, newest first (`lastChunkPos`). A cell:
    /// a resident read through `&self` fills it too (`note_full_read`, `getChunkNow`).
    recent_full: std::cell::Cell<[Option<(i32, i32)>; 4]>,
    /// `extra`'s squares as a grid, made again when `extra` changes.
    cover: Option<ExtraCover>,
    /// Set while `update` runs, after it refreshed `cover`: nothing else can
    /// change `extra` meanwhile, so `cover` answers without comparing them.
    cover_current: bool,
    /// The combined `forced` set as each column's distance to the nearest one, made again when
    /// they change; read like `cover`, while `cover_current`.
    forced_cover: Option<ForcedCover>,
    /// The pregeneration band (`set_band`).
    band: Option<Band>,
    /// The columns and the next row the band's decoration ranks were given
    /// for (`set_band`).
    band_ranked: Option<(i32, i32, i32)>,
    /// Active non-blocking mod chunk requests at `FULL` (level 33): generated, decorated, lit,
    /// and kept resident until their request completes or is cancelled, sorted strictly after
    /// player views, extra areas, forced chunks, and pregeneration bands.
    async_requests: HashSet<(i32, i32)>,
    /// Spatial cover over `async_requests`, rebuilt in `begin_plan_update` when non-empty.
    async_request_cover: Option<ForcedCover>,
    /// Dirty resident visibility halos immediately upon column installation when listening.
    dirty_visibility_on_load: bool,
}

pub struct ChunkStreamer<A: AuxiliaryJobs = NoAuxiliary> {
    state: ChunkState,
    shared: Arc<Shared<A::Payload, A::Output>>,
    /// Results received while `save` waited, for the next `update`.
    deferred: Vec<Mail<A::Output>>,
    /// This streamer's place in the shared pool (`sunrise_threads::jobs`);
    /// dropping it waits for its running jobs and must remain the last field.
    jobs: Registration,
}

impl<A: AuxiliaryJobs> std::ops::Deref for ChunkStreamer<A> {
    type Target = ChunkState;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}
impl<A: AuxiliaryJobs> std::ops::DerefMut for ChunkStreamer<A> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

/// The ranges a finalize/decoration pass ran for, compared every commit so
/// that a change wakes the next pass even when `GenerationScan` could not
/// record it (a scan returns at capacity without remembering its ranges).
#[derive(Default)]
struct DecorationBasis {
    center: (i32, i32),
    radius: i32,
    extra: Vec<((i32, i32), i32)>,
    forced: HashSet<(i32, i32)>,
    band: Option<Band>,
    async_requests: HashSet<(i32, i32)>,
}
/// Snapshot only after a scan found no missing candidates. Public range fields
/// and `World::chunks` can change without going through ChunkStreamer, so compare
/// their values and structural membership token rather than a frame timeout.
///
/// A scan also keeps every candidate it found, nearest first (`plan`): while
/// the ranges stay the same and no column leaves (`World::chunks` removals, or
/// `invalidate` for proto chunks dropped), no column becomes wanted, so the
/// next batches are the plan's entries still missing. A full scan looks at
/// every column of the areas (thousands at render distance 32), and one ran
/// whenever a worker finished: it took a quarter of the thread that runs
/// `ChunkStreamer::update` while a world loaded.
#[derive(Default)]
struct GenerationScan {
    exhausted: bool,
    center: (i32, i32),
    radius: i32,
    extra: Vec<((i32, i32), i32)>,
    forced: HashSet<(i32, i32)>,
    band: Option<Band>,
    async_requests: HashSet<(i32, i32)>,
    chunk_revision: u64,
    plan: Vec<(i64, i32, i32)>,
    /// Entries before it are no longer missing.
    next: usize,
    /// What `plan` was made for: the ranges and decoration; the chunk
    /// map's revision is `chunk_revision`.
    plan_basis: Option<bool>,
}
impl GenerationScan {
    fn same_ranges(&self, ranges: &Ranges<'_>) -> bool {
        let same_async = match ranges.async_requests {
            Some(req) => self.async_requests == *req.chunks,
            None => self.async_requests.is_empty(),
        };
        self.center == ranges.center
            && self.radius == ranges.radius
            && self.extra == ranges.extra
            && self.forced == *ranges.forced
            && self.band.as_ref() == ranges.band
            && same_async
    }
    fn unchanged(&self, ranges: &Ranges<'_>, world: &World) -> bool {
        self.exhausted
            && self.same_ranges(ranges)
            && self.chunk_revision == world.chunks.membership_revision()
    }
    /// A column may be wanted again (a proto chunk was dropped): the next
    /// call scans the areas again.
    fn invalidate(&mut self) {
        self.exhausted = false;
        self.plan_basis = None;
    }
    /// Whether `plan` still holds every candidate: same ranges, and no
    /// column left the chunk map since it was made.
    fn plan_holds(&mut self, input: &GenerationInputs<'_>) -> bool {
        if self.plan_basis != Some(input.decorates) || !self.same_ranges(&input.ranges) {
            return false;
        }
        let revision = input.world.chunks.membership_revision();
        if revision != self.chunk_revision {
            let Some(mut changes) = input.world.chunks.deltas_since(self.chunk_revision) else {
                return false;
            };
            if !changes.all(|(_, change)| matches!(change, ChunkMembershipDelta::Added(_))) {
                return false;
            }
            self.chunk_revision = revision;
        }
        true
    }
    fn remember(&mut self, ranges: Ranges<'_>, world: &World) {
        self.exhausted = true;
        self.center = ranges.center;
        self.radius = ranges.radius;
        self.extra.clear();
        self.extra.extend_from_slice(ranges.extra);
        self.forced.clone_from(ranges.forced);
        self.band = ranges.band.copied();
        match ranges.async_requests {
            Some(req) => self.async_requests.clone_from(req.chunks),
            None => self.async_requests.clear(),
        }
        self.chunk_revision = world.chunks.membership_revision();
    }
}
struct GenerationInputs<'a> {
    ranges: Ranges<'a>,
    world: &'a World,
    generating: &'a FxHashSet<(i32, i32)>,
    failed: &'a FxHashSet<(i32, i32)>,
    protos: &'a FxHashMap<(i32, i32), Option<Proto>>,
    finishing: &'a FxHashSet<(i32, i32)>,
    decorates: bool,
    remote: bool,
}
impl GenerationScan {
    /// None means the scan was skipped; Some preserves the original candidate
    /// ordering and bounded batch, including an empty exhausted scan.
    fn candidates(
        &mut self,
        input: GenerationInputs<'_>,
        capacity: usize,
    ) -> Option<Vec<(i64, i32, i32)>> {
        // Generation (load or proto), closest first, bounded in flight.
        let (margin, reach) = Ranges::margins(input.decorates);
        if input.remote
            || input.generating.len() >= capacity
            || self.unchanged(&input.ranges, input.world)
        {
            return None;
        }
        let missing = |&(_, x, z): &(i64, i32, i32)| {
            !input.world.chunks.contains_key(&(x, z))
                && !input.generating.contains(&(x, z))
                && !input.failed.contains(&(x, z))
                && !input.protos.contains_key(&(x, z))
                && !input.finishing.contains(&(x, z))
        };
        if self.plan_holds(&input) {
            while self.next < self.plan.len() && !missing(&self.plan[self.next]) {
                self.next += 1;
            }
            let room = capacity - input.generating.len();
            let batch: Vec<_> = self.plan[self.next..]
                .iter()
                .filter(|c| missing(c))
                .take(room)
                .copied()
                .collect();
            // Past the plan's end, the full scan below decides whether
            // anything is left (and remembers when nothing is).
            if !batch.is_empty() {
                self.exhausted = false;
                return Some(batch);
            }
        }
        {
            let mut wanted = Vec::new();
            // Each area's square of columns, as one run per row. Players'
            // areas overlap (a server's players often stand together), so
            // the runs of a row are merged and each column is looked at once.
            let mut runs: Vec<(i32, i32, i32)> = Vec::new();
            for (center, radius) in std::iter::once((input.ranges.center, input.ranges.radius))
                .chain(input.ranges.extra.iter().copied())
            {
                let r = radius + margin.max(reach + 1);
                runs.extend((center.1 - r..=center.1 + r).map(|z| (z, center.0 - r, center.0 + r)));
            }
            if let Some(band) = input.ranges.band {
                let r = margin.max(reach + 1);
                runs.extend((band.z0 - r..=band.z1 + r).map(|z| (z, band.x0 - r, band.x1 + r)));
            }
            runs.sort_unstable();
            let mut runs = runs.into_iter().peekable();
            while let Some((z, x0, mut x1)) = runs.next() {
                while let Some((_, _, next_x1)) =
                    runs.next_if(|&(nz, nx0, _)| nz == z && nx0 <= x1.saturating_add(1))
                {
                    x1 = x1.max(next_x1);
                }
                for x in x0..=x1 {
                    if !input.ranges.requested(x, z, input.decorates)
                        || input.world.chunks.contains_key(&(x, z))
                        || input.generating.contains(&(x, z))
                        || input.failed.contains(&(x, z))
                        || input.protos.contains_key(&(x, z))
                        || input.finishing.contains(&(x, z))
                    {
                        continue;
                    }
                    wanted.push((input.ranges.priority(x, z), x, z));
                }
            }
            // Forced chunks and their generation neighbourhood: the ring that
            // is decorated (or finished, without decoration) and the one its
            // 3x3 needs.
            let forced_reach = if input.decorates {
                FORCED_TERRAIN_RING
            } else {
                FORCED_FEATURES_RING
            };
            for &(fx, fz) in input.ranges.forced {
                for dz in -forced_reach..=forced_reach {
                    for dx in -forced_reach..=forced_reach {
                        let (x, z) = (fx + dx, fz + dz);
                        if input.world.chunks.contains_key(&(x, z))
                            || input.generating.contains(&(x, z))
                            || input.failed.contains(&(x, z))
                            || input.protos.contains_key(&(x, z))
                            || input.finishing.contains(&(x, z))
                        {
                            continue;
                        }
                        wanted.push((input.ranges.priority(x, z), x, z));
                    }
                }
            }
            // Non-blocking mod chunk requests and their FULL generation halo,
            // sorted strictly after player views, forced chunks, and pregen bands.
            if let Some(req) = input.ranges.async_requests {
                let req_reach = margin.max(reach + 1);
                for &(rx, rz) in req.chunks {
                    for dz in -req_reach..=req_reach {
                        for dx in -req_reach..=req_reach {
                            let (x, z) = (rx + dx, rz + dz);
                            if !input.ranges.requested(x, z, input.decorates)
                                || input.world.chunks.contains_key(&(x, z))
                                || input.generating.contains(&(x, z))
                                || input.failed.contains(&(x, z))
                                || input.protos.contains_key(&(x, z))
                                || input.finishing.contains(&(x, z))
                            {
                                continue;
                            }
                            wanted.push((input.ranges.priority(x, z), x, z));
                        }
                    }
                }
            }
            let ranges = Ranges {
                center: input.ranges.center,
                radius: input.ranges.radius,
                extra: input.ranges.extra,
                forced: input.ranges.forced,
                cover: input.ranges.cover,
                forced_cover: None,
                band: input.ranges.band,
                async_requests: input.ranges.async_requests,
            };
            self.remember(ranges, input.world);
            if wanted.is_empty() {
                self.plan.clear();
                self.plan_basis = None;
            } else {
                // A bounded batch may leave candidates behind: the next calls
                // take them from the plan as capacity opens.
                self.exhausted = false;
                self.plan_basis = Some(input.decorates);
            }
            wanted.sort_unstable();
            wanted.dedup();
            self.plan.clear();
            self.plan.extend_from_slice(&wanted);
            self.next = 0;
            wanted.truncate(capacity - input.generating.len());
            Some(wanted)
        }
    }
}
/// What a pool worker keeps for one streamer between its jobs: the
/// generator's worker and the companion executor's per-worker state.
struct WorkerLocal<W, AW> {
    worker: W,
    aux: AW,
}
/// A streamer's jobs, run by the process's shared pool (`sunrise_threads::jobs`).
struct Jobs<G: ChunkGenerator, A: AuxiliaryJobs> {
    shared: Arc<Shared<A::Payload, A::Output>>,
    generator: Arc<G>,
    light: Arc<LightTables>,
    aux: A,
    locals: PerWorker<WorkerLocal<G::Worker, A::Worker>>,
}
impl<G: ChunkGenerator, A: AuxiliaryJobs> Source for Jobs<G, A> {
    fn next(&self) -> Option<Rank> {
        let queue = self.shared.queue.lock().unwrap();
        if self
            .shared
            .aborted
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            for q in queue.iter() {
                if matches!(q.job, Job::Save(..)) {
                    return Some(Rank::new(Class::Urgent, q.priority));
                }
            }
            return None;
        }
        queue.peek().map(|q| {
            // Jobs without a column (saves, meshes to free) go first.
            let class = if q.at.is_none() {
                Class::Urgent
            } else {
                Class::Near
            };
            Rank::new(class, q.priority)
        })
    }
    fn run(&self, worker: usize) -> bool {
        let Some(job) = self.shared.queue.lock().unwrap().pop() else {
            return false;
        };
        if self
            .shared
            .aborted
            .load(std::sync::atomic::Ordering::Relaxed)
            && !matches!(job.job, Job::Save(..))
        {
            return true;
        }
        let started = job_stats::enabled().then(|| {
            let kind = match &job.job {
                Job::Generate(..) => 1,
                Job::Decorate(..) => 2,
                Job::Finish(..) => 3,
                Job::Light(..) => 4,
                Job::Auxiliary(payload) => A::classify(payload).0,
                Job::Save(..) | Job::LightView(..) => 6,
            };
            // The other jobs (saves, light views, freed meshes) are many and short: counted, not
            // timed, so the clock reads don't cost more than they do.
            let cpu = if kind == 6 {
                std::time::Duration::ZERO
            } else {
                crate::spans::thread_cpu().unwrap_or_default()
            };
            (kind, cpu)
        });
        let _tag = sunrise_threads::heap_tag(match &job.job {
            Job::Generate(..) => "stream.generate",
            Job::Decorate(..) => "stream.decorate",
            Job::Finish(..) => "stream.finish",
            Job::Light(..) => "stream.light",
            Job::Auxiliary(payload) => A::classify(payload).1,
            Job::Save(..) | Job::LightView(..) => "stream.other",
        });
        let done = self.locals.with(
            worker,
            || {
                let _tag = sunrise_threads::heap_tag("stream.worker_state");
                WorkerLocal {
                    worker: self.generator.worker(),
                    aux: self.aux.worker(),
                }
            },
            |local| self.run_job(job.job, local),
        );
        if let Some((kind, started)) = started {
            // A generation job that found the column saved loaded it.
            let kind = if kind == 1 && matches!(done, Some(Mail::Chunk(Done::Generated(_)))) {
                0
            } else {
                kind
            };
            let nanos = if kind == 6 {
                0
            } else {
                let cpu = crate::spans::thread_cpu().unwrap_or_default();
                cpu.saturating_sub(started).as_nanos() as u64
            };
            job_stats::add(kind, nanos);
        }
        if let Some(done) = done {
            self.shared.done.send(done);
        }
        true
    }
    fn idle(&self, worker: usize) {
        // A server nobody is on, a client standing still: free the
        // generator's buffers (and any companion scratch) until the next job needs them.
        self.locals.with_existing(worker, |local| {
            self.generator.idle(&mut local.worker);
            self.aux.idle(&mut local.aux);
        });
    }
    fn all_idle(&self) {
        // No worker needs its kept meshes or light scratch: freed outside the pool lock.
        self.aux.all_idle();
        crate::light::release_scratch();
        self.generator.all_idle();
    }
}
impl<G: ChunkGenerator, A: AuxiliaryJobs> Jobs<G, A> {
    fn run_job(
        &self,
        job: Job<A::Payload>,
        local: &mut WorkerLocal<G::Worker, A::Worker>,
    ) -> Option<Mail<A::Output>> {
        let (generator, light) = (&self.generator, &self.light);
        let WorkerLocal { worker, aux } = local;
        let done = match job {
            Job::Generate(x, z) => match generator.load(x, z) {
                Some(chunk) => {
                    generator.prepare_loaded(&mut *worker, &chunk);
                    Done::Generated(chunk)
                }
                None => {
                    // A panic or explicit error in the generator loses this column only: it is
                    // reported, not left generating forever. This covers every generator, the
                    // vanilla one included, not only a mod's `build_terrain`.
                    let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        generator.worker_proto(&mut *worker, x, z)
                    }));
                    match made {
                        Ok(Ok((proto, saved))) => {
                            let decorated = saved.is_none() && generator.is_decorated(&proto);
                            Done::Proto(x, z, proto, decorated, saved)
                        }
                        Ok(Err(reason)) => Done::Failed(x, z, reason),
                        Err(_) => Done::Failed(x, z, "panicked".to_owned()),
                    }
                }
            },
            Job::Decorate(x, z, mut region, finals, outer) => {
                // The final chunks' proto chunks, made here: a
                // generated proto chunk holding their blocks.
                for (i, snapshot) in &finals {
                    if let Some(proto) = generator.proto_beside_final(&mut *worker, snapshot) {
                        region[*i] = proto;
                    }
                }
                generator.decorate_with_outer(&mut *worker, &mut region, &outer);
                let mut writes = FinalWrites::default();
                let blocks: [Vec<_>; 9] = std::array::from_fn(|n| {
                    finals.get(n).map_or_else(Vec::new, |(i, snapshot)| {
                        generator.final_writes(snapshot, &region[*i])
                    })
                });
                let entries: Vec<_> = finals
                    .iter()
                    .zip(&blocks)
                    .map(|((i, _), blocks)| (&region[*i], blocks.as_slice()))
                    .collect();
                writes.block_entities = generator.final_block_entities(&entries);
                writes.blocks.extend(blocks.into_iter().flatten());
                for (i, _) in &finals {
                    region[*i] = Box::new(());
                }
                let finals = finals.into_iter().map(|(i, _)| i).collect();
                Done::Decorated(x, z, region, finals, writes)
            }
            Job::Finish(proto) => {
                let mut chunk = generator.finish(proto);
                chunk.generated = true;
                Done::Generated(chunk)
            }
            Job::Save(protos) => {
                let refs: Vec<&Proto> = protos.iter().map(|p| &p.proto).collect();
                generator.save_protos(&refs);
                Done::Saved(protos.into_iter().map(|p| (p.pos, p.proto)).collect())
            }
            Job::LightView(x, z, proto) => {
                let view = generator.light_view(&proto);
                Done::LightView(x, z, proto, view)
            }
            Job::Light(x, z, region, seq, sky) => {
                Done::Lit(x, z, light_chunk(&borrow(&region), light, sky), seq)
            }
            Job::Auxiliary(payload) => {
                return self.aux.run(payload, aux).map(Mail::Auxiliary);
            }
        };
        Some(Mail::Chunk(done))
    }
}

mod bounded;
pub use bounded::nearest_first;
mod dynamic_emission;
mod forced_cover;
mod forced_ring;
mod queue_census;
pub use queue_census::QueueCensus;
mod relight_now;
use dynamic_emission::light_region_ready;
use forced_cover::ForcedCover;

impl ChunkState {
    /// Shares generation resources, but gives distant work its own worker state.
    pub fn lod_source(&self) -> Arc<dyn LodSource> {
        self.saves.clone()
    }
    /// Collect existing resident columns in the `FEATURES` `STRUCTURE_STARTS`
    /// dependency ring (Chebyshev distance `2..=8` around `(center_x, center_z)`).
    fn outer_decoration_columns(
        chunks: &FxHashMap<(i32, i32), Arc<Chunk>>,
        light_views: &FxHashMap<(i32, i32), Arc<Chunk>>,
        center_x: i32,
        center_z: i32,
    ) -> Vec<Arc<Chunk>> {
        // ChunkPyramid.GENERATION_PYRAMID: FEATURES -> STRUCTURE_STARTS radius 8.
        const FEATURES_DEPENDENCY_RADIUS: i32 = 8;
        if chunks.is_empty() && light_views.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for dz in -FEATURES_DEPENDENCY_RADIUS..=FEATURES_DEPENDENCY_RADIUS {
            let Some(cz) = center_z.checked_add(dz) else {
                continue;
            };
            for dx in -FEATURES_DEPENDENCY_RADIUS..=FEATURES_DEPENDENCY_RADIUS {
                if dx.abs().max(dz.abs()) <= 1 {
                    continue;
                }
                let Some(cx) = center_x.checked_add(dx) else {
                    continue;
                };
                if let Some(chunk) = chunks.get(&(cx, cz)).or_else(|| light_views.get(&(cx, cz))) {
                    out.push(Arc::clone(chunk));
                }
            }
        }
        out
    }
    /// The resident sections `LevelExtractor.allChanged` queues again. An
    /// unlit column will queue itself when its light job completes.
    pub fn mark_loaded_sections_dirty(&self, world: &mut World) {
        Self::mark_loaded_sections_dirty_for(world, &self.lit);
    }
    /// Mark all sections of loaded columns present in `lit` dirty.
    pub fn mark_loaded_sections_dirty_for(world: &mut World, lit: &FxHashSet<(i32, i32)>) {
        let mask = SectionMask::below(world.layout().sections());
        for &(x, z) in world.chunks.keys() {
            if !lit.contains(&(x, z)) {
                continue;
            }
            world.dirty_sections.insert_column_mask((x, z), mask);
        }
    }
    /// A column received from a server, with the light it sent
    /// (`ClientPacketListener.handleLevelChunkWithLight`): nothing is
    /// relit, and it and its neighbours remesh (`enableChunkLight`).
    pub fn insert_remote(&mut self, world: &mut World, chunk: Chunk) {
        let key = (chunk.x, chunk.z);
        if let Some(job) = self.lighting.get_mut(&key) {
            job.snapshot = None;
        }
        world.install_light_column(chunk);
        self.lit.insert(key);
        self.generated_total += 1;
        let mask = SectionMask::below(world.layout().sections());
        for dz in -1..=1 {
            for dx in -1..=1 {
                let n = (key.0 + dx, key.1 + dz);
                if n == key || world.chunks.contains_key(&n) {
                    world.dirty_sections.insert_column_mask(n, mask);
                }
            }
        }
    }
    /// A resident column a server sent again whole (it changed it in bulk):
    /// its new blocks taken, the sections that did not change kept as they
    /// were, and (for a Sunrise server's bulk edit) its light and its
    /// neighbours' worked out again here, off the game thread, as light is for
    /// the blocks a server changes one at a time; the sections that changed
    /// remesh (with theirs) once lit. Until then the sections that changed
    /// show the light the packet carried (a Sunrise server sends the light
    /// from before its relight, then the new light; another server's is already
    /// right, which matters where the neighbours this light needs aren't all
    /// here).
    pub fn replace_remote(&mut self, world: &mut World, mut chunk: Chunk) {
        let key = (chunk.x, chunk.z);
        let Some(old) = world.chunks.get(&key).cloned() else {
            return self.insert_remote(world, chunk);
        };
        if !self.lit.contains(&key) {
            keep_resent_extra_models(&old, &mut chunk);
            return self.insert_remote(world, chunk);
        }
        take_resent_column(world, &old, chunk, self.relight_resent);
    }
    fn receive_decorated(
        &mut self,
        x: i32,
        z: i32,
        region: [Proto; 9],
        finals: Vec<usize>,
        writes: FinalWrites,
        world: &mut World,
    ) {
        self.decoration_changed = true;
        self.generation_scan.exhausted = false;
        self.decorating -= 1;
        for i in &finals {
            self.decorating_finals
                .remove(&(x + (i % 3) as i32 - 1, z + (i / 3) as i32 - 1));
        }
        self.decorated.insert((x, z));
        for ([bx, by, bz], state) in writes.blocks {
            let _ = world.set(bx, by, bz, sunrise_world::registry::BlockStateId(state));
        }
        for (i, proto) in region.into_iter().enumerate() {
            // A final chunk keeps its own chunk: only what decoration wrote
            // into it goes there.
            if finals.contains(&i) {
                continue;
            }
            let key = (x + (i % 3) as i32 - 1, z + (i / 3) as i32 - 1);
            self.light_views.remove(&key);
            self.protos.insert(key, Some(proto));
            self.written.insert(key);
            // Features can change section occupancy in initialized neighbours too.
            Self::publish_proto_light_support(world, key, self.proto_sections(key.0, key.1));
        }
        // After the blocks: the write of a block with a block entity made a
        // default one, which the generated one replaces.
        let load = if writes.block_entities.is_empty() {
            None
        } else {
            crate::spans::raw_flight_guard("load_block_entities")
        };
        let mut inserted = 0;
        for (pos, entity) in writes.block_entities {
            if world.chunks.contains_key(&(pos.0 >> 4, pos.2 >> 4)) {
                world.block_entities.insert(pos, entity);
                inserted += usize::from(load.is_some());
            }
        }
        if load.is_some() {
            crate::spans::note_block_entity_loads(inserted);
        }
    }
    /// Saved proto chunks come back; those now out of range are dropped by
    /// the next `update`.
    fn receive_saved(&mut self, protos: Vec<((i32, i32), Proto)>) {
        self.decoration_changed = true;
        self.generation_scan.exhausted = false;
        self.saving -= 1;
        for (key, proto) in protos {
            self.light_views.remove(&key);
            self.protos.insert(key, Some(proto));
        }
    }
    /// Check out proto chunks without submitting them yet.
    fn checkout_protos(&mut self, keys: impl IntoIterator<Item = (i32, i32)>) -> Vec<ProtoSave> {
        checkout_protos(&mut self.protos, &mut self.light_views, keys)
    }
    fn receive_generated(&mut self, mut chunk: Chunk, world: &mut World) -> Option<Chunk> {
        self.decoration_changed = true;
        self.generation_scan.exhausted = false;
        let key = (chunk.x, chunk.z);
        self.generating.remove(&key);
        self.finishing.remove(&key);
        self.decorated.insert(key);
        // A column that finished after the player moved away is dropped
        // whole: its block entities and saved entities too, or they would
        // stay in the world (and the entities spawn again, duplicated, when
        // the column loads later). A generated one is saved first: its
        // neighbours were decorated around it, so generating it again would
        // not give it back.
        if !self.ranges().chunk_kept(key.0, key.1, self.decorates) {
            // Its proto chunk is gone too: forget the decoration with it, as
            // the sweep used to do for the whole set on every tick.
            self.decorated.remove(&key);
            world.set_proto_light_support(key, SectionMask::EMPTY);
            if chunk.fresh {
                self.dropped_new.insert(key);
                return Some(chunk);
            }
            return None;
        }
        if !chunk.generated {
            self.saves.accept_loaded_chunk(&mut chunk);
        }
        // Generated columns are saved like edited ones, as vanilla saves
        // every chunk it generates.
        if chunk.fresh {
            world.modified.insert(key);
        }
        world.install_chunk_block_entities(&mut chunk);
        world
            .block_names
            .extend(std::mem::take(&mut chunk.block_names));
        use crate::world::PendingEntity;
        let entities = std::mem::take(&mut chunk.entities);
        let disk_entities = std::mem::take(&mut chunk.disk_entities);
        world.entities_pending.insert(key);
        if !entities.is_empty() || !disk_entities.is_empty() {
            let pending = entities
                .into_iter()
                .map(PendingEntity::from)
                .chain(
                    disk_entities
                        .into_iter()
                        .map(PendingEntity::from_disk_section),
                )
                .collect();
            world.loaded_entities.push((key, pending));
        }
        self.generated_total += 1;
        // A chunk saved with its light needs no light job; its unlit
        // neighbours still light themselves from it.
        let loaded = chunk.light_loaded;
        let new = self.dropped_new.remove(&key) || chunk.generated;
        world.loaded_chunks.push((key.0, key.1, new));
        chunk.dynamic_pending = self.light_tables.holds_dynamic(&chunk);
        let status = self
            .light_tables
            .publish_section_status(&mut chunk, world.shape);
        if loaded {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let neighbour = (key.0 + dx, key.1 + dz);
                    if neighbour != key && !world.chunks.contains_key(&neighbour) {
                        Self::publish_proto_light_support(
                            world,
                            neighbour,
                            self.proto_sections(neighbour.0, neighbour.1),
                        );
                    }
                }
            }
        }
        world.install_light_column_initialized(chunk, status);
        if self.dirty_visibility_on_load {
            dirty_visibility_columns(world, key.0, key.1);
        }
        if loaded {
            self.lit.insert(key);
            for sy in 0..world.layout().sections() as i32 {
                world.dirty_sections.insert((key.0, sy, key.1));
            }
        }
        for dz in -1..=1 {
            for dx in -1..=1 {
                let n = (key.0 + dx, key.1 + dz);
                if !loaded || world.chunks.get(&n).is_some_and(|c| !c.light_loaded) {
                    world.dirty_light.insert(n);
                }
            }
        }
        None
    }
    /// Submit a generated column's deferred full-chunk save to persistence.
    pub fn save_finished(&self, chunk: Chunk) {
        self.saves.save_finished(chunk);
    }
    /// The order jobs around the player run in, lowest first.
    fn distance(&self, x: i32, z: i32) -> i64 {
        self.ranges().priority(x, z)
    }
    /// Public distance priority for companion schedulers sharing this state.
    pub fn column_distance(&self, x: i32, z: i32) -> i64 {
        self.distance(x, z)
    }
    /// Whether `(x, z)` is within the player's view distance or forced.
    pub fn in_view(&self, x: i32, z: i32) -> bool {
        self.ranges().in_view(x, z)
    }
    /// A proto chunk back from its light view: the view waits in
    /// `light_views` for the light jobs that asked for it.
    fn receive_light_view(&mut self, x: i32, z: i32, proto: Proto, view: Option<Chunk>) {
        self.decoration_changed = true;
        self.viewing.remove(&(x, z));
        self.protos.insert((x, z), Some(proto));
        if let Some(view) = view {
            self.light_views.insert((x, z), Arc::new(view));
        }
    }
    /// A column whose generation failed or panicked: no longer generating, and never planned again.
    fn receive_failed(&mut self, x: i32, z: i32, reason: String) {
        self.generation_scan.exhausted = false;
        self.generating.remove(&(x, z));
        if self.failed.insert((x, z)) {
            log::error!(
                "Sunrise: generating chunk [{x}, {z}] failed ({reason}); the column stays out of the level until it opens again"
            );
        }
    }
    /// Whether `light_region` would find all nine columns: the same walk
    /// without building the snapshot, so the scheduler can rank the ready
    /// columns before any region is cloned. A proto neighbour whose light
    /// view turns out empty is skipped when its region is built, and asked
    /// for again, as `light_region` returning `None` always was.
    fn light_region_ready(&self, world: &World, x: i32, z: i32) -> bool {
        light_region_ready(
            world,
            |x, z| self.in_range(x, z, 1),
            &self.decorated,
            &self.protos,
            x,
            z,
        )
    }
    /// Spawn height of this level's generator, not the initial world's cached origin.
    pub fn spawn_height(&self) -> i32 {
        self.spawn_height
    }
    pub fn is_lit(&self, x: i32, z: i32) -> bool {
        self.lit.contains(&(x, z))
    }
    pub fn is_lighting(&self, x: i32, z: i32) -> bool {
        self.lighting
            .get(&(x, z))
            .is_some_and(|job| job.snapshot.is_some())
    }
    /// Retire the actual worker job. If the column was dirtied again, leave its coalesced
    /// entry in `dirty_light` for the scheduler and do not publish this snapshot's light.
    fn receive_lit(
        &mut self,
        world: &mut World,
        x: i32,
        z: i32,
        sections: Vec<crate::chunk::Light>,
        seq: u64,
    ) {
        let std::collections::hash_map::Entry::Occupied(entry) = self.lighting.entry((x, z)) else {
            return;
        };
        if entry.get().seq != seq {
            return;
        }
        let job = entry.remove();
        let covered = matches!(
            world.chunk_resends.get(&(x, z)),
            Some(&ChunkResend::WaitingLight { from_seq }) if seq >= from_seq
        );
        if let Some(snapshot) = job.snapshot
            && (covered || !world.dirty_light.contains(&(x, z)))
        {
            self.apply_lit(world, x, z, sections, snapshot);
            if covered {
                world.chunk_resends.insert((x, z), ChunkResend::Ready);
            }
        }
        match self.lighting.values().filter_map(|job| job.snapshot).min() {
            Some(oldest) => self.light_log.retain(|(seq, _)| *seq > oldest),
            None => self.light_log.clear(),
        }
    }
    /// Record an explicit chunk resend for `key` behind a completion marker covering only the
    /// light work already queued or running for `key`.
    pub fn queue_chunk_resend(&self, world: &mut World, key: (i32, i32)) {
        let queued = world.dirty_light.contains(&key);
        let running = self
            .lighting
            .get(&key)
            .filter(|job| job.snapshot.is_some())
            .map(|job| job.seq);
        let state = if queued || running.is_some() {
            ChunkResend::WaitingLight {
                from_seq: if queued {
                    self.sequence + 1
                } else {
                    running.unwrap_or_default()
                },
            }
        } else if world.light_edits.iter().any(|&((ex, _, ez), _, _)| {
            (ex >> 4).abs_diff(key.0) <= 1 && (ez >> 4).abs_diff(key.1) <= 1
        }) {
            ChunkResend::WaitingEdits {
                through: world.light_edits.len(),
            }
        } else {
            ChunkResend::Ready
        };
        world.chunk_resends.insert(key, state);
    }
    /// Whether a whole-column job for `key` was submitted and its result has not been taken.
    pub fn has_column_light_job(&self, key: (i32, i32)) -> bool {
        self.lighting.contains_key(&key)
    }
    /// The columns whose light is not settled: queued or running whole-column jobs and the
    /// columns an unapplied edit can still reach (light spreads at most one column).
    pub fn columns_with_outstanding_light(&self, world: &World) -> Vec<(i32, i32)> {
        let mut columns: FxHashSet<(i32, i32)> = self.lighting.keys().copied().collect();
        columns.extend(world.dirty_light.iter().copied());
        for &((x, _, z), _, _) in &world.light_edits {
            let (cx, cz) = (x >> 4, z >> 4);
            for dz in -1..=1 {
                for dx in -1..=1 {
                    columns.insert((cx + dx, cz + dz));
                }
            }
        }
        columns.into_iter().collect()
    }
    /// Whether this level has light work still running or ready to schedule.
    pub fn has_pending_light(&self, world: &World) -> bool {
        !self.lighting.is_empty()
            || !self.viewing.is_empty()
            || !world.light_edits.is_empty()
            || world
                .chunk_resends
                .values()
                .any(|state| matches!(state, ChunkResend::WaitingEdits { .. }))
            || world
                .dirty_light
                .iter()
                .any(|&(x, z)| self.light_region_ready(world, x, z))
    }
    /// The non-air sections of a column that is a decorated proto chunk
    /// here (vanilla's INITIALIZE_LIGHT and on), if it is one: a level
    /// chunk's light layers count them (#4294).
    pub fn proto_sections(&self, x: i32, z: i32) -> Option<SectionMask> {
        if !self.decorated.contains(&(x, z)) {
            return None;
        }
        (self.non_air_sections)(self.protos.get(&(x, z))?.as_ref()?)
    }
    fn publish_proto_light_support(
        world: &mut World,
        key: (i32, i32),
        sections: Option<SectionMask>,
    ) {
        // None means the proto is not initialized or is checked out, not that its
        // published support was withdrawn. Eviction/promotion retire it separately.
        let Some(sections) = sections else { return };
        let first = world.layout().min_y() >> 4;
        let mut support = SectionMask::EMPTY;
        for s in sections
            .iter()
            .filter(|s| world.shape.has_section(first + *s as i32))
        {
            for padded in s..=s + 2 {
                support.insert(padded);
            }
        }
        world.set_proto_light_support(key, support);
    }
    /// `ChunkHolder.isReadyForSaving`: the centre has no generation or save
    /// work holding it. With no holder, there is also no such work to wait for.
    pub fn chunk_ready_for_saving(&self, key: (i32, i32)) -> bool {
        !self.generating.contains(&key)
            && !self.decorating_finals.contains(&key)
            && !self.finishing.contains(&key)
            && !self.lighting.contains_key(&key)
            && !self.protos.get(&key).is_some_and(Option::is_none)
    }
    /// `ChunkMap.getLatestStatus`: the furthest step this column has reached,
    /// `None` before it is asked for.
    pub fn latest_chunk_status(
        &self,
        world: &World,
        x: i32,
        z: i32,
    ) -> Option<sunrise_world::chunk_status::ChunkStatus> {
        use sunrise_world::chunk_status::ChunkStatus;
        let key = (x, z);
        Some(if self.lit.contains(&key) {
            ChunkStatus::Full
        } else if world.chunks.contains_key(&key) {
            ChunkStatus::InitializeLight
        } else if self.decorated.contains(&key) || self.finishing.contains(&key) {
            ChunkStatus::Features
        } else if self.protos.contains_key(&key) {
            ChunkStatus::Terrain
        } else if self.generating.contains(&key) {
            ChunkStatus::Empty
        } else {
            return None;
        })
    }
    /// `ChunkMap.getLatestStatus` for the loading screen's chunk map: the
    /// furthest step this column has reached, `None` before it is asked for.
    pub fn latest_status(
        &self,
        world: &World,
        x: i32,
        z: i32,
    ) -> Option<crate::level_loading::Status> {
        use crate::level_loading::Status;
        use sunrise_world::chunk_status::ChunkStatus;
        self.latest_chunk_status(world, x, z).map(|s| match s {
            ChunkStatus::Empty => Status::Empty,
            ChunkStatus::StructureStarts => Status::StructureStarts,
            ChunkStatus::StructureReferences => Status::StructureReferences,
            ChunkStatus::Biomes => Status::Biomes,
            ChunkStatus::Terrain => Status::Terrain,
            ChunkStatus::Features => Status::Features,
            ChunkStatus::InitializeLight => Status::InitializeLight,
            ChunkStatus::Light => Status::Light,
            ChunkStatus::Spawn => Status::Spawn,
            ChunkStatus::Full => Status::Full,
        })
    }
    /// The sequence the next mesh job will be dispatched after: a clock for
    /// `compile_start`, whose jobs all snapshot the world as they are
    /// dispatched (`Util.getNanos()` at the start of a compile task).
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    /// Advance and return the monotonic dispatch sequence shared with companion jobs.
    pub fn next_sequence(&mut self) -> u64 {
        self.sequence += 1;
        self.sequence
    }
    /// Number of pool workers this streamer sizes its in-flight bounds for.
    pub fn workers(&self) -> usize {
        self.workers
    }
    pub fn chunks_in_flight(&self) -> bool {
        !(self.generating.is_empty()
            && self.decorating == 0
            && self.viewing.is_empty()
            && self.finishing.is_empty()
            && self.lighting.is_empty())
    }
    /// Whether near terrain generation or lighting is still in flight.
    pub fn terrain_loading(&self) -> bool {
        self.chunks_in_flight()
    }
    /// The proto chunks held (not checked out): how many, their heap bytes by
    /// part and their sections by bits per block; and the light views of
    /// them with their bytes. For `sunrise-ctl memory`.
    pub fn proto_census(&self) -> ProtoCensus {
        let mut census = ProtoCensus::default();
        for proto in self.protos.values().flatten() {
            let (bytes, bits) = (self.proto_bytes)(proto);
            census.protos += 1;
            for (sum, b) in census.bytes.iter_mut().zip(bytes) {
                *sum += b;
            }
            for (sum, b) in census.sections_by_bits.iter_mut().zip(bits) {
                *sum += b;
            }
        }
        census.light_views = self.light_views.len();
        census.light_view_bytes = self.light_views.values().map(|v| v.heap_bytes()).sum();
        census.caches = (self.caches)();
        census
    }
    /// Remove planned columns and check out proto saves before further scheduling.
    pub fn prepare_commit(
        &mut self,
        world: &mut World,
        plan: &mut ChunkUpdate,
    ) -> (ChunkCommit, Vec<ProtoSave>) {
        if plan.paused {
            return (ChunkCommit::paused(), Vec::new());
        }
        let mut hz_laps = crate::spans::laps();
        let mut removed = Vec::new();
        let mut proto_saves = Vec::new();
        let mut sweep_far = HashSet::new();
        if plan.sweep {
            self.decoration_changed = true;
            let far = &mut plan.far;
            removed = commit_unloads(world, far);
            for key in far.iter() {
                self.decorated.remove(key);
                world.entities_pending.remove(key);
                self.lit.remove(key);
                if let Some(job) = self.lighting.get_mut(key) {
                    job.snapshot = None;
                }
            }
            if !far.is_empty() {
                world
                    .dirty_sections
                    .retain_columns(|x, z| !far.contains(&(x, z)));
                sweep_far = far.clone();
            }
            // Drop far proto chunks that are not checked out.
            let (center, r4) = (self.center, i64::from(self.radius + 4));
            let ranges = self.ranges();
            let far_protos: Vec<(i32, i32)> = self
                .protos
                .iter()
                .filter(|&(&(x, z), p)| {
                    p.is_some()
                        && (self.radius < 0
                            || i64::from(x - center.0).pow(2) + i64::from(z - center.1).pow(2)
                                > r4 * r4)
                        && !ranges.in_extra(x, z, 4)
                        && !ranges.in_band(x, z, 4)
                        && !ranges.in_async_requests(x, z, 4)
                        && !ranges.forced_within(x, z, FORCED_TERRAIN_RING)
                })
                .map(|(k, _)| *k)
                .collect();
            if !far_protos.is_empty() {
                self.generation_scan.invalidate();
            }
            // Those decoration wrote to are saved first (see
            // `ChunkGenerator::save_protos`), and dropped once back.
            let mut written = Vec::new();
            for key in far_protos {
                if self.written.remove(&key) {
                    written.push(key);
                } else {
                    self.protos.remove(&key);
                    self.light_views.remove(&key);
                    // Nothing will decorate it again until it is generated
                    // afresh, so it is no longer decorated.
                    if !world.chunks.contains_key(&key) && !self.finishing.contains(&key) {
                        self.decorated.remove(&key);
                    }
                    world.set_proto_light_support(key, SectionMask::EMPTY);
                }
            }
            proto_saves = self.checkout_protos(written);
        }
        hz_laps.lap("st_sweep");
        (
            ChunkCommit {
                removed,
                sweep_far,
                paused: false,
            },
            proto_saves,
        )
    }
    /// End the active update's cover validity window after any companion pass finishes.
    pub fn finish_update_cover(&mut self) {
        self.cover_current = false;
    }
    /// Toggle immediate visibility-halo dirtying when newly generated columns are installed.
    pub fn set_dirty_visibility_on_load(&mut self, on: bool) {
        self.dirty_visibility_on_load = on;
    }
    /// Whether the ranges changed since the last finalize/decoration pass,
    /// recording the current ones when they did. Unlike
    /// `GenerationScan::same_ranges`, this is not affected by a scan that
    /// returned at capacity without remembering where its candidates came
    /// from.
    fn decoration_ranges_changed(&mut self) -> bool {
        let (center, radius, band, extra, forced, async_requests) = (
            self.center,
            self.radius,
            self.band,
            self.extra.as_slice(),
            &self.forced,
            &self.async_requests,
        );
        let changed = match &self.decoration_basis {
            Some(basis) => {
                basis.center != center
                    || basis.radius != radius
                    || basis.band != band
                    || basis.extra != extra
                    || basis.forced != *forced
                    || basis.async_requests != *async_requests
            }
            None => true,
        };
        if changed {
            let basis = self
                .decoration_basis
                .get_or_insert_with(DecorationBasis::default);
            basis.center = center;
            basis.radius = radius;
            basis.band = band;
            basis.extra.clear();
            basis.extra.extend_from_slice(extra);
            basis.forced.clone_from(forced);
            basis.async_requests.clone_from(async_requests);
        }
        changed
    }
    /// Drop decorated keys whose column left `World::chunks` without going
    /// through streaming (`World::unload_chunk`, a server packet). A stale key
    /// would let a proto finish against a missing neighbour before it is
    /// regenerated and decorated. Falls back to a full prune when the
    /// membership ring cannot replay the changes.
    fn reconcile_decorated(&mut self, world: &World) {
        let Some(changes) = world.chunks.deltas_since(self.decoration_revision) else {
            let (protos, finishing, chunks) = (&self.protos, &self.finishing, &world.chunks);
            self.decorated.retain(|key| {
                protos.contains_key(key) || finishing.contains(key) || chunks.contains_key(key)
            });
            return;
        };
        for (_, change) in changes {
            if let ChunkMembershipDelta::Removed(key) = change
                && !self.protos.contains_key(&key)
                && !self.finishing.contains(&key)
                && !world.chunks.contains_key(&key)
            {
                self.decorated.remove(&key);
            }
        }
    }
    /// Drain the ordered edit queue, completing resend markers at their recorded prefixes
    /// (`ThreadedLevelLightEngine.waitForPendingTasks`).
    fn relight_edits(&mut self, world: &mut World) {
        if world.light_edits.is_empty() {
            return;
        }
        let edits = std::mem::take(&mut world.light_edits);
        let mut markers: Vec<_> = world
            .chunk_resends
            .iter()
            .filter_map(|(&key, state)| match *state {
                ChunkResend::WaitingEdits { through } => Some((through, key)),
                _ => None,
            })
            .collect();
        markers.sort_unstable();
        let base_sequence = self.sequence;
        let mut required = FxHashSet::default();
        let mut start = 0;
        for (through, key) in markers {
            if start != through {
                self.relight_edit_batch(world, &edits[start..through], |&key| {
                    required.insert(key);
                });
                start = through;
            }
            world.chunk_resends.insert(
                key,
                if required.contains(&key) {
                    ChunkResend::WaitingLight {
                        from_seq: base_sequence + 1,
                    }
                } else {
                    ChunkResend::Ready
                },
            );
        }
        if start != edits.len() {
            self.relight_edit_batch(world, &edits[start..], |_| {});
        }
    }
    /// Apply a batch in place or, for a large group of edits, hand the columns that can
    /// be lit whole to the workers.
    fn relight_edit_batch(
        &mut self,
        world: &mut World,
        edits: &[LightEdit],
        mut requires_column: impl FnMut(&(i32, i32)),
    ) {
        let mut positions: Vec<_> = edits
            .iter()
            .filter(|(_, old, new)| self.light_tables.differs(*old, *new))
            .map(|(p, _, _)| *p)
            .collect();
        if positions.len() > INLINE_RELIGHT_MAX {
            let (columns, inline) = whole_relight_columns(positions, |x, z| {
                self.lit.contains(&(x, z)) && self.light_region_ready(world, x, z)
            });
            world
                .dirty_light
                .extend(columns.into_iter().inspect(&mut requires_column));
            positions = inline;
        }
        if positions.is_empty() {
            return;
        }
        let mut changed = LightChanges::default();
        crate::light::relight(
            &mut world.chunks,
            &self.light_tables,
            self.sky_light,
            &positions,
            false,
            &mut changed,
        );
        Self::mark_light_changes(world, &changed);
        let columns = Self::light_jobs_beside_unlit(world, &changed);
        world
            .dirty_light
            .extend(columns.into_iter().inspect(requires_column));
        if !self.lighting.is_empty() {
            for p in positions {
                self.light_edit_sequence += 1;
                self.light_log.push((self.light_edit_sequence, p));
            }
        }
    }
    /// A light job computed from blocks older than some edits: redo those
    /// edits on top of its result.
    fn replay_light_edits(&mut self, world: &mut World, x: i32, z: i32, snapshot: u64) {
        let positions: Vec<(i32, i32, i32)> = self
            .light_log
            .iter()
            .filter(|(seq, p)| {
                *seq > snapshot && ((p.0 >> 4) - x).abs() <= 1 && ((p.2 >> 4) - z).abs() <= 1
            })
            .map(|(_, p)| *p)
            .collect();
        if !positions.is_empty() {
            let mut changed = LightChanges::default();
            crate::light::relight(
                &mut world.chunks,
                &self.light_tables,
                self.sky_light,
                &positions,
                true,
                &mut changed,
            );
            Self::mark_light_changes(world, &changed);
            let columns = Self::light_jobs_beside_unlit(world, &changed);
            world.dirty_light.extend(columns);
        }
    }
    /// A column whose light an edit changed beside a column not lit yet gets
    /// a full light job ([`crate::light::beside_unlit`]).
    fn light_jobs_beside_unlit(world: &World, changed: &LightChanges) -> Vec<(i32, i32)> {
        crate::light::beside_unlit(changed, |x, z| {
            world.chunks.get(&(x, z)).is_some_and(|c| !c.lit)
        })
    }
    /// Sections whose meshes sample a changed light value (faces and corners
    /// read light from neighbouring blocks). Every column whose stored light
    /// changed is saved again (`ChunkHolder.sectionLightChanged` marks the
    /// chunk unsaved): an edit's light reaches up to 15 blocks into columns
    /// the edit itself did not mark, which may already have been saved.
    fn mark_light_changes(world: &mut World, changed: &LightChanges) {
        for (x, sy, z) in changed.sections() {
            world.modified.insert((x, z));
            if let Some(log) = &mut world.light_log {
                log.entry((x, z)).or_default().insert(sy as usize + 1);
            }
        }
        changed.dirty_sections(|section| {
            world.dirty_sections.insert(section);
        });
    }
    fn async_requests_range(&self) -> Option<AsyncRequests<'_>> {
        (!self.async_requests.is_empty()).then(|| AsyncRequests {
            chunks: &self.async_requests,
            cover: self
                .async_request_cover
                .as_ref()
                .filter(|_| self.cover_current),
        })
    }
    fn ranges(&self) -> Ranges<'_> {
        Ranges {
            center: self.center,
            radius: self.radius,
            extra: &self.extra,
            forced: &self.forced,
            cover: ExtraCover::current(&self.cover, &self.extra, self.cover_current),
            forced_cover: self.forced_cover.as_ref().filter(|_| self.cover_current),
            band: self.band.as_ref(),
            async_requests: self.async_requests_range(),
        }
    }
    fn near_finished(&self, x: i32, z: i32, reach: i32) -> bool {
        self.ranges().near_finished(x, z, reach)
    }
    fn in_range(&self, x: i32, z: i32, margin: i32) -> bool {
        self.ranges().in_range(x, z, margin)
    }
    /// Rank the 3x3 a chunk loaded to FULL decorates, in one generation
    /// task's order (`ChunkGenerationTask`: x-outer, z-inner), after every
    /// chunk ranked before.
    pub fn rank_full_task(&mut self, x: i32, z: i32) {
        for dx in -1..=1 {
            for dz in -1..=1 {
                self.decoration_order.rank_or_append(x + dx, z + dz);
            }
        }
    }
    pub fn band(&self) -> Option<Band> {
        self.band
    }
    /// Update one saved forced ticket without waiting for generation. An add
    /// ranks decoration; gameplay uses `set_forced_loaded` for the load barrier.
    pub fn set_forced(&mut self, x: i32, z: i32, forced: bool) -> bool {
        let mut changed = false;
        self.set_forced_area((x, z), (x, z), forced, |_, _, c| changed = c);
        changed
    }
    /// The end of a [`FullColumn::Pending`] load: the wait's square goes, and a column that came
    /// FULL joins the recent lookups (`ServerChunkCache.storeInCache`). Whether it is FULL.
    pub fn end_full_column(&mut self, world: &World, chunk: (i32, i32)) -> bool {
        let at = self
            .extra
            .iter()
            .rposition(|area| *area == (chunk, FULL_WAIT_RADIUS));
        if let Some(at) = at {
            self.extra.remove(at);
        }
        let ready = full_column_ready(world, chunk);
        if ready {
            self.remember_full(chunk);
        }
        ready
    }
    /// Whether column `chunk` cannot come FULL because a generation failed (`Done::Failed`, a
    /// panic contained in a mod's chunk generator): its own, or that of a column its wait
    /// generates around it (the margin `Ranges::margins` asks for, which decoration and light
    /// need). Generation does not try those columns again while the level is open.
    pub fn full_column_failed(&self, chunk: (i32, i32)) -> bool {
        let (margin, _) = Ranges::margins(self.decorates);
        let reach = FULL_WAIT_RADIUS + margin;
        self.failed
            .iter()
            .any(|&(x, z)| (x - chunk.0).abs() <= reach && (z - chunk.1).abs() <= reach)
    }
    /// `ServerChunkCache.storeInCache`: the newest of the four recent FULL lookups.
    fn remember_full(&self, chunk: (i32, i32)) {
        let mut recent = self.recent_full.get();
        recent.rotate_right(1);
        recent[0] = Some(chunk);
        self.recent_full.set(recent);
    }
    /// `ServerChunkCache.getChunkNow` on a resident column: a FULL column not among the recent
    /// lookups joins them, so a `Level.getChunk` that follows answers from them with no ticket.
    /// A remote streamer keeps none.
    pub fn note_full_read(&self, world: &World, chunk: (i32, i32)) {
        if !self.remote
            && !self.recent_full.get().contains(&Some(chunk))
            && full_column_ready(world, chunk)
        {
            self.remember_full(chunk);
        }
    }
    /// `ServerChunkCache.clearCache`: forget the recent FULL lookups, at the end of every
    /// `ServerChunkCache.tick`, the level's chunk tick, whether or not the server runs normally.
    pub fn clear_full_lookups(&self) {
        self.recent_full.set([None; 4]);
    }
    /// `TicketStorage.purgeStaleTickets` for the `UNKNOWN` tickets: each one's ticks left go
    /// down by one, and it is removed once they fall below zero. With `TicketType.UNKNOWN`'s
    /// timeout of 1, a ticket survives the first purge after it was placed and goes at the
    /// second. Called once per server tick that runs normally, at the level's chunk tick.
    pub fn purge_unknown_tickets(&mut self) {
        self.unknown.retain_mut(|(_, left)| {
            *left -= 1;
            *left >= 0
        });
    }
    /// Whether a live `UNKNOWN` ticket holds a column of this level.
    pub fn has_unknown_tickets(&self) -> bool {
        !self.unknown.is_empty()
    }
    /// Keep `chunk` loading/generating to `FULL` (level 33) at priority strictly below player
    /// views, forced chunks, and pregeneration bands until removed.
    pub fn add_async_request(&mut self, chunk: (i32, i32)) {
        if self.async_requests.insert(chunk) {
            self.decoration_order_dirty = true;
        }
    }
    /// Release a non-blocking mod chunk request's hold on `chunk`.
    pub fn remove_async_request(&mut self, chunk: (i32, i32)) {
        if self.async_requests.remove(&chunk) {
            self.decoration_order_dirty = true;
        }
    }
    /// Release every active non-blocking mod chunk request on this level (e.g. level close).
    pub fn clear_async_requests(&mut self) {
        if !self.async_requests.is_empty() {
            self.async_requests.clear();
            self.async_request_cover = None;
            self.generation_scan.invalidate();
            self.decoration_order_dirty = true;
        }
    }
    /// Whether any non-blocking mod chunk request is currently active on this level.
    pub fn has_async_requests(&self) -> bool {
        !self.async_requests.is_empty()
    }
    /// Every column currently held by an active non-blocking mod chunk request.
    pub fn async_requests(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.async_requests.iter().copied()
    }
    /// Add the `UNKNOWN` tickets to `extra` for one owner update, each as the square of its
    /// column alone, so the update neither unloads their columns nor finishes more around
    /// them. The caller truncates `extra` afterwards, as for its other per-update areas.
    pub fn hold_unknown_tickets(&mut self) {
        self.extra
            .extend(self.unknown.iter().map(|&(chunk, _)| (chunk, -1)));
    }
    /// Release every `/forceload`ed chunk (`/forceload remove all`, a game-test
    /// batch's end). Chunks a mod holds a ticket for stay forced.
    pub fn clear_forced(&mut self) {
        self.ordered_commanded.clear();
        let cleared: Vec<_> = self.commanded.drain().collect();
        for chunk in cleared {
            self.settle(chunk);
        }
    }
    /// Activate saved `/forceload` tickets without discarding live tickets.
    /// Saved mod tickets wait for validation in `take_loaded_tickets`.
    pub fn load_forced(&mut self, commanded: HashSet<(i32, i32)>, tickets: Vec<ModTicket>) {
        self.mod_tickets.stage_saved(tickets);
        for chunk in commanded {
            if self.set_commanded(chunk, true) {
                self.settle(chunk);
            }
        }
    }
    /// The mods' tickets read from the save, which hold nothing until the
    /// mods have validated them and the survivors are put back with
    /// `set_mod_forced`.
    pub fn take_loaded_tickets(&mut self) -> Vec<ModTicket> {
        self.mod_tickets.take_loaded()
    }
    /// The `/forceload`ed chunks.
    pub fn commanded(&self) -> &HashSet<(i32, i32)> {
        &self.commanded
    }
    /// The `/forceload`ed chunks in the command's signed packed order.
    pub fn commanded_in_order(&self) -> impl ExactSizeIterator<Item = (i32, i32)> + '_ {
        self.ordered_commanded.iter().map(|&pos| {
            let pos = ChunkPos::unpack(pos);
            (pos.x, pos.z)
        })
    }
    /// The chunk tickets of mods.
    pub fn mod_tickets(&self) -> &ModTickets {
        &self.mod_tickets
    }
    /// Hold or release a mod's ticket: true when the ticket set changed. A
    /// chunk stays forced while `/forceload` or any ticket holds it; one
    /// newly forced is ranked for decoration as `/forceload add` does.
    pub fn set_mod_forced(&mut self, ticket: ModTicket, forced: bool) -> bool {
        let chunk = ticket.chunk;
        let changed = if forced {
            self.mod_tickets.insert(ticket)
        } else {
            self.mod_tickets.remove(&ticket)
        };
        if self.settle(chunk) {
            self.rank_forced(chunk.0, chunk.1, chunk.0, chunk.1);
        }
        changed
    }
    /// Change commanded membership and its ordered projection together.
    fn set_commanded(&mut self, chunk: (i32, i32), forced: bool) -> bool {
        let changed = if forced {
            self.commanded.insert(chunk)
        } else {
            self.commanded.remove(&chunk)
        };
        if changed {
            let packed = ChunkPos::new(chunk.0, chunk.1).pack();
            if forced {
                self.ordered_commanded.insert(packed);
            } else {
                self.ordered_commanded.remove(&packed);
            }
        }
        changed
    }
    /// Bring `forced` in line with who holds `chunk`: true when it just
    /// became forced. The one writer of `forced`.
    fn settle(&mut self, chunk: (i32, i32)) -> bool {
        if self.commanded.contains(&chunk) || self.mod_tickets.holds(chunk) {
            self.forced.insert(chunk)
        } else {
            self.forced.remove(&chunk);
            false
        }
    }
    /// `ServerLevel.setChunkForced` over chunks `(x0, z0)..=(x1, z1)`, x then
    /// z: `each` hears whether each chunk changed in `/forceload`'s set. An
    /// add then ranks the area once for decoration (`forced_order`).
    pub fn set_forced_area(
        &mut self,
        (x0, z0): (i32, i32),
        (x1, z1): (i32, i32),
        forced: bool,
        mut each: impl FnMut(i32, i32, bool),
    ) {
        for x in x0..=x1 {
            for z in z0..=z1 {
                let changed = self.set_commanded((x, z), forced);
                self.settle((x, z));
                each(x, z, changed);
            }
        }
        if forced {
            self.rank_forced(x0, z0, x1, z1);
        }
    }
    /// Rank the chunks `/forceload add` over chunks `(x0, z0)..=(x1, z1)`
    /// decorates in vanilla's order (`forced_order`), after every chunk
    /// ranked before.
    fn rank_forced(&mut self, x0: i32, z0: i32, x1: i32, z1: i32) {
        for (x, z) in sunrise_world::worldgen::decoration_order::forced_order(x0, z0, x1, z1) {
            self.decoration_order.rank_or_append(x, z);
        }
    }
    /// Generate, light and keep the chunks of `band` (none: stop), and let
    /// go of what earlier bands held as it falls out of range. The chunks a
    /// band reaches are ranked for decoration row by row, as they first come
    /// within reach, so that a task decorates in the same order however its
    /// work is timed and shared among the workers.
    pub fn set_band(&mut self, band: Option<Band>) {
        if self.band == band {
            return;
        }
        self.band = band;
        self.generation_scan.exhausted = false;
        let Some(band) = band else {
            return;
        };
        // Decoration reaches two chunks past the band and generation three.
        const REACH: i32 = 4;
        let (x0, x1) = (band.x0 - REACH, band.x1 + REACH);
        let first = match self.band_ranked {
            Some((rx0, rx1, next)) if (rx0, rx1) == (x0, x1) => next,
            _ => band.z0 - REACH,
        };
        let end = band.z1 + REACH;
        for z in first..=end {
            for x in x0..=x1 {
                self.decoration_order.rank_or_append(x, z);
            }
        }
        self.band_ranked = Some((x0, x1, end.max(first - 1) + 1));
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn mark_unlit_for_test(&mut self, key: (i32, i32)) {
        self.lit.remove(&key);
    }
}

impl ChunkStreamer<NoAuxiliary> {
    pub fn new<G: ChunkGenerator>(
        generator: Arc<G>,
        context: Arc<ChunkWorkerContext>,
        threads: usize,
    ) -> Self {
        Self::with_auxiliary(generator, context, NoAuxiliary, threads)
    }
    /// Remove resident columns before clearing their streaming and level state.
    pub fn take_remote_columns(world: &mut World, gone: &FxHashSet<(i32, i32)>) -> Vec<Arc<Chunk>> {
        let mut chunks = Vec::with_capacity(gone.len());
        for key in gone {
            chunks.extend(world.unload_chunk(*key));
        }
        chunks
    }
    /// Advance streaming: collect finished work, then schedule more.
    /// Returns columns that were unloaded.
    pub fn update(&mut self, world: &mut World) -> Vec<Arc<Chunk>> {
        let plan = self.plan_update(world);
        self.finish_update(world, plan)
    }
    /// Collect completed work and plan removals without removing live chunks.
    pub fn plan_update(&mut self, world: &mut World) -> ChunkUpdate {
        let mut step = self.begin_plan_update(world);
        loop {
            match step {
                ChunkPlanStep::Ready(plan) => return plan,
                ChunkPlanStep::Save { chunk, resume } => {
                    self.save_finished(chunk);
                    step = self.resume_plan_update(world, resume);
                }
            }
        }
    }
    /// Start planning, yielding before any late-generated column is saved.
    pub fn begin_plan_update(&mut self, world: &mut World) -> ChunkPlanStep {
        self.begin_plan_update_with(world, |_, never| match never {})
    }
    /// Resume processing the same batch after the caller saves a late column.
    pub fn resume_plan_update(
        &mut self,
        world: &mut World,
        resume: ChunkPlanResume,
    ) -> ChunkPlanStep {
        self.resume_plan_update_with(world, resume, |_, never| match never {})
    }
    /// Commit a planned update after the caller has finished with live chunks.
    pub fn finish_update(&mut self, world: &mut World, mut plan: ChunkUpdate) -> Vec<Arc<Chunk>> {
        let (commit, protos) = self.prepare_commit(world, &mut plan);
        self.submit_protos(protos);
        self.complete_commit(world, commit)
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
        if self.is_aborted() {
            return false;
        }
        let mut plan = self.plan_update(world);
        plan.suppress_unloads();
        let unloaded = self.finish_update(world, plan);
        debug_assert!(unloaded.is_empty());
        true
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
        if self.is_aborted() {
            return false;
        }
        if self.state.remote {
            // Remote columns arrive via packets, not the generation queue.
            return world.chunks.contains_key(&(x, z)) && self.set_forced(x, z, true);
        }
        if self.state.paused {
            return self.set_forced(x, z, true);
        }
        if !self.set_forced(x, z, true) {
            return false;
        }
        self.wait_for_column(world, |world| world.chunks.is_ticking_ready((x, z)));
        true
    }
}

impl<A: AuxiliaryJobs> ChunkStreamer<A> {
    pub fn with_auxiliary<G: ChunkGenerator>(
        generator: Arc<G>,
        context: Arc<ChunkWorkerContext>,
        aux: A,
        threads: usize,
    ) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(BinaryHeap::new()),
            done: Mailbox::default(),
            aborted: std::sync::atomic::AtomicBool::new(false),
        });
        let light_tables = context.light.clone();
        let saves: Arc<dyn Saves> = generator.clone();
        let pool = sunrise_threads::jobs::pool();
        let source = Arc::new(Jobs {
            shared: shared.clone(),
            generator: generator.clone(),
            light: context.light.clone(),
            aux,
            locals: sunrise_threads::jobs::PerWorker::new(pool.threads()),
        });
        let jobs = pool.register(source);
        // The work kept in flight is sized for the workers this streamer may
        // use: the pool's, or fewer when asked (tests, a placeholder).
        let workers = threads.clamp(1, pool.threads());
        let non_air_sections = {
            let generator = generator.clone();
            Arc::new(move |p: &Proto| generator.non_air_sections(p))
        };
        let proto_bytes = {
            let generator = generator.clone();
            Arc::new(move |p: &Proto| generator.proto_heap_bytes(p))
        };
        let caches = {
            let generator = generator.clone();
            Arc::new(move || generator.cache_census())
        };
        let state = ChunkState {
            spawn_height: generator.spawn_height(),
            non_air_sections,
            proto_bytes,
            caches,
            workers,
            order: 0,
            scan_age: 0,
            scan_area: ((i32::MIN, i32::MIN), i32::MIN),
            generating: Default::default(),
            failed: Default::default(),
            generation_scan: GenerationScan::default(),
            protos: Default::default(),
            light_views: Default::default(),
            decorated: Default::default(),
            dropped_new: Default::default(),
            decorating: 0,
            decorating_finals: HashSet::new(),
            saving: 0,
            viewing: Default::default(),
            decoration_order: DecorationOrder::default(),
            order_area: None,
            decoration_order_dirty: false,
            decoration_changed: false,
            decoration_revision: 0,
            decoration_basis: None,
            queue_basis: Default::default(),
            finishing: Default::default(),
            decorates: generator.decorates(),
            saves_beside_final: generator.decorates_beside_final(),
            lighting: Default::default(),
            lit: Default::default(),
            written: Default::default(),
            saves,
            light_scratch: Vec::new(),
            gone_scratch: Vec::new(),
            finalize_scratch: Vec::new(),
            sequence: 0,
            center: (0, 0),
            radius: 8,
            forced: HashSet::new(),
            commanded: HashSet::new(),
            ordered_commanded: BTreeSet::new(),
            mod_tickets: ModTickets::default(),
            paused: false,
            sky_light: true,
            biome_zoom: BiomeZoom::new(0),
            light_tables,
            light_log: Vec::new(),
            light_edit_sequence: 0,
            generated_total: 0,
            remote: false,
            relight_resent: false,
            extra: Vec::new(),
            unknown: Vec::new(),
            recent_full: std::cell::Cell::new([None; 4]),
            cover: None,
            cover_current: false,
            forced_cover: None,
            band: None,
            band_ranked: None,
            async_requests: HashSet::new(),
            async_request_cover: None,
            dirty_visibility_on_load: false,
        };
        Self {
            state,
            shared,
            deferred: Vec::new(),
            jobs,
        }
    }
    pub fn state(&self) -> &ChunkState {
        &self.state
    }
    /// Clear streaming state after resident remote columns have been removed.
    pub fn forget_columns(&mut self, world: &mut World, gone: &FxHashSet<(i32, i32)>) {
        // `ChunkTaskPriorityQueue.release` for each of them (`ChunkTaskDispatcher`'s
        // `release(col, true)` from the chunk's unload): the jobs still queued for a
        // column go with it instead of running for a level they no longer belong to.
        if !gone.is_empty()
            && let Ok(mut queue) = self.shared.queue.lock()
        {
            release_columns(&mut queue, gone, |job| {
                if let Job::Light(x, z, ..) = job {
                    self.state.lighting.remove(&(*x, *z));
                }
            });
        }
        for &(x, z) in gone {
            self.state.lit.remove(&(x, z));
            if let Some(job) = self.state.lighting.get_mut(&(x, z)) {
                job.snapshot = None;
            }
            // The column and its proto both left: not decorated any more.
            if !self.state.protos.contains_key(&(x, z)) && !self.state.finishing.contains(&(x, z)) {
                self.state.decorated.remove(&(x, z));
            }
            world.dirty_sections.take_column(x, z);
            world.entities_pending.remove(&(x, z));
        }
        if !gone.is_empty() {
            self.state.decoration_changed = true;
        }
        if self.state.lighting.is_empty() {
            self.state.light_log.clear();
        }
    }
    /// Queue a job for the column `at`, nearest first: its distance (`distance`)
    /// times four plus `bias`, which orders a column's jobs. A job without a
    /// column (a save, meshes to free) goes ahead of every other.
    fn submit(&mut self, at: Option<(i32, i32)>, bias: i64, job: Job<A::Payload>) {
        self.state.order += 1;
        let priority = at.map_or(i64::MIN, |(x, z)| self.state.distance(x, z) * 4 + bias);
        self.shared.queue.lock().unwrap().push(Queued {
            priority,
            at,
            bias,
            order: self.state.order,
            job,
        });
        self.jobs.notify();
    }
    /// Queue an inline companion payload into the shared priority heap.
    pub fn submit_auxiliary(&mut self, at: Option<(i32, i32)>, bias: i64, payload: A::Payload) {
        self.submit(at, bias, Job::Auxiliary(payload));
    }
    /// Rank the queued jobs again when what their priorities were computed
    /// from changed: the centre, the extra areas, the band, or the loading
    /// screen (ring order, `Ranges::rings`, whose values are far above the
    /// distances used after it). Vanilla's chunk task queue follows the
    /// current ticket levels in the same way. Without it, generation queued
    /// while the loading screen was up waited behind every job queued after
    /// it, and with it the decoration of the chunks around the player: at
    /// render distance 32 nothing was decorated until every proto chunk had
    /// been generated.
    fn requeue(&mut self) {
        if !forced_ring::basis_changed(&mut self.state) {
            return;
        }
        let mut queue = self.shared.queue.lock().unwrap();
        rank_again(&mut queue, |x, z| self.state.distance(x, z));
    }
    /// Queue checked-out protos ahead of other generation work.
    pub fn submit_protos(&mut self, protos: Vec<ProtoSave>) -> bool {
        if !protos.is_empty() {
            if self.state.saves.save_protos_on_owner() {
                // This only captures and queues the region records; encoding stays on workers.
                self.finish_save(protos, true);
            } else {
                self.state.saving += 1;
                self.submit(None, 0, Job::Save(protos));
            }
        }
        self.state.saving == 0
    }
    /// The 3x3 a light job reads: the finished columns, and for a neighbour
    /// that will not finish (past the area), its decorated proto chunk, as
    /// vanilla lights its last FULL ring against neighbours at FEATURES.
    /// A proto neighbour without its view yet is checked out to a worker,
    /// which builds the view off the server thread (`Job::LightView`); the
    /// region is asked for again once it is back.
    fn light_region(&mut self, world: &World, x: i32, z: i32) -> Option<Box<Region>> {
        let mut region: Region = Default::default();
        let mut complete = true;
        for dz in 0..3 {
            for dx in 0..3 {
                let n = (x + dx as i32 - 1, z + dz as i32 - 1);
                region[dz][dx] = match world.chunks.get(&n) {
                    Some(chunk) => Some(chunk.clone()),
                    None if !self.state.in_range(n.0, n.1, 1)
                        && self.state.decorated.contains(&n) =>
                    {
                        if let Some(view) = self.state.light_views.get(&n) {
                            Some(view.clone())
                        } else {
                            let proto = self.state.protos.get_mut(&n)?.take()?;
                            self.state.viewing.insert(n);
                            self.submit(Some((x, z)), 0, Job::LightView(n.0, n.1, proto));
                            complete = false;
                            None
                        }
                    }
                    None => return None,
                };
            }
        }
        complete.then(|| Box::new(region))
    }
    pub fn pending(&self) -> usize {
        self.shared.queue.lock().unwrap().len()
    }
    /// Whether any chunk is queued or on its way from generation through lighting.
    pub fn busy(&self) -> bool {
        self.state.chunks_in_flight() || self.pending() != 0
    }
    /// Whether any queued, in-flight, cached proto or extra-area work remains
    /// for an idle parked server level to drain.
    pub fn has_idle_work(&self) -> bool {
        self.busy()
            || !self.state.protos.is_empty()
            || !self.state.extra.is_empty()
            || !self.state.async_requests.is_empty()
            || !self.deferred.is_empty()
            || self.state.saving != 0
    }
    /// How much work each portable stage holds: queued jobs, then what is in
    /// flight or waiting at each step from generation through lighting.
    pub fn stages(&self) -> [(&'static str, usize); 6] {
        [
            ("queued", self.pending()),
            ("generating", self.state.generating.len()),
            (
                "protos",
                self.state.protos.values().filter(|p| p.is_some()).count(),
            ),
            ("decorating", self.state.decorating),
            ("finishing", self.state.finishing.len()),
            ("lighting", self.state.lighting.len()),
        ]
    }
    /// Request an unload sweep on the next `begin_plan_update`, even if the
    /// view center and radius did not change and fewer than 8 ticks elapsed.
    pub fn force_unload_sweep(&mut self) {
        self.scan_age = 8;
    }
    /// Start planning, yielding before submitting a late-generated column's save.
    /// Swaps out the mailbox deque under one `try_lock` and releases the guard
    /// before processing any chunk result or invoking `on_auxiliary`.
    pub fn begin_plan_update_with(
        &mut self,
        world: &mut World,
        on_auxiliary: impl FnMut(&World, A::Output),
    ) -> ChunkPlanStep<A::Output> {
        let mut hz_laps = crate::spans::laps();
        if self.state.paused {
            return ChunkPlanStep::Ready(ChunkUpdate::paused());
        }
        ExtraCover::refresh(&mut self.state.cover, &self.state.extra);
        ForcedCover::refresh(&mut self.state.forced_cover, &self.state.forced);
        let mut async_requests_changed = false;
        if self.state.async_requests.is_empty() {
            async_requests_changed = self.state.async_request_cover.is_some();
            self.state.async_request_cover = None;
        } else {
            let had_diff = self
                .state
                .async_request_cover
                .as_ref()
                .is_none_or(|c| !c.matches(&self.state.async_requests));
            if had_diff {
                ForcedCover::refresh(
                    &mut self.state.async_request_cover,
                    &self.state.async_requests,
                );
                async_requests_changed = true;
            }
        }
        if async_requests_changed {
            self.state.generation_scan.invalidate();
            self.state.decoration_order_dirty = true;
        }
        self.state.cover_current = true;
        // The sweeps below walk every loaded column and proto chunk. Run them
        // at once when the view moves and every eighth tick otherwise: a
        // column can stay a little past the edge without anyone noticing.
        let area = (self.state.center, self.state.radius);
        // The sweep unloads columns, saving them and dropping their block entities' bindings:
        // never while a transaction is open, whose values may still be tentative. A due sweep
        // waits for the next update instead.
        let sweep = (area != self.state.scan_area || self.state.scan_age >= 8)
            && !world.block_entities.defers_retirement();
        if sweep {
            (self.state.scan_area, self.state.scan_age) = (area, 0);
        } else {
            self.state.scan_age = self.state.scan_age.saturating_add(1);
        }
        self.requeue();
        self.state.relight_edits(world);
        hz_laps.lap("st_relight_edits");
        let deferred = std::mem::take(&mut self.deferred).into_iter();
        let received = self.shared.done.take();
        self.resume_plan_update_with(
            world,
            ChunkPlanResume {
                sweep,
                deferred,
                received,
            },
            on_auxiliary,
        )
    }
    /// Resume processing the same batch after the caller submits a late column's save.
    pub fn resume_plan_update_with(
        &mut self,
        world: &mut World,
        mut resume: ChunkPlanResume<A::Output>,
        mut on_auxiliary: impl FnMut(&World, A::Output),
    ) -> ChunkPlanStep<A::Output> {
        let mut hz_laps = crate::spans::laps();
        while let Some(mail) = resume
            .deferred
            .next()
            .or_else(|| resume.received.pop_front())
        {
            match mail {
                Mail::Chunk(done) => {
                    // Any result but light or a mesh can leave a proto or column
                    // decorated (or gone): the next commit finalizes and decorates.
                    if !matches!(&done, Done::Lit(..)) {
                        self.state.decoration_changed = true;
                    }
                    match done {
                        Done::Proto(x, z, mut proto, decorated, saved) => {
                            self.state.generation_scan.exhausted = false;
                            self.state.generating.remove(&(x, z));
                            if self.state.in_range(x, z, 4) || self.state.near_finished(x, z, 2) {
                                let decorated = saved.map_or(decorated, |saved| {
                                    self.state.saves.accept_loaded_proto(&mut proto, *saved)
                                });
                                self.state.light_views.remove(&(x, z));
                                self.state.protos.insert((x, z), Some(proto));
                                if decorated || !self.state.decorates {
                                    self.state.decorated.insert((x, z));
                                    ChunkState::publish_proto_light_support(
                                        world,
                                        (x, z),
                                        self.state.proto_sections(x, z),
                                    );
                                }
                            }
                        }
                        Done::Failed(x, z, reason) => self.state.receive_failed(x, z, reason),
                        Done::Decorated(x, z, region, finals, writes) => self
                            .state
                            .receive_decorated(x, z, *region, finals, writes, world),
                        Done::Generated(chunk) => {
                            if let Some(chunk) = self.state.receive_generated(chunk, world) {
                                return ChunkPlanStep::Save { chunk, resume };
                            }
                        }
                        Done::Saved(protos) => self.state.receive_saved(protos),
                        Done::LightView(x, z, proto, view) => {
                            self.state.receive_light_view(x, z, proto, view)
                        }
                        Done::Lit(x, z, sections, seq) => {
                            self.state.receive_lit(world, x, z, sections, seq)
                        }
                    }
                }
                Mail::Auxiliary(output) => {
                    on_auxiliary(world, output);
                }
            }
        }
        hz_laps.lap("st_results");
        let far = if resume.sweep {
            plan_unloads(world, |x, z| {
                !self.state.remote && self.state.ranges().unloaded(x, z, self.state.decorates)
            })
        } else {
            HashSet::new()
        };
        ChunkPlanStep::Ready(ChunkUpdate {
            sweep: resume.sweep,
            far,
            paused: false,
        })
    }
    /// Resume an update after submitting its proto saves, clearing dirty sections
    /// and ending the cover window on active commits.
    pub fn complete_commit(&mut self, world: &mut World, commit: ChunkCommit) -> Vec<Arc<Chunk>> {
        if commit.is_paused() {
            return commit.into_removed();
        }
        let removed = self.complete_commit_without_clearing_dirty(world, commit);
        world.dirty_sections.clear();
        self.state.finish_update_cover();
        removed
    }
    /// Run finalize, decoration, generation and lighting without clearing `world.dirty_sections`
    /// or resetting `cover_current`, so a companion meshing pass can consume both first.
    pub fn complete_commit_without_clearing_dirty(
        &mut self,
        world: &mut World,
        commit: ChunkCommit,
    ) -> Vec<Arc<Chunk>> {
        let ChunkCommit {
            removed,
            sweep_far: _,
            paused,
        } = commit;
        if paused {
            return removed;
        }
        let mut hz_laps = crate::spans::laps();
        // `decorated` is pruned where a proto chunk or a column leaves, not by
        // scanning every decorated key each tick: a sweep at 20 exploring bots
        // paid 0.6 ms of the tick for that scan alone.
        // Finalizing and decorating read only proto chunks, decorated keys,
        // finished columns and the ranges. Light and mesh results change none
        // of them, and about a third of a busy server's commits carry only
        // those; they can skip both passes and their scans.
        let membership = world.chunks.membership_revision();
        let membership_changed = membership != self.state.decoration_revision;
        let ranges_changed = self.state.decoration_ranges_changed();
        let run_decoration = self.state.decoration_changed || membership_changed || ranges_changed;
        self.state.decoration_changed = false;
        // Reconcile against the revision the last pass recorded, before
        // advancing it: the removed deltas are replayed from there.
        if run_decoration && membership_changed {
            self.state.reconcile_decorated(world);
        }
        self.state.decoration_revision = membership;
        let workers = self.state.workers;
        if run_decoration {
            // Finalize chunks whose neighbourhood is decorated.
            let mut finalize_candidates = std::mem::take(&mut self.state.finalize_scratch);
            finalize_candidates.clear();
            finalize_candidates.extend(self.state.protos.keys().copied());
            for &(x, z) in &finalize_candidates {
                if world.chunks.contains_key(&(x, z))
                    || self.state.finishing.contains(&(x, z))
                    || !self.state.in_range(x, z, 1)
                {
                    continue;
                }
                let ready = (-1..=1).all(|dz| {
                    (-1..=1).all(|dx| {
                        self.state.decorated.contains(&(x + dx, z + dz))
                            || world.chunks.contains_key(&(x + dx, z + dz))
                    })
                });
                if ready && let Some(Some(_)) = self.state.protos.get(&(x, z)) {
                    let proto = self
                        .state
                        .protos
                        .remove(&(x, z))
                        .flatten()
                        .expect("present proto");
                    self.state.light_views.remove(&(x, z));
                    self.state.finishing.insert((x, z));
                    self.state.written.remove(&(x, z));
                    self.submit(Some((x, z)), 0, Job::Finish(proto));
                }
            }
            self.state.finalize_scratch = finalize_candidates;
            hz_laps.lap("st_finalize");
            // Decorate chunks whose 3x3 proto neighbourhood is available, in
            // vanilla's order: decoration is order dependent where write regions
            // overlap (see `DecorationOrder`).
            let area_changed =
                self.state.order_area != Some((self.state.center, self.state.radius));
            if self.state.decorates && (area_changed || self.state.decoration_order_dirty) {
                if area_changed {
                    let (decorated, chunks) = (&self.state.decorated, &world.chunks);
                    // The render distance is the server view distance in singleplayer.
                    self.state.decoration_order.reassign(
                        self.state.center,
                        self.state.radius,
                        |x, z| decorated.contains(&(x, z)) || chunks.contains_key(&(x, z)),
                    );
                    self.state.order_area = Some((self.state.center, self.state.radius));
                }
                self.state.decoration_order_dirty = false;
                let ranges = Ranges {
                    center: self.state.center,
                    radius: self.state.radius,
                    extra: &self.state.extra,
                    forced: &self.state.forced,
                    cover: ExtraCover::current(
                        &self.state.cover,
                        &self.state.extra,
                        self.state.cover_current,
                    ),
                    forced_cover: self
                        .state
                        .forced_cover
                        .as_ref()
                        .filter(|_| self.state.cover_current),
                    band: self.state.band.as_ref(),
                    async_requests: (!self.state.async_requests.is_empty()).then(|| {
                        AsyncRequests {
                            chunks: &self.state.async_requests,
                            cover: self
                                .state
                                .async_request_cover
                                .as_ref()
                                .filter(|_| self.state.cover_current),
                        }
                    }),
                };
                let (protos, generating) = (&self.state.protos, &self.state.generating);
                self.state.decoration_order.retain(|x, z| {
                    ranges.in_range(x, z, 4)
                        || ranges.near_finished(x, z, 2)
                        || protos.contains_key(&(x, z))
                        || generating.contains(&(x, z))
                });
            }
            if self.state.decorates && self.state.decorating < workers * 2 {
                let wants = |s: &ChunkState, x: i32, z: i32| {
                    !s.decorated.contains(&(x, z))
                        && !world.chunks.contains_key(&(x, z))
                        && (s.in_range(x, z, 2) || s.near_finished(x, z, 1))
                };
                // A wanted chunk holds back higher-ranked ones only if it can be
                // decorated: its 3x3 neighbourhood is (or will be) generated as
                // protos, none of them already final.
                let waits_for = |s: &ChunkState, x: i32, z: i32| {
                    wants(s, x, z)
                        && (-1..=1).all(|dz| {
                            (-1..=1).all(|dx| {
                                let n = (x + dx, z + dz);
                                !world.chunks.contains_key(&n)
                                    && !s.failed.contains(&n)
                                    && (s.protos.contains_key(&n)
                                        || s.generating.contains(&n)
                                        || s.ranges().requested(n.0, n.1, true))
                            })
                        })
                };
                // Out for the call: ranking reads the streamer's ranges.
                let mut order = std::mem::take(&mut self.state.decoration_order);
                let mut wanted = decoration_queue(
                    &mut order,
                    self.state
                        .protos
                        .iter()
                        .filter(|&(&(x, z), proto)| proto.is_some() && wants(&self.state, x, z))
                        .map(|(&k, _)| k),
                    |x, z| self.state.ranges().priority(x, z),
                );
                self.state.decoration_order = order;
                // Most wanted chunks are blocked, and the same neighbours are
                // asked about for each of them: what `waits_for` says is kept for
                // the pass (nothing it reads changes in it but `decorated`, which
                // updates the entry). Without it, scheduling at render distance 32
                // took most of the thread that runs it while the world loaded.
                let waiting: std::cell::RefCell<FxHashMap<(i32, i32), bool>> = Default::default();
                let mut initialized = Vec::new();
                while let Some(std::cmp::Reverse((_, x, z))) = wanted.pop() {
                    if self.state.decorating >= workers * 2 {
                        break;
                    }
                    if self.state.decoration_order.blocked(x, z, |nx, nz| {
                        if let Some(&known) = waiting.borrow().get(&(nx, nz)) {
                            return known;
                        }
                        let known = waits_for(&self.state, nx, nz);
                        waiting.borrow_mut().insert((nx, nz), known);
                        known
                    }) {
                        continue;
                    }
                    let keys: [(i32, i32); 9] =
                        std::array::from_fn(|i| (x + (i as i32 % 3) - 1, z + (i as i32 / 3) - 1));
                    // A final neighbour cannot be generated again: a generator that
                    // can decorates beside it (a chunk of an older world), writing
                    // into it as vanilla's features do; the others skip decoration
                    // (worlds saved before proto chunks were saved get here: their
                    // final chunks' neighbours are generated again).
                    if keys
                        .iter()
                        .any(|k| self.state.decorating_finals.contains(k))
                    {
                        continue;
                    }
                    let has_final = keys.iter().any(|k| world.chunks.contains_key(k));
                    if has_final && !self.state.saves_beside_final {
                        self.state.decorated.insert((x, z));
                        initialized.push((x, z));
                        waiting.borrow_mut().insert((x, z), false);
                        continue;
                    }
                    if !keys.iter().all(|k| {
                        world.chunks.contains_key(k)
                            || matches!(self.state.protos.get(k), Some(Some(_)))
                    }) {
                        continue;
                    }
                    let finals: Vec<(usize, Chunk)> = keys
                        .iter()
                        .enumerate()
                        .filter_map(|(i, k)| Some((i, (**world.chunks.get(k)?).clone())))
                        .collect();
                    let region: Box<[Proto; 9]> = Box::new(std::array::from_fn(|i| {
                        let k = &keys[i];
                        if finals.iter().any(|(f, _)| *f == i) {
                            return Box::new(()) as Proto;
                        }
                        self.state.light_views.remove(k);
                        self.state
                            .protos
                            .insert(*k, None)
                            .flatten()
                            .expect("present proto")
                    }));
                    let outer = ChunkState::outer_decoration_columns(
                        &world.chunks,
                        &self.state.light_views,
                        x,
                        z,
                    );
                    self.state.decorating += 1;
                    self.state
                        .decorating_finals
                        .extend(finals.iter().map(|(i, _)| keys[*i]));
                    self.submit(Some((x, z)), 1, Job::Decorate(x, z, region, finals, outer));
                }
                if !initialized.is_empty() {
                    // These became decorated after this call's finalize pass:
                    // the next commit must look at the protos they unblock.
                    self.state.decoration_changed = true;
                }
                for key in initialized {
                    ChunkState::publish_proto_light_support(
                        world,
                        key,
                        self.state.proto_sections(key.0, key.1),
                    );
                }
            }
        }
        hz_laps.lap("st_decorate");
        // Generation only gains candidates from the same changes as decoration
        // (a proto, a finished column or the ranges); a light or mesh result
        // leaves its inputs alone.
        if run_decoration {
            self.schedule_generation(world, workers);
        }
        hz_laps.lap("st_generate");
        self.schedule_light(world);
        hz_laps.lap("st_light");
        removed
    }
    /// Schedule queued whole-column light jobs once all neighbours exist, nearest first.
    fn schedule_light(&mut self, world: &mut World) {
        let room = (self.state.workers * 4).saturating_sub(self.state.lighting.len());
        if room == 0 {
            return;
        }
        let mut light = std::mem::take(&mut self.state.light_scratch);
        light.clear();
        let mut gone = std::mem::take(&mut self.state.gone_scratch);
        gone.clear();
        for &(x, z) in world.dirty_light.iter() {
            if !world.chunks.contains_key(&(x, z)) {
                gone.push((x, z));
            } else if !self.state.lighting.contains_key(&(x, z))
                && self.state.light_region_ready(world, x, z)
            {
                light.push((self.state.distance(x, z), x, z));
            }
        }
        for key in gone.drain(..) {
            world.dirty_light.remove(&key);
        }
        nearest_first(&mut light, room);
        for (_, x, z) in light.drain(..) {
            let Some(region) = self.light_region(world, x, z) else {
                continue;
            };
            world.dirty_light.remove(&(x, z));
            self.state.sequence += 1;
            let seq = self.state.sequence;
            let sky = self.state.sky_light;
            self.state.lighting.insert(
                (x, z),
                LightJob {
                    seq,
                    snapshot: Some(self.state.light_edit_sequence),
                },
            );
            self.submit(Some((x, z)), 1, Job::Light(x, z, region, seq, sky));
        }
        self.state.light_scratch = light;
        self.state.gone_scratch = gone;
    }
    /// Advance light work only, without streaming generation, unloads or meshing.
    /// Used for parked server levels so relights queued on them run and batch on
    /// their own worker pool without waiting for that dimension to be ticked.
    pub fn update_light(&mut self, world: &mut World) {
        if self.state.paused {
            return;
        }
        self.state.relight_edits(world);
        let pending = std::mem::take(&mut self.deferred)
            .into_iter()
            .chain(self.shared.done.take());
        for mail in pending {
            match mail {
                Mail::Chunk(Done::LightView(x, z, proto, view)) => {
                    self.state.receive_light_view(x, z, proto, view);
                }
                Mail::Chunk(Done::Lit(x, z, sections, seq)) => {
                    self.state.receive_lit(world, x, z, sections, seq);
                }
                other => self.deferred.push(other),
            }
        }
        self.schedule_light(world);
    }
    fn schedule_generation(&mut self, world: &World, workers: usize) {
        let input = GenerationInputs {
            ranges: Ranges {
                center: self.state.center,
                radius: self.state.radius,
                extra: &self.state.extra,
                forced: &self.state.forced,
                cover: ExtraCover::current(
                    &self.state.cover,
                    &self.state.extra,
                    self.state.cover_current,
                ),
                forced_cover: self
                    .state
                    .forced_cover
                    .as_ref()
                    .filter(|_| self.state.cover_current),
                band: self.state.band.as_ref(),
                async_requests: (!self.state.async_requests.is_empty()).then(|| AsyncRequests {
                    chunks: &self.state.async_requests,
                    cover: self
                        .state
                        .async_request_cover
                        .as_ref()
                        .filter(|_| self.state.cover_current),
                }),
            },
            world,
            generating: &self.state.generating,
            failed: &self.state.failed,
            protos: &self.state.protos,
            finishing: &self.state.finishing,
            decorates: self.state.decorates,
            remote: self.state.remote,
        };
        if let Some(wanted) = self.state.generation_scan.candidates(input, workers * 3) {
            for (_, x, z) in wanted {
                self.state.generating.insert((x, z));
                self.submit(Some((x, z)), 0, Job::Generate(x, z));
            }
        }
    }
    /// Check whether a non-blocking mod chunk request at `chunk` can be admitted.
    pub fn check_async_request(&self, world: &World, chunk: (i32, i32)) -> FullColumn {
        if !chunk_pos_is_valid(chunk) || self.state.remote || self.state.paused || self.is_aborted()
        {
            return FullColumn::Refused;
        }
        if full_column_ready(world, chunk) {
            return FullColumn::Ready;
        }
        if self.state.full_column_failed(chunk) {
            return FullColumn::Failed;
        }
        FullColumn::Pending
    }
    /// Whether generation has been aborted (`abort_generation`).
    pub fn is_aborted(&self) -> bool {
        self.shared.aborted.load(Ordering::Relaxed)
    }
    /// Abort unvisited background chunk generation, decoration, lighting, and companion jobs on world exit.
    pub fn abort_generation(&mut self) {
        self.shared
            .aborted
            .store(true, std::sync::atomic::Ordering::SeqCst);
        if let Ok(mut queue) = self.shared.queue.lock() {
            let mut saves = Vec::new();
            for item in std::mem::take(&mut *queue).into_vec() {
                if matches!(item.job, Job::Save(..)) {
                    saves.push(item);
                }
            }
            *queue = std::collections::BinaryHeap::from(saves);
        }
        self.state.generating.clear();
        self.state.failed.clear();
        self.state.decorating = 0;
        self.state.decorating_finals.clear();
        self.state.finishing.clear();
        self.state.viewing.clear();
        self.state.lighting.clear();
        self.state.light_log.clear();
        self.jobs.notify_all();
    }
    /// Save what generation holds outside the world: the proto chunks
    /// decoration wrote to. With `wait`, the decorations, columns and saves
    /// in flight are waited for first (their proto chunks are out in the
    /// jobs) and the protos are saved here, before the world's chunks;
    /// otherwise a worker saves them (vanilla's autosave hands its chunks to
    /// the IOWorker too), and those out in jobs are saved later, as they
    /// unload or at the next save.
    pub fn save(&mut self, world: &mut World, wait: bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            match self.prepare_save(world, wait, deadline) {
                SaveStep::Full(chunk) => self.save_finished(chunk),
                SaveStep::Protos(protos) => {
                    self.finish_save(protos, wait);
                    return;
                }
                SaveStep::Pending => unreachable!(),
            }
        }
    }
    /// Check out the proto saves, yielding for late full saves first.
    pub fn prepare_save(
        &mut self,
        world: &mut World,
        wait: bool,
        deadline: std::time::Instant,
    ) -> SaveStep {
        if !wait {
            let written: Vec<(i32, i32)> = self.state.written.drain().collect();
            return SaveStep::Protos(self.state.checkout_protos(written));
        }
        let aborted = self.is_aborted();
        while (self.state.saving > 0)
            || (!aborted
                && (self.state.decorating > 0
                    || !self.state.finishing.is_empty()
                    || !self.state.viewing.is_empty()))
        {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.shared.done.recv_timeout(left) {
                Some(Mail::Chunk(Done::Decorated(x, z, region, finals, writes))) => self
                    .state
                    .receive_decorated(x, z, *region, finals, writes, world),
                Some(Mail::Chunk(Done::Generated(chunk))) => {
                    if let Some(chunk) = self.state.receive_generated(chunk, world) {
                        return SaveStep::Full(chunk);
                    }
                }
                Some(Mail::Chunk(Done::Saved(protos))) => self.state.receive_saved(protos),
                Some(Mail::Chunk(Done::LightView(x, z, proto, view))) => {
                    self.state.receive_light_view(x, z, proto, view)
                }
                Some(other) => self.deferred.push(other),
                None => {
                    log::warn!("Sunrise: chunk generation did not finish before saving");
                    break;
                }
            }
        }
        let written: Vec<_> = self.state.written.drain().collect();
        SaveStep::Protos(self.state.checkout_protos(written))
    }
    /// Complete a prepared save, returning synchronous protos to streaming.
    pub fn finish_save(&mut self, protos: Vec<ProtoSave>, wait: bool) {
        if wait {
            if !protos.is_empty() {
                let refs: Vec<&Proto> = protos.iter().map(|p| &p.proto).collect();
                self.state.saves.save_protos(&refs);
                for proto in protos {
                    self.state.protos.insert(proto.pos, Some(proto.proto));
                }
            }
        } else {
            self.submit_protos(protos);
        }
    }
    /// Finish generation for a closing world without waiting on the event
    /// loop. No new generation is scheduled while the save screen is shown.
    /// Poll generation and yield for each late full save before proto submission.
    /// The caller keeps the same deadline across every yield in this tick.
    pub fn prepare_poll_save(
        &mut self,
        world: &mut World,
        deadline: std::time::Instant,
    ) -> SaveStep {
        self.prepare_poll_save_with_clock(world, deadline, std::time::Instant::now)
    }
    fn prepare_poll_save_with_clock(
        &mut self,
        world: &mut World,
        deadline: std::time::Instant,
        mut now: impl FnMut() -> std::time::Instant,
    ) -> SaveStep {
        while now() < deadline {
            let Some(done) = self.shared.done.try_recv() else {
                break;
            };
            match done {
                Mail::Chunk(Done::Decorated(x, z, region, finals, writes)) => self
                    .state
                    .receive_decorated(x, z, *region, finals, writes, world),
                Mail::Chunk(Done::Generated(chunk)) => {
                    if let Some(chunk) = self.state.receive_generated(chunk, world) {
                        return SaveStep::Full(chunk);
                    }
                }
                Mail::Chunk(Done::Saved(protos)) => self.state.receive_saved(protos),
                Mail::Chunk(Done::LightView(x, z, proto, view)) => {
                    self.state.receive_light_view(x, z, proto, view)
                }
                other => self.deferred.push(other),
            }
        }
        if now() >= deadline {
            return SaveStep::Pending;
        }
        let aborted = self.is_aborted();
        if (self.state.saving > 0)
            || (!aborted
                && (self.state.decorating > 0
                    || !self.state.finishing.is_empty()
                    || !self.state.viewing.is_empty()))
        {
            return SaveStep::Pending;
        }
        let written: Vec<_> = self.state.written.drain().collect();
        SaveStep::Protos(self.state.checkout_protos(written))
    }
    /// The start of `Level.getChunk(x, z)`, that is `ServerChunkCache.getChunk(x, z, FULL, true)`,
    /// for the caller's synchronous load (the game pumps it, tells the mods of the columns it
    /// brings in, and calls [`ChunkState::end_full_column`] once [`full_column_ready`] or the pump
    /// gives up, which is vanilla's `managedBlock` with no timeout).
    ///
    /// A column among the level's recent FULL lookups answers at once with no ticket, as
    /// `getChunk`'s four-entry cache does. Any other lookup places an `UNKNOWN` ticket at FULL's
    /// level on the column, or resets its ticks left (`TicketStorage.addTicket`); the ticket
    /// outlives the call and keeps only that column, not ticking, until
    /// [`ChunkState::purge_unknown_tickets`] expires it. A column that is not FULL yet (missing, or
    /// resident but not lit) clears the recent lookups, as `runDistanceManagerUpdates` does, and
    /// finishes with its 3x3 for the wait (`FULL_WAIT_RADIUS`). Refused for a chunk past
    /// `ChunkPos.isValid`, and for a column that is not FULL while streaming is paused or its
    /// generation stopped; a remote streamer answers only for a FULL column it holds, with no
    /// ticket. A column that is not FULL and that a failed generation keeps from it
    /// ([`ChunkState::full_column_failed`]) is `Failed`, with no ticket.
    pub fn begin_full_column(&mut self, world: &World, chunk: (i32, i32)) -> FullColumn {
        if !chunk_pos_is_valid(chunk) {
            return FullColumn::Refused;
        }
        let ready = full_column_ready(world, chunk);
        if self.state.remote || (ready && self.state.recent_full.get().contains(&Some(chunk))) {
            return if ready {
                FullColumn::Ready
            } else {
                FullColumn::Refused
            };
        }
        if !ready && (self.state.paused || self.is_aborted()) {
            return FullColumn::Refused;
        }
        if !ready && self.state.full_column_failed(chunk) {
            return FullColumn::Failed;
        }
        match self.state.unknown.iter_mut().find(|(c, _)| *c == chunk) {
            Some((_, left)) => *left = UNKNOWN_TICKET_TIMEOUT,
            None => self.state.unknown.push((chunk, UNKNOWN_TICKET_TIMEOUT)),
        }
        if ready {
            self.state.remember_full(chunk);
            return FullColumn::Ready;
        }
        self.state.clear_full_lookups();
        self.state.extra.push((chunk, FULL_WAIT_RADIUS));
        self.state.rank_full_task(chunk.0, chunk.1);
        FullColumn::Pending
    }
    /// What holds up the columns `keys`, for a task that stopped advancing:
    /// how many are not loaded, loaded but unlit with a light job in flight,
    /// and unlit with none (those whose neighbours are all loaded are queued
    /// for light again).
    pub fn unstick_light(&mut self, world: &mut World, keys: &[(i32, i32)]) -> [usize; 3] {
        let mut counts = [0; 3];
        for &key in keys {
            if !world.chunks.contains_key(&key) {
                counts[0] += 1;
            } else if self.state.lit.contains(&key) {
            } else if self.state.lighting.contains_key(&key) {
                counts[1] += 1;
            } else {
                counts[2] += 1;
                if self.light_region(world, key.0, key.1).is_some()
                    || (-1..=1).any(|dz| {
                        (-1..=1).any(|dx| self.state.viewing.contains(&(key.0 + dx, key.1 + dz)))
                    })
                {
                    world.dirty_light.insert(key);
                }
            }
        }
        counts
    }
    /// Wait up to `timeout` for a worker to finish a job, leaving the result
    /// queued. Return time parked on the worker mailbox, zero if work was ready.
    pub fn wait(&mut self, timeout: Duration) -> Duration {
        self.shared.done.wait(timeout, Instant::now).1
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn send_auxiliary_done_for_test(&self, output: A::Output) {
        self.shared.done.send(Mail::Auxiliary(output));
    }
}

/// `ChunkTrackingView.isInViewDistance`: whether the column (x, z) is drawn
/// at this render distance around `center`, the camera's column
/// (`SectionOcclusionGraph.isInViewDistance`). Each axis gets a column of
/// slack, so this reaches past the circle of `radius` columns.
pub fn in_view_distance(center: (i32, i32), radius: i32, x: i32, z: i32) -> bool {
    let gap = |d: i32| i64::from((d.abs() - 1).max(0));
    let (dx, dz) = (gap(x - center.0), gap(z - center.1));
    dx * dx + dz * dz < i64::from(radius) * i64::from(radius)
}
/// How long a view's columns are waited for (`ViewArrival`): a server may never
/// fill it.
const VIEW_WAIT: std::time::Duration = std::time::Duration::from_secs(30);
/// Whether the columns of a view (centre, radius) have all arrived, as a client
/// sees them come: a server forgets the old view's columns, then sends the new
/// ones nearest first, so the view's outermost ones come last. Single player
/// joins its integrated server the same way, so `ChunkStreamer::busy` cannot see
/// what is not sent yet.
#[derive(Default)]
pub struct ViewArrival {
    view: ((i32, i32), i32),
    since: Option<std::time::Instant>,
    arrived: bool,
}
impl ViewArrival {
    /// Whether columns of `view` have yet to arrive (`has`): its outermost ones,
    /// at both ends of each of its rows and columns (`in_view_distance`). Once
    /// they have, false until the view changes; false too once it is no longer
    /// `recent`, and for a radius below 0 (no view).
    pub fn arriving(
        &mut self,
        view: ((i32, i32), i32),
        now: std::time::Instant,
        has: impl Fn((i32, i32)) -> bool,
    ) -> bool {
        if self.since.is_none() || view != self.view {
            (self.view, self.since, self.arrived) = (view, Some(now), false);
        }
        let ((x, z), r) = view;
        if self.arrived || r < 0 || !self.recent(now) {
            return false;
        }
        self.arrived = (-r..=r).all(|d| {
            // How far row (and column) `d` reaches: `in_view_distance` solved for it.
            let gap = i64::from((d.abs() - 1).max(0));
            let room = i64::from(r).pow(2) - gap * gap;
            room <= 0 || {
                let e = (room - 1).isqrt() as i32 + 1;
                // Both ends of row `d`, and of column `d`.
                [-e, e]
                    .into_iter()
                    .all(|e| has((x + d, z + e)) && has((x + e, z + d)))
            }
        });
        !self.arrived
    }
    /// Whether the view `arriving` was last given changed less than `VIEW_WAIT`
    /// ago: until then sections meshed are its terrain coming in, later an edit's.
    pub fn recent(&self, now: std::time::Instant) -> bool {
        self.since
            .is_some_and(|since| now.saturating_duration_since(since) < VIEW_WAIT)
    }
}
/// How [`ChunkStreamer::begin_full_column`] answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullColumn {
    /// The column is FULL: nothing to wait for.
    Ready,
    /// Pump until [`full_column_ready`], then [`ChunkState::end_full_column`].
    Pending,
    /// Not a column this level can bring to FULL now.
    Refused,
    /// A generation the column needs panicked ([`ChunkState::full_column_failed`]).
    Failed,
}

/// Whether column `chunk` is FULL: resident and lit (vanilla's FULL follows the LIGHT step).
pub fn full_column_ready(world: &World, chunk: (i32, i32)) -> bool {
    world.chunks.get(&chunk).is_some_and(|c| c.lit)
}

/// The square a synchronous FULL load finishes while it waits: the column and its 3x3. Vanilla
/// lights the column against neighbours at INITIALIZE_LIGHT (FULL's level 33 gives them 34), but
/// streaming finalizes a column before it decorates the neighbours its own decoration read, so a
/// column finished alone (radius -1) is never lit; its neighbours finished too light it. Only the
/// column keeps a ticket afterwards: the neighbours go with the next unload sweep.
const FULL_WAIT_RADIUS: i32 = 0;

/// `TicketType.UNKNOWN`'s timeout, in ticks.
const UNKNOWN_TICKET_TIMEOUT: i8 = 1;

/// `ChunkPos.isValid`: the chunk lies within `ChunkPyramid.MAX_CHUNK_COORDINATE_VALUE` on both
/// axes, the packed `BlockPos`'s 26-bit horizontal bound in chunks less its safety margin (the
/// view distance and FULL's 12 accumulated dependency rings, plus one, twice). Vanilla's block
/// reads and writes refuse a position past it (`Level.isInValidBounds`) before `getChunk`.
pub fn chunk_pos_is_valid((x, z): (i32, i32)) -> bool {
    const MAX_CHUNK_COORDINATE_VALUE: u32 = (((1 << 25) - 1) >> 4) - (32 + 12 + 1) * 2;
    x.unsigned_abs().max(z.unsigned_abs()) <= MAX_CHUNK_COORDINATE_VALUE
}

/// The rings around a `/forceload`ed chunk, in chunks. Vanilla's FORCED
/// ticket is level 31 (`ChunkMap.FORCED_TICKET_LEVEL`), the level rises one
/// per ring (`ChunkTracker.computeLevelFromNeighbor`), and the level picks
/// the column's status (`ChunkLevel.generationStatus`): full (33) two rings
/// out, features (34) at three, terrain (35) at four.
const FORCED_FULL_RING: i32 = 2;
const FORCED_FEATURES_RING: i32 = 3;
const FORCED_TERRAIN_RING: i32 = 4;
#[derive(Clone, Copy)]
struct AsyncRequests<'a> {
    chunks: &'a HashSet<(i32, i32)>,
    cover: Option<&'a ForcedCover>,
}
/// The areas kept loaded: circles around the player and `extra`, and forced chunks.
struct Ranges<'a> {
    center: (i32, i32),
    radius: i32,
    extra: &'a [((i32, i32), i32)],
    forced: &'a HashSet<(i32, i32)>,
    /// `extra` indexed by a grid or buckets: coverage queries without a
    /// scan of every area.
    cover: Option<&'a ExtraCover>,
    /// The combined `forced` set by distance (`ChunkState::forced_cover`), while it matches.
    forced_cover: Option<&'a ForcedCover>,
    band: Option<&'a Band>,
    /// Active non-blocking mod chunk requests at `FULL` (level 33).
    async_requests: Option<AsyncRequests<'a>>,
}
/// The chunks a pregeneration task (`/pregen`) wants lit now: rows `z0..=z1`
/// of the columns `x0..=x1`, those within `circle` (a centre and a radius in
/// chunks) when there is one. Like an extra area, it finishes its chunks and
/// the ring around them, so that its own chunks can be lit, and generation
/// reaches past that. A task moves it down a strip of columns a row at a
/// time, so each chunk is generated once and only the rows in flight stay
/// loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Band {
    pub x0: i32,
    pub x1: i32,
    pub z0: i32,
    pub z1: i32,
    pub circle: Option<((i32, i32), i32)>,
}
impl Band {
    /// Whether (x, z) lies within `margin` (Chebyshev) of one of the band's
    /// chunks: the square of side `2 * margin + 1` around it meets the
    /// rectangle at a point inside the circle.
    pub fn within(&self, x: i32, z: i32, margin: i32) -> bool {
        let (x0, x1) = (
            x.saturating_sub(margin).max(self.x0),
            x.saturating_add(margin).min(self.x1),
        );
        let (z0, z1) = (
            z.saturating_sub(margin).max(self.z0),
            z.saturating_add(margin).min(self.z1),
        );
        if x0 > x1 || z0 > z1 {
            return false;
        }
        self.circle.is_none_or(|((cx, cz), r)| {
            let dx = i64::from(cx.clamp(x0, x1)) - i64::from(cx);
            let dz = i64::from(cz.clamp(z0, z1)) - i64::from(cz);
            dx * dx + dz * dz <= i64::from(r) * i64::from(r)
        })
    }
    /// The band's chunks go after the players' (lower runs first), row by
    /// row, as the task wants them lit.
    fn priority(&self, x: i32, z: i32) -> i64 {
        const AFTER_PLAYERS: i64 = 1 << 40;
        AFTER_PLAYERS
            + (i64::from(z - self.z0 + 8).max(0) << 20)
            + i64::from(x - self.x0 + 8).max(0)
    }
}

/// For every column near the extra areas, how far it lies outside the
/// nearest one's square (Chebyshev distance less its radius; 0 inside). A server has an area per player, and streaming asks of every
/// loaded and wanted column whether one reaches it, several times a tick:
/// with fifty players, a scan of fifty squares per question.
///
/// Areas spread too far apart for that grid (players thousands of blocks from
/// each other) are found by the square of [`Self::BUCKET`] columns their centre
/// is in instead, which also answers which centre is nearest a column
/// (`Ranges::distance`, asked of every wanted column to rank it).
struct ExtraCover {
    extra: Vec<((i32, i32), i32)>,
    grid: Option<CoverGrid>,
    /// The indices in `extra` of the areas centred in each bucket.
    buckets: FxHashMap<(i32, i32), Vec<u32>>,
    /// The buckets' bounding box, inclusive: (x0, z0, x1, z1).
    bucket_box: (i32, i32, i32, i32),
    /// The largest radius in `extra`.
    radius: i32,
}
struct CoverGrid {
    x0: i32,
    z0: i32,
    width: u32,
    depth: u32,
    cells: Box<[i8]>,
}
impl ExtraCover {
    /// The largest margin the grid answers for; beyond it (and outside the
    /// grid) a column is further than that from every square.
    const REACH: i32 = 4;
    /// The largest grid, in columns (a mebibyte).
    const CELLS: u64 = 1 << 20;
    /// The side of a bucket, in columns.
    const BUCKET: i32 = 64;
    /// The farthest centre kept in buckets: far enough for any world border.
    const LIMIT: i32 = 1 << 26;
    fn matching<'a>(
        cover: &'a Option<ExtraCover>,
        extra: &[((i32, i32), i32)],
    ) -> Option<&'a ExtraCover> {
        cover.as_ref().filter(|c| c.extra == extra)
    }
    /// [`Self::matching`], told whether `cover` is known to match: the
    /// comparison reads every area, and `update` asks for each column it
    /// looks at (a server holds an area per player).
    fn current<'a>(
        cover: &'a Option<ExtraCover>,
        extra: &[((i32, i32), i32)],
        known: bool,
    ) -> Option<&'a ExtraCover> {
        if known {
            debug_assert!(cover.as_ref().is_none_or(|c| c.extra == extra));
            return cover.as_ref();
        }
        Self::matching(cover, extra)
    }
    /// Rebuild the coverage index if `extra` changed; none for one area or none
    /// (a scan is as quick).
    fn refresh(cover: &mut Option<ExtraCover>, extra: &[((i32, i32), i32)]) {
        if cover.as_ref().is_some_and(|c| c.extra == extra) {
            return;
        }
        *cover = Self::new(extra);
    }
    fn new(extra: &[((i32, i32), i32)]) -> Option<Self> {
        if extra.len() < 2
            || extra.iter().any(|&((x, z), r)| {
                // -1: an `UNKNOWN` ticket's square of its column alone (`hold_unknown_tickets`).
                !(-1..=64).contains(&r) || x.abs() > Self::LIMIT || z.abs() > Self::LIMIT
            })
        {
            return None;
        }
        let mut buckets: FxHashMap<(i32, i32), Vec<u32>> = FxHashMap::default();
        let mut bucket_box = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for (i, &((x, z), _)) in extra.iter().enumerate() {
            let key = (x.div_euclid(Self::BUCKET), z.div_euclid(Self::BUCKET));
            buckets.entry(key).or_default().push(i as u32);
            bucket_box = (
                bucket_box.0.min(key.0),
                bucket_box.1.min(key.1),
                bucket_box.2.max(key.0),
                bucket_box.3.max(key.1),
            );
        }
        Some(Self {
            extra: extra.to_vec(),
            grid: CoverGrid::new(extra),
            buckets,
            bucket_box,
            radius: extra.iter().map(|&(_, r)| r).max().unwrap_or(0),
        })
    }
    /// Whether (x, z) is within `margin` of an area's square, for a
    /// `margin` up to `REACH`.
    fn within(&self, x: i32, z: i32, margin: i32) -> bool {
        if let Some(grid) = &self.grid {
            return grid.within(x, z, margin);
        }
        let reach = self.radius + margin;
        let (kx0, kx1) = (
            (x.saturating_sub(reach)).div_euclid(Self::BUCKET),
            (x.saturating_add(reach)).div_euclid(Self::BUCKET),
        );
        let (kz0, kz1) = (
            (z.saturating_sub(reach)).div_euclid(Self::BUCKET),
            (z.saturating_add(reach)).div_euclid(Self::BUCKET),
        );
        (kz0..=kz1).any(|kz| {
            (kx0..=kx1).any(|kx| {
                self.buckets.get(&(kx, kz)).is_some_and(|areas| {
                    areas.iter().any(|&i| {
                        let ((cx, cz), r) = self.extra[i as usize];
                        (i64::from(x) - i64::from(cx))
                            .abs()
                            .max((i64::from(z) - i64::from(cz)).abs())
                            <= i64::from(r + margin)
                    })
                })
            })
        })
    }
    /// The squared distance from (x, z) to the nearest area's centre.
    /// Buckets are searched ring by ring outwards: once the nearest found is
    /// no further than the next ring can be, the search stops.
    fn nearest_sq(&self, x: i32, z: i32) -> i64 {
        let d = |i: u32| {
            let (c, _) = self.extra[i as usize];
            let (dx, dz) = (i64::from(x) - i64::from(c.0), i64::from(z) - i64::from(c.1));
            dx * dx + dz * dz
        };
        let (qx, qz) = (x.div_euclid(Self::BUCKET), z.div_euclid(Self::BUCKET));
        let (x0, z0, x1, z1) = self.bucket_box;
        let last = (qx - x0)
            .abs()
            .max((qx - x1).abs())
            .max((qz - z0).abs())
            .max((qz - z1).abs());
        let mut best = i64::MAX;
        let mut looked = 0usize;
        for ring in 0..=last {
            let mut look = |kx: i32, kz: i32| {
                looked += 1;
                if let Some(areas) = self.buckets.get(&(kx, kz)) {
                    best = areas.iter().map(|&i| d(i)).fold(best, i64::min);
                }
            };
            if ring == 0 {
                look(qx, qz);
            } else {
                for k in -ring..=ring {
                    look(qx + k, qz - ring);
                    look(qx + k, qz + ring);
                }
                for k in 1 - ring..ring {
                    look(qx - ring, qz + k);
                    look(qx + ring, qz + k);
                }
            }
            // Every centre in a further ring is more than `ring` buckets away
            // on one axis.
            let bound = i64::from(ring) * i64::from(Self::BUCKET);
            if best <= bound * bound {
                break;
            }
            // Far from every area: a scan costs less than more empty rings.
            if looked > self.extra.len() * 4 {
                return (0..self.extra.len() as u32).map(d).fold(best, i64::min);
            }
        }
        best
    }
}
impl CoverGrid {
    /// None for areas too far apart for [`ExtraCover::CELLS`].
    fn new(extra: &[((i32, i32), i32)]) -> Option<Self> {
        let k = i64::from(ExtraCover::REACH);
        let (mut x0, mut z0, mut x1, mut z1) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
        for &((x, z), r) in extra {
            let (x, z, r) = (i64::from(x), i64::from(z), i64::from(r) + k);
            (x0, z0, x1, z1) = (x0.min(x - r), z0.min(z - r), x1.max(x + r), z1.max(z + r));
        }
        let (width, depth) = ((x1 - x0 + 1) as u64, (z1 - z0 + 1) as u64);
        if width * depth > ExtraCover::CELLS {
            return None;
        }
        let (x0, z0, width, depth) = (x0 as i32, z0 as i32, width as u32, depth as u32);
        let mut cells =
            vec![(ExtraCover::REACH + 1) as i8; (width * depth) as usize].into_boxed_slice();
        // Inside a square counts as 0: no margin asked for is below it. Each
        // row of a square is a run of one value with a slope at either end.
        let lower = |run: &mut [i8], v: i8| run.iter_mut().for_each(|c| *c = (*c).min(v));
        for &((cx, cz), r) in extra {
            let reach = r + ExtraCover::REACH;
            for dz in -reach..=reach {
                let start = (cz + dz - z0) as usize * width as usize + (cx - x0) as usize;
                let row = &mut cells[start - reach as usize..=start + reach as usize];
                let flat = dz.abs().max(r);
                let centre = reach as usize;
                lower(
                    &mut row[centre - flat as usize..=centre + flat as usize],
                    (flat - r) as i8,
                );
                for d in flat + 1..=reach {
                    let v = (d - r) as i8;
                    lower(&mut row[centre - d as usize..=centre - d as usize], v);
                    lower(&mut row[centre + d as usize..=centre + d as usize], v);
                }
            }
        }
        Some(Self {
            x0,
            z0,
            width,
            depth,
            cells,
        })
    }
    fn within(&self, x: i32, z: i32, margin: i32) -> bool {
        let (dx, dz) = (
            x.wrapping_sub(self.x0) as u32,
            z.wrapping_sub(self.z0) as u32,
        );
        dx < self.width
            && dz < self.depth
            && i32::from(self.cells[(dz * self.width + dx) as usize]) <= margin
    }
}
impl Ranges<'_> {
    /// `distance`, or the band's nearer priority close to it; columns needed only by
    /// non-blocking mod chunk requests always sort strictly after player, forced, and band work.
    fn priority(&self, x: i32, z: i32) -> i64 {
        let own = self.distance(x, z);
        let p = match self.band {
            Some(band) if band.within(x, z, 4) => own.min(band.priority(x, z)),
            _ => own,
        };
        if self.async_requests.is_some()
            && !self.in_main_range(x, z, 4)
            && !self.in_extra(x, z, 4)
            && !self.in_band(x, z, 4)
            && self.in_async_requests(x, z, 4)
        {
            const AFTER_BAND: i64 = 1 << 42;
            return AFTER_BAND.saturating_add(own.clamp(0, (1 << 40) - 1));
        }
        p
    }
    fn player_distance(&self, x: i32, z: i32) -> i64 {
        let d = |c: (i32, i32)| {
            let (dx, dz) = (i64::from(x - c.0), i64::from(z - c.1));
            dx * dx + dz * dz
        };
        if let Some(cover) = self.cover {
            return d(self.center).min(cover.nearest_sq(x, z));
        }
        self.extra
            .iter()
            .map(|(c, _)| d(*c))
            .fold(d(self.center), i64::min)
    }

    /// How far past the view distance chunks are generated, and how far around
    /// a finished chunk (`near_finished`): decorated chunks need their
    /// neighbours generated too.
    fn margins(decorates: bool) -> (i32, i32) {
        if decorates { (3, 2) } else { (1, 1) }
    }
    /// Chunks generation asks for: loaded from the save, or generated as protos.
    fn requested(&self, x: i32, z: i32, decorates: bool) -> bool {
        let (margin, reach) = Self::margins(decorates);
        self.in_range(x, z, margin) || self.near_finished(x, z, reach)
    }
    /// Finished columns kept when they arrive: within two of the view distance
    /// or next to a finished chunk, and every column generation asked for (a
    /// saved one loads finished; dropping it had it asked for and loaded again
    /// every frame).
    fn chunk_kept(&self, x: i32, z: i32, decorates: bool) -> bool {
        self.in_range(x, z, 2) || self.near_finished(x, z, 1) || self.requested(x, z, decorates)
    }
    /// Columns unloaded: past every column kept.
    fn unloaded(&self, x: i32, z: i32, decorates: bool) -> bool {
        !self.in_range(x, z, 3) && !self.near_finished(x, z, Self::margins(decorates).1)
    }
    /// Whether a chunk finished here (`in_range(_, _, 1)`) lies within `reach`
    /// chunks of (x, z) on both axes: its decoration needs its 3x3 neighbours
    /// decorated, which need their own 3x3 neighbours generated. The ranges are
    /// circles, so a fixed margin alone misses the corners of those squares
    /// (render distance 2 never finished the chunks next to the player).
    fn near_finished(&self, x: i32, z: i32, reach: i32) -> bool {
        let gap = |d: i32| i64::from((d.abs() - reach).max(0));
        let (dx, dz) = (gap(x - self.center.0), gap(z - self.center.1));
        let r = i64::from(self.radius + 1);
        // The extra areas are squares: their finished square grown by `reach`.
        (self.radius >= 0 && dx * dx + dz * dz <= r * r)
            || self.in_extra(x, z, 1 + reach)
            || self.in_band(x, z, 1 + reach)
            || self.in_async_requests(x, z, 1 + reach)
    }
    fn in_range(&self, x: i32, z: i32, margin: i32) -> bool {
        self.in_main_range(x, z, margin)
            || self.in_extra(x, z, margin)
            || self.in_band(x, z, margin)
            || self.in_async_requests(x, z, margin)
    }
    /// Within `margin` of the pregeneration band.
    fn in_band(&self, x: i32, z: i32, margin: i32) -> bool {
        self.band.is_some_and(|b| b.within(x, z, margin))
    }
    /// Within `margin` (Chebyshev) of an active non-blocking mod chunk request.
    fn in_async_requests(&self, x: i32, z: i32, margin: i32) -> bool {
        let Some(req) = self.async_requests else {
            return false;
        };
        if req.chunks.is_empty() || margin < 0 {
            return false;
        }
        if let Some(cover) = req.cover {
            return cover.within(x, z, margin);
        }
        req.chunks
            .iter()
            .any(|&(rx, rz)| (x - rx).abs().max((z - rz).abs()) <= margin)
    }
    /// Within `margin` of an extra area's square (Chebyshev distance, as
    /// vanilla's ticket levels spread).
    fn in_extra(&self, x: i32, z: i32, margin: i32) -> bool {
        if self.extra.is_empty() {
            return false;
        }
        if let Some(cover) = self.cover
            && (0..=ExtraCover::REACH).contains(&margin)
        {
            return cover.within(x, z, margin);
        }
        self.extra
            .iter()
            .any(|&(c, radius)| (x - c.0).abs().max((z - c.1).abs()) <= radius + margin)
    }
    /// The columns meshed for drawing: those vanilla draws around the
    /// player (`in_view_distance`), and forced ones.
    fn in_view(&self, x: i32, z: i32) -> bool {
        in_view_distance(self.center, self.radius, x, z) || self.forced.contains(&(x, z))
    }
    fn in_main_range(&self, x: i32, z: i32, margin: i32) -> bool {
        let r = i64::from(self.radius + margin);
        let (dx, dz) = (i64::from(x - self.center.0), i64::from(z - self.center.1));
        if self.radius >= 0 && dx * dx + dz * dz <= r * r {
            return true;
        }
        self.near_forced(x, z, margin)
    }
}
impl<A: AuxiliaryJobs> Drop for ChunkStreamer<A> {
    fn drop(&mut self) {
        self.abort_generation();
        // Saves in flight hold proto chunks nothing else would save.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while self.state.saving > 0 {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.shared.done.recv_timeout(left) {
                Some(Mail::Chunk(Done::Saved(_))) => self.state.saving -= 1,
                Some(_) => {}
                None => break,
            }
        }
        // `jobs` leaves the pool after this, once its running jobs are
        // done; the jobs still queued go with the queue.
    }
}
/// The queued jobs' priorities computed again from each one's column
/// `distance` (`ChunkStreamer::submit`).
fn rank_again<P>(queue: &mut BinaryHeap<Queued<P>>, distance: impl Fn(i32, i32) -> i64) {
    if queue.is_empty() {
        return;
    }
    let mut jobs = std::mem::take(queue).into_vec();
    for job in &mut jobs {
        if let Some((x, z)) = job.at {
            job.priority = distance(x, z) * 4 + job.bias;
        }
    }
    *queue = BinaryHeap::from(jobs);
}
/// `ChunkTaskPriorityQueue.release(pos, clearQueue=true)`: the jobs queued for the
/// `gone` columns are dropped, so a view that moved or shrank does not wait behind the
/// generation of the chunks it left. A job without a column (a save, meshes to free)
/// stays, as vanilla keeps the tasks of a chunk that is still loaded. Returns how many
/// jobs went.
fn release_columns<P>(
    queue: &mut BinaryHeap<Queued<P>>,
    gone: &FxHashSet<(i32, i32)>,
    mut release: impl FnMut(&Job<P>),
) -> usize {
    if queue.is_empty() || gone.is_empty() {
        return 0;
    }
    let queued = std::mem::take(queue).into_vec();
    let before = queued.len();
    let kept: Vec<Queued<P>> = queued
        .into_iter()
        .filter(|job| {
            if job.at.is_some_and(|at| gone.contains(&at)) {
                release(&job.job);
                false
            } else {
                true
            }
        })
        .collect();
    let released = before - kept.len();
    *queue = BinaryHeap::from(kept);
    released
}
/// What `keep_unchanged_sections` found changed.
#[derive(Debug, Default, PartialEq)]
struct Resent {
    /// Any section's blocks changed (in a way light or the mesher sees).
    blocks: bool,
    /// The sections whose blocks did not change but whose biomes did.
    biomes: Vec<usize>,
    /// The sections whose blocks did not change but whose light the packet changes.
    light: Vec<usize>,
}
/// `new`, a resident column `old` sent again, takes `old`'s sections whose
/// blocks it holds unchanged, with `new`'s biomes and light; the sections whose blocks
/// changed keep `old`'s light (until lit again) and their cells' classes go in
/// `cells`.
fn keep_unchanged_sections(
    old: &Chunk,
    new: &mut Chunk,
    cells: &mut crate::light::LightChanges,
) -> Resent {
    let (mut a, mut b) = (Box::new([0u32; 4096]), Box::new([0u32; 4096]));
    let mut out = Resent::default();
    let (x, z) = (new.x, new.z);
    for s in 0..new.sections.len().min(old.sections.len()) {
        // (sections stored alike hold the same blocks: most of a column sent again, found without unpacking)
        let same = old.sections[s].blocks.same_storage(&new.sections[s].blocks) || {
            old.sections[s].unpack(&mut a);
            new.sections[s].unpack(&mut b);
            let classes = crate::light::LightChanges::classes_differing(&a, &b);
            if classes != 0 {
                cells.add((x, s as i32, z), classes);
            }
            classes == 0
        };
        if !same {
            out.blocks = true;
            continue;
        }
        let biomes = new.sections[s].biomes.clone();
        let changed = !same_values(&old.sections[s].biomes, &biomes);
        let light = (!same_light(&old.sections[s].light, &new.sections[s].light))
            .then(|| new.sections[s].light.clone());
        new.sections[s] = old.sections[s].clone();
        if changed {
            Arc::make_mut(&mut new.sections[s]).biomes = biomes;
            out.biomes.push(s);
        }
        if let Some(light) = light {
            Arc::make_mut(&mut new.sections[s]).light = light;
            out.light.push(s);
        }
    }
    out
}
/// A resend keeps the client-owned models of cells that still hold the same block.
fn keep_resent_extra_models(old: &Chunk, chunk: &mut Chunk) {
    for (&old_cell, models) in &old.extra_models {
        let (x, y, z) = old.dynamic_cell_coords(old_cell);
        let (x, z) = (i32::from(x), i32::from(z));
        if let Some(cell) = chunk.dynamic_cell(x, y, z)
            && old.get(x, y, z) == chunk.get(x, y, z)
        {
            chunk.extra_models.insert(cell, models.clone());
        }
    }
}

/// Takes the server's new `chunk` over the resident column `old`: the sections
/// whose blocks did not change keep `old`'s (with `chunk`'s biomes and light),
/// and the sections whose blocks changed remesh. `relight` says the packet's
/// light is from before the server's relight (a Sunrise server's bulk edit), so
/// the column and its neighbours are lit here; another server's packet already
/// carries the light vanilla applies, so no light job is queued for it.
pub fn take_resent_column(world: &mut World, old: &Chunk, mut chunk: Chunk, relight: bool) {
    let key = (chunk.x, chunk.z);
    // The cells whose blocks changed, by section: they remesh with the neighbours within a block of them (as
    // vanilla's `setBlocksDirty`), not every section round each section changed (which, for the few blocks a
    // section often changes in a bulk edit, is mostly sections that did not).
    let mut cells = crate::light::LightChanges::default();
    cells.keyed(world.layout());
    // A pad has no mesh of its own. Only the adjacent row can be sampled by the
    // boundary sections in this column or its horizontal neighbours.
    let pads = [
        (chunk.min_y() - 1, 0),
        (chunk.max_y(), chunk.sections.len() as i32 - 1),
    ];
    for (y, s) in pads {
        if (0..16).any(|z| (0..16).any(|x| old.light(x, y, z) != chunk.light(x, y, z))) {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    world.dirty_sections.insert((key.0 + dx, s, key.1 + dz));
                }
            }
        }
    }
    keep_resent_extra_models(old, &mut chunk);
    let kept = keep_unchanged_sections(old, &mut chunk, &mut cells);
    chunk.light_loaded = old.light_loaded;
    chunk.lit = true;
    world.dirty_connection_group_diff(old, &chunk);
    world.install_light_column(chunk);
    world.revision = world.revision.wrapping_add(1);
    // A section with new biomes and the same blocks remeshes now (its tint), with its neighbours, whose tint
    // blends across the border: above and below too, as a block's tint can sample the biome quart of the
    // section next to it (`BiomeManager.getBiome`'s zoom reaches a quart beyond the block).
    world.light_updated(key.0, key.1, &kept.biomes);
    // A section with the same blocks and new light takes the packet's light (vanilla's
    // `handleLevelChunkWithLight` always applies it) and remeshes as a light update would.
    world.light_updated(key.0, key.1, &kept.light);
    if !kept.blocks {
        return;
    }
    if relight {
        for dz in -1..=1 {
            for dx in -1..=1 {
                world.dirty_light.insert((key.0 + dx, key.1 + dz));
            }
        }
    }
    cells.dirty_sections(|section| {
        world.dirty_sections.insert(section);
    });
}

/// Whether two sections' packed light (4,096 cells) holds the same values.
fn same_light(a: &crate::chunk::Light, b: &crate::chunk::Light) -> bool {
    a.same_as(b)
}
/// Whether two biome containers (64 cells) hold the same values.
fn same_values(
    a: &sunrise_core::paletted::PalettedContainer,
    b: &sunrise_core::paletted::PalettedContainer,
) -> bool {
    if a.same_storage(b) {
        return true;
    }
    let (mut x, mut y) = ([0u32; 64], [0u32; 64]);
    a.unpack(&mut x);
    b.unpack(&mut y);
    x == y
}

/// The most changed blocks one frame relights itself (`ChunkState::relight_edits`): a 16^3
/// section's worth. Past it, the columns are lit again whole on the workers.
const INLINE_RELIGHT_MAX: usize = 4096;

type LightEdit = ((i32, i32, i32), u32, u32);
/// Splits edits at `positions` into the columns to light again whole and the
/// edits to relight in place. An edited column is lit whole with its
/// neighbours (light reaches 15 blocks, never past the next column) when all
/// nine are `ready` to be lit whole; otherwise its edits stay in place.
fn whole_relight_columns(
    positions: Vec<(i32, i32, i32)>,
    ready: impl Fn(i32, i32) -> bool,
) -> (FxHashSet<(i32, i32)>, Vec<(i32, i32, i32)>) {
    let around =
        |(x, z): (i32, i32)| (-1..=1).flat_map(move |dz| (-1..=1).map(move |dx| (x + dx, z + dz)));
    let touched: FxHashSet<(i32, i32)> = positions
        .iter()
        .map(|&(x, _, z)| (x >> 4, z >> 4))
        .collect();
    let whole: FxHashSet<(i32, i32)> = touched
        .into_iter()
        .filter(|&column| around(column).all(|(x, z)| ready(x, z)))
        .collect();
    let columns = whole.iter().flat_map(|&column| around(column)).collect();
    let inline = positions
        .into_iter()
        .filter(|&(x, _, z)| !whole.contains(&(x >> 4, z >> 4)))
        .collect();
    (columns, inline)
}

/// Take save ownership before dispatch, so no generation job can mutate the view.
fn checkout_protos(
    protos: &mut FxHashMap<(i32, i32), Option<Proto>>,
    light_views: &mut FxHashMap<(i32, i32), Arc<Chunk>>,
    keys: impl IntoIterator<Item = (i32, i32)>,
) -> Vec<ProtoSave> {
    keys.into_iter()
        .filter_map(|pos| {
            let proto = protos.get_mut(&pos)?.take()?;
            light_views.remove(&pos);
            Some(ProtoSave { pos, proto })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk_fixture() -> (World, ChunkStreamer) {
        use crate::{assets::Assets, biome::Biomes, light::LightTables};
        use std::sync::OnceLock;
        use sunrise_assets::resource_source::ResourceRoot;
        use sunrise_items::blocks::BlockFacts;
        use sunrise_world::registry::BlockStateId;

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
        static CONTEXT: OnceLock<Arc<ChunkWorkerContext>> = OnceLock::new();
        let context = CONTEXT.get_or_init(|| {
            let root = std::path::PathBuf::from(
                std::env::var_os("SUNRISE_MESH_TEST_RESOURCES")
                    .or_else(|| std::env::var_os("SUNRISE_TEST_RESOURCES"))
                    .unwrap(),
            );
            let source = ResourceRoot::directory(root.clone());
            let assets = Arc::new(Assets::load(&root).unwrap());
            let facts = BlockFacts::load(&root, assets.registry).unwrap();
            Arc::new(ChunkWorkerContext {
                light: Arc::new(LightTables::load(&root, assets.registry, &facts).unwrap()),
                biomes: Arc::new(Biomes::load(&source).unwrap()),
                assets,
            })
        });
        let world = World::new(BlockStateId(0), 0);
        let mut streaming = ChunkStreamer::new(Arc::new(RemoteOnly), context.clone(), 1);
        streaming.remote = true;
        (world, streaming)
    }

    #[test]
    #[ignore = "daily"]
    fn decoration_updates_saved_column_light_through_neighbour_checkout() {
        use crate::{chunk::MIN_Y, light::light_from_saved_layers};

        let (mut world, mut streaming) = decoration_fixture();
        streaming.state.non_air_sections =
            Arc::new(|proto| proto.downcast_ref::<SectionMask>().copied());
        let neighbour = (1, 0);
        streaming.shared.done.send(Mail::Chunk(Done::Proto(
            neighbour.0,
            neighbour.1,
            Box::new(SectionMask::single(6)),
            true,
            None,
        )));
        streaming.plan_update(&mut world);

        // A decoration centred at (0, 0) leaves its already initialized east
        // neighbour with section 0 occupied instead of section 6.
        streaming.state.decorating = 1;
        streaming.state.protos.insert(neighbour, None);
        let mut region: [Proto; 9] = std::array::from_fn(|_| Box::new(SectionMask::EMPTY) as Proto);
        region[5] = Box::new(SectionMask::single(0));
        streaming.shared.done.send(Mail::Chunk(Done::Decorated(
            0,
            0,
            Box::new(region),
            Vec::new(),
            FinalWrites::default(),
        )));
        streaming.plan_update(&mut world);

        let SaveStep::Protos(checked_out) =
            streaming.prepare_save(&mut world, false, std::time::Instant::now())
        else {
            panic!("expected proto saves");
        };
        assert!(checked_out.iter().any(|save| save.pos == neighbour));
        assert_eq!(streaming.proto_sections(neighbour.0, neighbour.1), None);

        // The arriving saved column touches the neighbour, but not the centre
        // of decoration. It can only use the checked-out proto's published mask.
        let mut saved = Chunk::empty(2, 0);
        light_from_saved_layers::<&[u8]>(&mut saved, &[], true);
        streaming
            .shared
            .done
            .send(Mail::Chunk(Done::Generated(saved)));
        streaming.plan_update(&mut world);
        let column = &world.chunks[&(2, 0)];
        for b in 0..world.layout().sections() + 2 {
            assert_eq!(column.stored_light_layers(b), Some([b < 3; 2]), "layer {b}");
        }
        assert_eq!(world.light(32, MIN_Y - 1, 0), 0);
    }

    fn finish_jobs(streaming: &ChunkStreamer) -> usize {
        streaming
            .shared
            .queue
            .lock()
            .unwrap()
            .iter()
            .filter(|queued| matches!(queued.job, Job::Finish(_)))
            .count()
    }

    fn decoration_fixture() -> (World, ChunkStreamer) {
        struct Inert;
        impl Source for Inert {
            fn next(&self) -> Option<Rank> {
                None
            }

            fn run(&self, _: usize) -> bool {
                false
            }
        }

        let (world, mut streaming) = chunk_fixture();
        // Keep submitted jobs in the queue so the assertions see them.
        streaming.jobs = sunrise_threads::jobs::pool().register(Arc::new(Inert));
        (world, streaming)
    }

    /// A proto at `key` whose 3x3 is decorated, checked out for saving (its
    /// slot is `None` until the result comes back).
    fn checked_out_proto(streaming: &mut ChunkStreamer, key: (i32, i32)) {
        for dz in -1..=1 {
            for dx in -1..=1 {
                streaming.state.decorated.insert((key.0 + dx, key.1 + dz));
            }
        }
        streaming.state.protos.insert(key, None);
    }

    fn empty_commit() -> ChunkCommit {
        ChunkCommit {
            removed: Vec::new(),
            sweep_far: HashSet::new(),
            paused: false,
        }
    }

    /// The save drain applies results outside `resume_plan_update`; a proto it
    /// returns can be finalizable at once, so the next commit must not skip
    /// the finalize pass.
    #[test]
    #[ignore = "daily"]
    fn a_save_drain_wakes_the_next_finalize() {
        let (mut world, mut streaming) = decoration_fixture();
        streaming.remote = true;
        streaming.state.decorates = false;
        streaming.center = (0, 0);
        streaming.radius = 2;
        // Sync the membership revision before decorating by hand: the first
        // pass otherwise prunes keys that are not in the still-empty chunk map.
        streaming.complete_commit(&mut world, empty_commit());
        let key = (0, 0);
        checked_out_proto(&mut streaming, key);
        streaming.complete_commit(&mut world, empty_commit());
        assert_eq!(
            finish_jobs(&streaming),
            0,
            "nothing finishes while the proto is checked out"
        );

        streaming.state.saving = 1;
        streaming
            .shared
            .done
            .send(Mail::Chunk(Done::Saved(vec![(key, Box::new(()))])));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let _ = streaming.prepare_save(&mut world, true, deadline);
        streaming.complete_commit(&mut world, empty_commit());
        assert_eq!(
            finish_jobs(&streaming),
            1,
            "the returned proto is finalized on the next commit"
        );
    }

    /// A generation scan that returns at capacity does not remember the
    /// ranges it saw. An extra area that moves away and back must still wake
    /// finalize, or a proto eligible only in the original area is skipped.
    #[test]
    #[ignore = "daily"]
    fn a_range_moving_back_wakes_decoration_while_generation_is_at_capacity() {
        let (mut world, mut streaming) = decoration_fixture();
        streaming.remote = false;
        streaming.state.decorates = false;
        streaming.center = (0, 0);
        streaming.radius = 0;
        streaming.state.workers = 1; // Generation capacity: 3 in flight.
        let (area_a, area_b) = (((20, 0), 2), ((40, 0), 2));
        let key = (20, 0);
        // Sync the membership revision before decorating by hand: the first
        // pass otherwise prunes keys that are not in the still-empty chunk map.
        streaming.complete_commit(&mut world, empty_commit());
        streaming.state.generating.clear();
        checked_out_proto(&mut streaming, key);

        streaming.extra = vec![area_a];
        streaming.complete_commit(&mut world, empty_commit());
        let ranges = streaming.state.ranges();
        assert!(
            streaming.state.generation_scan.same_ranges(&ranges),
            "the first scan remembers A"
        );

        // Move to B at capacity and make the proto ready there. It is out of
        // range, and the generation scan still remembers A.
        streaming.extra = vec![area_b];
        streaming.state.generating.clear();
        streaming
            .state
            .generating
            .extend([(100, 0), (101, 0), (102, 0)]);
        streaming
            .state
            .protos
            .insert(key, Some(Box::new(()) as Proto));
        streaming.complete_commit(&mut world, empty_commit());
        assert_eq!(finish_jobs(&streaming), 0, "the ready proto is outside B");
        let ranges = streaming.state.ranges();
        assert!(
            !streaming.state.generation_scan.same_ranges(&ranges),
            "a scan at capacity leaves A remembered"
        );

        // Only the commit's own range basis can wake the pass back in A.
        streaming.extra = vec![area_a];
        let ranges = streaming.state.ranges();
        assert!(
            streaming.state.generation_scan.same_ranges(&ranges),
            "generation alone would think the ranges unchanged"
        );
        streaming.complete_commit(&mut world, empty_commit());
        assert_eq!(
            finish_jobs(&streaming),
            1,
            "the pass ran for the area that moved back"
        );
    }

    /// A finished column can leave `World::chunks` without streaming's
    /// cleanup. Its stale decorated key must not let a neighbour finish
    /// against a missing chunk.
    #[test]
    #[ignore = "daily"]
    fn a_neighbour_removed_outside_streaming_is_not_decorated() {
        let (mut world, mut streaming) = decoration_fixture();
        streaming.remote = true;
        streaming.state.decorates = false;
        streaming.center = (0, 0);
        streaming.radius = 2;
        // Sync the membership revision before decorating by hand, as in the
        // save-drain case; then the chunk map changes are replayable deltas.
        streaming.complete_commit(&mut world, empty_commit());
        let key = (0, 0);
        let neighbour = (1, 0);
        let backed_neighbour = (0, 1);
        checked_out_proto(&mut streaming, key);
        for (x, z) in [neighbour, backed_neighbour] {
            world.chunks.insert((x, z), Arc::new(Chunk::empty(x, z)));
        }
        streaming.complete_commit(&mut world, empty_commit());
        assert_eq!(finish_jobs(&streaming), 0);

        world.unload_chunk(neighbour);
        streaming.state.protos.insert(backed_neighbour, None);
        world.unload_chunk(backed_neighbour);
        streaming
            .state
            .protos
            .insert(key, Some(Box::new(()) as Proto));
        assert!(
            streaming.state.decorated.contains(&neighbour),
            "the unload left the decorated key behind"
        );
        streaming.complete_commit(&mut world, empty_commit());
        assert!(
            !streaming.state.decorated.contains(&neighbour),
            "the membership removal pruned the stale key"
        );
        assert!(
            streaming.state.decorated.contains(&backed_neighbour),
            "the tracked proto still backs the decorated key"
        );
        assert_eq!(
            finish_jobs(&streaming),
            0,
            "the missing neighbour is not decorated"
        );
    }

    /// The queue hands out the wanted chunks in the order the full sort did
    /// (every chunk measured, sorted by distance, the new ones ranked in that
    /// order, then all sorted by rank), and ranks the new ones the same way,
    /// whatever order the chunks are listed in.
    #[test]
    fn decoration_queue_pops_in_the_order_a_full_sort_gives() {
        // Distinct for every chunk, so the full sort has one answer.
        let distance = |x: i32, z: i32| i64::from(x) * 100_000 + i64::from(z);
        let full_sort = |order: &mut DecorationOrder, wanted: &[(i32, i32)]| {
            let mut w: Vec<(u64, i64, i32, i32)> = wanted
                .iter()
                .map(|&(x, z)| (0, distance(x, z), x, z))
                .collect();
            w.sort_unstable_by_key(|w| w.1);
            for w in &mut w {
                w.0 = order.rank_or_append(w.2, w.3);
            }
            w.sort_unstable();
            w.into_iter().map(|w| (w.2, w.3)).collect::<Vec<_>>()
        };
        let ranked = |order: &mut DecorationOrder| {
            order.reassign((0, 0), 6, |x, z| (x + z).rem_euclid(5) == 0);
        };
        let (mut fast, mut full) = (DecorationOrder::default(), DecorationOrder::default());
        ranked(&mut fast);
        ranked(&mut full);
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut shuffled = |mut keys: Vec<(i32, i32)>| {
            for i in (1..keys.len()).rev() {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                keys.swap(i, (seed % (i as u64 + 1)) as usize);
            }
            keys
        };
        // Some of the player's ranked chunks, and an extra area's that no
        // ticket reached; the second time, more of the area, the first part
        // ranked by then.
        let mut wanted: Vec<(i32, i32)> = (-6i32..=6)
            .flat_map(|x| (-6..=6).map(move |z| (x, z)))
            .filter(|&(x, z)| (x * 7 + z).rem_euclid(3) != 0)
            .collect();
        wanted.extend((20..=24).flat_map(|x| (-3..=3).map(move |z| (x, z))));
        for round in 0..2 {
            if round == 1 {
                wanted.extend((25..=27).flat_map(|x| (-3..=3).map(move |z| (x, z))));
            }
            let wanted = shuffled(wanted.clone());
            let mut queue = decoration_queue(&mut fast, wanted.iter().copied(), distance);
            let popped: Vec<(i32, i32)> = std::iter::from_fn(|| queue.pop())
                .map(|std::cmp::Reverse((_, x, z))| (x, z))
                .collect();
            assert_eq!(popped, full_sort(&mut full, &wanted), "round {round}");
            assert_eq!(fast.ordered(), full.ordered(), "round {round}");
        }
    }

    #[test]
    fn taking_remote_columns_keeps_their_data_until_cleanup() {
        let mut world = World::new(sunrise_world::registry::BlockStateId(0), 0);
        world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        world.chunks.insert((1, 0), Arc::new(Chunk::empty(1, 0)));
        world
            .block_entities
            .insert((0, 64, 0), crate::containers::BlockEntity::Chest(vec![]));
        let gone = [(0, 0), (2, 0)].into_iter().collect();
        let chunks = ChunkStreamer::take_remote_columns(&mut world, &gone);
        assert_eq!(chunks.len(), 1);
        assert_eq!((chunks[0].x, chunks[0].z), (0, 0));
        assert!(!world.chunks.contains_key(&(0, 0)));
        assert!(world.chunks.contains_key(&(1, 0)));
        assert!(world.block_entities.contains_key(&(0, 64, 0)));
        assert!(ChunkStreamer::take_remote_columns(&mut world, &gone).is_empty());
    }

    /// The barrier `set_forced_loaded` holds `/forceload add` to: a locally
    /// generated column is loaded once it can tick — lit, with the 3x3
    /// neighbourhood vanilla's ticking-chunk future demands (#3341).
    #[test]
    fn forced_column_is_loaded_once_it_can_tick() {
        let mut world = World::new(sunrise_world::registry::BlockStateId(0), 0);
        assert!(!world.chunks.is_ticking_ready((4, 4)));
        let mut center = Chunk::empty(4, 4);
        center.lit = true;
        world.chunks.insert((4, 4), Arc::new(center));
        assert!(!world.chunks.is_ticking_ready((4, 4)));
        for dz in -1..=1 {
            for dx in -1..=1 {
                if (dx, dz) != (0, 0) {
                    world
                        .chunks
                        .insert((4 + dx, 4 + dz), Arc::new(Chunk::empty(4 + dx, 4 + dz)));
                }
            }
        }
        assert!(world.chunks.is_ticking_ready((4, 4)));
        Arc::make_mut(world.chunks.get_mut(&(4, 4)).unwrap()).lit = false;
        assert!(!world.chunks.is_ticking_ready((4, 4)));
    }

    #[test]
    fn waiting_for_a_chunk_worker_counts_only_the_park() {
        let mailbox: Mailbox = Mailbox::default();
        mailbox.send(Mail::Chunk(Done::Saved(Vec::new())));
        let (mut done, idle) = mailbox.wait(Duration::from_secs(1), || {
            panic!("a queued result does not park")
        });
        assert_eq!(idle, Duration::ZERO);
        assert!(matches!(
            done.pop_front(),
            Some(Mail::Chunk(Done::Saved(_)))
        ));
        drop(done);

        let start = Instant::now();
        let mut times = [start, start + Duration::from_millis(7)].into_iter();
        let (done, idle) = mailbox.wait(Duration::from_nanos(1), || times.next().unwrap());
        assert!(done.is_empty());
        assert_eq!(idle, Duration::from_millis(7));
        assert!(times.next().is_none());
    }

    #[test]
    #[ignore = "daily"]
    fn late_generated_saves_yield_before_writing_in_all_receive_loops() {
        use crate::{assets::Assets, biome::Biomes, light::LightTables};
        use std::path::PathBuf;
        use sunrise_assets::resource_source::ResourceRoot;
        use sunrise_items::blocks::BlockFacts;
        use sunrise_world::registry::BlockStateId;

        #[derive(Default)]
        struct Generator(std::sync::Mutex<Vec<(i32, i32)>>);
        impl ChunkGenerator for Generator {
            type Worker = ();
            fn worker(&self) {}
            fn proto(&self, _: &mut (), _: i32, _: i32) -> Result<Proto, String> {
                Ok(Box::new(()))
            }
            fn finish(&self, _: Proto) -> Chunk {
                Chunk::empty(0, 0)
            }
            fn save_finished(&self, chunk: Chunk) {
                self.0.lock().unwrap().push((chunk.x, chunk.z));
            }
        }
        let root = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let assets = Arc::new(Assets::load(&root).unwrap());
        let facts = BlockFacts::load(&root, assets.registry).unwrap();
        let context = Arc::new(ChunkWorkerContext {
            light: Arc::new(LightTables::load(&root, assets.registry, &facts).unwrap()),
            biomes: Arc::new(Biomes::load(&ResourceRoot::directory(root.to_path_buf())).unwrap()),
            assets,
        });
        let generator = Arc::new(Generator::default());
        let mut streaming = ChunkStreamer::new(generator.clone(), context, 1);
        streaming.radius = 0;
        let mut world = World::new(BlockStateId(0), 0);
        let late = |x| {
            let mut chunk = Chunk::empty(x, 0);
            chunk.fresh = true;
            Mail::Chunk(Done::Generated(chunk))
        };
        streaming.state.finishing.extend([(100, 0), (101, 0)]);
        streaming.shared.done.send(late(100));
        streaming.shared.done.send(late(101));
        let ChunkPlanStep::Save { chunk, resume } = streaming.begin_plan_update(&mut world) else {
            panic!("first plan yield")
        };
        assert!(generator.0.lock().unwrap().is_empty());
        streaming.save_finished(chunk);
        let ChunkPlanStep::Save { chunk, resume } =
            streaming.resume_plan_update(&mut world, resume)
        else {
            panic!("second plan yield")
        };
        assert_eq!(*generator.0.lock().unwrap(), [(100, 0)]);
        streaming.save_finished(chunk);
        assert!(matches!(
            streaming.resume_plan_update(&mut world, resume),
            ChunkPlanStep::Ready(_)
        ));
        streaming.state.finishing.extend([(102, 0), (104, 0)]);
        streaming.shared.done.send(late(102));
        streaming.shared.done.send(late(104));
        let before = std::time::Instant::now();
        let poll_deadline = before + std::time::Duration::from_millis(2);
        let SaveStep::Full(chunk) =
            streaming.prepare_poll_save_with_clock(&mut world, poll_deadline, || before)
        else {
            panic!("poll yield")
        };
        assert_eq!(*generator.0.lock().unwrap(), [(100, 0), (101, 0)]);
        streaming.save_finished(chunk);
        assert!(matches!(
            streaming.prepare_poll_save_with_clock(&mut world, poll_deadline, || poll_deadline),
            SaveStep::Pending
        ));
        let next_deadline = poll_deadline + std::time::Duration::from_millis(2);
        let SaveStep::Full(chunk) =
            streaming.prepare_poll_save_with_clock(&mut world, next_deadline, || poll_deadline)
        else {
            panic!("next tick yield")
        };
        assert_eq!(*generator.0.lock().unwrap(), [(100, 0), (101, 0), (102, 0)]);
        streaming.save_finished(chunk);
        streaming.state.finishing.insert((103, 0));
        streaming.shared.done.send(late(103));
        // Startup can wait for work and then save before its next update.
        streaming.wait(std::time::Duration::ZERO);
        let deadline = std::time::Instant::now();
        let SaveStep::Full(chunk) = streaming.prepare_save(&mut world, true, deadline) else {
            panic!("blocking yield")
        };
        assert_eq!(
            *generator.0.lock().unwrap(),
            [(100, 0), (101, 0), (102, 0), (104, 0)]
        );
        streaming.save_finished(chunk);
        assert!(matches!(
            streaming.prepare_save(&mut world, true, deadline),
            SaveStep::Protos(_)
        ));
        assert_eq!(
            *generator.0.lock().unwrap(),
            [(100, 0), (101, 0), (102, 0), (104, 0), (103, 0)]
        );
    }

    #[test]
    fn checked_out_proto_is_the_read_only_state_given_to_the_save_job() {
        let mut protos = FxHashMap::default();
        protos.insert((2, -1), Some(Box::new(73_u32) as Proto));
        protos.insert((3, -1), None);
        let mut views = FxHashMap::default();
        views.insert((2, -1), Arc::new(Chunk::empty(2, -1)));
        let saves = checkout_protos(&mut protos, &mut views, [(2, -1), (3, -1), (4, -1)]);
        assert_eq!(saves.len(), 1);
        assert!(protos[&(2, -1)].is_none());
        assert!(!views.contains_key(&(2, -1)));
        let Job::Save(saved) = Job::<NoPayload>::Save(saves) else {
            unreachable!()
        };
        assert_eq!(saved[0].proto.downcast_ref::<u32>(), Some(&73));
    }

    #[test]
    fn proto_checkout_transfers_the_column_and_removes_its_light_view() {
        let mut protos = FxHashMap::default();
        protos.insert((2, -1), Some(Box::new(73_u32) as Proto));
        protos.insert((3, -1), None);
        let mut views = FxHashMap::default();
        views.insert((2, -1), Arc::new(Chunk::empty(2, -1)));
        let saves = checkout_protos(&mut protos, &mut views, [(2, -1), (3, -1), (4, -1)]);
        assert_eq!(saves.len(), 1);
        assert!(protos[&(2, -1)].is_none());
        assert!(!views.contains_key(&(2, -1)));
        assert_eq!(saves[0].pos, (2, -1));
        assert_eq!(saves[0].proto.downcast_ref::<u32>(), Some(&73));
    }

    #[test]
    fn unload_plan_keeps_dirty_chunks_live_until_the_snapshot() {
        let mut world = World::new(sunrise_world::registry::BlockStateId(0), 0);
        world.chunks.insert((10, 0), Arc::new(Chunk::empty(10, 0)));
        world.chunks.insert((11, 0), Arc::new(Chunk::empty(11, 0)));
        world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        world.modified.insert((10, 0));
        let mut plan = plan_unloads(&world, |x, _| x > 5);
        assert!(world.chunks.contains_key(&(10, 0)));
        let saved = world.chunks.get(&(10, 0)).unwrap().clone();
        let unloaded = commit_unloads(&mut world, &mut plan);
        assert_eq!((saved.x, saved.z), (10, 0));
        assert_eq!(unloaded.len(), 2);
        assert!(!world.chunks.contains_key(&(10, 0)));
        assert!(!world.chunks.contains_key(&(11, 0)));
        assert!(world.chunks.contains_key(&(0, 0)));
    }

    #[test]
    fn a_resent_column_keeps_its_sections_but_takes_new_biomes() {
        let mut old = Chunk::empty(2, -3);
        old.set(1, 10, 1, 1);
        old.set(1, 40, 1, 1);
        old.set_biome_cell(0, 0, 0, 1);
        let mut new = old.clone();
        // a new biome under the same blocks (section 0), and a block changed (section 6)
        new.set_biome_cell(0, 0, 0, 3);
        new.set(1, 40, 1, 2);
        let mut cells = crate::light::LightChanges::default();
        cells.keyed(crate::chunk::ColumnLayout::VANILLA);
        let kept = keep_unchanged_sections(&old, &mut new, &mut cells);
        assert_eq!(
            kept,
            Resent {
                blocks: true,
                biomes: vec![0],
                light: vec![],
            }
        );
        assert_eq!(new.biome(0, -64, 0), 3);
        assert_eq!(new.block(1, -54, 1).0, 0);
        assert_eq!(new.block(1, 40, 1).0, 2);
        // the sections alike are shared with the old column
        assert!(Arc::ptr_eq(&new.sections[1], &old.sections[1]));
        assert!(!Arc::ptr_eq(&new.sections[0], &old.sections[0]));
        // nothing changed: nothing to do
        let mut same = old.clone();
        let kept = keep_unchanged_sections(&old, &mut same, &mut cells);
        assert_eq!(kept, Resent::default());
        assert!(Arc::ptr_eq(&same.sections[0], &old.sections[0]));
        // the same blocks under new light: the packet's light, as vanilla applies it
        let mut lit = old.clone();
        Arc::make_mut(&mut lit.sections[2]).light = crate::chunk::Light::Uniform(0xF0);
        Arc::make_mut(&mut old.sections[2]).light = crate::chunk::Light::Uniform(0x50);
        let kept = keep_unchanged_sections(&old, &mut lit, &mut cells);
        assert_eq!(
            kept,
            Resent {
                light: vec![2],
                ..Resent::default()
            }
        );
        assert_eq!(lit.sections[2].light.get(0), 0xF0);
        assert!(Arc::ptr_eq(&lit.sections[1], &old.sections[1]));
    }

    /// The frame never waits for a worker: while one holds the mailbox, the
    /// frame takes nothing and finds the results, in order, the next time.
    #[test]
    fn the_frame_takes_finished_jobs_without_waiting() {
        let mailbox: Mailbox = Mailbox::default();
        let lit = |seq| Mail::Chunk(Done::Lit(0, 0, Vec::new(), seq));
        mailbox.send(lit(1));
        mailbox.send(lit(2));
        let sending = mailbox.done.lock().unwrap();
        assert!(mailbox.take().is_empty());
        assert!(mailbox.try_recv().is_none());
        drop(sending);
        let seqs: Vec<u64> = mailbox
            .take()
            .into_iter()
            .map(|done| match done {
                Mail::Chunk(Done::Lit(_, _, _, seq)) => seq,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(seqs, [1, 2]);
        assert!(mailbox.take().is_empty());
        let waiting = std::time::Instant::now();
        assert!(
            mailbox
                .recv_timeout(std::time::Duration::from_millis(10))
                .is_none()
        );
        assert!(waiting.elapsed() >= std::time::Duration::from_millis(10));
    }

    #[test]
    #[ignore = "daily"]
    fn redirtying_a_batch_keeps_its_worker_slots_until_the_results_are_consumed() {
        use std::time::{Duration, Instant};

        let (mut world, mut streaming) = chunk_fixture();
        for z in -1..=1 {
            for x in -1..=5 {
                world.chunks.insert((x, z), Arc::new(Chunk::empty(x, z)));
            }
        }
        let columns = (0..5).map(|x| (x, 0));
        world.dirty_light.extend(columns.clone());
        streaming.update_light(&mut world);
        assert_eq!(streaming.order, 4, "initial one-worker batch");

        let shared = Arc::clone(&streaming.shared);
        let mailbox = shared.done.done.lock().unwrap();
        for _ in 0..3 {
            world.dirty_light.extend(columns.clone());
            streaming.update_light(&mut world);
            assert_eq!(
                streaming.order, 4,
                "no new snapshots before completions are taken"
            );
            assert_eq!(world.dirty_light.len(), 5, "one replacement per column");
        }
        drop(mailbox);

        let deadline = Instant::now() + Duration::from_secs(10);
        streaming.wait(Duration::from_millis(50));
        streaming.update_light(&mut world);
        assert!(
            columns.clone().all(|(x, z)| !streaming.is_lit(x, z)),
            "stale light is discarded"
        );
        while columns.clone().any(|(x, z)| !streaming.is_lit(x, z)) {
            assert!(Instant::now() < deadline, "replacement jobs did not finish");
            streaming.wait(Duration::from_millis(50));
            streaming.update_light(&mut world);
        }
        assert!(world.dirty_light.is_empty());
        assert_eq!(
            streaming.order, 9,
            "four superseded jobs and five current jobs"
        );
    }
    #[test]
    #[ignore = "daily"]
    fn resend_marker_is_released_by_its_covering_job_while_later_requests_pile_up() {
        use std::time::{Duration, Instant};

        let (mut world, mut streaming) = chunk_fixture();
        for z in -1..=1 {
            for x in -1..=1 {
                world.chunks.insert((x, z), Arc::new(Chunk::empty(x, z)));
            }
        }
        world.dirty_light.insert((0, 0));
        streaming.queue_chunk_resend(&mut world, (0, 0));
        assert!(matches!(
            world.chunk_resends.get(&(0, 0)),
            Some(&ChunkResend::WaitingLight { .. })
        ));
        streaming.update_light(&mut world);
        assert_eq!(streaming.order, 1);
        assert!(streaming.has_column_light_job((0, 0)));

        // Re-dirty while the covering job is still in flight: a relight added after the resend.
        let shared = Arc::clone(&streaming.shared);
        let mailbox = shared.done.done.lock().unwrap();
        world.dirty_light.insert((0, 0));
        streaming.update_light(&mut world);
        assert_eq!(streaming.order, 1);
        assert!(matches!(
            world.chunk_resends.get(&(0, 0)),
            Some(&ChunkResend::WaitingLight { .. })
        ));
        drop(mailbox);

        // The covering job's light is published and the resend released, though the newer
        // request has not run: later requests never extend the wait.
        let deadline = Instant::now() + Duration::from_secs(10);
        while streaming.order < 2 {
            assert!(Instant::now() < deadline, "covering job did not return");
            streaming.wait(Duration::from_millis(50));
            streaming.update_light(&mut world);
        }
        assert!(streaming.is_lit(0, 0), "the covered light was published");
        assert_eq!(world.chunk_resends.get(&(0, 0)), Some(&ChunkResend::Ready));

        while streaming.has_column_light_job((0, 0)) || world.dirty_light.contains(&(0, 0)) {
            assert!(Instant::now() < deadline, "replacement job did not finish");
            streaming.wait(Duration::from_millis(50));
            streaming.update_light(&mut world);
        }
        assert!(streaming.is_lit(0, 0));
        assert_eq!(world.chunk_resends.get(&(0, 0)), Some(&ChunkResend::Ready));

        // A fresh light check queued after completion does not move the ready marker.
        world.light_edits.push(((8, 64, 8), 0, 0));
        assert_eq!(world.chunk_resends.get(&(0, 0)), Some(&ChunkResend::Ready));

        // A resend queued behind an existing light check waits for one `relight_edits` pass,
        // and is unaffected by a later check queued after that pass.
        world.chunk_resends.clear();
        streaming.queue_chunk_resend(&mut world, (0, 0));
        assert_eq!(
            world.chunk_resends.get(&(0, 0)),
            Some(&ChunkResend::WaitingEdits { through: 1 })
        );
        streaming.update_light(&mut world);
        assert!(world.light_edits.is_empty());
        assert_eq!(world.chunk_resends.get(&(0, 0)), Some(&ChunkResend::Ready));
        world.light_edits.push(((8, 64, 8), 0, 0));
        assert_eq!(world.chunk_resends.get(&(0, 0)), Some(&ChunkResend::Ready));
    }
    /// Edits in two neighbouring columns light those and the ten around
    /// them again; a column next to one that can't be lit whole keeps its
    /// edits in the frame.
    #[test]
    fn whole_relights_cover_the_edited_columns_and_their_neighbours() {
        // Blocks in columns (0, 0) and (1, 0).
        let positions = vec![(3, 70, 4), (15, 64, 15), (16, 80, 0), (31, 64, 9)];
        let (columns, inline) = whole_relight_columns(positions.clone(), |_, _| true);
        let mut columns: Vec<_> = columns.into_iter().collect();
        columns.sort();
        let expected: Vec<(i32, i32)> = (-1..=2)
            .flat_map(|x| (-1..=1).map(move |z| (x, z)))
            .collect();
        assert_eq!(columns, expected);
        assert!(inline.is_empty());
        // (2, 1) is next to (1, 0) only: (0, 0) is still lit whole.
        let (columns, inline) = whole_relight_columns(positions, |x, z| (x, z) != (2, 1));
        assert_eq!(inline, vec![(16, 80, 0), (31, 64, 9)]);
        assert_eq!(columns.len(), 9);
        assert!(columns.contains(&(-1, -1)) && columns.contains(&(1, 1)));
        // Negative coordinates floor to their column.
        let (west, _) = whole_relight_columns(vec![(-1, 64, -17)], |_, _| true);
        assert!(west.contains(&(-2, -3)) && west.contains(&(0, -1)) && west.len() == 9);
    }

    #[test]
    fn queued_jobs_follow_the_centre() {
        let forced = HashSet::new();
        let ranges = |center| Ranges {
            center,
            radius: 32,
            extra: &[],
            forced: &forced,
            cover: None,
            forced_cover: None,
            band: None,
            async_requests: None,
        };
        let generate = |x, z, priority, order| -> Queued {
            Queued {
                priority,
                at: Some((x, z)),
                bias: 0,
                order,
                job: Job::Generate(x, z),
            }
        };
        // Both queued around an earlier centre, where the farther job came first.
        let earlier = ranges((40, 0));
        let current = ranges((0, 0));
        let queued = || {
            BinaryHeap::from(vec![
                generate(3, 1, earlier.priority(3, 1) * 4, 1),
                generate(20, 0, earlier.priority(20, 0) * 4, 2),
                Queued {
                    priority: i64::MIN,
                    at: None,
                    bias: 0,
                    order: 3,
                    job: Job::Save(Vec::new()),
                },
            ])
        };
        let pops = |mut queue: BinaryHeap<Queued>| {
            std::iter::from_fn(move || queue.pop())
                .map(|q| q.at)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            pops(queued()),
            [None, Some((20, 0)), Some((3, 1))],
            "the stale order"
        );
        let mut queue = queued();
        rank_again(&mut queue, |x, z| current.priority(x, z));
        assert_eq!(pops(queue), [None, Some((3, 1)), Some((20, 0))]);
        // The player moved: the job near the new centre goes first.
        let mut queue = BinaryHeap::from(vec![generate(3, 1, 0, 1), generate(20, 0, 1, 2)]);
        let moved = ranges((19, 0));
        rank_again(&mut queue, |x, z| moved.priority(x, z));
        assert_eq!(queue.pop().and_then(|q| q.at), Some((20, 0)));
    }

    /// `ChunkTaskPriorityQueue.release`: a column that unloaded keeps no queued job, so
    /// a teleport or a smaller render distance does not spend its workers on the world
    /// it left; the other columns' jobs and the jobs without a column (saves, meshes to
    /// free) stay, in the same order.
    #[test]
    fn a_released_column_keeps_no_queued_job() {
        let generate = |x, z, priority, order| -> Queued {
            Queued {
                priority,
                at: Some((x, z)),
                bias: 0,
                order,
                job: Job::Generate(x, z),
            }
        };
        let save = |order| -> Queued {
            Queued {
                priority: i64::MIN,
                at: None,
                bias: 0,
                order,
                job: Job::Save(Vec::new()),
            }
        };
        let queued = || {
            BinaryHeap::from(vec![
                generate(0, 0, 0, 1),
                generate(0, 0, 4, 2),
                generate(1, 0, 8, 3),
                generate(9, 9, 12, 4),
                save(5),
            ])
        };
        let pops = |mut queue: BinaryHeap<Queued>| {
            std::iter::from_fn(move || queue.pop())
                .map(|q| q.at)
                .collect::<Vec<_>>()
        };
        // Nothing goes for a column that is still loaded, or for nothing at all.
        let mut queue = queued();
        assert_eq!(
            release_columns(&mut queue, &FxHashSet::default(), |_| {}),
            0
        );
        assert_eq!(
            release_columns(&mut queue, &[(20, 20)].into_iter().collect(), |_| {}),
            0
        );
        assert_eq!(queue.len(), 5);
        // Both of (0, 0)'s jobs go, its neighbour's and the save stay.
        let gone: FxHashSet<(i32, i32)> = [(0, 0)].into_iter().collect();
        assert_eq!(release_columns(&mut queue, &gone, |_| {}), 2);
        assert_eq!(pops(queue), [None, Some((1, 0)), Some((9, 9))]);
        // Every column leaving at once: only the jobs without one are left.
        let mut queue = queued();
        let all: FxHashSet<(i32, i32)> = [(0, 0), (1, 0), (9, 9)].into_iter().collect();
        assert_eq!(release_columns(&mut queue, &all, |_| {}), 4);
        assert_eq!(pops(queue), [None]);
    }

    /// What the scheduler counts as ready to light: every neighbour a
    /// finished column, or a decorated proto chunk past the area — the
    /// walk `light_region` makes without its snapshot.
    #[test]
    fn light_readiness_follows_the_region_a_light_job_reads() {
        let mut world = World::new(sunrise_world::registry::BlockStateId(0), 0);
        let chunk = Arc::new(Chunk::empty(0, 0));
        let nine = (-1..=1).flat_map(|dz| (-1..=1).map(move |dx| (dx, dz)));
        for key in nine {
            world.chunks.insert(key, chunk.clone());
        }
        // The kept area is the cross around (0, 0); the corners of the
        // 3x3 light reads lie past it.
        let in_range = |x: i32, z: i32| x.abs() + z.abs() <= 1;
        let none = FxHashSet::<(i32, i32)>::default();
        let empty = FxHashMap::<(i32, i32), Option<Proto>>::default();
        assert!(light_region_ready(&world, in_range, &none, &empty, 0, 0));
        // A corner is missing, and nothing stands in for it yet.
        world.chunks.remove(&(1, 1));
        assert!(!light_region_ready(&world, in_range, &none, &empty, 0, 0));
        // A decorated, checked-in proto chunk past the area lights against.
        let decorated: FxHashSet<(i32, i32)> = [(1, 1)].into_iter().collect();
        let mut protos: FxHashMap<(i32, i32), Option<Proto>> = Default::default();
        protos.insert((1, 1), Some(Box::new(()) as Proto));
        assert!(light_region_ready(
            &world, in_range, &decorated, &protos, 0, 0
        ));
        // The proto is checked out to a job: it cannot be read.
        protos.insert((1, 1), None);
        assert!(!light_region_ready(
            &world, in_range, &decorated, &protos, 0, 0
        ));
        // A neighbour inside the area waits for the chunk itself.
        world.chunks.remove(&(1, 0));
        let mut inside = decorated.clone();
        inside.insert((1, 0));
        protos.insert((1, 0), Some(Box::new(()) as Proto));
        assert!(!light_region_ready(
            &world, in_range, &inside, &protos, 0, 0
        ));
    }

    #[test]
    fn view_distance_is_vanillas_shape() {
        // ChunkTrackingView.isInViewDistance at render distance 12: a column
        // of slack on each axis, so 533 columns rather than the 441 of a
        // circle of radius 12.
        let count = |r: i32| {
            (-40..=40)
                .flat_map(|x| (-40..=40).map(move |z| (x, z)))
                .filter(|&(x, z)| in_view_distance((5, -7), r, x + 5, z - 7))
                .count()
        };
        assert_eq!(count(12), 533);
        assert_eq!(count(2), 25);
        for (x, z, inside) in [(12, 0, true), (13, 0, false), (9, 9, true), (10, 9, false)] {
            assert_eq!(in_view_distance((0, 0), 12, x, z), inside, "({x}, {z})");
            assert_eq!(
                in_view_distance((0, 0), 12, -z, -x),
                inside,
                "({}, {})",
                -z,
                -x
            );
        }
    }

    /// A view has arrived once the columns at both ends of each of its rows and
    /// columns are in: exactly those with a neighbour outside it, at every radius.
    /// Then it stays arrived until it changes; a view is `recent`, and waited for,
    /// `VIEW_WAIT` at most, and one of no radius not at all.
    #[test]
    fn a_view_has_arrived_once_its_outermost_columns_are_in() {
        let start = std::time::Instant::now();
        let center = (3, -4);
        for r in 0..=16 {
            let view: FxHashSet<(i32, i32)> = (-r - 1..=r + 1)
                .flat_map(|x| (-r - 1..=r + 1).map(move |z| (center.0 + x, center.1 + z)))
                .filter(|&(x, z)| in_view_distance(center, r, x, z))
                .collect();
            let arriving = |has: &dyn Fn((i32, i32)) -> bool| {
                ViewArrival::default().arriving((center, r), start, has)
            };
            assert!(!arriving(&|c| view.contains(&c)), "radius {r}");
            for &(x, z) in &view {
                let edge = [(x + 1, z), (x - 1, z), (x, z + 1), (x, z - 1)]
                    .iter()
                    .any(|c| !view.contains(c));
                let missing = arriving(&|c| c != (x, z) && view.contains(&c));
                assert_eq!(missing, edge, "radius {r}, column ({x}, {z})");
            }
        }
        let mut arrival = ViewArrival::default();
        let (none, all) = (|_| false, |_| true);
        assert!(arrival.arriving(((0, 0), 2), start, none));
        assert!(!arrival.arriving(((0, 0), 2), start, all));
        assert!(!arrival.arriving(((0, 0), 2), start, none));
        // Moved: waited for again, until `VIEW_WAIT` after the move.
        let (late, ms) = (start + VIEW_WAIT, std::time::Duration::from_millis(1));
        assert!(arrival.arriving(((1, 0), 2), start, none));
        assert!(arrival.arriving(((1, 0), 2), late - ms, none));
        assert!(arrival.recent(late - ms) && !arrival.recent(late));
        assert!(!arrival.arriving(((1, 0), 2), late, none));
        assert!(!arrival.arriving(((1, 0), -1), start, none));
    }

    struct GenerationFixture {
        world: World,
        center: (i32, i32),
        radius: i32,
        extra: Vec<((i32, i32), i32)>,
        forced: HashSet<(i32, i32)>,
        generating: FxHashSet<(i32, i32)>,
        failed: FxHashSet<(i32, i32)>,
        protos: FxHashMap<(i32, i32), Option<Proto>>,
        finishing: FxHashSet<(i32, i32)>,
        decorates: bool,
        remote: bool,
    }
    impl GenerationFixture {
        fn new(radius: i32, decorates: bool) -> Self {
            Self {
                world: World::new(sunrise_world::registry::BlockStateId(0), 0),
                center: (0, 0),
                radius,
                extra: Vec::new(),
                forced: HashSet::new(),
                generating: Default::default(),
                failed: Default::default(),
                protos: Default::default(),
                finishing: Default::default(),
                decorates,
                remote: false,
            }
        }
        fn input(&self) -> GenerationInputs<'_> {
            GenerationInputs {
                ranges: Ranges {
                    center: self.center,
                    radius: self.radius,
                    extra: &self.extra,
                    forced: &self.forced,
                    cover: None,
                    forced_cover: None,
                    band: None,
                    async_requests: None,
                },
                world: &self.world,
                generating: &self.generating,
                failed: &self.failed,
                protos: &self.protos,
                finishing: &self.finishing,
                decorates: self.decorates,
                remote: self.remote,
            }
        }
        fn fill(&mut self) {
            // Scheduler only reads membership. Share one real empty column to
            // keep a radius-32 fixture small; chunk content is never consulted.
            let chunk = Arc::new(Chunk::empty(0, 0));
            for (_, x, z) in baseline_candidates(self.input(), usize::MAX) {
                self.world.chunks.insert((x, z), chunk.clone());
            }
        }
        fn fill_steady(&mut self) {
            let chunk = Arc::new(Chunk::empty(0, 0));
            for (_, x, z) in baseline_candidates(self.input(), usize::MAX) {
                if self.input().ranges.in_range(x, z, 1) {
                    self.world.chunks.insert((x, z), chunk.clone());
                } else {
                    // Generation's outer neighbour margin remains proto;
                    // only columns in the finish range become resident.
                    self.protos.insert((x, z), Some(Box::new(())));
                }
            }
        }
        fn assert_same(&self, gate: &mut GenerationScan, capacity: usize) -> bool {
            let expected = baseline_candidates(self.input(), capacity);
            let result = gate.candidates(self.input(), capacity);
            assert_eq!(result.as_deref().unwrap_or(&[]), expected);
            result.is_some()
        }
    }

    // Original scheduling scan, without snapshot bookkeeping, for differential
    // correctness and timing. Keep this independent of the production gate.
    fn baseline_candidates(input: GenerationInputs<'_>, capacity: usize) -> Vec<(i64, i32, i32)> {
        if input.remote || input.generating.len() >= capacity {
            return Vec::new();
        }
        let margin: i32 = if input.decorates { 3 } else { 1 };
        let reach = if input.decorates { 2 } else { 1 };
        let mut wanted = Vec::new();
        let areas: Vec<_> = std::iter::once((input.ranges.center, input.ranges.radius))
            .chain(input.ranges.extra.iter().copied())
            .collect();
        let missing = |x, z| {
            !input.world.chunks.contains_key(&(x, z))
                && !input.generating.contains(&(x, z))
                && !input.failed.contains(&(x, z))
                && !input.protos.contains_key(&(x, z))
                && !input.finishing.contains(&(x, z))
        };
        for (center, radius) in areas {
            let r = radius + margin.max(reach + 1);
            for dz in -r..=r {
                for dx in -r..=r {
                    let (x, z) = (center.0 + dx, center.1 + dz);
                    if (input.ranges.in_range(x, z, margin)
                        || input.ranges.near_finished(x, z, reach))
                        && missing(x, z)
                    {
                        wanted.push((input.ranges.distance(x, z), x, z));
                    }
                }
            }
        }
        let forced_reach = if input.decorates {
            FORCED_TERRAIN_RING
        } else {
            FORCED_FEATURES_RING
        };
        for &(fx, fz) in input.ranges.forced {
            for dz in -forced_reach..=forced_reach {
                for dx in -forced_reach..=forced_reach {
                    let (x, z) = (fx + dx, fz + dz);
                    if missing(x, z) {
                        wanted.push((input.ranges.distance(x, z), x, z));
                    }
                }
            }
        }
        wanted.sort_unstable();
        wanted.dedup();
        wanted.truncate(capacity - input.generating.len());
        wanted
    }

    #[test]
    fn generation_gate_tracks_public_ranges_and_exact_membership() {
        for decorates in [false, true] {
            let mut f = GenerationFixture::new(2, decorates);
            f.fill();
            let mut gate = GenerationScan::default();
            assert!(f.assert_same(&mut gate, 6));
            assert!(!f.assert_same(&mut gate, 6));
            // Light/mesh state and block content do not affect generation.
            f.world.dirty_light.insert((0, 0));
            f.world.dirty_sections.insert((0, 0, 0));
            Arc::make_mut(f.world.chunks.get_mut(&(0, 0)).unwrap()).lit = true;
            assert!(!f.assert_same(&mut gate, 6));
            // Same-count removal/replacement must not leave a hole unnoticed.
            let chunk = f.world.chunks.remove(&(0, 0)).unwrap();
            f.world.chunks.insert((100, 100), chunk.clone());
            assert!(f.assert_same(&mut gate, 6));
            f.world.chunks.remove(&(100, 100));
            f.world.chunks.insert((0, 0), chunk);
            assert!(f.assert_same(&mut gate, 6));
            assert!(!f.assert_same(&mut gate, 6));
            f.center = (1, -1);
            assert!(f.assert_same(&mut gate, 6));
            f.fill();
            f.assert_same(&mut gate, 6);
            f.radius += 1;
            assert!(f.assert_same(&mut gate, 6));
            f.fill();
            f.assert_same(&mut gate, 6);
            f.extra.push(((20, -20), 2));
            assert!(f.assert_same(&mut gate, 6));
            f.fill();
            f.assert_same(&mut gate, 6);
            f.forced.insert((40, 40));
            assert!(f.assert_same(&mut gate, 6));
            f.fill();
            f.assert_same(&mut gate, 6);
            f.extra.clear();
            f.forced.clear();
            assert!(f.assert_same(&mut gate, 6));
            f.remote = true;
            assert!(!f.assert_same(&mut gate, 6));
            f.world.chunks.clear();
            f.remote = false;
            assert!(f.assert_same(&mut gate, 6));
            f.fill();
            f.assert_same(&mut gate, 6);
            f.world = World::new(sunrise_world::registry::BlockStateId(0), 1);
            assert!(f.assert_same(&mut gate, 6));
        }
    }

    #[test]
    fn generation_gate_preserves_batches_stage_transitions_and_retries() {
        for decorates in [false, true] {
            let mut f = GenerationFixture::new(2, decorates);
            let mut gate = GenerationScan::default();
            let mut scheduled = FxHashSet::default();
            loop {
                let expected = baseline_candidates(f.input(), 3);
                let batch = gate.candidates(f.input(), 3).unwrap_or_default();
                assert_eq!(batch, expected);
                if batch.is_empty() {
                    break;
                }
                for (_, x, z) in batch {
                    assert!(scheduled.insert((x, z)), "duplicate scheduling");
                    f.generating.insert((x, z));
                }
                if f.generating.len() == 3 {
                    assert!(!f.assert_same(&mut gate, 3)); // capacity pressure
                } else {
                    f.assert_same(&mut gate, 3);
                }
                for key in std::mem::take(&mut f.generating) {
                    // load miss -> proto -> checked out decoration -> finish.
                    f.protos.insert(key, None);
                }
                gate.exhausted = false; // Done::Proto/Decorated invalidation
            }
            assert!(!scheduled.is_empty());
            assert!(!f.assert_same(&mut gate, 3));
            let key = (0, 0);
            f.protos.remove(&key);
            f.finishing.insert(key);
            assert!(!f.assert_same(&mut gate, 3)); // eligibility unchanged
            f.finishing.remove(&key);
            f.world.chunks.insert(key, Arc::new(Chunk::empty(0, 0)));
            gate.exhausted = false; // Done::Generated
            assert!(f.assert_same(&mut gate, 3));
            assert!(!f.assert_same(&mut gate, 3));
            // A discarded/pruned proto must be requested again, even without
            // any range or loaded-membership change.
            let key = scheduled.iter().copied().find(|k| *k != (0, 0)).unwrap();
            f.protos.remove(&key);
            gate.exhausted = false;
            assert!(f.assert_same(&mut gate, 3));
            assert_eq!(baseline_candidates(f.input(), 3).len(), 1);
        }
    }

    /// Batches taken from the plan are the full scan's, as jobs finish,
    /// columns and proto chunks leave and the centre moves.
    #[test]
    fn generation_plan_matches_full_scans_as_columns_arrive() {
        for decorates in [false, true] {
            let mut f = GenerationFixture::new(6, decorates);
            let mut gate = GenerationScan::default();
            let chunk = Arc::new(Chunk::empty(0, 0));
            let mut seed = 0x2545_F491_4F6C_DD1Du64;
            let mut rand = |n: u64| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed % n
            };
            let mut scheduled = 0;
            for step in 0..600 {
                let capacity = 3 + rand(6) as usize;
                let expected = baseline_candidates(f.input(), capacity);
                let batch = gate.candidates(f.input(), capacity).unwrap_or_default();
                assert_eq!(batch, expected, "step {step}");
                for (_, x, z) in batch {
                    f.generating.insert((x, z));
                    scheduled += 1;
                }
                // Some jobs finish: as a proto chunk, or a final column.
                let mut done: Vec<_> = f.generating.iter().copied().collect();
                done.sort_unstable();
                for key in done {
                    if rand(3) != 0 {
                        continue;
                    }
                    f.generating.remove(&key);
                    if rand(2) == 0 {
                        f.protos.insert(key, None);
                    } else {
                        f.world.chunks.insert(key, chunk.clone());
                    }
                    gate.exhausted = false;
                }
                // Now and then a column or a proto chunk leaves.
                if rand(20) == 0 {
                    let mut keys: Vec<_> = f.world.chunks.keys().copied().collect();
                    keys.sort_unstable();
                    if let Some(key) = keys.get(rand(keys.len().max(1) as u64) as usize) {
                        f.world.chunks.remove(key);
                    }
                }
                if rand(20) == 0 {
                    let mut keys: Vec<_> = f.protos.keys().copied().collect();
                    keys.sort_unstable();
                    if let Some(key) = keys.get(rand(keys.len().max(1) as u64) as usize) {
                        f.protos.remove(key);
                        gate.invalidate();
                    }
                }
                if step == 300 {
                    f.center = (3, -2);
                }
            }
            assert!(scheduled > 200);
        }
    }

    #[test]
    fn generation_scan_looks_at_overlapping_areas_once() {
        // A server's players: many areas around one spot, some apart, one
        // touching another only at a corner, and part of the land present.
        for decorates in [false, true] {
            let mut f = GenerationFixture::new(10, decorates);
            for i in 0..40 {
                f.extra.push(((i % 7 - 3, i / 7 - 3), 19));
            }
            f.extra.push(((200, -150), 19));
            f.extra.push(((243, -107), 19));
            f.extra.push(((-60, 5), 4));
            f.forced.insert((90, 90));
            let chunk = Arc::new(Chunk::empty(0, 0));
            for x in -40i32..40 {
                for z in -40i32..40 {
                    match (x * 7 + z * 13).rem_euclid(5) {
                        0 => drop(f.world.chunks.insert((x, z), chunk.clone())),
                        1 => drop(f.protos.insert((x, z), None)),
                        2 if x % 3 == 0 => drop(f.generating.insert((x, z))),
                        _ => {}
                    }
                }
            }
            assert!(baseline_candidates(f.input(), usize::MAX).len() > 1000);
            for capacity in [usize::MAX, f.generating.len() + 7] {
                let mut gate = GenerationScan::default();
                assert!(f.assert_same(&mut gate, capacity));
            }
        }
    }

    #[test]
    #[ignore = "reproducible actual generation scheduler benchmark; run with --ignored --nocapture"]
    fn generation_scan_benchmark() {
        use std::{hint::black_box, time::Instant};
        let mut f = GenerationFixture::new(32, true);
        f.fill_steady();
        let mut gate = GenerationScan::default();
        f.assert_same(&mut gate, 12);
        assert!(!f.assert_same(&mut gate, 12));
        const FRAMES: usize = 2000;
        let measure = |optimized: bool, dynamic: bool| {
            let mut samples = Vec::new();
            let mut scans = 0;
            let mut jobs = 0;
            for repetition in 0..9 {
                let mut fixture = GenerationFixture::new(32, true);
                fixture.fill_steady();
                let saved = fixture.world.chunks.get(&(0, 0)).unwrap().clone();
                let mut gate = GenerationScan::default();
                gate.candidates(fixture.input(), 12);
                let start = Instant::now();
                let mut count = 0;
                let mut scheduled = 0;
                for frame in 0..FRAMES {
                    // Remove a column, queue generation, then accept a saved
                    // load completion. Both schedulers see identical events.
                    if dynamic && frame % 32 == 0 {
                        fixture.world.chunks.remove(&(0, 0));
                    }
                    if dynamic && frame % 32 == 1 {
                        fixture.generating.remove(&(0, 0));
                        fixture.world.chunks.insert((0, 0), saved.clone());
                        gate.exhausted = false; // Done::Generated
                    }
                    let result = if optimized {
                        let result = gate.candidates(black_box(fixture.input()), 12);
                        count += usize::from(result.is_some());
                        result.unwrap_or_default()
                    } else {
                        count += 1;
                        baseline_candidates(black_box(fixture.input()), 12)
                    };
                    let expected_jobs = usize::from(dynamic && frame % 32 == 0);
                    assert_eq!(result.len(), expected_jobs);
                    for (_, x, z) in black_box(result) {
                        assert_eq!((x, z), (0, 0));
                        fixture.generating.insert((x, z));
                        scheduled += 1;
                    }
                }
                if repetition != 0 {
                    samples.push(start.elapsed().as_secs_f64() * 1e9 / FRAMES as f64);
                }
                assert!(fixture.generating.is_empty());
                assert_eq!(fixture.world.chunks.len(), f.world.chunks.len());
                scans = count;
                jobs = scheduled;
            }
            samples.sort_by(f64::total_cmp);
            (samples[samples.len() / 2], scans, jobs)
        };
        println!(
            "radius=32 decorates=true positions/frame=5041 loaded_keys={} proto_keys={} frames/sample={FRAMES} warmup=1 measured=8",
            f.world.chunks.len(),
            f.protos.len()
        );
        for dynamic in [false, true] {
            let (before, baseline_scans, before_jobs) = measure(false, dynamic);
            let (after, optimized_scans, after_jobs) = measure(true, dynamic);
            assert_eq!(before_jobs, after_jobs);
            println!(
                "dynamic={dynamic}: baseline={before:.1}ns/frame optimized={after:.1}ns/frame speedup={:.2}x scans={baseline_scans}/{optimized_scans} jobs={before_jobs}/{after_jobs}",
                before / after
            );
        }
    }

    /// The ranges `ChunkStreamer::update` checks at each step, from generation to
    /// light, must let every chunk the player spawns on finish and light: the
    /// 3x3 columns around the player need their own 3x3 neighbours finished,
    /// each of those its 3x3 neighbours decorated, and each of those its 3x3
    /// neighbours generated and kept. Render distance 2 once stopped short.
    #[test]
    fn spawn_neighbourhood_completes_at_every_distance() {
        let forced = HashSet::new();
        for decorates in [true, false] {
            let (margin, reach) = Ranges::margins(decorates);
            for radius in 2..=32 {
                let r = Ranges {
                    center: (5, -7),
                    radius,
                    extra: &[],
                    forced: &forced,
                    cover: None,
                    forced_cover: None,
                    band: None,
                    async_requests: None,
                };
                let square = |x: i32, z: i32, n: i32| {
                    (-n..=n).flat_map(move |dz| (-n..=n).map(move |dx| (x + dx, z + dz)))
                };
                let generated = |x, z| {
                    r.requested(x, z, decorates)
                        && (x - r.center.0).abs() <= radius + margin.max(reach + 1)
                        && (z - r.center.1).abs() <= radius + margin.max(reach + 1)
                };
                let proto_kept = |x, z| r.in_range(x, z, 4) || r.near_finished(x, z, 2);
                let decorated =
                    |x, z| !decorates || r.in_range(x, z, 2) || r.near_finished(x, z, 1);
                let chunk_kept = |x, z| r.chunk_kept(x, z, decorates);
                let unloaded = |x, z| r.unloaded(x, z, decorates);
                let finishes = |x: i32, z: i32| {
                    r.in_range(x, z, 1)
                        && chunk_kept(x, z)
                        && !unloaded(x, z)
                        && square(x, z, 1).all(|(nx, nz)| {
                            decorated(nx, nz)
                                && (!decorates
                                    || square(nx, nz, 1)
                                        .all(|(gx, gz)| generated(gx, gz) && proto_kept(gx, gz)))
                                && generated(nx, nz)
                                && proto_kept(nx, nz)
                        })
                };
                let (cx, cz) = r.center;
                for (x, z) in square(cx, cz, 1) {
                    for (nx, nz) in square(x, z, 1) {
                        assert!(
                            finishes(nx, nz),
                            "radius {radius} (decorates: {decorates}): ({nx}, {nz}) never finishes"
                        );
                    }
                }
            }
        }
    }

    /// A column generation asks for is kept when it arrives finished (loaded
    /// from the save) and is not unloaded right after: either way it would be
    /// asked for and loaded again on the next frame, forever (8 such columns
    /// after a teleport at render distance 16 held 8 of the in-flight slots).
    #[test]
    fn requested_columns_stay_loaded() {
        let forced = HashSet::new();
        for decorates in [true, false] {
            for radius in 2..=32 {
                let r = Ranges {
                    center: (5, -7),
                    radius,
                    extra: &[((radius * 2 + 9, 3), 4)],
                    forced: &forced,
                    cover: None,
                    forced_cover: None,
                    band: None,
                    async_requests: None,
                };
                let span = radius * 3 + 20;
                for x in -span..=span {
                    for z in -span..=span {
                        if r.requested(x, z, decorates) {
                            assert!(
                                r.chunk_kept(x, z, decorates),
                                "radius {radius} (decorates: {decorates}): ({x}, {z}) is asked for but dropped"
                            );
                        }
                        if r.chunk_kept(x, z, decorates) {
                            assert!(
                                !r.unloaded(x, z, decorates),
                                "radius {radius} (decorates: {decorates}): ({x}, {z}) is kept then unloaded"
                            );
                        }
                    }
                }
            }
        }
    }

    /// An idle dedicated server keeps no area around its centre: only a spawn
    /// search's chunk (FULL alone: its 3x3 decorated, 5x5 generated) and the
    /// forced chunks.
    #[test]
    fn no_area_around_an_idle_servers_centre() {
        let forced = HashSet::from([(30, 30)]);
        let extra = [((5, -3), -1)];
        let r = Ranges {
            center: (5, -3),
            radius: -1,
            extra: &extra,
            forced: &forced,
            cover: None,
            forced_cover: None,
            band: None,
            async_requests: None,
        };
        for x in -5i32..=15 {
            for z in -13i32..=7 {
                let d = (x - 5).abs().max((z + 3).abs());
                assert_eq!(r.in_range(x, z, 1), d == 0, "({x}, {z})");
                assert_eq!(r.chunk_kept(x, z, true), d <= 2, "({x}, {z})");
                assert_eq!(r.requested(x, z, true), d <= 2, "({x}, {z})");
            }
        }
        assert!(r.in_range(30, 30, 1));
        let idle = Ranges { extra: &[], ..r };
        assert!(!idle.requested(5, -3, true) && idle.unloaded(5, -3, true));
    }

    /// An idle server level with no players, extra areas or tickets unloads its
    /// resident chunks while the world itself stays alive.
    #[test]
    fn idle_level_unloads_all_unticketed_chunks() {
        let mut world = World::new(sunrise_world::registry::BlockStateId(0), 0);
        for key in [(0, 0), (1, -2), (-4, 5)] {
            world
                .chunks
                .insert(key, Arc::new(Chunk::empty(key.0, key.1)));
        }
        let forced = HashSet::new();
        let idle = Ranges {
            center: (0, 0),
            radius: -1,
            extra: &[],
            forced: &forced,
            cover: None,
            forced_cover: None,
            band: None,
            async_requests: None,
        };
        let mut planned = plan_unloads(&world, |x, z| idle.unloaded(x, z, true));
        assert_eq!(planned.len(), 3);
        let unloaded = commit_unloads(&mut world, &mut planned);
        assert_eq!(unloaded.len(), 3);
        assert!(world.chunks.is_empty());
    }

    /// A server player's area (`lan_update_streaming`: the server view
    /// distance + 1) finishes vanilla's FULL square: PLAYER_LOADING tickets at
    /// level 31 out to the view distance, FULL (33) two chunks past it. Every
    /// chunk the player tracks has its 3x3 neighbourhood finished, so it can
    /// be lit and sent, and generation reaches vanilla's FEATURES (34) and
    /// TERRAIN (35) rings.
    #[test]
    fn server_player_areas_finish_vanillas_full_square() {
        let forced = HashSet::new();
        for view in 2..=32 {
            let extra = [((40, -9), view + 1)];
            let r = Ranges {
                // The window's own area, far away.
                center: (-400, 300),
                radius: 2,
                extra: &extra,
                forced: &forced,
                cover: None,
                forced_cover: None,
                band: None,
                async_requests: None,
            };
            let level = |x: i32, z: i32| 31 + ((x - 40).abs().max((z + 9).abs()) - view).max(0);
            for x in 40 - view - 8..=40 + view + 8 {
                for z in -9 - view - 8..=-9 + view + 8 {
                    assert_eq!(
                        r.in_range(x, z, 1),
                        level(x, z) <= 33,
                        "view {view}: ({x}, {z})"
                    );
                    assert_eq!(
                        r.requested(x, z, true),
                        level(x, z) <= 35,
                        "view {view}: ({x}, {z})"
                    );
                    assert_eq!(
                        r.unloaded(x, z, true),
                        level(x, z) > 35,
                        "view {view}: ({x}, {z})"
                    );
                    // ChunkTrackingView.contains, at any view distance the player may ask for.
                    let tracked = |v: i64| {
                        let gap = |d: i32| i64::from(((d).abs() - 2).max(0));
                        gap(x - 40).pow(2) + gap(z + 9).pow(2) < v * v
                    };
                    if tracked(i64::from(view)) {
                        for (dx, dz) in (-1..=1).flat_map(|dx| (-1..=1).map(move |dz| (dx, dz))) {
                            assert!(
                                r.in_range(x + dx, z + dz, 1),
                                "view {view}: ({x}, {z}) tracked, not lit"
                            );
                        }
                    }
                }
            }
        }
    }

    /// A `/forceload`ed chunk holds a FORCED ticket at level 31
    /// (`ChunkMap.FORCED_TICKET_LEVEL`): the level rises one per ring
    /// (`ChunkTracker.computeLevelFromNeighbor`) and picks the column's
    /// status (`ChunkLevel.generationStatus`). So vanilla finishes the two
    /// rings around it (33), gives the third its features (34) and the
    /// fourth its terrain (35), and the rings past that are left alone.
    #[test]
    fn forced_chunks_reach_vanillas_rings() {
        let forced: HashSet<(i32, i32)> = [(40, -40)].into();
        let r = Ranges {
            // The player's own area, far away.
            center: (-400, 300),
            radius: 2,
            extra: &[],
            forced: &forced,
            cover: None,
            forced_cover: None,
            band: None,
            async_requests: None,
        };
        let level = |x: i32, z: i32| 31 + (x - 40).abs().max((z + 40).abs());
        for x in 40 - 6..=40 + 6 {
            for z in -40 - 6..=-40 + 6 {
                // The full ring: the column becomes a level chunk.
                assert_eq!(r.in_range(x, z, 1), level(x, z) <= 33, "({x}, {z})");
                // The features ring, and the terrain ring.
                assert_eq!(r.in_range(x, z, 2), level(x, z) <= 34, "({x}, {z})");
                assert_eq!(r.in_range(x, z, 3), level(x, z) <= 35, "({x}, {z})");
                assert_eq!(r.requested(x, z, true), level(x, z) <= 35, "({x}, {z})");
                assert_eq!(r.chunk_kept(x, z, true), level(x, z) <= 35, "({x}, {z})");
                assert_eq!(r.unloaded(x, z, true), level(x, z) > 35, "({x}, {z})");
            }
        }
        // Generation reaches the terrain ring: the features ring is
        // decorated from it.
        let mut fixture = GenerationFixture::new(2, true);
        fixture.center = (-400, 300);
        fixture.forced = forced;
        let candidates = GenerationScan::default()
            .candidates(fixture.input(), usize::MAX)
            .expect("a scan");
        for (dx, dz) in [(-4, 0), (4, 0), (0, -4), (0, 4), (3, 3)] {
            assert!(
                candidates
                    .iter()
                    .any(|&(_, x, z)| (x, z) == (40 + dx, -40 + dz)),
                "the terrain ring at ({}, {}) is not generated",
                40 + dx,
                -40 + dz
            );
        }
    }

    /// An edit's light change in a neighbouring column marks that column
    /// unsaved too, or a column saved before the edit keeps its old light on
    /// disk.
    #[test]
    fn light_spilling_into_a_saved_neighbour_marks_it_unsaved() {
        use crate::light::tests::{AIR, TORCH, tables, world};
        let tables = tables();
        let mut w = World::new(sunrise_world::registry::BlockStateId(0), 0);
        for (key, chunk) in world(|c| {
            c.set(14, 0, 8, TORCH);
        }) {
            w.chunks.insert(key, chunk);
        }
        assert!(w.modified.is_empty());
        Arc::make_mut(w.chunks.get_mut(&(0, 0)).unwrap()).set(14, 0, 8, AIR);
        let mut changed = crate::light::LightChanges::default();
        crate::light::relight(
            &mut w.chunks,
            &tables,
            true,
            &[(14, 0, 8)],
            false,
            &mut changed,
        );
        assert_eq!(w.chunks[&(1, 0)].light(0, 0, 8) & 15, 0); // was 12
        ChunkState::mark_light_changes(&mut w, &changed);
        assert!(w.modified.contains(&(1, 0)));
        assert!(w.modified.contains(&(0, 0)));
        // The torch never reached x -1: that column's light is unchanged.
        assert!(!w.modified.contains(&(-1, 0)));
    }

    fn resident_column(world: &mut World, light: u8) -> Chunk {
        let mut chunk = Chunk::empty(2, -3);
        chunk.set(1, 10, 1, 1);
        Arc::make_mut(&mut chunk.sections[0]).light = crate::chunk::Light::Uniform(light);
        chunk.lit = true;
        world.chunks.insert((2, -3), Arc::new(chunk.clone()));
        chunk
    }

    #[test]
    fn a_pad_only_resend_dirties_the_boundary_mesh_and_its_neighbours() {
        use crate::chunk::SECTIONS;
        use crate::light::light_from_layers;
        for (pad, boundary) in [(0, 0), (SECTIONS + 1, SECTIONS - 1)] {
            let mut world = World::new(sunrise_world::registry::BlockStateId(0), 0);
            let mut old = Chunk::empty(2, -3);
            let mut layers = vec![(None, None); SECTIONS + 2];
            layers[pad].1 = Some(vec![0x22; 2048]);
            light_from_layers(&mut old, &layers, true);
            world.install_light_column(old.clone());
            let mut new = old.clone();
            layers[pad].1 = Some(vec![0x77; 2048]);
            light_from_layers(&mut new, &layers, true);
            take_resent_column(&mut world, &old, new, false);
            assert_eq!(world.dirty_sections.len(), 9);
            for z in -4..=-2 {
                for x in 1..=3 {
                    assert!(world.dirty_sections.contains(&(x, boundary as i32, z)));
                }
            }
        }
    }

    /// A server's resent column uses the light in its packet, as vanilla's
    /// `handleLevelChunkWithLight` does: only a Sunrise server's bulk edit,
    /// whose packet light is from before its relight, is relit here. A proxy's
    /// next server resends its whole world at the same coordinates, and
    /// relighting every column and its neighbours is work that world never
    /// asked for (#10273).
    #[test]
    fn a_resent_column_is_relit_only_for_a_sunrise_servers_bulk_edit() {
        for relight in [false, true] {
            let mut world = World::new(sunrise_world::registry::BlockStateId(0), 0);
            let old = resident_column(&mut world, 0x50);
            let mut new = old.clone();
            new.set(1, 10, 1, 2);
            Arc::make_mut(&mut new.sections[0]).light = crate::chunk::Light::Uniform(0xF0);
            take_resent_column(&mut world, &old, new, relight);
            let chunk = &world.chunks[&(2, -3)];
            assert_eq!(chunk.block(1, 10, 1).0, 2);
            // The packet's light is what the column shows.
            assert_eq!(chunk.sections[0].light.get(0), 0xF0);
            // The changed section remeshes either way.
            assert!(world.dirty_sections.contains(&(2, 0, -3)));
            // Only a Sunrise bulk edit queues light work, for the column and
            // its eight neighbours.
            assert_eq!(world.dirty_light.len(), if relight { 9 } else { 0 });
        }
    }

    #[test]
    fn empty_resent_column_dirties_the_old_groups_even_with_unchanged_blocks() {
        let mut world = World::new(sunrise_world::registry::BlockStateId(0), 0);
        let mut old = resident_column(&mut world, 0x50);
        let mut resent = old.clone();
        resent.set(5, 104, 5, 2);
        old.set_connection_group(3, 64, 4, Some(42));
        world.install_light_column(old.clone());
        world.dirty_sections.clear();
        take_resent_column(&mut world, &old, resent, true);

        let group_section = (64 - crate::chunk::MIN_Y) >> 4;
        let block_section = (104 - crate::chunk::MIN_Y) >> 4;
        assert!((block_section - group_section).abs() > 1);
        assert!(world.dirty_sections.contains(&(2, block_section, -3)));
        assert!(world.dirty_sections.contains(&(2, group_section, -3)));
        assert_eq!(world.connection_group(2 * 16 + 3, 64, -3 * 16 + 4), None);
    }

    #[test]
    fn outer_decoration_columns_collects_distance_two_to_eight_only() {
        let mut chunks = FxHashMap::default();
        let mut light_views = FxHashMap::default();
        // Inside write zone (0..=1): excluded.
        chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        chunks.insert((1, -1), Arc::new(Chunk::empty(1, -1)));
        // Distance 2 in both maps: world.chunks wins over light_views.
        let mut full_c2 = Chunk::empty(2, 0);
        full_c2.set(0, 0, 0, 7);
        let mut view_c2 = Chunk::empty(2, 0);
        view_c2.set(0, 0, 0, 3);
        chunks.insert((2, 0), Arc::new(full_c2));
        light_views.insert((2, 0), Arc::new(view_c2));
        // Distance 8 in light_views only: included.
        light_views.insert((-8, 8), Arc::new(Chunk::empty(-8, 8)));
        // Distance 9: excluded.
        chunks.insert((9, 0), Arc::new(Chunk::empty(9, 0)));
        light_views.insert((0, -9), Arc::new(Chunk::empty(0, -9)));

        let outer = ChunkState::outer_decoration_columns(&chunks, &light_views, 0, 0);
        assert_eq!(outer.len(), 2);
        let c2 = outer.iter().find(|c| (c.x, c.z) == (2, 0)).unwrap();
        assert_eq!(c2.get(0, 0, 0), 7);
        assert!(outer.iter().any(|c| (c.x, c.z) == (-8, 8)));
    }

    #[test]
    fn async_request_columns_sort_strictly_after_player_forced_and_band_work() {
        let forced: HashSet<(i32, i32)> = [(200, 200)].into();
        let async_reqs: HashSet<(i32, i32)> = [(15, 0), (30, 0)].into();
        let band = Band {
            x0: 40,
            x1: 42,
            z0: 0,
            z1: 2,
            circle: None,
        };
        let ranges = Ranges {
            center: (0, 0),
            radius: 4,
            extra: &[],
            forced: &forced,
            cover: None,
            forced_cover: None,
            band: Some(&band),
            async_requests: Some(AsyncRequests {
                chunks: &async_reqs,
                cover: None,
            }),
        };
        let player_edge = ranges.priority(4, 0);
        let forced_chunk = ranges.priority(200, 200);
        let band_chunk = ranges.priority(40, 0);
        let near_async = ranges.priority(15, 0);
        let far_async = ranges.priority(30, 0);

        assert!(player_edge < near_async);
        assert!(forced_chunk < near_async);
        assert!(band_chunk < near_async);
        assert!(near_async < far_async);
        assert!(ranges.in_range(15, 0, 0));
        assert!(ranges.in_range(16, 1, 1));
        assert!(!ranges.unloaded(15, 0, true));
    }
}

#[cfg(test)]
mod cover_tests {
    use super::*;

    fn areas(seed: &mut u64, n: usize, spread: i32) -> Vec<((i32, i32), i32)> {
        let mut next = |m: i32| {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 7;
            *seed ^= *seed << 17;
            (*seed % (2 * m as u64 + 1)) as i32 - m
        };
        (0..n)
            .map(|_| ((next(spread), next(spread)), 2 + next(14).abs()))
            .collect()
    }

    /// Every question streaming asks gets the same answer with the grid.
    ///
    /// A debug build checks every fourth cell of each row, shifted by row and case so the cases still cover every
    /// cell's position relative to the areas: the full check took 20 of the ~30 s of the crate's tests.
    /// `cargo test --release -p sunrise-level --lib the_grid_answers_as_the_scan` checks every cell.
    #[test]
    #[ignore = "daily"]
    fn the_grid_answers_as_the_scan() {
        let stride = if cfg!(debug_assertions) { 4 } else { 1 };
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let forced: HashSet<(i32, i32)> = [(40, -40)].into();
        for case in 0..60 {
            let extra = areas(&mut seed, 2 + case % 20, [4, 30, 120][case % 3]);
            let cover = ExtraCover::new(&extra).expect("a grid");
            let ranges = |cover| Ranges {
                center: (3, -2),
                radius: 10,
                extra: &extra,
                forced: &forced,
                cover,
                forced_cover: None,
                band: None,
                async_requests: None,
            };
            let (scan, grid) = (ranges(None), ranges(Some(&cover)));
            let span = [4, 30, 120][case % 3] + 40;
            for z in -span..=span {
                let first = -span + (z + case as i32).rem_euclid(stride);
                for x in (first..=span).step_by(stride as usize) {
                    for m in 0..=ExtraCover::REACH + 2 {
                        assert_eq!(
                            scan.in_extra(x, z, m),
                            grid.in_extra(x, z, m),
                            "case {case} ({x}, {z}) {m}"
                        );
                    }
                    for d in [false, true] {
                        assert_eq!(scan.unloaded(x, z, d), grid.unloaded(x, z, d));
                        assert_eq!(scan.requested(x, z, d), grid.requested(x, z, d));
                        assert_eq!(scan.chunk_kept(x, z, d), grid.chunk_kept(x, z, d));
                    }
                }
            }
        }
    }

    /// A ticket's single column (radius -1) keeps the index beside a player's area: the
    /// grid and the buckets answer as a scan, and the column is covered from margin one.
    #[test]
    fn a_single_column_ticket_keeps_the_index() {
        let forced: HashSet<(i32, i32)> = HashSet::new();
        for (extra, span) in [
            (vec![((0, 0), 10), ((30, 5), -1)], 0..=40),
            (vec![((0, 0), 10), ((5000, 5000), -1)], 4980..=5020),
        ] {
            let cover = ExtraCover::new(&extra).expect("an index");
            let ranges = |cover| Ranges {
                center: (3, -2),
                radius: 10,
                extra: &extra,
                forced: &forced,
                cover,
                forced_cover: None,
                band: None,
                async_requests: None,
            };
            let (scan, index) = (ranges(None), ranges(Some(&cover)));
            let (cx, cz) = extra[1].0;
            assert!(!index.in_extra(cx, cz, 0) && index.in_extra(cx, cz, 1));
            for z in span.clone() {
                for x in span.clone() {
                    for m in 0..=ExtraCover::REACH {
                        assert_eq!(
                            scan.in_extra(x, z, m),
                            index.in_extra(x, z, m),
                            "({x}, {z}) {m}"
                        );
                    }
                }
            }
        }
    }

    /// Players thousands of blocks apart: no grid fits them, and the buckets
    /// answer what a scan of every area does, the nearest centre included.
    #[test]
    #[ignore = "daily"]
    fn the_buckets_answer_as_the_scan() {
        let mut seed = 0x51_7CC1_B727_220Au64;
        let forced: HashSet<(i32, i32)> = HashSet::new();
        for case in 0..12 {
            // Spread over 6000 columns (96 000 blocks), some side by side.
            let mut extra = areas(&mut seed, 2 + case * 9, 3000);
            extra.push((extra[0].0, 10));
            extra.push(((extra[1].0.0 + 5, extra[1].0.1 - 70), 4));
            let cover = ExtraCover::new(&extra).expect("buckets");
            let ranges = |cover| Ranges {
                center: (3, -2),
                radius: 10,
                extra: &extra,
                forced: &forced,
                cover,
                forced_cover: None,
                band: None,
                async_requests: None,
            };
            let (scan, buckets) = (ranges(None), ranges(Some(&cover)));
            // Around each area, and on a coarse lattice over the whole spread. A debug build
            // takes every third lattice point, shifted by case so the points still fall at
            // different positions against the 64-block bucket edges.
            let step = if cfg!(debug_assertions) { 21 } else { 7 };
            let shift = if cfg!(debug_assertions) {
                (case % 21) as i32
            } else {
                0
            };
            let near = extra.iter().flat_map(move |&((cx, cz), _)| {
                (-80 + shift..=80).step_by(step).flat_map(move |dz| {
                    (-80 + shift..=80)
                        .step_by(step)
                        .map(move |dx| (cx + dx, cz + dz))
                })
            });
            let lattice = (-3200..=3200)
                .step_by(97)
                .flat_map(|z| (-3200..=3200).step_by(89).map(move |x| (x, z)));
            for (x, z) in near.chain(lattice) {
                assert_eq!(
                    scan.distance(x, z),
                    buckets.distance(x, z),
                    "case {case} ({x}, {z})"
                );
                for m in 0..=ExtraCover::REACH {
                    assert_eq!(
                        scan.in_extra(x, z, m),
                        buckets.in_extra(x, z, m),
                        "case {case} ({x}, {z}) {m}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_stale_or_impossible_grid_is_not_used() {
        let mut cover = None;
        ExtraCover::refresh(&mut cover, &[((0, 0), 11), ((5, 5), 11)]);
        assert!(ExtraCover::matching(&cover, &[((0, 0), 11), ((5, 5), 11)]).is_some());
        assert!(ExtraCover::matching(&cover, &[((0, 0), 11), ((5, 6), 11)]).is_none());
        // Too far apart for a mebibyte of grid: buckets alone. One area alone: scans.
        let far = ExtraCover::new(&[((0, 0), 11), ((100_000, 0), 11)]).expect("buckets");
        assert!(far.grid.is_none());
        assert!(ExtraCover::new(&[((0, 0), 11)]).is_none());
        assert!(ExtraCover::new(&[((i32::MAX - 1, 0), 11), ((i32::MAX - 3, 0), 11)]).is_none());
        assert!(ExtraCover::new(&[((0, i32::MAX - 1), 11), ((0, i32::MAX - 3), 11)]).is_none());
        assert!(ExtraCover::new(&[((0, i32::MIN + 1), 11), ((0, i32::MIN + 3), 11)]).is_none());
    }

    /// cargo test --release -p sunrise-level --lib cover_tests::cover_cost -- --ignored --nocapture
    #[test]
    #[ignore]
    fn cover_cost() {
        let forced = HashSet::new();
        for players in [10usize, 25, 50] {
            // The benchmark's bots: most near spawn, a fifth flying outwards.
            let extra: Vec<_> = (0..players)
                .map(|i| {
                    let a = i as f64 * 2.399;
                    let d = if i % 5 == 0 { 30.0 } else { 3.0 };
                    (((a.cos() * d) as i32, (a.sin() * d) as i32), 11)
                })
                .collect();
            let cover = ExtraCover::new(&extra).unwrap();
            let ranges = |cover| Ranges {
                center: (0, 0),
                radius: 10,
                extra: &extra,
                forced: &forced,
                cover,
                forced_cover: None,
                band: None,
                async_requests: None,
            };
            // The columns streaming asks about: everything kept, and the
            // generation scan's ring past it.
            let columns: Vec<(i32, i32)> = (-60..=60)
                .flat_map(|z| (-60..=60).map(move |x| (x, z)))
                .filter(|&(x, z)| {
                    ranges(None).requested(x, z, true) || !ranges(None).unloaded(x, z, true)
                })
                .collect();
            let rounds = 100;
            let time = |r: &Ranges| {
                let t = std::time::Instant::now();
                let mut n = 0usize;
                for _ in 0..rounds {
                    for &(x, z) in &columns {
                        n += usize::from(r.unloaded(x, z, true));
                        n += usize::from(r.requested(x, z, true));
                        n += usize::from(r.in_range(x, z, 1));
                    }
                }
                std::hint::black_box(n);
                t.elapsed().as_secs_f64() * 1e3 / f64::from(rounds)
            };
            let t = std::time::Instant::now();
            for _ in 0..rounds {
                std::hint::black_box(ExtraCover::new(std::hint::black_box(&extra)));
            }
            let build = t.elapsed().as_secs_f64() * 1e3 / f64::from(rounds);
            println!(
                "{players} players, {} columns, 3 questions each: scan {:.3} ms, grid {:.3} ms (+ {build:.3} ms to build)",
                columns.len(),
                time(&ranges(None)),
                time(&ranges(Some(&cover))),
            );
        }
    }
}
