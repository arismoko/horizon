//! Data packs in the game: functions (`/function`, `/return`, tick and load
//! tags, `/schedule`), predicates, `/reload` and `/datapack`.
use super::{
    Game,
    commands::{
        R, Target, err,
        execute::{ExecState, Source},
    },
};
use horizon_client::{
    command::{Arg, Context},
    datapack::{self, DataPack, DataStore, Function},
    save,
    scoreboard::bracketed,
    text::{self, Text},
};
use horizon_core::nbt::Tag;
use mod_api::__private::reload::ReloadStageError;
use std::sync::Arc;

/// A `TimerQueue` event.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Scheduled {
    /// `ns:function`, or `#ns:tag` for a function tag.
    pub id: String,
    pub time: i64,
    pub seq: u64,
}
#[derive(Default)]
pub(super) struct FnState {
    pub store: DataStore,
    packs: Vec<Arc<DataPack>>,
    /// Enabled pack ids, in load order (later overrides earlier).
    enabled: Vec<String>,
    loaded: bool,
    /// Run the `minecraft:load` tag on the next tick.
    load_pending: bool,
    pub schedule: Vec<Scheduled>,
    next_seq: u64,
    /// A dedicated server's `function-permission-level`.
    pub permission: Option<u8>,
}
/// What a function reported through `return`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    NoReturn,
    Value(i32),
    Fail,
}

/// The world's pack stack: the built-in pack (unless disabled), the packs
/// of `<world>/datapacks` and the resources root's feature packs, with the
/// enabled ids from `datapacks.json` (`PackRepository.reload`).
pub(super) struct PackStack {
    pub builtin: Option<DataPack>,
    content: DataPack,
    pub packs: Vec<Arc<DataPack>>,
    pub enabled: Vec<String>,
}
impl PackStack {
    /// The enabled packs in `enabled` (the selected) order, later overriding
    /// earlier: `PackRepository.openAllSelected` keeps the selected order.
    pub(super) fn enabled_packs(&self) -> impl Iterator<Item = &DataPack> {
        self.enabled
            .iter()
            .filter_map(|id| self.packs.iter().find(|p| &p.id == id).map(Arc::as_ref))
    }
}
pub(super) fn pack_stack(
    world_root: &std::path::Path,
    resources: &std::path::Path,
    builtin_source: &horizon_assets::resource_source::ResourceSource,
    content_source: &horizon_assets::resource_source::ResourceSource,
    vanilla_disabled: bool,
    enabled_features: &[String],
    additional: &[Arc<DataPack>],
) -> PackStack {
    let builtin = (!vanilla_disabled)
        .then(|| DataPack::from_source("vanilla".into(), builtin_source.clone()));
    let packs = super::pack_finders::data_repository(world_root, resources, additional);
    let enabled = datapack::enabled_ids(world_root, &packs, enabled_features);
    PackStack {
        content: DataPack::from_source("horizon-content".into(), content_source.clone()),
        builtin,
        packs: packs.into_iter().map(Arc::new).collect(),
        enabled,
    }
}

/// The world's enabled packs' content files (`recipe/`, `loot_table/`,
/// `advancement/`, `tags/…`) for the tables to merge over the resource
/// root's own. Empty when the world has no packs.
pub(super) fn content_overlay(stack: &PackStack) -> Vec<(String, Vec<Vec<u8>>)> {
    let packs: Vec<&DataPack> = std::iter::once(&stack.content)
        .chain(stack.enabled_packs())
        .collect();
    if packs.is_empty() {
        return Vec::new();
    }
    DataStore::load(&packs).files.into_iter().collect()
}

/// The files the content tables load from: the world's enabled packs'
/// (`content_overlay`) with what native mods contribute for this load
/// (`native_data_pack_files`) below them. `PackOverlay::top` is a path's last
/// file, so a pack's file still overrides a mod's for the same path, and a
/// later contribution overrides an earlier one, as packs do. `Game` builds
/// its tables through here on a world's first open and on `/reload`, so the
/// mods are asked once per load.
pub(super) fn overlay_files(
    stack: &PackStack,
    contributed: Vec<(String, Vec<u8>)>,
) -> Vec<(String, Vec<Vec<u8>>)> {
    if contributed.is_empty() {
        return content_overlay(stack);
    }
    // Passive content replaces prepared data, native contributions sit over
    // it, and the world's selected packs retain the highest priority.
    let mut files = DataStore::load(&[&stack.content]).files;
    for (path, bytes) in contributed {
        if let Err(e) = serde_json::from_slice::<Box<serde_json::value::RawValue>>(&bytes) {
            log::warn!("Horizon: a native mod's data pack file {path} is not JSON, skipped: {e}");
            continue;
        }
        files.entry(path).or_default().push(bytes);
    }
    let packs = stack.enabled_packs().collect::<Vec<_>>();
    for (path, versions) in DataStore::load(&packs).files {
        files.entry(path).or_default().extend(versions);
    }
    files.into_iter().collect()
}

impl FnState {
    /// The highest enabled world pack providing this data resource.
    pub(super) fn data_resource(
        &self,
        name: &str,
    ) -> Option<horizon_assets::resource_source::Resource> {
        self.enabled
            .iter()
            .rev()
            .filter_map(|id| self.packs.iter().find(|pack| &pack.id == id))
            .find_map(|pack| {
                let source = pack.source();
                source.contains(name).then(|| {
                    horizon_assets::resource_source::Resource::file(source.clone(), name.to_owned())
                })
            })
    }

    /// Enabled pack ids, in load order (later overrides earlier).
    #[cfg(test)]
    pub(super) fn enabled(&self) -> &[String] {
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

impl Game<'_> {
    // ---- loading -----------------------------------------------------------

    fn datapack_settings_path(&self) -> std::path::PathBuf {
        self.world_root.join("datapacks.json")
    }
    /// Discover and load the data packs of the world (once, or on `/reload`).
    pub(super) fn ensure_datapacks(&mut self) {
        if self.fns.loaded || self.world_root.as_os_str().is_empty() {
            return;
        }
        let _ = self.reload_datapacks(false, &self.level.enabled_features.clone());
    }
    pub(super) fn reload_datapacks(
        &mut self,
        run_load: bool,
        enabled_features: &[String],
    ) -> Result<PackStack, ReloadStageError> {
        horizon_client::mod_content::check_jars(
            &self.options.resources,
            &horizon_client::logging::game_home().join("jars"),
        )
        .map_err(ReloadStageError::OriginalJars)?;
        let next_root = self
            .stage_enabled_inventory()
            .map_err(|error| ReloadStageError::DataSource(error.to_string()))?;
        let resources = next_root
            .as_ref()
            .map_or(self.options.resources.as_path(), |root| root.prepared())
            .to_path_buf();
        let data = next_root
            .as_ref()
            .map_or(&self.pack.data_source, |root| root.data());
        let changes = next_root
            .as_ref()
            .map_or(&self.pack.data_changes, |root| root.data_changes());
        let additional = super::pack_finders::data_packs(&self.mods);
        let stack = pack_stack(
            &self.world_root,
            &resources,
            data,
            changes,
            self.cmds.vanilla_disabled,
            enabled_features,
            &additional,
        );
        let prepared = self.prepare_server_reload(&stack, &resources)?;
        let context = self
            .prepare_loop_context(&resources)
            .map_err(|error| ReloadStageError::DataSource(error.to_string()))?;
        let switched = self.mods.data_switched || next_root.is_some();
        let prepared = self.apply_server_reload_callbacks(prepared);
        let world = if run_load && self.remote.is_none() {
            Some(
                self.prepare_world_reload(&stack, &resources, switched)
                    .map_err(|error| ReloadStageError::DataSource(error.to_string()))?,
            )
        } else {
            None
        };
        if let Some(root) = next_root {
            self.options.resources = root.prepared().to_owned();
            self.pack.data_source = root.data().clone();
            self.pack.data_changes = root.data_changes().clone();
            self.mods.data_switched = true;
        }
        self.install_loop_context(context);
        if !run_load {
            self.fns = Default::default();
        }
        self.fns.packs = stack.packs.clone();
        self.fns.enabled = stack.enabled.clone();
        self.apply_server_reload(prepared, &stack);
        self.pack.additional_data = additional.into();
        self.fns.loaded = true;
        self.fns.load_pending = true;
        // An explicit reload (`/reload`, `/datapack`) also rebuilds the
        // content tables, as `ReloadableServerResources` does; on world open
        // `finish_open_world` builds them right after this.
        if let Some(world) = world {
            self.reload_world_tables(world);
        }
        if self.fns.schedule.is_empty() {
            self.load_schedule();
        }
        Ok(stack)
    }
    fn save_datapack_settings(&self) {
        let disabled: Vec<&String> = self
            .fns
            .packs
            .iter()
            .filter(|p| !self.fns.enabled.contains(&p.id))
            .map(|p| &p.id)
            .collect();
        let json = serde_json::json!({"enabled": self.fns.enabled, "disabled": disabled});
        let _ = std::fs::write(self.datapack_settings_path(), json.to_string());
    }
    pub(super) fn sync_function_registries(&mut self, stack: &PackStack) {
        self.sync_schedule_registry();
        self.sync_enchantment_registries();
        let reg = &mut self.ui.registries;
        reg.functions = self.fns.store.functions.keys().cloned().collect();
        reg.function_tags = self.fns.store.function_tags.keys().cloned().collect();
        reg.predicates = self.fns.store.predicates.keys().cloned().collect();
        reg.item_modifiers = self.fns.store.item_modifiers.keys().cloned().collect();
        reg.datapacks_enabled = std::iter::once("vanilla".to_owned())
            .chain(self.fns.enabled.iter().cloned())
            .collect();
        reg.datapacks_disabled = self
            .fns
            .packs
            .iter()
            .filter(|p| !self.fns.enabled.contains(&p.id))
            .map(|p| p.id.clone())
            .collect();
        // A pack's worldgen ids join the completion registries
        // (`command-registries.json` lists only the game's own), and so do
        // its worldgen tag ids for the `#tag` completions. The packs are the
        // ones the world's worldgen is built from.
        let enabled: Vec<_> = stack.enabled_packs().collect();
        let (overrides, _) = horizon_client::datapack::worldgen_overrides(&enabled, |_| true);
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

    fn sync_schedule_registry(&mut self) {
        self.ui.registries.schedules = self.fns.schedule.iter().map(|s| s.id.clone()).collect();
    }

    // ---- scheduled events ------------------------------------------------------

    fn schedule_path(&self) -> std::path::PathBuf {
        self.world_root.join("data/minecraft/scheduled_events.dat")
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
            self.fns.next_seq += 1;
            self.fns.schedule.push(Scheduled {
                id: id.to_string_lossy(),
                time: *time,
                seq: self.fns.next_seq,
            });
        }
    }
    pub(super) fn save_schedule(&self) {
        if !self.fns.loaded || self.remote.is_some() {
            return;
        }
        let mut events = self.fns.schedule.clone();
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
                ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
            ]),
        );
    }

    // ---- ticking -----------------------------------------------------------------

    /// `ServerFunctionManager.tick` and the scheduled events.
    pub(super) fn tick_functions(&mut self) {
        if self.remote.is_some() {
            return;
        }
        self.exec = ExecState::default();
        let source = self.server_source();
        if self.fns.load_pending {
            self.fns.load_pending = false;
            for f in self.fns.store.tag_functions("minecraft:load") {
                self.run_function_by_id(&f, None, source);
            }
        }
        let now = self.world.game_time;
        loop {
            let due = self
                .fns
                .schedule
                .iter()
                .enumerate()
                .filter(|(_, e)| e.time <= now)
                .min_by_key(|(_, e)| (e.time, e.seq))
                .map(|(i, _)| i);
            let Some(i) = due else { break };
            let event = self.fns.schedule.remove(i);
            match event.id.strip_prefix('#') {
                Some(tag) => {
                    for f in self.fns.store.tag_functions(tag) {
                        self.run_function_by_id(&f, None, source);
                    }
                }
                None => self.run_function_by_id(&event.id, None, source),
            }
        }
        for f in self.fns.store.tag_functions("minecraft:tick") {
            self.run_function_by_id(&f, None, source);
        }
    }
    fn run_function_by_id(&mut self, id: &str, args: Option<&Tag>, source: Source) {
        self.exec.commands = 0;
        if let Some(f) = self.fns.store.functions.get(id).cloned() {
            // `ServerFunctionManager.execute` logs a function that failed to
            // instantiate or run; its lines' own failures stay silent.
            if let Err(e) = self.run_function(&f, args, source) {
                log::warn!(
                    "Horizon: failed to run function {id}: {}",
                    e.message.plain()
                );
            }
        }
    }

    /// The function branch of advancement rewards: resolve the optional function
    /// and execute it as the rewarded player, with suppressed output and GAMEMASTER permission.
    pub(super) fn run_advancement_reward_function(&mut self, id: &str) {
        self.ensure_datapacks();
        let Some(function) = self.fns.store.functions.get(id).cloned() else {
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
            .fns
            .permission
            .replace(horizon_client::command::vanilla::GAMEMASTERS);
        let callback = self.exec.callback.take();
        let result = self.run_function(&function, None, source);
        self.exec.callback = callback;
        self.fns.permission = permission;
        if let Err(error) = result {
            log::warn!(
                "Horizon: failed to run advancement reward function {id}: {}",
                error.message.plain()
            );
        }
    }

    // ---- running -----------------------------------------------------------------

    /// Run one function with `source`: its commands in order, until a `return`.
    pub(in crate::game) fn run_function(
        &mut self,
        f: &Function,
        args: Option<&Tag>,
        source: Source,
    ) -> R<Outcome> {
        let lines = f.instantiate(args).map_err(|(key, a)| {
            let args: Vec<&dyn std::fmt::Display> =
                a.iter().map(|s| s as &dyn std::fmt::Display).collect();
            err(
                "commands.function.instantiationFailure",
                &[&f.id, &horizon_client::text::Text::tr(&key, &args).plain()],
            )
        })?;
        if self.exec.functions.len() >= 128 {
            return Ok(Outcome::NoReturn);
        }
        self.exec.functions.push(f.id.clone());
        let outer = self.exec.returned.take();
        let level = std::mem::replace(&mut self.ui.registries.permission, self.fns.command_level());
        let inner = Source {
            silent: true,
            store: None,
            ..source
        };
        for line in lines {
            let _ = self.execute_line(&line, inner);
            if self.exec.returned.is_some() {
                break;
            }
            if self.exec.commands
                > self
                    .ui
                    .rules
                    .int_at(horizon_client::gamerules::vanilla::MAX_COMMAND_SEQUENCE_LENGTH)
                    .max(1) as u32
            {
                break;
            }
        }
        self.ui.registries.permission = level;
        let returned = std::mem::replace(&mut self.exec.returned, outer);
        self.exec.functions.pop();
        Ok(match returned {
            None => Outcome::NoReturn,
            Some(Some(v)) => Outcome::Value(v),
            Some(None) => Outcome::Fail,
        })
    }
    /// The functions a `FunctionArgument` names: a function or a tag.
    pub(in crate::game) fn functions_named(&mut self, name: &str) -> R<Vec<Function>> {
        match name.strip_prefix('#') {
            // ServerFunctionLibrary.getTag: an unknown tag is empty.
            Some(tag) => {
                if !self.fns.store.function_tags.contains_key(tag) {
                    return Ok(Vec::new());
                }
                Ok(self
                    .fns
                    .store
                    .tag_functions(tag)
                    .iter()
                    .filter_map(|id| self.fns.store.functions.get(id).cloned())
                    .collect())
            }
            None => self
                .fns
                .store
                .functions
                .get(name)
                .cloned()
                .map(|f| vec![f])
                .ok_or_else(|| err("arguments.function.unknown", &[&name])),
        }
    }
    /// `if function`: what each function returns (None if it did not).
    pub(in crate::game) fn call_function_ref(
        &mut self,
        name: &str,
        args: Option<&Tag>,
    ) -> R<Vec<Option<i32>>> {
        let functions = self.functions_named(name)?;
        let source = self.command_source.unwrap_or_else(|| self.player_source());
        let mut out = Vec::new();
        for f in functions {
            out.push(match self.run_function(&f, args, source)? {
                Outcome::NoReturn => None,
                Outcome::Value(v) => Some(v),
                Outcome::Fail => Some(0),
            });
        }
        Ok(out)
    }

    // ---- commands ------------------------------------------------------------------

    /// `FunctionCommand.decorateOutputIfNeeded`: report returned values even for failure.
    pub(super) fn cmd_function(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::Resource(name)) = ctx.get("name").cloned() else {
            return Ok(0);
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
                    let path = horizon_client::nbt_path::NbtPath::parse(p)
                        .map_err(horizon_client::data_cmd::path_error)?;
                    horizon_client::data_cmd::single_tag(&path, &data)?
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
                horizon_core::nbt::parse_snbt(text)
                    .map_err(|e| err("argument.nbt.invalid", &[&e]))?
                    .0,
            )
        } else {
            None
        };
        let message = if functions.len() == 1 {
            Text::translate(
                "commands.function.scheduled.single",
                &[Text::literal(functions[0].id.clone())],
            )
        } else {
            Text::translate(
                "commands.function.scheduled.multiple",
                &[horizon_client::scoreboard_cmd::format_list(
                    functions
                        .iter()
                        .map(|f| Text::literal(f.id.clone()))
                        .collect(),
                )],
            )
        };
        self.feedback(message);
        let source = self.command_source.unwrap_or_else(|| self.player_source());
        let mut sum: i32 = 0;
        let mut any = false;
        let mut failed = false;
        for f in functions {
            match self.run_function(&f, args.as_ref(), source)? {
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
        self.exec.callback = Some(if failed && sum == 0 {
            Some((false, 0))
        } else if any {
            Some((true, sum))
        } else {
            None
        });
        Ok(sum)
    }
    /// `return <value>` and `return fail` (`return run` is a chain).
    pub(super) fn cmd_return(&mut self, id: &str, ctx: &Context) -> R<i32> {
        if id == "return.fail" {
            self.finish_return(None);
            self.exec.callback = Some(Some((false, 0)));
            return Ok(0);
        }
        let value = match ctx.get("value") {
            Some(Arg::Int(v)) => *v,
            _ => 0,
        };
        self.finish_return(Some(value));
        Ok(value)
    }
    pub(super) fn cmd_schedule(&mut self, id: &str, ctx: &Context) -> R<i32> {
        if id == "schedule.clear" {
            let Some(Arg::Resource(name)) = ctx.get("function").cloned() else {
                return Ok(0);
            };
            let before = self.fns.schedule.len();
            self.fns.schedule.retain(|e| e.id != name);
            let count = before - self.fns.schedule.len();
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
        let tick_time = self.world.game_time + i64::from(time);
        let schedule_id = match name.strip_prefix('#') {
            // An unknown tag is empty, and scheduling it is allowed.
            Some(_) => name.clone(),
            None => {
                let Some(f) = self.fns.store.functions.get(&name) else {
                    return Err(err("arguments.function.unknown", &[&name]));
                };
                if f.is_macro() {
                    return Err(err("commands.schedule.macro", &[]));
                }
                name.clone()
            }
        };
        if replace {
            self.fns.schedule.retain(|e| e.id != schedule_id);
        }
        if !self
            .fns
            .schedule
            .iter()
            .any(|e| e.id == schedule_id && e.time == tick_time)
        {
            self.fns.next_seq += 1;
            let seq = self.fns.next_seq;
            self.fns.schedule.push(Scheduled {
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
    pub(super) fn cmd_reload(&mut self) -> R<i32> {
        self.feedback(Text::translate("commands.reload.success", &[]));
        self.reload_datapacks(true, &self.level.enabled_features.clone())
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
            .fns
            .packs
            .iter()
            .find(|p| p.id == id)
            .map_or_else(|| "Default data pack".to_owned(), |p| p.description());
        let style = text::Style {
            hover: Some(Box::new(Text::literal(format!("{id}\n{description}")))),
            ..style
        };
        let source = if id == "vanilla" {
            "pack.source.builtin"
        } else if self
            .fns
            .packs
            .iter()
            .any(|pack| pack.id == id && pack.is_feature())
        {
            "pack.source.feature"
        } else if self
            .fns
            .packs
            .iter()
            .find(|pack| pack.id == id)
            .and_then(|pack| pack.selection.as_ref())
            .is_some_and(|info| info.source == horizon_client::pack_screen::Source::BuiltIn)
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
        let vanilla = (!self.cmds.vanilla_disabled).then(|| "vanilla".to_owned());
        vanilla
            .into_iter()
            .chain(self.fns.enabled.iter().cloned())
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
        if self.world_root.as_os_str().is_empty() {
            return Err(err("commands.datapack.create.io_failure", &[&id]));
        }
        let datapacks = self.world_root.join("datapacks");
        let pack = datapacks.join(&id);
        if pack.exists() {
            return Err(err("commands.datapack.create.already_exists", &[&id]));
        }
        // The data pack version (SharedConstants), written as a bare major
        // number for both bounds, as vanilla does.
        let pack_format = horizon_core::version::DATA_PACK_FORMAT.0;
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
        self.fns
            .packs
            .iter()
            .find(|pack| pack.id == id)
            .map(|pack| {
                pack.features()
                    .into_iter()
                    .filter(|flag| !self.level.enabled_features.contains(flag))
                    .collect()
            })
            .unwrap_or_default()
    }
    pub(super) fn cmd_datapack(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let name = match ctx.get("name") {
            Some(Arg::Str(n)) => Some(n.clone()),
            _ => None,
        };
        let selected = self.selected_packs();
        let previous_enabled = self.fns.enabled.clone();
        let previous_vanilla_disabled = self.cmds.vanilla_disabled;
        // `DataPackCommand.listAvailablePacks`: a feature pack the world's
        // features do not cover is not available.
        let available: Vec<String> = std::iter::once("vanilla".to_owned())
            .chain(self.fns.packs.iter().map(|p| p.id.clone()))
            .filter(|p| !selected.contains(p))
            .filter(|p| self.missing_pack_features(p).is_empty())
            .collect();
        let known = |g: &Self, n: &str| n == "vanilla" || g.fns.packs.iter().any(|p| p.id == n);
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
                                horizon_client::scoreboard_cmd::format_list(links),
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
                                horizon_client::scoreboard_cmd::format_list(links),
                            ],
                        ));
                    }
                }
                Ok(total as i32)
            }
            "datapack.enable" => {
                let name = name.unwrap_or_default();
                if !known(self, &name) {
                    return Err(err("commands.datapack.unknown", &[&name]));
                }
                if selected.contains(&name) {
                    return Err(err("commands.datapack.enable.failed", &[&name]));
                }
                let missing = self.missing_pack_features(&name);
                if !missing.is_empty() {
                    // `DataPackCommand`: the pack's requested features must
                    // be a subset of the world's enabled ones.
                    return Err(err(
                        "commands.datapack.enable.failed.no_flags",
                        &[&name, &missing.join(", ")],
                    ));
                }
                if name == "vanilla" {
                    self.cmds.vanilla_disabled = false;
                } else {
                    let existing = match ctx.get("existing") {
                        Some(Arg::Str(e)) => Some(e.clone()),
                        _ => None,
                    };
                    let position = if ctx.has("first") {
                        0
                    } else if let Some(e) = &existing {
                        let Some(i) = self.fns.enabled.iter().position(|p| p == e) else {
                            return Err(err("commands.datapack.unknown", &[e]));
                        };
                        if ctx.has("before") { i } else { i + 1 }
                    } else if !ctx.has("last")
                        && self
                            .fns
                            .packs
                            .iter()
                            .find(|pack| pack.id == name)
                            .and_then(|pack| pack.selection.as_ref())
                            .is_some_and(|info| {
                                info.position == horizon_client::packs::Position::Bottom
                            })
                    {
                        0
                    } else {
                        self.fns.enabled.len()
                    };
                    self.fns.enabled.insert(position, name.clone());
                }
                self.feedback(Text::translate(
                    "commands.datapack.modify.enable",
                    &[self.pack_link(&name, true)],
                ));
                self.apply_pack_change(previous_enabled, previous_vanilla_disabled)?;
                Ok(self.selected_packs().len() as i32)
            }
            _ => {
                let name = name.unwrap_or_default();
                if !known(self, &name) {
                    return Err(err("commands.datapack.unknown", &[&name]));
                }
                if !selected.contains(&name) {
                    return Err(err("commands.datapack.disable.failed", &[&name]));
                }
                if self
                    .fns
                    .packs
                    .iter()
                    .find(|pack| pack.id == name)
                    .and_then(|pack| pack.selection.as_ref())
                    .is_some_and(|info| info.required)
                {
                    return Err(err("horizon.datapack.required", &[&name]));
                }
                if self
                    .fns
                    .packs
                    .iter()
                    .any(|pack| pack.id == name && pack.is_feature() && !pack.features().is_empty())
                {
                    // `DataPackCommand`'s ERROR_CANNOT_DISABLE_FEATURE: the
                    // flag stays on for the world's life.
                    return Err(err("commands.datapack.disable.failed.feature", &[&name]));
                }
                let missing = self.missing_pack_features(&name);
                if !missing.is_empty() {
                    return Err(err(
                        "commands.datapack.enable.failed.no_flags",
                        &[&name, &missing.join(", ")],
                    ));
                }
                if name == "vanilla" {
                    self.cmds.vanilla_disabled = true;
                } else {
                    self.fns.enabled.retain(|p| p != &name);
                }
                self.feedback(Text::translate(
                    "commands.datapack.modify.disable",
                    &[self.pack_link(&name, true)],
                ));
                self.apply_pack_change(previous_enabled, previous_vanilla_disabled)?;
                Ok(self.selected_packs().len() as i32)
            }
        }
    }
    /// `ReloadCommand.reloadPacks`: no message of its own. It reloads the
    /// server's data as `/reload` does (`reload_datapacks`), the content
    /// tables with the functions.
    fn apply_pack_change(
        &mut self,
        previous_enabled: Vec<String>,
        previous_vanilla_disabled: bool,
    ) -> R<()> {
        // One resolve for the store and the tables, as `/reload`'s is: the
        // timers already in memory stay, and none come back from disk
        // (`ServerFunctionManager` keeps its queue over a pack change).
        let additional = super::pack_finders::data_packs(&self.mods);
        let mut stack = pack_stack(
            &self.world_root,
            &self.options.resources,
            &self.pack.data_source,
            &self.pack.data_changes,
            self.cmds.vanilla_disabled,
            &self.level.enabled_features,
            &additional,
        );
        stack.enabled =
            datapack::selected_ids(stack.packs.iter().map(Arc::as_ref), &self.fns.enabled);
        let prepared = match self.prepare_server_reload(&stack, &self.options.resources) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.fns.enabled = previous_enabled;
                self.cmds.vanilla_disabled = previous_vanilla_disabled;
                return Err(Self::reload_failure(error));
            }
        };
        let root = self.options.resources.clone();
        let context = match self.prepare_loop_context(&root) {
            Ok(context) => context,
            Err(error) => {
                self.fns.enabled = previous_enabled;
                self.cmds.vanilla_disabled = previous_vanilla_disabled;
                return Err(Self::reload_failure(ReloadStageError::DataSource(
                    error.to_string(),
                )));
            }
        };
        let prepared = self.apply_server_reload_callbacks(prepared);
        let world = match self.prepare_world_reload(&stack, &root, self.mods.data_switched) {
            Ok(world) => world,
            Err(error) => {
                self.fns.enabled = previous_enabled;
                self.cmds.vanilla_disabled = previous_vanilla_disabled;
                return Err(Self::reload_failure(ReloadStageError::DataSource(
                    error.to_string(),
                )));
            }
        };
        self.install_loop_context(context);
        self.fns.packs = stack.packs.clone();
        self.fns.enabled = stack.enabled.clone();
        self.pack.additional_data = additional.into();
        self.save_datapack_settings();
        self.apply_server_reload(prepared, &stack);
        self.fns.load_pending = true;
        self.reload_world_tables(world);
        Ok(())
    }

    fn reload_failure(error: ReloadStageError) -> horizon_client::command::CommandError {
        log::warn!("Horizon: {error}");
        horizon_client::command::CommandError::new(
            Text::translate("commands.reload.failure", &[])
                .append(Text::literal(format!("\n{error}"))),
        )
    }

    // ---- predicates -------------------------------------------------------------------

    /// `ResourceOrIdArgument.getLootPredicate`: accept the inline or named
    /// predicate, then test it with the existing source entity and origin.
    pub(in crate::game) fn check_predicate(&mut self, id: &str) -> R<bool> {
        self.ensure_datapacks();
        // ResourceOrIdArgument retains either an inline value or a named id.
        // Both use the same evaluator and command-source context below.
        let json = if id.starts_with('{') {
            let (tag, _) = horizon_core::nbt::parse_snbt(id)
                .map_err(|e| err("argument.nbt.invalid", &[&e]))?;
            horizon_client::scoreboard::tag_to_json(&tag)
        } else {
            self.fns
                .store
                .predicates
                .get(id)
                .cloned()
                .ok_or_else(|| err("argument.predicate.unknown", &[&id]))?
        };
        let source = self.command_source.unwrap_or_else(|| self.player_source());
        let view = source.entity.map(|t| {
            let scores = self.holder_name(t).map(|holder| {
                self.score
                    .board
                    .holder_scores(&holder)
                    .into_iter()
                    .map(|(objective, value)| (objective.name.clone(), value))
                    .collect()
            });
            let mut view = self.entity_view_asked(t, &json);
            view.scores = scores;
            view
        });
        Ok(self.conditions_hold(&json, view.as_ref(), source.pos))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn function_frames_restore_outer_frame() {
        use crate::game::native_entity_refs::tests::with_game;
        use horizon_client::datapack::{Function, FunctionLine};

        let mut ran = false;
        with_game(|game| {
            ran = true;
            let inner = Function {
                id: "test:inner".into(),
                lines: vec![FunctionLine::Command("return 1".into())],
                parameters: vec![],
            };
            game.exec.functions.push("test:outer".into());
            let source = game.server_source();
            game.run_function(&inner, None, source).unwrap();
            assert_eq!(game.exec.functions, ["test:outer"]);
            game.exec.functions.clear();
            game.run_function(&inner, None, source).unwrap();
            assert!(game.exec.functions.is_empty());
        });
        assert!(ran, "the prepared-resource fixture must run this case");
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn refused_item_key_reload_keeps_previous_world_generation() {
        use mod_api::{Hooks, ModEntry, Registrar, RegistrationContext};

        struct MissingItem;
        #[mod_api::__subscriptions]
        impl Hooks for MissingItem {
            fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
                registrar.block_key("minecraft:stone");
                registrar.item_key("reload_test:missing");
            }
        }

        crate::game::native_entity_refs::tests::with_game(|game| {
            let root =
                std::env::temp_dir().join(format!("reload-atomicity-{}", std::process::id()));
            let pack = root.join("datapacks/candidate");
            let tags = pack.join("data/reload_test/tags/block");
            std::fs::create_dir_all(&tags).unwrap();
            std::fs::write(pack.join("pack.mcmeta"), serde_json::to_vec(&serde_json::json!({
                "pack": {"description": "atomic reload", "min_format": horizon_core::version::DATA_PACK_FORMAT, "max_format": horizon_core::version::DATA_PACK_FORMAT}
            })).unwrap()).unwrap();
            std::fs::write(
                tags.join("candidate.json"),
                br#"{"values":["minecraft:stone"]}"#,
            )
            .unwrap();
            game.world_root = root.clone();
            game.set_allow_commands(true);
            crate::game::api_values::install_parts(&[ModEntry::new(
                "reload_test",
                Box::new(MissingItem),
            )])
            .unwrap();
            let stone = mod_api::__private::part_tests::block_key("minecraft:stone");
            let registry = game.pack.assets.registry;
            mod_api::__private::resolve_blocks(|id| {
                let default = registry.default_state(id)?;
                Some(mod_api::__private::ResolvedBlock {
                    default,
                    states: mod_api::BlockStateRange::new(default, 1),
                    block: game.res.behaviours().block(default),
                })
            })
            .unwrap();
            let old_tag =
                super::super::native_world_read::block_in_tag(stone, "reload_test:candidate");
            let old_context = game.loop_context.clone();
            let old_root = game.options.resources.clone();
            let old_data = game.pack.data_source.clone();
            let old_tables = game.world_tables.as_ref().map(|tables| tables as *const _);
            let old_generation = game.damage_tag_generation;
            let old_enabled = game.fns.enabled.clone();
            let error = game.reload_datapacks(true, &[]).err().unwrap();
            assert!(error.to_string().contains("reload_test:missing"));
            assert_eq!(game.options.resources, old_root);
            assert!(game.pack.data_source.same_generation(&old_data));
            assert!(Arc::ptr_eq(&game.loop_context, &old_context));
            assert_eq!(
                old_tables,
                game.world_tables.as_ref().map(|tables| tables as *const _)
            );
            assert_eq!(game.damage_tag_generation, old_generation);
            assert_eq!(game.fns.enabled, old_enabled);
            assert_eq!(
                super::super::native_world_read::block_in_tag(stone, "reload_test:candidate"),
                old_tag
            );
            struct Valid;
            #[mod_api::__subscriptions]
            impl Hooks for Valid {
                fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
                    registrar.block_key("minecraft:stone");
                }
            }
            crate::game::api_values::install_parts(&[ModEntry::new(
                "reload_test",
                Box::new(Valid),
            )])
            .unwrap();
            game.reload_datapacks(true, &[]).unwrap();
            assert!(game.items().block_tags["reload_test:candidate"].contains("minecraft:stone"));
            assert_eq!(
                super::super::native_world_read::block_in_tag(stone, "reload_test:candidate"),
                Some(true)
            );
            // The command path also prepares the replacement before publishing
            // its canonical selection or persisting the player's pack settings.
            assert!(game.run_command("datapack disable \"file/candidate\""));
            assert!(game.run_command("datapack enable \"file/candidate\""));
            game.save_datapack_settings();
            let settings = std::fs::read(root.join("datapacks.json")).unwrap();
            let enabled = game.fns.enabled.clone();
            let context = game.loop_context.clone();
            let generation = game.damage_tag_generation;
            crate::game::api_values::install_parts(&[ModEntry::new(
                "reload_test",
                Box::new(MissingItem),
            )])
            .unwrap();
            assert!(!game.run_command("datapack disable \"file/candidate\""));
            assert_eq!(game.fns.enabled, enabled);
            assert_eq!(
                std::fs::read(root.join("datapacks.json")).unwrap(),
                settings
            );
            assert!(Arc::ptr_eq(&game.loop_context, &context));
            assert_eq!(game.damage_tag_generation, generation);
            assert!(game.items().block_tags["reload_test:candidate"].contains("minecraft:stone"));
            let _ = std::fs::remove_dir_all(root);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn pack_changes_publish_the_new_zip_generation_for_template_reads() {
        use std::{io::Write, path::PathBuf};
        use {
            horizon_core::{nbt, nbt::Limits},
            horizon_world::compression::Compression,
        };
        let root = std::env::temp_dir().join(format!(
            "template-pack-change-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("datapacks")).unwrap();
        struct Scratch(PathBuf);
        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let scratch = Scratch(root);
        let archive = scratch.0.join("datapacks/templates.zip");
        let publish = |marker: &str| {
            let named = nbt::NamedTag {
                name: "".into(),
                tag: Tag::compound([("marker", Tag::String(marker.into()))]),
            };
            let bytes = Compression::Gzip
                .encode(&nbt::write_named(&named, Limits::templates()).unwrap())
                .unwrap();
            let next = archive.with_extension("next");
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&next).unwrap());
            zip.start_file("pack.mcmeta", zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"{}").unwrap();
            zip.start_file(
                "data/generation_test/structure/probe.nbt",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(&bytes).unwrap();
            zip.finish().unwrap();
            std::fs::rename(next, &archive).unwrap();
        };
        publish("old");
        let mut ran = false;
        crate::game::native_entity_refs::tests::with_game(|game| {
            ran = true;
            game.world_root = scratch.0.clone();
            game.set_allow_commands(true);
            game.reload_datapacks(false, &[]).unwrap();
            let marker = |game: &Game<'_>| {
                game.read_template_tag("generation_test:probe")
                    .and_then(|tag| tag.get("marker").cloned())
            };
            assert_eq!(marker(game), Some(Tag::String("old".into())));
            publish("new");
            assert!(game.run_command("datapack disable \"file/templates.zip\""));
            assert_eq!(marker(game), None);
            assert!(game.run_command("datapack enable \"file/templates.zip\""));
            assert_eq!(marker(game), Some(Tag::String("new".into())));
            assert!(
                !scratch
                    .0
                    .join("data/generation_test/structure/probe.nbt")
                    .exists()
            );
        });
        assert!(ran, "the prepared-resource fixture must run this case");
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn world_open_loads_functions_before_first_tick() {
        use crate::game::{native_entity_refs::tests::with_game, world_loading::WorldRequest};

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
        std::fs::write(pack.join("pack.mcmeta"), "{}").unwrap();
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
            game.load_world(WorldRequest {
                rule_registry: std::sync::Arc::new(
                    horizon_client::gamerules::RuleRegistry::vanilla(),
                ),
                save_root: root.clone(),
                folder: "test".into(),
                name: Some("test".into()),
                seed: Some(1),
                creative: true,
                create: None,
            })
            .unwrap();
            assert!(game.fns.store.functions.contains_key("test:start"));
            assert_eq!(
                game.fns.store.tag_functions("minecraft:load"),
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
            game.fns.schedule.push(Scheduled {
                id: "test:start".into(),
                time: 1000,
                seq: 1,
            });
            game.save_schedule();
            game.fns.schedule.clear();
            assert!(game.run_command("datapack disable \"file/test\""));
            assert!(game.fns.schedule.is_empty());
            assert!(game.run_command("datapack enable \"file/test\""));
            assert!(game.fns.schedule.is_empty());

            // World-open selection and completions must both use the incoming
            // level's flags, even while `self.level` is still the old level.
            game.ui.registries.by_registry.clear();
            game.level.enabled_features = Default::default();
            std::fs::write(
                pack.join("pack.mcmeta"),
                r#"{"features":{"enabled":["test:feature"]}}"#,
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
            (game.world_tables, game.health_resources) = game
                .build_world_tables(
                    &stack,
                    false,
                    &game.options.resources.clone(),
                    game.mods.data_switched,
                )
                .unwrap();
            assert!(game.items().item_tags["test:sample"].contains("minecraft:diamond"));
        });
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn unless_function_keeps_a_fallthrough_after_running_it_once() {
        use crate::game::{Audio, Options, Settings, load_resources};
        use horizon_client::datapack::FunctionLine;
        use std::time::Instant;
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &crate::game::Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        game.fns.loaded = true;
        let run = |game: &mut Game<'_>, line: &str| {
            game.exec = ExecState::default();
            let source = game.player_source();
            game.execute_line(line, source).unwrap();
        };
        run(&mut game, "scoreboard objectives add h154 dummy");
        let clock = game.world.game_time;
        for (name, tail) in [
            ("counter", None),
            ("zero", Some("return 0")),
            ("five", Some("return 5")),
            ("fail", Some("return fail")),
        ] {
            let id = format!("h154:{name}");
            let mut lines = vec![FunctionLine::Command(
                "scoreboard players add #calls h154 1".into(),
            )];
            if let Some(tail) = tail {
                lines.push(FunctionLine::Command(tail.into()));
            }
            game.fns.store.functions.insert(
                id.clone(),
                Function {
                    id,
                    lines,
                    parameters: Vec::new(),
                },
            );
        }
        // Explicit returns establish both signs before the old fallthrough
        // failure. The commands run through the actual function/execute route.
        for name in ["zero", "five", "fail", "counter"] {
            for sign in ["if", "unless"] {
                run(&mut game, "scoreboard players set #calls h154 0");
                run(&mut game, "scoreboard players set #out h154 99");
                run(
                    &mut game,
                    &format!(
                        "execute {sign} function h154:{name} run scoreboard players set #out h154 7"
                    ),
                );
                assert_eq!(game.score.board.score("#calls", "h154").unwrap().value, 1);
                let keep = (name == "five") == (sign == "if");
                assert_eq!(
                    game.score.board.score("#out", "h154").unwrap().value,
                    if keep { 7 } else { 99 },
                    "{sign} {name}"
                );
            }
        }
        assert_eq!(game.world.game_time, clock);
    }

    /// Two packs override the same recipe: the later one in the selected
    /// (`datapacks.json`) order wins, not the later one alphabetically.
    #[test]
    fn content_overlay_follows_enabled_order() {
        for order in [["file/a", "file/b"], ["file/b", "file/a"]] {
            let dir = std::env::temp_dir().join(format!(
                "horizon-overlay-{}-{}",
                std::process::id(),
                order[0].replace('/', "_")
            ));
            let _ = std::fs::remove_dir_all(&dir);
            for name in ["a", "b"] {
                let pack = dir.join("datapacks").join(name);
                std::fs::create_dir_all(pack.join("data/x/recipe")).unwrap();
                std::fs::write(pack.join("pack.mcmeta"), "{}").unwrap();
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
            let files = content_overlay(&stack);
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
    /// (`Game::build_world_tables` is both), and sits below the world's own
    /// packs. Each load asks again, so it is the current contribution that
    /// counts.
    #[test]
    fn contributed_files_join_the_load_below_the_worlds_packs() {
        let dir = std::env::temp_dir().join(format!("horizon-contributed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pack = dir.join("datapacks").join("pack");
        std::fs::create_dir_all(pack.join("data/pkg/tags/item")).unwrap();
        std::fs::write(pack.join("pack.mcmeta"), "{}").unwrap();
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
        let passive = horizon_assets::resource_source::ResourceSource::indexed(
            ["shared", "native_only"]
                .map(|name| {
                    (
                        format!("data/pkg/recipe/{name}.json"),
                        horizon_assets::resource_source::Resource::bytes(
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
        let files = overlay_files(
            &stack,
            vec![
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
            ],
        );
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
        assert!(horizon_world::loot::PackOverlay::top(versions).starts_with(b"{\"replace\""));
        let overlay = horizon_world::loot::PackOverlay::new(&files);
        let tags = horizon_items::items::load_tags(&dir, "item", &overlay).unwrap();
        assert!(tags["pkg:both"].contains("pkg:from_pack"));
        assert!(!tags["pkg:both"].contains("pkg:from_mod"));
        // A path only the mod has reaches the tables too.
        assert_eq!(
            tags["pkg:mod_only"].iter().collect::<Vec<_>>(),
            ["pkg:only_mod"]
        );
        // The next load asks again: the new contribution, not the first's.
        let files = overlay_files(
            &stack,
            vec![("tags/pkg:item/mod_only".to_owned(), tag("pkg:second_load"))],
        );
        let overlay = horizon_world::loot::PackOverlay::new(&files);
        let tags = horizon_items::items::load_tags(&dir, "item", &overlay).unwrap();
        assert_eq!(
            tags["pkg:mod_only"].iter().collect::<Vec<_>>(),
            ["pkg:second_load"]
        );
        // A file that isn't JSON is skipped alone, not the whole load.
        let files = overlay_files(
            &stack,
            vec![
                ("tags/pkg:item/mod_only".to_owned(), b"{not json".to_vec()),
                ("tags/pkg:item/good".to_owned(), tag("pkg:good")),
            ],
        );
        let overlay = horizon_world::loot::PackOverlay::new(&files);
        let tags = horizon_items::items::load_tags(&dir, "item", &overlay).unwrap();
        assert!(!tags.contains_key("pkg:mod_only"));
        assert!(tags["pkg:good"].contains("pkg:good"));
        // With nothing contributed the overlay is the packs' own.
        assert_eq!(overlay_files(&stack, Vec::new()), content_overlay(&stack));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod entity_score_tests {
    use super::*;
    use crate::game::{Audio, Options, Progress, Settings, commands::Target, load_resources};
    use horizon_core::nbt::Tag;

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn named_scores_follow_the_registered_target_holder_and_mutations() {
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();

        game.set_allow_commands(true);
        game.ensure_datapacks();
        game.fns.store.predicates.insert(
            "h270:min2".into(),
            serde_json::json!({
                "type": "minecraft:entity_scores", "entity": "this", "scores": {"pts": {"min": 2}}
            }),
        );
        assert!(game.run_command("scoreboard objectives add pts dummy"));
        let mut actors = Vec::new();
        for (index, score) in [Some(0), Some(2), None].into_iter().enumerate() {
            let uuid = [270, 1, 0, index as i32 + 1];
            let tag = Tag::compound([
                ("id", Tag::String("minecraft:pig".into())),
                ("UUID", Tag::IntArray(uuid.to_vec())),
                (
                    "Pos",
                    Tag::List(vec![
                        Tag::Double((3.5 + index as f64).to_bits()),
                        Tag::Double(200.0_f64.to_bits()),
                        Tag::Double(4.5_f64.to_bits()),
                    ]),
                ),
                ("Health", Tag::Float(10.0_f32.to_bits())),
                ("NoAI", Tag::Byte(1)),
                ("NoGravity", Tag::Byte(1)),
            ]);
            let id = game.load_mob(tag, None).unwrap();
            assert_eq!(game.persist.uuids.get(&id), Some(&uuid));
            let holder = game.holder_name(Target::Mob(id)).unwrap();
            assert_eq!(holder, horizon_client::save::uuid_text(uuid));
            if let Some(score) = score {
                assert!(game.run_command(&format!("scoreboard players set {holder} pts {score}")));
            }
            assert_eq!(
                game.score.board.score(&holder, "pts").map(|s| s.value),
                score
            );
            actors.push((id, holder));
        }
        let time = game.world.game_time;
        let matches = |game: &mut Game<'_>, id| {
            let mut source = game.player_source();
            source.entity = Some(Target::Mob(id));
            source.pos = game.target_position(Target::Mob(id)).unwrap();
            game.command_source = Some(source);
            let result = game.check_predicate("h270:min2").unwrap();
            game.command_source = None;
            result
        };
        assert!(!matches(&mut game, actors[0].0));
        assert!(matches(&mut game, actors[1].0));
        assert!(!matches(&mut game, actors[2].0));
        for (index, score, expected) in [(0, 3, true), (1, 1, false)] {
            let (id, holder) = &actors[index];
            assert!(game.run_command(&format!("scoreboard players set {holder} pts {score}")));
            assert_eq!(
                game.score.board.score(holder, "pts").map(|s| s.value),
                Some(score)
            );
            assert_eq!(matches(&mut game, *id), expected);
        }
        assert!(!matches(&mut game, actors[2].0));
        assert!(game.score.board.score(&actors[2].1, "pts").is_none());
        assert_eq!(game.world.game_time, time);
    }
}
