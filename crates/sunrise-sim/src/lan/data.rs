//! What the configuration phase sends: the synchronized registries (names in
//! network order, with their data for clients that do not know the vanilla
//! pack) and the tags of every registry, loaded from the prepared resources
//! (`data/minecraft/...`) and the registries report.
use crate::{advancements::Tree, chunk::LevelShapes, items::Items};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};
use sunrise_core::nbt::Tag;
use sunrise_protocol::item::consume_effects;
use sunrise_protocol::{
    entity_data,
    packets::configuration::{RegistryData, RegistryEntry, RegistryTags, TagEntry, UpdateTags},
    registry::{Builtin, Dynamic},
};
use sunrise_world::worldgen::biome::BiomeDefinition;
use sunrise_world::{damage_source::Catalog, level::classes::is_generated_tag};

/// RegistryDataLoader.SYNCHRONIZED_REGISTRIES, in vanilla's order.
pub const SYNCHRONIZED: &[&str] = &[
    "minecraft:worldgen/biome",
    "minecraft:chat_type",
    "minecraft:trim_pattern",
    "minecraft:trim_material",
    "minecraft:wolf_variant",
    "minecraft:wolf_sound_variant",
    "minecraft:pig_variant",
    "minecraft:pig_sound_variant",
    "minecraft:frog_variant",
    "minecraft:cat_variant",
    "minecraft:cat_sound_variant",
    "minecraft:cow_sound_variant",
    "minecraft:cow_variant",
    "minecraft:chicken_sound_variant",
    "minecraft:chicken_variant",
    "minecraft:zombie_nautilus_variant",
    "minecraft:painting_variant",
    "minecraft:sulfur_cube_archetype",
    "minecraft:dimension_type",
    "minecraft:damage_type",
    "minecraft:banner_pattern",
    "minecraft:enchantment",
    "minecraft:jukebox_song",
    "minecraft:instrument",
    "minecraft:test_environment",
    "minecraft:test_instance",
    "minecraft:dialog",
    "minecraft:world_clock",
    "minecraft:timeline",
    "minecraft:decorated_pot_pattern",
    "minecraft:block_transformer",
    "minecraft:worldgen/block_state_provider",
];

/// Append validated load declarations after vanilla, preserving its numeric IDs.
/// Both the LAN server and a client's prepared built-ins use this same ordering.
pub fn with_custom_stats(mut builtin: Builtin) -> Builtin {
    let custom = mod_api::__private::custom_stat_declarations();
    if !custom.is_empty() {
        let ids = builtin
            .registries
            .entry("minecraft:custom_stat".into())
            .or_default();
        let mut names = std::mem::take(&mut ids.names);
        for id in custom {
            if !names.contains(id) {
                names.push(id.clone());
            }
        }
        *ids = sunrise_protocol::registry::Ids::from_names(names);
    }
    builtin
}
pub const BIOME_REGISTRY: &str = "minecraft:worldgen/biome";
/// The table a client plays a server with: `known`, the biomes of the
/// server's biome registry that it lacks built from their data, and the ones
/// it has whose data differs from its own file (a pack's or a mod's
/// replacement) replaced, keeping their ids; None when there are none.
pub fn client_biomes(
    known: &crate::biome::Biomes,
    registry: &[(String, Option<Tag>)],
) -> crate::Result<Option<crate::biome::Biomes>> {
    let files: Vec<(&str, Value)> = registry
        .iter()
        .filter_map(|(name, data)| {
            let data = data.as_ref()?;
            // Compare the network projection, not the local generation settings.
            let same = known
                .builtin_file(name)
                .and_then(|own| BiomeDefinition::encode_network(own).ok())
                .is_some_and(|own| &own == data);
            (!same).then(|| (name.as_str(), crate::scoreboard::tag_to_json(data)))
        })
        .collect();
    // A name the registry lists with no data that this game does not define: it keeps a slot
    // under its own key, so a stored biome of it is never read back as plains.
    let unresolved: Vec<&str> = registry
        .iter()
        .filter(|(name, data)| data.is_none() && !known.contains(name))
        .map(|(name, _)| name.as_str())
        .collect();
    if files.is_empty() && unresolved.is_empty() {
        return Ok(None);
    }
    let table = known
        .with_packs(files.iter().map(|(n, v)| (*n, v)))?
        .with_unresolved(unresolved.iter().copied())?;
    log::info!(
        "Sunrise: the server has {} biomes the game lacks or changes, {} without data",
        files.len(),
        unresolved.len()
    );
    Ok(Some(table))
}
/// KnownPack.vanilla("core") for this version.
pub use sunrise_game_data::core_pack::CORE_PACK;

#[path = "data_pack_registries.rs"]
mod pack_registries;
use pack_registries::bind_dialog_tags;
pub use pack_registries::{
    ChatType, PackRegistryDefinitions, REGISTRY_KINDS, RegistryKind, RegistryRef,
};

/// Bind one registry's resource and pack tags to its actual holders. Shared by
/// WorldLoader's pending condition context and the configuration tag snapshot.
pub fn registry_tag_bindings(
    resources: &Path,
    overlay: &sunrise_world::loot::PackOverlay<'_>,
    registry: &str,
    contains: impl Fn(&str) -> bool,
) -> Result<BTreeMap<String, Vec<String>>, String> {
    if registry == "minecraft:dialog" {
        // Dialog registry admission also tracks missing writable holders.
        return Ok(bind_dialog_tags(&resources.join("data/minecraft"), overlay, contains).0);
    }
    let kind = registry.split_once(':').map_or(registry, |(_, path)| path);
    sunrise_world::tag_layers::load(resources, kind, overlay, contains)
        .map(|tags| tags.into_iter().collect())
        .map_err(|error| error.to_string())
}

/// World overlays key definition files by `registry/id`.
pub fn is_pack_registry_file(key: &str) -> bool {
    key.starts_with("worldgen/biome/")
        || key.split_once('/').is_some_and(|(kind, _)| {
            REGISTRY_KINDS
                .iter()
                .any(|table| table.key.strip_prefix("minecraft:") == Some(kind))
        })
}

/// A registry's entries from the resource root's namespaces other than
/// `minecraft` (`data/<ns>/<dir>/`): content mods' own, marked as no client's.
fn mod_entries(resources: &Path, dir: &str, entries: &mut Vec<((String, String), Value, bool)>) {
    let namespaces = sunrise_world::json_folder::namespaces(&resources.join("data"));
    for ns in namespaces.into_iter().filter(|ns| ns != "minecraft") {
        let mut found = BTreeMap::new();
        walk(
            &resources.join("data").join(&ns).join(dir),
            &ns,
            &mut found,
            dir == "enchantment",
        );
        entries.extend(found.into_iter().map(|(p, v)| ((ns.clone(), p), v, true)));
    }
}

/// A data-driven registry: entry names in network order and their NBT.
#[derive(Clone)]
struct Entries {
    names: Vec<String>,
    data: Vec<Tag>,
    /// The entry comes from a world's data pack, which no client knows, so
    /// its data is always sent.
    from_pack: Vec<bool>,
}

/// The damage registry already bound for gameplay at world entry. Keeping
/// only these entries lets unrelated pack data remain lazy until a hub needs it.
pub struct FrozenDamage(Entries);

impl FrozenDamage {
    pub fn new(
        catalog: &sunrise_world::damage_source::Catalog,
        overlay: &sunrise_world::loot::PackOverlay<'_>,
    ) -> Self {
        let packed: HashSet<_> = overlay.kind("damage_type").map(|(id, _)| id).collect();
        let mut definitions: Vec<_> = catalog.entries().map(|(_, d)| d).collect();
        definitions.sort_by_cached_key(|d| {
            let (ns, path) = d.name.split_once(':').expect("canonical damage identifier");
            (format!("{path}.json"), ns.to_owned())
        });
        Self(Entries {
            names: definitions.iter().map(|d| d.name.clone()).collect(),
            data: definitions.iter().map(|d| d.to_nbt()).collect(),
            from_pack: definitions
                .iter()
                .map(|d| !d.name.starts_with("minecraft:") || packed.contains(d.name.as_str()))
                .collect(),
        })
    }
}

/// Tags already bound by the world. Lazy wire loading must not re-read these files.
pub struct WorldTags {
    items: Arc<Items>,
    damage: sunrise_world::loot::RegistryTags,
}

impl WorldTags {
    pub fn new(items: Arc<Items>, damage: &Catalog) -> Self {
        Self {
            items,
            damage: damage
                .tags()
                .map(|tag| {
                    (
                        tag.to_owned(),
                        damage.tag_members(tag).map(str::to_owned).collect(),
                    )
                })
                .collect(),
        }
    }

    fn for_registry(&self, registry: &str) -> Option<&sunrise_world::loot::RegistryTags> {
        let kind = registry.strip_prefix("minecraft:")?;
        if kind == "damage_type" {
            Some(&self.damage)
        } else {
            self.items
                .ordered_tags()
                .into_iter()
                .find_map(|(name, tags)| (name == kind).then_some(tags))
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SyncLoad {
    Fresh,
    SameFiles,
    Reload,
}

pub struct SyncData {
    pub content_identity: Option<[u8; 32]>,
    pub builtin: Builtin,
    /// Entry names per synchronized registry (for id lookups).
    pub dynamic: Dynamic,
    registries: Vec<(String, Entries)>,
    packed_registries: Option<Arc<PackRegistryDefinitions>>,
    pub tags: UpdateTags,
    pub features: Vec<String>,
    /// Vanilla's command tree (the commands report), when found.
    pub commands: Option<super::commands::CommandTree>,
    /// ClientboundUpdateRecipesPacket's payload.
    pub recipes: Vec<u8>,
    pub stonecutter_buttons: Arc<super::recipes::StonecutterButtons>,
    pub advancements: super::advancements::ServerAdvancements,
    /// The registry ids of the particle types effects show, looked up once (`effect_meta`).
    effect_particle_ids: std::sync::Mutex<super::effect_meta::ParticleTypeIds>,
}

/// Every `*.json` under `dir`, as (path without extension relative to dir, value), sorted;
/// a file that can't be read or doesn't parse is left out. Through `json_folder`: from the
/// prepared data file, and read once for the tables a load builds from the same folder.
fn walk(dir: &Path, namespace: &str, out: &mut BTreeMap<String, Value>, enchantment: bool) {
    let Ok(folder) = sunrise_world::json_folder::read(dir) else {
        return;
    };
    for file in &folder.files {
        if !sunrise_world::json_folder::valid_resource_path(namespace, &file.name) {
            continue;
        }
        let bytes = file.bytes().unwrap_or_default();
        let value = if enchantment {
            sunrise_world::json_folder::enchantment_definition(
                bytes,
                &dir.join(format!("{}.json", file.name)),
            )
        } else {
            serde_json::from_slice::<Value>(bytes).ok()
        };
        if let Some(value) = value {
            out.insert(file.name.clone(), value);
        }
    }
}

fn synchronized_entries(
    root: &Path,
    resources: &Path,
    key: &str,
) -> Vec<((String, String), Value, bool)> {
    let path = key.trim_start_matches("minecraft:");
    let enchantments = key == "minecraft:enchantment";
    let mut found = BTreeMap::new();
    walk(&root.join(path), "minecraft", &mut found, enchantments);
    let mut entries: Vec<_> = found
        .into_iter()
        .map(|(id, value)| (("minecraft".to_owned(), id), value, false))
        .collect();
    if enchantments
        || matches!(
            path,
            "enchantment_provider"
                | "instrument"
                | "jukebox_song"
                | "timeline"
                | "wolf_variant"
                | "wolf_sound_variant"
                | "cat_variant"
                | "cat_sound_variant"
                | "pig_variant"
                | "pig_sound_variant"
                | "cow_variant"
                | "cow_sound_variant"
                | "chicken_variant"
                | "chicken_sound_variant"
                | "frog_variant"
                | "zombie_nautilus_variant"
        )
    {
        mod_entries(resources, path, &mut entries);
    }
    if key == "minecraft:world_clock" {
        // The mods' clocks (`register_world_clock`), below the packs: a `WorldClock` has no field.
        for id in mod_api::__private::world_clock_declarations() {
            if let Some((ns, path)) = id.split_once(':') {
                entries.push((
                    (ns.to_owned(), path.to_owned()),
                    Value::Object(<_>::default()),
                    true,
                ));
            }
        }
    }
    if key == "minecraft:dimension_type" {
        mod_dimension_types(
            sunrise_world::worldgen::overrides::mod_dimensions(),
            &mut entries,
        );
    }
    entries
}

/// The mods' dimension types, below the packs': the overworld's with the
/// mod's fields over it. No client knows them, so their data is always sent.
fn mod_dimension_types(
    mods: &[sunrise_world::worldgen::overrides::ModDimension],
    entries: &mut Vec<((String, String), Value, bool)>,
) {
    let overworld = entries
        .iter()
        .find(|((ns, path), ..)| ns == "minecraft" && path == "overworld")
        .map(|(_, value, _)| value.clone());
    for dimension in mods {
        let Some((ns, path)) = dimension.id.split_once(':') else {
            continue;
        };
        let mut value = overworld
            .clone()
            .unwrap_or_else(|| Value::Object(<_>::default()));
        let fields = serde_json::from_str::<Value>(&dimension.dimension_type);
        if let (Some(value), Ok(Value::Object(mut fields))) = (value.as_object_mut(), fields) {
            // The mod's attributes go over the Overworld's one by one, not in place of its sky
            // and fog attributes.
            let own = fields.remove("attributes");
            value.extend(fields);
            match (value.get_mut("attributes"), own) {
                (Some(Value::Object(base)), Some(Value::Object(own))) => base.extend(own),
                (_, Some(own)) => {
                    value.insert("attributes".into(), own);
                }
                _ => {}
            }
        }
        entries.push(((ns.to_owned(), path.to_owned()), value, true));
    }
}

/// ResourceManagerRegistryLoadTask.load: the entries register sorted by the
/// file `Identifier` FileToIdConverter.listMatchingResources keys them by
/// (`<dir>/<path>.json`), path first, then namespace (Identifier.compareTo).
/// The `.json` counts: `x-y` comes before `x`, as `-` sorts before `.`.
fn file_order<T>(entries: &mut [((String, String), T, bool)]) {
    entries.sort_by_cached_key(|((ns, path), ..)| (format!("{path}.json"), ns.clone()));
}

/// The JSON-to-NBT projection shared by pack loading and text component encoding.
pub use sunrise_game_data::json_nbt::{component_nbt, json_to_nbt};

pub const OPEN_COMMAND_BLOCK_CHANNEL: &str = "sunrise:open_command_block";

pub fn encode_open_command_block(pos: [i32; 3]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12);
    out.extend_from_slice(&pos[0].to_be_bytes());
    out.extend_from_slice(&pos[1].to_be_bytes());
    out.extend_from_slice(&pos[2].to_be_bytes());
    out
}

pub fn decode_open_command_block(data: &[u8]) -> Option<(i32, i32, i32)> {
    let bytes: [u8; 12] = data.try_into().ok()?;
    let x = i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let y = i32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let z = i32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    Some((x, y, z))
}

impl SyncData {
    /// Admit the typed pack winners and collect conditions from the other
    /// files used by the lazy wire reader, even if typed admission fails.
    /// Keep tombstones available for dependent diagnostics on a refused candidate.
    /// The world's damage catalog is already staged, and supplies its frozen wire data.
    pub fn stage_registry_definitions(
        resources: &Path,
        defaults: &sunrise_world::item_defaults::ItemDefaults,
        overlay: &sunrise_world::loot::PackOverlay<'_>,
        read_errors: &BTreeMap<String, String>,
        text_registries: &crate::command::Registries,
        sound_exists: impl Fn(&str) -> bool,
    ) -> (PackRegistryDefinitions, Result<(), String>) {
        let (mut packed, admitted) = PackRegistryDefinitions::stage(
            resources,
            defaults,
            overlay,
            read_errors,
            text_registries,
            sound_exists,
        );
        let root = resources.join("data/minecraft");
        for &key in SYNCHRONIZED {
            if matches!(key, "minecraft:damage_type" | BIOME_REGISTRY)
                || PackRegistryDefinitions::includes_prepared_entries(key)
            {
                continue;
            }
            let path = key.trim_start_matches("minecraft:");
            let mut entries = synchronized_entries(&root, resources, key);
            // Admission already checked the typed winners' conditions.
            packed.discard_replaced(key, &mut entries);
            condition_filter::filter(&mut entries, false, overlay.conditions, path);
            packed.prepared_ids.extend(
                entries
                    .into_iter()
                    .map(|((namespace, path), ..)| (key, format!("{namespace}:{path}"))),
            );
        }
        (packed, admitted)
    }

    /// No registries, tags or recipes: enough for tests of the login phase.
    pub fn empty() -> Self {
        Self {
            effect_particle_ids: Default::default(),
            builtin: Builtin::default(),
            dynamic: Dynamic::default(),
            registries: Vec::new(),
            packed_registries: None,
            tags: UpdateTags {
                registries: Vec::new(),
            },
            features: Vec::new(),
            commands: None,
            recipes: Vec::new(),
            stonecutter_buttons: Arc::default(),
            advancements: super::advancements::ServerAdvancements::load(
                Path::new(""),
                &sunrise_world::loot::PackOverlay::NONE,
                super::recipes::BookProjections::default(),
            ),
            content_identity: None,
        }
    }
    pub fn load(
        resources: &Path,
        overlay: &sunrise_world::loot::PackOverlay,
        defaults: &sunrise_world::item_defaults::ItemDefaults,
        sources: &crate::crafting::ResolvedSources,
    ) -> Result<Self, String> {
        Self::load_with(resources, overlay, None, defaults, sources)
    }

    /// `load` with the open world's biomes: when its data packs added or
    /// replaced some, the biome registry is that table's, in its order.
    pub fn load_with(
        resources: &Path,
        overlay: &sunrise_world::loot::PackOverlay,
        biomes: Option<&crate::biome::Biomes>,
        defaults: &sunrise_world::item_defaults::ItemDefaults,
        sources: &crate::crafting::ResolvedSources,
    ) -> Result<Self, String> {
        Self::load_with_kept(resources, overlay, biomes, None, defaults, sources)
    }

    /// Reload tags and reloadable data against the registries frozen at open,
    /// using the same advancement snapshot as the owning world's reload.
    pub fn reload(
        resources: &Path,
        overlay: &sunrise_world::loot::PackOverlay,
        kept: &Self,
        defaults: &sunrise_world::item_defaults::ItemDefaults,
        sources: &crate::crafting::ResolvedSources,
        advancements: &Arc<Tree>,
        world_tags: Option<&WorldTags>,
    ) -> Result<Self, String> {
        Self::load_with_frozen(
            resources,
            overlay,
            None,
            Some(kept),
            None,
            None,
            SyncLoad::Reload,
            defaults,
            sources,
            Some(advancements),
            world_tags,
        )
    }

    /// Load reloadable data with frozen synchronized registries; an explicit
    /// biome table supplies the world's already-loaded biome registry.
    pub fn load_with_kept(
        resources: &Path,
        overlay: &sunrise_world::loot::PackOverlay,
        biomes: Option<&crate::biome::Biomes>,
        kept: Option<&Self>,
        defaults: &sunrise_world::item_defaults::ItemDefaults,
        sources: &crate::crafting::ResolvedSources,
    ) -> Result<Self, String> {
        Self::load_with_frozen(
            resources,
            overlay,
            biomes,
            kept,
            None,
            None,
            SyncLoad::Fresh,
            defaults,
            sources,
            None,
            None,
        )
    }

    /// [`Self::load_with_kept`] over the packs `kept` was itself loaded with (a world's own
    /// biomes or flags over the data its packs gave): the recipes and advancements, read
    /// from the same files, are `kept`'s rather than read again (~0.7 s with a large modpack).
    pub fn load_over(
        resources: &Path,
        overlay: &sunrise_world::loot::PackOverlay,
        biomes: Option<&crate::biome::Biomes>,
        kept: &Self,
        defaults: &sunrise_world::item_defaults::ItemDefaults,
        sources: &crate::crafting::ResolvedSources,
    ) -> Result<Self, String> {
        Self::load_with_frozen(
            resources,
            overlay,
            biomes,
            Some(kept),
            None,
            None,
            SyncLoad::SameFiles,
            defaults,
            sources,
            None,
            None,
        )
    }

    /// First wire load after world entry: use the damage snapshot frozen at
    /// open and the staged advancement tree, which `/reload` can replace.
    pub fn load_with_frozen_damage(
        resources: &Path,
        overlay: &sunrise_world::loot::PackOverlay,
        damage: &FrozenDamage,
        biomes: Option<&crate::biome::Biomes>,
        packed: Option<&Arc<PackRegistryDefinitions>>,
        defaults: &sunrise_world::item_defaults::ItemDefaults,
        sources: &crate::crafting::ResolvedSources,
        advancements: Option<&Arc<Tree>>,
        world_tags: Option<&WorldTags>,
    ) -> Result<Self, String> {
        Self::load_with_frozen(
            resources,
            overlay,
            biomes,
            None,
            Some(damage),
            packed,
            SyncLoad::Fresh,
            defaults,
            sources,
            advancements,
            world_tags,
        )
    }

    fn load_with_frozen(
        resources: &Path,
        overlay: &sunrise_world::loot::PackOverlay,
        biomes: Option<&crate::biome::Biomes>,
        kept: Option<&Self>,
        damage: Option<&FrozenDamage>,
        packed: Option<&Arc<PackRegistryDefinitions>>,
        mode: SyncLoad,
        defaults: &sunrise_world::item_defaults::ItemDefaults,
        sources: &crate::crafting::ResolvedSources,
        advancements: Option<&Arc<Tree>>,
        world_tags: Option<&WorldTags>,
    ) -> Result<Self, String> {
        use crate::spans::startup;
        // The recipe book, the recipe sync and the advancements read the
        // recipe folder: held here, they share one read of it.
        let _recipe_files = startup::time("sync.recipe_folder", || {
            sunrise_world::json_folder::read(&resources.join("data/minecraft/recipe"))
        });
        let registries_step = startup::step("sync.registries");
        let builtin = with_custom_stats(
            Builtin::find(resources)
                .and_then(|p| Builtin::load(&p).ok())
                .ok_or("registries.json not found next to the resources")?,
        );
        let packed = if let Some(kept) = kept {
            kept.packed_registries.clone()
        } else if let Some(packed) = packed {
            Some(packed.clone())
        } else {
            let text_registries = crate::command::Registries {
                items: defaults.entries().keys().cloned().collect(),
                entity_types: builtin
                    .get("minecraft:entity_type")
                    .map_or_else(Vec::new, |ids| ids.names.clone()),
                ..Default::default()
            };
            Some(Arc::new(PackRegistryDefinitions::load(
                resources,
                defaults,
                overlay,
                &BTreeMap::new(),
                &text_registries,
                |id| {
                    builtin
                        .get("minecraft:sound_event")
                        .and_then(|sounds| sounds.id(id))
                        .is_some()
                },
            )?))
        };
        let root: PathBuf = resources.join("data/minecraft");
        let mut registries = Vec::new();
        let mut dynamic = Dynamic {
            custom_components: sunrise_protocol::item::custom::entries().next().is_some(),
            ..Default::default()
        };
        for key in SYNCHRONIZED {
            if let Some(kept) = kept.filter(|_| *key != BIOME_REGISTRY || biomes.is_none()) {
                if let Some((_, entries)) = kept.registries.iter().find(|(name, _)| name == key) {
                    dynamic
                        .registries
                        .insert((*key).into(), kept.dynamic.registries[*key].clone());
                    registries.push(((*key).into(), entries.clone()));
                }
                continue;
            }
            if *key == "minecraft:damage_type"
                && let Some(damage) = damage
            {
                dynamic.registries.insert(
                    (*key).into(),
                    damage
                        .0
                        .names
                        .iter()
                        .map(|name| (name.clone(), None))
                        .collect(),
                );
                registries.push(((*key).into(), damage.0.clone()));
                continue;
            }
            if *key == BIOME_REGISTRY {
                let prepared;
                let modified = crate::mod_content::biomes::modified(resources);
                let table = match biomes {
                    Some(table) => table,
                    None => {
                        if !modified.is_empty() {
                            return Err(
                                "modified biomes require the world's admitted biome table".into()
                            );
                        }
                        prepared = crate::biome::Biomes::load_vanilla_for_server(
                            &sunrise_assets::resource_source::ResourceRoot::directory(
                                resources.to_owned(),
                            ),
                        )
                        .map_err(|error| error.to_string())?;
                        &prepared
                    }
                };
                let names = table.registry_order();
                let entries = Entries {
                    data: names
                        .iter()
                        .map(|name| {
                            let file = table
                                .pack_file(name)
                                .or_else(|| table.builtin_file(name))
                                .ok_or_else(|| {
                                format!("biome {name}: missing admitted definition")
                            })?;
                            BiomeDefinition::encode_network(file)
                                .map_err(|error| format!("biome {name}: {error}"))
                        })
                        .collect::<Result<_, _>>()?,
                    from_pack: names
                        .iter()
                        .map(|&name| table.pack_file(name).is_some() || modified.contains(name))
                        .collect(),
                    names: names.into_iter().map(str::to_owned).collect(),
                };
                dynamic.registries.insert(
                    (*key).into(),
                    entries.names.iter().map(|id| (id.clone(), None)).collect(),
                );
                registries.push(((*key).into(), entries));
                continue;
            }
            let path = key.trim_start_matches("minecraft:");
            let mut entries = if PackRegistryDefinitions::includes_prepared_entries(key) {
                Vec::new()
            } else {
                synchronized_entries(&root, resources, key)
            };
            let mut damage_entries = None;
            if *key == "minecraft:damage_type" {
                // Validate the actual resource bytes before publishing any
                // registry data. Small tools can supply just these files,
                // without the gameplay-only exported damage catalog.
                let mut files = sunrise_world::damage_source::resource_files(resources, path)
                    .map_err(|e| e.to_string())?;
                let packed: HashSet<_> = overlay.kind(path).map(|(id, _)| id).collect();
                for (id, versions) in overlay.kind(path) {
                    files.insert(
                        id.into(),
                        sunrise_world::loot::PackOverlay::top(versions).to_vec(),
                    );
                }
                let mut decoded = Vec::new();
                for (id, bytes) in files {
                    // Only the load conditions are decoded as a `Value`: the definition's own
                    // parser accepts numbers `Value` refuses (an exhaustion of `1e400` is
                    // infinity, as in vanilla), so the whole file must not go through it.
                    let top: BTreeMap<String, &serde_json::value::RawValue> =
                        serde_json::from_slice(&bytes)
                            .map_err(|e| format!("damage type {id}: {e}"))?;
                    if let Some(raw) = top.get("neoforge:conditions") {
                        let payload: Value = serde_json::from_str(raw.get())
                            .map_err(|e| format!("damage type {id}: {e}"))?;
                        let conditions = serde_json::json!({ "neoforge:conditions": payload });
                        if !overlay.conditions.allows_id(&conditions, path, &id) {
                            continue;
                        }
                    }
                    let definition =
                        sunrise_world::damage_source::Definition::from_json(&id, &bytes)
                            .map_err(|e| format!("damage type {id}: {e}"))?;
                    let (ns, p) = id.split_once(':').ok_or("invalid damage identifier")?;
                    decoded.push((
                        (ns.into(), p.into()),
                        definition.to_nbt(),
                        ns != "minecraft" || packed.contains(id.as_str()),
                    ));
                }
                damage_entries = Some(decoded);
            }
            if let Some(packed) = &packed {
                packed.discard_replaced(key, &mut entries);
            }
            condition_filter::filter(
                &mut entries,
                damage_entries.is_some(),
                overlay.conditions,
                path,
            );
            let mut entries = if let Some(damage_entries) = damage_entries {
                damage_entries
            } else {
                entries
                    .into_iter()
                    .map(|(key, value, packed)| (key, json_to_nbt(&value), packed))
                    .collect()
            };
            if let Some(packed) = &packed {
                packed.append_encoded(key, &mut entries);
            }
            file_order(&mut entries);
            let mut e = Entries {
                names: Vec::new(),
                data: Vec::new(),
                from_pack: Vec::new(),
            };
            for ((ns, p), v, from_pack) in entries {
                e.names.push(format!("{ns}:{p}"));
                e.data.push(v);
                e.from_pack.push(from_pack);
            }
            dynamic.registries.insert(
                (*key).to_owned(),
                e.names.iter().map(|n| (n.clone(), None)).collect(),
            );
            registries.push(((*key).to_owned(), e));
        }
        if consume_effects::has_custom_types() {
            let (names, data): (Vec<_>, Vec<_>) = consume_effects::configuration()
                .map(|(name, schema)| (name.to_owned(), Tag::Long(schema as i64)))
                .unzip();
            dynamic
                .add_consume_effects(RegistryData {
                    registry: consume_effects::REGISTRY.to_owned(),
                    entries: names
                        .iter()
                        .zip(&data)
                        .map(|(name, schema)| RegistryEntry {
                            id: name.clone(),
                            data: Some(schema.clone()),
                        })
                        .collect(),
                })
                .map_err(|error| error.to_string())?;
            let from_pack = vec![true; names.len()];
            registries.push((
                consume_effects::REGISTRY.to_owned(),
                Entries {
                    names,
                    data,
                    from_pack,
                },
            ));
        }
        // Merge the world's tag files against the admitted registry IDs above.
        drop(registries_step);
        let tags_step = startup::step("sync.tags");
        let same_files = kept
            .filter(|_| mode == SyncLoad::SameFiles)
            .map(|kept| (&kept.tags, biomes.is_some()));
        let mut tags = Self::load_tags(
            resources, overlay, &builtin, &dynamic, world_tags, same_files,
        )?;
        if let Some(kept) = kept.filter(|_| mode == SyncLoad::Reload) {
            // TagLoader.loadPendingTags does not rebind a registry when
            // no tags were built. Play reloads and future joins both send
            // the complete current snapshot, including retained bindings.
            let loaded: HashSet<_> = tags
                .registries
                .iter()
                .map(|r| r.registry.as_str())
                .collect();
            let retained: Vec<_> = kept
                .tags
                .registries
                .iter()
                .filter(|r| {
                    !loaded.contains(r.registry.as_str())
                        && world_tags
                            .and_then(|tags| tags.for_registry(&r.registry))
                            .is_none()
                })
                .cloned()
                .collect();
            tags.registries.extend(retained);
        }
        drop(tags_step);
        let content_identity = if resources.join(crate::content_identity::MARKER).exists()
            || crate::content_identity::requires_sunrise()
        {
            Some(
                startup::time("sync.content_identity", || {
                    crate::content_identity::identity(resources, defaults)
                })
                .map_err(|error| error.to_string())?,
            )
        } else {
            None
        };
        let mut sync = Self {
            effect_particle_ids: Default::default(),
            content_identity,
            builtin,
            dynamic,
            registries,
            tags,
            features: vec!["minecraft:vanilla".to_owned()],
            commands: startup::time("sync.commands", || {
                super::commands::CommandTree::load(resources)
            }),
            recipes: Vec::new(),
            stonecutter_buttons: Arc::default(),
            advancements: match kept.filter(|_| mode == SyncLoad::SameFiles) {
                Some(kept) => kept.advancements.clone(),
                None => {
                    let projections = startup::time("sync.book_projections", || {
                        super::recipes::book_projections(sources)
                    })
                    .map_err(|error| error.to_string())?;
                    startup::time("sync.advancements", || {
                        let tree = advancements
                            .cloned()
                            .unwrap_or_else(|| Tree::shared(&resources.join("data"), overlay));
                        super::advancements::ServerAdvancements::from_tree(tree, projections)
                    })
                }
            },
            packed_registries: packed,
        };
        let (recipes, buttons) = startup::time("sync.update_recipes", || {
            super::recipes::update_recipes(&sync, sources)
        });
        sync.recipes = recipes;
        sync.stonecutter_buttons = Arc::new(buttons);
        overlay.conditions.check_errors()?;
        Ok(sync)
    }

    /// The creative builder's server view: the same ordered entries and current tags a
    /// joining client receives. The network's name-only index omits the decoded payloads.
    pub fn creative_holders(&self) -> sunrise_items::creative_catalog::Holders {
        let mut holders = Dynamic::default();
        for (key, entries) in &self.registries {
            if matches!(
                key.as_str(),
                "minecraft:enchantment"
                    | "minecraft:painting_variant"
                    | "minecraft:instrument"
                    | "minecraft:banner_pattern"
            ) {
                holders.registries.insert(
                    key.clone(),
                    entries
                        .names
                        .iter()
                        .cloned()
                        .zip(entries.data.iter().cloned().map(Some))
                        .collect(),
                );
            }
        }
        for tags in &self.tags.registries {
            if holders.registries.contains_key(&tags.registry) {
                holders.tags.insert(
                    tags.registry.clone(),
                    tags.tags
                        .iter()
                        .map(|tag| (tag.name.clone(), tag.ids.clone()))
                        .collect(),
                );
            }
        }
        crate::item_names::live_holders(&self.builtin, &holders)
    }

    /// Whether a synchronized registry holds an entry from a world's pack.
    pub fn has_pack_entries(&self) -> bool {
        self.registries
            .iter()
            .any(|(_, e)| e.from_pack.contains(&true))
    }

    /// `LivingEntity.updateSynchronizedMobEffectParticles`: DATA_EFFECT_PARTICLES
    /// (10) and DATA_EFFECT_AMBIENCE_ID (11) for the visible `effects`, as
    /// (effect, ambient). The entries are left out when there is nothing to
    /// show, which is their default.
    pub fn effect_meta<'a>(
        &self,
        effects: impl IntoIterator<Item = (&'a str, bool)>,
    ) -> Vec<(u8, i32, entity_data::Value)> {
        let mut ids = self
            .effect_particle_ids
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut particles = Vec::new();
        let mut all_ambient = true;
        for (effect, ambient) in effects {
            let effect = effect.trim_start_matches("minecraft:");
            let particle = super::effect_meta::particle_of(effect, ambient, |name| {
                ids.get_or_insert_with(name, || self.id("minecraft:particle_type", name))
            });
            let Some(particle) = particle else {
                continue;
            };
            particles.push(particle);
            all_ambient &= ambient;
        }
        if particles.is_empty() {
            return Vec::new();
        }
        let mut meta = vec![(
            10,
            entity_data::serializer::PARTICLES,
            entity_data::Value::Particles(particles.into()),
        )];
        if all_ambient {
            meta.push((
                11,
                entity_data::serializer::BOOLEAN,
                entity_data::Value::Bool(true),
            ));
        }
        meta
    }

    /// The id of a name in a built-in or synchronized registry.
    pub fn id(&self, registry: &str, name: &str) -> Option<i32> {
        if registry == consume_effects::REGISTRY {
            return self.dynamic.consume_effect_id(name);
        }
        if registry == "minecraft:data_component_type"
            && self.dynamic.custom_components
            && let Some(id) = sunrise_protocol::item::custom::id(name)
        {
            return Some(id);
        }
        if let Some(ids) = self.builtin.get(registry) {
            return ids.id(name);
        }
        self.dynamic
            .registries
            .get(registry)?
            .iter()
            .position(|(n, _)| n == name)
            .map(|i| i as i32)
    }

    /// ClientboundRegistryDataPacket per registry. With `known_pack` the
    /// client rebuilds the entries from its own data, so none is sent, the
    /// dialogs among them (`RegistrySynchronization.packRegistry`): only an
    /// entry a world's pack added or replaced (`from_pack`) carries its data.
    /// The registries for a client, with the data of a dimension type the
    /// world's data packs changed (`levels`) even when it knows the vanilla
    /// pack.
    pub fn registry_packets(&self, known_pack: bool, levels: &LevelShapes) -> Vec<RegistryData> {
        self.registries
            .iter()
            .map(|(key, e)| RegistryData {
                registry: key.clone(),
                entries: e
                    .names
                    .iter()
                    .zip(&e.data)
                    .zip(&e.from_pack)
                    .map(|((name, data), &from_pack)| {
                        let changed = (key == "minecraft:dimension_type")
                            .then(|| changed_dimension_type(name, data, levels))
                            .flatten();
                        RegistryEntry {
                            id: name.clone(),
                            data: match changed {
                                Some(changed) => Some(changed),
                                None if known_pack && !from_pack => None,
                                None => Some(data.clone()),
                            },
                        }
                    })
                    .collect(),
            })
            .collect()
    }
    fn registry_names<'a>(
        builtin: &'a Builtin,
        dynamic: &'a Dynamic,
        key: &str,
    ) -> Option<HashMap<&'a str, i32>> {
        if let Some(ids) = builtin.get(key) {
            return Some(
                ids.names
                    .iter()
                    .enumerate()
                    .map(|(i, n)| (n.as_str(), i as i32))
                    .collect(),
            );
        }
        dynamic.registries.get(key).map(|v| {
            v.iter()
                .enumerate()
                .map(|(i, (n, _))| (n.as_str(), i as i32))
                .collect()
        })
    }

    /// `TagNetworkSerialization.serializeTagsToNetwork`: encode bound tags
    /// by registry ID. Standalone sync also binds registries with no world owner.
    fn load_tags(
        resources: &Path,
        overlay: &sunrise_world::loot::PackOverlay,
        builtin: &Builtin,
        dynamic: &Dynamic,
        world_tags: Option<&WorldTags>,
        same_files: Option<(&UpdateTags, bool)>,
    ) -> Result<UpdateTags, String> {
        let mut keys: Vec<_> = builtin
            .registries
            .keys()
            .chain(dynamic.registries.keys())
            .collect();
        keys.sort_unstable();
        keys.dedup();
        let mut registries = Vec::new();
        for key in keys {
            if let Some((kept, rebuild_biomes)) = same_files
                && (!rebuild_biomes || key != BIOME_REGISTRY)
            {
                // Same-file loads only change the biome registry or feature flags.
                // Rebinding other registries here would discard retained tags.
                registries.extend(kept.registries.iter().find(|r| &r.registry == key).cloned());
                continue;
            }
            let Some(names) = Self::registry_names(builtin, dynamic, key) else {
                continue;
            };
            let tags = if let Some(bound) = world_tags.and_then(|tags| tags.for_registry(key)) {
                let bound: BTreeMap<_, _> = bound.iter().collect();
                bound
                    .into_iter()
                    // Class tags are generated on each side from the same block facts.
                    .filter(|(tag, _)| key != "minecraft:block" || !is_generated_tag(tag))
                    .map(|(tag, members)| {
                        let ids = members
                            .iter()
                            .map(|id| {
                                names
                                    .get(id.as_str())
                                    .copied()
                                    .ok_or_else(|| format!("tag {key} {tag}: no wire id for {id}"))
                            })
                            .collect::<Result<_, _>>()?;
                        Ok(TagEntry {
                            name: tag.clone(),
                            ids,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?
            } else if key == "minecraft:damage_type" {
                sunrise_world::damage_source::resource_tags(resources, overlay, |id, _| {
                    names.get(id).copied()
                })
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|(name, ids)| TagEntry { name, ids })
                .collect()
            } else {
                let kind = match key.split_once(':') {
                    Some(("minecraft", path)) => path.to_owned(),
                    Some((ns, path)) => format!("{ns}/{path}"),
                    None => key.clone(),
                };
                let contains = |id: &str| names.contains_key(id);
                let bound: BTreeMap<_, _> = if key == "minecraft:dialog" {
                    // Dialog registry admission also tracks missing writable holders.
                    bind_dialog_tags(&resources.join("data/minecraft"), overlay, contains).0
                } else {
                    sunrise_world::tag_layers::load(resources, &kind, overlay, contains)
                        .map_err(|e| e.to_string())?
                        .into_iter()
                        .collect()
                };
                bound
                    .into_iter()
                    .map(|(name, members)| TagEntry {
                        name,
                        ids: members.iter().map(|id| names[id.as_str()]).collect(),
                    })
                    .collect()
            };
            if !tags.is_empty() {
                registries.push(RegistryTags {
                    registry: key.clone(),
                    tags,
                });
            }
        }
        Ok(UpdateTags { registries })
    }
}

/// The vanilla dimension type `name` (its NBT `data`) with the shape the
/// world gives the level of that name, if its data packs changed it.
fn changed_dimension_type(name: &str, data: &Tag, levels: &LevelShapes) -> Option<Tag> {
    if !matches!(
        name,
        "minecraft:overworld" | "minecraft:the_nether" | "minecraft:the_end"
    ) {
        return None;
    }
    let shape = levels.get(name);
    if shape == LevelShapes::VANILLA.get(name) {
        return None;
    }
    let mut data = data.clone();
    let Tag::Compound(fields) = &mut data else {
        return None;
    };
    fields.insert("min_y".into(), Tag::Int(shape.min_y));
    fields.insert("height".into(), Tag::Int(shape.height));
    fields.insert("logical_height".into(), Tag::Int(shape.logical_height));
    fields.insert("has_skylight".into(), Tag::Byte(i8::from(shape.sky_light)));
    fields.insert("has_ceiling".into(), Tag::Byte(i8::from(shape.has_ceiling)));
    Some(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack_entries(
        key: &str,
        overlay: &sunrise_world::loot::PackOverlay,
        mut entries: Vec<((String, String), Value, bool)>,
    ) -> Vec<((String, String), Tag, bool)> {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("items.json"), b"{}").unwrap();
        for ((namespace, path), value, _) in &entries {
            let file = root
                .path()
                .join(format!("data/{namespace}/{key}/{path}.json"));
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, value.to_string()).unwrap();
        }
        let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
        let registries = PackRegistryDefinitions::load(
            root.path(),
            &defaults,
            overlay,
            &BTreeMap::new(),
            &Default::default(),
            |_| true,
        )
        .unwrap();
        let key = format!("minecraft:{key}");
        registries.discard_replaced(&key, &mut entries);
        let mut entries = entries
            .into_iter()
            .map(|(key, value, packed)| (key, json_to_nbt(&value), packed))
            .collect();
        registries.append_encoded(&key, &mut entries);
        entries
    }

    #[test]
    fn synchronized_registries_validate_only_the_winning_pack_files_before_join() {
        let root = tempfile::tempdir().unwrap();
        let bad = |id| {
            serde_json::json!({"neoforge:conditions":[{"type":id}]})
                .to_string()
                .into_bytes()
        };
        let chat = root.path().join("data/minecraft/chat_type");
        std::fs::create_dir_all(&chat).unwrap();
        std::fs::write(chat.join("rejected.json"), bad("absent:chat")).unwrap();
        let instrument = root.path().join("data/minecraft/instrument");
        std::fs::create_dir_all(&instrument).unwrap();
        std::fs::write(
            instrument.join("override.json"),
            bad("absent:root_instrument"),
        )
        .unwrap();
        let files = [
            ("instrument/minecraft:override".into(), vec![b"{".to_vec()]),
            (
                "dialog/test:valid".into(),
                vec![
                    bad("absent:loser"),
                    br#"{"type":"minecraft:notice","title":"Winner"}"#.to_vec(),
                ],
            ),
            ("world_clock/test:bad".into(), vec![b"[]".to_vec()]),
            (
                "dialog/test:bad".into(),
                vec![b"{}".to_vec(), bad("absent:dialog")],
            ),
            (
                "dimension_type/test:bad".into(),
                vec![bad("minecraft:unknown")],
            ),
        ];
        std::fs::write(root.path().join("items.json"), b"{}").unwrap();
        let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
        let conditions = sunrise_world::conditions::LoadConditions::new([]);
        let (packed, Err(typed)) = SyncData::stage_registry_definitions(
            root.path(),
            &defaults,
            &sunrise_world::loot::PackOverlay {
                files: &files,
                conditions: &conditions,
                ..sunrise_world::loot::PackOverlay::default()
            },
            &BTreeMap::new(),
            &Default::default(),
            |_| true,
        ) else {
            panic!("staged malformed typed registries")
        };
        for diagnostic in [
            "minecraft:dialog:\n  test:bad: data/test/dialog/bad.json: unknown load conditions [absent:dialog]",
            "minecraft:dimension_type:\n  test:bad: data/test/dimension_type/bad.json: unknown load conditions [minecraft:unknown]",
            "minecraft:world_clock:\n  test:bad: data/test/world_clock/bad.json: not a map",
        ] {
            assert!(typed.contains(diagnostic), "{typed}");
        }
        assert!(!typed.contains("absent:loser"), "{typed}");
        assert!(packed.entries().any(|(registry, id, admitted)| {
            registry == "minecraft:instrument" && id == "minecraft:override" && !admitted
        }));
        let untyped = conditions.check_errors().unwrap_err();
        assert!(
            untyped.contains(
                "data/minecraft/chat_type/rejected.json: unknown load conditions [absent:chat]"
            ),
            "{untyped}"
        );
        for losing_or_already_checked in [
            "absent:root_instrument",
            "absent:dialog",
            "minecraft:unknown",
        ] {
            assert!(!untyped.contains(losing_or_already_checked), "{untyped}");
        }
    }

    #[test]
    fn a_pack_clock_is_admitted_against_the_world_candidates_bound_tags() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("items.json"), b"{}").unwrap();
        let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
        let files = [(
            "world_clock/test:conditional".into(),
            vec![
                br#"{"neoforge:conditions":[{"type":"neoforge:tag_empty","tag":"test:marker"}]}"#
                    .to_vec(),
            ],
        )];
        for (tags, emitted) in [(["test:unrelated"], true), (["test:marker"], false)] {
            let conditions = sunrise_world::conditions::LoadConditions::new([])
                .with_tags([("minecraft:item", tags)]);
            let (packed, admitted) = SyncData::stage_registry_definitions(
                root.path(),
                &defaults,
                &sunrise_world::loot::PackOverlay {
                    files: &files,
                    conditions: &conditions,
                    ..sunrise_world::loot::PackOverlay::default()
                },
                &BTreeMap::new(),
                &Default::default(),
                |_| true,
            );
            admitted.unwrap();
            assert_eq!(conditions.check_errors(), Ok(()));
            assert_eq!(
                packed.entries().collect::<Vec<_>>(),
                [(
                    "minecraft:world_clock",
                    "test:conditional".to_owned(),
                    emitted
                ),]
            );
            let mut entries = Vec::new();
            packed.append_encoded("minecraft:world_clock", &mut entries);
            assert_eq!(
                entries.first().map(|entry| entry.0.1.as_str()),
                emitted.then_some("conditional")
            );
        }
    }

    #[test]
    fn false_conditions_do_not_suppress_lan_tag_operations() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("data/minecraft/tags/dialog")).unwrap();
        std::fs::write(
            root.path()
                .join("data/minecraft/tags/dialog/conditional.json"),
            r#"{"neoforge:conditions":[{"type":"neoforge:never"}],"values":["minecraft:a"]}"#,
        )
        .unwrap();
        let files = vec![("tags/minecraft:dialog/conditional".into(), vec![br#"{"neoforge:conditions":[{"type":"neoforge:never"}],"values":["minecraft:b"],"remove":["minecraft:a"]}"#.to_vec()])];
        let dynamic = Dynamic {
            registries: HashMap::from([(
                "minecraft:dialog".into(),
                vec![("minecraft:a".into(), None), ("minecraft:b".into(), None)],
            )]),
            ..Default::default()
        };
        let read_tags = |overlay: &sunrise_world::loot::PackOverlay| {
            SyncData::load_tags(
                root.path(),
                overlay,
                &Builtin::default(),
                &dynamic,
                None,
                None,
            )
            .unwrap()
        };
        assert_eq!(
            read_tags(&sunrise_world::loot::PackOverlay::NONE).registries[0].tags[0].ids,
            [0]
        );
        assert_eq!(
            read_tags(&sunrise_world::loot::PackOverlay::new(&files)).registries[0].tags[0].ids,
            [1]
        );
    }

    #[test]
    fn pack_tags_reject_unresolved_required_entries() {
        let root = tempfile::tempdir().unwrap();
        let registry = "minecraft:mob_effect";
        let builtin = Builtin {
            registries: HashMap::from([(
                registry.into(),
                sunrise_protocol::registry::Ids::from_names(vec!["minecraft:speed".into()]),
            )]),
        };
        let files = [
            ("rejected", r#"{"values":["minecraft:speed","test:unknown"]}"#),
            ("optional_id", r#"{"values":["minecraft:speed",{"id":"test:unknown","required":false}]}"#),
            ("dependent", r##"{"values":["#test:rejected"]}"##),
            ("missing_tag", r##"{"values":["minecraft:speed","#test:unknown"]}"##),
            ("optional_tag", r##"{"values":["minecraft:speed",{"id":"#test:unknown","required":false}]}"##),
            ("remove_child", r##"{"values":["#test:optional_id"],"remove":["#test:optional_id","#test:unknown"]}"##),
        ]
        .map(|(name, json)| (format!("tags/test:mob_effect/{name}"), vec![json.as_bytes().to_vec()]));
        for count in [files.len(), 1] {
            let tags = SyncData::load_tags(
                root.path(),
                &sunrise_world::loot::PackOverlay::new(&files[..count]),
                &builtin,
                &Dynamic::default(),
                None,
                None,
            )
            .unwrap();
            let mut received = Dynamic::default();
            received.add_tags(&tags);
            for missing in ["test:rejected", "test:dependent", "test:missing_tag"] {
                assert_eq!(received.tag(registry, missing), None, "{missing}");
            }
            let loaded = count == files.len();
            assert_eq!(tags.registries.len(), usize::from(loaded));
            assert_eq!(
                received.tag(registry, "test:optional_id"),
                loaded.then_some(&[0][..])
            );
            assert_eq!(
                received.tag(registry, "test:optional_tag"),
                loaded.then_some(&[0][..])
            );
            assert_eq!(
                received.tag(registry, "test:remove_child"),
                loaded.then_some(&[][..])
            );
        }
    }

    #[test]
    fn synchronized_optional_cycle_uses_vanilla_dependency_order() {
        let root = tempfile::tempdir().unwrap();
        let registry = "minecraft:mob_effect";
        let builtin = Builtin {
            registries: HashMap::from([(
                registry.into(),
                sunrise_protocol::registry::Ids::from_names(vec![
                    "minecraft:speed".into(),
                    "minecraft:slowness".into(),
                ]),
            )]),
        };
        let files = [
            (
                "tags/minecraft:mob_effect/a".into(),
                vec![br##"{"values":[{"id":"#b","required":false},"speed"]}"##.to_vec()],
            ),
            (
                "tags/minecraft:mob_effect/b".into(),
                vec![br##"{"values":[{"id":"#a","required":false},"slowness"]}"##.to_vec()],
            ),
        ];
        let tags = SyncData::load_tags(
            root.path(),
            &sunrise_world::loot::PackOverlay::new(&files),
            &builtin,
            &Dynamic::default(),
            None,
            None,
        )
        .unwrap();
        let mut client = Dynamic::default();
        client.add_tags(&tags);
        assert_eq!(client.tag(registry, "minecraft:a"), Some(&[0][..]));
        assert_eq!(client.tag(registry, "minecraft:b"), Some(&[0, 1][..]));
    }

    #[test]
    fn all_rejected_reload_keeps_previous_tags_for_server_and_clients() {
        use sunrise_world::loot::PackOverlay;
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        std::fs::write(root.join("items.json"), "{}").unwrap();
        std::fs::write(
            root.join("registries.json"),
            r#"{"minecraft:mob_effect":{"entries":{"minecraft:speed":{"protocol_id":0}}}}"#,
        )
        .unwrap();
        let files = |json: &[u8]| [("tags/test:mob_effect/boosts".into(), vec![json.to_vec()])];
        let valid = files(br#"{"values":["speed"]}"#);
        let rejected = files(br#"{"values":["speed","unknown"]}"#);
        let defaults = sunrise_world::item_defaults::read(root).unwrap();
        let sources = crate::crafting::ResolvedSources::default();
        let opened = SyncData::load(root, &PackOverlay::new(&valid), &defaults, &sources).unwrap();
        let mut connected = Dynamic::default();
        connected.add_tags(&opened.tags);
        let overlay = PackOverlay::new(&rejected);
        let reloaded = SyncData::reload(
            root,
            &overlay,
            &opened,
            &defaults,
            &sources,
            &Arc::new(Tree::load_with(&root.join("data"), &overlay)),
            None,
        )
        .unwrap();
        let registry = "minecraft:mob_effect";
        let tag = "test:boosts";
        let packet = &reloaded.tags;
        let retained = packet
            .registries
            .iter()
            .find(|r| r.registry == registry)
            .unwrap();
        assert_eq!(retained.tags.len(), 1);
        assert_eq!(retained.tags[0].name, tag);
        assert_eq!(retained.tags[0].ids, [0]);
        connected.add_tags(packet);
        assert_eq!(connected.tag(registry, tag), Some(&[0][..]));
        let mut fresh_join = Dynamic::default();
        fresh_join.add_tags(&reloaded.tags);
        assert_eq!(fresh_join.tag(registry, tag), connected.tag(registry, tag));
        let fresh_world = SyncData::load(root, &overlay, &defaults, &sources).unwrap();
        assert!(
            !fresh_world
                .tags
                .registries
                .iter()
                .any(|r| r.registry == registry)
        );
    }

    #[test]
    fn wire_reload_uses_the_staged_advancement_generation() {
        use sunrise_world::loot::PackOverlay;
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        let data = root.join("data");
        std::fs::create_dir(&data).unwrap();
        for name in ["items.json", "registries.json"] {
            std::fs::write(root.join(name), "{}").unwrap();
        }
        let files = |trigger: &str| {
            vec![(
                "advancement/test:staged".into(),
                vec![
                    serde_json::json!({"criteria":{"fixture":{"trigger":trigger}}})
                        .to_string()
                        .into_bytes(),
                ],
            )]
        };
        let tree = Arc::new(Tree::load_with(
            &data,
            &PackOverlay::new(&files("minecraft:tick")),
        ));
        let reloaded = SyncData::reload(
            root,
            &PackOverlay::new(&files("minecraft:impossible")),
            &SyncData::empty(),
            &sunrise_world::item_defaults::read(root).unwrap(),
            &crate::crafting::ResolvedSources::default(),
            &tree,
            None,
        )
        .unwrap();
        let advancement = reloaded.advancements.definition("test:staged").unwrap();
        assert_eq!(advancement.criteria[0].trigger, "minecraft:tick");
    }

    #[test]
    #[ignore = "daily"]
    fn datapack_damage_registry_reload_keeps_wire_ids_and_reloads_tags_before_future_join() {
        use sunrise_world::loot::PackOverlay;
        let root = std::env::var_os("SUNRISE_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/sunrise-client")
                    .join(sunrise_core::VANILLA_VERSION)
            });
        let files = vec![
            ("damage_type/demo:heat".into(), vec![br#"{"message_id":"heat","scaling":"never","exhaustion":0.1,"effects":"burning"}"#.to_vec()]),
            ("damage_type/demo:infinite".into(), vec![br#"{"message_id":"infinite","scaling":"never","exhaustion":1e400}"#.to_vec()]),
            ("tags/minecraft:damage_type/is_fire".into(), vec![br#"{"values":["demo:heat"]}"#.to_vec()]),
        ];
        let open = SyncData::load(
            &root,
            &PackOverlay::new(&files),
            &sunrise_world::item_defaults::read(&root).unwrap(),
            &crate::crafting::ResolvedSources::default(),
        )
        .unwrap();
        let heat = open.id("minecraft:damage_type", "demo:heat").unwrap();
        let packet = open.registry_packets(true, &LevelShapes::VANILLA);
        let packet = packet
            .iter()
            .find(|p| p.registry == "minecraft:damage_type")
            .unwrap();
        assert!(
            packet.entries[heat as usize].data.is_some(),
            "custom definitions are sent even with known vanilla packs"
        );
        let infinite = open.id("minecraft:damage_type", "demo:infinite").unwrap();
        let data = packet.entries[infinite as usize].data.as_ref().unwrap();
        assert_eq!(
            data.get("exhaustion"),
            Some(&Tag::Float(f32::INFINITY.to_bits()))
        );
        assert!(data.get("effects").is_none() && data.get("death_message_type").is_none());
        let changed = vec![
            (
                "damage_type/demo:heat".into(),
                vec![br#"{"invalid":true}"#.to_vec()],
            ),
            (
                "damage_type/demo:unregistered".into(),
                vec![br#"{"invalid":true}"#.to_vec()],
            ),
            (
                "tags/demo:damage_type/current".into(),
                vec![
                    br#"{"values":["demo:heat",{"id":"demo:unregistered","required":false}]}"#
                        .to_vec(),
                ],
            ),
        ];
        let reloaded = SyncData::reload(
            &root,
            &PackOverlay::new(&changed),
            &open,
            &sunrise_world::item_defaults::read(&root).unwrap(),
            &crate::crafting::ResolvedSources::default(),
            &Arc::new(Tree::load_with(
                &root.join("data"),
                &PackOverlay::new(&changed),
            )),
            None,
        )
        .unwrap();
        assert_eq!(
            reloaded.id("minecraft:damage_type", "demo:heat"),
            Some(heat)
        );
        assert!(
            reloaded
                .id("minecraft:damage_type", "demo:unregistered")
                .is_none()
        );
        let tags = reloaded
            .tags
            .registries
            .iter()
            .find(|r| r.registry == "minecraft:damage_type")
            .unwrap();
        assert_eq!(
            tags.tags
                .iter()
                .find(|t| t.name == "demo:current")
                .unwrap()
                .ids,
            [heat]
        );
        assert!(
            !tags
                .tags
                .iter()
                .find(|t| t.name == "minecraft:is_fire")
                .unwrap()
                .ids
                .contains(&heat)
        );
        // Another world performs registry loading again and refuses malformed definitions.
        assert!(
            SyncData::load(
                &root,
                &PackOverlay::new(&changed),
                &sunrise_world::item_defaults::read(&root).unwrap(),
                &crate::crafting::ResolvedSources::default()
            )
            .is_err()
        );
        let fresh = SyncData::load(
            &root,
            &PackOverlay::NONE,
            &sunrise_world::item_defaults::read(&root).unwrap(),
            &crate::crafting::ResolvedSources::default(),
        )
        .unwrap();
        assert!(fresh.id("minecraft:damage_type", "demo:heat").is_none());

        let mut explicit_null = files;
        explicit_null.push(("damage_type/demo:null_conditions".into(), vec![
            br#"{"message_id":"test","scaling":"never","exhaustion":0,"neoforge:conditions":null}"#.to_vec(),
        ]));
        let conditions = sunrise_world::conditions::LoadConditions::new([]);
        let error = SyncData::load(
            &root,
            &PackOverlay {
                conditions: &conditions,
                ..PackOverlay::new(&explicit_null)
            },
            &sunrise_world::item_defaults::read(&root).unwrap(),
            &crate::crafting::ResolvedSources::default(),
        )
        .err()
        .unwrap();
        assert!(
            error.contains(
                "data/demo/damage_type/null_conditions.json: neoforge:conditions: expected a list"
            ),
            "{error}"
        );
    }

    /// A content mod's enchantments (`data/<ns>/enchantment/`, in the resource
    /// root beside vanilla's) join the synchronized registry with their data,
    /// which no client knows; a world's pack still replaces one.
    #[test]
    fn mod_and_pack_enchantments_join_the_synchronized_registry() {
        use sunrise_world::loot::PackOverlay;
        let root = std::env::temp_dir().join(format!(
            "sunrise-lan-mod-enchantments-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for (path, body) in [
            (
                "data/harvestmod/enchantment/scythe.json",
                r#"{"max_level":3}"#,
            ),
            (
                "data/harvestmod/enchantment/tools/sweep.json",
                r#"{"max_level":1}"#,
            ),
            ("data/harvestmod/recipe/x.json", r#"{}"#),
            ("data/harvestmod/enchantment/tools/broken.json", "{"),
        ] {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        let mut found = vec![(
            ("minecraft".to_owned(), "mending".to_owned()),
            serde_json::json!({ "max_level": 1 }),
            false,
        )];
        mod_entries(&root, "enchantment", &mut found);
        let files = vec![(
            "enchantment/harvestmod:scythe".to_owned(),
            vec![br#"{"description":"Scythe","max_level":5,"weight":1,"anvil_cost":1,"supported_items":"minecraft:iron_hoe","slots":["mainhand"],"min_cost":{"base":1,"per_level_above_first":1},"max_cost":{"base":5,"per_level_above_first":1}}"#.to_vec()],
        )];
        let mut found = pack_entries("enchantment", &PackOverlay::new(&files), found);
        file_order(&mut found);
        let names: Vec<String> = found
            .iter()
            .map(|((ns, p), _, _)| format!("{ns}:{p}"))
            .collect();
        assert_eq!(
            names,
            [
                "minecraft:mending",
                "harvestmod:scythe",
                "harvestmod:tools/sweep"
            ]
        );
        assert!(!found[0].2, "vanilla's entry stays the known pack's");
        assert!(found[1].2 && found[2].2, "a mod's entries carry their data");
        assert_eq!(
            found[1].1.get("max_level"),
            Some(&Tag::Int(5)),
            "a pack overrides a mod's entry"
        );
        let mut sync = SyncData::empty();
        sync.registries.push((
            "minecraft:enchantment".into(),
            Entries {
                names: names.clone(),
                data: found.iter().map(|(_, value, _)| value.clone()).collect(),
                from_pack: found.iter().map(|(_, _, packed)| *packed).collect(),
            },
        ));
        let packets = sync.registry_packets(true, &LevelShapes::VANILLA);
        assert_eq!(
            packets[0]
                .entries
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            [
                "minecraft:mending",
                "harvestmod:scythe",
                "harvestmod:tools/sweep"
            ]
        );
        assert!(packets[0].entries[0].data.is_none());
        assert_eq!(
            packets[0].entries[1]
                .data
                .as_ref()
                .and_then(|data| data.get("max_level")),
            Some(&Tag::Int(5)),
        );
        assert!(packets[0].entries[2].data.is_some());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn world_packs_supply_dialog_entries_and_tags() {
        use sunrise_world::loot::PackOverlay;
        let vanilla = |p: &str| {
            (
                ("minecraft".to_owned(), p.to_owned()),
                serde_json::json!({ "type": "minecraft:dialog_list" }),
                false,
            )
        };
        let found = vec![vanilla("quick_actions"), vanilla("server_links")];
        let files = vec![
            (
                "dialog/test:menu".to_owned(),
                vec![br#"{"type":"minecraft:multi_action","title":"Menu","actions":[{"label":"OK"}]}"#.to_vec()],
            ),
            (
                "dialog/minecraft:quick_actions".to_owned(),
                vec![br#"{"type":"minecraft:notice","title":"Quick"}"#.to_vec()],
            ),
            (
                "tags/minecraft:dialog/quick_actions".to_owned(),
                vec![br##"{"values":["test:menu","#minecraft:more"]}"##.to_vec()],
            ),
            (
                "tags/minecraft:dialog/more".to_owned(),
                vec![br#"{"values":["minecraft:server_links"]}"#.to_vec()],
            ),
        ];
        let overlay = PackOverlay::new(&files);
        let mut found = pack_entries("dialog", &overlay, found);
        file_order(&mut found);
        let names: Vec<String> = found
            .iter()
            .map(|((ns, p), _, _)| format!("{ns}:{p}"))
            .collect();
        // Identifier order: by path, then namespace.
        assert_eq!(
            names,
            [
                "test:menu",
                "minecraft:quick_actions",
                "minecraft:server_links"
            ]
        );
        assert_eq!(
            found[1].1.get("title"),
            Some(&Tag::String("Quick".into())),
            "a pack overrides an entry"
        );
        let dynamic = Dynamic {
            registries: HashMap::from([(
                "minecraft:dialog".to_owned(),
                names.iter().map(|n| (n.clone(), None)).collect(),
            )]),
            ..Default::default()
        };
        let tags = SyncData::load_tags(
            Path::new("a-resource-root-without-tags"),
            &overlay,
            &Builtin::default(),
            &dynamic,
            None,
            None,
        )
        .unwrap();
        let dialog = tags
            .registries
            .iter()
            .find(|r| r.registry == "minecraft:dialog")
            .expect("the dialog tags are sent");
        let quick = dialog
            .tags
            .iter()
            .find(|t| t.name == "minecraft:quick_actions")
            .unwrap();
        assert_eq!(
            quick.ids,
            [0, 2],
            "the pack's dialog, then the nested tag's"
        );
    }

    fn assert_pack_holder_sync(
        kind: &str,
        tag: &str,
        base: Value,
        field: &str,
        lower: Value,
        higher: Value,
        expected: Tag,
    ) {
        use crate::datapack::{DataPack, DataPacks, DataStore};
        use sunrise_assets::resource_source::{Resource, ResourceSource};
        use sunrise_world::{item_defaults, loot::PackOverlay};

        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("items.json"), "{}").unwrap();
        std::fs::write(root.path().join("registries.json"), r#"{"minecraft:sound_event":{"entries":{"minecraft:item.goat_horn.sound.0":{"protocol_id":0}}}}"#).unwrap();
        let definitions = root.path().join("data/minecraft").join(kind);
        std::fs::create_dir_all(&definitions).unwrap();
        for file in ["x.json", "z.json"] {
            std::fs::write(definitions.join(file), base.to_string()).unwrap();
        }
        let mut low = base.clone();
        low[field] = lower;
        let mut high = base.clone();
        high[field] = higher;
        let bytes = |value: &Value| Resource::bytes(value.to_string().into_bytes());
        let packs = [
            DataPack::from_source(
                "file/low".into(),
                ResourceSource::indexed(BTreeMap::from([(
                    format!("data/minecraft/{kind}/x.json"),
                    bytes(&low),
                )])),
            ),
            DataPack::from_source(
                "file/high".into(),
                ResourceSource::indexed(BTreeMap::from([
                    (format!("data/minecraft/{kind}/x.json"), bytes(&high)),
                    (format!("data/demo/{kind}/x-y.json"), bytes(&base)),
                    (format!("data/demo/{kind}/x.json"), bytes(&base)),
                    (
                        format!("data/minecraft/tags/{kind}/{tag}.json"),
                        Resource::bytes(
                            br#"{"values":["minecraft:x","demo:x-y","demo:x"]}"#.to_vec(),
                        ),
                    ),
                ])),
            ),
        ];
        let files: Vec<_> = DataStore::load(&DataPacks::new(&packs.iter().collect::<Vec<_>>()))
            .files
            .into_iter()
            .collect();
        let sync = SyncData::load(
            root.path(),
            &PackOverlay::new(&files),
            &item_defaults::read(root.path()).unwrap(),
            &crate::crafting::ResolvedSources::default(),
        )
        .unwrap();
        let registry = format!("minecraft:{kind}");
        let packet = sync
            .registry_packets(true, &LevelShapes::VANILLA)
            .into_iter()
            .find(|packet| packet.registry == registry)
            .unwrap();
        let ids: Vec<_> = packet
            .entries
            .iter()
            .map(|entry| entry.id.as_str())
            .collect();
        assert_eq!(ids, ["demo:x-y", "demo:x", "minecraft:x", "minecraft:z"]);
        assert!(packet.entries[0].data.is_some());
        assert_eq!(
            packet.entries[2].data.as_ref().unwrap().get(field),
            Some(&expected)
        );
        assert!(packet.entries[3].data.is_none());
        let tags = sync
            .tags
            .registries
            .iter()
            .find(|row| row.registry == registry)
            .unwrap();
        let tag = tags
            .tags
            .iter()
            .find(|row| row.name == format!("minecraft:{tag}"))
            .unwrap();
        assert_eq!(tag.ids, [2, 0, 1]);
    }

    #[test]
    fn world_pack_paintings_reach_clients() {
        use serde_json::json;
        assert_pack_holder_sync(
            "painting_variant",
            "placeable",
            json!({"asset_id":"demo:mural","width":1,"height":2}),
            "width",
            json!(2),
            json!(4),
            Tag::Int(4),
        );
    }

    #[test]
    fn world_pack_instruments_reach_clients() {
        use serde_json::json;
        assert_pack_holder_sync(
            "instrument",
            "goat_horns",
            json!({"sound_event":"minecraft:item.goat_horn.sound.0","use_duration":4.0,"range":128.0,"description":"Base"}),
            "description",
            json!("Lower"),
            json!({"text":"Winner","bold":null,"italic":true,
                   "click_event":{"action":"run_command","command":"/help","value":"/ignored"}}),
            Tag::compound([
                ("text", Tag::String("Winner".into())),
                ("italic", Tag::Byte(1)),
                (
                    "click_event",
                    Tag::compound([
                        ("action", Tag::String("run_command".into())),
                        ("command", Tag::String("/help".into())),
                    ]),
                ),
            ]),
        );
    }

    #[test]
    fn pack_instrument_inline_sound_range_is_a_float_tag() {
        let files = [(
            "instrument/demo:horn".into(),
            vec![br#"{"sound_event":{"sound_id":"test:sound","range":9.5},"use_duration":4,"range":128,"description":"Horn"}"#.to_vec()],
        )];
        let overlay = sunrise_world::loot::PackOverlay::new(&files);
        let entries = pack_entries("instrument", &overlay, Vec::new());
        assert_eq!(
            entries[0]
                .1
                .get("sound_event")
                .and_then(|sound| sound.get("range")),
            Some(&Tag::Float(9.5_f32.to_bits())),
        );
    }

    #[test]
    fn world_pack_banner_patterns_reach_clients() {
        use serde_json::json;
        assert_pack_holder_sync(
            "banner_pattern",
            "no_item_required",
            json!({"asset_id":"demo:mural","translation_key":"test.banner.base"}),
            "translation_key",
            json!("test.banner.lower"),
            json!("test.banner.winner"),
            Tag::String("test.banner.winner".into()),
        );
    }

    #[test]
    fn world_pack_trims_reach_clients() {
        use serde_json::json;
        assert_pack_holder_sync(
            "trim_pattern",
            "test_patterns",
            json!({"asset_id":"demo:zigzag","description":{"translate":"trim_pattern.demo.zigzag"}}),
            "asset_id",
            json!("demo:lower"),
            json!("demo:winner"),
            Tag::String("demo:winner".into()),
        );
        assert_pack_holder_sync(
            "trim_material",
            "test_materials",
            json!({"palette_id":"demo:trim/jade","description":{"translate":"trim_material.demo.jade"}}),
            "palette_id",
            json!("demo:trim/lower"),
            json!("demo:trim/winner"),
            Tag::String("demo:trim/winner".into()),
        );
    }

    /// #2506: a world pack's clocks reach the clients from the pack's own files, so a dimension
    /// type that names one as `default_clock` resolves on their side. Under known-pack
    /// negotiation a clock the pack replaces goes with its data, one it leaves does not.
    #[test]
    #[ignore = "daily"]
    fn world_packs_supply_world_clocks() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        let world =
            std::env::temp_dir().join(format!("sunrise-pack-clocks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&world);
        let pack = world.join("datapacks/moon");
        let write = |p: &str, t: &str| {
            let path = pack.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, t).unwrap();
        };
        write(
            "pack.mcmeta",
            r#"{"pack":{"description":"Moon","pack_format":1}}"#,
        );
        write("data/demo/world_clock/moon.json", "{}");
        write(
            "data/minecraft/world_clock/the_end.json",
            r#"{"from":"pack"}"#,
        );
        write(
            "data/demo/dimension_type/moon.json",
            r##"{
                "min_y":0,"height":64,"logical_height":64,"has_skylight":true,
                "has_ceiling":false,"has_ender_dragon_fight":false,"coordinate_scale":1.0,
                "ambient_light":0.0,"infiniburn":"#minecraft:infiniburn_overworld",
                "monster_spawn_light_level":0,"monster_spawn_block_light_limit":0,
                "default_clock":"demo:moon"
            }"##,
        );
        let packs = crate::datapack::discover(&world);
        let mut files = crate::datapack::DataStore::load(&crate::datapack::DataPacks::new(
            &packs.iter().collect::<Vec<_>>(),
        ))
        .files;
        // Native contributions sit below the world's enabled packs.
        files
            .get_mut("world_clock/minecraft:the_end")
            .unwrap()
            .insert(
                0,
                br#"{"neoforge:conditions":[{"type":"neoforge:never"}]}"#.to_vec(),
            );
        files.insert("world_clock/test:frozen".into(), vec![b"{}".to_vec()]);
        let files: Vec<_> = files.into_iter().collect();
        let _ = std::fs::remove_dir_all(&world);
        let sync = SyncData::load(
            &root,
            &sunrise_world::loot::PackOverlay::new(&files),
            &sunrise_world::item_defaults::read(&root).unwrap(),
            &crate::crafting::ResolvedSources::default(),
        )
        .unwrap();
        let packets = sync.registry_packets(true, &LevelShapes::VANILLA);
        let registry = |key: &str| packets.iter().find(|p| p.registry == key).unwrap();
        let moon = registry("minecraft:dimension_type")
            .entries
            .iter()
            .find(|e| e.id == "demo:moon")
            .and_then(|e| e.data.as_ref())
            .expect("the pack's dimension type goes with its data");
        let Some(Tag::String(clock)) = moon.get("default_clock") else {
            panic!("the dimension type names no clock: {moon:?}");
        };
        let clock = clock.to_string_lossy();
        let sent = |name: &str| {
            let clocks = &registry("minecraft:world_clock").entries;
            clocks
                .iter()
                .find(|e| e.id == name)
                .map(|e| e.data.as_ref())
        };
        assert!(
            sent(&clock).is_some_and(|data| data.is_some()),
            "no client knows the clock {clock} the dimension names"
        );
        assert_eq!(
            sent("minecraft:the_end"),
            Some(Some(&json_to_nbt(&serde_json::json!({"from":"pack"})))),
            "the enabled pack's clock wins over the false-conditioned native clock"
        );
        assert_eq!(
            sent("test:frozen"),
            Some(Some(&json_to_nbt(&serde_json::json!({})))),
            "a clock only the native mod gives"
        );
        assert_eq!(
            sent("minecraft:overworld"),
            Some(None),
            "the untouched clock"
        );
    }

    /// A pack's clock that is not a map refuses the registry load with its id
    /// (`WorldClock.DIRECT_CODEC`), where it used to be skipped; any map is a clock.
    #[test]
    fn a_malformed_world_clock_refuses_the_load() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("items.json"), b"{}").unwrap();
        let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
        let check = |json: &[u8]| {
            let files = vec![
                (
                    "world_clock/demo:fine".to_owned(),
                    vec![br#"{"any":1}"#.to_vec()],
                ),
                ("world_clock/demo:moon".to_owned(), vec![json.to_vec()]),
            ];
            PackRegistryDefinitions::load(
                root.path(),
                &defaults,
                &sunrise_world::loot::PackOverlay::new(&files),
                &BTreeMap::new(),
                &Default::default(),
                |_| true,
            )
            .map(|_| ())
        };
        assert_eq!(check(b"{}"), Ok(()));
        for bad in [&b"[]"[..], b"{", b"5"] {
            let refused = check(bad).unwrap_err();
            assert!(refused.contains("demo:moon"), "{refused}");
        }
    }

    /// The tags the world open binds for the registries it reads through `registry_tag_bindings`
    /// (biome, structure, timeline, dialog): two pack layers of one tag, neither `replace`, the
    /// second with an optional entry the registry lacks, give the held members in layer order.
    #[test]
    fn pack_tag_layers_bind_for_biome_structure_timeline_and_dialog() {
        let root = tempfile::tempdir().unwrap();
        let files: Vec<_> = ["worldgen/biome", "worldgen/structure", "timeline", "dialog"]
            .into_iter()
            .map(|kind| {
                (
                    format!("tags/test:{kind}/t"),
                    vec![
                        br#"{"values":["test:a"]}"#.to_vec(),
                        br#"{"values":["test:b",{"id":"test:ghost","required":false}]}"#.to_vec(),
                    ],
                )
            })
            .collect();
        let overlay = sunrise_world::loot::PackOverlay::new(&files);
        for registry in [
            "minecraft:worldgen/biome",
            "minecraft:worldgen/structure",
            "minecraft:timeline",
            "minecraft:dialog",
        ] {
            let tags = registry_tag_bindings(root.path(), &overlay, registry, |id| {
                id == "test:a" || id == "test:b"
            })
            .unwrap();
            assert_eq!(tags["test:t"], ["test:a", "test:b"], "{registry}");
        }
    }

    /// #7843: the server sends its tags as `TagLoader` merged them from the
    /// whole pack stack. A world pack that adds dirt to `climbable` and
    /// replaces `mineable/pickaxe` reaches the block tags sent, over the
    /// resource root's; a nested tag expands; a registry no pack has tag files
    /// for keeps what `load_tags` read.
    #[test]
    fn world_packs_tags_reach_every_synchronized_registry() {
        let root = std::env::temp_dir().join(format!("sunrise-pack-tags-{}", std::process::id()));
        let block_tags = root.join("data/minecraft/tags/block");
        std::fs::create_dir_all(block_tags.join("mineable")).unwrap();
        std::fs::write(
            block_tags.join("climbable.json"),
            r#"{"values":["minecraft:ladder"]}"#,
        )
        .unwrap();
        std::fs::write(
            block_tags.join("mineable/pickaxe.json"),
            r#"{"values":["minecraft:stone"]}"#,
        )
        .unwrap();
        let names = |list: &[&str]| {
            sunrise_protocol::registry::Ids::from_names(
                list.iter().map(|s| (*s).to_owned()).collect(),
            )
        };
        let builtin = Builtin {
            registries: HashMap::from([
                (
                    "minecraft:block".to_owned(),
                    names(&["minecraft:stone", "minecraft:dirt", "minecraft:ladder"]),
                ),
                ("minecraft:item".to_owned(), names(&["minecraft:stick"])),
            ]),
        };
        let dynamic = Dynamic::default();
        let item_dir = root.join("data/minecraft/tags/item");
        std::fs::create_dir_all(&item_dir).unwrap();
        std::fs::write(
            item_dir.join("sticks.json"),
            r#"{"values":["minecraft:stick"]}"#,
        )
        .unwrap();
        let files = vec![
            (
                "tags/minecraft:block/climbable".to_owned(),
                vec![br##"{"values":["minecraft:dirt","#test:more"]}"##.to_vec()],
            ),
            (
                "tags/test:block/more".to_owned(),
                vec![br#"{"values":["minecraft:stone"]}"#.to_vec()],
            ),
            (
                "tags/minecraft:block/mineable/pickaxe".to_owned(),
                vec![
                    br#"{"replace":true,"values":["minecraft:dirt"]}"#.to_vec(),
                    br#"{"neoforge:conditions":[{"type":"unsupported"}],"replace":true,"values":["minecraft:stone"]}"#.to_vec(),
                ],
            ),
        ];
        let overlay = sunrise_world::loot::PackOverlay::new(&files);
        let tags = SyncData::load_tags(&root, &overlay, &builtin, &dynamic, None, None).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        let sent = |registry: &str, tag: &str| -> Vec<i32> {
            tags.registries
                .iter()
                .find(|r| r.registry == registry)
                .and_then(|r| r.tags.iter().find(|t| t.name == tag))
                .map(|t| t.ids.clone())
                .unwrap_or_else(|| panic!("{registry} {tag} is sent"))
        };
        // The root's ladder, then the pack's dirt and its nested tag's stone.
        assert_eq!(sent("minecraft:block", "minecraft:climbable"), [2, 1, 0]);
        // Tag conditions are ignored: the final replace restores stone after dirt.
        assert_eq!(sent("minecraft:block", "minecraft:mineable/pickaxe"), [0]);
        assert_eq!(sent("minecraft:block", "test:more"), [0]);
        // No pack tags for items: the root's tag stays as it was read alongside built-in c: tags.
        let items = tags
            .registries
            .iter()
            .find(|r| r.registry == "minecraft:item")
            .unwrap();
        let non_convention: Vec<_> = items
            .tags
            .iter()
            .filter(|t| !t.name.starts_with("c:") && !t.name.starts_with("neoforge:"))
            .collect();
        assert_eq!(non_convention.len(), 1);
        assert_eq!(non_convention[0].name, "minecraft:sticks");
        assert_eq!(non_convention[0].ids, [0]);
        assert_eq!(
            tags.registries
                .iter()
                .filter(|r| r.registry == "minecraft:block")
                .count(),
            1,
            "one entry per registry"
        );
    }

    /// RegistryDataLoader.SYNCHRONIZED_REGISTRIES: a world pack that adds one entry to each of
    /// the registries reaches every client in that registry's `RegistryData`, with its data.
    /// Entries a registry's own codec is complex for are the prepared vanilla file of that
    /// registry under a new id, except where the registry validates across entries (the timeline
    /// is written out); the biome table is the world's admitted one.
    #[test]
    #[ignore = "daily"]
    fn world_packs_add_an_entry_to_every_synchronized_registry() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        let written = |registry: &str, value: &str| {
            (
                format!("{registry}/demo:added"),
                vec![value.as_bytes().to_vec()],
            )
        };
        let mut files = Vec::new();
        for (registry, value) in [
            ("world_clock", r#"{}"#),
            // Timeline.validateRegistry refuses a (clock, marker) pair defined twice, so the
            // entry is not a copy of a vanilla timeline with its markers.
            (
                "timeline",
                r#"{"clock":"minecraft:overworld","period_ticks":24000}"#,
            ),
            ("decorated_pot_pattern", r#"{"asset_id":"demo:p"}"#),
            (
                "block_transformer",
                r#"[{"block_state_provider":{"id":"minecraft:dirt"},"sound":"minecraft:item.hoe.till"}]"#,
            ),
            (
                "worldgen/block_state_provider",
                r#"{"type":"minecraft:simple_state_provider","state":{"Name":"minecraft:stone"}}"#,
            ),
            (
                "test_environment",
                r#"{"type":"minecraft:all_of","definitions":[]}"#,
            ),
            (
                "test_instance",
                r#"{"type":"minecraft:function","function":"demo:f","structure":"demo:s","max_ticks":1,
                    "environment":{"type":"minecraft:all_of","definitions":[]}}"#,
            ),
            (
                "trim_pattern",
                r#"{"asset_id":"demo:zigzag","description":{"translate":"trim_pattern.demo.zigzag"}}"#,
            ),
            (
                "trim_material",
                r#"{"palette_id":"demo:trim/jade","description":{"translate":"trim_material.demo.jade"}}"#,
            ),
            (
                "banner_pattern",
                r#"{"asset_id":"demo:mural","translation_key":"test.banner"}"#,
            ),
            (
                "painting_variant",
                r#"{"asset_id":"demo:mural","width":1,"height":2}"#,
            ),
            (
                "instrument",
                r#"{"sound_event":"minecraft:item.goat_horn.sound.0","use_duration":4.0,"range":128.0,"description":"Demo"}"#,
            ),
        ] {
            files.push(written(registry, value));
        }
        for &registry in SYNCHRONIZED {
            let path = registry.trim_start_matches("minecraft:");
            if registry == BIOME_REGISTRY || files.iter().any(|(key, _)| key.starts_with(path)) {
                continue;
            }
            let folder = sunrise_world::json_folder::read(&root.join("data/minecraft").join(path))
                .unwrap_or_else(|e| panic!("{registry}: no prepared folder: {e}"));
            let first = folder
                .files
                .iter()
                .find(|file| !file.name.contains('/'))
                .unwrap_or_else(|| panic!("{registry}: no prepared file"));
            files.push((
                format!("{path}/demo:added"),
                vec![first.bytes().unwrap().to_vec()],
            ));
        }
        let biomes = crate::biome::Biomes::load_vanilla_for_server(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        )
        .unwrap();
        let plains = biomes.builtin_file("minecraft:plains").unwrap().clone();
        let biomes = biomes.with_packs([("demo:added", &plains)]).unwrap();
        let sync = SyncData::load_with(
            &root,
            &sunrise_world::loot::PackOverlay::new(&files),
            Some(&biomes),
            &sunrise_world::item_defaults::read(&root).unwrap(),
            &crate::crafting::ResolvedSources::default(),
        )
        .unwrap();
        let packets = sync.registry_packets(true, &LevelShapes::VANILLA);
        assert_eq!(SYNCHRONIZED.len(), 32);
        for &registry in SYNCHRONIZED {
            let entry = packets
                .iter()
                .find(|packet| packet.registry == registry)
                .unwrap_or_else(|| panic!("{registry} is not sent"))
                .entries
                .iter()
                .find(|entry| entry.id == "demo:added")
                .unwrap_or_else(|| panic!("{registry}: the pack's entry is not sent"));
            assert!(
                entry.data.is_some(),
                "{registry}: the entry goes with its data"
            );
        }
    }

    /// RegistrySynchronization.packRegistry: a client that knows the core
    /// pack gets the vanilla dialogs without data; a pack's dialog has it.
    #[test]
    fn known_pack_dialogs_are_sent_without_data() {
        let mut sync = SyncData::empty();
        sync.registries.push((
            "minecraft:dialog".to_owned(),
            Entries {
                names: vec!["minecraft:quick_actions".into(), "test:menu".into()],
                data: vec![json_to_nbt(&serde_json::json!({ "type": "minecraft:dialog_list" })); 2],
                from_pack: vec![false, true],
            },
        ));
        let data = |known_pack| -> Vec<bool> {
            sync.registry_packets(known_pack, &LevelShapes::VANILLA)[0]
                .entries
                .iter()
                .map(|e| e.data.is_some())
                .collect()
        };
        assert_eq!(data(true), [false, true]);
        assert_eq!(data(false), [true, true]);
    }

    #[test]
    fn cached_dialog_nbt_keeps_the_existing_wire_payload() {
        let data = json_to_nbt(&serde_json::json!({"type":"minecraft:dialog_list"}));
        let mut sync = SyncData::empty();
        sync.registries.push((
            "minecraft:dialog".into(),
            Entries {
                names: vec!["minecraft:quick_actions".into()],
                data: vec![data],
                from_pack: vec![false],
            },
        ));
        let packets = sync.registry_packets(false, &LevelShapes::VANILLA);
        let data = packets[0].entries[0].data.as_ref().unwrap();
        let encoded =
            sunrise_core::nbt::write_network(data, sunrise_core::nbt::Limits::default()).unwrap();
        // Compound, string key/type, 21-byte value, compound end.
        let mut expected = vec![10, 8, 0, 4, b't', b'y', b'p', b'e', 0, 21];
        expected.extend_from_slice(b"minecraft:dialog_list");
        expected.push(0);
        assert_eq!(encoded, expected);
    }

    #[test]
    fn a_mods_dimension_type_is_the_overworlds_under_the_packs() {
        use sunrise_world::{loot::PackOverlay, worldgen::overrides::ModDimension};
        let overworld = serde_json::json!({
            "min_y":-64,"height":384,"ambient_light":0.0,
            "attributes":{"minecraft:visual/sky_color":"#78a7ff","minecraft:gameplay/fast_lava":false}
        });
        let mut entries = vec![(
            ("minecraft".to_owned(), "overworld".to_owned()),
            overworld,
            false,
        )];
        let dimension = |id: &str| ModDimension {
            id: id.into(),
            stem: String::new(),
            dimension_type:
                r#"{"min_y":-128,"height":448,"attributes":{"minecraft:gameplay/fast_lava":true}}"#
                    .into(),
            sea_level: None,
        };
        mod_dimension_types(
            &[dimension("test:pocket"), dimension("test:packed")],
            &mut entries,
        );
        let files = [(
            "dimension_type/test:packed".to_owned(),
            vec![br##"{"min_y":0,"height":64,"logical_height":64,"has_ceiling":false,"has_skylight":true,"has_ender_dragon_fight":false,"ambient_light":1e39,"coordinate_scale":1.0,"infiniburn":"#minecraft:infiniburn_overworld","monster_spawn_light_level":{"type":"clamped_normal","mean":1e39,"deviation":-1e39,"min_inclusive":0,"max_inclusive":15},"monster_spawn_block_light_limit":0,"attributes":{"visual/sky_color":"#ff0000"}}"##.to_vec()],
        )];
        let entries = pack_entries("dimension_type", &PackOverlay::new(&files), entries);

        let entry = |path: &str| entries.iter().find(|((_, p), ..)| p == path).unwrap();
        let (_, pocket, sent) = entry("pocket");
        assert!(*sent, "no client knows a mod's type");
        assert_eq!(
            *pocket,
            json_to_nbt(&serde_json::json!({
                "min_y":-128,"height":448,"ambient_light":0.0,
                "attributes":{"minecraft:visual/sky_color":"#78a7ff","minecraft:gameplay/fast_lava":true}
            }))
        );
        assert_eq!(entry("packed").1.get("min_y"), Some(&Tag::Int(0)));
        assert_eq!(entry("packed").1.get("height"), Some(&Tag::Int(64)));
        assert_eq!(
            entry("packed").1.get("ambient_light"),
            Some(&Tag::Float(f32::INFINITY.to_bits()))
        );
        let provider = entry("packed").1.get("monster_spawn_light_level").unwrap();
        assert_eq!(
            provider.get("mean"),
            Some(&Tag::Float(f32::INFINITY.to_bits()))
        );
        assert_eq!(
            provider.get("deviation"),
            Some(&Tag::Float(f32::NEG_INFINITY.to_bits()))
        );
        let attrs = entry("packed").1.get("attributes").unwrap();
        assert_eq!(
            attrs.get("minecraft:visual/sky_color"),
            Some(&Tag::List(vec![
                Tag::Float(1.0_f32.to_bits()),
                Tag::Float(0),
                Tag::Float(0)
            ]))
        );
        assert!(!entry("overworld").2);
    }

    #[test]
    fn a_changed_dimension_type_is_sent_with_its_shape() {
        let vanilla: Value =
            serde_json::from_str(r#"{"min_y":-64,"height":384,"logical_height":384,"has_skylight":true,"ultrawarm":false}"#)
                .unwrap();
        let vanilla = json_to_nbt(&vanilla);
        let mut levels = LevelShapes::VANILLA;
        assert_eq!(
            changed_dimension_type("minecraft:overworld", &vanilla, &levels),
            None
        );
        levels.overworld = crate::chunk::LevelShape::new(-64, 2096, 2096, true, true).unwrap();
        let sent = changed_dimension_type("minecraft:overworld", &vanilla, &levels).unwrap();
        assert_eq!(
            (sent.get("min_y"), sent.get("height")),
            (Some(&Tag::Int(-64)), Some(&Tag::Int(2096)))
        );
        assert_eq!(sent.get("has_ceiling"), Some(&Tag::Byte(1)));
        assert_eq!(
            sent.get("ultrawarm"),
            Some(&Tag::Byte(0)),
            "the rest of the type is kept"
        );
        assert_eq!(
            changed_dimension_type("minecraft:the_nether", &vanilla, &levels),
            None
        );
        assert_eq!(
            changed_dimension_type("minecraft:overworld_caves", &vanilla, &levels),
            None
        );
    }
}

#[path = "data_condition_filter.rs"]
mod condition_filter;

#[cfg(test)]
mod transient_dialog_tests {
    use super::*;
    #[test]
    fn a_dialog_naming_a_transient_component_refuses_the_registry_load() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("items.json"), br#"{"minecraft:stone":{}}"#).unwrap();
        let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
        let files = [
            (
                "dialog/test:bad_item".into(),
                vec![br#"{"type":"minecraft:notice","title":"bad","body":[{"type":"minecraft:item","item":{"id":"minecraft:stone","components":{"!minecraft:map_post_processing":{}}}}]}"#.to_vec()],
            ),
            (
                "dialog/test:bad_hover".into(),
                vec![br#"{"type":"minecraft:notice","title":{"text":"bad","hover_event":{"action":"show_item","id":"minecraft:stone","components":{"minecraft:additional_trade_cost":3}}}}"#.to_vec()],
            ),
            (
                "dialog/test:good".into(),
                vec![br#"{"type":"minecraft:notice","title":"kept","body":[{"type":"minecraft:item","item":{"id":"minecraft:stone","components":{"minecraft:tooltip_display":{"hidden_components":["minecraft:creative_slot_lock"]}}}}]}"#.to_vec()],
            ),
        ];
        let conditions = sunrise_world::conditions::LoadConditions::new([]);
        let (_, admitted) = PackRegistryDefinitions::stage(
            root.path(),
            &defaults,
            &sunrise_world::loot::PackOverlay {
                files: &files,
                conditions: &conditions,
                ..sunrise_world::loot::PackOverlay::default()
            },
            &BTreeMap::new(),
            &Default::default(),
            |_| true,
        );
        let error = admitted.unwrap_err();
        assert!(error.contains("minecraft:dialog:"), "{error}");
        assert!(error.contains("test:bad_item: "), "{error}");
        assert!(error.contains("test:bad_hover: "), "{error}");
        assert!(error.contains("minecraft:map_post_processing"), "{error}");
        assert!(!error.contains("test:good"), "{error}");
    }
}
