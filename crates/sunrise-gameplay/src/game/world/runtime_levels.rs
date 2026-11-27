//! Levels a mod opens and closes while the server runs (`host.runtime_levels()`,
//! docs/modding/src/how-to/world/runtime-levels.md).
//!
//! A request is checked when the mod makes it and queued; the queue is applied after the server
//! tick's level loop (`apply_runtime_levels`), never inside a level's tick, so no hook ever sees a
//! level appear or vanish under it. The table below holds what is open, waiting or closing, and
//! the generation each open of an id was given: a `LevelRef` from before an unload never names a
//! later open of the same id. A delete is an unload and then the removal of the level's files, on
//! the server thread in the same pass, so a reopen is applied only after the files are gone.
use super::{Dim, world_loading::ModStem};
use mod_api::{LevelRef, RuntimeBiomeSource, RuntimeGenerator, RuntimeLevel, RuntimeLevelError};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};
use sunrise_core::{LevelKey, identifier::Identifier};
use sunrise_level::chunk::{ExtraLevel, ExtraLevels, LevelShape};
use sunrise_sim::vanillagen::{TerrainResources, shared_biome_source_data, shared_resource_data};
use sunrise_world::{
    dimension_type::DimensionFacts,
    worldgen::{
        DecodeError,
        biome_source::{BiomeSourceError, ResolvedBiomeSource},
        mod_generators::{ModChunkGenerators, mod_chunk_generators},
        overrides::{DIMENSION_TYPES, DIMENSIONS, WorldgenOverrides, mod_dimensions},
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Queued to open.
    Opening,
    Open,
    /// Queued to close.
    Closing,
}

struct Entry {
    generation: u32,
    state: State,
    level: OpenLevel,
    /// The mod that opened it, named when a request of its fails after the call.
    owner: &'static str,
}

enum Request {
    Open(String),
    Unload(&'static str, String),
    /// The mod that asked, which a level with no entry cannot name any other way.
    Delete(&'static str, String),
}

/// The runtime levels of one world open: pure bookkeeping, which the game applies.
#[derive(Default)]
pub struct RuntimeTable {
    entries: BTreeMap<String, Entry>,
    /// Ids whose level is closed but whose files a delete could not all remove: not open, and not
    /// to be opened before a `delete` has finished the removal.
    erasing: BTreeSet<String>,
    queue: Vec<Request>,
    /// The last generation given; never reset, so no two opens of an id share one.
    last_generation: u32,
    #[cfg(any(test, feature = "test-support"))]
    pub fail_open_after_publication: bool,
    #[cfg(any(test, feature = "test-support"))]
    pub fail_erase: bool,
}

impl RuntimeTable {
    /// Clear levels while preserving handle generations.
    pub(in crate::game::server_side) fn clear(&mut self) {
        self.entries.clear();
        self.erasing.clear();
        self.queue.clear();
    }

    /// Queue an admitted level and return its generation.
    pub fn queue_open(
        &mut self,
        owner: &'static str,
        id: &str,
        level: OpenLevel,
    ) -> Result<u32, RuntimeLevelError> {
        if self.entries.contains_key(id) {
            return Err(RuntimeLevelError::AlreadyOpen);
        }
        if self.erasing.contains(id) {
            return Err(RuntimeLevelError::Removing);
        }
        self.last_generation += 1;
        let generation = self.last_generation;
        self.entries.insert(
            id.to_owned(),
            Entry {
                generation,
                state: State::Opening,
                level,
                owner,
            },
        );
        self.queue.push(Request::Open(id.to_owned()));
        Ok(generation)
    }

    /// Queue the close of `id`: it must be open, not waiting to open or to close.
    pub fn queue_unload(&mut self, owner: &'static str, id: &str) -> Result<(), RuntimeLevelError> {
        match self.entries.get_mut(id) {
            Some(entry) if entry.state == State::Open => {
                entry.state = State::Closing;
                self.queue.push(Request::Unload(owner, id.to_owned()));
                Ok(())
            }
            _ => Err(RuntimeLevelError::NotOpen),
        }
    }

    /// Queue the removal of `id`, which the caller has checked except against the table. A level
    /// that is open is marked closing until the removal is applied; one that is not in the table
    /// has only files to remove. A level waiting to open is refused: its open comes first.
    pub fn queue_delete(&mut self, owner: &'static str, id: &str) -> Result<(), RuntimeLevelError> {
        if let Some(entry) = self.entries.get_mut(id) {
            if entry.state == State::Opening {
                return Err(RuntimeLevelError::AlreadyOpen);
            }
            entry.state = State::Closing;
        }
        self.queue.push(Request::Delete(owner, id.to_owned()));
        Ok(())
    }

    /// Whether `id` is open: a level waiting to open is not, one waiting to close still is, and so
    /// is one the idle close dropped from memory (it opens again when entered).
    pub fn is_open(&self, id: &str) -> bool {
        self.entries
            .get(id)
            .is_some_and(|entry| entry.state != State::Opening)
    }

    /// Whether `id` is a runtime level, open or not yet.
    pub fn holds(&self, id: &str) -> bool {
        self.entries.contains_key(id)
    }

    /// The generation a handle to the level `id` must carry: 0 for a level every world has.
    pub fn generation_of(&self, id: &str) -> u32 {
        self.entries.get(id).map_or(0, |entry| entry.generation)
    }

    /// Whether a level below `id` (`id/...`) is in the table, open, waiting or with files to
    /// remove.
    pub(in crate::game::server_side) fn any_below(&self, id: &str) -> bool {
        let below = format!("{id}/");
        self.entries
            .keys()
            .chain(self.erasing.iter())
            .any(|other| other.starts_with(&below))
    }

    /// Whether the table knows `id` at all, as a level or as files left to remove.
    fn knows(&self, id: &str) -> bool {
        self.entries.contains_key(id) || self.erasing.contains(id)
    }

    fn is_erasing(&self, id: &str) -> bool {
        self.erasing.contains(id)
    }

    /// `id` is closed and its files are not all gone: it waits for a `delete` to finish.
    fn mark_erasing(&mut self, id: &str) {
        self.erasing.insert(id.to_owned());
    }

    fn finish_erasing(&mut self, id: &str) {
        self.erasing.remove(id);
    }

    /// Whether `id` is a runtime level that is open or closing: a level whose open is still
    /// waiting has nothing to close.
    fn is_live(&self, id: &str) -> bool {
        self.entries
            .get(id)
            .is_some_and(|entry| entry.state != State::Opening)
    }

    /// The own seed of each level in the table, by id: what the hub tells a joining client.
    pub fn seeds(&self) -> BTreeMap<String, i64> {
        self.entries
            .iter()
            .map(|(id, entry)| (id.clone(), entry.level.seed))
            .collect()
    }

    /// The mod that opened `id`, or "?" for a level the table no longer holds.
    fn owner_of(&self, id: &str) -> &'static str {
        self.entries.get(id).map_or("?", |entry| entry.owner)
    }

    /// The requests waiting, in the order made. Later requests wait for the next tick.
    fn take_queue(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.queue)
    }

    /// Put a level that was to close back in use: a player came in, or the close failed.
    fn keep_open(&mut self, id: &str) {
        if let Some(entry) = self.entries.get_mut(id)
            && entry.state == State::Closing
        {
            entry.state = State::Open;
        }
    }

    /// Forget `id`: it is closed, or never opened.
    pub fn remove(&mut self, id: &str) {
        self.entries.remove(id);
    }

    pub fn mark_open(&mut self, id: &str) {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.state = State::Open;
        }
    }
}

/// Parse a registry identifier like `BiomeSourceCatalog`, requiring a non-empty path.
fn parse_identifier(name: &str) -> Option<String> {
    let id = Identifier::parse(name).ok()?;
    (!id.path().is_empty()).then(|| id.to_string())
}

/// Convert a [`RuntimeBiomeSource`] into the level stem JSON `biome_source` object.
pub fn runtime_biome_source_json(
    source: &RuntimeBiomeSource,
) -> Result<serde_json::Value, RuntimeLevelError> {
    Ok(match source {
        RuntimeBiomeSource::Fixed(biome) => {
            serde_json::json!({"type": "minecraft:fixed", "biome": biome})
        }
        RuntimeBiomeSource::Checkerboard { biomes, scale } => {
            let holders = match biomes.as_slice() {
                [tag] if tag.starts_with('#') => serde_json::json!(tag),
                entries if entries.iter().any(|entry| entry.starts_with('#')) => {
                    return Err(RuntimeLevelError::InvalidBiomeSource);
                }
                entries => serde_json::json!(entries),
            };
            serde_json::json!({"type": "minecraft:checkerboard", "biomes": holders, "scale": scale})
        }
        RuntimeBiomeSource::MultiNoise(preset) => serde_json::json!({
            "type": "minecraft:multi_noise",
            "preset": preset,
        }),
        RuntimeBiomeSource::End => serde_json::json!({"type": "minecraft:the_end"}),
        RuntimeBiomeSource::Points(points) => {
            let interval = |(min, max): (f32, f32)| serde_json::json!([min, max]);
            let biomes: Vec<serde_json::Value> = points
                .iter()
                .map(|point| {
                    serde_json::json!({
                        "biome": point.biome,
                        "parameters": {
                            "temperature": interval(point.temperature),
                            "humidity": interval(point.humidity),
                            "continentalness": interval(point.continentalness),
                            "erosion": interval(point.erosion),
                            "depth": interval(point.depth),
                            "weirdness": interval(point.weirdness),
                            "offset": point.offset,
                        },
                    })
                })
                .collect();
            serde_json::json!({"type": "minecraft:multi_noise", "biomes": biomes})
        }
        RuntimeBiomeSource::Registered(id) => {
            let canonical = parse_identifier(id).ok_or(RuntimeLevelError::InvalidBiomeSource)?;
            if matches!(
                canonical.as_str(),
                "minecraft:fixed"
                    | "minecraft:checkerboard"
                    | "minecraft:multi_noise"
                    | "minecraft:the_end"
            ) {
                return Err(RuntimeLevelError::UnknownBiomeSource);
            }
            serde_json::json!({"type": canonical})
        }
        _ => return Err(RuntimeLevelError::InvalidBiomeSource),
    })
}

/// Classify registry lookup and codec errors for runtime biome sources.
pub fn classify_biome_source_error(error: BiomeSourceError) -> RuntimeLevelError {
    match error {
        BiomeSourceError::Decode(DecodeError::MissingRegistryEntry(_))
        | BiomeSourceError::MissingPreset => RuntimeLevelError::UnknownBiomeSource,
        _ => RuntimeLevelError::InvalidBiomeSource,
    }
}

/// Build a runtime level stem with the registered generator ID and requested biome source.
fn mod_stem(
    dimension_type: &str,
    id: &Identifier,
    biome_source: Option<&RuntimeBiomeSource>,
    runs_stages: bool,
) -> Result<serde_json::Value, RuntimeLevelError> {
    let mut generator = if let Some(dimension) = mod_dimensions()
        .iter()
        .find(|dimension| dimension.id == dimension_type)
    {
        let mut stem: serde_json::Value = serde_json::from_str(&dimension.stem)
            .map_err(|_| RuntimeLevelError::InvalidBiomeSource)?;
        stem.get_mut("generator")
            .map(serde_json::Value::take)
            .ok_or(RuntimeLevelError::InvalidBiomeSource)?
    } else {
        let mut generator = serde_json::json!({
            "biome_source": {"type": "minecraft:fixed", "biome": "minecraft:plains"},
        });
        // The runtime generator has always used the Overworld settings for opted-in stages
        // when its dimension type supplies no registered stem. No-stage generators use none.
        if runs_stages {
            generator["settings"] = "minecraft:overworld".into();
        }
        generator
    };
    let generator = generator
        .as_object_mut()
        .ok_or(RuntimeLevelError::InvalidBiomeSource)?;
    generator.insert("type".into(), id.as_str().into());
    if let Some(source) = biome_source {
        generator.insert("biome_source".into(), runtime_biome_source_json(source)?);
    }
    Ok(serde_json::json!({"type": dimension_type, "generator": generator}))
}

/// Maximum recent extra-level shape tables retained for reuse (#1367).
const RETAINED_TABLE_CAP: usize = 64;

/// Eviction drops the cache's Arc while readers keep their snapshots.
#[derive(Default)]
struct ShapeTableCache {
    tables: VecDeque<Arc<[ExtraLevel]>>,
}

impl ShapeTableCache {
    fn intern(&mut self, levels: Vec<ExtraLevel>) -> Arc<[ExtraLevel]> {
        if levels.is_empty() {
            return Arc::from([]);
        }
        if let Some(index) = self
            .tables
            .iter()
            .position(|table| table.as_ref() == levels.as_slice())
        {
            let table = self.tables.remove(index).expect("the table was found");
            self.tables.push_back(Arc::clone(&table));
            return table;
        }
        let table: Arc<[ExtraLevel]> = Arc::from(levels);
        if self.tables.len() >= RETAINED_TABLE_CAP {
            self.tables.pop_front();
        }
        self.tables.push_back(Arc::clone(&table));
        table
    }
}

/// Runtime levels share retained facts by startup-known type and shape tables by layout.
#[derive(Default)]
struct Leaked {
    types: HashMap<String, &'static DimensionFacts>,
    tables: ShapeTableCache,
}

static LEAKED: LazyLock<Mutex<Leaked>> = LazyLock::new(Mutex::default);

/// The names and facts of the level `id` of the startup-known `dimension_type`.
fn leaked_level(
    id: &str,
    dimension_type: &str,
    facts: DimensionFacts,
) -> (&'static str, &'static str, &'static DimensionFacts) {
    let key = mod_api::__private::dimension_key(id);
    let id_name = mod_api::__private::dimension_id(key).expect("the game's table made the key");
    let type_name = DimensionFacts::vanilla(dimension_type)
        .map(|facts| facts.id.as_str())
        .or_else(|| {
            mod_dimensions()
                .iter()
                .find(|dimension| dimension.id == dimension_type)
                .map(|dimension| dimension.id.as_str())
        })
        .expect("runtime level types are known at startup");
    let mut leaked = LEAKED.lock().unwrap_or_else(|e| e.into_inner());
    let facts = *leaked
        .types
        .entry(type_name.to_owned())
        .or_insert_with(|| Box::leak(Box::new(facts)));
    (id_name, type_name, facts)
}

fn retained_table(levels: Vec<ExtraLevel>) -> Arc<[ExtraLevel]> {
    LEAKED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .tables
        .intern(levels)
}

#[cfg(feature = "test-support")]
pub fn retained_metadata_counts() -> (usize, usize) {
    let leaked = LEAKED.lock().unwrap_or_else(|e| e.into_inner());
    (leaked.types.len(), leaked.tables.tables.len())
}

#[cfg(feature = "test-support")]
pub const RETAINED_TABLE_LIMIT: usize = RETAINED_TABLE_CAP;

/// Resolve a runtime level against its registered dimension type.
fn resolve_type(id: &str, dimension_type: &str) -> Option<(LevelShape, DimensionFacts)> {
    let mut overrides = WorldgenOverrides::default();
    if let Some(registered) = mod_dimensions()
        .iter()
        .find(|dimension| dimension.id == dimension_type)
    {
        overrides.set(
            DIMENSION_TYPES,
            &registered.id,
            registered.dimension_type.clone(),
        );
    } else if sunrise_world::worldgen::overrides::LevelHeight::vanilla(dimension_type).is_none() {
        return None;
    }
    overrides.set(
        DIMENSIONS,
        id,
        serde_json::json!({"type": dimension_type}).to_string(),
    );
    let height = overrides.level_height(id)?;
    let shape = LevelShape::new(
        height.min_y,
        height.height,
        height.logical_height,
        height.has_skylight,
        height.has_ceiling,
    )
    .ok()?;
    let facts = overrides.level_facts(id)?.ok()?;
    Some((shape, facts))
}

/// Whether every component of `id`'s namespace and path names one folder on every system: no
/// component ends with a dot or a space, and none is a Windows device name (`con`, `nul`,
/// `com1`, `lpt9`, with or without an extension), which Windows maps to something else.
fn portable_level_id(id: &str) -> bool {
    const DEVICES: [&str; 4] = ["con", "prn", "aux", "nul"];
    id.split([':', '/']).all(|component| {
        let stem = component
            .split('.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let numbered = ["com", "lpt"].iter().any(|device| {
            stem.strip_prefix(device)
                .is_some_and(|n| n.len() == 1 && n.as_bytes()[0].is_ascii_digit())
        });
        !component.ends_with(['.', ' ']) && !DEVICES.contains(&stem.as_str()) && !numbered
    })
}

/// The entries of `folder`, or what to answer for it when it cannot be listed: a folder that is
/// not there holds nothing, one that cannot be read may hold a level.
fn listing(folder: &Path) -> Result<std::fs::ReadDir, bool> {
    std::fs::read_dir(folder).map_err(|error| error.kind() != std::io::ErrorKind::NotFound)
}

/// Whether `folder`, or a folder below it, has the shape of a level: it holds a folder named
/// `region`, `entities` or `poi`, as every level's save writes. A link counts, since it cannot be
/// looked into, and so does anything that cannot be read.
fn tree_has_level_shape(folder: &Path) -> bool {
    let entries = match listing(folder) {
        Ok(entries) => entries,
        Err(unreadable) => return unreadable,
    };
    entries.into_iter().any(|entry| {
        let Ok(entry) = entry else {
            return true;
        };
        let Ok(kind) = entry.file_type() else {
            return true;
        };
        if kind.is_file() {
            return false;
        }
        kind.is_symlink()
            || matches!(
                entry.file_name().to_str(),
                Some("region" | "entities" | "poi")
            )
            || tree_has_level_shape(&entry.path())
    })
}

/// Whether the folder of a level holds a level of its own: a folder that is not one a level's
/// save writes (`region`, `entities`, `poi`, `data`), one of the first three holding a folder, as
/// the level `mymod:a/region` does under `mymod:a`, or a folder of `data` with a level's shape
/// somewhere below (`mymod:a/data/ab`). Plain files are not levels, and what else `data` holds
/// (mods keep their own files there) is not in the way. What cannot be read counts as a level:
/// the delete is refused rather than remove what it could not look at.
fn folder_holds_levels(folder: &Path) -> bool {
    let entries = match listing(folder) {
        Ok(entries) => entries,
        Err(unreadable) => return unreadable,
    };
    for entry in entries {
        let Ok(entry) = entry else {
            return true;
        };
        let Ok(kind) = entry.file_type() else {
            return true;
        };
        if kind.is_file() {
            continue;
        }
        // A link, or a folder no level's save writes.
        let name = entry.file_name();
        let own = !kind.is_symlink()
            && match name.to_str() {
                Some("data") => !tree_has_level_shape(&entry.path()),
                Some("region" | "entities" | "poi") => match listing(&entry.path()) {
                    Ok(inner) => !inner.into_iter().any(|child| {
                        child
                            .and_then(|child| child.file_type())
                            .map_or(true, |kind| !kind.is_file())
                    }),
                    Err(unreadable) => !unreadable,
                },
                _ => false,
            };
        if !own {
            return true;
        }
    }
    false
}

/// The ids other than `id` whose structure references file is `id`'s: each way of putting a `:`
/// back in place of a `_` of the file name that gives a valid id.
fn sharing_references_file(id: &str) -> Vec<String> {
    let name = id.replace(':', "_");
    let namespace_end = name.find('/').unwrap_or(name.len());
    name.char_indices()
        .filter(|(at, c)| *c == '_' && *at < namespace_end)
        .map(|(at, _)| format!("{}:{}", &name[..at], &name[at + 1..]))
        .filter(|other| other != id && sunrise_core::level_id::valid_level_id(other))
        .collect()
}

/// Fail if `path`, or a folder above it up to `stop` (included), is a link: a link would be
/// followed out of the world. What does not exist is no link.
fn ensure_no_links(path: &Path, stop: &Path, include_path: bool) -> std::io::Result<()> {
    for folder in path.ancestors().skip(usize::from(!include_path)) {
        // `stop` itself, `dimensions` or `data`, is looked at too: a link there leads out of the
        // world as well.
        match std::fs::symlink_metadata(folder) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(std::io::Error::other(format!(
                    "{} is a link",
                    folder.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        if folder == stop {
            break;
        }
    }
    Ok(())
}

/// Remove the files of the level `id` from the world `root`: its folder, and the structure
/// references kept beside the world for it unless `references` says another level shares that
/// file, with the folders they leave empty. Whatever is not there is not an error; a link on the
/// way is, before anything is removed. `id` has passed `owned_level_id`; the folder comes from it
/// alone.
pub fn erase_level_files(root: &Path, id: &str, references: bool) -> std::io::Result<()> {
    let dimensions = root.join("dimensions");
    let folder = sunrise_sim::save::dimension_dir(root, id);
    // Never the folder of anything but a level: `dimension_dir` gives one under `dimensions/`.
    if folder == dimensions || !folder.starts_with(&dimensions) {
        return Err(std::io::Error::other("not a level folder"));
    }
    let file = super::world_loading::structure_references_file(root, id);
    let data = root.join("data");
    ensure_no_links(&folder, &dimensions, true)?;
    if references {
        ensure_no_links(&file, &data, false)?;
    }
    let folder_gone = ignore_missing(std::fs::remove_dir_all(&folder));
    let file_gone = if references {
        ignore_missing(std::fs::remove_file(&file))
    } else {
        Ok(())
    };
    prune_empty_parents(&folder, &dimensions);
    if references {
        prune_empty_parents(&file, &data);
    }
    folder_gone.and(file_gone)
}

fn ignore_missing(result: std::io::Result<()>) -> std::io::Result<()> {
    match result {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Remove the folders above `path` that are empty, up to but not including `stop`.
fn prune_empty_parents(path: &Path, stop: &Path) {
    for parent in path.ancestors().skip(1) {
        if parent == stop || !parent.starts_with(stop) || std::fs::remove_dir(parent).is_err() {
            break;
        }
    }
}

/// An admitted runtime level: its dimension type and its bound generator and biome source.
pub struct OpenLevel {
    pub dimension_type: String,
    pub shape: LevelShape,
    pub facts: DimensionFacts,
    pub seed: i64,
    pub generate: Generate,
}

/// What `make_streaming` generates a runtime level with.
pub enum Generate {
    /// The default superflat layers or air, over a fixed biome.
    Layers { flat: bool, biome: u16 },
    /// A mod's generator, paired with the bound biome source of its stem.
    Stem(ModStem),
}

impl crate::game::ServerGame<'_> {
    /// The mod running, if it is one and a server world is open: who a runtime level request
    /// belongs to.
    fn runtime_caller(&self) -> Result<&'static str, RuntimeLevelError> {
        if !self.common.is_logical_server() || self.common.native_lifecycle.epoch().is_none() {
            return Err(RuntimeLevelError::Unavailable);
        }
        Ok(mod_api::content::crash::current_native_mod().unwrap_or("?"))
    }

    /// Validate a portable ID before deriving its folder.
    fn valid_runtime_level_id(id: &str) -> Result<(), RuntimeLevelError> {
        if !sunrise_core::level_id::valid_level_id(id) || !portable_level_id(id) {
            return Err(RuntimeLevelError::InvalidId);
        }
        Ok(())
    }

    /// Bind a runtime generator and its biome source before enqueueing the level.
    fn runtime_generation(
        &self,
        level: &RuntimeLevel,
        generators: &ModChunkGenerators,
    ) -> Result<Generate, RuntimeLevelError> {
        let biomes = world_biomes!(self);
        match &level.generator {
            RuntimeGenerator::Flat | RuntimeGenerator::Void => {
                let flat = matches!(level.generator, RuntimeGenerator::Flat);
                let canonical = match &level.biome_source {
                    Some(RuntimeBiomeSource::Fixed(name)) => {
                        parse_identifier(name).ok_or(RuntimeLevelError::InvalidBiomeSource)?
                    }
                    Some(_) => return Err(RuntimeLevelError::IncompatibleBiomeSource),
                    None if flat => "minecraft:plains".to_owned(),
                    None => "minecraft:the_void".to_owned(),
                };
                if !biomes.contains(&canonical) {
                    return Err(RuntimeLevelError::UnknownBiomeSource);
                }
                Ok(Generate::Layers {
                    flat,
                    biome: biomes.id(&canonical),
                })
            }
            RuntimeGenerator::Mod(name) => {
                let requested = Identifier::parse_namespaced(name)
                    .ok_or(RuntimeLevelError::UnknownGenerator)?;
                let (id, generator) = generators
                    .get_key_value(&requested)
                    .ok_or(RuntimeLevelError::UnknownGenerator)?;
                if let Some(source) = &level.biome_source
                    && source.invalid_field().is_some()
                {
                    return Err(RuntimeLevelError::InvalidBiomeSource);
                }
                let runs_stages = generator.vanilla_stages().any();
                let json = mod_stem(
                    &level.dimension_type,
                    id,
                    level.biome_source.as_ref(),
                    runs_stages,
                )?;
                let resources = &self.common.options.resources;
                let dir = resources.join("worldgen");
                let empty = Default::default();
                let overrides = self
                    .server
                    .world_tables
                    .0
                    .as_ref()
                    .map_or(&empty, |tables| &tables.biomes.overrides);
                let biome_data = shared_biome_source_data(&dir, overrides).map_err(|e| {
                    log::warn!("runtime level biome registries are unavailable: {e}");
                    RuntimeLevelError::Unavailable
                })?;
                let mut stem =
                    ModStem::bind(&biome_data, json, id.clone(), generator.clone(), level.seed)
                        .map_err(classify_biome_source_error)?;
                if matches!(level.biome_source, Some(RuntimeBiomeSource::Registered(_)))
                    && !matches!(stem.source.as_ref(), ResolvedBiomeSource::Registered(_))
                {
                    return Err(RuntimeLevelError::UnknownBiomeSource);
                }
                if !stem
                    .source
                    .possible_biomes()
                    .into_iter()
                    .all(|name| biomes.contains(name))
                {
                    return Err(RuntimeLevelError::UnknownBiomeSource);
                }
                if runs_stages {
                    let data = shared_resource_data(
                        TerrainResources {
                            data: Some(&self.common.pack.data_changes),
                            dir: &dir,
                            registry: self.common.pack.assets.registry,
                            biomes,
                            item_data: &self.common.res.core.item_data,
                        },
                        overrides,
                    )
                    .map_err(|e| {
                        log::warn!("runtime level worldgen registries are unavailable: {e}");
                        RuntimeLevelError::Unavailable
                    })?;
                    stem = stem.with_stage_data(data);
                }
                Ok(Generate::Stem(stem))
            }
            _ => Err(RuntimeLevelError::UnknownGenerator),
        }
    }

    /// `RuntimeLevels::open`: check the request now, queue it for after the level loop.
    pub fn runtime_level_open(
        &mut self,
        id: &str,
        level: RuntimeLevel,
    ) -> Result<LevelRef, RuntimeLevelError> {
        let result = self.runtime_level_queue_open(id, level, mod_chunk_generators());
        if let Err(error) = &result {
            self.runtime_refusal("open", id, *error);
        }
        result
    }

    pub(in crate::game) fn runtime_level_queue_open(
        &mut self,
        id: &str,
        level: RuntimeLevel,
        generators: &ModChunkGenerators,
    ) -> Result<LevelRef, RuntimeLevelError> {
        let owner = self.runtime_caller()?;
        Self::valid_runtime_level_id(id)?;
        if !self.server.levels.0.runtime.holds(id) && Dim::exists(id) {
            return Err(RuntimeLevelError::ReservedId);
        }
        let (shape, facts) = resolve_type(id, &level.dimension_type)
            .ok_or(RuntimeLevelError::UnknownDimensionType)?;
        let generate = self.runtime_generation(&level, generators)?;
        let generation = self.server.levels.0.runtime.queue_open(
            owner,
            id,
            OpenLevel {
                dimension_type: level.dimension_type,
                shape,
                facts,
                seed: level.seed,
                generate,
            },
        )?;
        let epoch = self
            .common
            .native_lifecycle
            .epoch()
            .expect("checked by the caller");
        Ok(LevelRef::with_generation(
            epoch,
            mod_api::__private::dimension_key(id),
            generation,
        ))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_runtime_level_queue_open(
        &mut self,
        id: &str,
        level: RuntimeLevel,
        generators: &ModChunkGenerators,
    ) -> Result<LevelRef, RuntimeLevelError> {
        self.runtime_level_queue_open(id, level, generators)
    }

    /// `RuntimeLevels::unload`: check the request now, queue it for after the level loop.
    pub fn runtime_level_unload(&mut self, id: &str) -> Result<(), RuntimeLevelError> {
        let result = self.runtime_level_queue_unload(id);
        if let Err(error) = &result {
            self.runtime_refusal("unload", id, *error);
        }
        result
    }

    fn runtime_level_queue_unload(&mut self, id: &str) -> Result<(), RuntimeLevelError> {
        let owner = self.runtime_caller()?;
        Self::valid_runtime_level_id(id)?;
        if self.server.levels.0.runtime.is_open(id) && self.level_has_players(Dim::from_id(id)) {
            return Err(RuntimeLevelError::PlayersInside);
        }
        self.server.levels.0.runtime.queue_unload(owner, id)
    }

    /// `RuntimeLevels::delete`: check the request now, queue it for after the level loop.
    pub fn runtime_level_delete(&mut self, id: &str) -> Result<(), RuntimeLevelError> {
        let result = self.runtime_level_queue_delete(id);
        if let Err(error) = &result {
            self.runtime_refusal("delete", id, *error);
        }
        result
    }

    fn runtime_level_queue_delete(&mut self, id: &str) -> Result<(), RuntimeLevelError> {
        let owner = self.runtime_caller()?;
        Self::valid_runtime_level_id(id)?;
        if !self.server.levels.0.runtime.holds(id) && Dim::exists(id) {
            return Err(RuntimeLevelError::ReservedId);
        }
        if self.server.levels.0.runtime.is_open(id) && self.level_has_players(Dim::from_id(id)) {
            return Err(RuntimeLevelError::PlayersInside);
        }
        if self.levels_below(id) {
            return Err(RuntimeLevelError::LevelsBelow);
        }
        self.server.levels.0.runtime.queue_delete(owner, id)
    }

    /// Whether a level exists below `id` (`id/...`): in the table, among the world's levels, or
    /// as a folder in the level's own. Folders nest, so removing `id` would remove it.
    fn levels_below(&self, id: &str) -> bool {
        let below = format!("{id}/");
        self.server.levels.0.runtime.any_below(id)
            || Dim::all().iter().any(|dim| dim.id().starts_with(&below))
            || folder_holds_levels(&sunrise_sim::save::dimension_dir(
                &self.server.world_root,
                id,
            ))
    }

    /// Whether the structure references file of `id` is also the file of a level that exists. A
    /// folder that cannot be looked at may be one: only one that is not there is no level.
    fn references_file_shared(&self, id: &str) -> bool {
        sharing_references_file(id).iter().any(|other| {
            self.server.levels.0.runtime.knows(other)
                || Dim::exists(other)
                || !std::fs::symlink_metadata(sunrise_sim::save::dimension_dir(
                    &self.server.world_root,
                    other,
                ))
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        })
    }

    /// `RuntimeLevels::is_open`.
    pub fn runtime_level_is_open(&self, id: &str) -> bool {
        self.server.levels.0.runtime.is_open(id)
    }

    fn runtime_refusal(&self, call: &str, id: &str, error: RuntimeLevelError) {
        log::warn!(
            "mod {}: runtime_levels {call} {id}: refused ({error:?})",
            mod_api::content::crash::current_native_mod().unwrap_or("?"),
        );
    }

    /// Whether a player, or an arrival on its way, is in the level `dim`.
    fn level_has_players(&self, dim: Dim) -> bool {
        let id = dim.id();
        self.server
            .lan
            .as_ref()
            .is_some_and(|host| host.hub.guests.as_slice().iter().any(|g| g.dimension == id))
            || !self.arrival_chunks(dim).is_empty()
            || (self.common.options.dedicated.is_none()
                && self.common.player.is_some()
                && self.common.dim == dim)
    }

    /// A handle to the server level `key`, of the generation it is open with.
    pub fn level_ref(&self, epoch: mod_api::WorldEpoch, key: mod_api::DimensionKey) -> LevelRef {
        let id = mod_api::__private::dimension_id(key).expect("the game's table made the key");
        LevelRef::with_generation(epoch, key, self.server.levels.0.runtime.generation_of(id))
    }

    /// Apply the requests mods made since the last time, in order. Runs after the level loop of a
    /// server tick, on the server thread, with no level in the middle of a tick. A request
    /// that no longer holds is dropped, with a line in the log.
    pub fn apply_runtime_levels(&mut self) {
        for request in self.server.levels.0.runtime.take_queue() {
            match request {
                Request::Open(id) => {
                    let owner = self.server.levels.0.runtime.owner_of(&id);
                    if let Err(why) = self.apply_runtime_open(&id) {
                        // Nothing of it stays: `is_open` says false and the id opens again.
                        self.unpublish_runtime_level(&id);
                        log::warn!("mod {owner}: runtime level {id} did not open: {why}");
                    }
                }
                Request::Unload(owner, id) => {
                    if let Err(why) = self.apply_runtime_unload(&id) {
                        // Still open: `is_open` says true and the mod can unload it again.
                        self.server.levels.0.runtime.keep_open(&id);
                        log::warn!(
                            "mod {owner}: runtime level {id} stays open, its close failed: {why}"
                        );
                    }
                }
                Request::Delete(owner, id) => self.apply_runtime_delete(owner, &id),
            }
        }
    }

    fn apply_runtime_open(&mut self, id: &str) -> sunrise_game_data::Result<()> {
        if self.server.levels.0.runtime.is_erasing(id) {
            return Err("the files of an earlier delete are not all removed".into());
        }
        let level = &self
            .server
            .levels
            .0
            .runtime
            .entries
            .get(id)
            .ok_or("the request is gone")?
            .level;
        let shape = level.shape;
        // The tables the game reads a level's shape, type and place from learn it first; a failure
        // below takes it out of them again (`unpublish_runtime_level`).
        let (id_name, dimension_type, facts) =
            leaked_level(id, &level.dimension_type, level.facts.clone());
        let mut extras = self.server.level_shapes.extra.to_vec();
        extras.push(ExtraLevel {
            id: id_name,
            dimension_type,
            shape,
        });
        self.server.level_shapes.extra = ExtraLevels::from_shared(retained_table(extras));
        if let Some(host) = self.server.lan.as_mut() {
            host.hub.config.levels = self.server.level_shapes.clone();
            host.hub.config.level_seeds = self.server.levels.0.runtime.seeds();
        }
        if let Ok(key) = LevelKey::parse(id) {
            DimensionFacts::add_level(key, facts);
        }
        Dim::add_extra(id);
        let dim = Dim::extra(id).ok_or("the level was not added")?;
        #[cfg(any(test, feature = "test-support"))]
        if self.server.levels.0.runtime.fail_open_after_publication {
            return Err("injected level construction failure".into());
        }
        let mut parked = self.open_runtime_level(dim)?;
        parked.world.game_time = self.common.world.game_time;
        self.server.levels.0.parked.insert(dim, parked);
        self.server.levels.0.runtime.mark_open(id);
        self.native_level_opened(dim);
        self.validate_mod_chunk_tickets(dim);
        Ok(())
    }

    fn apply_runtime_unload(&mut self, id: &str) -> sunrise_game_data::Result<()> {
        let dim = Dim::extra(id).ok_or("the level is not open")?;
        if self.level_has_players(dim) {
            return Err("a player came in".into());
        }
        // The level in place is parked by leaving it; the Overworld is always there to enter.
        let back = self.common.dim;
        if self.common.dim == dim {
            self.enter_level(Dim::Overworld)?;
        }
        if self.server.levels.0.parked.contains_key(&dim)
            && let Err(why) = self.close_parked_level(dim)
        {
            // The level stays as it was, and in place if it was.
            if self.common.dim != back {
                self.enter_level(back)?;
            }
            return Err(why);
        }
        self.unpublish_runtime_level(id);
        Ok(())
    }

    /// Delete the level `id`: close it if it is open, then remove its files. A level that cannot
    /// close, or has a level below it, stays as it was. One that closed but whose files cannot
    /// all be removed stays closed, with its id kept from `open` until a `delete` finishes.
    fn apply_runtime_delete(&mut self, owner: &'static str, id: &str) {
        if let Err(why) = self.close_to_delete(id) {
            // Still open, for another try.
            self.server.levels.0.runtime.keep_open(id);
            log::warn!("mod {owner}: runtime level {id} was not deleted: {why}");
            return;
        }
        match self.erase_runtime_files(id) {
            Ok(()) => self.server.levels.0.runtime.finish_erasing(id),
            Err(why) => {
                self.server.levels.0.runtime.mark_erasing(id);
                log::warn!(
                    "mod {owner}: runtime level {id} is closed but its files were not all \
                     removed ({why}); delete it again"
                );
            }
        }
    }

    fn close_to_delete(&mut self, id: &str) -> sunrise_game_data::Result<()> {
        // Admission already checked the id's syntax and portability. Recheck live-level safeguards.
        if !self.server.levels.0.runtime.holds(id) && Dim::exists(id) {
            return Err("a level every world has".into());
        }
        if self.levels_below(id) {
            return Err("a level exists below it".into());
        }
        // A level whose open is still waiting comes after this request: only files are removed.
        if self.server.levels.0.runtime.is_live(id) {
            self.apply_runtime_unload(id)?;
        }
        Ok(())
    }

    fn erase_runtime_files(&self, id: &str) -> std::io::Result<()> {
        #[cfg(any(test, feature = "test-support"))]
        if self.server.levels.0.runtime.fail_erase {
            return Err(std::io::Error::other("injected removal failure"));
        }
        erase_level_files(
            &self.server.world_root,
            id,
            !self.references_file_shared(id),
        )
    }

    /// The runtime level `id` was unloaded, or failed to open: it leaves every table it was added
    /// to, and its id is free again.
    fn unpublish_runtime_level(&mut self, id: &str) {
        // A save a mod asked for has nothing left to write, and must not open the level again.
        if let Some(dim) = Dim::extra(id) {
            self.server.levels.0.cancel_requested_save(dim);
        }
        self.server.levels.0.runtime.remove(id);
        Dim::remove_extra(id);
        let extras: Vec<_> = self
            .server
            .level_shapes
            .extra
            .iter()
            .copied()
            .filter(|level| level.id != id)
            .collect();
        self.server.level_shapes.extra = ExtraLevels::from_shared(retained_table(extras));
        if let Some(host) = self.server.lan.as_mut() {
            host.hub.config.levels = self.server.level_shapes.clone();
            host.hub.config.level_seeds = self.server.levels.0.runtime.seeds();
            host.hub.close_views_of(id);
        }
        if let Ok(key) = LevelKey::parse(id) {
            DimensionFacts::remove_level(key);
        }
    }

    /// What the level `dim` is opened with: its generator, and its seed, which the world's is for
    /// a level every world has.
    pub(in crate::game::server_side) fn runtime_terrain(
        &self,
        dim: Dim,
    ) -> Option<(&Generate, i64)> {
        let level = &self.server.levels.0.runtime.entries.get(dim.id())?.level;
        Some((&level.generate, level.seed))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Weak};

    #[test]
    fn evicted_shape_tables_free_after_the_last_reader() {
        use sunrise_level::chunk::{ExtraLevel, ExtraLevels, LevelShape, LevelShapes};

        let limit = super::RETAINED_TABLE_CAP;
        let mut cache = super::ShapeTableCache::default();
        let layout = |index: usize| ExtraLevel {
            id: "cycle:a",
            dimension_type: "minecraft:overworld",
            shape: LevelShape {
                logical_height: LevelShape::OVERWORLD.logical_height - index as i32,
                ..LevelShape::OVERWORLD
            },
        };
        let mut tracked: Vec<Weak<[ExtraLevel]>> = Vec::new();
        let mut first_reader = None;
        let mut first_table = None;
        let mut oldest = None;
        let mut live_after_round = None;

        for round in 0..3 {
            for index in 0..=limit {
                let table = cache.intern(vec![layout(index)]);
                let weak = Arc::downgrade(&table);
                if !tracked.iter().any(|known| Weak::ptr_eq(known, &weak)) {
                    tracked.push(weak.clone());
                }
                if index == 1 {
                    oldest = Some(weak.clone());
                }
                if round == 0 && index == 0 {
                    first_table = Some(weak);
                    let shapes = LevelShapes {
                        extra: ExtraLevels::from_shared(Arc::clone(&table)),
                        ..LevelShapes::VANILLA
                    };
                    first_reader = Some(shapes.clone());
                }
            }

            let live = tracked
                .iter()
                .filter(|table| table.strong_count() > 0)
                .count();
            assert_eq!(
                live,
                limit + 1,
                "the cache retains {limit} tables and the evicted first table has one reader"
            );
            if let Some(previous) = live_after_round {
                assert_eq!(live, previous, "outstanding storage grew on round {round}");
            }
            live_after_round = Some(live);
        }

        let first_table = first_table.expect("the first table was allocated");
        assert_eq!(
            first_reader.as_ref().unwrap().extra[0],
            layout(0),
            "a reader still sees its immutable snapshot after cache eviction"
        );
        drop(first_reader);
        assert!(
            first_table.upgrade().is_none(),
            "eviction plus the final reader drop leaves no strong owner"
        );
        assert_eq!(
            tracked
                .iter()
                .filter(|table| table.strong_count() > 0)
                .count(),
            limit,
            "only the bounded cache's tables remain"
        );

        // Layout 0 fell out of the cache, so layout 1 is now the oldest entry.
        let promoted = Arc::downgrade(&cache.intern(vec![layout(1)]));
        assert!(
            oldest.unwrap().ptr_eq(&promoted),
            "a cache hit reuses the table"
        );
        drop(cache.intern(vec![layout(0)]));
        assert!(promoted.upgrade().is_some(), "a hit promotes the table");
    }
}
