//! Levels a mod opens and closes while the server runs (`host.runtime_levels()`,
//! docs/modding/src/how-to/world/runtime-levels.md).
//!
//! A request is checked when the mod makes it and queued; the queue is applied after the server
//! tick's level loop (`apply_runtime_levels`), never inside a level's tick, so no hook ever sees a
//! level appear or vanish under it. The table below holds what is open, waiting or closing, and
//! the generation each open of an id was given: a `LevelRef` from before an unload never names a
//! later open of the same id.
use super::{Dim, Game};
use horizon_client::chunk::{ExtraLevel, LevelShape};
use horizon_core::LevelKey;
use horizon_world::{
    dimension_type::DimensionFacts,
    worldgen::{
        mod_generators::mod_chunk_generators,
        overrides::{DIMENSION_TYPES, DIMENSIONS, WorldgenOverrides, mod_dimensions},
    },
};
use mod_api::{LevelRef, RuntimeGenerator, RuntimeLevel, RuntimeLevelError};
use std::collections::BTreeMap;

/// How many runtime levels a server keeps open unless `horizon-max-runtime-levels` says otherwise.
pub(in crate::game) const DEFAULT_CAP: usize = 64;

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
    level: RuntimeLevel,
    /// The mod that opened it, named when a request of its fails after the call.
    owner: &'static str,
}

enum Request {
    Open(String),
    Unload(String),
}

/// The runtime levels of one world open: pure bookkeeping, which the game applies.
pub(in crate::game) struct RuntimeTable {
    entries: BTreeMap<String, Entry>,
    queue: Vec<Request>,
    /// The last generation given; never reset, so no two opens of an id share one.
    last_generation: u32,
    cap: usize,
    #[cfg(test)]
    pub(super) fail_open_after_publication: bool,
}

impl Default for RuntimeTable {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            queue: Vec::new(),
            last_generation: 0,
            cap: DEFAULT_CAP,
            #[cfg(test)]
            fail_open_after_publication: false,
        }
    }
}

impl RuntimeTable {
    /// Forget every level, at the end of a world open. The cap and the generations stay.
    pub(in crate::game) fn clear(&mut self) {
        self.entries.clear();
        self.queue.clear();
    }

    pub(in crate::game) fn set_cap(&mut self, cap: usize) {
        self.cap = cap;
    }

    /// Queue the open of `id`, which the caller has checked except against the table: the cap and
    /// a repeat. Returns the generation of this open.
    pub(in crate::game) fn queue_open(
        &mut self,
        owner: &'static str,
        id: &str,
        level: RuntimeLevel,
    ) -> Result<u32, RuntimeLevelError> {
        if self.entries.contains_key(id) {
            return Err(RuntimeLevelError::AlreadyOpen);
        }
        if self.entries.len() >= self.cap {
            return Err(RuntimeLevelError::CapReached);
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
    pub(in crate::game) fn queue_unload(&mut self, id: &str) -> Result<(), RuntimeLevelError> {
        match self.entries.get_mut(id) {
            Some(entry) if entry.state == State::Open => {
                entry.state = State::Closing;
                self.queue.push(Request::Unload(id.to_owned()));
                Ok(())
            }
            _ => Err(RuntimeLevelError::NotOpen),
        }
    }

    /// Whether `id` is open: a level waiting to open is not, one waiting to close still is, and so
    /// is one the idle close dropped from memory (it opens again when entered).
    pub(in crate::game) fn is_open(&self, id: &str) -> bool {
        self.entries
            .get(id)
            .is_some_and(|entry| entry.state != State::Opening)
    }

    /// Whether `id` is a runtime level, open or not yet.
    pub(in crate::game) fn holds(&self, id: &str) -> bool {
        self.entries.contains_key(id)
    }

    /// The generation a handle to the level `id` must carry: 0 for a level every world has.
    pub(in crate::game) fn generation_of(&self, id: &str) -> u32 {
        self.entries.get(id).map_or(0, |entry| entry.generation)
    }

    pub(in crate::game) fn level_of(&self, id: &str) -> Option<&RuntimeLevel> {
        self.entries.get(id).map(|entry| &entry.level)
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
        if let Some(entry) = self.entries.get_mut(id) {
            entry.state = State::Open;
        }
    }

    /// Forget `id`: it is closed, or never opened.
    pub(in crate::game) fn remove(&mut self, id: &str) {
        self.entries.remove(id);
    }

    pub(in crate::game) fn mark_open(&mut self, id: &str) {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.state = State::Open;
        }
    }
}

/// The JSON of the stem a runtime level is generated from, as a mod's registered level has one:
/// the generator of the registered level `dimension_type` names, if any, with the generator
/// `name` in place of its own.
fn mod_stem(dimension_type: &str, name: &str) -> serde_json::Value {
    let mut generator = mod_dimensions()
        .iter()
        .find(|dimension| dimension.id == dimension_type)
        .and_then(|dimension| serde_json::from_str::<serde_json::Value>(&dimension.stem).ok())
        .map(|stem| stem["generator"].clone())
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| {
            serde_json::json!({
                "settings": "minecraft:overworld",
                "biome_source": {"type": "minecraft:fixed", "biome": "minecraft:plains"},
            })
        });
    generator["type"] = name.into();
    serde_json::json!({"type": dimension_type, "generator": generator})
}

/// The shape and facts of the dimension type `level` names, for the level `id`: a vanilla type,
/// or the type of a level a mod registered at startup. None for any other. Nothing is kept: the
/// level's names are leaked into the game's tables only when it opens.
fn resolve_type(id: &str, level: &RuntimeLevel) -> Option<(LevelShape, DimensionFacts)> {
    let mut overrides = WorldgenOverrides::default();
    if let Some(registered) = mod_dimensions()
        .iter()
        .find(|dimension| dimension.id == level.dimension_type)
    {
        overrides.set(
            DIMENSION_TYPES,
            &registered.id,
            registered.dimension_type.clone(),
        );
    } else if horizon_world::worldgen::overrides::LevelHeight::vanilla(&level.dimension_type)
        .is_none()
    {
        return None;
    }
    overrides.set(
        DIMENSIONS,
        id,
        serde_json::json!({"type": level.dimension_type}).to_string(),
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

/// What `make_streaming` generates a runtime level with.
pub(in crate::game) enum Generate {
    /// The default superflat layers.
    Flat,
    /// Air.
    Void,
    /// A mod's generator, by the stem that names it.
    Stem(serde_json::Value),
}

impl Generate {
    fn of(level: &RuntimeLevel) -> Option<Self> {
        match &level.generator {
            RuntimeGenerator::Flat => Some(Self::Flat),
            RuntimeGenerator::Void => Some(Self::Void),
            RuntimeGenerator::Mod(name) => Some(Self::Stem(mod_stem(&level.dimension_type, name))),
            _ => None,
        }
    }
}

impl Dim {
    /// Drops the dimension `id` that [`Self::add_extra`] added: its level is closed.
    fn remove_extra(id: &str) {
        super::EXTRA_DIMS
            .write()
            .unwrap()
            .retain(|key| mod_api::__private::dimension_id(*key) != Some(id));
    }
}

impl Game<'_> {
    /// The cap on open runtime levels `server.properties` sets (`horizon-max-runtime-levels`,
    /// docs/hosting.md): the default, 64, for a missing or unreadable value.
    pub(in crate::game) fn read_runtime_level_cap<'a>(
        &mut self,
        property: impl Fn(&str) -> Option<&'a str>,
    ) {
        if let Some(cap) = property("horizon-max-runtime-levels")
            .and_then(|value| value.trim().parse::<usize>().ok())
        {
            self.levels.runtime.set_cap(cap);
        }
    }

    /// The mod running, if it is one and a server world is open: who a runtime level request
    /// belongs to.
    fn runtime_caller(&self) -> Result<&'static str, RuntimeLevelError> {
        if !self.is_logical_server() || self.native_lifecycle.epoch().is_none() {
            return Err(RuntimeLevelError::Unavailable);
        }
        mod_api::content::crash::current_native_mod().ok_or(RuntimeLevelError::Unavailable)
    }

    /// `id` as a level the calling mod `owner` may open or close: a valid, portable id in a
    /// namespace the mod owns. The folder of a level is only ever derived from an id that passed
    /// this.
    fn owned_level_id(owner: &str, id: &str) -> Result<(), RuntimeLevelError> {
        if !horizon_core::level_id::valid_level_id(id) || !portable_level_id(id) {
            return Err(RuntimeLevelError::InvalidId);
        }
        let namespace = id.split_once(':').map_or("", |(namespace, _)| namespace);
        if !mod_api::__private::owns_namespace(owner, namespace) {
            return Err(RuntimeLevelError::OutsideNamespace);
        }
        Ok(())
    }

    /// `RuntimeLevels::open`: check the request now, queue it for after the level loop.
    pub(in crate::game) fn runtime_level_open(
        &mut self,
        id: &str,
        level: RuntimeLevel,
    ) -> Result<LevelRef, RuntimeLevelError> {
        let result = self.runtime_level_queue_open(id, level);
        if let Err(error) = &result {
            self.runtime_refusal("open", id, *error);
        }
        result
    }

    fn runtime_level_queue_open(
        &mut self,
        id: &str,
        level: RuntimeLevel,
    ) -> Result<LevelRef, RuntimeLevelError> {
        let owner = self.runtime_caller()?;
        Self::owned_level_id(owner, id)?;
        if !self.levels.runtime.holds(id) && Dim::exists(id) {
            return Err(RuntimeLevelError::ReservedId);
        }
        if resolve_type(id, &level).is_none() {
            return Err(RuntimeLevelError::UnknownDimensionType);
        }
        match &level.generator {
            RuntimeGenerator::Mod(name) if !mod_chunk_generators().contains_key(name) => {
                return Err(RuntimeLevelError::UnknownGenerator);
            }
            RuntimeGenerator::Noise => return Err(RuntimeLevelError::NoiseUnsupported),
            RuntimeGenerator::Flat | RuntimeGenerator::Void | RuntimeGenerator::Mod(_) => {}
            _ => return Err(RuntimeLevelError::UnknownGenerator),
        }
        let generation = self.levels.runtime.queue_open(owner, id, level)?;
        let epoch = self
            .native_lifecycle
            .epoch()
            .expect("checked by the caller");
        Ok(LevelRef::with_generation(
            epoch,
            mod_api::__private::dimension_key(id),
            generation,
        ))
    }

    /// `RuntimeLevels::unload`: check the request now, queue it for after the level loop.
    pub(in crate::game) fn runtime_level_unload(
        &mut self,
        id: &str,
    ) -> Result<(), RuntimeLevelError> {
        let result = self.runtime_level_queue_unload(id);
        if let Err(error) = &result {
            self.runtime_refusal("unload", id, *error);
        }
        result
    }

    fn runtime_level_queue_unload(&mut self, id: &str) -> Result<(), RuntimeLevelError> {
        let owner = self.runtime_caller()?;
        Self::owned_level_id(owner, id)?;
        if self.levels.runtime.is_open(id) && self.level_has_players(Dim::from_id(id)) {
            return Err(RuntimeLevelError::PlayersInside);
        }
        self.levels.runtime.queue_unload(id)
    }

    /// `RuntimeLevels::is_open`.
    pub(in crate::game) fn runtime_level_is_open(&self, id: &str) -> bool {
        self.levels.runtime.is_open(id)
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
        self.lan
            .as_ref()
            .is_some_and(|host| host.hub.guests.as_slice().iter().any(|g| g.dimension == id))
            || !self.arrival_chunks(dim).is_empty()
            || (self.options.dedicated.is_none() && self.player.is_some() && self.dim == dim)
    }

    /// Whether `level` names the level now open under its id: the world open it was made in, and
    /// for a runtime level the open it was given. A handle of an earlier open reads as stale.
    pub(in crate::game) fn level_handle_current(&self, level: LevelRef) -> bool {
        self.native_lifecycle.epoch() == Some(level.epoch())
            && level.generation() == self.levels.runtime.generation_of(level.dimension_id())
    }

    /// A handle to the server level `key`, of the generation it is open with.
    pub(in crate::game) fn level_ref(
        &self,
        epoch: mod_api::WorldEpoch,
        key: mod_api::DimensionKey,
    ) -> LevelRef {
        let id = mod_api::__private::dimension_id(key).expect("the game's table made the key");
        LevelRef::with_generation(epoch, key, self.levels.runtime.generation_of(id))
    }

    /// Apply the requests mods made since the last time, in order. Runs after the level loop of a
    /// server tick, on the server thread, with no level in the middle of a tick. A request
    /// that no longer holds is dropped, with a line in the log.
    pub(in crate::game) fn apply_runtime_levels(&mut self) {
        for request in self.levels.runtime.take_queue() {
            match request {
                Request::Open(id) => {
                    let owner = self.levels.runtime.owner_of(&id);
                    if let Err(why) = self.apply_runtime_open(&id) {
                        // Nothing of it stays: `is_open` says false and the id opens again.
                        self.unpublish_runtime_level(&id);
                        log::warn!("mod {owner}: runtime level {id} did not open: {why}");
                    }
                }
                Request::Unload(id) => {
                    let owner = self.levels.runtime.owner_of(&id);
                    if let Err(why) = self.apply_runtime_unload(&id) {
                        // Still open: `is_open` says true and the mod can unload it again.
                        self.levels.runtime.keep_open(&id);
                        log::warn!(
                            "mod {owner}: runtime level {id} stays open, its close failed: {why}"
                        );
                    }
                }
            }
        }
    }

    fn apply_runtime_open(&mut self, id: &str) -> horizon_client::Result<()> {
        let level = self
            .levels
            .runtime
            .level_of(id)
            .cloned()
            .ok_or("the request is gone")?;
        let (shape, facts) =
            resolve_type(id, &level).ok_or("its dimension type is no longer known")?;
        // The tables the game reads a level's shape, type and place from learn it first; a failure
        // below takes it out of them again (`unpublish_runtime_level`).
        let mut extras = self.level_shapes.extra.to_vec();
        extras.push(ExtraLevel {
            id: id.to_owned().leak(),
            dimension_type: level.dimension_type.clone().leak(),
            shape,
            seed: Some(level.seed),
        });
        self.level_shapes.extra = extras.leak();
        if let Some(host) = self.lan.as_mut() {
            host.hub.config.levels = self.level_shapes;
        }
        if let Ok(key) = LevelKey::parse(id) {
            DimensionFacts::add_level(key, facts);
        }
        Dim::add_extra(id);
        let dim = Dim::extra(id).ok_or("the level was not added")?;
        #[cfg(test)]
        if self.levels.runtime.fail_open_after_publication {
            return Err("injected level construction failure".into());
        }
        let mut parked = self.open_level(dim)?;
        parked.world.game_time = self.world.game_time;
        self.levels.parked.insert(dim, parked);
        self.levels.runtime.mark_open(id);
        self.native_level_opened(dim);
        Ok(())
    }

    fn apply_runtime_unload(&mut self, id: &str) -> horizon_client::Result<()> {
        let dim = Dim::extra(id).ok_or("the level is not open")?;
        if self.level_has_players(dim) {
            return Err("a player came in".into());
        }
        // The level in place is parked by leaving it; the Overworld is always there to enter.
        let back = self.dim;
        if self.dim == dim {
            self.enter_level(Dim::Overworld)?;
        }
        // A level the idle close already saved and dropped is only forgotten.
        if self.levels.parked.contains_key(&dim)
            && let Err(why) = self.close_parked_level(dim)
        {
            // The level stays as it was, and in place if it was.
            if self.dim != back {
                self.enter_level(back)?;
            }
            return Err(why);
        }
        self.unpublish_runtime_level(id);
        Ok(())
    }

    /// The runtime level `id` was unloaded, or failed to open: it leaves every table it was added
    /// to, and its id is free again.
    fn unpublish_runtime_level(&mut self, id: &str) {
        // A save a mod asked for has nothing left to write, and must not open the level again.
        if let Some(dim) = Dim::extra(id) {
            self.levels.cancel_requested_save(dim);
        }
        self.levels.runtime.remove(id);
        Dim::remove_extra(id);
        let extras: Vec<_> = self
            .level_shapes
            .extra
            .iter()
            .copied()
            .filter(|level| level.id != id)
            .collect();
        self.level_shapes.extra = extras.leak();
        if let Some(host) = self.lan.as_mut() {
            host.hub.config.levels = self.level_shapes;
        }
        if let Ok(key) = LevelKey::parse(id) {
            DimensionFacts::remove_level(key);
        }
    }

    /// What the level `dim` is opened with: its generator, and its seed, which the world's is for
    /// a level every world has.
    pub(in crate::game) fn runtime_terrain(&self, dim: Dim) -> Option<(Generate, i64)> {
        let level = self.levels.runtime.level_of(dim.id())?;
        Some((Generate::of(level)?, level.seed))
    }
}
