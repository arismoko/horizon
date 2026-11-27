//! World persistence in the vanilla 26.3 layout, readable by both Sunrise and
//! the vanilla game:
//! - `level.dat` (`Data`: name, game type, time, spawn, difficulty, version)
//!   plus the saved-data files `data/minecraft/{weather,world_clocks,
//!   world_gen_settings,game_rules}.dat`;
//! - the player in `players/data/<uuid>.dat` (`singleplayer_uuid`);
//! - per dimension `dimensions/minecraft/<name>/region/r.X.Z.mca` (chunks:
//!   paletted sections with light, heightmaps, block entities, scheduled
//!   block/fluid ticks) and `entities/r.X.Z.mca` (entities, `entity_nbt`).
//!
//! Data Sunrise does not simulate (structure references, unknown block
//! entities and entities, player fields such as the ender chest or effects,
//! game rules) is kept from the loaded files and written back unchanged.
//! Worlds saved by older Sunrise builds (`region/`, `DIM-1/`, `Player` in
//! `level.dat`) are migrated on open.
use crate::mod_chunk_tickets::{ModTicket, ModTickets, TicketOwner};
mod backpressure;
use compression_pool::{CompressedWrite, compress_records};
pub mod biome_regions;
pub mod block_entity_encode;
mod block_entity_order;
pub mod carrier;
mod far_regions;
mod horizon_names;
pub(crate) use horizon_names::adopt_in_player as adopt_horizon_player;
#[cfg(test)]
mod horizon_names_tests;
pub mod level_mods;
mod respawn;
mod writes;
use writes::{EvictionWrites, Outstanding, Writer};

use crate::{
    Result,
    beehive::BeehiveExt,
    biome::Biomes,
    chunk::{BIOME_STRATEGY, Chunk, ColumnLayout, Light, Section, block_strategy},
    mobs::{spawner::SpawnerExt, trial_spawner::TrialSpawnerExt},
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
};
use sunrise_core::{
    paletted::PalettedContainer,
    position::{BlockPos, ChunkPos},
    ticks::{SavedTick, TickPriority, TickTypeId},
};
use sunrise_level::chunk_streamer::ChunkGenerator;
use sunrise_mod_runtime::attachments::{ATTACHMENTS_KEY, attachment_compound};
use sunrise_world::worldgen::blending::BlendingData;
use {
    sunrise_core::{
        nbt,
        nbt::{Compound, JavaString, Limits, NamedTag, Tag},
    },
    sunrise_world::{
        DATA_VERSION,
        chunk::{BlockState, block_entity_pos},
        chunk_status::ChunkStatus,
        compression::Compression,
        heightmap::{Heightmap, HeightmapChunk, HeightmapState, HeightmapType},
        region::{ChunkRecord, RegionFile},
        registry::{BlockRegistry, BlockStateId},
        worldgen::terrain::TerrainBlockStates,
    },
};

pub const VERSION_NAME: &str = sunrise_core::VANILLA_VERSION;

mod chunk_disk;
#[cfg(test)]
mod chunk_disk_tests;
#[cfg(test)]
mod compression_bench;
mod compression_pool;
#[cfg(test)]
mod compression_tests;
mod gateway;
#[cfg(test)]
mod loot_tests;
mod record;
use record::{Encoding, Record};

/// Snapshot live block entities and custom names for a save batch without
/// mutating the world. Each map is visited once, retaining its iteration order
/// within each chunk (including repeated chunk coordinates in the input).
/// An empty batch skips both maps; one chunk uses the direct filter path.
pub fn chunks_with_block_entities<'a, E: std::hash::BuildHasher, N: std::hash::BuildHasher>(
    chunks: impl IntoIterator<Item = &'a Arc<Chunk>>,
    entities: &HashMap<(i32, i32, i32), crate::containers::BlockEntity, E>,
    names: &HashMap<(i32, i32, i32), Tag, N>,
) -> Vec<Chunk> {
    let chunks: Vec<_> = chunks.into_iter().collect();
    if chunks.is_empty() {
        return Vec::new();
    }
    if chunks.len() == 1 {
        let mut c = (**chunks[0]).clone();
        c.block_entities = entities
            .iter()
            .filter(|(p, _)| p.0 >> 4 == c.x && p.2 >> 4 == c.z)
            .map(|(p, e)| (*p, e.clone()))
            .collect();
        c.block_names = names
            .iter()
            .filter(|(p, _)| p.0 >> 4 == c.x && p.2 >> 4 == c.z)
            .map(|(p, n)| (*p, n.clone()))
            .collect();
        return vec![c];
    }
    // Index only requested chunks, borrowing their entries so unsaved entities
    // are never cloned. Clone one chunk at a time for inventory cache locality.
    let mut grouped = crate::fast_hash::FxHashMap::<_, (Vec<_>, Vec<_>)>::default();
    grouped.reserve(chunks.len());
    for c in &chunks {
        grouped.entry((c.x, c.z)).or_default();
    }
    for (p, e) in entities {
        if let Some((entries, _)) = grouped.get_mut(&(p.0 >> 4, p.2 >> 4)) {
            entries.push((p, e));
        }
    }
    for (p, n) in names {
        if let Some((_, entries)) = grouped.get_mut(&(p.0 >> 4, p.2 >> 4)) {
            entries.push((p, n));
        }
    }
    chunks
        .into_iter()
        .map(|chunk| {
            let mut c = (**chunk).clone();
            let (entities, names) = &grouped[&(c.x, c.z)];
            c.block_entities = entities.iter().map(|(p, e)| (**p, (*e).clone())).collect();
            c.block_names = names.iter().map(|(p, n)| (**p, (*n).clone())).collect();
            c
        })
        .collect()
}

use sunrise_game_data::nbt_value::string;
pub use sunrise_game_data::nbt_value::{float, number, text};
/// Insert or replace a key of a compound tag.
pub(crate) fn put(tag: &mut Tag, key: &str, value: Tag) {
    if let Tag::Compound(map) = tag {
        map.insert(key.into(), value);
    }
}
fn compound_or_empty(tag: Option<Tag>) -> Tag {
    match tag {
        Some(t @ Tag::Compound(_)) => t,
        _ => Tag::Compound(Compound::new()),
    }
}

/// `dimensions/<namespace>/<path>` inside a world folder. An id that is no safe folder name
/// (`test:../minecraft/overworld`, an absolute path) never leaves the world: it gets a folder
/// of its own under `dimensions/%invalid/`, named by its bytes, which no valid id can reach.
pub fn dimension_dir(world: &Path, dimension: &str) -> PathBuf {
    let id = if dimension.contains(':') {
        dimension.to_owned()
    } else {
        format!("minecraft:{dimension}")
    };
    let dimensions = world.join("dimensions");
    match id.split_once(':') {
        Some((namespace, path))
            if sunrise_core::level_id::valid_level_namespace(namespace)
                && sunrise_core::level_id::valid_level_path(path) =>
        {
            dimensions.join(namespace).join(path)
        }
        _ => {
            let hex: String = id.bytes().map(|b| format!("{b:02x}")).collect();
            dimensions.join("%invalid").join(hex)
        }
    }
}
mod session;
pub use session::{
    SessionLock, downgraded_from, incompatible_version, migrate_legacy, needs_file_fixes,
    upgrade_older_world, upgrade_older_world_with,
};

/// `LevelStorageAccess.checkForLowDiskSpace`: under 64 MiB usable.
pub const LOW_DISK_SPACE: u64 = 67_108_864;

/// The `PlayerData::inventory` slots of `EquipmentSlot.BODY` and `SADDLE`.
/// Vanilla saves them in the `equipment` compound with the armor and off hand.
pub const BODY_SLOT: u8 = 151;
pub const SADDLE_SLOT: u8 = 152;

/// The `EquipmentSlot` ordinals a player saves in `equipment`, with the
/// `PlayerData::inventory` slot each is held in: armor 100 (feet) to 103
/// (head), the off hand 150, body 151, saddle 152. The main hand is the
/// selected inventory slot and never in `equipment`.
const PLAYER_EQUIPMENT_SLOTS: [(usize, u8); 7] = [
    (sunrise_items::equipment::FEET, 100),
    (sunrise_items::equipment::LEGS, 101),
    (sunrise_items::equipment::CHEST, 102),
    (sunrise_items::equipment::HEAD, 103),
    (sunrise_items::equipment::OFFHAND, 150),
    (sunrise_items::equipment::BODY, BODY_SLOT),
    (sunrise_items::equipment::SADDLE, SADDLE_SLOT),
];

/// The `EquipmentSlot` ordinal a saved slot number stands for, for the
/// slots a player holder keeps in its `Equipment`.
pub fn equipment_slot(slot: u8) -> Option<usize> {
    match slot {
        BODY_SLOT => Some(sunrise_items::equipment::BODY),
        SADDLE_SLOT => Some(sunrise_items::equipment::SADDLE),
        _ => None,
    }
}
/// The saved slot number of a held `EquipmentSlot` ordinal (the inverse).
pub fn saved_slot(equipment_slot: usize) -> Option<u8> {
    match equipment_slot {
        sunrise_items::equipment::BODY => Some(BODY_SLOT),
        sunrise_items::equipment::SADDLE => Some(SADDLE_SLOT),
        _ => None,
    }
}
/// `LevelStorageAccess.estimateDiskSpace`: the usable bytes of the disk holding `dir`
/// (`FileStore.getUsableSpace`), `u64::MAX` when it cannot be asked.
pub fn estimate_disk_space(dir: &Path) -> u64 {
    fs4::available_space(dir).unwrap_or(u64::MAX)
}
/// `LevelStorageAccess.checkForLowDiskSpace`.
pub fn low_disk_space(dir: &Path) -> bool {
    estimate_disk_space(dir) < LOW_DISK_SPACE
}
mod worldgen;
pub use worldgen::*;
/// A data pack's JSON as the NBT `dimension_is_vanilla_like` reads (what a codec gets from
/// either one is the same data).
pub fn json_tag(value: &serde_json::Value) -> Tag {
    use serde_json::Value;
    match value {
        Value::Null => Tag::compound(std::iter::empty::<(&str, Tag)>()),
        Value::Bool(b) => Tag::Byte(i8::from(*b)),
        Value::Number(n) => n.as_i64().map_or_else(
            || Tag::Double(n.as_f64().unwrap_or_default().to_bits()),
            Tag::Long,
        ),
        Value::String(s) => Tag::String(s.as_str().into()),
        Value::Array(items) => Tag::List(items.iter().map(json_tag).collect()),
        Value::Object(map) => Tag::compound(map.iter().map(|(k, v)| (k.as_str(), json_tag(v)))),
    }
}

/// Registry lookups shared by chunk encoding and decoding.
pub struct SaveTables {
    /// Block registry IDs (`BlockId`, scheduled tick types) in registry order.
    block_names: Vec<String>,
    block_ids: HashMap<String, u16>,
    /// Default state of each state's block, to write vanilla's short palette form.
    default_state: Vec<bool>,
    block_entity: Vec<bool>,
    heightmaps: crate::assets::Properties,
    air: u32,
    /// The POI type and its max tickets of each state (`PoiTypes`).
    poi: Vec<Option<(&'static str, i32)>>,
}
impl SaveTables {
    pub fn new(assets: &crate::assets::Assets) -> Self {
        let registry = assets.registry;
        let mut block_names: Vec<String> = Vec::new();
        let mut default_state = Vec::with_capacity(registry.len());
        for id in 0..registry.len() {
            let state = registry
                .state(BlockStateId(id as u32))
                .expect("registry state");
            if block_names.last() != Some(&state.name) {
                block_names.push(state.name.clone());
            }
            default_state
                .push(registry.default_state(&state.name) == Some(BlockStateId(id as u32)));
        }
        let block_ids = block_names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.clone(), i as u16))
            .collect();
        Self {
            block_names,
            block_ids,
            default_state,
            block_entity: Vec::new(),
            heightmaps: assets.properties.clone(),
            air: registry.default_state("minecraft:air").map_or(0, |s| s.0),
            poi: (0..registry.len())
                .map(|id| {
                    registry
                        .state(BlockStateId(id as u32))
                        .and_then(sunrise_world::poi::poi_type)
                })
                .collect(),
        }
    }
    /// Tables with only the heightmaps' state facts, for tests.
    #[cfg(test)]
    pub(crate) fn for_heightmaps(heightmaps: crate::assets::Properties) -> Self {
        Self {
            block_names: Vec::new(),
            block_ids: HashMap::new(),
            default_state: Vec::new(),
            block_entity: Vec::new(),
            heightmaps,
            air: 0,
            poi: Vec::new(),
        }
    }
    /// Which states carry a block entity (`BlockFacts`), to drop entries for
    /// unknown block entities whose block is gone.
    pub fn with_block_entities(mut self, facts: &crate::blocks::BlockFacts) -> Self {
        self.block_entity = facts.states.iter().map(|s| s.block_entity).collect();
        self
    }
}

pub use crate::chunk::{ChunkExtras, PendingContainerLoot, SavedProto};

fn tick_list(ticks: &[SavedTick], name: impl Fn(u32) -> Option<String>) -> Tag {
    Tag::List(
        ticks
            .iter()
            .filter_map(|t| {
                Some(Tag::compound([
                    ("i", string(&name(t.kind.0)?)),
                    ("x", Tag::Int(t.pos.x)),
                    ("y", Tag::Int(t.pos.y)),
                    ("z", Tag::Int(t.pos.z)),
                    ("t", Tag::Int(t.delay)),
                    ("p", Tag::Int(t.priority as i32)),
                ]))
            })
            .collect(),
    )
}
fn read_ticks(
    tag: Option<&Tag>,
    chunk: (i32, i32),
    id: impl Fn(&str) -> Option<u32>,
) -> Vec<SavedTick> {
    let Some(Tag::List(list)) = tag else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|t| {
            let kind = id(&text(t.get("i"))?)?;
            let pos = BlockPos::new(
                number(t.get("x"))? as i32,
                number(t.get("y"))? as i32,
                number(t.get("z"))? as i32,
            );
            // SavedTick.filterTickListForChunk
            if (pos.x >> 4, pos.z >> 4) != chunk {
                return None;
            }
            Some(SavedTick {
                kind: TickTypeId(kind),
                pos,
                delay: number(t.get("t")).unwrap_or(0) as i32,
                priority: TickPriority::by_value(number(t.get("p")).unwrap_or(0) as i32),
            })
        })
        .collect()
}
fn fluid_name(kind: u32) -> Option<String> {
    use sunrise_world::fluid::FluidKind;
    match FluidKind::from_tick_id(kind)? {
        FluidKind::Empty => None,
        fluid => Some(fluid.name().to_owned()),
    }
}
fn fluid_id(name: &str) -> Option<u32> {
    use sunrise_world::fluid::FluidKind;
    FluidKind::from_name(name)
        .filter(|kind| *kind != FluidKind::Empty)
        .map(FluidKind::tick_id)
}

pub use sunrise_items::item_nbt::{
    decode_items, encode_item, encode_items, item_from_nbt, item_to_nbt,
};
mod block_entity_decode;
pub use block_entity_decode::*;

mod villager_poi;
pub use villager_poi::encode_poi;
mod column;
pub use column::*;

pub use crate::chunk::LevelShape;

type RegionCache = Mutex<RegionFiles>;
#[derive(Clone, Copy)]
struct RegionStorage<'a> {
    root: &'a Path,
    folder: Folder,
    cache: &'a RegionCache,
    reporting: &'a writes::SaveReporting,
}

impl RegionStorage<'_> {
    /// Readers and writers share a cache, so a read never sees a header older than the sectors
    /// it points to. `create` makes a missing file; otherwise a missing file is `None`.
    fn open_cached(
        self,
        (rx, rz): (i32, i32),
        create: bool,
    ) -> std::io::Result<Option<Arc<Mutex<RegionFile>>>> {
        let key = (rx, rz);
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| std::io::Error::other("region cache poisoned"))?;
        cache.clock += 1;
        let now = cache.clock;
        if let Some((cached, used)) = cache.regions.get_mut(&key)
            && (cached.is_some() || !create)
        {
            *used = now;
            return Ok(cached.clone());
        }
        let failed_evictions = cache.make_room();
        let opened = (|| {
            let path = region_path(self.root, self.folder.name(), rx, rz);
            if create && let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            // A file this process can't write (a read-only world) is still read.
            let region = match RegionFile::open_writable(&path, create) {
                Err(e) if !create && e.kind() == std::io::ErrorKind::PermissionDenied => {
                    RegionFile::open(&path)
                }
                other => other,
            };
            let region = match region {
                Ok(r) => Some(Arc::new(Mutex::new(r))),
                Err(e) if !create && e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e),
            };
            cache.regions.insert(key, (region.clone(), now));
            Ok(region)
        })();
        drop(cache);
        // Isolate the evicted region's failure: the requested region must use its own result.
        for ((rx, rz), error) in failed_evictions {
            let error = std::io::Error::new(
                error.kind(),
                format!(
                    "Failed to close region [{rx}, {rz}] in {}: {error}",
                    self.folder.name()
                ),
            );
            self.reporting
                .report(self.folder, (rx << 5, rz << 5), &error);
        }
        opened
    }
}
mod region;
#[cfg(test)]
use region::{MAX_OPEN_REGIONS, write_chunk_record, write_prepared_region_records};
use region::{RegionFiles, read_region_record, region_path, resolve_poi, write_region_records};

/// `job(i)` for every `i` below `count`, in order, shared between this thread
/// and up to `helpers - 1` scoped threads, each taking the next index as it
/// finishes one. A helper the system refuses to start (`start` false, or a
/// container at its limit of threads: EAGAIN) leaves its share to the
/// others: this thread does all of it at worst, and never panics for it, as
/// `thread::scope`'s own `spawn` would, which stopped the region writer.
#[cfg(test)]
fn share_out<R: Send>(
    count: usize,
    helpers: usize,
    job: impl Fn(usize) -> R + Sync,
    mut start: impl FnMut() -> bool,
) -> Vec<R> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let work = || {
        let mut out = Vec::new();
        loop {
            let i = next.fetch_add(1, Ordering::Relaxed);
            if i >= count {
                return out;
            }
            out.push((i, job(i)));
        }
    };
    let mut done: Vec<(usize, R)> = std::thread::scope(|scope| {
        let threads: Vec<_> = (1..helpers)
            .filter(|_| start())
            .filter_map(|_| std::thread::Builder::new().spawn_scoped(scope, work).ok())
            .collect();
        let mut done = work();
        for t in threads {
            done.extend(t.join().unwrap_or_else(|e| std::panic::resume_unwind(e)));
        }
        done
    });
    done.sort_unstable_by_key(|(i, _)| *i);
    done.into_iter().map(|(_, r)| r).collect()
}

/// The `__context` the chunk fixes read: the dimension and its generator type
/// (`ChunkMap.getChunkDataFixContextTag`).
struct DatafixContext {
    dimension: String,
    generator: Option<String>,
}
impl DatafixContext {
    /// From a dimension folder made by [`dimension_dir`] and the world's
    /// `world_gen_settings.dat`; None for a folder outside a world's `dimensions/`.
    fn of(root: &Path) -> Option<Self> {
        let mut parts = Vec::new();
        let mut at = root;
        while at.file_name()? != "dimensions" {
            parts.push(at.file_name()?.to_str()?);
            at = at.parent()?;
        }
        let world = at.parent()?;
        let namespace = parts.pop()?;
        if parts.is_empty() {
            return None;
        }
        parts.reverse();
        let dimension = format!("{namespace}:{}", parts.join("/"));
        let generator = sunrise_world::datafix::files::generators(world)
            .ok()
            .and_then(|g| g.into_iter().find(|(d, _)| *d == dimension))
            .map(|(_, g)| g);
        Some(Self {
            dimension,
            generator,
        })
    }
}
/// `SimpleRegionStorage.upgradeChunkTag`, as `ChunkMap.upgradeChunkTag` (chunks),
/// `EntityStorage` and `SectionStorage.PackedChunk.parse` (POI) call it on every
/// record they read: a world 26.3 has opened keeps the chunks nobody loaded since
/// an older version in that version's format, and `upgrade_older_world` only runs
/// for an older level.dat. A record without a DataVersion counts as -1 (POI: 1945).
/// `context` is needed only for an older chunk; without one it stays as saved.
fn upgrade_record<'a>(
    folder: Folder,
    record: &mut NamedTag,
    context: impl FnOnce() -> Option<&'a DatafixContext>,
) -> Result<()> {
    use sunrise_world::datafix::{self, schema::Ty};
    horizon_names::adopt_in_record(folder, &mut record.tag);
    let default = if folder == Folder::Poi { 1945 } else { -1 };
    if number(record.tag.get("DataVersion")).unwrap_or(default) >= i64::from(DATA_VERSION) {
        return Ok(());
    }
    let context = match folder {
        Folder::Region => match context() {
            Some(c) => Some(c),
            None => return Ok(()),
        },
        _ => None,
    };
    let tag = &mut record.tag;
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match (folder, context) {
        (Folder::Region, Some(c)) => {
            datafix::update_chunk(tag, &c.dimension, c.generator.as_deref());
        }
        (Folder::Entities, _) => datafix::update_region_tag(Ty::EntityChunk, tag),
        (Folder::Poi, _) => datafix::update_region_tag(Ty::PoiChunk, tag),
        (Folder::Region, None) => {}
    }))
    .map_err(|_| "upgrading an older record failed".into())
}
/// Chunk and entity records waiting for the writer thread, by (entities?,
/// x, z): reads see them before the region files do.
type PendingWrites = Mutex<HashMap<(Folder, i32, i32), (u64, Record)>>;
/// The region folders of a dimension.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Folder {
    Region,
    Entities,
    Poi,
}
impl Folder {
    fn of(name: &str) -> Self {
        match name {
            "entities" => Self::Entities,
            "poi" => Self::Poi,
            _ => Self::Region,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Region => "region",
            Self::Entities => "entities",
            Self::Poi => "poi",
        }
    }
}

mod dir;
pub use dir::*;
mod tickets;
use tickets::*;
mod lod;
pub use lod::*;

mod level_dat;
pub use level_dat::*;
mod saved_data;
use saved_data::{read_saved_data, saved_data_path, write_saved_data};
include!("save/preset_saved.rs");
pub use sunrise_core::uuid::uuid_text;
pub use sunrise_game_data::uuid::uuid_from_text;
/// A random (version 4) UUID as vanilla's four-int form.
///
/// Every call gets its own: the generator is seeded once from the clock and
/// the process, then each call takes the next step of one shared counter. A
/// seed from the clock alone repeats within its resolution (a microsecond on
/// macOS): a pack of mobs generated together all had the same UUID, and a
/// vanilla client keeps only one entity per UUID (the rest never moved).
pub fn random_uuid() -> [i32; 4] {
    use std::sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    };
    static NEXT: OnceLock<AtomicU64> = OnceLock::new();
    let next = NEXT.get_or_init(|| {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        AtomicU64::new(t ^ (std::process::id() as u64).rotate_left(32))
    });
    // SplitMix64: distinct steps give distinct outputs.
    let split = || {
        let mut z = next
            .fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed)
            .wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let (a, b) = (split(), split());
    let a = (a & !0xF000) | 0x4000;
    let b = (b & !(0xC000_0000_0000_0000)) | 0x8000_0000_0000_0000;
    [(a >> 32) as i32, a as i32, (b >> 32) as i32, b as i32]
}
/// `Mth.createInsecureUUID`: a new entity's UUID is its own random's first two
/// draws (`Entity.uuid`).
pub fn insecure_uuid(random: &mut impl sunrise_core::random::RandomSource) -> [i32; 4] {
    let most = random.next_long() & -61441 | 16384;
    let least = random.next_long() & 4_611_686_018_427_387_903 | i64::MIN;
    [
        (most >> 32) as i32,
        most as i32,
        (least >> 32) as i32,
        least as i32,
    ]
}
pub(crate) fn uuid_tag(u: [i32; 4]) -> Tag {
    Tag::IntArray(u.to_vec())
}
pub(crate) fn uuid_from_tag(tag: Option<&Tag>) -> Option<[i32; 4]> {
    match tag? {
        Tag::IntArray(v) if v.len() == 4 => Some([v[0], v[1], v[2], v[3]]),
        Tag::String(s) => uuid_from_text(&s.to_string_lossy()),
        _ => None,
    }
}
// `DoubleTag.valueOf` / `FloatTag.valueOf` store -0.0 as +0.0 (`ZERO`).
pub fn doubles(values: &[f64]) -> Tag {
    Tag::List(
        values
            .iter()
            .map(|&v| Tag::Double(if v == 0.0 { 0 } else { v.to_bits() }))
            .collect(),
    )
}
pub fn floats(values: &[f32]) -> Tag {
    Tag::List(
        values
            .iter()
            .map(|&v| Tag::Float(if v == 0.0 { 0 } else { v.to_bits() }))
            .collect(),
    )
}
pub(crate) fn read_doubles<const N: usize>(tag: Option<&Tag>) -> Option<[f64; N]> {
    match tag? {
        Tag::List(l) if l.len() == N => {
            let mut out = [0.0; N];
            for (o, v) in out.iter_mut().zip(l) {
                *o = float(Some(v))?;
            }
            Some(out)
        }
        _ => None,
    }
}

include!("save/preset_generation.rs");

mod level;
pub use level::*;

mod worlds;
pub use worlds::{WorldEntry, list_worlds};

#[cfg(test)]
mod tests;

/// Far-terrain floating-cell census on a real save: `SUNRISE_TEST_RESOURCES`
/// (prepared data), `SUNRISE_FAR_SAVE` (a world folder), `SUNRISE_FAR_SEED`
/// and `SUNRISE_FAR_AT` ("x,z", the camera). Samples the tiles each level
/// draws around the camera as the far sampler gives them (saved chunks up to
/// 16 blocks a cell, the seed beyond) and counts components of drawn cells
/// that touch neither the world's bottom nor the tile's halo.
#[cfg(test)]
mod far_census;

#[cfg(test)]
mod jukebox_song_clock_tests;

/// Added controls (H077): an old chunk's calculated blending data is taken
/// when its save is requested and written with `BlendingData.pack`; the raw
/// compound stays for the rest. The heights are the official 26.3 server's
/// for the constructed edge chunk (12, 0).
#[cfg(test)]
mod calculated_blending_save_tests;

#[cfg(test)]
mod biome_tests;
