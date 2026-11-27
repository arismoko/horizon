//! Load-scoped registration and per-candidate condition lookup.
use crate::game::native_registry_entries::load_registry_tags;
use mod_api::__private::game::{
    CallbackOwner, DatapackCandidateBuilder, RegistrationContextNew, datapack_entry_id,
    datapack_registry_directory, decode_datapack_entry, valid_namespaced,
};
use mod_api::__private::{NoVanillaValueDecoder, VanillaValueDecoder};
use mod_api::{
    DatapackCandidate, DatapackDecodeError, DatapackLoadReason, DatapackRegistryDeclaration,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Arc,
};
use sunrise_level::datapack::RegistryFiles;
use sunrise_world::{
    conditions::{ConditionProgress, LoadConditions},
    loot::PackOverlay,
};

pub(in crate::game) type ConditionTable =
    Arc<BTreeMap<mod_api::Identifier, (CallbackOwner, Arc<dyn mod_api::LoadCondition>)>>;

impl crate::game::CommonGame<'_> {
    pub fn stage_condition_inventory(
        &mut self,
    ) -> sunrise_game_data::Result<Option<sunrise_assets::resource_source::ResourceRoot>> {
        self.protect_condition_root();
        self.mods.load_condition_candidate = None;
        let inventory = sunrise_mod_package::enabled::EnabledMods::read(&self.options.resources)?;
        self.mods.load_condition_candidate = Some(conditions_for_mods(&self.mods, &inventory)?);
        self.stage_enabled_inventory()
    }

    pub(in crate::game) fn protect_condition_root(&mut self) {
        self.mods.active_condition_root = Some(sunrise_sim::mod_content::retain_root(
            &self.options.resources,
        ));
    }

    pub(in crate::game) fn condition_root_needs_reload(&self) -> bool {
        self.mods
            .load_condition_candidate
            .as_ref()
            .is_some_and(|view| view.has_registered())
            && (!self.mods.selected.is_empty()
                || !self.mods.dev.is_empty()
                || !self.mods.native.packages().is_empty())
    }

    pub fn worldgen_base_conditions(
        &self,
    ) -> std::result::Result<sunrise_world::conditions::LoadConditions, String> {
        let inventory = sunrise_mod_package::enabled::EnabledMods::read(&self.options.resources)
            .map_err(|error| error.to_string())?;
        conditions_for_mods(&self.mods, &inventory)
    }

    /// Stage the enabled packs’ actual winners before readers use their holders.
    pub fn stage_datapack_registries(
        &self,
        stack: &super::function_pack_stack::PackStack,
        root: &Path,
        overlay: &PackOverlay<'_>,
        view: &mut RegistryView,
        inputs: &VanillaValueInputs<'_, '_>,
    ) -> Result<
        Option<(
            DatapackCandidate,
            Option<super::native_datapack_values::DatapackVanillaValues>,
        )>,
        String,
    > {
        let declarations = mod_api::__private::datapack_registry_declarations();
        if declarations.is_empty() {
            return Ok(None);
        }
        let keys: Vec<_> = declarations
            .iter()
            .map(|declaration| declaration.key.as_str())
            .collect();
        let packs: Vec<_> = std::iter::once(&stack.content)
            .chain(stack.enabled_packs())
            .collect();
        let files = sunrise_level::datapack::registry_files(&packs, &keys)
            .map_err(|error| format_datapack_error(&error))?;
        if !mod_api::__private::declarations_use_vanilla_values(declarations) {
            return admit_datapack_registries(
                root,
                overlay,
                view,
                declarations,
                &files,
                &mut NoVanillaValueDecoder,
            )
            .map(|candidate| Some((candidate, None)));
        }
        let biome_tags = sunrise_items::items::load_tags(root, "worldgen/biome", inputs.tag_files)
            .map_err(|error| format!("datapack vanilla value biome tags: {error}"))?;
        let live_features = self.res.behaviours().live_features();
        let catalog = live_features
            .as_ref()
            .and_then(|features| features.state_provider_catalog());
        let biome_names = inputs.biomes.registry_order();
        let mut decoder = super::native_datapack_values::VanillaValueTableBuilder::new(
            self.pack.assets.registry,
            self.res.behaviours(),
            &inputs.items.block_tags,
            &inputs.items.fluid_tags,
            &biome_names,
            &biome_tags,
            catalog,
        )
        .map_err(|error| format!("datapack vanilla value decoder: {error}"))?;
        let candidate =
            admit_datapack_registries(root, overlay, view, declarations, &files, &mut decoder)?;
        Ok(Some((candidate, Some(decoder.finish()))))
    }
}

/// The world's loaded tables a vanilla value decoder reads while datapack registries are staged.
pub struct VanillaValueInputs<'a, 'b> {
    pub items: &'a sunrise_items::items::Items,
    pub tag_files: &'a PackOverlay<'b>,
    pub biomes: &'a sunrise_level::biome::Biomes,
}

#[derive(Default)]
pub struct RegistryView {
    pub ids: BTreeMap<String, BTreeSet<String>>,
    /// Each registry's bound tags, each with whether it holds an entry.
    pub tags: BTreeMap<String, BTreeMap<String, bool>>,
}

impl RegistryView {
    pub fn conditions(&self, base: &LoadConditions) -> LoadConditions {
        base.clone()
            .with_registries(
                self.ids
                    .iter()
                    .map(|(key, ids)| (key.as_str(), ids.iter().map(String::as_str))),
            )
            .with_tags(self.tags.iter().map(|(key, tags)| {
                (
                    key.as_str(),
                    tags.iter().map(|(tag, held)| (tag.as_str(), *held)),
                )
            }))
    }
}

fn format_datapack_error(error: &mod_api::DatapackLoadError) -> String {
    format!(
        "datapack registry {} entry {} (pack {}, file {}): {:?}",
        error.registry, error.entry, error.pack, error.file, error.reason
    )
}

#[cfg(any(test, feature = "test-support"))]
pub fn admit_datapack_registries_for_test(
    root: &Path,
    overlay: &PackOverlay<'_>,
    view: &mut RegistryView,
    declarations: &[DatapackRegistryDeclaration],
    files: &RegistryFiles,
    decoder: &mut dyn VanillaValueDecoder,
) -> Result<DatapackCandidate, String> {
    admit_datapack_registries(root, overlay, view, declarations, files, decoder)
}

fn admit_datapack_registries(
    root: &Path,
    overlay: &PackOverlay<'_>,
    view: &mut RegistryView,
    declarations: &[DatapackRegistryDeclaration],
    files: &RegistryFiles,
    decoder: &mut dyn VanillaValueDecoder,
) -> Result<DatapackCandidate, String> {
    use DatapackLoadReason::{Decode, InvalidEntryId, NotUtf8};
    let base = overlay.conditions;
    let malformed = |message| Decode(DatapackDecodeError::Malformed(message));
    let winners: Vec<_> = files
        .iter()
        .map(|(file, (pack, bytes))| {
            let json = std::str::from_utf8(bytes)
                .map_err(|_| NotUtf8)
                .and_then(|text| {
                    serde_json::from_str::<serde_json::Value>(text)
                        .map(|value| (text, value))
                        .map_err(|error| malformed(error.to_string()))
                });
            (file.as_str(), pack.as_str(), json)
        })
        .collect();
    let programs: Vec<_> = winners
        .iter()
        .map(|(file, _, json)| {
            let (text, json) = json.as_ref().map_err(Clone::clone)?;
            base.decode_file_conditions(json, file)
                .map(|program| (*text, program))
                .map_err(malformed)
        })
        .collect();
    let directories = declarations
        .iter()
        .map(|declaration| {
            datapack_registry_directory(&declaration.key)
                .ok_or_else(|| format!("invalid datapack registry key {}", declaration.key))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let producers: BTreeMap<_, _> = declarations
        .iter()
        .enumerate()
        .map(|(index, declaration)| {
            view.ids.remove(&declaration.key);
            view.tags.remove(&declaration.key);
            (declaration.key.as_str(), index)
        })
        .collect();
    let mut pending: Vec<_> = declarations.iter().map(|_| Vec::new()).collect();
    let mut errors = Vec::new();
    for (index, (declaration, directory)) in declarations.iter().zip(&directories).enumerate() {
        for ((file, pack, _), program) in winners.iter().zip(&programs) {
            let Some(entry) = datapack_entry_id(directory, file) else {
                continue;
            };
            let program = if valid_namespaced(&entry) {
                program.as_ref()
            } else {
                Err(&InvalidEntryId)
            };
            match program {
                Ok((text, program)) => {
                    pending[index].push((entry, *file, *pack, *text, program.evaluation()));
                }
                Err(reason) => errors.push(format!(
                    "datapack registry {} entry {entry} (pack {pack}, file {file}): {reason:?}",
                    declaration.key,
                )),
            }
        }
    }

    let mut builder = DatapackCandidateBuilder::default();
    let mut unbound = vec![true; declarations.len()];
    let mut groups: Vec<_> = (0..declarations.len()).collect();
    let mut cyclic = vec![false; declarations.len()];
    let mut entries = vec![BTreeMap::new(); declarations.len()];
    while unbound.contains(&true) {
        let context = view.conditions(base);
        let mut dependencies = vec![BTreeSet::new(); declarations.len()];
        for (index, declaration) in declarations.iter().enumerate() {
            let group = groups[index];
            pending[index].retain_mut(|(entry, file, pack, text, evaluation)| {
                let decoded = match evaluation.advance(&context, |registry| {
                    producers.get(registry).is_some_and(|&producer| {
                        unbound[producer] && (groups[producer] != group || !cyclic[group])
                    })
                }) {
                    Ok(ConditionProgress::NeedsRegistry(registry)) => {
                        dependencies[group].insert(groups[producers[registry]]);
                        return true;
                    }
                    Ok(ConditionProgress::Complete(false)) => return false,
                    Ok(ConditionProgress::Complete(true)) => {
                        decode_datapack_entry(declaration, text, &mut *decoder).map_err(Decode)
                    }
                    Err(error) => Err(malformed(error)),
                };
                match decoded {
                    Ok(value) => {
                        entries[index].insert(std::mem::take(entry), (value, (*pack).to_owned()));
                    }
                    Err(reason) => errors.push(format!(
                        "datapack registry {} entry {entry} (pack {pack}, file {file}): {reason:?}",
                        declaration.key,
                    )),
                }
                false
            });
        }
        let mut bound_any = false;
        for (index, declaration) in declarations.iter().enumerate() {
            if !unbound[index] || !dependencies[groups[index]].is_empty() {
                continue;
            }
            let entries = std::mem::take(&mut entries[index]);
            let tags = load_registry_tags(root, overlay, &directories[index], |id| {
                entries.contains_key(id)
            })
            .unwrap_or_else(|error| {
                errors.push(format!(
                    "datapack registry {} tags: {error}",
                    declaration.key
                ));
                BTreeMap::new()
            });
            view.ids
                .insert(declaration.key.clone(), entries.keys().cloned().collect());
            view.tags.insert(
                declaration.key.clone(),
                tags.iter()
                    .map(|(tag, members)| (tag.clone(), !members.is_empty()))
                    .collect(),
            );
            builder.stage_registry(&declaration.key, entries, tags);
            unbound[index] = false;
            bound_any = true;
        }
        if !bound_any {
            let component = registry_components(&dependencies)
                .into_iter()
                .rev()
                .find(|members| !dependencies[members[0]].is_empty())
                .expect("nonempty graph of blocked registries");
            let group = component[0];
            cyclic[group] = true;
            for current in &mut groups {
                if component.contains(current) {
                    *current = group;
                }
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    builder
        .finish(declarations)
        .map_err(|error| format_datapack_error(&error))
}

fn builtin_command_registries(
    root: &std::path::Path,
) -> BTreeMap<String, std::collections::BTreeSet<String>> {
    let export: serde_json::Value = std::fs::read(root.join("command-registries.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    export["by_registry"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(registry, entry)| {
            let ids = entry["ids"].as_array().into_iter().flatten();
            let ids = ids.filter_map(serde_json::Value::as_str).map(str::to_owned);
            (registry.clone(), ids.collect())
        })
        .collect()
}

/// Static built-in registries whose tags are bound before `WORLD_REGISTRIES` /
/// `DIMENSION_REGISTRIES` load (`WorldLoader.java.patch` lines 43, 51 `staticLayerTags`).
const WORLDGEN_STATIC_TAG_REGISTRIES: [&str; 6] = [
    "block",
    "item",
    "fluid",
    "entity_type",
    "game_event",
    "villager_profession",
];

/// Builds the `LoadConditions` used by worldgen and dimension pack loaders
/// (`RegistryDataLoader.WORLD_REGISTRIES` / `DIMENSION_REGISTRIES`).
pub fn worldgen_load_conditions(
    base: &sunrise_world::conditions::LoadConditions,
    resources: &std::path::Path,
    packs: &[&sunrise_level::datapack::DataPack],
) -> sunrise_world::conditions::LoadConditions {
    let registries = builtin_command_registries(resources);
    let mut pack_tag_files: BTreeMap<String, Vec<Vec<u8>>> = BTreeMap::new();
    for pack in packs {
        if pack.id == "vanilla" {
            continue;
        }
        for file in pack.list_kinds(&["tags"]) {
            let Some(rest) = file.strip_prefix("data/") else {
                continue;
            };
            let Some((ns, rest)) = rest.split_once('/') else {
                continue;
            };
            let Some(after_tags) = rest.strip_prefix("tags/") else {
                continue;
            };
            let Some((kind, path)) = after_tags.split_once('/') else {
                continue;
            };
            if !WORLDGEN_STATIC_TAG_REGISTRIES.contains(&kind) {
                continue;
            }
            let Some(path) = path.strip_suffix(".json") else {
                continue;
            };
            if let Some(bytes) = pack.read(&file) {
                pack_tag_files
                    .entry(format!("tags/{ns}:{kind}/{path}"))
                    .or_default()
                    .push(bytes);
            }
        }
    }
    let files: Vec<(String, Vec<Vec<u8>>)> = pack_tag_files.into_iter().collect();
    let tag_overlay = sunrise_world::loot::PackOverlay {
        conditions: &sunrise_world::conditions::LoadConditions::vanilla(),
        files: &files,
        ..sunrise_world::loot::PackOverlay::default()
    };
    let builtin_ids = sunrise_world::tag_layers::builtin_ids(resources).unwrap_or_default();
    let mut bound_tags: Vec<(String, Vec<(String, bool)>)> = Vec::new();
    for kind in WORLDGEN_STATIC_TAG_REGISTRIES {
        let ids = builtin_ids.get(kind);
        if let Ok(loaded) = sunrise_world::tag_layers::load(resources, kind, &tag_overlay, |id| {
            ids.is_none_or(|known| known.contains(id))
        }) {
            let tags: Vec<(String, bool)> = loaded
                .into_iter()
                .map(|(tag, members)| (tag, !members.is_empty()))
                .collect();
            bound_tags.push((format!("minecraft:{kind}"), tags));
        }
    }
    base.clone()
        .with_features(["minecraft:vanilla"])
        .with_registries(
            registries
                .iter()
                .map(|(registry, ids)| (registry.as_str(), ids.iter().map(String::as_str))),
        )
        .with_tags(bound_tags.iter().map(|(registry, tags)| {
            (
                registry.as_str(),
                tags.iter().map(|(tag, held)| (tag.as_str(), *held)),
            )
        }))
}

/// Strongly connected components, consumers before their dependencies
/// (Kosaraju's two traversals). Pack input controls the depth, so use explicit stacks.
fn registry_components(dependencies: &[BTreeSet<usize>]) -> Vec<Vec<usize>> {
    let mut consumers = vec![Vec::new(); dependencies.len()];
    for (consumer, producers) in dependencies.iter().enumerate() {
        for &producer in producers {
            consumers[producer].push(consumer);
        }
    }
    let mut seen = vec![false; dependencies.len()];
    let mut finished = Vec::with_capacity(dependencies.len());
    let mut stack = Vec::new();
    for root in 0..dependencies.len() {
        stack.push((root, false));
        while let Some((index, leaving)) = stack.pop() {
            if leaving {
                finished.push(index);
            } else if !std::mem::replace(&mut seen[index], true) {
                stack.push((index, true));
                stack.extend(
                    dependencies[index]
                        .iter()
                        .map(|&producer| (producer, false)),
                );
            }
        }
    }
    seen.fill(false);
    let mut components = Vec::new();
    let mut stack = Vec::new();
    for root in finished.into_iter().rev() {
        if seen[root] {
            continue;
        }
        let mut component = Vec::new();
        stack.push(root);
        while let Some(index) = stack.pop() {
            if !std::mem::replace(&mut seen[index], true) {
                component.push(index);
                stack.extend(consumers[index].iter().copied());
            }
        }
        components.push(component);
    }
    components
}

pub fn conditions_for_mods(
    sources: &super::mods::ModsState,
    inventory: &sunrise_mod_package::enabled::EnabledMods,
) -> Result<sunrise_world::conditions::LoadConditions, String> {
    if let Some(candidate) = &sources.load_condition_candidate {
        return Ok(candidate.with_mod_inventory(inventory.server_ids()));
    }
    let table = sources
        .load_conditions
        .get_or_init(|| {
            let mut registrar = mod_api::LoadConditionDeclarations::new(
                sunrise_world::conditions::is_builtin_condition,
            );
            crate::game::native_hooks::native_register_load_conditions(
                &mod_api::RegistrationContext::new(),
                &mut registrar,
            );
            let entries = registrar.finish().map_err(|error| error.message)?;
            // Every candidate shares the owner of each retained registration.
            Ok(Arc::new(
                entries
                    .into_iter()
                    .map(|(id, (owner, handler))| (id, (CallbackOwner::for_mod(owner), handler)))
                    .collect(),
            ))
        })
        .clone()?;
    let conditions = sunrise_world::conditions::LoadConditions::new(inventory.server_ids());
    Ok(
        conditions.with_lookup(table.iter().map(|(id, (owner, handler))| {
            let name = id.to_string();
            let enabled = sources.code_owner_enabled(owner.id());
            let owner = owner.clone();
            let handler = handler.clone();
            (name.clone(), move |args: &serde_json::Value| {
                if !enabled {
                    return Ok(false);
                }
                // The evaluator passes the arguments without the key that named the condition.
                owner
                    .call("load condition", "test_load_condition", || {
                        handler.test(args)
                    })
                    .map_err(|reason| format!("condition {name}: {reason}"))?
                    .map_err(|error| format!("condition {name}: mod {}: {error}", owner.id()))
            })
        })),
    )
}

pub(in crate::game) fn content_root_with_conditions(
    root: &std::path::Path,
    jars: &std::path::Path,
    mods: &[sunrise_sim::mod_content::ContentMod],
    cache: &std::path::Path,
    inventory: &sunrise_mod_package::enabled::EnabledMods,
    sources: &super::mods::ModsState,
) -> sunrise_game_data::Result<std::path::PathBuf> {
    let conditions = conditions_for_mods(sources, inventory)?;
    let conditions = worldgen_load_conditions(&conditions, root, &[]);
    sunrise_sim::mod_content::resource_root_with_conditions(
        root,
        jars,
        mods,
        cache,
        inventory,
        &conditions,
    )
}
