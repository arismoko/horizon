//! Shared memory census report types, formatting, and common `Game` entry points.
pub use crate::game::CommonGame;
pub use serde::Serialize;
pub use serde_json::Value;
pub use std::collections::BTreeMap;
pub use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{Receiver, Sender, channel, sync_channel},
};
pub use std::task::Poll;
pub use std::thread::{Builder, JoinHandle};
pub use sunrise_level::chunk::{Chunk, ColumnCensus, Section};
pub use sunrise_level::fast_hash::FxHashSet;
pub use sunrise_sim::server_perf::MemoryCost;

pub const MB: f64 = 1048576.0;
/// Entity types and players listed apart; the rest only count in the totals.
pub const TOP: usize = 30;
pub const TOP_PLAYERS: usize = 50;

pub fn mb(bytes: impl TryInto<u64>) -> f64 {
    let bytes: u64 = bytes.try_into().unwrap_or(0);
    (bytes as f64 / MB * 10.0).round() / 10.0
}

pub fn mb_signed(bytes: i64) -> f64 {
    (bytes as f64 / MB * 10.0).round() / 10.0
}

/// Counts each section once however many columns (or worlds) share it.
pub fn census(chunks: &[Arc<Chunk>]) -> (ColumnCensus, FxHashSet<*const Section>) {
    let mut census = ColumnCensus::default();
    let mut seen = FxHashSet::default();
    for chunk in chunks {
        chunk.census(&mut census, &mut seen);
    }
    (census, seen)
}

#[derive(Serialize)]
pub struct Columns {
    pub(in crate::game) total_mb: f64,
    pub(in crate::game) columns: usize,
    pub(in crate::game) sections: usize,
    pub(in crate::game) unique_sections: usize,
    pub(in crate::game) uniform_block_sections: usize,
    pub(in crate::game) light_arrays: usize,
    pub(in crate::game) blocks_mb: f64,
    pub(in crate::game) biomes_mb: f64,
    pub(in crate::game) light_mb: f64,
    pub(in crate::game) heightmaps_mb: f64,
    pub(in crate::game) section_records_mb: f64,
    pub(in crate::game) column_records_mb: f64,
    #[serde(skip)]
    pub(in crate::game) bytes: usize,
}

impl Columns {
    pub(in crate::game) fn new(c: &ColumnCensus) -> Self {
        Self {
            total_mb: mb(c.total_bytes()),
            columns: c.columns,
            sections: c.sections,
            unique_sections: c.unique_sections,
            uniform_block_sections: c.uniform_block_sections,
            light_arrays: c.light_arrays,
            blocks_mb: mb(c.block_bytes),
            biomes_mb: mb(c.biome_bytes),
            light_mb: mb(c.light_bytes),
            heightmaps_mb: mb(c.heightmap_bytes),
            section_records_mb: mb(c.section_record_bytes),
            column_records_mb: mb(c.column_bytes),
            bytes: c.total_bytes(),
        }
    }
}

/// Sections shared with `others` by address, and the heap bytes of one copy.
pub fn shared<'a>(
    entries: impl IntoIterator<Item = &'a Arc<Section>>,
    mut others: FxHashSet<*const Section>,
) -> SharedSections {
    let (mut sections, mut bytes) = (0, 0);
    for section in entries {
        if others.remove(&Arc::as_ptr(section)) {
            sections += 1;
            bytes += section.heap_bytes();
        }
    }
    SharedSections {
        sections,
        mb: mb(bytes),
        bytes,
    }
}

#[derive(Serialize)]
pub struct SharedSections {
    pub sections: usize,
    pub mb: f64,
    #[serde(skip)]
    pub bytes: usize,
}

#[derive(Serialize)]
pub struct Generation {
    pub(in crate::game) protos: usize,
    pub(in crate::game) protos_mb: f64,
    pub(in crate::game) proto_blocks_mb: f64,
    pub(in crate::game) proto_section_lists_mb: f64,
    pub(in crate::game) proto_heightmaps_mb: f64,
    pub(in crate::game) proto_other_terrain_mb: f64,
    pub(in crate::game) proto_biomes_mb: f64,
    pub(in crate::game) proto_records_mb: f64,
    pub(in crate::game) proto_sections_by_bits: [usize; 16],
    pub(in crate::game) light_views: usize,
    pub(in crate::game) light_views_mb: f64,
    /// Name, entries and megabytes where the cache can say.
    pub(in crate::game) caches: Vec<(&'static str, usize, Option<f64>)>,
    pub(in crate::game) stages: Vec<(&'static str, usize)>,
    #[serde(skip)]
    pub(in crate::game) bytes: usize,
}

/// Entities of every level by registry id: how many, and the bytes of their records.
#[derive(Serialize)]
pub struct Entities {
    pub count: usize,
    pub total_mb: f64,
    /// The `TOP` types holding the most bytes: id, count, megabytes.
    pub by_type: Vec<(String, usize, f64)>,
    #[serde(skip)]
    pub bytes: usize,
}

/// The connected players: queues and what each is owed or holds, the biggest first.
#[derive(Default, Serialize)]
pub struct Players {
    pub count: usize,
    pub total_mb: f64,
    /// Name, packets and megabytes waiting to be sent.
    pub send_queue: Vec<(String, usize, f64)>,
    pub chunks_waiting: Vec<(String, usize)>,
    pub chunks_sent: Vec<(String, usize)>,
    pub tracked_entities: Vec<(String, usize)>,
    #[serde(skip)]
    pub bytes: usize,
}

#[derive(Default, Serialize)]
pub struct Cached {
    pub entries: usize,
    pub size_mb: f64,
}

/// What the hub keeps to send chunks: encoded sections, heightmaps and compressed frames.
#[derive(Default, Serialize)]
pub struct ChunkSend {
    pub total_mb: f64,
    pub sections: Cached,
    pub heightmaps: Cached,
    pub frames: Cached,
    #[serde(skip)]
    pub bytes: usize,
}

#[derive(Serialize)]
pub struct Queue {
    pub len: usize,
    pub size_mb: f64,
}

#[derive(Serialize)]
pub struct Process {
    pub(in crate::game) resident_mb: Option<f64>,
    pub(in crate::game) allocator_commit_mb: f64,
    pub(in crate::game) allocator_peak_commit_mb: f64,
}

#[derive(Serialize)]
pub struct Allocator {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::game) live_heap_mb: Option<f64>,
    pub(in crate::game) commit_mb: f64,
    pub(in crate::game) peak_commit_mb: f64,
    /// Committed minus the pages in use.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::game) overhead_mb: Option<f64>,
}

#[derive(Serialize)]
pub struct FarTiles {
    pub active: usize,
    pub cached: usize,
    pub kept: usize,
    pub kept_mb: f64,
}

#[derive(Serialize)]
pub struct Gpu {
    pub allocated_mb: f64,
    pub reserved_mb: f64,
    pub by_label_mb: Vec<(String, f64)>,
}

#[derive(Serialize)]
pub struct Renderer {
    pub section_arenas_mb: f64,
    pub section_vertices_mb: f64,
    pub cpu: BTreeMap<String, f64>,
    pub gpu: Option<Gpu>,
}

/// A serving game's own parts: columns, generation, entities, players, chunk send caches, queues.
pub struct ServerSide {
    pub chunks: Vec<Arc<Chunk>>,
    pub generation: Generation,
    pub entities: Entities,
    pub players: Players,
    pub chunk_send: ChunkSend,
    pub light_queue: Queue,
    pub generation_queue: Queue,
    pub queue_bytes: usize,
}

impl ServerSide {
    /// The bytes of every part this side names.
    pub(in crate::game) fn bytes(&self) -> usize {
        self.generation.bytes
            + self.entities.bytes
            + self.players.bytes
            + self.chunk_send.bytes
            + self.queue_bytes
    }
}

/// How often a running capture records what holds the memory.
pub const MEMORY_EVERY_S: f64 = 10.0;

/// `sunrise-ctl memory`'s report; `*_mb` values round to tenths while bytes sum unrounded.
#[derive(Serialize)]
pub struct MemoryReport {
    pub(in crate::game) process: Process,
    pub(in crate::game) allocator: Allocator,
    pub(in crate::game) render_distance: i32,
    /// The window's columns; a dedicated server has no window and names them `columns`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::game) client_columns: Option<Columns>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::game) columns: Option<Columns>,
    pub(in crate::game) server_columns: Option<Columns>,
    pub(in crate::game) shared_with_server: Option<SharedSections>,
    pub(in crate::game) server_generation: Option<Generation>,
    pub(in crate::game) entities: Option<Entities>,
    pub(in crate::game) players: Option<Players>,
    pub(in crate::game) chunk_send: Option<ChunkSend>,
    pub(in crate::game) light_queue: Option<Queue>,
    pub(in crate::game) generation_queue: Option<Queue>,
    pub(in crate::game) far_tiles: Option<FarTiles>,
    pub(in crate::game) renderer: Option<Renderer>,
    /// Live heap by startup step, span, pool job, or thread when `alloc-census` is enabled.
    pub(in crate::game) heap_by_tag_mb: Option<Vec<(String, f64)>>,
    /// Columns, generation, entities, players, chunk send caches, queues, renderer CPU, far tiles.
    pub(in crate::game) named_mb: f64,
    /// Resident memory minus `named_mb` (may be negative where parts overlap).
    pub(in crate::game) unattributed_mb: Option<f64>,
}

/// Values owned by the game or renderer while copy-on-write handles keep section identities alive.
pub struct MemoryInputs {
    pub render_distance: i32,
    pub client: Option<Vec<Arc<Chunk>>>,
    pub far: Option<(FarTiles, usize)>,
}

impl MemoryInputs {
    pub fn report(
        self,
        server: Option<ServerSide>,
        active: impl Fn() -> bool,
        renderer: impl FnOnce() -> Option<(Renderer, usize)>,
    ) -> Option<MemoryReport> {
        let (commit, peak_commit) = sunrise_threads::allocator_commit();
        let live = sunrise_threads::allocator_stats::live_bytes();
        let resident = sunrise_threads::process_memory::resident_bytes();
        if !active() {
            return None;
        }
        let renderer = renderer();
        if !active() {
            return None;
        }
        let window = self
            .client
            .as_ref()
            .map(|chunks| Columns::new(&census(chunks).0));
        if !active() {
            return None;
        }
        let (server_columns, shared_with_server) = server.as_ref().map_or((None, None), |s| {
            let (columns, sections) = census(&s.chunks);
            let shared = self
                .client
                .as_ref()
                .map(|chunks| shared(chunks.iter().flat_map(|c| &c.sections), sections));
            (Some(Columns::new(&columns)), shared)
        });
        if !active() {
            return None;
        }
        let named = window.as_ref().map_or(0, |c| c.bytes)
            + server_columns.as_ref().map_or(0, |c| c.bytes)
            + server.as_ref().map_or(0, ServerSide::bytes)
            + renderer.as_ref().map_or(0, |r| r.1)
            + self.far.as_ref().map_or(0, |f| f.1);
        let named = named.saturating_sub(shared_with_server.as_ref().map_or(0, |s| s.bytes));
        let (columns, server_columns) = if window.is_some() {
            (None, server_columns)
        } else {
            (server_columns, None)
        };
        let mut report = MemoryReport {
            process: Process {
                resident_mb: resident.map(mb),
                allocator_commit_mb: mb(commit),
                allocator_peak_commit_mb: mb(peak_commit),
            },
            allocator: Allocator {
                live_heap_mb: live.map(mb),
                commit_mb: mb(commit),
                peak_commit_mb: mb(peak_commit),
                overhead_mb: live.map(|live| mb(commit.saturating_sub(live))),
            },
            render_distance: self.render_distance,
            client_columns: window,
            columns,
            server_columns,
            shared_with_server,
            server_generation: None,
            entities: None,
            players: None,
            chunk_send: None,
            light_queue: None,
            generation_queue: None,
            far_tiles: self.far.map(|f| f.0),
            renderer: renderer.map(|r| r.0),
            heap_by_tag_mb: sunrise_threads::HEAP_CENSUS.then(|| {
                sunrise_threads::heap_census::census()
                    .into_iter()
                    .take(60)
                    .map(|(tag, bytes)| (tag.to_string(), mb(bytes.max(0) as u64)))
                    .collect()
            }),
            named_mb: mb(named),
            unattributed_mb: unattributed(resident, named),
        };
        if let Some(s) = server {
            report.server_generation = Some(s.generation);
            report.entities = Some(s.entities);
            report.players = Some(s.players);
            report.chunk_send = Some(s.chunk_send);
            report.light_queue = Some(s.light_queue);
            report.generation_queue = Some(s.generation_queue);
        }
        Some(report)
    }
}

/// `resident - named` in megabytes, None without a resident size.
pub fn unattributed(resident: Option<u64>, named: usize) -> Option<f64> {
    resident.map(|r| mb_signed(r as i64 - named as i64))
}

/// `sunrise-ctl memory`'s report as capture rows: `a.b` paths and `key[name]` list entries.
pub fn flatten_report(report: &Value) -> Vec<(String, f64)> {
    let mut rows = Vec::new();
    flatten(String::new(), report, &mut rows);
    rows
}

pub fn flatten(path: String, value: &Value, rows: &mut Vec<(String, f64)>) {
    match value {
        Value::Number(n) => rows.extend(n.as_f64().map(|n| (path, n))),
        Value::Object(fields) => {
            for (key, inner) in fields {
                let next = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                flatten(next, inner, rows);
            }
        }
        Value::Array(items) => {
            for item in items {
                let Some([Value::String(name), count, rest @ ..]) =
                    item.as_array().map(Vec::as_slice)
                else {
                    continue;
                };
                rows.extend(count.as_f64().map(|n| (format!("{path}[{name}]"), n)));
                if let Some(mb) = rest.first().and_then(Value::as_f64) {
                    rows.push((format!("{path}_mb[{name}]"), mb));
                }
            }
        }
        _ => {}
    }
}

pub type MemoryRows = Vec<(String, f64)>;

/// Integrated owner answers directly to the worker and skips collection once cancelled.
pub struct ServerMemoryRequest {
    pub(in crate::game) reply: Option<Sender<Option<ServerSide>>>,
    pub(in crate::game) cancelled: Arc<AtomicBool>,
    pub(in crate::game) cost: MemoryCost,
}

pub struct MemoryCancellation {
    pub(in crate::game) cancelled: Arc<AtomicBool>,
    pub(in crate::game) wake_worker: Option<Sender<Option<ServerSide>>>,
}

impl Drop for MemoryCancellation {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// One census slot across captures; stop cancels the sample and waits for the worker to exit.
pub struct PendingMemory {
    pub(in crate::game) cancel: MemoryCancellation,
    pub worker: JoinHandle<()>,
    pub rows: Receiver<MemoryRows>,
}

pub trait MemoryCommonGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    fn memory_report(&self) -> MemoryReport {
        self.side_memory_report()
    }

    /// The report as JSON, for a manual control-socket request or a dedicated capture.
    fn memory_value(&self) -> Value {
        serde_json::to_value(self.memory_report()).unwrap_or(Value::Null)
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> MemoryCommonGameExt1<'__sg> for G {}

impl Drop for ServerMemoryRequest {
    fn drop(&mut self) {
        // The server may discard the request on shutdown without handling it.
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(None);
        }
    }
}

impl Generation {
    pub fn of(streaming: &sunrise_level::chunk_streamer::ChunkStreamer) -> Self {
        let c = streaming.proto_census();
        let [blocks, lists, heightmaps, other, biomes, records] = c.bytes;
        let protos: usize = c.bytes.iter().sum();
        let cached: usize = c.caches.iter().filter_map(|(_, _, bytes)| *bytes).sum();
        Self {
            protos: c.protos,
            protos_mb: mb(protos),
            proto_blocks_mb: mb(blocks),
            proto_section_lists_mb: mb(lists),
            proto_heightmaps_mb: mb(heightmaps),
            proto_other_terrain_mb: mb(other),
            proto_biomes_mb: mb(biomes),
            proto_records_mb: mb(records),
            proto_sections_by_bits: c.sections_by_bits,
            light_views: c.light_views,
            light_views_mb: mb(c.light_view_bytes),
            caches: c
                .caches
                .iter()
                .map(|(name, n, bytes)| (*name, *n, bytes.map(mb)))
                .collect(),
            stages: streaming.stages().to_vec(),
            bytes: protos + c.light_view_bytes + cached,
        }
    }
}

impl ServerMemoryRequest {
    pub fn collect(mut self, collect: impl FnOnce() -> ServerSide) {
        if self.cancelled.load(Ordering::Acquire) {
            return;
        }
        let snapshot = self.cost.measure(collect);
        if !self.cancelled.load(Ordering::Acquire) {
            let _ = self.reply.take().unwrap().send(Some(snapshot));
        }
    }
}

impl MemoryCancellation {
    pub fn cancel(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(wake) = self.wake_worker.take() {
            let _ = wake.send(None);
        }
    }
}

impl PendingMemory {
    pub fn cancel(&mut self) {
        self.cancel.cancel();
    }
    pub fn poll(&self) -> Poll<Option<MemoryRows>> {
        // Keep the slot reserved while either the worker or queued request still holds the flag.
        if !self.worker.is_finished() || Arc::strong_count(&self.cancel.cancelled) != 1 {
            return Poll::Pending;
        }
        if self.cancel.cancelled.load(Ordering::Acquire) {
            return Poll::Ready(None);
        }
        Poll::Ready(self.rows.try_recv().ok())
    }
    pub fn spawn(
        cost: MemoryCost,
        from_server: bool,
        report: impl FnOnce(Option<ServerSide>, &AtomicBool) -> Option<MemoryRows> + Send + 'static,
    ) -> std::io::Result<(Self, Option<ServerMemoryRequest>)> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let (reply, snapshot) = channel();
        let request = from_server.then(|| ServerMemoryRequest {
            reply: Some(reply.clone()),
            cancelled: cancelled.clone(),
            cost: cost.clone(),
        });
        let (publish, rows) = sync_channel(1);
        let worker = Builder::new().name("perf-memory".into()).spawn({
            let cancelled = cancelled.clone();
            move || {
                let server = if from_server {
                    let Ok(Some(snapshot)) = snapshot.recv() else {
                        return;
                    };
                    Some(snapshot)
                } else {
                    None
                };
                if cancelled.load(Ordering::Acquire) {
                    return;
                }
                sunrise_sim::server_perf::profiler_thread();
                let completed = cost.measure(|| report(server, &cancelled));
                if let Some(rows) = completed.filter(|_| !cancelled.load(Ordering::Acquire)) {
                    let _ = publish.send(rows);
                }
            }
        })?;
        let cancel = MemoryCancellation {
            cancelled,
            wake_worker: from_server.then_some(reply),
        };
        Ok((
            Self {
                cancel,
                worker,
                rows,
            },
            request,
        ))
    }
}

use crate::game::side_ops::presentation::PresentationSideGameExt as _;
