use crate::game::{
    data_map_files::{self, folder},
    data_map_json::to_tag,
};
pub use mod_api::game::DataMapType;
use serde_json::Value;
use std::{
    borrow::Cow,
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};
use sunrise_core::{identifier::Identifier, nbt::Tag};
use sunrise_sim::neoforge_mob_maps::{BuiltinMap, NeoForgeMobMaps};

/// The registries mods read through a typed key (`DataMapEntry`): always
/// known, even where the command registries list no ids.
const TYPED: [&str; 9] = [
    "minecraft:item",
    "minecraft:block",
    "minecraft:fluid",
    "minecraft:mob_effect",
    "minecraft:entity_type",
    "minecraft:enchantment",
    "minecraft:block_entity_type",
    "minecraft:villager_profession",
    "minecraft:worldgen/biome",
];

/// The registries a data load resolves maps against.
pub trait MapRegistries {
    /// Whether the game names `registry`: built in, a pack's or a datapack
    /// registry a mod declared.
    fn known(&self, registry: &str) -> bool;
    /// Whether `entry` is in `registry`; true where the registry's ids aren't listed.
    fn contains(&self, registry: &str, entry: &str) -> bool;
    /// The members of `tag` (`minecraft:logs`) in `registry`; none for an unknown tag.
    fn tag(&self, registry: &str, tag: &str) -> Vec<String>;
}

/// A world's registries while its server tables build: candidate IDs, the
/// declared registries' admitted entries, and the candidate's bound tags.
/// Standalone readers can also supply their own registry metadata and packs.
pub struct WorldRegistries<'a> {
    ids: BTreeMap<&'a str, &'a [String]>,
    biome_keys: Option<&'a [mod_api::BiomeKey]>,
    candidate: Option<&'a super::load_conditions::RegistryView>,
    datapack: Option<&'a mod_api::DatapackRegistrySet>,
    items: &'a sunrise_items::items::Items,
    bound_tags: Option<&'a PackTags>,
    root: &'a std::path::Path,
    overlay: &'a sunrise_world::loot::PackOverlay<'a>,
    id_sets: RefCell<HashMap<String, HashSet<String>>>,
    tags: RefCell<HashMap<String, HashMap<String, HashSet<String>>>>,
}
pub type PackTags = HashMap<&'static str, HashMap<String, HashSet<String>>>;
impl<'a> WorldRegistries<'a> {
    pub fn new(
        ids: impl IntoIterator<Item = (&'a str, &'a [String])>,
        datapack: Option<&'a mod_api::DatapackRegistrySet>,
        items: &'a sunrise_items::items::Items,
        root: &'a std::path::Path,
        overlay: &'a sunrise_world::loot::PackOverlay<'a>,
    ) -> Self {
        Self {
            ids: ids.into_iter().collect(),
            biome_keys: None,
            candidate: None,
            datapack,
            items,
            bound_tags: None,
            root,
            overlay,
            id_sets: RefCell::default(),
            tags: RefCell::default(),
        }
    }

    /// Resolve biome entries only against registered keys, not save placeholders.
    pub fn with_biome_keys(mut self, keys: &'a [mod_api::BiomeKey]) -> Self {
        self.biome_keys = Some(keys);
        self
    }

    /// Prefer the complete staged registry and tag bindings over command metadata.
    pub fn with_candidate(
        mut self,
        candidate: &'a super::load_conditions::RegistryView,
        tags: &'a PackTags,
    ) -> Self {
        self.candidate = Some(candidate);
        self.bound_tags = Some(tags);
        self
    }

    /// Whether a mod's declaration names `registry`.
    fn declared(&self, registry: &str) -> bool {
        mod_api::__private::datapack_registry_declarations()
            .iter()
            .any(|declaration| declaration.key == registry)
    }
}
impl MapRegistries for WorldRegistries<'_> {
    fn known(&self, registry: &str) -> bool {
        self.candidate
            .is_some_and(|candidate| candidate.ids.contains_key(registry))
            || self.ids.contains_key(registry)
            || TYPED.contains(&registry)
            || self.declared(registry)
    }
    fn contains(&self, registry: &str, entry: &str) -> bool {
        if registry == sunrise_sim::lan::data::BIOME_REGISTRY
            && let Some(keys) = self.biome_keys
        {
            let mut sets = self.id_sets.borrow_mut();
            let set = sets
                .entry(registry.to_owned())
                .or_insert_with(|| keys.iter().map(ToString::to_string).collect());
            return set.contains(entry);
        }
        if self.declared(registry) {
            return self
                .datapack
                .is_some_and(|set| set.contains_entry(registry, entry));
        }
        if let Some(ids) = self
            .candidate
            .and_then(|candidate| candidate.ids.get(registry))
        {
            return ids.contains(entry);
        }
        if let Some(ids) = self.ids.get(registry).filter(|ids| !ids.is_empty()) {
            let mut sets = self.id_sets.borrow_mut();
            let set = sets
                .entry(registry.to_owned())
                .or_insert_with(|| ids.iter().cloned().collect());
            return set.contains(entry);
        }
        // A typed registry the sync did not list: no source says its entries.
        true
    }
    fn tag(&self, registry: &str, tag: &str) -> Vec<String> {
        let members = |tags: &HashMap<String, HashSet<String>>| -> Vec<String> {
            tags.get(tag)
                .map(|set| set.iter().cloned().collect())
                .unwrap_or_default()
        };
        if self.declared(registry) {
            return self.datapack.map_or_else(Vec::new, |set| {
                mod_api::__private::game::datapack_tag_members(set, registry, tag).to_vec()
            });
        }
        match registry {
            "minecraft:item" => members(&self.items.item_tags),
            "minecraft:block" => members(&self.items.block_tags),
            "minecraft:fluid" => members(&self.items.fluid_tags),
            sunrise_items::items::EntityTypeTags::REGISTRY => {
                members(self.items.entity_type_tags.members())
            }
            _ => {
                if let Some(tags) = self
                    .bound_tags
                    .and_then(|tags| tags.get(folder(registry).as_str()))
                {
                    return members(tags);
                }
                let mut loaded = self.tags.borrow_mut();
                let tags = loaded.entry(registry.to_owned()).or_insert_with(|| {
                    sunrise_items::items::load_tags(self.root, &folder(registry), self.overlay)
                        .unwrap_or_else(|e| {
                            log::warn!("Sunrise: the {registry} tags data maps read: {e}");
                            HashMap::new()
                        })
                });
                members(tags)
            }
        }
    }
}

pub type LoadedMaps<V> = BTreeMap<String, HashMap<String, BTreeMap<String, V>>>;

#[derive(Default)]
pub struct DataMaps {
    pub biome_keys: std::sync::Arc<mod_api::__private::BiomeKeys>,
    maps: BTreeMap<String, HashMap<String, Vec<(String, Tag)>>>,
    pub mob_maps: std::sync::Arc<sunrise_sim::neoforge_mob_maps::NeoForgeMobMaps>,
}

/// `minecraft:` for an id written without a namespace.
pub(in crate::game) fn full_id(id: &str) -> Cow<'_, str> {
    if id.contains(':') {
        Cow::Borrowed(id)
    } else {
        Cow::Owned(format!("minecraft:{id}"))
    }
}

/// Merge the types' values from the server's tables' pack files, in pack order (mods' below
/// the packs'), with ids and tags resolved against `registries`. Unknown registries and bad
/// type ids are left out with a warning; a file that isn't a valid data map or contains an
/// undecodable value is skipped atomically with an error.
pub fn load_maps<V: Clone>(
    types: &[DataMapType],
    overlay: &sunrise_world::loot::PackOverlay<'_>,
    registries: &dyn MapRegistries,
    decode: impl Fn(&str, &Value) -> Result<V, String>,
) -> LoadedMaps<V> {
    let mut maps: LoadedMaps<V> = BTreeMap::new();
    for ty in types {
        if !registries.known(&ty.registry) {
            log::warn!(
                "Sunrise: data map {} is for {}, a registry the game doesn't name: left out",
                ty.id,
                ty.registry
            );
            continue;
        }
        let Some(id) = Identifier::parse_namespaced(&ty.id) else {
            log::warn!(
                "Sunrise: data map type {} is not a valid namespaced identifier: left out",
                ty.id
            );
            continue;
        };
        let registered = maps.entry(ty.registry.clone()).or_default();
        if registered.contains_key(&ty.id) {
            log::warn!(
                "Sunrise: data map {} registered twice: the first is kept",
                ty.id
            );
            continue;
        }
        let (map, _) = data_map_files::load(
            &id,
            &ty.registry,
            BTreeMap::new(),
            overlay,
            |tag| registries.tag(&ty.registry, tag),
            |raw| {
                // A direct object can have a field named `value`; only an object with
                // exactly the entry wrapper's fields is unwrapped. Conditions are already read.
                if let Value::Object(object) = raw
                    && let Some(value) = object.get("value")
                    && object
                        .iter()
                        .all(|(k, v)| k == "value" || (k == "replace" && v.is_boolean()))
                {
                    decode(&ty.id, value)
                } else {
                    decode(&ty.id, raw)
                }
            },
            |entry| registries.contains(&ty.registry, entry),
        );
        registered.insert(ty.id.clone(), map);
    }
    maps
}

pub struct Loaded {
    pub values: Vec<(BuiltinMap, BTreeMap<String, Tag>)>,
    pub snapshot: Arc<NeoForgeMobMaps>,
}

impl DataMaps {
    pub fn with_biome_keys(mut self, keys: mod_api::__private::BiomeKeys) -> Self {
        self.biome_keys = std::sync::Arc::new(keys);
        self
    }

    /// Merge the pack files and publish their winning values as NBT.
    pub fn load(
        types: &[DataMapType],
        overlay: &sunrise_world::loot::PackOverlay<'_>,
        registries: &dyn MapRegistries,
    ) -> Self {
        let merged = load_maps(types, overlay, registries, |_, value| {
            to_tag(value).map_err(|_| "value cannot be represented as NBT".to_owned())
        });
        let mut published = Self::default();
        for (registry, maps) in merged {
            let mut types = HashMap::with_capacity(maps.len());
            for (id, values) in maps {
                types.insert(id, values.into_iter().collect());
            }
            published.maps.insert(registry, types);
        }
        published
    }

    pub fn with_builtins(mut self, builtin: Loaded) -> Self {
        for (definition, values) in builtin.values {
            if self
                .maps
                .entry(definition.registry.to_owned())
                .or_default()
                .insert(definition.id.to_owned(), values.into_iter().collect())
                .is_some()
            {
                log::warn!(
                    "Sunrise: builtin data map {} is reserved: engine definition kept",
                    definition.id
                );
            }
        }
        self.mob_maps = builtin.snapshot;
        self
    }

    /// The registries with a loaded type, in id order: what `data_maps_updated` names.
    pub fn registries(&self) -> Vec<String> {
        self.maps.keys().cloned().collect()
    }

    /// Map `type_id` of `registry`, refused where no mod registered it there.
    fn map(
        &self,
        registry: &str,
        type_id: &str,
    ) -> Result<&[(String, Tag)], mod_api::DataMapError> {
        self.maps
            .get(registry)
            .and_then(|maps| maps.get(type_id))
            .map(Vec::as_slice)
            .ok_or(mod_api::DataMapError::UnknownType)
    }

    /// `entry`'s value in map `type_id` of `registry`, as NBT: none where the
    /// map gives it none.
    pub fn value(
        &self,
        registry: &str,
        type_id: &str,
        entry: &str,
    ) -> Result<Option<&Tag>, mod_api::DataMapError> {
        let rows = self.map(registry, type_id)?;
        match rows.binary_search_by(|(id, _)| id.as_str().cmp(entry)) {
            Ok(index) => Ok(Some(&rows[index].1)),
            Err(_) => Ok(None),
        }
    }

    /// Every entry map `type_id` of `registry` gives a value, in id order.
    pub fn entries(
        &self,
        registry: &str,
        type_id: &str,
    ) -> Result<&[(String, Tag)], mod_api::DataMapError> {
        self.map(registry, type_id)
    }
}
