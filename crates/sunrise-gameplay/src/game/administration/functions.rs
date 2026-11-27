//! Data packs in the game: functions (`/function`, `/return`, tick and load
//! tags, `/schedule`), predicates, `/reload` and `/datapack`.
use super::commands::{
    R, Target, err,
    execute::{ExecState, Source},
};
use mod_api::__private::reload::ReloadStageError;
use std::sync::Arc;
use sunrise_command::{Arg, Context};
use sunrise_core::nbt::Tag;
use sunrise_game_data::{
    scoreboard::bracketed,
    text::{self, Text},
};
pub use sunrise_gameplay_common::game::function_pack_stack::PackStack;
use sunrise_level::datapack::{self, DataPack, DataPacks, DataStore, Function};
use sunrise_sim::save;

/// A `TimerQueue` event.
#[derive(Clone, Debug, PartialEq)]
pub struct Scheduled {
    /// `ns:function`, or `#ns:tag` for a function tag.
    pub id: String,
    pub time: i64,
    pub seq: u64,
}
#[derive(Default)]
pub struct FnState {
    pub store: DataStore,
    pub packs: Vec<Arc<DataPack>>,
    pub data: Arc<DataPacks>,
    /// Enabled pack ids, in load order (later overrides earlier).
    pub enabled: Vec<String>,
    pub loaded: bool,
    /// Run the `minecraft:load` tag on the next tick.
    load_pending: bool,
    pub schedule: Vec<Scheduled>,
    /// The world's saved queue has been read once: it is the live
    /// `TimerQueue` `SavedData`, so a later reload keeps it as it is.
    schedule_loaded: bool,
    next_seq: u64,
    /// A dedicated server's `function-permission-level`.
    pub permission: Option<u8>,
}
/// What a function reported through `return`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    NoReturn,
    Value(i32),
    Fail,
}

#[derive(Clone)]
struct FunctionCall {
    functions: Vec<Function>,
    args: Option<Tag>,
}
/// One owned call for the still-live parsed invocation, not a generic queue.
/// The context address is compared only, never dereferenced or persisted.
pub(in crate::game) struct PreparedFunctionCall {
    invocation: usize,
    source: Source,
    call: std::sync::Arc<FunctionCall>,
}
impl PreparedFunctionCall {
    fn matches(&self, ctx: &Context, source: &Source) -> bool {
        let s = &self.source;
        self.invocation == ctx as *const Context as usize
            && s.entity == source.entity
            && s.pos.map(f64::to_bits) == source.pos.map(f64::to_bits)
            && s.rot.map(f32::to_bits) == source.rot.map(f32::to_bits)
            && s.eyes == source.eyes
            && s.silent == source.silent
            && s.store == source.store
            && s.dimension == source.dimension
            && s.level == source.level
            && s.block == source.block
            && s.cart == source.cart
    }
}

pub fn pack_stack(
    world_root: &std::path::Path,
    resources: &std::path::Path,
    builtin_source: &sunrise_assets::resource_source::ResourceSource,
    content_source: &sunrise_assets::resource_source::ResourceSource,
    vanilla_disabled: bool,
    enabled_features: &[String],
    additional: &[Arc<DataPack>],
) -> PackStack {
    let packs = sunrise_gameplay_common::game::pack_discovery::data_repository(
        world_root, resources, additional,
    );
    let enabled = datapack::enabled_ids(world_root, &packs, enabled_features);
    PackStack::new(
        builtin_source,
        content_source,
        vanilla_disabled,
        packs,
        enabled,
        enabled_features,
    )
}

/// Build the Create World stack from the temporary datapacks folder and the resources root.
pub(in crate::game) fn creation_stack(
    pack_dir: Option<&std::path::Path>,
    resources: &std::path::Path,
    builtin_source: &sunrise_assets::resource_source::ResourceSource,
    content_source: &sunrise_assets::resource_source::ResourceSource,
    selected: &[String],
    additional: &[Arc<DataPack>],
) -> PackStack {
    let packs = sunrise_gameplay_common::game::pack_discovery::data_repository_in(
        pack_dir.unwrap_or(std::path::Path::new("")),
        resources,
        additional,
    );
    let enabled = selected
        .iter()
        .filter(|id| packs.iter().any(|pack| &pack.id == *id))
        .cloned()
        .collect();
    PackStack::new(builtin_source, content_source, false, packs, enabled, &[])
}

pub(in crate::game) struct OverlayFiles {
    pub(in crate::game) files: Vec<(String, Vec<Vec<u8>>)>,
    pub(in crate::game) read_errors: std::collections::BTreeMap<String, String>,
    pub(in crate::game) priority_layers: Vec<sunrise_world::loot::RecipePriorityLayer>,
}

/// Collect the content files in pack order so the last layer supplies each winning file.
pub(in crate::game) fn overlay_files(data: &DataPacks) -> OverlayFiles {
    let store = DataStore::load(data);
    OverlayFiles {
        files: store.files.into_iter().collect(),
        read_errors: store.file_errors,
        priority_layers: store.priority_layers,
    }
}

impl FnState {
    pub(in crate::game) fn enabled_packs(&self) -> impl DoubleEndedIterator<Item = &DataPack> {
        self.enabled.iter().filter_map(|id| {
            self.packs
                .iter()
                .find(|pack| &pack.id == id)
                .map(Arc::as_ref)
        })
    }

    /// The enabled world packs in resource priority order, highest first: a
    /// later selected pack overrides an earlier one (`PackRepository` order).
    pub(in crate::game) fn enabled_packs_highest_first(&self) -> impl Iterator<Item = &DataPack> {
        self.enabled.iter().rev().filter_map(|id| {
            self.packs
                .iter()
                .find(|pack| &pack.id == id)
                .map(Arc::as_ref)
        })
    }

    /// The highest enabled world pack providing this data resource.
    pub fn data_resource(&self, name: &str) -> Option<sunrise_assets::resource_source::Resource> {
        self.enabled_packs_highest_first()
            .map(DataPack::source)
            .find(|source| source.contains(name))
            .map(|source| {
                sunrise_assets::resource_source::Resource::file(source.clone(), name.to_owned())
            })
    }

    /// Enabled pack ids, in load order (later overrides earlier).
    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(&self) -> &[String] {
        &self.enabled
    }
    /// The level function files compile at (`ServerFunctionLibrary`'s
    /// `functionCompilationPermissions`): `function-permission-level` on a
    /// dedicated server, game master (2) otherwise. Whoever runs a function,
    /// its commands need no more than this.
    fn command_level(&self) -> u8 {
        self.permission.unwrap_or(2)
    }
}

impl crate::game::ServerGame<'_> {
    // ---- loading -----------------------------------------------------------

    fn datapack_settings_path(&self) -> std::path::PathBuf {
        self.server.world_root.join("datapacks.json")
    }
    /// Discover and load the data packs of the world (once, or on `/reload`).
    pub fn ensure_datapacks(&mut self) {
        if self.server.fns.0.loaded || self.server.world_root.as_os_str().is_empty() {
            return;
        }
        if let Err(error) =
            self.reload_datapacks(false, &self.common.level.enabled_features.clone())
        {
            log::warn!("Sunrise: {}", error.diagnostic());
        }
    }
    pub fn open_world_datapacks(
        &mut self,
        enabled_features: &[String],
        prepared: Option<&mut super::world_loading::PreparedWorld>,
    ) -> Result<(PackStack, Option<super::world_loading::PreparedTerrain>), ReloadStageError> {
        // Function admission parses commands immediately, including block arguments.
        // Populate static registries on first open; admission synchronizes pack IDs.
        self.build_registries();
        self.load_datapacks(false, enabled_features, prepared)
    }

    pub fn reload_datapacks(
        &mut self,
        run_load: bool,
        enabled_features: &[String],
    ) -> Result<PackStack, ReloadStageError> {
        self.load_datapacks(run_load, enabled_features, None)
            .map(|(stack, _)| stack)
    }

    fn load_datapacks(
        &mut self,
        run_load: bool,
        enabled_features: &[String],
        opening: Option<&mut super::world_loading::PreparedWorld>,
    ) -> Result<(PackStack, Option<super::world_loading::PreparedTerrain>), ReloadStageError> {
        let opening_world = !run_load;
        sunrise_sim::mod_content::check_jars(
            &self.common.options.resources,
            &sunrise_level::logging::game_home().join("jars"),
        )
        .map_err(|error| {
            ReloadStageError::OriginalJars(
                sunrise_gameplay_common::game::api_values::TryIntoApi::try_into_api(&error)
                    .unwrap(),
            )
        })?;
        let next_root = self
            .common
            .stage_condition_inventory()
            .map_err(|error| ReloadStageError::DataSource(error.to_string()))?;
        let resources = next_root
            .as_ref()
            .map_or(self.common.options.resources.as_path(), |root| {
                root.prepared()
            })
            .to_path_buf();
        let data = next_root
            .as_ref()
            .map_or(&self.common.pack.data_source, |root| root.data());
        let changes = next_root
            .as_ref()
            .map_or(&self.common.pack.data_changes, |root| root.data_changes())
            .clone();
        if opening_world {
            // `DataPackConfig` keeps `vanilla` in Disabled across opens.
            self.server.cmds.0.vanilla_disabled =
                datapack::vanilla_disabled(&self.server.world_root);
        }
        let additional =
            sunrise_gameplay_common::game::pack_discovery::data_packs(&self.common.mods);
        let stack = pack_stack(
            &self.server.world_root,
            &resources,
            data,
            &changes,
            self.server.cmds.0.vanilla_disabled,
            enabled_features,
            &additional,
        );
        let prepared = self.begin_condition_reload(&stack, &resources)?;
        let context = self
            .common
            .prepare_loop_context(&resources)
            .map_err(|error| ReloadStageError::DataSource(error.to_string()))?;
        let world = if run_load {
            Some(
                self.prepare_world_reload(&stack, &resources)
                    .map_err(|error| ReloadStageError::DataSource(error.to_string()))?,
            )
        } else {
            None
        };
        let mut terrain = None;
        let initial = if opening_world {
            // This load warns again about saved attribute bases clamped to their bounds (#3728).
            sunrise_sim::living_attributes::LOAD_CLAMPS.begin_load();
            let tables = self
                .stage_world_tables(&stack, false, &resources)
                .map_err(|error| ReloadStageError::DataSource(error.to_string()))?;
            let biomes = tables.tables.as_ref().map(|tables| &tables.biomes);
            if let Some(opening) = opening {
                let mut options = self.common.options.clone();
                options.resources.clone_from(&resources);
                let mut pack = self.common.pack.clone();
                pack.data_changes = changes;
                pack.additional_data = additional.clone().into();
                terrain = Some(
                    opening
                        .prepare_terrain(
                            &self.common.res,
                            &pack,
                            &options,
                            &self.server.settings,
                            biomes.expect("opening stages the biome registry"),
                        )
                        .map_err(|error| ReloadStageError::DataSource(error.to_string()))?,
                );
            }
            let wire = super::lan::sync::prepare_world_sync_data(
                &resources,
                &self.common.res.core.item_data,
                &sunrise_gameplay_common::game::pack_files(&tables.tables),
                &tables.health.damage,
                biomes.map(|biomes| (biomes.table.clone(), biomes.differs_from_preload())),
                tables.tables.as_ref().map_or_else(
                    || self.common.res.shared_items(),
                    |tables| tables.items.clone(),
                ),
                tables
                    .tables
                    .as_ref()
                    .map(|tables| tables.packed_registries.clone()),
                tables.advancements.clone(),
            )
            .map_err(|error| ReloadStageError::DataSource(format!("world registries: {error}")))?;
            Some((tables, wire))
        } else {
            None
        };
        if let Some(world) = &world {
            prepared.bind_loot(world);
        }
        // Every candidate, including world tables, must prepare successfully
        // before any listener publishes its new state.
        if let Some(root) = next_root {
            self.common.options.resources = root.prepared().to_owned();
            self.common.pack.data_source = root.data().clone();
            self.common.pack.data_changes = root.data_changes().clone();
            self.common.pack.mod_layers = root.mod_layers().clone();
            self.common.mods.data_switched = true;
        }
        self.install_loop_context(context);
        if opening_world {
            self.server.fns.0 = Default::default();
        }
        self.server.fns.0.packs = stack.packs.clone();
        self.server.fns.0.enabled = stack.enabled.clone();
        self.common.pack.additional_data = additional.into();
        self.server.fns.0.loaded = true;
        self.server.fns.0.load_pending = true;
        // An explicit reload (`/reload`, `/datapack`) also rebuilds the
        // content tables, as `ReloadableServerResources` does. World open must
        // also install its tables before function admission validates inline holders.
        if let Some(world) = world {
            self.install_world_tables(world);
        } else if let Some((initial, wire)) = initial {
            if let Some(terrain) = &terrain {
                self.server.level_shapes = terrain.shapes.clone();
                self.common.end.set_locator(terrain.locator.clone());
                self.share_spawn_conditions();
            }
            self.open_world_tables(initial)
                .map_err(|error| ReloadStageError::DataSource(error.to_string()))?;
            super::lan::sync::install_world_sync_data(wire);
        }
        self.apply_server_reload(prepared, enabled_features);
        self.run_native_advancement_grants();
        // The schedule is the world's live `TimerQueue` `SavedData`, not pack
        // content: firing an event (`TimerQueue.tick`) and `/schedule clear`
        // (`TimerQueue.remove`) drop it from the queue, and a reload only
        // replaces the function library. Read the saved file when the world's
        // data loads, then keep the queue as it is.
        if !self.server.fns.0.schedule_loaded {
            self.server.fns.0.schedule_loaded = true;
            self.load_schedule();
        }
        Ok((stack, terrain))
    }
    /// `CommandFunction.parseCommand`: compile ordinary lines without running them.
    /// Macro lines are parsed
    /// only after substitution, but ordinary lines in a macro file still
    /// have to compile (`CommandFunction.fromLines`).
    pub(in crate::game::server_side) fn admit_store(&mut self, mut store: DataStore) {
        // The installed candidate already selected and checked the same winners
        // gameplay uses, including the native contributions.
        if let Some(tables) = &self.server.world_tables.0 {
            let copy = |entries: &std::collections::HashMap<String, serde_json::Value>| {
                entries
                    .iter()
                    .map(|(id, value)| (id.clone(), value.clone()))
                    .collect()
            };
            store.predicates = copy(tables.loot.predicates());
            store.item_modifiers = copy(tables.loot.item_modifiers());
            store.slot_sources = copy(tables.loot.slot_sources());
        }
        self.sync_enchantment_registries();
        // A function's `loot ... loot <id>` names the reloaded packs' tables.
        self.sync_loot_table_ids();
        // `ReloadableServerRegistries.reload` binds the packs' tags before
        // `ServerFunctionLibrary` compiles its functions: an item or block
        // argument's `#tag` resolves against them.
        self.sync_pack_tag_ids();
        let mut registries = self.common.ui.registries.clone();
        registries.permission = self.server.fns.0.command_level();
        registries.functions = store.functions.keys().cloned().collect();
        registries.function_tags = store.function_tags.keys().cloned().collect();
        registries.predicates = store.predicates.keys().cloned().collect();
        registries.item_modifiers = store.item_modifiers.keys().cloned().collect();
        let sources = registries
            .by_registry
            .entry("minecraft:slot_source".into())
            .or_default();
        sources.ids = store.slot_sources.keys().cloned().collect();
        sources.tags = store
            .slot_source_tags
            .keys()
            .filter(|id| store.slot_source_tag(id).is_ok())
            .cloned()
            .collect();
        for (id, ids) in [
            ("minecraft:predicate", &registries.predicates),
            ("minecraft:item_modifier", &registries.item_modifiers),
        ] {
            if let Some(entry) = registries.by_registry.get_mut(id) {
                entry.ids.clone_from(ids);
            }
        }
        let dispatcher = self.function_dispatcher();
        store.functions.retain(|id, function| {
            for line in &function.lines {
                if let datapack::FunctionLine::Command(line) = line
                    && let Err(error) = dispatcher.parse(line, &registries).executable()
                {
                    store.errors.push((
                        id.clone(),
                        format!(
                            "Invalid command {line:?}: {} at position {}",
                            error.message.plain(),
                            error.java_cursor()
                        ),
                    ));
                    return false;
                }
            }
            true
        });
        for (id, e) in &store.errors {
            log::warn!("Sunrise: failed to load function {id}: {e}");
        }
        self.server.fns.0.store = store;
        self.sync_function_registries();
    }
    pub fn save_datapack_settings(&self) {
        let disabled: Vec<&str> = self
            .server
            .fns
            .0
            .packs
            .iter()
            .filter(|p| !self.server.fns.0.enabled.contains(&p.id))
            .map(|p| p.id.as_str())
            .chain(self.server.cmds.0.vanilla_disabled.then_some("vanilla"))
            .collect();
        let json = serde_json::json!({"enabled": self.server.fns.0.enabled, "disabled": disabled});
        let _ = std::fs::write(self.datapack_settings_path(), json.to_string());
    }
    pub fn sync_function_registries(&mut self) {
        self.sync_schedule_registry();
        self.sync_enchantment_registries();
        let reg = &mut self.common.ui.registries;
        reg.functions = self.server.fns.0.store.functions.keys().cloned().collect();
        reg.function_tags = self
            .server
            .fns
            .0
            .store
            .function_tags
            .keys()
            .cloned()
            .collect();
        reg.predicates = self.server.fns.0.store.predicates.keys().cloned().collect();
        reg.item_modifiers = self
            .server
            .fns
            .0
            .store
            .item_modifiers
            .keys()
            .cloned()
            .collect();
        let sources = reg
            .by_registry
            .entry("minecraft:slot_source".into())
            .or_default();
        sources.ids = self
            .server
            .fns
            .0
            .store
            .slot_sources
            .keys()
            .cloned()
            .collect();
        sources.tags = self
            .server
            .fns
            .0
            .store
            .slot_source_tags
            .keys()
            .filter(|id| self.server.fns.0.store.slot_source_tag(id).is_ok())
            .cloned()
            .collect();
        reg.schedules = self
            .server
            .fns
            .0
            .schedule
            .iter()
            .map(|s| s.id.clone())
            .collect();

        reg.datapacks_enabled = (!self.server.cmds.0.vanilla_disabled)
            .then(|| "vanilla".to_owned())
            .into_iter()
            .chain(self.server.fns.0.enabled.iter().cloned())
            .collect();
        reg.datapacks_disabled = self
            .server
            .fns
            .0
            .packs
            .iter()
            .filter(|p| !self.server.fns.0.enabled.contains(&p.id))
            .map(|p| p.id.clone())
            .chain(
                self.server
                    .cmds
                    .0
                    .vanilla_disabled
                    .then(|| "vanilla".to_owned()),
            )
            .collect();
        // A pack's worldgen ids join the completion registries
        // (`command-registries.json` lists only the game's own), and so do
        // its worldgen tag ids for the `#tag` completions. The packs are the
        // ones the world's worldgen is built from.
        // World load checks condition errors before these completions read the accepted IDs.
        if let Some(tables) = &self.server.world_tables.0 {
            let (overrides, _) = sunrise_level::datapack::worldgen_overrides(
                &self.server.fns.0.data,
                &tables.conditions,
            );
            for (bundle, registry) in [
                ("biomes", "minecraft:worldgen/biome"),
                ("structures", "minecraft:worldgen/structure"),
                ("structure_sets", "minecraft:worldgen/structure_set"),
                ("template_pools", "minecraft:worldgen/template_pool"),
                ("processor_lists", "minecraft:worldgen/processor_list"),
            ] {
                let entry = reg.by_registry.entry(registry.to_owned()).or_default();
                for (id, _) in overrides.entries(bundle) {
                    if !entry.ids.contains(&id.to_owned()) {
                        entry.ids.push(id.to_owned());
                    }
                }
                entry.ids.sort();
            }
            let entry = reg
                .by_registry
                .entry("minecraft:worldgen/biome".to_owned())
                .or_default();
            for (id, _, _) in overrides.tags("biome_tags") {
                if !entry.tags.contains(&id.to_owned()) {
                    entry.tags.push(id.to_owned());
                }
            }
            entry.ids = tables
                .biomes
                .table
                .registry_order()
                .into_iter()
                .map(str::to_owned)
                .collect();
            entry.ids.sort();
            entry.tags = tables
                .pack_tags
                .get("worldgen/biome")
                .into_iter()
                .flat_map(|tags| tags.keys().cloned())
                .collect();
            entry.tags.sort();
            let entry = reg
                .by_registry
                .entry("minecraft:worldgen/structure".to_owned())
                .or_default();
            for (id, _, _) in overrides.tags("structure_tags") {
                if !entry.tags.contains(&id.to_owned()) {
                    entry.tags.push(id.to_owned());
                }
            }
            entry.tags.sort();
        }
    }

    fn sync_schedule_registry(&mut self) {
        self.common.ui.registries.schedules = self
            .server
            .fns
            .0
            .schedule
            .iter()
            .map(|s| s.id.clone())
            .collect();
    }

    // ---- scheduled events ------------------------------------------------------

    fn schedule_path(&self) -> std::path::PathBuf {
        self.server
            .world_root
            .join("data/minecraft/scheduled_events.dat")
    }
    fn load_schedule(&mut self) {
        let Some(root) = save::read_dat(&self.schedule_path()) else {
            return;
        };
        let Some(Tag::List(events)) = root.get("data").and_then(|d| d.get("events")) else {
            return;
        };
        for e in events {
            let (Some(Tag::Long(time)), Some(Tag::String(id)), Some(cb)) =
                (e.get("trigger_time"), e.get("id"), e.get("callback"))
            else {
                continue;
            };
            let _ = cb;
            self.server.fns.0.next_seq += 1;
            self.server.fns.0.schedule.push(Scheduled {
                id: id.to_string_lossy(),
                time: *time,
                seq: self.server.fns.0.next_seq,
            });
        }
    }
    pub fn save_schedule(&self) {
        if !self.server.fns.0.loaded || !self.common.is_logical_server() {
            return;
        }
        let mut events = self.server.fns.0.schedule.clone();
        events.sort_by_key(|e| (e.time, e.seq));
        let list = events
            .iter()
            .map(|e| {
                let (kind, name) = match e.id.strip_prefix('#') {
                    Some(tag) => ("minecraft:function_tag", tag),
                    None => ("minecraft:function", e.id.as_str()),
                };
                Tag::compound([
                    ("trigger_time", Tag::Long(e.time)),
                    ("id", Tag::String(e.id.as_str().into())),
                    (
                        "callback",
                        Tag::compound([
                            ("type", Tag::String(kind.into())),
                            ("id", Tag::String(name.into())),
                        ]),
                    ),
                ])
            })
            .collect();
        if events.is_empty() && !self.schedule_path().exists() {
            return;
        }
        let _ = save::write_dat(
            &self.schedule_path(),
            Tag::compound([
                ("data", Tag::compound([("events", Tag::List(list))])),
                ("DataVersion", Tag::Int(sunrise_world::DATA_VERSION)),
            ]),
        );
    }

    // ---- ticking -----------------------------------------------------------------

    /// `ServerFunctionManager.tick`: the `#minecraft:load` functions after a
    /// reload, then the `#minecraft:tick` ones, before the levels tick (and
    /// before the game time moves on).
    pub(in crate::game::server_side) fn tick_functions(&mut self) {
        // Load the lazy scoreboard and command storage before a load tag writes to them.
        self.load_scoreboard();
        self.server.exec.0 = ExecState::default();
        let source = self.server_source();
        if self.server.fns.0.load_pending {
            self.server.fns.0.load_pending = false;
            for f in self.server.fns.0.store.tag_functions("minecraft:load") {
                self.run_function_by_id(&f, None, source.clone());
            }
        }
        for f in self.server.fns.0.store.tag_functions("minecraft:tick") {
            self.run_function_by_id(&f, None, source.clone());
        }
    }
    /// `ServerLevel.tickTime`'s `getScheduledEvents().tick`: the scheduled
    /// functions due at the game time this tick has just reached.
    pub fn run_scheduled_functions(&mut self) {
        self.ensure_datapacks();
        if !self.common.is_logical_server() {
            return;
        }
        self.server.exec.0 = ExecState::default();
        let source = self.server_source();
        let now = self.common.world.game_time;
        loop {
            let due = self
                .server
                .fns
                .0
                .schedule
                .iter()
                .enumerate()
                .filter(|(_, e)| e.time <= now)
                .min_by_key(|(_, e)| (e.time, e.seq))
                .map(|(i, _)| i);
            let Some(i) = due else { break };
            let event = self.server.fns.0.schedule.remove(i);
            match event.id.strip_prefix('#') {
                Some(tag) => {
                    for f in self.server.fns.0.store.tag_functions(tag) {
                        self.run_function_by_id(&f, None, source.clone());
                    }
                }
                None => self.run_function_by_id(&event.id, None, source.clone()),
            }
        }
    }
    fn run_function_by_id(&mut self, id: &str, args: Option<&Tag>, source: Source) {
        self.server.exec.0.commands = 0;
        if let Some(f) = self.server.fns.0.store.functions.get(id).cloned() {
            // `ServerFunctionManager.execute` logs a function that failed to
            // instantiate or run; its lines' own failures stay silent.
            if let Err(e) = self.run_function(&f, args, source) {
                log::warn!(
                    "Sunrise: failed to run function {id}: {}",
                    e.message.plain()
                );
            }
        }
    }

    /// The function branch of advancement rewards: resolve the optional function
    /// and execute it as the rewarded player, with suppressed output and GAMEMASTER permission.
    pub(in crate::game::server_side) fn run_advancement_reward_function(&mut self, id: &str) {
        self.ensure_datapacks();
        let Some(function) = self.server.fns.0.store.functions.get(id).cloned() else {
            return;
        };
        let Some(pos) = self.target_position(Target::Player) else {
            return;
        };
        // createCommandSourceStack belongs to the player, not an enclosing
        // execute's position, rotation, output sink or store callback.
        let source = Source {
            pos,
            rot: self.target_rotation(Target::Player),
            silent: true,
            ..self.player_source()
        };
        // The existing runner applies this level to nested function calls too.
        let permission = self
            .server
            .fns
            .0
            .permission
            .replace(sunrise_command::vanilla::GAMEMASTERS);
        let callback = self.server.exec.0.callback.take();
        let result = self.run_function(&function, None, source);
        self.server.exec.0.callback = callback;
        self.server.fns.0.permission = permission;
        if let Err(error) = result {
            log::warn!(
                "Sunrise: failed to run advancement reward function {id}: {}",
                error.message.plain()
            );
        }
    }

    // ---- running -----------------------------------------------------------------

    /// Run one function with `source`: its commands in order, until a `return`.
    pub fn run_function(&mut self, f: &Function, args: Option<&Tag>, source: Source) -> R<Outcome> {
        let failure = |key: &str, a: &[String]| {
            let args: Vec<&dyn std::fmt::Display> =
                a.iter().map(|s| s as &dyn std::fmt::Display).collect();
            err(
                "commands.function.instantiationFailure",
                &[
                    &f.id,
                    &sunrise_game_data::text::Text::tr(key, &args).plain(),
                ],
            )
        };
        let lines = f.instantiate(args).map_err(|(key, a)| failure(&key, &a))?;
        if let Err((key, a)) = self.parse_macro_lines(f, &lines) {
            return Err(failure(&key, &a));
        }
        if self.server.exec.0.functions.len() >= 128 {
            return Ok(Outcome::NoReturn);
        }
        self.server.exec.0.functions.push(f.id.clone());
        let outer = self.server.exec.0.returned.take();
        let level = std::mem::replace(
            &mut self.common.ui.registries.permission,
            self.server.fns.0.command_level(),
        );
        let inner = Source {
            silent: true,
            store: None,
            ..source
        };
        for line in lines {
            let _ = self.execute_line(&line, inner.clone());
            if self.server.exec.0.returned.is_some() {
                break;
            }
            if self.server.exec.0.commands
                > self
                    .common
                    .ui
                    .rules
                    .int_at(sunrise_game_data::gamerules::vanilla::MAX_COMMAND_SEQUENCE_LENGTH)
                    .max(1) as u32
            {
                break;
            }
        }
        self.common.ui.registries.permission = level;
        let returned = std::mem::replace(&mut self.server.exec.0.returned, outer);
        self.server.exec.0.functions.pop();
        Ok(match returned {
            None => Outcome::NoReturn,
            Some(Some(v)) => Outcome::Value(v),
            Some(None) => Outcome::Fail,
        })
    }
    /// `MacroFunction.MacroEntry.instantiate`: a `$` line is parsed only after
    /// substitution, at the function compilation permission
    /// (`CommandFunction.fromLines`' `compilationContext`), and a line that
    /// does not parse fails the whole call before any of its lines run.
    fn parse_macro_lines(
        &mut self,
        f: &Function,
        lines: &[String],
    ) -> Result<(), (String, Vec<String>)> {
        if !f.is_macro() {
            return Ok(());
        }
        let level = std::mem::replace(
            &mut self.common.ui.registries.permission,
            self.server.fns.0.command_level(),
        );
        let d = self.function_dispatcher();
        let mut out = Ok(());
        for (line, entry) in lines.iter().zip(&f.lines) {
            if matches!(entry, datapack::FunctionLine::Macro(_))
                && let Err(error) = d.parse(line, &self.common.ui.registries).executable()
            {
                out = Err((
                    "commands.function.error.parse".into(),
                    vec![f.id.clone(), line.clone(), error.message.plain()],
                ));
                break;
            }
        }
        self.common.ui.registries.permission = level;
        out
    }
    /// The tree every function line is checked against, a plain line at load and a macro line
    /// after substitution: the session's (live rules, enabled features and the mods' commands,
    /// registered before functions load), as the server's own dispatcher compiles functions.
    fn function_dispatcher(&self) -> Arc<sunrise_command::Dispatcher> {
        self.common.ui.dispatcher.0.clone()
    }
    /// The functions a `FunctionArgument` names: a function or a tag.
    pub(in crate::game::server_side) fn functions_named(&mut self, name: &str) -> R<Vec<Function>> {
        match name.strip_prefix('#') {
            // ServerFunctionLibrary.getTag: an unknown tag is empty.
            Some(tag) => {
                if !self.server.fns.0.store.function_tags.contains_key(tag) {
                    return Ok(Vec::new());
                }
                Ok(self
                    .server
                    .fns
                    .0
                    .store
                    .tag_functions(tag)
                    .iter()
                    .filter_map(|id| self.server.fns.0.store.functions.get(id).cloned())
                    .collect())
            }
            None => self
                .server
                .fns
                .0
                .store
                .functions
                .get(name)
                .cloned()
                .map(|f| vec![f])
                .ok_or_else(|| err("arguments.function.unknown", &[&name])),
        }
    }
    /// `if function`: what each function returns (None if it did not).
    pub(in crate::game::server_side) fn call_function_ref(
        &mut self,
        name: &str,
        args: Option<&Tag>,
    ) -> R<Vec<Option<i32>>> {
        let functions = self.functions_named(name)?;
        let source = self
            .server
            .command_source
            .clone()
            .unwrap_or_else(|| self.player_source());
        let mut out = Vec::new();
        for f in functions {
            out.push(match self.run_function(&f, args, source.clone())? {
                Outcome::NoReturn => None,
                Outcome::Value(v) => Some(v),
                Outcome::Fail => Some(0),
            });
        }
        Ok(out)
    }

    // ---- commands ------------------------------------------------------------------

    /// `FunctionCommand.decorateOutputIfNeeded`: consume only this invocation's
    /// prepared call before its body can invoke another function.
    pub(in crate::game::server_side) fn cmd_function(&mut self, ctx: &Context) -> R<i32> {
        let source = self
            .server
            .command_source
            .clone()
            .unwrap_or_else(|| self.player_source());
        let call = if self
            .server
            .exec
            .0
            .prepared_function
            .as_ref()
            .is_some_and(|p| p.matches(ctx, &source))
        {
            std::sync::Arc::unwrap_or_clone(
                self.server.exec.0.prepared_function.take().unwrap().call,
            )
        } else {
            let Some(call) = self.prepare_function_call(ctx)? else {
                return Ok(0);
            };
            self.start_function_call(&call.functions);
            call
        };
        self.execute_function_call(call)
    }
    /// `FunctionCommand.getArgumentTag`: retain the existing resolution and argument validation.
    fn prepare_function_call(&mut self, ctx: &Context) -> R<Option<FunctionCall>> {
        let Some(Arg::Resource(name)) = ctx.get("name").cloned() else {
            return Ok(None);
        };
        let functions = self.functions_named(&name)?;
        if functions.is_empty() {
            let id = name.trim_start_matches('#');
            return Err(err("commands.function.scheduled.no_functions", &[&id]));
        }
        let args = if ctx.has("with") {
            let r = self.data_ref_named(ctx, 0, "source")?;
            let data = self.data_of(&r)?;
            let tag = match ctx.get("path") {
                Some(Arg::Str(p)) => {
                    let path = sunrise_game_data::nbt_path::NbtPath::parse(p)
                        .map_err(sunrise_sim::data_cmd::path_error)?;
                    sunrise_sim::data_cmd::single_tag(&path, &data)?
                }
                _ => data,
            };
            if !matches!(tag, Tag::Compound(_)) {
                // FunctionCommand.getArgumentTag uses TagType.getName, not the tag's value.
                let names = [
                    "END", "BYTE", "SHORT", "INT", "LONG", "FLOAT", "DOUBLE", "BYTE[]", "STRING",
                    "LIST", "COMPOUND", "INT[]", "LONG[]",
                ];
                return Err(err(
                    "commands.function.error.argument_not_compound",
                    &[&names[usize::from(tag.id())]],
                ));
            }
            Some(tag)
        } else if let Some(Arg::Str(text)) = ctx.get("arguments") {
            Some(
                sunrise_core::nbt::parse_snbt(text)
                    .map_err(|e| err("argument.nbt.invalid", &[&e]))?
                    .0,
            )
        } else {
            None
        };
        Ok(Some(FunctionCall { functions, args }))
    }
    /// `FunctionCommand.FunctionCustomExecutor.runGuarded`: send the existing start reply.
    fn start_function_call(&mut self, functions: &[Function]) {
        let message = if functions.len() == 1 {
            Text::translate(
                "commands.function.scheduled.single",
                &[Text::literal(functions[0].id.clone())],
            )
        } else {
            Text::translate(
                "commands.function.scheduled.multiple",
                &[sunrise_sim::scoreboard_cmd::format_list(
                    functions
                        .iter()
                        .map(|f| Text::literal(f.id.clone()))
                        .collect(),
                )],
            )
        };
        self.feedback(message);
    }
    /// `FunctionCommand.decorateOutputIfNeeded`: retain body, result and callback delivery.
    fn execute_function_call(&mut self, call: FunctionCall) -> R<i32> {
        let FunctionCall { functions, args } = call;
        let source = self
            .server
            .command_source
            .clone()
            .unwrap_or_else(|| self.player_source());
        let mut sum: i32 = 0;
        let mut any = false;
        let mut failed = false;
        for f in functions {
            match self.run_function(&f, args.as_ref(), source.clone())? {
                Outcome::NoReturn => {}
                Outcome::Value(v) => {
                    any = true;
                    sum = sum.wrapping_add(v);
                    let message = Text::translate(
                        "commands.function.result",
                        &[Text::literal(f.id.clone()), Text::literal(v.to_string())],
                    );
                    self.feedback(message);
                }
                Outcome::Fail => {
                    any = true;
                    failed = true;
                    self.feedback(Text::translate(
                        "commands.function.result",
                        &[Text::literal(f.id.clone()), Text::literal("0")],
                    ));
                }
            }
        }
        self.server.exec.0.callback = Some(if failed && sum == 0 {
            Some((false, 0))
        } else if any {
            Some((true, sum))
        } else {
            None
        });
        Ok(sum)
    }
    /// `FunctionCommand.queueFunctionsNoReturn`: starts are emitted while the
    /// eligible direct plain-function calls are prepared, before their bodies.
    pub(in crate::game::server_side) fn prepare_function_batch(
        &mut self,
        ctx: &Context,
        sources: &[Source],
    ) -> Option<Vec<PreparedFunctionCall>> {
        if !self.server.fns.0.loaded
            || sources.len() < 2
            || !self.server.exec.0.functions.is_empty()
            || self.server.exec.0.prepared_function.is_some()
            || sources.iter().any(|s| s.silent)
            || ctx.has("with")
            || ctx.get("arguments").is_some()
        {
            return None;
        }
        let Some(Arg::Resource(name)) = ctx.get("name") else {
            return None;
        };
        if name.starts_with('#') || self.server.fns.0.store.functions.get(name)?.is_macro() {
            return None;
        }
        let limit = self
            .common
            .ui
            .rules
            .int_at(sunrise_game_data::gamerules::vanilla::MAX_COMMAND_SEQUENCE_LENGTH)
            .max(1) as u32;
        if sources.len() > limit.saturating_sub(self.server.exec.0.commands) as usize {
            return None;
        }
        // No start or slot is published until the existing validation succeeds.
        let call = self
            .with_source(sources[0].clone(), |g| g.prepare_function_call(ctx))
            .ok()??;
        let call = std::sync::Arc::new(call);
        let tasks: Vec<_> = sources
            .iter()
            .map(|source| PreparedFunctionCall {
                invocation: ctx as *const Context as usize,
                source: source.clone(),
                call: call.clone(),
            })
            .collect();
        for task in &tasks {
            self.with_source(task.source.clone(), |g| {
                g.start_function_call(&task.call.functions)
            });
        }
        Some(tasks)
    }
    /// `return <value>` and `return fail` (`return run` is a chain).
    pub(in crate::game::server_side) fn cmd_return(&mut self, id: &str, ctx: &Context) -> R<i32> {
        if id == "return.fail" {
            self.finish_return(None);
            self.server.exec.0.callback = Some(Some((false, 0)));
            return Ok(0);
        }
        let value = match ctx.get("value") {
            Some(Arg::Int(v)) => *v,
            _ => 0,
        };
        self.finish_return(Some(value));
        Ok(value)
    }
    pub(in crate::game::server_side) fn cmd_schedule(&mut self, id: &str, ctx: &Context) -> R<i32> {
        if id == "schedule.clear" {
            let Some(Arg::Resource(name)) = ctx.get("function").cloned() else {
                return Ok(0);
            };
            let before = self.server.fns.0.schedule.len();
            self.server.fns.0.schedule.retain(|e| e.id != name);
            let count = before - self.server.fns.0.schedule.len();
            if count == 0 {
                return Err(err("commands.schedule.cleared.failure", &[&name]));
            }
            self.feedback(Text::translate(
                "commands.schedule.cleared.success",
                &[Text::literal(count.to_string()), Text::literal(name)],
            ));
            self.sync_schedule_registry();
            return Ok(count as i32);
        }
        let Some(Arg::Resource(name)) = ctx.get("function").cloned() else {
            return Ok(0);
        };
        let time = match ctx.get("time") {
            Some(Arg::Int(t)) => *t,
            _ => 0,
        };
        if time == 0 {
            return Err(err("commands.schedule.same_tick", &[]));
        }
        let replace = !ctx.has("append");
        let tick_time = self.common.world.game_time + i64::from(time);
        let schedule_id = match name.strip_prefix('#') {
            // An unknown tag is empty, and scheduling it is allowed.
            Some(_) => name.clone(),
            None => {
                let Some(f) = self.server.fns.0.store.functions.get(&name) else {
                    return Err(err("arguments.function.unknown", &[&name]));
                };
                if f.is_macro() {
                    return Err(err("commands.schedule.macro", &[]));
                }
                name.clone()
            }
        };
        if replace {
            self.server.fns.0.schedule.retain(|e| e.id != schedule_id);
        }
        if !self
            .server
            .fns
            .0
            .schedule
            .iter()
            .any(|e| e.id == schedule_id && e.time == tick_time)
        {
            self.server.fns.0.next_seq += 1;
            let seq = self.server.fns.0.next_seq;
            self.server.fns.0.schedule.push(Scheduled {
                id: schedule_id,
                time: tick_time,
                seq,
            });
        }
        let key = if name.starts_with('#') {
            "commands.schedule.created.tag"
        } else {
            "commands.schedule.created.function"
        };
        self.feedback(Text::translate(
            key,
            &[
                Text::literal(name.trim_start_matches('#').to_owned()),
                Text::literal(time.to_string()),
                Text::literal(tick_time.to_string()),
            ],
        ));
        self.sync_schedule_registry();
        Ok((tick_time.rem_euclid(i64::from(i32::MAX))) as i32)
    }
    /// ReloadCommand: the message, then the (asynchronous) reload; returns 0.
    pub fn cmd_reload(&mut self) -> R<i32> {
        self.feedback(Text::translate("commands.reload.success", &[]));
        self.reload_datapacks(true, &self.common.level.enabled_features.clone())
            .map_err(Self::reload_failure)?;
        Ok(0)
    }
    /// `PackLocationInfo.createChatLink`: the id decorated with its source
    /// (`pack.nameAndSource`, grey) in brackets, green when enabled.
    fn pack_link(&self, id: &str, enabled: bool) -> Text {
        let hint = if enabled { "disable" } else { "enable" };
        let style = text::Style {
            color: Some(if enabled { text::GREEN } else { text::RED }),
            click: Some(text::ClickEvent::SuggestCommand(format!(
                "/datapack {hint} \"{id}\""
            ))),
            ..text::Style::default()
        };
        let description = self
            .server
            .fns
            .0
            .packs
            .iter()
            .find(|p| p.id == id)
            .map_or_else(|| Text::literal("Default data pack"), |p| p.description());
        let style = text::Style {
            hover: Some(std::sync::Arc::new(
                Text::literal(format!("{id}\n")).append(description),
            )),
            ..style
        };
        let source = if id == "vanilla" {
            "pack.source.builtin"
        } else if self
            .server
            .fns
            .0
            .packs
            .iter()
            .any(|pack| pack.id == id && pack.is_feature())
        {
            "pack.source.feature"
        } else if self
            .server
            .fns
            .0
            .packs
            .iter()
            .find(|pack| pack.id == id)
            .and_then(|pack| pack.selection.as_ref())
            .is_some_and(|info| info.source == sunrise_level::pack_meta::PackSource::BuiltIn)
        {
            "pack.source.builtin"
        } else {
            "pack.source.world"
        };
        let name = Text::translate(
            "pack.nameAndSource",
            &[Text::literal(id), Text::translate(source, &[])],
        )
        .with_color(text::GRAY);
        bracketed(name).with_style(&style)
    }
    /// The selected packs, lowest priority first.
    fn selected_packs(&self) -> Vec<String> {
        let vanilla = (!self.server.cmds.0.vanilla_disabled).then(|| "vanilla".to_owned());
        vanilla
            .into_iter()
            .chain(self.server.fns.0.enabled.iter().cloned())
            .collect()
    }
    /// `FileUtil.isPathPartPortable`: no trailing dot and no Windows device
    /// name (CON, PRN, AUX, NUL, COM and LPT with a digit from 1 to 9).
    fn is_portable_name(name: &str) -> bool {
        let stem = name.split('.').next().unwrap_or_default();
        let device = match stem.len() {
            3 => matches!(stem, "com" | "con" | "prn" | "aux" | "nul"),
            4 => matches!(&stem[..3], "com" | "lpt") && matches!(stem.as_bytes()[3], b'1'..=b'9'),
            _ => false,
        };
        !device && !name.ends_with('.')
    }
    /// `DataPackCommand.createPack`: a pack folder with its `data` folder and a
    /// `pack.mcmeta` in the world's datapacks folder.
    fn create_pack(&mut self, ctx: &Context) -> R<i32> {
        let id = match ctx.get("id") {
            Some(Arg::Str(s)) => s.clone(),
            _ => String::new(),
        };
        let description = self.component_arg(ctx, "description")?;
        // FileUtil.isValidPathSegment: lowercase letters, digits, '-', '.' and '_'.
        let valid = !id.is_empty()
            && id != "."
            && id != ".."
            && id
                .chars()
                .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '-' | '.' | '_'));
        if !valid {
            return Err(err("commands.datapack.create.invalid_name", &[&id]));
        }
        if !Self::is_portable_name(&id) {
            return Err(err("commands.datapack.create.invalid_full_name", &[&id]));
        }
        // Without a world there is no datapacks folder to write to.
        if self.server.world_root.as_os_str().is_empty() {
            return Err(err("commands.datapack.create.io_failure", &[&id]));
        }
        let datapacks = self.server.world_root.join("datapacks");
        let pack = datapacks.join(&id);
        if pack.exists() {
            return Err(err("commands.datapack.create.already_exists", &[&id]));
        }
        // The data pack version (SharedConstants), written as a bare major
        // number for both bounds, as vanilla does.
        let pack_format = sunrise_core::version::DATA_PACK_FORMAT.0;
        let mcmeta = serde_json::json!({
            "pack": {
                "description": description.component(),
                "min_format": pack_format,
                "max_format": pack_format,
            }
        });
        let written = std::fs::create_dir_all(&datapacks)
            .and_then(|()| std::fs::create_dir(&pack))
            .and_then(|()| std::fs::create_dir(pack.join("data")))
            .and_then(|()| {
                std::fs::write(
                    pack.join("pack.mcmeta"),
                    serde_json::to_string_pretty(&mcmeta).unwrap_or_default(),
                )
            });
        if written.is_err() {
            return Err(err("commands.datapack.create.io_failure", &[&id]));
        }
        self.feedback(Text::translate(
            "commands.datapack.create.success",
            &[Text::literal(id)],
        ));
        Ok(1)
    }
    fn missing_pack_features(&self, id: &str) -> Vec<String> {
        self.server
            .fns
            .0
            .packs
            .iter()
            .find(|pack| pack.id == id)
            .map(|pack| {
                pack.features()
                    .into_iter()
                    .filter(|flag| !self.common.level.enabled_features.contains(flag))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Rescan the repository, retaining the selection of packs still present.
    fn reload_pack_repository(&mut self) {
        let additional =
            sunrise_gameplay_common::game::pack_discovery::data_packs(&self.common.mods);
        let packs = sunrise_gameplay_common::game::pack_discovery::data_repository(
            &self.server.world_root,
            &self.common.options.resources,
            &additional,
        );
        self.server.fns.0.packs = packs.into_iter().map(Arc::new).collect();
        self.server.fns.0.enabled = datapack::selected_ids(
            self.server.fns.0.packs.iter().map(Arc::as_ref),
            &self.server.fns.0.enabled,
        );
    }
    /// Check the pack's availability, selection and feature flags for this operation.
    fn get_pack(&self, name: &str, enabling: bool, selected: &[String]) -> R<()> {
        if name != "vanilla" && !self.server.fns.0.packs.iter().any(|p| p.id == name) {
            return Err(err("commands.datapack.unknown", &[&name]));
        }
        let on = selected.iter().any(|p| p == name);
        if enabling && on {
            return Err(err("commands.datapack.enable.failed", &[&name]));
        }
        if !enabling && !on {
            return Err(err("commands.datapack.disable.failed", &[&name]));
        }
        if !enabling
            && self
                .server
                .fns
                .0
                .packs
                .iter()
                .any(|pack| pack.id == name && pack.is_feature() && !pack.features().is_empty())
        {
            // An enabled feature flag stays on for the world's lifetime.
            return Err(err("commands.datapack.disable.failed.feature", &[&name]));
        }
        let missing = self.missing_pack_features(name);
        if !missing.is_empty() {
            return Err(err(
                "commands.datapack.enable.failed.no_flags",
                &[&name, &missing.join(", ")],
            ));
        }
        Ok(())
    }
    pub(in crate::game::server_side) fn cmd_datapack(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let name = match ctx.get("name") {
            Some(Arg::Str(n)) => Some(n.clone()),
            _ => None,
        };
        if id == "datapack.list" {
            self.reload_pack_repository();
        }
        let selected = self.selected_packs();
        let mut enabled = self.server.fns.0.enabled.clone();
        // `DataPackCommand.listAvailablePacks`: a feature pack the world's
        // features do not cover is not available.
        let mut available: Vec<String> = std::iter::once("vanilla".to_owned())
            .chain(self.server.fns.0.packs.iter().map(|p| p.id.clone()))
            .filter(|p| !selected.contains(p))
            .filter(|p| self.missing_pack_features(p).is_empty())
            .collect();
        available.sort_by(|a, b| datapack::compare_ids(a, b));
        match id {
            "datapack.create" => self.create_pack(ctx),
            "datapack.list" => {
                let want_enabled = ctx.has("enabled") || (!ctx.has("available"));
                let want_available = ctx.has("available") || (!ctx.has("enabled"));
                let mut total = 0;
                if want_enabled {
                    if selected.is_empty() {
                        self.feedback(Text::translate("commands.datapack.list.enabled.none", &[]));
                    } else {
                        let links: Vec<Text> =
                            selected.iter().map(|p| self.pack_link(p, true)).collect();
                        total += links.len();
                        self.feedback(Text::translate(
                            "commands.datapack.list.enabled.success",
                            &[
                                Text::literal(links.len().to_string()),
                                sunrise_sim::scoreboard_cmd::format_list(links),
                            ],
                        ));
                    }
                }
                if want_available {
                    if available.is_empty() {
                        self.feedback(Text::translate(
                            "commands.datapack.list.available.none",
                            &[],
                        ));
                    } else {
                        let links: Vec<Text> =
                            available.iter().map(|p| self.pack_link(p, false)).collect();
                        total += links.len();
                        self.feedback(Text::translate(
                            "commands.datapack.list.available.success",
                            &[
                                Text::literal(links.len().to_string()),
                                sunrise_sim::scoreboard_cmd::format_list(links),
                            ],
                        ));
                    }
                }
                Ok(total as i32)
            }
            "datapack.enable" => {
                let name = name.unwrap_or_default();
                self.get_pack(&name, true, &selected)?;
                let vanilla_disabled = name != "vanilla" && self.server.cmds.0.vanilla_disabled;
                {
                    let existing = match ctx.get("existing") {
                        Some(Arg::Str(e)) => Some(e.clone()),
                        _ => None,
                    };
                    if let Some(e) = &existing {
                        self.get_pack(e, false, &selected)?;
                    }
                    let position = if ctx.has("first") {
                        0
                    } else if let Some(e) = &existing {
                        // The selected index includes vanilla when enabled.
                        let i = selected
                            .iter()
                            .position(|p| p == e)
                            .expect("get_pack checked that existing is selected");
                        let i = if ctx.has("before") { i } else { i + 1 };
                        // `before` vanilla is the bottom of `enabled`.
                        i.saturating_sub(selected.len() - enabled.len())
                    } else if !ctx.has("last")
                        && self
                            .server
                            .fns
                            .0
                            .packs
                            .iter()
                            .find(|pack| pack.id == name)
                            .and_then(|pack| pack.selection.as_ref())
                            .is_some_and(|info| {
                                info.position == sunrise_level::pack_meta::Position::Bottom
                            })
                    {
                        0
                    } else {
                        enabled.len()
                    };
                    if name != "vanilla" {
                        enabled.insert(position, name.clone());
                    }
                }
                self.feedback(Text::translate(
                    "commands.datapack.modify.enable",
                    &[self.pack_link(&name, true)],
                ));
                self.apply_pack_change(&enabled, vanilla_disabled)?;
                Ok(self.selected_packs().len() as i32)
            }
            _ => {
                let name = name.unwrap_or_default();
                self.get_pack(&name, false, &selected)?;
                if self
                    .server
                    .fns
                    .0
                    .packs
                    .iter()
                    .find(|pack| pack.id == name)
                    .and_then(|pack| pack.selection.as_ref())
                    .is_some_and(|info| info.required)
                {
                    return Err(err("sunrise.datapack.required", &[&name]));
                }
                let vanilla_disabled = name == "vanilla" || self.server.cmds.0.vanilla_disabled;
                enabled.retain(|p| p != &name);
                self.feedback(Text::translate(
                    "commands.datapack.modify.disable",
                    &[self.pack_link(&name, true)],
                ));
                self.apply_pack_change(&enabled, vanilla_disabled)?;
                Ok(self.selected_packs().len() as i32)
            }
        }
    }
    /// `ReloadCommand.reloadPacks`: no message of its own. It reloads the
    /// server's data as `/reload` does (`reload_datapacks`), the content
    /// tables with the functions.
    fn apply_pack_change(&mut self, enabled: &[String], vanilla_disabled: bool) -> R<()> {
        // One resolve for the store and the tables, as `/reload`'s is: the
        // timers already in memory stay, and none come back from disk
        // (`ServerFunctionManager` keeps its queue over a pack change).
        let additional =
            sunrise_gameplay_common::game::pack_discovery::data_packs(&self.common.mods);
        let mut stack = pack_stack(
            &self.server.world_root,
            &self.common.options.resources,
            &self.common.pack.data_source,
            &self.common.pack.data_changes,
            vanilla_disabled,
            &self.common.level.enabled_features,
            &additional,
        );
        stack.enabled = datapack::selected_ids(stack.packs.iter().map(Arc::as_ref), enabled);
        let prepared = self
            .prepare_pack_condition_reload(&stack)
            .map_err(Self::reload_failure)?;
        let root = self.common.options.resources.clone();
        let context = self.common.prepare_loop_context(&root).map_err(|error| {
            Self::reload_failure(ReloadStageError::DataSource(error.to_string()))
        })?;
        let world = self.prepare_world_reload(&stack, &root).map_err(|error| {
            Self::reload_failure(ReloadStageError::DataSource(error.to_string()))
        })?;
        prepared.bind_loot(&world);
        // `MinecraftServer.reloadResources`: publish the selection only after preparation succeeds.
        self.install_loop_context(context);
        self.server.fns.0.packs = stack.packs.clone();
        self.server.fns.0.enabled = stack.enabled.clone();
        self.server.cmds.0.vanilla_disabled = vanilla_disabled;
        self.common.pack.additional_data = additional.into();
        self.save_datapack_settings();
        self.server.fns.0.load_pending = true;
        self.install_world_tables(world);
        self.apply_server_reload(prepared, &self.common.level.enabled_features.clone());
        self.run_native_advancement_grants();
        Ok(())
    }

    fn reload_failure(error: ReloadStageError) -> sunrise_command::CommandError {
        let diagnostic = error.diagnostic();
        log::warn!("Sunrise: {diagnostic}");
        sunrise_command::CommandError::new(
            Text::translate("commands.reload.failure", &[])
                .append(Text::literal(format!("\n{diagnostic}"))),
        )
    }

    // ---- predicates -------------------------------------------------------------------

    /// `ResourceOrIdArgument.getLootPredicate`: accept the inline or named
    /// predicate, then test it with the existing source entity and origin.
    pub fn check_predicate(&mut self, id: &str) -> R<bool> {
        self.ensure_datapacks();
        // ResourceOrIdArgument retains either an inline value or a named id.
        // Both use the same evaluator and command-source context below.
        let json = if id.starts_with('{') {
            let (tag, _) = sunrise_core::nbt::parse_snbt(id)
                .map_err(|e| err("argument.nbt.invalid", &[&e]))?;
            let value = sunrise_game_data::scoreboard::tag_to_json(&tag);
            sunrise_world::loot::validate_typed_loot_condition(&value)
                .map_err(|e| err("argument.nbt.invalid", &[&e]))?;
            value
        } else {
            self.server
                .fns
                .0
                .store
                .predicates
                .get(id)
                .cloned()
                .ok_or_else(|| {
                    let id = if id.contains(':') {
                        id.to_string()
                    } else {
                        format!("minecraft:{id}")
                    };
                    err(
                        "argument.resource_or_id.no_such_element",
                        &[&id, &"minecraft:predicate"],
                    )
                })?
        };
        let source = self
            .server
            .command_source
            .clone()
            .unwrap_or_else(|| self.player_source());
        let view = source.entity.map(|t| self.entity_view_scored(t, &json));
        Ok(self.command_conditions_hold(&json, view.as_ref(), source.pos))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn tick_functions_write_to_the_overworld_before_its_idle_environment_tick() {
        sunrise_gameplay_common::game::in_own_process(
            "game::server_side::functions::tests::tick_functions_write_to_the_overworld_before_its_idle_environment_tick",
            || {
                crate::game::native_entity_refs::tests::with_game(|game| {
                    use sunrise_gameplay_common::game::Dim;
                    game.server.world_root = game.options.game_dir.clone().unwrap();
                    game.build_registries();
                    game.enter_level(Dim::Nether).unwrap();
                    game.server.streaming.forced.insert((0, 0));
                    assert_eq!(game.levels_to_tick(), [Dim::Nether]);
                    assert_eq!(
                        game.server
                            .levels
                            .0
                            .parked_world(Dim::Overworld)
                            .unwrap()
                            .weather
                            .rain_level
                            .1,
                        0.0
                    );
                    let id = "test:change_environment";
                    game.server.fns.0.loaded = true;
                    game.server.fns.0.store.functions.insert(
                        id.into(),
                        Function {
                            id: id.into(),
                            lines: [
                                "execute in minecraft:overworld run worldborder set 20 5t",
                                "execute in minecraft:overworld run weather rain 40t",
                            ]
                            .into_iter()
                            .map(|line| datapack::FunctionLine::Command(line.into()))
                            .collect(),
                            parameters: Vec::new(),
                        },
                    );
                    game.server
                        .fns
                        .0
                        .store
                        .function_tags
                        .insert("minecraft:tick".into(), vec![(id.into(), true, false)]);

                    game.tick_server(false).unwrap();

                    assert_eq!(game.levels_to_tick(), [Dim::Nether]);
                    assert_eq!(game.dim, Dim::Nether);
                    let border = game.server.levels.0.parked_border(Dim::Overworld).unwrap();
                    assert_eq!((border.target_size(), border.remaining_ticks()), (20.0, 4));
                    let world = game.server.levels.0.parked_world(Dim::Overworld).unwrap();
                    assert!(world.chunks.is_empty());
                    assert_eq!(world.weather.rain_level, (0.0, 0.01));
                    assert_eq!(game.world.weather.rain_time, 39);
                });
            },
        );
    }

    fn datapack_world(label: &str, names: &[&str]) -> std::path::PathBuf {
        let root = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for name in names {
            add_datapack(&root, name);
        }
        std::fs::create_dir_all(root.join("datapacks")).unwrap();
        root
    }
    fn add_datapack(root: &std::path::Path, name: &str) {
        let pack = root.join("datapacks").join(name);
        std::fs::create_dir_all(pack.join("data")).unwrap();
        std::fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"Fixture"}}"#,
        )
        .unwrap();
    }
    fn open_datapack_world(game: &mut crate::game::ServerGame<'_>, root: &std::path::Path) {
        let mut request = crate::game::world_loading::WorldRequest::new(
            "test",
            Some("test"),
            Some(1),
            true,
            None,
            std::sync::Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla()),
        )
        .unwrap();
        request.save_root = root.to_path_buf();
        game.load_world(request).unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn datapack_list_scans_the_folder_again_so_a_copied_pack_enables() {
        use crate::game::native_entity_refs::tests::with_game;
        let root = datapack_world("datapack-rescan", &["a"]);
        with_game(|game| {
            open_datapack_world(game, &root);
            add_datapack(&root, "b");
            // `getPack` reads the repository as it stands: not yet scanned.
            assert!(!game.run_command("datapack enable \"file/b\""));
            assert!(game.run_command("datapack list available"));
            assert!(
                game.server
                    .fns
                    .0
                    .packs
                    .iter()
                    .any(|pack| pack.id == "file/b")
            );
            assert!(game.run_command("datapack enable \"file/b\""));
            assert_eq!(
                game.server.fns.0.enabled.last().map(String::as_str),
                Some("file/b")
            );
        });
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[ignore = "daily"]
    fn datapack_disable_vanilla_survives_a_reopen() {
        use crate::game::native_entity_refs::tests::with_game;
        let root = datapack_world("datapack-vanilla-off", &["a"]);
        with_game(|game| {
            open_datapack_world(game, &root);
            assert!(game.run_command("datapack disable vanilla"));
            assert!(game.server.cmds.0.vanilla_disabled);
        });
        with_game(|game| {
            assert!(!game.server.cmds.0.vanilla_disabled);
            open_datapack_world(game, &root);
            assert!(game.server.cmds.0.vanilla_disabled);
            assert!(!game.selected_packs().contains(&"vanilla".to_owned()));
        });
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[ignore = "daily"]
    fn datapack_enable_next_to_vanilla_and_the_existing_pack_is_checked_for_vanilla() {
        use crate::game::native_entity_refs::tests::with_game;
        let root = datapack_world("datapack-vanilla-neighbour", &["a", "b"]);
        std::fs::write(
            root.join("datapacks.json"),
            r#"{"enabled":["file/a"],"disabled":["file/b"]}"#,
        )
        .unwrap();
        with_game(|game| {
            open_datapack_world(game, &root);
            assert!(game.run_command("datapack enable \"file/b\" after \"vanilla\""));
            assert_eq!(game.server.fns.0.enabled, ["file/b", "file/a"]);
            assert!(game.run_command("datapack disable \"file/b\""));
            assert!(game.run_command("datapack enable \"file/b\" before \"vanilla\""));
            assert_eq!(game.server.fns.0.enabled, ["file/b", "file/a"]);
            // `getPack(existing, false)` runs whatever the name is.
            assert!(game.run_command("datapack disable vanilla"));
            assert!(!game.run_command("datapack enable vanilla after \"nosuchpack\""));
            assert!(game.server.cmds.0.vanilla_disabled);
            assert!(!game.run_command("datapack enable vanilla after \"vanilla\""));
            assert!(game.run_command("datapack enable vanilla after \"file/a\""));
            assert!(!game.server.cmds.0.vanilla_disabled);
        });
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[ignore = "daily"]
    fn datapack_enable_next_to_a_flagged_feature_pack_is_refused() {
        use crate::game::native_entity_refs::tests::with_game;
        let root = datapack_world("datapack-feature-neighbour", &["a", "b"]);
        std::fs::write(
            root.join("datapacks.json"),
            r#"{"enabled":["file/a"],"disabled":["file/b"]}"#,
        )
        .unwrap();
        with_game(|game| {
            open_datapack_world(game, &root);
            game.level.enabled_features = vec![
                "minecraft:vanilla".into(),
                "minecraft:trade_rebalance".into(),
            ]
            .into();
            game.reload_datapacks(true, &game.level.enabled_features.clone())
                .unwrap();
            assert!(
                game.server
                    .fns
                    .0
                    .enabled
                    .contains(&"trade_rebalance".to_owned())
            );
            assert!(!game.run_command("datapack enable \"file/b\" after \"trade_rebalance\""));
            assert!(!game.run_command("datapack enable \"file/b\" before \"trade_rebalance\""));
            assert!(!game.server.fns.0.enabled.contains(&"file/b".to_owned()));
            assert!(game.run_command("datapack enable \"file/b\" after \"file/a\""));
            assert!(game.server.fns.0.enabled.contains(&"file/b".to_owned()));
        });
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn reload_failure_formats_nested_listener_error_source_chain() {
        #[derive(Debug)]
        struct Inner;
        impl std::fmt::Display for Inner {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("inner cause")
            }
        }
        impl std::error::Error for Inner {}

        #[derive(Debug)]
        struct Middle(Inner);
        impl std::fmt::Display for Middle {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("middle cause")
            }
        }
        impl std::error::Error for Middle {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }

        #[derive(Debug)]
        struct Outer(Middle);
        impl std::fmt::Display for Outer {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("outer cause")
            }
        }
        impl std::error::Error for Outer {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }

        let source = mod_api::ReloadResourceError::new(
            mod_api::Identifier::parse("test_mod:machine").unwrap(),
            Outer(Middle(Inner)),
        );
        let reply = crate::game::ServerGame::reload_failure(ReloadStageError::Listener {
            listener: mod_api::Identifier::parse("test_mod:listener").unwrap(),
            source,
        })
        .message
        .plain();
        assert!(
            reply.contains(
                "reload listener test_mod:listener: test_mod:machine: outer cause: middle cause: inner cause"
            ),
            "{reply}"
        );
    }

    #[test]
    #[ignore = "daily"]
    fn prepared_function_calls_keep_causality_and_fallback_boundaries() {
        use crate::game::native_entity_refs::tests::with_game;
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::motion::PlayerMotion;
        with_game(|game| {
            game.options.dedicated = None;
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 202.0, 4.5)).unwrap());
            game.creative = true;
            // Supply the synthetic chunk before forceload's synchronous load barrier.
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let scratch = tempfile::tempdir().unwrap();
            game.server.world_root = scratch.path().join("h155-causal-world");
            let pack = game.server.world_root.join("datapacks/h155_causal");
            let dir = pack.join("data/h155c/function");
            std::fs::create_dir_all(&dir).unwrap();
            let f = sunrise_core::version::DATA_PACK_FORMAT;
            std::fs::write(pack.join("pack.mcmeta"), serde_json::json!({"pack":{"description":"H155 causal controls","pack_format":f.0,"min_format":[f.0,f.1],"max_format":[f.0,f.1]}}).to_string()).unwrap();
            let body = "execute if score @s h155c_id matches 1 run scoreboard players operation #seen_a h155c = #r h155c\nexecute if score @s h155c_id matches 2 run scoreboard players operation #seen_b h155c = #r h155c\nscoreboard players add #calls h155c 1\nscoreboard players operation #trace h155c *= #ten h155c\nscoreboard players operation #trace h155c += @s h155c_id\n";
            for (name, tail) in [
                ("zero", "return 0\n"),
                ("five", "return 5\n"),
                ("fail", "return fail\n"),
                ("none", ""),
            ] {
                std::fs::write(
                    dir.join(format!("{name}.mcfunction")),
                    format!("{body}{tail}"),
                )
                .unwrap();
            }
            for (name, text) in [
                (
                    "early",
                    "scoreboard players add #early h155c 1\nreturn 0\nscoreboard players add #late h155c 1\n",
                ),
                ("inner", "scoreboard players add #inner h155c 1\nreturn 0\n"),
                (
                    "outer",
                    "function h155c:inner\nscoreboard players add #outer h155c 1\nreturn 5\n",
                ),
                (
                    "nested",
                    "execute as @e[type=pig,tag=h155c,sort=nearest,limit=2] run function h155c:inner\nreturn 0\n",
                ),
                (
                    "macro",
                    "$scoreboard players add #macro h155c $(amount)\nreturn 0\n",
                ),
            ] {
                std::fs::write(dir.join(format!("{name}.mcfunction")), text).unwrap();
            }
            std::fs::create_dir_all(pack.join("data/h155c/tags/function")).unwrap();
            std::fs::write(
                pack.join("data/h155c/tags/function/pair.json"),
                r#"{"values":["h155c:zero","h155c:five"]}"#,
            )
            .unwrap();
            game.reload_datapacks(false, &game.level.enabled_features.clone())
                .unwrap();
            game.build_registries();
            game.set_allow_commands(true);
            assert_eq!(game.server.fns.0.store.tag_functions("h155c:pair").len(), 2);
            assert!(
                game.server
                    .fns
                    .0
                    .store
                    .functions
                    .get("h155c:macro")
                    .unwrap()
                    .is_macro()
            );
            assert!(game.run_command("forceload add 0 0 15 15"));
            assert!(game.world.loaded(4, 4));
            assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
            for (n, x) in [(1, 4.5), (2, 8.5)] {
                assert!(game.run_command(&format!("summon pig {x} 200 4.5 {{UUID:[I;155,2,0,{n}],Tags:[\"h155c\"],NoAI:1b,NoGravity:1b,Health:10f}}")));
                assert!(game.run_command(&format!("execute if entity @e[type=pig,nbt={{UUID:[I;155,2,0,{n}],Health:10f,NoAI:1b,NoGravity:1b,Pos:[{x}d,200d,4.5d]}}]")));
            }
            assert!(game.run_command("scoreboard objectives add h155c dummy"));
            assert!(game.run_command("scoreboard objectives add h155c_id dummy"));
            for (holder, id) in [
                ("@s".to_owned(), 3),
                ("@e[nbt={UUID:[I;155,2,0,1]},limit=1]".to_owned(), 1),
                ("@e[nbt={UUID:[I;155,2,0,2]},limit=1]".to_owned(), 2),
            ] {
                assert!(
                    game.run_command(&format!("scoreboard players set {holder} h155c_id {id}"))
                );
            }
            assert!(game.run_command("scoreboard players set #ten h155c 10"));
            fn score(g: &crate::game::ServerGame<'_>, h: &str) -> i32 {
                g.score.board.score(h, "h155c").unwrap().value
            }
            fn reset(g: &mut crate::game::ServerGame<'_>) {
                for h in [
                    "#calls", "#trace", "#seen_a", "#seen_b", "#early", "#late", "#inner",
                    "#outer", "#macro",
                ] {
                    assert!(g.run_command(&format!("scoreboard players set {h} h155c 0")));
                }
                assert!(g.run_command("scoreboard players set #r h155c 91"));
                assert!(g.run_command("scoreboard players set #s h155c 17"));
            }
            fn capture(
                g: &mut crate::game::ServerGame<'_>,
                line: &str,
            ) -> (Vec<String>, Vec<(bool, i32)>) {
                g.server.chat_capture = Some(Vec::new());
                g.server.cmds.0.results = Some(Vec::new());
                assert!(g.run_command(line));
                let texts = g.server.chat_capture.take().unwrap();
                let callbacks = g.server.cmds.0.results.take().unwrap();
                println!("causal raw {line}: {texts:?} callbacks={callbacks:?}");
                assert!(
                    g.server.exec.0.prepared_function.is_none(),
                    "no prepared call after command/error/return"
                );
                (
                    texts
                        .iter()
                        .map(|m| m.translation_key().unwrap_or("literal").to_owned())
                        .collect(),
                    callbacks,
                )
            }
            let start = "commands.function.scheduled.single".to_owned();
            let result = "commands.function.result".to_owned();
            let prefix = "execute store result score #r h155c store success score #s h155c as @e[type=pig,tag=h155c,sort=nearest,limit=2] run ";
            let clock = game.world.game_time;
            let chunks = game.world.chunks.len();
            for (name, value, success) in [
                ("zero", Some(0), true),
                ("five", Some(5), true),
                ("fail", Some(0), false),
                ("none", None, false),
            ] {
                reset(game);
                let (keys, callbacks) = capture(game, &format!("{prefix}function h155c:{name}"));
                let mut expected = vec![start.clone(); 2];
                if value.is_some() {
                    expected.extend(vec![result.clone(); 2]);
                }
                assert_eq!(keys, expected);
                assert_eq!(
                    callbacks,
                    value.map_or_else(Vec::new, |v| vec![(success, v); 2])
                );
                assert_eq!((score(game, "#calls"), score(game, "#trace")), (2, 12));
                assert_eq!(score(game, "#seen_a"), 91);
                assert_eq!(
                    score(game, "#seen_b"),
                    value.unwrap_or(91),
                    "first source callback/store precedes second body"
                );
                assert_eq!(score(game, "#r"), value.unwrap_or(91));
                assert_eq!(
                    score(game, "#s"),
                    if value.is_some() {
                        i32::from(success)
                    } else {
                        17
                    }
                );
            }
            // Each actual actor owns its leaf store; shared ancestors still deliver oldest first.
            reset(game);
            for n in [1, 2] {
                assert!(game.run_command(&format!(
                    "scoreboard players set @e[nbt={{UUID:[I;155,2,0,{n}]}},limit=1] h155c 91"
                )));
            }
            let (keys, callbacks) = capture(
                game,
                "execute store result score #chain h155c as @e[type=pig,tag=h155c,sort=nearest,limit=2] store success score #chain h155c store result score @s h155c run function h155c:five",
            );
            assert_eq!(
                keys,
                vec![start.clone(), start.clone(), result.clone(), result.clone()]
            );
            assert_eq!(callbacks, vec![(true, 5); 2]);
            assert_eq!(
                score(game, "#chain"),
                1,
                "ancestor result then child success"
            );
            assert_eq!((score(game, "#calls"), score(game, "#trace")), (2, 12));
            for n in [1, 2] {
                let (_, callbacks) = capture(
                    game,
                    &format!(
                        "scoreboard players get @e[nbt={{UUID:[I;155,2,0,{n}]}},limit=1] h155c"
                    ),
                );
                assert_eq!(
                    callbacks,
                    vec![(true, 5)],
                    "actual actor's own result store"
                );
            }
            // These ineligible variants preserve the legacy path, not a Java-order parity claim.
            for line in [
                "execute as @e[type=pig,tag=h155c,sort=nearest,limit=2] if entity @s run function h155c:zero",
                "execute run execute as @e[type=pig,tag=h155c,sort=nearest,limit=2] run function h155c:zero",
                "execute as @e[type=pig,tag=h155c,sort=nearest,limit=2] run function h155c:macro {amount:1}",
            ] {
                reset(game);
                let (keys, callbacks) = capture(game, line);
                assert_eq!(
                    keys,
                    vec![start.clone(), result.clone(), start.clone(), result.clone()],
                    "unchanged ineligible path"
                );
                assert_eq!(callbacks, vec![(true, 0); 2]);
            }
            reset(game);
            let (keys, callbacks) = capture(
                game,
                "execute as @e[type=pig,tag=h155c,sort=nearest,limit=2] run function #h155c:pair",
            );
            assert_eq!(
                keys,
                vec![
                    "commands.function.scheduled.multiple".to_owned(),
                    result.clone(),
                    result.clone(),
                    "commands.function.scheduled.multiple".to_owned(),
                    result.clone(),
                    result.clone()
                ]
            );
            assert_eq!(callbacks, vec![(true, 5); 2]);
            assert_eq!(score(game, "#calls"), 4);
            reset(game);
            let source = Source {
                silent: true,
                ..game.player_source()
            };
            let (keys, callbacks) = game.with_source(source, |g| {
                capture(g, &format!("{prefix}function h155c:zero"))
            });
            assert!(keys.is_empty());
            assert_eq!(callbacks, vec![(true, 0); 2]);
            assert_eq!(score(game, "#trace"), 12);
            reset(game);
            let (keys, callbacks) = capture(game, &format!("{prefix}function h155c:missing"));
            assert!(keys.is_empty());
            assert_eq!(callbacks, vec![(false, 0); 2]);
            assert_eq!(score(game, "#calls"), 0);
            for line in [
                "function h155c:five",
                "execute as @e[type=pig,tag=h155c,sort=nearest,limit=1] run function h155c:five",
            ] {
                reset(game);
                let (keys, callbacks) = capture(game, line);
                assert_eq!(keys, vec![start.clone(), result.clone()]);
                assert_eq!(callbacks, vec![(true, 5)]);
            }
            reset(game);
            let (keys, callbacks) = capture(game, &format!("{prefix}function h155c:outer"));
            assert_eq!(
                keys,
                vec![start.clone(), start.clone(), result.clone(), result.clone()]
            );
            assert_eq!(callbacks, vec![(true, 5); 2]);
            assert_eq!((score(game, "#inner"), score(game, "#outer")), (2, 2));
            reset(game);
            let (keys, callbacks) = capture(game, "function h155c:nested");
            assert_eq!(keys, vec![start.clone(), result.clone()]);
            assert_eq!(callbacks, vec![(true, 0)]);
            assert_eq!(score(game, "#inner"), 2);
            reset(game);
            let (_, callbacks) = capture(game, &format!("{prefix}function h155c:early"));
            assert_eq!(callbacks, vec![(true, 0); 2]);
            assert_eq!((score(game, "#early"), score(game, "#late")), (2, 0));
            // These are preservation/cleanup controls, not a Java limit-parity claim.
            for budget in [1, 4] {
                reset(game);
                assert!(
                    game.run_command(&format!("gamerule max_command_sequence_length {budget}"))
                );
                let (keys, callbacks) = capture(game, &format!("{prefix}function h155c:early"));
                if budget == 1 {
                    assert!(keys.is_empty() && callbacks.is_empty());
                    assert_eq!(score(game, "#early"), 0);
                } else {
                    assert_eq!(callbacks, vec![(true, 0)]);
                    assert_eq!((score(game, "#early"), score(game, "#late")), (1, 0));
                    assert_eq!(keys.iter().filter(|k| **k == start).count(), 2);
                }
                assert!(game.run_command("gamerule max_command_sequence_length 65536"));
                reset(game);
                let (keys, callbacks) = capture(game, &format!("{prefix}function h155c:five"));
                assert_eq!(
                    keys,
                    vec![start.clone(), start.clone(), result.clone(), result.clone()]
                );
                assert_eq!(callbacks, vec![(true, 5); 2]);
                assert_eq!(score(game, "#seen_b"), 5);
            }
            assert_eq!(game.world.game_time, clock);
            assert_eq!(game.world.chunks.len(), chunks);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn forked_function_replies_keep_task_order_and_actual_callbacks() {
        use crate::game::native_entity_refs::tests::with_game;
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::motion::PlayerMotion;
        with_game(|game| {
            game.options.dedicated = None;
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 202.0, 4.5)).unwrap());
            game.creative = true;
            // Supply the synthetic chunk before forceload's synchronous load barrier.
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let scratch = tempfile::tempdir().unwrap();
            game.server.world_root = scratch.path().join("h155-world");
            let pack = game.server.world_root.join("datapacks/h155_unit");
            let dir = pack.join("data/h155/function");
            std::fs::create_dir_all(&dir).unwrap();
            let format = sunrise_core::version::DATA_PACK_FORMAT;
            std::fs::write(
                pack.join("pack.mcmeta"),
                serde_json::json!({"pack": {
                    "description":"H155 actual Game regression", "pack_format":format.0,
                    "min_format":[format.0,format.1], "max_format":[format.0,format.1]
                }})
                .to_string(),
            )
            .unwrap();
            let body = "scoreboard players add #calls h155 1\nscoreboard players add @s h155 1\nscoreboard players operation #trace h155 *= #ten h155\nscoreboard players operation #trace h155 += @s h155_id\n";
            for (name, tail) in [
                ("zero", "return 0\n"),
                ("five", "return 5\n"),
                ("fail", "return fail\n"),
                ("fallthrough", ""),
            ] {
                std::fs::write(
                    dir.join(format!("{name}.mcfunction")),
                    format!("{body}{tail}"),
                )
                .unwrap();
            }
            std::fs::write(
                dir.join("admit.mcfunction"),
                "scoreboard players add #pack h155 1\nreturn 1\n",
            )
            .unwrap();
            game.reload_datapacks(false, &game.level.enabled_features.clone())
                .unwrap();
            game.build_registries();
            game.set_allow_commands(true);
            for name in ["zero", "five", "fail", "fallthrough", "admit"] {
                assert!(
                    game.server
                        .fns
                        .0
                        .store
                        .functions
                        .contains_key(&format!("h155:{name}"))
                );
                assert!(
                    game.ui
                        .registries
                        .functions
                        .contains(&format!("h155:{name}"))
                );
            }
            assert!(game.run_command("forceload add 0 0 15 15"));
            assert!(game.world.loaded(4, 4));
            assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
            for (n, x) in [(1, 4.5), (2, 8.5)] {
                assert!(game.run_command(&format!("summon pig {x} 200 4.5 {{UUID:[I;155,1,0,{n}],Tags:[\"h155u\"],NoAI:1b,NoGravity:1b,Silent:1b,PersistenceRequired:1b,Health:10f}}")));
                assert!(game.run_command(&format!("execute if entity @e[type=pig,nbt={{UUID:[I;155,1,0,{n}],Health:10f,NoAI:1b,NoGravity:1b,Pos:[{x}d,200d,4.5d]}}]")));
            }
            assert!(game.run_command("scoreboard objectives add h155 dummy"));
            assert!(game.run_command("scoreboard objectives add h155_id dummy"));
            assert!(game.run_command("scoreboard players set #ten h155 10"));
            assert!(game.run_command("scoreboard players set @s h155_id 3"));
            let actor = |n| format!("@e[nbt={{UUID:[I;155,1,0,{n}]}},limit=1]");
            for n in [1, 2] {
                assert!(
                    game.run_command(&format!("scoreboard players set {} h155_id {n}", actor(n)))
                );
            }
            assert!(game.run_command("scoreboard players set #pack h155 0"));
            assert!(game.run_command("function h155:admit"));
            assert_eq!(game.score.board.score("#pack", "h155").unwrap().value, 1);
            let clock = game.world.game_time;
            let chunks = game.world.chunks.len();
            let mut mismatches = Vec::new();
            for (name, value, success) in [
                ("zero", Some(0), true),
                ("five", Some(5), true),
                ("fail", Some(0), false),
                ("fallthrough", None, false),
            ] {
                for route in ["direct", "one", "two", "nonfork"] {
                    for holder in [
                        "#calls".to_owned(),
                        "#trace".to_owned(),
                        "@s".to_owned(),
                        actor(1),
                        actor(2),
                    ] {
                        assert!(
                            game.run_command(&format!("scoreboard players set {holder} h155 0"))
                        );
                    }
                    assert!(game.run_command("scoreboard players set #r h155 91"));
                    assert!(game.run_command("scoreboard players set #s h155 17"));
                    let middle = match route {
                        "one" => "as @e[type=pig,tag=h155u,sort=nearest,limit=1] ",
                        "two" => "as @e[type=pig,tag=h155u,sort=nearest,limit=2] ",
                        "nonfork" => "positioned 4.5 202 4.5 ",
                        _ => "",
                    };
                    game.server.chat_capture = Some(Vec::new());
                    game.server.cmds.0.results = Some(Vec::new());
                    let accepted = game.run_command(&format!("execute store result score #r h155 store success score #s h155 {middle}run function h155:{name}"));
                    let messages = game.server.chat_capture.take().unwrap();
                    let callbacks = game.server.cmds.0.results.take().unwrap();
                    assert!(accepted, "actual registered function command");
                    let count = if route == "two" { 2 } else { 1 };
                    assert_eq!(
                        game.score.board.score("#calls", "h155").unwrap().value,
                        count
                    );
                    assert_eq!(
                        game.score.board.score("#trace", "h155").unwrap().value,
                        match route {
                            "two" => 12,
                            "one" => 1,
                            _ => 3,
                        }
                    );
                    assert_eq!(
                        callbacks,
                        value.map_or_else(Vec::new, |v| vec![(success, v); count as usize])
                    );
                    assert_eq!(
                        game.score.board.score("#r", "h155").unwrap().value,
                        value.unwrap_or(91)
                    );
                    assert_eq!(
                        game.score.board.score("#s", "h155").unwrap().value,
                        if value.is_some() {
                            i32::from(success)
                        } else {
                            17
                        }
                    );
                    let keys: Vec<_> = messages
                        .iter()
                        .map(|m| m.translation_key().unwrap_or("literal").to_owned())
                        .collect();
                    let mut expected =
                        vec!["commands.function.scheduled.single".to_owned(); count as usize];
                    if value.is_some() {
                        expected
                            .extend(vec!["commands.function.result".to_owned(); count as usize]);
                    }
                    if keys != expected {
                        mismatches.push((name, route, keys, expected));
                    }
                }
            }
            assert!(game.run_command("scoreboard players set #plain h155 0"));
            game.server.cmds.0.results = Some(Vec::new());
            assert!(game.run_command("execute as @e[type=pig,tag=h155u,sort=nearest,limit=2] run scoreboard players add #plain h155 1"));
            assert_eq!(
                game.server.cmds.0.results.take().unwrap(),
                vec![(true, 1), (true, 2)]
            );
            assert_eq!(game.score.board.score("#plain", "h155").unwrap().value, 2);
            assert_eq!(game.world.game_time, clock);
            assert_eq!(game.world.chunks.len(), chunks);
            assert!(
                mismatches.is_empty(),
                "actual fork function feedback order mismatches: {mismatches:?}"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn function_frames_restore_outer_frame() {
        use crate::game::native_entity_refs::tests::with_game;
        use sunrise_level::datapack::{Function, FunctionLine};

        let mut ran = false;
        with_game(|game| {
            ran = true;
            let inner = Function {
                id: "test:inner".into(),
                lines: vec![FunctionLine::Command("return 1".into())],
                parameters: vec![],
            };
            game.server.exec.0.functions.push("test:outer".into());
            let source = game.server_source();
            game.run_function(&inner, None, source.clone()).unwrap();
            assert_eq!(game.server.exec.0.functions, ["test:outer"]);
            game.server.exec.0.functions.clear();
            game.run_function(&inner, None, source).unwrap();
            assert!(game.server.exec.0.functions.is_empty());
        });
        assert!(ran, "the prepared-resource fixture must run this case");
    }

    #[test]
    #[ignore = "daily"]
    #[cfg(any())]
    fn world_open_loads_functions_before_first_tick() {
        use crate::game::{native_entity_refs::tests::with_game, world_loading::WorldRequest};
        use sunrise_gameplay_common::game::tables::GameItems as _;

        let root = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("world-open-datapacks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let pack = root.join("datapacks/test");
        for path in [
            "data/test/function",
            "data/minecraft/tags/function",
            "data/test/tags/item",
            "data/test/tags/worldgen/biome",
            "data/test/recipe",
        ] {
            std::fs::create_dir_all(pack.join(path)).unwrap();
        }
        std::fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"Function fixture"}}"#,
        )
        .unwrap();
        let function = pack.join("data/test/function/start.mcfunction");
        std::fs::write(
            &function,
            "scoreboard objectives add loaded dummy\nscoreboard players add times loaded 1\n",
        )
        .unwrap();
        std::fs::write(
            pack.join("data/minecraft/tags/function/load.json"),
            r#"{"values":["test:start"]}"#,
        )
        .unwrap();
        std::fs::write(
            pack.join("data/test/tags/item/sample.json"),
            r#"{"values":["minecraft:stone"]}"#,
        )
        .unwrap();
        std::fs::write(pack.join("data/test/recipe/sample.json"), r#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stone"],"result":{"id":"minecraft:diamond","count":1}}"#).unwrap();
        std::fs::write(
            pack.join("data/test/tags/worldgen/biome/feature_marker.json"),
            r#"{"values":[]}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("datapacks.json"),
            r#"{"enabled":["file/test"],"disabled":[]}"#,
        )
        .unwrap();
        with_game(|game| {
            let mut request = WorldRequest::new(
                "test",
                Some("test"),
                Some(1),
                true,
                None,
                std::sync::Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla()),
            )
            .unwrap();
            request.save_root = root.clone();
            game.load_world(request).unwrap();
            assert!(game.server.fns.0.store.functions.contains_key("test:start"));
            assert_eq!(
                game.server.fns.0.store.tag_functions("minecraft:load"),
                ["test:start"]
            );
            assert!(game.items().item_tags["test:sample"].contains("minecraft:stone"));
            assert!(game.stations().recipe_index.contains("test:sample"));
            // Changes on disk must not make the first tick reload any table.
            std::fs::write(&function, "scoreboard objectives add changed dummy\n").unwrap();
            std::fs::write(
                pack.join("data/test/tags/item/sample.json"),
                r#"{"values":["minecraft:diamond"]}"#,
            )
            .unwrap();
            std::fs::remove_file(pack.join("data/test/recipe/sample.json")).unwrap();
            game.tick_functions();
            assert!(game.score.board.objective("loaded").is_some());
            assert!(game.score.board.objective("changed").is_none());
            game.tick_functions();
            assert_eq!(game.score.board.score("times", "loaded").unwrap().value, 1);
            assert!(game.items().item_tags["test:sample"].contains("minecraft:stone"));
            assert!(game.stations().recipe_index.contains("test:sample"));

            // Changing packs must not reload an old timer from disk.
            game.server.fns.0.schedule.push(Scheduled {
                id: "test:start".into(),
                time: 1000,
                seq: 1,
            });
            game.save_schedule();
            game.server.fns.0.schedule.clear();
            assert!(game.run_command("datapack disable \"file/test\""));
            assert!(game.server.fns.0.schedule.is_empty());
            assert!(game.run_command("datapack enable \"file/test\""));
            assert!(game.server.fns.0.schedule.is_empty());

            // World-open selection and completions must both use the incoming
            // level's flags, even while `self.level` is still the old level.
            game.ui.registries.by_registry.clear();
            game.level.enabled_features = Default::default();
            std::fs::write(
                pack.join("pack.mcmeta"),
                r#"{"pack":{"description":"Feature fixture"},"features":{"enabled":["test:feature"]}}"#,
            )
            .unwrap();
            let stack = game
                .reload_datapacks(false, &["test:feature".into()])
                .unwrap();
            assert!(
                game.ui.registries.by_registry["minecraft:worldgen/biome"]
                    .tags
                    .contains(&"test:feature_marker".to_owned())
            );
            let staged = game
                .stage_world_tables(&stack, false, &game.options.resources.clone())
                .unwrap();
            game.server.world_tables.0 = staged.tables;
            game.health_resources = staged.health;
            assert!(game.items().item_tags["test:sample"].contains("minecraft:diamond"));
        });
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Two packs override the same recipe: the later one in the selected
    /// (`datapacks.json`) order wins, not the later one alphabetically.
    #[test]
    fn content_overlay_follows_enabled_order() {
        for order in [["file/a", "file/b"], ["file/b", "file/a"]] {
            let dir = std::env::temp_dir().join(format!(
                "sunrise-overlay-{}-{}",
                std::process::id(),
                order[0].replace('/', "_")
            ));
            let _ = std::fs::remove_dir_all(&dir);
            for name in ["a", "b"] {
                let pack = dir.join("datapacks").join(name);
                std::fs::create_dir_all(pack.join("data/x/recipe")).unwrap();
                std::fs::write(
                    pack.join("pack.mcmeta"),
                    r#"{"pack":{"description":"fixture"}}"#,
                )
                .unwrap();
                std::fs::write(pack.join("data/x/recipe/r.json"), name).unwrap();
            }
            let json = serde_json::json!({"enabled": order, "disabled": []});
            std::fs::write(dir.join("datapacks.json"), json.to_string()).unwrap();
            let stack = pack_stack(
                &dir,
                std::path::Path::new(""),
                &Default::default(),
                &Default::default(),
                true,
                &[],
                &[],
            );
            let OverlayFiles {
                files, read_errors, ..
            } = overlay_files(&stack.data_packs(Vec::new()));
            assert!(read_errors.is_empty());
            let (_, versions) = files.iter().find(|(id, _)| id == "recipe/x:r").unwrap();
            let last = versions.last().unwrap();
            assert_eq!(
                last.as_slice(),
                order[1].trim_start_matches("file/").as_bytes()
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// What a native mod contributes (`native_data_pack_files`) joins the load
    /// the tables are built from, a world's first open and `/reload` alike
    /// (`Game::stage_world_tables` is both), and sits below the world's own
    /// packs. Each load asks again, so it is the current contribution that
    /// counts.
    #[test]
    fn contributed_files_join_the_load_below_the_worlds_packs() {
        let dir = std::env::temp_dir().join(format!("sunrise-contributed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pack = dir.join("datapacks").join("pack");
        std::fs::create_dir_all(pack.join("data/pkg/tags/item")).unwrap();
        std::fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"fixture"}}"#,
        )
        .unwrap();
        std::fs::write(
            pack.join("data/pkg/tags/item/both.json"),
            r#"{"replace": true, "values": ["pkg:from_pack"]}"#,
        )
        .unwrap();
        let settings = serde_json::json!({"enabled": ["file/pack"], "disabled": []});
        std::fs::write(dir.join("datapacks.json"), settings.to_string()).unwrap();
        let tag = |entry: &str| format!(r#"{{"values": ["{entry}"]}}"#).into_bytes();
        std::fs::create_dir_all(pack.join("data/pkg/recipe")).unwrap();
        std::fs::write(
            pack.join("data/pkg/recipe/shared.json"),
            br#"{"result":"world"}"#,
        )
        .unwrap();
        let passive = sunrise_assets::resource_source::ResourceSource::indexed(
            ["shared", "native_only"]
                .map(|name| {
                    (
                        format!("data/pkg/recipe/{name}.json"),
                        sunrise_assets::resource_source::Resource::bytes(
                            br#"{"result":"passive"}"#.to_vec(),
                        ),
                    )
                })
                .into_iter()
                .collect(),
        );
        let stack = pack_stack(
            &dir,
            std::path::Path::new(""),
            &Default::default(),
            &passive,
            true,
            &[],
            &[],
        );
        let OverlayFiles { files, .. } = overlay_files(&stack.data_packs(vec![
            ("tags/pkg:item/both".to_owned(), tag("pkg:from_mod")),
            ("tags/pkg:item/mod_only".to_owned(), tag("pkg:only_mod")),
            (
                "recipe/pkg:shared".into(),
                br#"{"result":"native"}"#.to_vec(),
            ),
            (
                "recipe/pkg:native_only".into(),
                br#"{"result":"native"}"#.to_vec(),
            ),
        ]));
        let recipe_result = |id: &str| {
            let (_, versions) = files.iter().find(|(key, _)| key == id).unwrap();
            serde_json::from_slice::<serde_json::Value>(versions.last().unwrap()).unwrap()["result"]
                .clone()
        };
        assert_eq!(recipe_result("recipe/pkg:shared"), "world");
        assert_eq!(recipe_result("recipe/pkg:native_only"), "native");
        // The pack's file is the path's last (`PackOverlay::top`), and its
        // `replace` drops what the mod put below it.
        let (_, versions) = files
            .iter()
            .find(|(id, _)| id == "tags/pkg:item/both")
            .unwrap();
        assert!(sunrise_world::loot::PackOverlay::top(versions).starts_with(b"{\"replace\""));
        let overlay = sunrise_world::loot::PackOverlay::new(&files);
        let tags = sunrise_items::items::load_tags(&dir, "item", &overlay).unwrap();
        assert!(tags["pkg:both"].contains("pkg:from_pack"));
        assert!(!tags["pkg:both"].contains("pkg:from_mod"));
        // A path only the mod has reaches the tables too.
        assert_eq!(
            tags["pkg:mod_only"].iter().collect::<Vec<_>>(),
            ["pkg:only_mod"]
        );
        // The next load asks again: the new contribution, not the first's.
        let OverlayFiles { files, .. } = overlay_files(&stack.data_packs(vec![(
            "tags/pkg:item/mod_only".to_owned(),
            tag("pkg:second_load"),
        )]));
        let overlay = sunrise_world::loot::PackOverlay::new(&files);
        let tags = sunrise_items::items::load_tags(&dir, "item", &overlay).unwrap();
        assert_eq!(
            tags["pkg:mod_only"].iter().collect::<Vec<_>>(),
            ["pkg:second_load"]
        );
        // A file that isn't JSON is skipped alone, not the whole load.
        let OverlayFiles { files, .. } = overlay_files(&stack.data_packs(vec![
            ("tags/pkg:item/mod_only".to_owned(), b"{not json".to_vec()),
            ("tags/pkg:item/good".to_owned(), tag("pkg:good")),
        ]));
        let overlay = sunrise_world::loot::PackOverlay::new(&files);
        let tags = sunrise_items::items::load_tags(&dir, "item", &overlay).unwrap();
        assert!(!tags.contains_key("pkg:mod_only"));
        assert!(tags["pkg:good"].contains("pkg:good"));
        // The resource path keeps any folders within the kind.
        let data = stack.data_packs(vec![
            ("data_maps/pkg:item/p".to_owned(), b"{\"a\":1}".to_vec()),
            ("worldgen/biome/pkg:p".to_owned(), b"{\"b\":2}".to_vec()),
        ]);
        let read = |path: &str| {
            data.read(&sunrise_core::identifier::Identifier::from_parts("pkg", path).unwrap())
        };
        assert_eq!(read("data_maps/item/p.json").unwrap(), b"{\"a\":1}");
        assert_eq!(read("worldgen/biome/p.json").unwrap(), b"{\"b\":2}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
include!("functions/typed_state_range_tests.rs");

#[cfg(test)]
#[path = "functions/pack_tag_tests.rs"]
mod pack_tag_tests;

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::command_registries::CommandRegistriesGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::mods::ModGameExt1 as _;
