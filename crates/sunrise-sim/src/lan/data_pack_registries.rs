//! Winning definitions for the synchronized registries that read world packs.
use super::{condition_filter, json_to_nbt, synchronized_entries};
use crate::dialog::Reference;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    marker::PhantomData,
    path::Path,
    sync::Arc,
};
use sunrise_core::{
    identifier::Identifier,
    nbt::{Compound, Tag},
};
use sunrise_items::{
    enchant::{EnchantmentProvider, Enchantments, HolderSetBinding},
    items::{Instrument, Items, Song},
};
use sunrise_world::{
    environment_timeline::Timeline, item_defaults::ItemDefaults, loot::PackOverlay,
};

/// One data-driven registry kind: `RegistryDataLoader.WORLD_REGISTRIES`'s entry for a
/// registry a world's packs add to or replace, with the typed decode of its codec.
/// A row is synchronized when `SYNCHRONIZED` lists its key (`trial_spawner` and
/// `enchantment_provider` are not: the server's gameplay reads them and no client is sent them);
/// a decode error of any row refuses the whole load.
pub struct RegistryKind {
    pub key: &'static str,
    decode: fn(&mut Decoder<'_>, &str, Value) -> Result<Definition, String>,
}

/// The registry kinds, in `WORLD_REGISTRIES` order. The others stay the game's own: Sunrise's
/// gameplay reads them from the prepared resources only.
/// Enchantments are among them: the items' enchantment catalog reads a
/// world's pack definitions too, so their ids must reach clients.
/// World clocks are too: a dimension type's `default_clock` must name a
/// clock the client was sent, or it reads that dimension's time as zero.
/// Block transformers, trial spawner configs, sulfur cube archetypes and decorated pot
/// patterns are too: gameplay and drawing read the world's table of them, never the resource
/// root's files, so the prepared files are decoded here like a pack's.
pub const REGISTRY_KINDS: &[RegistryKind] = &[
    RegistryKind {
        key: "minecraft:dimension_type",
        decode: decode_dimension_type,
    },
    RegistryKind {
        key: "minecraft:chat_type",
        decode: decode_chat_type,
    },
    // Not synchronized (absent from `SYNCHRONIZED`): the server's gameplay reads it, no client is sent it.
    RegistryKind {
        key: "minecraft:trial_spawner",
        decode: decode_trial_spawner,
    },
    RegistryKind {
        key: "minecraft:sulfur_cube_archetype",
        decode: decode_sulfur_cube_archetype,
    },
    RegistryKind {
        key: "minecraft:trim_pattern",
        decode: decode_trim_pattern,
    },
    RegistryKind {
        key: "minecraft:trim_material",
        decode: decode_trim_material,
    },
    RegistryKind {
        key: "minecraft:wolf_variant",
        decode: decode_wolf_variant,
    },
    RegistryKind {
        key: "minecraft:wolf_sound_variant",
        decode: decode_wolf_sound_variant,
    },
    RegistryKind {
        key: "minecraft:cat_variant",
        decode: decode_cat_variant,
    },
    RegistryKind {
        key: "minecraft:cat_sound_variant",
        decode: decode_cat_sound_variant,
    },
    RegistryKind {
        key: "minecraft:pig_variant",
        decode: decode_pig_variant,
    },
    RegistryKind {
        key: "minecraft:pig_sound_variant",
        decode: decode_pig_sound_variant,
    },
    RegistryKind {
        key: "minecraft:cow_sound_variant",
        decode: decode_cow_sound_variant,
    },
    RegistryKind {
        key: "minecraft:cow_variant",
        decode: decode_cow_variant,
    },
    RegistryKind {
        key: "minecraft:chicken_sound_variant",
        decode: decode_chicken_sound_variant,
    },
    RegistryKind {
        key: "minecraft:chicken_variant",
        decode: decode_chicken_variant,
    },
    RegistryKind {
        key: "minecraft:frog_variant",
        decode: decode_frog_variant,
    },
    RegistryKind {
        key: "minecraft:zombie_nautilus_variant",
        decode: decode_zombie_nautilus_variant,
    },
    RegistryKind {
        key: "minecraft:painting_variant",
        decode: decode_painting_variant,
    },
    RegistryKind {
        key: "minecraft:banner_pattern",
        decode: decode_banner_pattern,
    },
    RegistryKind {
        key: "minecraft:enchantment",
        decode: decode_enchantment,
    },
    // Not synchronized (absent from `SYNCHRONIZED`): the server's gameplay reads it, no client is sent it.
    RegistryKind {
        key: "minecraft:enchantment_provider",
        decode: decode_enchantment_provider,
    },
    RegistryKind {
        key: "minecraft:jukebox_song",
        decode: decode_jukebox_song,
    },
    RegistryKind {
        key: "minecraft:instrument",
        decode: decode_instrument,
    },
    RegistryKind {
        key: "minecraft:test_environment",
        decode: decode_test_environment,
    },
    RegistryKind {
        key: "minecraft:test_instance",
        decode: decode_test_instance,
    },
    RegistryKind {
        key: "minecraft:dialog",
        decode: decode_dialog,
    },
    RegistryKind {
        key: "minecraft:world_clock",
        decode: decode_world_clock,
    },
    RegistryKind {
        key: "minecraft:decorated_pot_pattern",
        decode: decode_decorated_pot_pattern,
    },
    RegistryKind {
        key: "minecraft:block_transformer",
        decode: decode_block_transformer,
    },
    RegistryKind {
        key: "minecraft:worldgen/block_state_provider",
        decode: decode_block_state_provider,
    },
    RegistryKind {
        key: "minecraft:timeline",
        decode: decode_timeline,
    },
];

/// The registry `ChatType` entries live in (`RegistryRef<ChatType>`).
pub enum ChatType {}

/// An entry of the server's registry `K`, held by its network id: the entry's index in the
/// server's sync order. Engine-internal: a client resolves it through the ids the server
/// sent, in the server's order, and `mod_api` exposes no raw index.
pub struct RegistryRef<K>(u32, PhantomData<fn() -> K>);

impl<K> RegistryRef<K> {
    pub fn new(id: u32) -> Self {
        Self(id, PhantomData)
    }

    pub fn network_id(&self) -> u32 {
        self.0
    }
}

impl<K> Clone for RegistryRef<K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K> Copy for RegistryRef<K> {}

impl<K> PartialEq for RegistryRef<K> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<K> Eq for RegistryRef<K> {}

impl<K> std::fmt::Debug for RegistryRef<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RegistryRef({})", self.0)
    }
}

type Registry = BTreeMap<(String, String), Option<(Definition, bool)>>;

/// Validated once for the world session. A false condition suppresses the
/// winning entry, including any prepared or lower-priority definition.
#[derive(Default)]
pub struct PackRegistryDefinitions {
    entries: BTreeMap<&'static str, Registry>,
    /// Admitted prepared holders for registries whose pack winners are stored above.
    pub(super) prepared_ids: Vec<(&'static str, String)>,
}

/// Keep the admitted item definitions alongside the exact wire encoding.
enum Definition {
    Encoded(Tag),
    Enchantment(Value),
    EnchantmentProvider(EnchantmentProvider),
    Instrument(Instrument, Tag),
    JukeboxSong(Song, Tag),
    Painting(crate::entities::PaintingVariant, Tag),
    BannerPattern(crate::banner::Pattern, Tag),
    /// A definition whose codec ran and whose gameplay reads the JSON itself, with its wire
    /// encoding when the registry is synchronized.
    Validated(Value, Option<Tag>),
    Timeline(Arc<Timeline>, Tag),
    WolfVariant(crate::mobs::variants::WolfVariant, Tag),
    WolfSoundVariant(crate::mobs::variants::WolfSoundVariant, Tag),
    CatVariant(crate::mobs::variants::CatVariant, Tag),
    CatSoundVariant(crate::mobs::variants::CatSoundVariant, Tag),
    PigVariant(crate::mobs::variants::PigVariant, Tag),
    PigSoundVariant(crate::mobs::variants::PigSoundVariant, Tag),
    CowVariant(crate::mobs::variants::CowVariant, Tag),
    CowSoundVariant(crate::mobs::variants::CowSoundVariant, Tag),
    ChickenVariant(crate::mobs::variants::ChickenVariant, Tag),
    ChickenSoundVariant(crate::mobs::variants::ChickenSoundVariant, Tag),
    FrogVariant(crate::mobs::variants::FrogVariant, Tag),
    ZombieNautilusVariant(crate::mobs::variants::ZombieNautilusVariant, Tag),
}

impl PackRegistryDefinitions {
    /// Registry holders from this load, including tombstones for skipped or
    /// refused winners. Standalone command metadata is not a holder source.
    pub fn entries(&self) -> impl Iterator<Item = (&'static str, String, bool)> + '_ {
        self.entries
            .iter()
            .flat_map(|(&registry, entries)| {
                entries.iter().map(move |((namespace, path), entry)| {
                    (registry, format!("{namespace}:{path}"), entry.is_some())
                })
            })
            .chain(
                self.prepared_ids
                    .iter()
                    .map(|(registry, id)| (*registry, id.clone(), true)),
            )
    }

    /// RegistryDataLoader's collected errors: try every winning definition,
    /// then refuse the entire load and report each failure by registry and id.
    pub(super) fn load(
        resources: &Path,
        defaults: &ItemDefaults,
        overlay: &PackOverlay<'_>,
        read_errors: &BTreeMap<String, String>,
        text_registries: &crate::command::Registries,
        sound_exists: impl Fn(&str) -> bool,
    ) -> Result<Self, String> {
        let (definitions, admitted) = Self::stage(
            resources,
            defaults,
            overlay,
            read_errors,
            text_registries,
            sound_exists,
        );
        admitted.map(|()| definitions)
    }

    /// Keep replaced keys even on failure, so the caller can collect errors
    /// from other registry files without checking a losing definition.
    pub(super) fn stage(
        resources: &Path,
        defaults: &ItemDefaults,
        overlay: &PackOverlay<'_>,
        read_errors: &BTreeMap<String, String>,
        text_registries: &crate::command::Registries,
        sound_exists: impl Fn(&str) -> bool,
    ) -> (Self, Result<(), String>) {
        let mut definitions = Self::default();
        let mut errors = BTreeMap::<_, BTreeMap<_, _>>::new();
        let mut decoder = Decoder {
            registries: text_registries,
            defaults,
            sound_exists: &sound_exists,
            dialog_references: Vec::new(),
            holder_references: Vec::new(),
        };
        for table in REGISTRY_KINDS {
            let registry = table.key;
            let kind = registry.trim_start_matches("minecraft:");
            let entries = definitions.entries.entry(registry).or_default();
            for (key, error) in read_errors {
                if let Some(id) = key.strip_prefix(kind).and_then(|id| id.strip_prefix('/')) {
                    if let Ok(id) = Identifier::parse(id) {
                        entries.insert((id.namespace().to_owned(), id.path().to_owned()), None);
                    }
                    errors
                        .entry(registry)
                        .or_default()
                        .insert(id.to_owned(), error.clone());
                }
            }
            for (id, files) in overlay.kind(kind) {
                if errors
                    .get(registry)
                    .is_some_and(|errors| errors.contains_key(id))
                {
                    continue;
                }
                let mut decode = || -> Result<(), String> {
                    let id = Identifier::parse(id).map_err(|error| error.to_string())?;
                    let key = (id.namespace().to_owned(), id.path().to_owned());
                    // Reserve the winner even if it fails admission or its condition is false.
                    let definition = entries.entry(key).or_insert(None);
                    let file = format!("data/{}/{kind}/{}.json", id.namespace(), id.path());
                    let value: Value = serde_json::from_slice(PackOverlay::top(files))
                        .map_err(|error| format!("{file}: {error}"))?;
                    if overlay
                        .conditions
                        .allows_file_isolated(&value, &file)
                        .map_err(|error| format!("{file}: {error}"))?
                    {
                        *definition = Some((
                            (table.decode)(&mut decoder, &id.to_string(), value)
                                .map_err(|error| format!("{file}: {error}"))?,
                            true,
                        ));
                    }
                    Ok(())
                };
                if let Err(error) = decode() {
                    errors
                        .entry(registry)
                        .or_default()
                        .insert(id.to_owned(), error);
                }
            }
            if Self::includes_prepared_entries(registry) {
                let mut prepared =
                    synchronized_entries(&resources.join("data/minecraft"), resources, registry);
                let had_candidates = !entries.is_empty() || !prepared.is_empty();
                // The pack winner (even if refused or skipped) hides resources below it.
                prepared.retain(|(key, _, _)| !entries.contains_key(key));
                for (key, value, from_pack) in prepared {
                    if !overlay
                        .conditions
                        .allows_parts(&value, kind, &key.0, &key.1)
                    {
                        entries.insert(key, None);
                        continue;
                    }
                    let definition = if kind == "enchantment" {
                        Definition::Enchantment(value)
                    } else if from_pack || Self::decodes_prepared(registry) || is_variant_kind(kind)
                    {
                        let id = format!("{}:{}", key.0, key.1);
                        let file = format!("data/{}/{kind}/{}.json", key.0, key.1);
                        match (table.decode)(&mut decoder, &id, value) {
                            Ok(definition) => definition,
                            Err(error) => {
                                errors
                                    .entry(registry)
                                    .or_default()
                                    .insert(id, format!("{file}: {error}"));
                                entries.insert(key, None);
                                continue;
                            }
                        }
                    } else {
                        let wire = json_to_nbt(&value);
                        match Instrument::decode(&value) {
                            Ok(instrument) => Definition::Instrument(instrument, wire),
                            Err(_) => Definition::Encoded(wire),
                        }
                    };
                    entries.insert(key, Some((definition, from_pack)));
                }
                // `RegistryValidator.nonEmpty()` for the variant registries.
                if had_candidates
                    && is_variant_kind(kind)
                    && entries.values().all(Option::is_none)
                    && !errors.contains_key(registry)
                {
                    errors.entry(registry).or_default().insert(
                        "(empty)".into(),
                        format!("registry {registry} must not be empty"),
                    );
                }
            }
        }
        if let Some(entries) = definitions.entries.get_mut("minecraft:enchantment") {
            let enchantments: Vec<_> = entries
                .iter_mut()
                .filter_map(|((namespace, path), entry)| {
                    let (Definition::Enchantment(value), from_pack) = entry.as_mut()? else {
                        return None;
                    };
                    Some(((namespace.clone(), path.clone()), value, *from_pack))
                })
                .collect();
            let binding = enchantments
                .iter()
                .any(|(_, value, _)| HolderSetBinding::needed(value))
                .then(|| {
                    HolderSetBinding::load(
                        resources,
                        overlay,
                        defaults,
                        enchantments
                            .iter()
                            .map(|((namespace, path), _, _)| format!("{namespace}:{path}"))
                            .collect(),
                    )
                });
            let mut rejected = Vec::new();
            for (key, value, from_pack) in enchantments {
                let id = format!("{}:{}", key.0, key.1);
                let result = if let Some(binding) =
                    binding.as_ref().filter(|_| HolderSetBinding::needed(value))
                {
                    match binding {
                        Ok(binding) => binding.bind(&id, value).map_err(|error| error.to_string()),
                        Err(error) => Err(error.to_string()),
                    }
                } else {
                    Ok(())
                };
                // Holder binding may change the transmitted shape. Preflight
                // the exact JSON-to-NBT projection the registry packet uses.
                let result = result.and_then(|()| {
                    if from_pack {
                        check_network_nbt(&json_to_nbt(value))?;
                    }
                    Ok(())
                });
                if let Err(error) = result {
                    errors.entry("minecraft:enchantment").or_default().insert(
                        id.clone(),
                        format!("{}: {error}", registry_file_path("enchantment", &id)),
                    );
                    rejected.push(key);
                }
            }
            for key in rejected {
                entries.insert(key, None);
            }
        }
        // Validate `minecraft:enchantment_provider` holder and tag references against
        // the admitted `minecraft:enchantment` registry (RegistryDataLoader freeze).
        let held_enchantments: BTreeSet<String> = definitions
            .entries
            .get("minecraft:enchantment")
            .into_iter()
            .flatten()
            .filter(|(_, entry)| entry.is_some())
            .map(|((namespace, path), _)| format!("{namespace}:{path}"))
            .collect();
        if let Some(provider_entries) = definitions
            .entries
            .get_mut("minecraft:enchantment_provider")
        {
            let needs_tags = provider_entries.values().any(|entry| {
                matches!(
                    entry,
                    Some((Definition::EnchantmentProvider(provider), _))
                        if provider.referenced_tag().is_some()
                )
            });
            let enchantment_tags = if needs_tags {
                let root = resources.join("data/minecraft");
                bind_registry_tags(&root, overlay, "enchantment", |id, _| {
                    held_enchantments.contains(id).then(|| id.to_owned())
                })
            } else {
                BTreeMap::new()
            };
            let mut rejected = Vec::new();
            for (key, entry) in provider_entries.iter() {
                let Some((Definition::EnchantmentProvider(provider), _)) = entry else {
                    continue;
                };
                let id = format!("{}:{}", key.0, key.1);
                let mut error = None;
                for enchantment in provider.referenced_enchantments() {
                    if !held_enchantments.contains(enchantment.as_ref()) {
                        error = Some(format!("unbound enchantment holder: {enchantment}"));
                        break;
                    }
                }
                if error.is_none()
                    && let Some(tag) = provider.referenced_tag()
                    && !enchantment_tags.contains_key(tag)
                {
                    error = Some(format!("unbound enchantment tag: #{tag}"));
                }
                if let Some(error) = error {
                    errors
                        .entry("minecraft:enchantment_provider")
                        .or_default()
                        .insert(
                            id.clone(),
                            format!(
                                "{}: {error}",
                                registry_file_path("enchantment_provider", &id)
                            ),
                        );
                    rejected.push(key.clone());
                }
            }
            for key in rejected {
                provider_entries.insert(key, None);
            }
        }
        // RegistryDataLoader.validateRegistry: every dialog a winner names is bound.
        let dialogs = definitions.entries.get("minecraft:dialog");
        let root = resources.join("data/minecraft");
        let mut prepared = synchronized_entries(&root, resources, "minecraft:dialog");
        prepared.retain(|(key, ..)| !dialogs.is_some_and(|d| d.contains_key(key)));
        condition_filter::filter(&mut prepared, false, overlay.conditions, "dialog");
        let held: BTreeSet<String> = dialogs
            .into_iter()
            .flatten()
            .filter(|(_, entry)| entry.is_some())
            .map(|((namespace, path), _)| format!("{namespace}:{path}"))
            .chain(
                prepared
                    .into_iter()
                    .map(|((namespace, path), ..)| format!("{namespace}:{path}")),
            )
            .collect();
        let (tags, mut unbound) = bind_dialog_tags(&root, overlay, |id| held.contains(id));
        let mut unbound_tags = BTreeSet::new();
        for reference in decoder.dialog_references.into_iter().flatten() {
            match reference {
                Reference::Id(id) if !held.contains(&id) => unbound.insert(id),
                Reference::Tag(tag) if !tags.contains_key(&tag) => unbound_tags.insert(tag),
                _ => false,
            };
        }
        for (what, names) in [("values", unbound), ("tags", unbound_tags)] {
            if !names.is_empty() {
                errors.entry("minecraft:dialog").or_default().insert(
                    format!("(unbound {what})"),
                    format!("unbound {what} in registry: {names:?}"),
                );
            }
        }
        // WorldClock.CODEC holder binding and Timeline.validateRegistry.
        let clocks = definitions.entries.get("minecraft:world_clock");
        let mut prepared_clocks = synchronized_entries(&root, resources, "minecraft:world_clock");
        prepared_clocks.retain(|(key, ..)| !clocks.is_some_and(|c| c.contains_key(key)));
        condition_filter::filter(
            &mut prepared_clocks,
            false,
            overlay.conditions,
            "world_clock",
        );
        let held_clocks: BTreeSet<String> = clocks
            .into_iter()
            .flatten()
            .filter(|(_, entry)| entry.is_some())
            .map(|((namespace, path), _)| format!("{namespace}:{path}"))
            .chain(
                prepared_clocks
                    .into_iter()
                    .map(|((namespace, path), ..)| format!("{namespace}:{path}")),
            )
            .collect();
        if let Some(entries) = definitions.entries.get_mut("minecraft:timeline") {
            let mut rejected = Vec::new();
            let mut valid_timelines: Vec<(String, (String, String), Arc<Timeline>)> = Vec::new();
            for (key, entry) in entries.iter() {
                let Some((Definition::Timeline(timeline, _), _)) = entry.as_ref() else {
                    continue;
                };
                let id = format!("{}:{}", key.0, key.1);
                if !held_clocks.contains(&timeline.clock) {
                    errors
                        .entry("minecraft:timeline")
                        .or_default()
                        .insert(id, format!("unknown world_clock {}", timeline.clock));
                    rejected.push(key.clone());
                } else {
                    valid_timelines.push((id, key.clone(), Arc::clone(timeline)));
                }
            }
            let duplicate_errors = Timeline::validate_registry(
                valid_timelines
                    .iter()
                    .map(|(id, _, timeline)| (id.as_str(), timeline.as_ref())),
            );
            for (id, error) in duplicate_errors {
                errors
                    .entry("minecraft:timeline")
                    .or_default()
                    .insert(id.clone(), error);
                if let Some((_, key, _)) = valid_timelines.iter().find(|(tid, _, _)| *tid == id) {
                    rejected.push(key.clone());
                }
            }
            for key in rejected {
                entries.insert(key, None);
            }
        }
        // RegistryDataLoader.validateRegistry: every test_environment, world_clock and timeline
        // holder a winning test_environment or test_instance names is bound.
        let held = |registry: &str| -> BTreeSet<String> {
            definitions
                .entries
                .get(registry)
                .into_iter()
                .flatten()
                .filter(|(_, entry)| entry.is_some())
                .map(|((namespace, path), _)| format!("{namespace}:{path}"))
                .collect()
        };
        let held_environments = held("minecraft:test_environment");
        let held_timelines = held("minecraft:timeline");
        for reference in decoder.holder_references {
            let bound = match reference.target_registry {
                "minecraft:test_environment" => &held_environments,
                "minecraft:world_clock" => &held_clocks,
                _ => &held_timelines,
            };
            if !bound.contains(&reference.target_id) {
                let kind = reference.target_registry.trim_start_matches("minecraft:");
                errors
                    .entry(reference.referring_registry)
                    .or_default()
                    .insert(
                        reference.referring_id,
                        format!(
                            "{}: unknown {kind} {}",
                            reference.referring_file, reference.target_id
                        ),
                    );
            }
        }
        if errors.is_empty() {
            return (definitions, Ok(()));
        }
        let mut report = String::from("failed to load world registry definitions:");
        for (registry, entries) in errors {
            report.push_str(&format!("\n{registry}:"));
            for (id, error) in entries {
                report.push_str(&format!("\n  {id}: {error}"));
            }
        }
        (definitions, Err(report))
    }

    pub(super) fn discard_replaced(
        &self,
        registry: &str,
        entries: &mut Vec<((String, String), Value, bool)>,
    ) {
        if let Some(pack) = self.entries.get(registry) {
            entries.retain(|(key, _, _)| !pack.contains_key(key));
        }
    }

    pub(super) fn includes_prepared_entries(registry: &str) -> bool {
        matches!(
            registry,
            "minecraft:enchantment"
                | "minecraft:enchantment_provider"
                | "minecraft:instrument"
                | "minecraft:jukebox_song"
                | "minecraft:trial_spawner"
                | "minecraft:sulfur_cube_archetype"
                | "minecraft:decorated_pot_pattern"
                | "minecraft:block_transformer"
                | "minecraft:test_environment"
                | "minecraft:test_instance"
                | "minecraft:timeline"
                | "minecraft:wolf_variant"
                | "minecraft:wolf_sound_variant"
                | "minecraft:cat_variant"
                | "minecraft:cat_sound_variant"
                | "minecraft:pig_variant"
                | "minecraft:pig_sound_variant"
                | "minecraft:cow_variant"
                | "minecraft:cow_sound_variant"
                | "minecraft:chicken_variant"
                | "minecraft:chicken_sound_variant"
                | "minecraft:frog_variant"
                | "minecraft:zombie_nautilus_variant"
        )
    }

    /// The kinds whose prepared (vanilla) files are decoded by the kind's own codec like a pack's,
    /// because gameplay reads the world's table and not the resource root's files.
    fn decodes_prepared(registry: &str) -> bool {
        Self::includes_prepared_entries(registry)
            && !matches!(registry, "minecraft:enchantment" | "minecraft:instrument")
    }

    /// The definitions of a [`Definition::Validated`] registry that are in this world, by id and
    /// in file order (`ResourceManager` lists a registry's files by path, then namespace).
    pub fn validated(&self, registry: &str) -> Vec<(String, &Value)> {
        let mut found: Vec<_> = self
            .entries
            .get(registry)
            .into_iter()
            .flatten()
            .filter_map(|((namespace, path), entry)| match entry {
                Some((Definition::Validated(value, _), _)) => {
                    Some((path.as_str(), namespace.as_str(), value))
                }
                _ => None,
            })
            .collect();
        found.sort_by_key(|&(path, namespace, _)| (path, namespace));
        found
            .into_iter()
            .map(|(path, namespace, value)| (format!("{namespace}:{path}"), value))
            .collect()
    }

    pub fn append_encoded(&self, registry: &str, entries: &mut Vec<((String, String), Tag, bool)>) {
        if let Some(pack) = self.entries.get(registry) {
            entries.extend(pack.iter().filter_map(|(key, value)| {
                let (definition, from_pack) = value.as_ref()?;
                let tag = match definition {
                    Definition::Encoded(tag)
                    | Definition::Instrument(_, tag)
                    | Definition::JukeboxSong(_, tag)
                    | Definition::Painting(_, tag)
                    | Definition::BannerPattern(_, tag)
                    | Definition::WolfVariant(_, tag)
                    | Definition::WolfSoundVariant(_, tag)
                    | Definition::CatVariant(_, tag)
                    | Definition::CatSoundVariant(_, tag)
                    | Definition::Timeline(_, tag)
                    | Definition::PigVariant(_, tag)
                    | Definition::PigSoundVariant(_, tag)
                    | Definition::CowVariant(_, tag)
                    | Definition::CowSoundVariant(_, tag)
                    | Definition::ChickenVariant(_, tag)
                    | Definition::ChickenSoundVariant(_, tag)
                    | Definition::FrogVariant(_, tag)
                    | Definition::ZombieNautilusVariant(_, tag) => tag.clone(),
                    Definition::Validated(_, tag) => tag.clone()?,
                    Definition::Enchantment(json) => json_to_nbt(json),
                    Definition::EnchantmentProvider(_) => return None,
                };
                Some((key.clone(), tag, *from_pack))
            }));
        }
    }

    /// The `painting_variant` registry this load holds: `prepared` (the root's
    /// variants) except those a pack winner replaces, even a refused or skipped
    /// one, then the packs' admitted definitions, in registry order (identifier
    /// order: path, then namespace).
    pub fn painting_variants(
        &self,
        prepared: Vec<crate::entities::PaintingVariant>,
    ) -> Vec<crate::entities::PaintingVariant> {
        let pack = self.entries.get("minecraft:painting_variant");
        let mut all: Vec<_> = prepared
            .into_iter()
            .filter(|variant| {
                let (namespace, path) = variant
                    .id
                    .split_once(':')
                    .unwrap_or(("minecraft", &variant.id));
                !pack
                    .is_some_and(|pack| pack.contains_key(&(namespace.to_owned(), path.to_owned())))
            })
            .collect();
        all.extend(
            pack.into_iter()
                .flatten()
                .filter_map(|(_, entry)| match entry {
                    Some((Definition::Painting(variant, _), _)) => Some(variant.clone()),
                    _ => None,
                }),
        );
        all.sort_by(|a, b| {
            let key = |id: &str| {
                let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
                (path.to_owned(), namespace.to_owned())
            };
            key(&a.id).cmp(&key(&b.id))
        });
        all
    }

    /// The admitted `banner_pattern` winners from the world's packs, as the
    /// registry packets carry them.
    pub fn banner_patterns(&self) -> impl Iterator<Item = &crate::banner::Pattern> {
        self.entries
            .get("minecraft:banner_pattern")
            .into_iter()
            .flatten()
            .filter_map(|(_, entry)| match entry {
                Some((Definition::BannerPattern(pattern, _), _)) => Some(pattern),
                _ => None,
            })
    }

    /// Every admitted `minecraft:timeline` definition of this world, keyed by canonical id.
    pub fn timelines(&self) -> BTreeMap<String, Arc<Timeline>> {
        self.entries
            .get("minecraft:timeline")
            .into_iter()
            .flatten()
            .filter_map(|((namespace, path), entry)| match entry {
                Some((Definition::Timeline(timeline, _), _)) => {
                    Some((format!("{namespace}:{path}"), Arc::clone(timeline)))
                }
                _ => None,
            })
            .collect()
    }

    /// Prepared-resource `test_environment` and `test_instance` entries for a game before a
    /// world opens (`WorldRest`), decoded through the same codecs as world open.
    pub fn load_prepared_game_tests(resources: &Path, defaults: &ItemDefaults) -> Self {
        let mut definitions = Self::default();
        let registries = crate::command::Registries::default();
        let sound_exists = |_: &str| true;
        let mut decoder = Decoder {
            registries: &registries,
            defaults,
            sound_exists: &sound_exists,
            dialog_references: Vec::new(),
            holder_references: Vec::new(),
        };
        let root = resources.join("data/minecraft");
        for (registry, decode) in [
            (
                "minecraft:test_environment",
                decode_test_environment
                    as fn(&mut Decoder<'_>, &str, Value) -> Result<Definition, String>,
            ),
            ("minecraft:test_instance", decode_test_instance),
        ] {
            let entries = definitions.entries.entry(registry).or_default();
            for (key, value, from_pack) in synchronized_entries(&root, resources, registry) {
                let id = format!("{}:{}", key.0, key.1);
                if let Ok(definition) = decode(&mut decoder, &id, value) {
                    entries.insert(key, Some((definition, from_pack)));
                }
            }
        }
        definitions
    }

    /// Build the frozen world variant tables from the staged definitions,
    /// in the same `file_order` that `SyncData` uses for network ids.
    pub fn variant_tables(&self) -> crate::mobs::variants::VariantTables {
        use crate::mobs::variants::{VariantTable, VariantTables};
        fn collect<T: Clone>(
            entries: &BTreeMap<
                &'static str,
                BTreeMap<(String, String), Option<(Definition, bool)>>,
            >,
            registry: &'static str,
            extract: impl Fn(&Definition) -> Option<&T>,
        ) -> VariantTable<T> {
            VariantTable::from_entries(
                entries
                    .get(registry)
                    .into_iter()
                    .flatten()
                    .filter_map(|(key, entry)| {
                        let (def, _) = entry.as_ref()?;
                        Some((key.clone(), extract(def)?.clone()))
                    })
                    .collect(),
            )
        }
        VariantTables {
            wolf: collect(&self.entries, "minecraft:wolf_variant", |d| match d {
                Definition::WolfVariant(v, _) => Some(v),
                _ => None,
            }),
            wolf_sound: collect(&self.entries, "minecraft:wolf_sound_variant", |d| match d {
                Definition::WolfSoundVariant(v, _) => Some(v),
                _ => None,
            }),
            cat: collect(&self.entries, "minecraft:cat_variant", |d| match d {
                Definition::CatVariant(v, _) => Some(v),
                _ => None,
            }),
            cat_sound: collect(&self.entries, "minecraft:cat_sound_variant", |d| match d {
                Definition::CatSoundVariant(v, _) => Some(v),
                _ => None,
            }),
            pig: collect(&self.entries, "minecraft:pig_variant", |d| match d {
                Definition::PigVariant(v, _) => Some(v),
                _ => None,
            }),
            pig_sound: collect(&self.entries, "minecraft:pig_sound_variant", |d| match d {
                Definition::PigSoundVariant(v, _) => Some(v),
                _ => None,
            }),
            cow: collect(&self.entries, "minecraft:cow_variant", |d| match d {
                Definition::CowVariant(v, _) => Some(v),
                _ => None,
            }),
            cow_sound: collect(&self.entries, "minecraft:cow_sound_variant", |d| match d {
                Definition::CowSoundVariant(v, _) => Some(v),
                _ => None,
            }),
            chicken: collect(&self.entries, "minecraft:chicken_variant", |d| match d {
                Definition::ChickenVariant(v, _) => Some(v),
                _ => None,
            }),
            chicken_sound: collect(
                &self.entries,
                "minecraft:chicken_sound_variant",
                |d| match d {
                    Definition::ChickenSoundVariant(v, _) => Some(v),
                    _ => None,
                },
            ),
            frog: collect(&self.entries, "minecraft:frog_variant", |d| match d {
                Definition::FrogVariant(v, _) => Some(v),
                _ => None,
            }),
            zombie_nautilus: collect(
                &self.entries,
                "minecraft:zombie_nautilus_variant",
                |d| match d {
                    Definition::ZombieNautilusVariant(v, _) => Some(v),
                    _ => None,
                },
            ),
        }
    }

    /// Use the same selected definitions as the synchronized registry packets.
    pub fn install_items(
        &self,
        items: &mut Items,
        resources: &Path,
        overlay: &PackOverlay<'_>,
        defaults: &ItemDefaults,
    ) -> sunrise_items::Result<()> {
        let enchantments: Vec<_> = self
            .entries
            .get("minecraft:enchantment")
            .into_iter()
            .flatten()
            .filter_map(|((namespace, path), entry)| match entry {
                Some((Definition::Enchantment(value), _)) => {
                    Some((format!("{namespace}:{path}"), value))
                }
                _ => None,
            })
            .collect();
        let mut enchantments = Enchantments::load_selected(
            resources,
            overlay,
            defaults,
            enchantments.iter().map(|(id, value)| (id.as_str(), *value)),
        )?;
        let providers = self
            .entries
            .get("minecraft:enchantment_provider")
            .into_iter()
            .flatten()
            .filter_map(|((namespace, path), entry)| match entry {
                Some((Definition::EnchantmentProvider(provider), _)) => {
                    Some((format!("{namespace}:{path}"), provider.clone()))
                }
                _ => None,
            })
            .collect();
        enchantments.install_providers(providers);
        let instruments = self
            .entries
            .get("minecraft:instrument")
            .into_iter()
            .flatten()
            .filter_map(|((namespace, path), entry)| match entry {
                Some((Definition::Instrument(instrument, _), _)) => {
                    Some((format!("{namespace}:{path}"), instrument.clone()))
                }
                _ => None,
            })
            .collect();
        items.install_registries(enchantments, instruments);
        items.songs = self
            .entries
            .get("minecraft:jukebox_song")
            .into_iter()
            .flatten()
            .filter_map(|((namespace, path), entry)| match entry {
                Some((Definition::JukeboxSong(song, _), _)) => {
                    Some((format!("{namespace}:{path}"), song.clone()))
                }
                _ => None,
            })
            .collect();
        Ok(())
    }
}

/// Bind the dialog tag stack both at world admission and when producing tag
/// packets (including reloads over the frozen registry). TagLoader's writable
/// lookup reserves a holder for each missing required value; these names are
/// reported by the registry freeze, even if no dialog uses the tag.
pub(super) fn bind_dialog_tags(
    root: &Path,
    overlay: &PackOverlay<'_>,
    held: impl Fn(&str) -> bool,
) -> (BTreeMap<String, Vec<String>>, BTreeSet<String>) {
    let mut unbound = BTreeSet::new();
    let bound = bind_registry_tags(root, overlay, "dialog", |id, required| {
        if held(id) {
            Some(id.to_owned())
        } else {
            if required {
                unbound.insert(id.to_owned());
            }
            None
        }
    });
    (bound, unbound)
}

/// Build the stacked tag files against the registry that will publish them.
/// Only a successfully built tag can satisfy a nested reference.
pub(super) fn bind_registry_tags<T: Clone + Ord>(
    root: &Path,
    overlay: &PackOverlay<'_>,
    kind: &str,
    mut lookup: impl FnMut(&str, bool) -> Option<T>,
) -> BTreeMap<String, Vec<T>> {
    use sunrise_world::worldgen::tags::TagDefinitions;

    // ResourceManager lists whole stacks in Identifier file order (path
    // first, namespace second). Pack versions then apply low to high.
    let prepared = sunrise_world::json_folder::read(&root.join("tags").join(kind)).ok();
    let mut files = BTreeMap::<_, Vec<&[u8]>>::new();
    for file in prepared.iter().flat_map(|folder| &folder.files) {
        if let Ok(bytes) = file.bytes() {
            files
                .entry((format!("{}.json", file.name), "minecraft".to_owned()))
                .or_default()
                .push(bytes);
        }
    }
    for (id, versions) in overlay.tag_kind(kind) {
        let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", &id));
        files
            .entry((format!("{path}.json"), namespace.to_owned()))
            .or_default()
            .extend(versions.iter().map(Vec::as_slice));
    }
    let mut definitions = TagDefinitions::default();
    for ((path, namespace), stack) in files {
        let id = format!("{namespace}:{}", path.strip_suffix(".json").unwrap());
        for bytes in stack {
            if let Ok(json) = std::str::from_utf8(bytes) {
                let _ = definitions.apply_json(&id, "pack", json);
            }
        }
    }
    let built = definitions.bind(&mut lookup);
    for (id, missing) in built.missing {
        let entries: Vec<_> = missing.iter().map(|e| e.entry.display()).collect();
        log::warn!(
            "Sunrise: {kind} tag {id} has unresolved required entries: {}",
            entries.join(", ")
        );
    }
    built.tags
}

fn check_network_nbt(tag: &Tag) -> Result<(), String> {
    sunrise_protocol::buf::check_network_nbt(tag).map_err(|error| format!("network {error}"))
}

/// What a kind's decode function reads besides the file: the text registries and sound
/// holders its codec checks, and the dialogs a winning dialog names, which the registry must hold.
struct Decoder<'a> {
    registries: &'a crate::command::Registries,
    defaults: &'a ItemDefaults,
    sound_exists: &'a dyn Fn(&str) -> bool,
    dialog_references: Vec<Vec<Reference>>,
    /// The registry holders `test_environment` and `test_instance` entries name, which their
    /// registries (and `world_clock` and `timeline`) must hold.
    holder_references: Vec<HolderReference>,
}

/// A `RegistryCodecs.holder` reference decoded from a winning entry.
struct HolderReference {
    referring_registry: &'static str,
    referring_id: String,
    referring_file: String,
    target_registry: &'static str,
    target_id: String,
}

impl Decoder<'_> {
    fn text(&self, field: &str, value: &Value) -> Result<Value, String> {
        crate::text_grammar::build(value, self.registries, false)
            .ok_or_else(|| format!("{field}: invalid JSON text component"))
    }
}

fn string(value: &str) -> Tag {
    Tag::String(value.into())
}

fn asset_id(asset: &str) -> Tag {
    // The render readers store the default namespace as a path only.
    if asset.contains(':') {
        string(asset)
    } else {
        string(&format!("minecraft:{asset}"))
    }
}

fn encoded(tag: Tag) -> Result<Definition, String> {
    check_network_nbt(&tag)?;
    Ok(Definition::Encoded(tag))
}

fn decode_dimension_type(
    _: &mut Decoder<'_>,
    id: &str,
    value: Value,
) -> Result<Definition, String> {
    encoded(sunrise_world::dimension_type::DimensionFacts::decode_pack_definition(id, &value)?)
}

/// `ChatType.DIRECT_CODEC`: the `chat` and `narration` decorations.
fn decode_chat_type(
    decoder: &mut Decoder<'_>,
    _: &str,
    value: Value,
) -> Result<Definition, String> {
    let fields = value.as_object().ok_or("not a map")?;
    let mut chat_type = Compound::new();
    for field in ["chat", "narration"] {
        let decoration = fields.get(field).ok_or(format!("{field} is required"))?;
        chat_type.insert(
            field.into(),
            decode_chat_decoration(decoder, field, decoration)?,
        );
    }
    encoded(Tag::Compound(chat_type))
}

/// `ChatTypeDecoration.CODEC`: `translation_key`, the `parameters` among sender, target and
/// content, and an optional `style`.
fn decode_chat_decoration(
    decoder: &Decoder<'_>,
    field: &str,
    value: &Value,
) -> Result<Tag, String> {
    const STYLE_FIELDS: [&str; 11] = [
        "color",
        "shadow_color",
        "bold",
        "italic",
        "underlined",
        "strikethrough",
        "obfuscated",
        "click_event",
        "hover_event",
        "insertion",
        "font",
    ];
    let fields = value.as_object().ok_or(format!("{field}: not a map"))?;
    let key = fields
        .get("translation_key")
        .and_then(Value::as_str)
        .ok_or(format!("{field}.translation_key: a string is required"))?;
    let parameters = fields
        .get("parameters")
        .and_then(Value::as_array)
        .ok_or(format!("{field}.parameters: a list is required"))?
        .iter()
        .map(|parameter| match parameter.as_str() {
            Some(name @ ("sender" | "target" | "content")) => Ok(string(name)),
            _ => Err(format!("{field}.parameters: unknown parameter {parameter}")),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut decoration = Compound::from([
        ("translation_key".into(), string(key)),
        ("parameters".into(), Tag::List(parameters)),
    ]);
    if let Some(style) = fields.get("style") {
        let style = style
            .as_object()
            .ok_or(format!("{field}.style: not a map"))?;
        // Style.Serializer.CODEC is the style fields of a component, read through the same grammar.
        let mut component: serde_json::Map<String, Value> = style
            .iter()
            .filter(|(name, _)| STYLE_FIELDS.contains(&name.as_str()))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        component.insert("text".into(), Value::String(String::new()));
        let mut built = decoder.text(&format!("{field}.style"), &Value::Object(component))?;
        if let Some(built) = built.as_object_mut() {
            built.remove("text");
        }
        decoration.insert("style".into(), json_to_nbt(&built));
    }
    Ok(Tag::Compound(decoration))
}

/// `Identifier.CODEC` of a record field, as the wire's string.
fn identifier(fields: &serde_json::Map<String, Value>, field: &str) -> Result<Tag, String> {
    let id = fields
        .get(field)
        .and_then(Value::as_str)
        .ok_or(format!("{field}: an identifier is required"))?;
    Identifier::parse(id)
        .map(|id| string(&id.to_string()))
        .map_err(|error| format!("{field}: {error}"))
}

/// `ComponentSerialization.CODEC` of a record's required `description` field.
fn description(
    decoder: &Decoder<'_>,
    fields: &serde_json::Map<String, Value>,
) -> Result<Tag, String> {
    let text = fields.get("description").ok_or("description is required")?;
    Ok(json_to_nbt(&decoder.text("description", text)?))
}

/// `TrimPattern.DIRECT_CODEC`: `asset_id`, `description` and `decal` (default false).
fn decode_trim_pattern(
    decoder: &mut Decoder<'_>,
    _: &str,
    value: Value,
) -> Result<Definition, String> {
    let fields = value.as_object().ok_or("not a map")?;
    let decal = match fields.get("decal") {
        None => false,
        Some(decal) => decal.as_bool().ok_or("decal: a boolean is required")?,
    };
    encoded(Tag::compound([
        ("asset_id", identifier(fields, "asset_id")?),
        ("description", description(decoder, fields)?),
        ("decal", Tag::Byte(i8::from(decal))),
    ]))
}

/// `TrimMaterial.DIRECT_CODEC`: `palette_id` and `description`.
fn decode_trim_material(
    decoder: &mut Decoder<'_>,
    _: &str,
    value: Value,
) -> Result<Definition, String> {
    let fields = value.as_object().ok_or("not a map")?;
    encoded(Tag::compound([
        ("palette_id", identifier(fields, "palette_id")?),
        ("description", description(decoder, fields)?),
    ]))
}

fn decode_painting_variant(
    decoder: &mut Decoder<'_>,
    id: &str,
    value: Value,
) -> Result<Definition, String> {
    let variant = crate::entities::PaintingVariant::decode(id, &value)?;
    let painting = &variant;
    let mut fields = Compound::from([
        ("asset_id".into(), asset_id(&painting.asset)),
        ("width".into(), Tag::Int(painting.width)),
        ("height".into(), Tag::Int(painting.height)),
    ]);
    for (field, text) in [
        ("title", painting.title.clone()),
        ("author", painting.author.clone()),
    ] {
        if let Some(text) = text.filter(|value| !value.is_null()) {
            fields.insert(field.into(), json_to_nbt(&decoder.text(field, &text)?));
        }
    }
    let tag = Tag::Compound(fields);
    check_network_nbt(&tag)?;
    Ok(Definition::Painting(variant, tag))
}

fn decode_banner_pattern(
    _: &mut Decoder<'_>,
    id: &str,
    value: Value,
) -> Result<Definition, String> {
    let pattern = crate::banner::Pattern::decode(id, &value)?;
    let tag = Tag::compound([
        ("asset_id", asset_id(&pattern.asset)),
        ("translation_key", string(&pattern.translation_key)),
    ]);
    check_network_nbt(&tag)?;
    Ok(Definition::BannerPattern(pattern, tag))
}

fn decode_enchantment(
    decoder: &mut Decoder<'_>,
    _: &str,
    value: Value,
) -> Result<Definition, String> {
    let mut value = sunrise_items::enchant::decode_pack_definition(value)?;
    value["description"] = decoder.text("description", &value["description"])?;
    Ok(Definition::Enchantment(value))
}

fn decode_enchantment_provider(
    _: &mut Decoder<'_>,
    _: &str,
    value: Value,
) -> Result<Definition, String> {
    EnchantmentProvider::decode(&value).map(Definition::EnchantmentProvider)
}

fn decode_instrument(
    decoder: &mut Decoder<'_>,
    _: &str,
    value: Value,
) -> Result<Definition, String> {
    let instrument = Instrument::decode(&value)?;
    let float = |v: f32| Tag::Float(v.to_bits());
    let sound = match instrument.inline_sound_range {
        None if !(decoder.sound_exists)(&instrument.sound) => {
            return Err(format!("sound_event: unknown holder {}", instrument.sound));
        }
        None => string(&instrument.sound),
        Some(fixed) => {
            let mut fields = Compound::from([("sound_id".into(), string(&instrument.sound))]);
            if let Some(range) = fixed {
                fields.insert("range".into(), float(range));
            }
            Tag::Compound(fields)
        }
    };
    let mut fields = Compound::from([
        ("sound_event".into(), sound),
        ("use_duration".into(), float(instrument.use_duration)),
        ("range".into(), float(instrument.range)),
        (
            "description".into(),
            json_to_nbt(&decoder.text("description", &value["description"])?),
        ),
    ]);
    if instrument.durability_damage != 0 {
        fields.insert(
            "durability_damage".into(),
            Tag::Int(instrument.durability_damage),
        );
    }
    let tag = Tag::Compound(fields);
    check_network_nbt(&tag)?;
    Ok(Definition::Instrument(instrument, tag))
}

/// `JukeboxSong.DIRECT_CODEC`: its description is a text component, and the wire keeps the
/// file's own fields.
fn decode_jukebox_song(
    decoder: &mut Decoder<'_>,
    _: &str,
    value: Value,
) -> Result<Definition, String> {
    let song = Song::decode(&value)?;
    decoder.text("description", &song.description)?;
    let wire = json_to_nbt(&value);
    Ok(Definition::JukeboxSong(song, wire))
}

/// `TestEnvironmentDefinition.DIRECT_CODEC`.
fn decode_test_environment(
    decoder: &mut Decoder<'_>,
    id: &str,
    value: Value,
) -> Result<Definition, String> {
    let file = registry_file_path("test_environment", id);
    let tag =
        decode_test_environment_value(decoder, "minecraft:test_environment", id, &file, &value)?;
    check_network_nbt(&tag)?;
    Ok(Definition::Validated(value, Some(tag)))
}

fn registry_file_path(kind: &str, id: &str) -> String {
    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
    format!("data/{ns}/{kind}/{path}.json")
}

fn parse_identifier_field(field: &str, value: &Value) -> Result<Identifier, String> {
    let raw = value
        .as_str()
        .ok_or_else(|| format!("{field}: a string is required"))?;
    Identifier::parse(raw).map_err(|error| format!("{field}: {error}"))
}

fn parse_int_range(field: &str, value: &Value, min: i32, max: i32) -> Result<i32, String> {
    let number = value
        .as_i64()
        .and_then(|n| i32::try_from(n).ok())
        .ok_or_else(|| format!("{field}: an integer is required"))?;
    if !(min..=max).contains(&number) {
        return Err(format!("{field}: {number} outside {min}..={max}"));
    }
    Ok(number)
}

/// `TestEnvironmentDefinition.CODEC`: either a registry id string or an inline definition.
fn decode_test_environment_holder(
    decoder: &mut Decoder<'_>,
    referring_registry: &'static str,
    referring_id: &str,
    referring_file: &str,
    field: &str,
    value: &Value,
) -> Result<Tag, String> {
    if let Some(raw) = value.as_str() {
        let env_id = Identifier::parse(raw).map_err(|error| format!("{field}: {error}"))?;
        let canonical = env_id.to_string();
        decoder.holder_references.push(HolderReference {
            referring_registry,
            referring_id: referring_id.to_owned(),
            referring_file: referring_file.to_owned(),
            target_registry: "minecraft:test_environment",
            target_id: canonical.clone(),
        });
        return Ok(string(&canonical));
    }
    if value.is_object() {
        return decode_test_environment_value(
            decoder,
            referring_registry,
            referring_id,
            referring_file,
            value,
        )
        .map_err(|error| format!("{field}: {error}"));
    }
    Err(format!("{field}: a string or object is required"))
}

fn decode_test_environment_value(
    decoder: &mut Decoder<'_>,
    referring_registry: &'static str,
    referring_id: &str,
    referring_file: &str,
    value: &Value,
) -> Result<Tag, String> {
    let fields = value.as_object().ok_or("not a map")?;
    let kind_value = fields.get("type").ok_or("type is required")?;
    let kind = parse_identifier_field("type", kind_value)?;
    if kind.namespace() != "minecraft" {
        return Err(format!("type: unknown test_environment type {kind}"));
    }
    let mut tag = Compound::from([("type".into(), string(&kind.to_string()))]);
    match kind.path() {
        "all_of" => {
            let list = fields
                .get("definitions")
                .and_then(Value::as_array)
                .ok_or("definitions: a list is required")?;
            let encoded = list
                .iter()
                .enumerate()
                .map(|(index, entry)| {
                    decode_test_environment_holder(
                        decoder,
                        referring_registry,
                        referring_id,
                        referring_file,
                        &format!("definitions[{index}]"),
                        entry,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            tag.insert("definitions".into(), Tag::List(encoded));
        }
        "clock_time" => {
            let clock =
                parse_identifier_field("clock", fields.get("clock").ok_or("clock is required")?)?;
            let time = parse_int_range(
                "time",
                fields.get("time").ok_or("time is required")?,
                0,
                i32::MAX,
            )?;
            decoder.holder_references.push(HolderReference {
                referring_registry,
                referring_id: referring_id.to_owned(),
                referring_file: referring_file.to_owned(),
                target_registry: "minecraft:world_clock",
                target_id: clock.to_string(),
            });
            tag.insert("clock".into(), string(&clock.to_string()));
            tag.insert("time".into(), Tag::Int(time));
        }
        "difficulty" => {
            let difficulty = fields
                .get("difficulty")
                .and_then(Value::as_str)
                .ok_or("difficulty: a string is required")?;
            if !matches!(difficulty, "peaceful" | "easy" | "normal" | "hard") {
                return Err(format!("difficulty: unknown difficulty {difficulty}"));
            }
            tag.insert("difficulty".into(), string(difficulty));
        }
        "function" => {
            for field in ["setup", "teardown"] {
                if let Some(raw) = fields.get(field) {
                    let fn_id = parse_identifier_field(field, raw)?;
                    tag.insert(field.into(), string(&fn_id.to_string()));
                }
            }
        }
        "game_rules" => {
            let rules_map = fields
                .get("rules")
                .and_then(Value::as_object)
                .ok_or("rules: a map is required")?;
            let registry = sunrise_game_data::gamerules::rule_registry();
            let mut rules_tag = Compound::new();
            for (name, val) in rules_map {
                let rule_id =
                    Identifier::parse(name).map_err(|error| format!("rules.{name}: {error}"))?;
                let canonical = rule_id.to_string();
                let rule = registry
                    .rule(&canonical)
                    .ok_or_else(|| format!("rules.{name}: unknown game rule"))?;
                let encoded = match rule.default {
                    sunrise_game_data::gamerules::Value::Bool(_) => {
                        let b = val
                            .as_bool()
                            .ok_or_else(|| format!("rules.{name}: a boolean is required"))?;
                        Tag::Byte(i8::from(b))
                    }
                    sunrise_game_data::gamerules::Value::Int(_) => {
                        let n = parse_int_range(&format!("rules.{name}"), val, rule.min, rule.max)?;
                        Tag::Int(n)
                    }
                };
                rules_tag.insert(canonical.as_str().into(), encoded);
            }
            tag.insert("rules".into(), Tag::Compound(rules_tag));
        }
        "timeline_attributes" => {
            let list = fields
                .get("timelines")
                .and_then(Value::as_array)
                .ok_or("timelines: a list is required")?;
            let timelines = list
                .iter()
                .enumerate()
                .map(|(index, entry)| {
                    parse_identifier_field(&format!("timelines[{index}]"), entry).map(|id| {
                        decoder.holder_references.push(HolderReference {
                            referring_registry,
                            referring_id: referring_id.to_owned(),
                            referring_file: referring_file.to_owned(),
                            target_registry: "minecraft:timeline",
                            target_id: id.to_string(),
                        });
                        string(&id.to_string())
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            tag.insert("timelines".into(), Tag::List(timelines));
        }
        "weather" => {
            let weather = fields
                .get("weather")
                .and_then(Value::as_str)
                .ok_or("weather: a string is required")?;
            if !matches!(weather, "clear" | "rain" | "thunder") {
                return Err(format!("weather: unknown weather {weather}"));
            }
            tag.insert("weather".into(), string(weather));
        }
        _ => return Err(format!("type: unknown test_environment type {kind}")),
    }
    Ok(Tag::Compound(tag))
}

/// `GameTestInstance.DIRECT_CODEC`: `BlockBasedTestInstance.CODEC` and
/// `FunctionGameTestInstance.CODEC` over `TestData.CODEC`.
fn decode_test_instance(
    decoder: &mut Decoder<'_>,
    id: &str,
    value: Value,
) -> Result<Definition, String> {
    let fields = value.as_object().ok_or("not a map")?;
    let file = registry_file_path("test_instance", id);
    let kind = parse_identifier_field("type", fields.get("type").ok_or("type is required")?)?;
    if kind.namespace() != "minecraft" || !matches!(kind.path(), "block_based" | "function") {
        return Err(format!("type: unknown test_instance type {kind}"));
    }
    let mut tag = Compound::from([("type".into(), string(&kind.to_string()))]);
    if kind.path() == "function" {
        let function = parse_identifier_field(
            "function",
            fields.get("function").ok_or("function is required")?,
        )?;
        tag.insert("function".into(), string(&function.to_string()));
    }
    let environment = decode_test_environment_holder(
        decoder,
        "minecraft:test_instance",
        id,
        &file,
        "environment",
        fields.get("environment").ok_or("environment is required")?,
    )?;
    tag.insert("environment".into(), environment);
    let structure = parse_identifier_field(
        "structure",
        fields.get("structure").ok_or("structure is required")?,
    )?;
    tag.insert("structure".into(), string(&structure.to_string()));
    let max_ticks = parse_int_range(
        "max_ticks",
        fields.get("max_ticks").ok_or("max_ticks is required")?,
        1,
        i32::MAX,
    )?;
    tag.insert("max_ticks".into(), Tag::Int(max_ticks));
    if let Some(raw) = fields.get("dimension") {
        let dimension = parse_identifier_field("dimension", raw)?;
        tag.insert("dimension".into(), string(&dimension.to_string()));
    }
    if let Some(raw) = fields.get("setup_ticks") {
        let setup_ticks = parse_int_range("setup_ticks", raw, 0, i32::MAX)?;
        tag.insert("setup_ticks".into(), Tag::Int(setup_ticks));
    }
    for field in ["required", "manual_only", "sky_access"] {
        if let Some(raw) = fields.get(field) {
            let b = raw
                .as_bool()
                .ok_or_else(|| format!("{field}: a boolean is required"))?;
            tag.insert(field.into(), Tag::Byte(i8::from(b)));
        }
    }
    if let Some(raw) = fields.get("rotation") {
        let rotation = raw.as_str().ok_or("rotation: a string is required")?;
        if sunrise_world::worldgen::structure::Rotation::from_name(rotation).is_none() {
            return Err(format!("rotation: unknown rotation {rotation}"));
        }
        tag.insert("rotation".into(), string(rotation));
    }
    for field in ["max_attempts", "required_successes"] {
        if let Some(raw) = fields.get(field) {
            let n = parse_int_range(field, raw, 1, i32::MAX)?;
            tag.insert(field.into(), Tag::Int(n));
        }
    }
    if let Some(raw) = fields.get("padding") {
        let padding = parse_int_range("padding", raw, 0, 128)?;
        tag.insert("padding".into(), Tag::Int(padding));
    }
    let tag = Tag::Compound(tag);
    check_network_nbt(&tag)?;
    Ok(Definition::Validated(value, Some(tag)))
}

/// A winning dialog definition: `Dialog.DIRECT_CODEC` over the file, and the
/// dialogs it names, which the registry must hold (`decode_pack`).
fn decode_dialog(decoder: &mut Decoder<'_>, _: &str, value: Value) -> Result<Definition, String> {
    // Transient components are excluded from persisted world definitions.
    sunrise_world::item_defaults::validate_dialog(&value)?;
    let (tag, references) = crate::dialog::Dialog::decode_pack(&value, decoder.registries, &|id| {
        decoder.defaults.entries().contains_key(id)
    })
    .map_err(|field| format!("invalid dialog field {field}"))?;
    check_network_nbt(&tag)?;
    decoder.dialog_references.push(references);
    Ok(Definition::Encoded(tag))
}

/// `SoundEvent.CODEC`: either a registered `minecraft:sound_event` holder id or
/// an inline `{ sound_id, range? }` object.
fn decode_sound_event_holder(
    decoder: &Decoder<'_>,
    field: &str,
    value: &Value,
) -> Result<(String, Tag), String> {
    match value {
        Value::String(raw) => {
            let id = Identifier::parse(raw)
                .map_err(|e| format!("{field}: invalid sound identifier {raw:?}: {e}"))?;
            let canonical = id.as_str();
            if !(decoder.sound_exists)(canonical) {
                return Err(format!("{field}: unknown holder {canonical}"));
            }
            Ok((canonical.to_owned(), string(canonical)))
        }
        Value::Object(map) => {
            let raw_id = map
                .get("sound_id")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{field}.sound_id: a string is required"))?;
            let id = Identifier::parse(raw_id)
                .map_err(|e| format!("{field}.sound_id: invalid identifier {raw_id:?}: {e}"))?;
            let canonical = id.as_str();
            let mut fields = Compound::from([("sound_id".into(), string(canonical))]);
            if let Some(raw_range) = map.get("range") {
                let range = raw_range
                    .as_f64()
                    .map(|n| n as f32)
                    .filter(|n| n.is_finite())
                    .ok_or_else(|| format!("{field}.range: a finite number is required"))?;
                fields.insert("range".into(), Tag::Float(range.to_bits()));
            }
            Ok((canonical.to_owned(), Tag::Compound(fields)))
        }
        _ => Err(format!("{field}: a sound event id or object is required")),
    }
}

/// `WolfVariant.DIRECT_CODEC` on disk, `WolfVariant.NETWORK_CODEC` on the wire.
fn decode_wolf_variant(_: &mut Decoder<'_>, _: &str, value: Value) -> Result<Definition, String> {
    let variant = crate::mobs::variants::WolfVariant::decode(&value)?;
    let wire = variant.to_network_nbt();
    check_network_nbt(&wire)?;
    Ok(Definition::WolfVariant(variant, wire))
}

/// `WolfSoundVariant.DIRECT_CODEC` / `NETWORK_CODEC`.
fn decode_wolf_sound_variant(
    decoder: &mut Decoder<'_>,
    _: &str,
    value: Value,
) -> Result<Definition, String> {
    let (variant, wire) =
        crate::mobs::variants::WolfSoundVariant::decode_with(&value, |field, raw| {
            decode_sound_event_holder(decoder, field, raw)
        })?;
    check_network_nbt(&wire)?;
    Ok(Definition::WolfSoundVariant(variant, wire))
}

/// `CatVariant.DIRECT_CODEC` on disk, `CatVariant.NETWORK_CODEC` on the wire.
fn decode_cat_variant(_: &mut Decoder<'_>, _: &str, value: Value) -> Result<Definition, String> {
    let variant = crate::mobs::variants::CatVariant::decode(&value)?;
    let wire = variant.to_network_nbt();
    check_network_nbt(&wire)?;
    Ok(Definition::CatVariant(variant, wire))
}

/// `CatSoundVariant.DIRECT_CODEC` / `NETWORK_CODEC`.
fn decode_cat_sound_variant(
    decoder: &mut Decoder<'_>,
    _: &str,
    value: Value,
) -> Result<Definition, String> {
    let (variant, wire) =
        crate::mobs::variants::CatSoundVariant::decode_with(&value, |field, raw| {
            decode_sound_event_holder(decoder, field, raw)
        })?;
    check_network_nbt(&wire)?;
    Ok(Definition::CatSoundVariant(variant, wire))
}

/// `PigVariant.DIRECT_CODEC` on disk, `PigVariant.NETWORK_CODEC` on the wire.
fn decode_pig_variant(_: &mut Decoder<'_>, _: &str, value: Value) -> Result<Definition, String> {
    let variant = crate::mobs::variants::PigVariant::decode(&value)?;
    let wire = variant.to_network_nbt();
    check_network_nbt(&wire)?;
    Ok(Definition::PigVariant(variant, wire))
}

/// `PigSoundVariant.DIRECT_CODEC` / `NETWORK_CODEC`.
fn decode_pig_sound_variant(
    decoder: &mut Decoder<'_>,
    _: &str,
    value: Value,
) -> Result<Definition, String> {
    let (variant, wire) =
        crate::mobs::variants::PigSoundVariant::decode_with(&value, |field, raw| {
            decode_sound_event_holder(decoder, field, raw)
        })?;
    check_network_nbt(&wire)?;
    Ok(Definition::PigSoundVariant(variant, wire))
}

/// `CowVariant.DIRECT_CODEC` on disk, `CowVariant.NETWORK_CODEC` on the wire.
fn decode_cow_variant(_: &mut Decoder<'_>, _: &str, value: Value) -> Result<Definition, String> {
    let variant = crate::mobs::variants::CowVariant::decode(&value)?;
    let wire = variant.to_network_nbt();
    check_network_nbt(&wire)?;
    Ok(Definition::CowVariant(variant, wire))
}

/// `CowSoundVariant.DIRECT_CODEC` / `NETWORK_CODEC`.
fn decode_cow_sound_variant(
    decoder: &mut Decoder<'_>,
    _: &str,
    value: Value,
) -> Result<Definition, String> {
    let (variant, wire) =
        crate::mobs::variants::CowSoundVariant::decode_with(&value, |field, raw| {
            decode_sound_event_holder(decoder, field, raw)
        })?;
    check_network_nbt(&wire)?;
    Ok(Definition::CowSoundVariant(variant, wire))
}

/// `ChickenVariant.DIRECT_CODEC` on disk, `ChickenVariant.NETWORK_CODEC` on the wire.
fn decode_chicken_variant(
    _: &mut Decoder<'_>,
    _: &str,
    value: Value,
) -> Result<Definition, String> {
    let variant = crate::mobs::variants::ChickenVariant::decode(&value)?;
    let wire = variant.to_network_nbt();
    check_network_nbt(&wire)?;
    Ok(Definition::ChickenVariant(variant, wire))
}

/// `ChickenSoundVariant.DIRECT_CODEC` / `NETWORK_CODEC`.
fn decode_chicken_sound_variant(
    decoder: &mut Decoder<'_>,
    _: &str,
    value: Value,
) -> Result<Definition, String> {
    let (variant, wire) =
        crate::mobs::variants::ChickenSoundVariant::decode_with(&value, |field, raw| {
            decode_sound_event_holder(decoder, field, raw)
        })?;
    check_network_nbt(&wire)?;
    Ok(Definition::ChickenSoundVariant(variant, wire))
}

/// `FrogVariant.DIRECT_CODEC` on disk, `FrogVariant.NETWORK_CODEC` on the wire.
fn decode_frog_variant(_: &mut Decoder<'_>, _: &str, value: Value) -> Result<Definition, String> {
    let variant = crate::mobs::variants::FrogVariant::decode(&value)?;
    let wire = variant.to_network_nbt();
    check_network_nbt(&wire)?;
    Ok(Definition::FrogVariant(variant, wire))
}

/// `ZombieNautilusVariant.DIRECT_CODEC` on disk, `ZombieNautilusVariant.NETWORK_CODEC` on the wire.
fn decode_zombie_nautilus_variant(
    _: &mut Decoder<'_>,
    _: &str,
    value: Value,
) -> Result<Definition, String> {
    let variant = crate::mobs::variants::ZombieNautilusVariant::decode(&value)?;
    let wire = variant.to_network_nbt();
    check_network_nbt(&wire)?;
    Ok(Definition::ZombieNautilusVariant(variant, wire))
}

fn is_variant_kind(kind: &str) -> bool {
    matches!(
        kind,
        "wolf_variant"
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
}

// WorldClock.DIRECT_CODEC is a unit map codec: extra keys are permitted.
/// `DecoratedPotPattern.CODEC`: the `asset_id` identifier.
fn decode_decorated_pot_pattern(
    _: &mut Decoder<'_>,
    _: &str,
    value: Value,
) -> Result<Definition, String> {
    let asset = value
        .as_object()
        .ok_or("not a map")?
        .get("asset_id")
        .ok_or("asset_id is required")?
        .as_str()
        .ok_or("asset_id: not an identifier")?;
    Identifier::parse(asset).map_err(|error| format!("asset_id: {error}"))?;
    validated(value, true)
}

/// `BlockTransformer.DIRECT_CODEC`; the blocks and states it names are bound when the world's
/// tables are built.
fn decode_block_transformer(
    decoder: &mut Decoder<'_>,
    _: &str,
    value: Value,
) -> Result<Definition, String> {
    crate::block_transform::Transformers::validate(&value, decoder.sound_exists)?;
    validated(value, true)
}

/// `BlockStateProvider.DIRECT_CODEC`: a map; the provider's own fields are read by the world
/// generator's state-provider catalog, which admits the same pack files.
fn decode_block_state_provider(
    _: &mut Decoder<'_>,
    _: &str,
    value: Value,
) -> Result<Definition, String> {
    if !value.is_object() {
        return Err("not a map".into());
    }
    validated(value, true)
}

/// `TrialSpawnerConfig.DIRECT_CODEC`; the registry is not synchronized.
fn decode_trial_spawner(_: &mut Decoder<'_>, _: &str, value: Value) -> Result<Definition, String> {
    crate::mobs::trial_spawner::validate_config(&value)?;
    validated(value, false)
}

/// `SulfurCubeArchetype.DIRECT_CODEC`.
fn decode_sulfur_cube_archetype(
    decoder: &mut Decoder<'_>,
    id: &str,
    value: Value,
) -> Result<Definition, String> {
    crate::mobs::sulfur_cube::Decoded::decode(id, &value, decoder.sound_exists)?;
    validated(value, true)
}

fn validated(value: Value, synchronized: bool) -> Result<Definition, String> {
    let wire = synchronized.then(|| json_to_nbt(&value));
    if let Some(wire) = &wire {
        check_network_nbt(wire)?;
    }
    Ok(Definition::Validated(value, wire))
}

fn decode_world_clock(_: &mut Decoder<'_>, _: &str, value: Value) -> Result<Definition, String> {
    if !value.is_object() {
        return Err("not a map".into());
    }
    encoded(json_to_nbt(&value))
}

/// `Timeline.DIRECT_CODEC` (full validation) and `Timeline.NETWORK_CODEC` (syncable tracks
/// only on the wire).
fn decode_timeline(_: &mut Decoder<'_>, _: &str, value: Value) -> Result<Definition, String> {
    let timeline = Timeline::parse(&value)?;
    let network = Timeline::filter_syncable_json(&value);
    let wire = json_to_nbt(&network);
    check_network_nbt(&wire)?;
    Ok(Definition::Timeline(Arc::new(timeline), wire))
}

#[cfg(test)]
#[path = "data_pack_game_tests.rs"]
mod game_tests;

#[cfg(test)]
#[path = "data_pack_enchantment_provider_tests.rs"]
mod enchantment_provider_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn enchantment(description: &str) -> Value {
        json!({
            "description": description, "weight": 1, "max_level": 1, "anvil_cost": 1,
            "min_cost": {"base": 1, "per_level_above_first": 0},
            "max_cost": {"base": 5, "per_level_above_first": 0},
            "supported_items": "minecraft:stick", "slots": ["hand"]
        })
    }

    #[test]
    fn a_set_object_reaches_clients_as_ids_and_plain_forms_are_untouched() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("items.json"),
            json!({"minecraft:stick": {}, "minecraft:iron_sword": {}, "test:custom": {}})
                .to_string(),
        )
        .unwrap();
        let mut sticky = enchantment("Sticky");
        sticky["supported_items"] = json!({"type": "neoforge:or",
            "values": ["#minecraft:swords", "minecraft:stick"]});
        sticky["primary_items"] = json!({"type": "neoforge:any"});
        sticky["exclusive_set"] = json!({"type": "neoforge:any"});
        let mut plain = enchantment("Plain");
        plain["supported_items"] = json!("#minecraft:swords");
        let files = [
            (
                "tags/minecraft:item/swords".into(),
                vec![
                    json!({"values": ["test:custom", "minecraft:iron_sword"]})
                        .to_string()
                        .into_bytes(),
                ],
            ),
            (
                "enchantment/test:sticky".into(),
                vec![sticky.to_string().into_bytes()],
            ),
            (
                "enchantment/test:plain".into(),
                vec![plain.to_string().into_bytes()],
            ),
        ];
        let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
        let packed = PackRegistryDefinitions::load(
            root.path(),
            &defaults,
            &PackOverlay::new(&files),
            &Default::default(),
            &Default::default(),
            |_| true,
        )
        .unwrap();
        let mut wire = Vec::new();
        packed.append_encoded("minecraft:enchantment", &mut wire);
        assert_eq!(wire[0].0.1, "plain");
        assert_eq!(wire[1].0.1, "sticky");
        let ids =
            |ids: &[&str]| Tag::List(ids.iter().map(|id| Tag::String((*id).into())).collect());
        for (field, expected) in [
            (
                "supported_items",
                ids(&["test:custom", "minecraft:iron_sword", "minecraft:stick"]),
            ),
            (
                "primary_items",
                ids(&["minecraft:iron_sword", "minecraft:stick", "test:custom"]),
            ),
            ("exclusive_set", ids(&["test:plain", "test:sticky"])),
        ] {
            assert_eq!(wire[1].1.get(field), Some(&expected));
        }
        assert_eq!(
            wire[0].1.get("supported_items"),
            Some(&Tag::String("#minecraft:swords".into()))
        );
    }

    #[test]
    fn bound_tag_conditions_select_the_same_item_definitions_for_gameplay_and_wire() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("items.json"),
            br#"{"minecraft:stick": {}}"#,
        )
        .unwrap();
        let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
        let mut enchantment = enchantment("Conditional");
        let lower_instrument = root.path().join("data/test/instrument");
        let lower_enchantment = root.path().join("data/test/enchantment");
        std::fs::create_dir_all(&lower_instrument).unwrap();
        std::fs::create_dir_all(&lower_enchantment).unwrap();
        std::fs::write(lower_instrument.join("conditional.json"),
            br#"{"sound_event":"test:lower","use_duration":3,"range":32,"description":"Lower horn"}"#).unwrap();
        std::fs::write(
            lower_enchantment.join("conditional.json"),
            enchantment.to_string(),
        )
        .unwrap();
        let guard = json!([{"type": "neoforge:tag_empty", "tag": "test:gate"}]);
        enchantment["neoforge:conditions"] = guard.clone();
        let files = [
            (
                "instrument/test:conditional".into(),
                vec![
                json!({"sound_event": {"sound_id": "test:horn"}, "use_duration": 5, "range": 64,
                    "description": "Conditional horn", "neoforge:conditions": guard})
                    .to_string().into_bytes()
            ],
            ),
            (
                "enchantment/test:conditional".into(),
                vec![enchantment.to_string().into_bytes()],
            ),
        ];
        for (tag, admitted) in [("test:unrelated", true), ("test:gate", false)] {
            let mut items = Items::for_tests(std::collections::HashMap::from([(
                tag.to_owned(),
                std::collections::HashSet::from(["minecraft:stick".to_owned()]),
            )]));
            let conditions =
                sunrise_world::conditions::LoadConditions::new([]).with_tags(items.bound_tags());
            let overlay = PackOverlay {
                conditions: &conditions,
                files: &files,
                ..PackOverlay::default()
            };
            let (packed, admission) = super::super::SyncData::stage_registry_definitions(
                root.path(),
                &defaults,
                &overlay,
                &Default::default(),
                &Default::default(),
                |_| true,
            );
            admission.unwrap();
            // Gameplay must not evaluate the definition conditions with this other context.
            packed
                .install_items(
                    &mut items,
                    root.path(),
                    &PackOverlay::new(&files),
                    &defaults,
                )
                .unwrap();
            assert_eq!(
                items
                    .instruments
                    .get("test:conditional")
                    .map(|i| i.sound.as_str()),
                admitted.then_some("test:horn")
            );
            assert_eq!(
                items.enchantments.get("test:conditional").is_some(),
                admitted
            );
            assert_eq!(
                items
                    .component_registry
                    .enchantments
                    .contains(&"test:conditional".to_owned()),
                admitted
            );
            for registry in ["minecraft:instrument", "minecraft:enchantment"] {
                let mut wire = Vec::new();
                packed.append_encoded(registry, &mut wire);
                assert_eq!(
                    wire.first().map(|((ns, id), _, from_pack)| (
                        ns.as_str(),
                        id.as_str(),
                        *from_pack
                    )),
                    admitted.then_some(("test", "conditional", true)),
                    "{registry}"
                );
            }
        }
    }

    /// What world open makes of `files` over a resource root holding the three
    /// vanilla dialogs and their tags (`stage` is the load the world open runs).
    fn load_dialogs(
        files: &[(String, Vec<Vec<u8>>)],
    ) -> Result<Vec<((String, String), Tag, bool)>, String> {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("items.json"),
            json!({"minecraft:stone": {}, "minecraft:air": {}}).to_string(),
        )
        .unwrap();
        let dialogs = root.path().join("data/minecraft/dialog");
        let tags = root.path().join("data/minecraft/tags/dialog");
        std::fs::create_dir_all(&dialogs).unwrap();
        std::fs::create_dir_all(&tags).unwrap();
        for name in ["custom_options", "quick_actions", "server_links"] {
            std::fs::write(
                dialogs.join(format!("{name}.json")),
                br#"{"type":"notice","title":"Vanilla"}"#,
            )
            .unwrap();
        }
        for name in ["pause_screen_additions", "quick_actions"] {
            std::fs::write(
                tags.join(format!("{name}.json")),
                br#"{"values":["minecraft:quick_actions"]}"#,
            )
            .unwrap();
        }
        let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
        let reg = crate::command::Registries {
            items: defaults.entries().keys().cloned().collect(),
            ..Default::default()
        };
        let (registry, result) = PackRegistryDefinitions::stage(
            root.path(),
            &defaults,
            &PackOverlay::new(files),
            &BTreeMap::new(),
            &reg,
            |_| true,
        );
        result?;
        let mut entries = Vec::new();
        registry.append_encoded("minecraft:dialog", &mut entries);
        Ok(entries)
    }

    /// Cases the dialog fixture leaves to the typed component codec: a persistent
    /// component patch's keys, values, removal payloads and duplicates.
    const COMPONENT_PATCH_CASES: &[&str] = &[
        "body/component/unknown_key",
        "body/component/unknown_removal",
        "body/component/wrong_type",
        "body/component/wrong_type_negative",
        "body/component/removal_number",
        "body/component/removal_string",
        "body/component/removal_list",
        "body/component/duplicate_after_namespace",
        "body/component/remove_and_removal_namespace",
        "body/component/invalid_key_chars",
        "body/component/not_transient_only",
        "body/component/bad_nested_text",
        "text_show_item/unknown_key",
        "text_show_item/wrong_type",
        "text_show_item/removal_number",
        "text_show_item/duplicate_after_namespace",
        "text_show_item/in_button_label",
        "text_show_item/in_body",
    ];

    const DIALOG_FIXTURE: &str =
        include_str!("../../../sunrise-client/tests/fixtures/dialog_codec_cases.json");

    #[test]
    fn dialog_packets_refuse_bad_components_and_unbound_values() {
        use crate::dialog::Dialog;
        let fixture: Value = serde_json::from_str(DIALOG_FIXTURE).unwrap();
        let reg = crate::command::Registries {
            items: vec!["minecraft:stone".into()],
            ..Default::default()
        };
        for case in fixture["packets"].as_array().unwrap() {
            let (tag, _) = sunrise_core::nbt::parse_snbt(case["snbt"].as_str().unwrap()).unwrap();
            let parse = |tag| Dialog::parse(tag, &reg, &|id| id == "minecraft:stone", &|_| false);
            let decoded = parse(&tag);
            assert_eq!(
                decoded.is_some(),
                case["accept"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
            if case["observe"] == "inline_custom_activation" {
                use crate::{dialog::Click, text::ClickEvent};
                let dialog = decoded.unwrap();
                let Some(ClickEvent::ShowDialog(holder)) = &dialog.title.spans[0].style.click
                else {
                    panic!("title must open an inline dialog");
                };
                let opened = parse(holder).unwrap();
                let buttons = opened.buttons();
                let button = buttons[0];
                let Some(Click::Custom { id, payload }) =
                    button.action.as_ref().unwrap().click(&[], &[])
                else {
                    panic!("button must activate a custom click");
                };
                let expected = &case["round_trip"];
                let (expected_payload, _) =
                    sunrise_core::nbt::parse_snbt(expected["payload"].as_str().unwrap()).unwrap();
                assert_eq!(
                    json!({"id": id, "width": button.width}),
                    json!({"id": expected["id"], "width": expected["width"]})
                );
                assert_eq!(payload, Some(expected_payload));
            }
        }
    }

    #[test]
    fn admitted_dialogs_emit_the_decoded_values_over_nbt() {
        use crate::dialog::{Body, Control, Dialog, Reference};
        use crate::text::ClickEvent;
        use sunrise_core::nbt::{read_network, write_network};
        let fixture: Value = serde_json::from_str(DIALOG_FIXTURE).unwrap();
        let reg = crate::command::Registries {
            items: vec!["minecraft:stone".into()],
            ..Default::default()
        };
        let parse = |tag: &Tag| {
            Dialog::parse(
                tag,
                &reg,
                &|id| id == "minecraft:stone",
                &|reference| matches!(reference, Reference::Id(id) if id == "minecraft:quick_actions"),
            )
        };
        let mut stacks = Vec::new();
        for case in fixture["cases"].as_array().unwrap() {
            let Some(observe) = case["observe"].as_str() else {
                continue;
            };
            let name = case["name"].as_str().unwrap();
            let files = [(
                "dialog/test:case".into(),
                vec![serde_json::to_vec(&case["dialog"]).unwrap()],
            )];
            let entries = load_dialogs(&files).unwrap_or_else(|error| panic!("{name}: {error}"));
            let (_, emitted, from_pack) = &entries[0];
            assert!(from_pack);
            let bytes = write_network(emitted, Default::default()).unwrap();
            let (tag, used) = read_network(&bytes, Default::default()).unwrap();
            assert_eq!(used, bytes.len());
            let mut dialog =
                parse(&tag).unwrap_or_else(|| panic!("{name}: refused emitted dialog"));
            if observe == "inline_button" {
                let Some(ClickEvent::ShowDialog(inner)) = dialog
                    .title
                    .spans
                    .iter()
                    .find_map(|span| span.click_event())
                else {
                    panic!("{name}: inline holder was lost");
                };
                dialog =
                    parse(inner).unwrap_or_else(|| panic!("{name}: refused emitted inline dialog"));
            }
            let actual = match observe {
                "button" | "inline_button" => {
                    let button = dialog.buttons()[0];
                    let label = button
                        .label
                        .source_component()
                        .and_then(|component| component.get("translate"))
                        .and_then(Value::as_str)
                        .map_or_else(|| button.label.plain(), str::to_owned);
                    json!({"label": label, "width": button.width, "has_action": button.action.is_some()})
                }
                "named_holder" => {
                    let Some(Tag::String(id)) = tag
                        .get("title")
                        .and_then(|title| title.get("click_event"))
                        .and_then(|click| click.get("dialog"))
                    else {
                        panic!("{name}: missing emitted holder")
                    };
                    json!({"id": id.to_string_lossy()})
                }
                "item" => {
                    let Body::Item { item, .. } = &dialog.body[0] else {
                        panic!("{name}: expected item body")
                    };
                    // The exact stack reader the renderer uses. Status reads the same id.
                    let stack = sunrise_items::item_nbt::item_from_nbt(item).unwrap();
                    let id = stack.item.as_ref().to_owned();
                    let count = stack.count;
                    assert_eq!(
                        emitted
                            .get("body")
                            .and_then(|body| match body {
                                Tag::List(items) => items.first(),
                                _ => None,
                            })
                            .and_then(|body| body.get("item"))
                            .and_then(|stack| stack.get("count")),
                        Some(&Tag::Int(count as i32))
                    );
                    assert_eq!(dialog.json(&[])["body"][0], id);
                    stacks.push((id.clone(), count));
                    json!({"id": id, "count": count})
                }
                "multiline" => {
                    let Control::Text {
                        multiline: Some((max_lines, height)),
                        ..
                    } = &dialog.inputs[0].control
                    else {
                        panic!("{name}: expected multiline control")
                    };
                    json!({"max_lines": max_lines, "height": crate::dialog::text_height(Some((*max_lines, *height))) as i32})
                }
                _ => panic!("{name}: unknown observation {observe}"),
            };
            assert_eq!(actual, case["round_trip"], "{name}");
        }
        assert_eq!(stacks.len(), 2);
        assert_eq!(stacks[0], stacks[1]);
    }

    /// The dialog codec fixture holds what real 26.3 answered
    /// (`Dialog.DIRECT_CODEC` for each case, `RegistryDataLoader` for each pack),
    /// recorded by `tools/test_dialog_codec_parity.py`.
    #[test]
    fn dialog_admission_answers_as_the_real_codecs_do() {
        let fixture: Value = serde_json::from_str(DIALOG_FIXTURE).unwrap();
        let mut wrong = Vec::new();
        for case in fixture["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let accept = case["accept"].as_bool().unwrap();
            // Cross-entry references are the packs' cases below.
            if COMPONENT_PATCH_CASES.contains(&name)
                || [
                    "action/show_dialog_unknown_id",
                    "list/dialog_list_unknown_id",
                    "list/dialog_list_unknown_tag",
                    "list/dialog_list_second_unknown",
                ]
                .contains(&name)
            {
                continue;
            }
            let files = [(
                "dialog/test:case".to_owned(),
                vec![serde_json::to_vec(&case["dialog"]).unwrap()],
            )];
            let result = load_dialogs(&files);
            if result.is_ok() != accept {
                wrong.push(format!(
                    "{name}: vanilla {}, Sunrise {}",
                    if accept {
                        "accepts".to_owned()
                    } else {
                        format!("rejects ({})", case["vanilla_error"].as_str().unwrap())
                    },
                    match result {
                        Ok(_) => "accepts".to_owned(),
                        Err(error) => format!("rejects ({error})"),
                    },
                ));
            }
        }
        for case in fixture["packs"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let mut files: Vec<(String, Vec<Vec<u8>>)> = case["files"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(path, value)| {
                    let rest = path.strip_prefix("data/").unwrap();
                    let (namespace, rest) = rest.split_once('/').unwrap();
                    let key = match rest.strip_prefix("tags/dialog/") {
                        Some(tag) => {
                            format!("tags/{namespace}:dialog/{}", tag.trim_end_matches(".json"))
                        }
                        None => format!(
                            "dialog/{namespace}:{}",
                            rest.strip_prefix("dialog/")
                                .unwrap()
                                .trim_end_matches(".json")
                        ),
                    };
                    (key, vec![serde_json::to_vec(value).unwrap()])
                })
                .collect();
            if let Some(length) = case["tag_chain"].as_u64() {
                files.extend((0..length).map(|i| {
                    let value = if i + 1 == length {
                        "test:leaf".into()
                    } else {
                        format!("#test:chain/{:05}", i + 1)
                    };
                    (
                        format!("tags/test:dialog/chain/{i:05}"),
                        vec![serde_json::to_vec(&json!({"values": [value]})).unwrap()],
                    )
                }));
            }
            let result = load_dialogs(&files);
            let accept = case["accept"].as_bool().unwrap();
            if result.is_ok() != accept {
                wrong.push(format!(
                    "{name}: vanilla accept={accept}, Sunrise {result:?}"
                ));
            }
            if let (Ok(entries), Some(expected)) = (result, case["round_trip"].as_object()) {
                let root = tempfile::tempdir().unwrap();
                let held: BTreeSet<_> = entries
                    .iter()
                    .map(|((namespace, path), _, _)| format!("{namespace}:{path}"))
                    .collect();
                let (bound, _) = bind_dialog_tags(root.path(), &PackOverlay::new(&files), |id| {
                    held.contains(id)
                });
                for (tag, members) in expected {
                    assert_eq!(json!(bound.get(tag)), *members, "{name}: {tag}");
                }
            }
        }
        assert!(
            wrong.is_empty(),
            "{} cases differ:\n{}",
            wrong.len(),
            wrong.join("\n")
        );
    }

    #[test]
    fn a_bad_chat_type_and_a_bad_painting_variant_refuse_the_load_together() {
        let files = [
            (
                "chat_type/test:bad".into(),
                vec![
                    json!({
                        "chat": {"translation_key": "x", "parameters": ["nobody"]},
                        "narration": {"translation_key": "y", "parameters": []}
                    })
                    .to_string()
                    .into_bytes(),
                ],
            ),
            (
                "painting_variant/test:bad".into(),
                vec![br#"{"asset_id":"test:mural","width":17,"height":1}"#.to_vec()],
            ),
        ];
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("items.json"), b"{}").unwrap();
        let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
        let (_, Err(error)) = super::super::SyncData::stage_registry_definitions(
            root.path(),
            &defaults,
            &PackOverlay::new(&files),
            &Default::default(),
            &Default::default(),
            |_| true,
        ) else {
            panic!("world registry load accepted invalid entries");
        };
        let group = |registry: &str| {
            error
                .split_once(&format!("\nminecraft:{registry}:\n"))
                .unwrap()
                .1
                .split("\nminecraft:")
                .next()
                .unwrap()
                .to_owned()
        };
        assert!(
            group("chat_type").contains(
                "test:bad: data/test/chat_type/bad.json: chat.parameters: unknown parameter"
            ),
            "{error}"
        );
        assert!(
            group("painting_variant").contains("test:bad: data/test/painting_variant/bad.json:")
                && group("painting_variant").contains("width"),
            "{error}"
        );
    }

    #[test]
    fn a_packs_banner_pattern_in_no_item_required_reaches_the_loom() {
        let files = [
            (
                "banner_pattern/test:stripe_x".into(),
                vec![br#"{"asset_id":"test:stripe_x_asset","translation_key":"block.test.banner.stripe_x"}"#.to_vec()],
            ),
            (
                "tags/minecraft:banner_pattern/no_item_required".into(),
                vec![br#"{"values":["test:stripe_x"]}"#.to_vec()],
            ),
        ];
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("items.json"), b"{}").unwrap();
        let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
        let overlay = PackOverlay::new(&files);
        let (packed, admitted) = super::super::SyncData::stage_registry_definitions(
            root.path(),
            &defaults,
            &overlay,
            &BTreeMap::new(),
            &Default::default(),
            |_| true,
        );
        admitted.unwrap();
        let tags = super::super::registry_tag_bindings(
            root.path(),
            &overlay,
            "minecraft:banner_pattern",
            |id| id == "test:stripe_x",
        )
        .unwrap();
        let banners = crate::banner::Banners::stage(root.path(), &defaults, &packed, tags);
        assert_eq!(
            banners.selectable(None),
            [std::sync::Arc::from("test:stripe_x")]
        );
        assert_eq!(
            banners.banner_texture("test:stripe_x"),
            "test:entity/banner/stripe_x_asset"
        );
        let layer = crate::banner::BannerLayer::new("test:stripe_x", "red");
        assert_eq!(banners.layer_key(&layer), "block.test.banner.stripe_x.red");
    }

    #[test]
    fn every_bad_winning_definition_is_reported_in_one_error() {
        let mut oversized = enchantment("Oversized");
        // Each string fits modified UTF-8, but together they exceed the 2 MiB quota.
        oversized["description"] = json!(vec!["x".repeat(64_000); 33]);
        assert!(
            sunrise_core::nbt::write_network(&json_to_nbt(&oversized), Default::default()).is_ok()
        );
        let files = [
            (
                "dialog/test:broken".into(),
                vec![
                    br#"{"type":"notice","title":"Lower"}"#.to_vec(),
                    b"{".to_vec(),
                ],
            ),
            (
                "dialog/test:negative_columns".into(),
                vec![br#"{"type":"multi_action","title":"Buttons","actions":[{"label":"OK"}],"columns":-1}"#.to_vec()],
            ),
            (
                "dialog/test:wrong_columns".into(),
                vec![br#"{"type":"dialog_list","title":"Buttons","dialogs":[],"columns":true}"#.to_vec()],
            ),
            (
                "dialog/test:wide_button".into(),
                vec![br#"{"type":"server_links","title":"Links","button_width":1025}"#.to_vec()],
            ),
            (
                "dialog/test:wrong_pause".into(),
                vec![br#"{"type":"notice","title":"Pause","pause":1}"#.to_vec()],
            ),
            (
                "dialog/test:wrong_action".into(),
                vec![br#"{"type":"notice","title":"Action","after_action":"stay"}"#.to_vec()],
            ),
            (
                "dialog/test:condition_false".into(),
                vec![br#"{"neoforge:conditions":[{"type":"neoforge:never"}]}"#.to_vec()],
            ),
            (
                "dimension_type/test:wrong_scale".into(),
                vec![br#"{"min_y":0,"height":64,"logical_height":64,"has_skylight":true,"has_ceiling":false,"has_ender_dragon_fight":false,"infiniburn":["minecraft:netherrack"],"coordinate_scale":"one","ambient_light":0,"monster_spawn_light_level":0,"monster_spawn_block_light_limit":0}"#.to_vec()],
            ),
            (
                "dimension_type/test:unknown_attribute".into(),
                vec![br#"{"min_y":0,"height":64,"logical_height":64,"has_skylight":true,"has_ceiling":false,"has_ender_dragon_fight":false,"infiniburn":["minecraft:netherrack"],"coordinate_scale":1,"ambient_light":0,"monster_spawn_light_level":0,"monster_spawn_block_light_limit":0,"attributes":{"minecraft:visual/not_an_attribute":1}}"#.to_vec()],
            ),
            (
                "dimension_type/test:empty_bed_rule".into(),
                vec![br#"{"min_y":0,"height":64,"logical_height":64,"has_skylight":true,"has_ceiling":false,"has_ender_dragon_fight":false,"infiniburn":["minecraft:netherrack"],"coordinate_scale":1,"ambient_light":0,"monster_spawn_light_level":0,"monster_spawn_block_light_limit":0,"attributes":{"minecraft:gameplay/bed_rule":{}}}"#.to_vec()],
            ),
            ("enchantment/test:not_read".into(), vec![b"{}".to_vec()]),
            ("enchantment/test:wrong_level".into(), vec![br#"{"max_level":"seven"}"#.to_vec()]),
            (
                "enchantment/test:unencodable".into(),
                vec![enchantment(&"\0".repeat(32_768)).to_string().into_bytes()],
            ),
            (
                "enchantment/test:oversized".into(),
                vec![oversized.to_string().into_bytes()],
            ),
            ("painting_variant/test:numeric_style".into(), vec![br#"{"asset_id":"test:mural","width":1,"height":1,"title":{"text":"No","bold":1}}"#.to_vec()]),
            ("painting_variant/test:numeric_object".into(), vec![br#"{"asset_id":"test:mural","width":1,"height":1,"author":{"object":7,"sprite":"test:icon"}}"#.to_vec()]),
            ("painting_variant/test:wrong_atlas".into(), vec![br#"{"asset_id":"test:mural","width":1,"height":1,"author":{"object":"atlas","atlas":7,"sprite":"test:icon","player":"Steve"}}"#.to_vec()]),
            ("painting_variant/test:legacy_click".into(), vec![br#"{"asset_id":"test:mural","width":1,"height":1,"title":{"text":"Label","hover_event":{"action":"show_text","value":{"text":"Click","click_event":{"action":"run_command","value":"/help"}}}}}"#.to_vec()]),
            (
                "painting_variant/test:bad".into(),
                vec![br#"{"asset_id":"test:mural","width":17,"height":1}"#.to_vec()],
            ),
            (
                "instrument/test:bad".into(),
                vec![br#"{"sound_event":"minecraft:item.goat_horn.sound.0","use_duration":-1,"range":128,"description":"Broken"}"#.to_vec()],
            ),
            (
                "instrument/test:wrong_sound_range".into(),
                vec![br#"{"sound_event":{"sound_id":"test:sound","range":"near"},"use_duration":4,"range":128,"description":"Horn"}"#.to_vec()],
            ),
            (
                "banner_pattern/test:bad".into(),
                vec![br#"{"asset_id":"test:banner","translation_key":7}"#.to_vec()],
            ),
            (
                "banner_pattern/test:unencodable".into(),
                vec![serde_json::json!({"asset_id": "test:banner", "translation_key": "\0".repeat(32_768)})
                    .to_string()
                    .into_bytes()],
            ),
        ];
        let io = BTreeMap::from([("enchantment/test:not_read".into(), "read failed".into())]);
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("items.json"), b"{}").unwrap();
        let instruments = root.path().join("data/test/instrument");
        std::fs::create_dir_all(&instruments).unwrap();
        std::fs::write(
            instruments.join("resource_duration.json"),
            br#"{"sound_event":"test:horn","use_duration":"four","range":128,"description":"Broken resource horn"}"#,
        )
        .unwrap();
        let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
        let (_, Err(error)) = super::super::SyncData::stage_registry_definitions(
            root.path(),
            &defaults,
            &PackOverlay::new(&files),
            &io,
            &Default::default(),
            |_| true,
        ) else {
            panic!("world registry load accepted invalid entries");
        };
        for (registry, defs) in [
            (
                "dialog",
                &[
                    ("broken", ""),
                    ("negative_columns", "columns"),
                    ("wrong_columns", "columns"),
                    ("wide_button", "button_width"),
                    ("wrong_pause", "pause"),
                    ("wrong_action", "after_action"),
                ][..],
            ),
            (
                "dimension_type",
                &[
                    ("wrong_scale", "coordinate_scale"),
                    ("empty_bed_rule", "bed_rule"),
                    ("unknown_attribute", "minecraft:visual/not_an_attribute"),
                ],
            ),
            (
                "enchantment",
                &[
                    ("not_read", "read failed"),
                    ("oversized", "byte limit exceeded"),
                    ("wrong_level", "max_level"),
                    ("unencodable", "modified UTF-8"),
                ],
            ),
            (
                "painting_variant",
                &[
                    ("numeric_style", "title"),
                    ("numeric_object", "author"),
                    ("wrong_atlas", "author"),
                    ("legacy_click", "title"),
                    ("bad", "width"),
                ],
            ),
            (
                "instrument",
                &[
                    ("bad", "use_duration"),
                    ("resource_duration", "use_duration"),
                    ("wrong_sound_range", "sound_event.range"),
                ],
            ),
            (
                "banner_pattern",
                &[
                    ("bad", "translation_key"),
                    ("unencodable", "modified UTF-8"),
                ],
            ),
        ] {
            let header = format!("\nminecraft:{registry}:\n");
            let group = error
                .split_once(&header)
                .unwrap()
                .1
                .split("\nminecraft:")
                .next()
                .unwrap();
            for (id, field) in defs {
                let entry = format!("  test:{id}:");
                assert!(
                    group
                        .lines()
                        .any(|line| line.starts_with(&entry) && line.contains(field)),
                    "{error}"
                );
            }
        }
        assert!(!error.contains("condition_false"));
    }

    fn stage(
        files: &[(String, Vec<Vec<u8>>)],
        sound_exists: impl Fn(&str) -> bool,
    ) -> (PackRegistryDefinitions, Result<(), String>) {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("items.json"), b"{}").unwrap();
        let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
        super::super::SyncData::stage_registry_definitions(
            root.path(),
            &defaults,
            &PackOverlay::new(files),
            &Default::default(),
            &Default::default(),
            sound_exists,
        )
    }

    fn pack(path: &str, value: Value) -> (String, Vec<Vec<u8>>) {
        (path.into(), vec![value.to_string().into_bytes()])
    }

    #[test]
    fn a_pack_decorated_pot_pattern_is_staged_for_gameplay_and_the_wire() {
        let files = [pack(
            "decorated_pot_pattern/test:flame",
            json!({"asset_id": "test:flame_pottery_pattern"}),
        )];
        let (staged, result) = stage(&files, |_| true);
        result.unwrap();
        let held = staged.validated("minecraft:decorated_pot_pattern");
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].0, "test:flame");
        assert_eq!(held[0].1["asset_id"], "test:flame_pottery_pattern");
        let mut wire = Vec::new();
        staged.append_encoded("minecraft:decorated_pot_pattern", &mut wire);
        assert_eq!(wire.len(), 1);
        assert_eq!(wire[0].0, ("test".to_owned(), "flame".to_owned()));
        assert_eq!(
            wire[0].1.get("asset_id"),
            Some(&string("test:flame_pottery_pattern"))
        );
        assert!(wire[0].2);
        let (_, refused) = stage(
            &[pack(
                "decorated_pot_pattern/test:bad",
                json!({"asset": "x"}),
            )],
            |_| true,
        );
        assert!(refused.unwrap_err().contains("asset_id is required"));
    }

    #[test]
    fn a_pack_block_transformer_is_decoded_and_a_bad_one_refuses_the_load() {
        let transformer = json!([{
            "block_state_provider": {"id": "minecraft:dirt"},
            "sound": "minecraft:item.hoe.till"
        }]);
        let (staged, result) = stage(
            &[pack("block_transformer/test:hammer", transformer.clone())],
            |sound| sound == "minecraft:item.hoe.till",
        );
        result.unwrap();
        assert_eq!(staged.validated("minecraft:block_transformer").len(), 1);
        let (_, refused) = stage(
            &[pack("block_transformer/test:hammer", transformer)],
            |_| false,
        );
        assert!(
            refused
                .unwrap_err()
                .contains("unknown holder minecraft:item.hoe.till")
        );
    }

    #[test]
    fn a_pack_trial_spawner_is_kept_for_the_server_and_never_sent() {
        let config = json!({
            "spawn_range": 6,
            "spawn_potentials": [{"weight": 1, "data": {"entity": {"id": "minecraft:husk"}}}]
        });
        let (staged, result) = stage(&[pack("trial_spawner/test:husks", config)], |_| true);
        result.unwrap();
        let held = staged.validated("minecraft:trial_spawner");
        assert_eq!(held[0].0, "test:husks");
        let decoded = <crate::mobs::trial_spawner::TrialConfig as crate::mobs::trial_spawner::TrialConfigExt>::decode(
            &crate::scoreboard::json_to_tag(held[0].1),
        )
        .unwrap();
        assert_eq!(decoded.spawn_range, 6);
        assert_eq!(decoded.spawn_potentials.len(), 1);
        let mut wire = Vec::new();
        staged.append_encoded("minecraft:trial_spawner", &mut wire);
        assert!(wire.is_empty());
        assert!(!super::super::SYNCHRONIZED.contains(&"minecraft:trial_spawner"));
        let (_, refused) = stage(
            &[pack("trial_spawner/test:far", json!({"spawn_range": 129}))],
            |_| true,
        );
        assert!(refused.unwrap_err().contains("spawn_range"));
    }

    fn archetype_file(items: &str) -> Value {
        json!({
            "items": items,
            "attribute_modifiers": [{
                "attribute": "minecraft:bounciness", "id": "test:springy_add_bounciness",
                "amount": 0.75, "operation": "add_value"
            }],
            "buoyant": true,
            "knockback_modifiers": {"horizontal_power": 0.5, "vertical_power": 0.1},
            "sound_settings": {
                "hit_sound": "minecraft:entity.sulfur_cube.regular.hit",
                "push_sound": "minecraft:entity.sulfur_cube.regular.push",
                "push_sound_impulse_threshold": 0.2, "push_sound_cooldown": 0.5
            }
        })
    }

    #[test]
    fn a_pack_sulfur_cube_archetype_takes_its_items_from_the_world_tags() {
        let files = [pack(
            "sulfur_cube_archetype/test:springy",
            archetype_file("#test:springy_items"),
        )];
        let (staged, result) = stage(&files, |_| true);
        result.unwrap();
        let held = staged.validated("minecraft:sulfur_cube_archetype");
        let tags = std::collections::HashMap::from([(
            "test:springy_items".to_owned(),
            std::collections::HashSet::from(["minecraft:slime_block".to_owned()]),
        )]);
        let built = crate::mobs::sulfur_cube::build(&held, &tags);
        assert_eq!(built.len(), 1);
        assert!(built[0].items.contains("minecraft:slime_block"));
        assert!(built[0].buoyant);
        assert_eq!(built[0].bounciness, 0.75);
        assert_eq!(built[0].hit_sound, "entity.sulfur_cube.regular.hit");
        let mut wire = Vec::new();
        staged.append_encoded("minecraft:sulfur_cube_archetype", &mut wire);
        assert_eq!(wire.len(), 1);
        let mut bad = archetype_file("#test:springy_items");
        bad.as_object_mut().unwrap().remove("knockback_modifiers");
        let (_, refused) = stage(&[pack("sulfur_cube_archetype/test:springy", bad)], |_| true);
        assert!(
            refused
                .unwrap_err()
                .contains("knockback_modifiers is required")
        );
        let (_, refused) = stage(&files, |sound| !sound.ends_with(".push"));
        assert!(refused.unwrap_err().contains("push_sound: unknown holder"));
    }

    #[test]
    fn a_pack_overriding_an_overworld_timeline_changes_server_and_synced_client_sky() {
        use crate::environment::{Environment, Sample, TimelineTable, sky_darken};
        use sunrise_assets::resource_source::ResourceSource;

        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data/minecraft");
        std::fs::create_dir_all(data.join("world_clock")).unwrap();
        std::fs::create_dir_all(data.join("timeline")).unwrap();
        std::fs::create_dir_all(data.join("tags/timeline")).unwrap();
        std::fs::create_dir_all(data.join("dimension_type")).unwrap();
        std::fs::write(root.path().join("items.json"), b"{}").unwrap();
        std::fs::write(data.join("world_clock/overworld.json"), b"{}").unwrap();
        std::fs::write(
            data.join("timeline/day.json"),
            json!({
                "clock": "minecraft:overworld",
                "period_ticks": 24000,
                "tracks": {
                    "minecraft:gameplay/sky_light_level": {
                        "keyframes": [
                            {"ticks": 0, "value": 15.0},
                            {"ticks": 12000, "value": 15.0}
                        ]
                    },
                    "minecraft:visual/sky_color": {
                        "keyframes": [
                            {"ticks": 0, "value": "#0000ff"}
                        ]
                    }
                }
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            data.join("tags/timeline/in_overworld.json"),
            json!({"values": ["minecraft:day"]}).to_string(),
        )
        .unwrap();
        std::fs::write(
            data.join("dimension_type/overworld.json"),
            json!({
                "timelines": "#minecraft:in_overworld",
                "attributes": {
                    "minecraft:gameplay/sky_light_level": 15.0,
                    "minecraft:visual/sky_color": "#000000"
                }
            })
            .to_string(),
        )
        .unwrap();

        let pack_day = json!({
            "clock": "minecraft:overworld",
            "period_ticks": 24000,
            "tracks": {
                "minecraft:gameplay/sky_light_level": {
                    "keyframes": [
                        {"ticks": 0, "value": 6.0},
                        {"ticks": 12000, "value": 6.0}
                    ]
                },
                "minecraft:visual/sky_color": {
                    "keyframes": [
                        {"ticks": 0, "value": "#ff0000"}
                    ]
                },
                "minecraft:gameplay/monsters_burn": {
                    "keyframes": [
                        {"ticks": 0, "value": true}
                    ]
                }
            }
        });
        let files = [(
            "timeline/minecraft:day".into(),
            vec![pack_day.to_string().into_bytes()],
        )];
        let overlay = PackOverlay::new(&files);
        let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
        let packed = PackRegistryDefinitions::load(
            root.path(),
            &defaults,
            &overlay,
            &Default::default(),
            &Default::default(),
            |_| true,
        )
        .unwrap();

        let server_timelines = packed.timelines();
        let tags = super::super::registry_tag_bindings(
            root.path(),
            &overlay,
            "minecraft:timeline",
            |id| server_timelines.contains_key(id),
        )
        .unwrap();
        let server_table = TimelineTable {
            timelines: server_timelines,
            tags: tags.clone(),
        };

        let base =
            Environment::load(&ResourceSource::directory(root.path().to_path_buf())).unwrap();
        let server_env = Environment::with_type(&base, &Value::Null, &server_table).unwrap();
        let sample = Sample {
            day_time: 6000,
            ..Sample::default()
        };
        let vanilla_sky = base.sample(&sample);
        let server_sky = server_env.sample(&sample);
        assert_eq!(vanilla_sky.sky_light_level, 15.0);
        assert_eq!(vanilla_sky.sky_color, [0.0, 0.0, 1.0]);
        assert_eq!(sky_darken(&base, 6000, 0.0, 0.0), 0);
        assert_eq!(server_sky.sky_light_level, 6.0);
        assert_eq!(server_sky.sky_color, [1.0, 0.0, 0.0]);
        assert_eq!(sky_darken(&server_env, 6000, 0.0, 0.0), 9);

        // Sync to a client: non-syncable tracks are stripped (`Timeline.NETWORK_CODEC`),
        // and decoding the synced entries reproduces the exact sky value.
        let mut wire = Vec::new();
        packed.append_encoded("minecraft:timeline", &mut wire);
        assert_eq!(wire.len(), 1);
        assert_eq!(wire[0].0, ("minecraft".into(), "day".into()));
        assert!(wire[0].2, "pack override must be marked from_pack");
        let wire_json = crate::scoreboard::tag_to_json(&wire[0].1);
        assert!(
            wire_json["tracks"]
                .as_object()
                .is_some_and(|t| !t.contains_key("minecraft:gameplay/monsters_burn")),
            "non-syncable track leaked into wire NBT: {wire_json}"
        );
        let mut client_timelines = BTreeMap::new();
        for ((ns, path), tag, _) in wire {
            let decoded = Timeline::parse(&crate::scoreboard::tag_to_json(&tag)).unwrap();
            client_timelines.insert(format!("{ns}:{path}"), Arc::new(decoded));
        }
        let client_table = TimelineTable {
            timelines: client_timelines,
            tags,
        };
        let client_env = Environment::with_type(&base, &Value::Null, &client_table).unwrap();
        let client_sky = client_env.sample(&sample);
        assert_eq!(client_sky.sky_color, server_sky.sky_color);
        assert_eq!(client_sky.sky_light_level, server_sky.sky_light_level);
    }

    #[test]
    fn a_malformed_timeline_refuses_the_world_load() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data/minecraft");
        std::fs::create_dir_all(data.join("world_clock")).unwrap();
        std::fs::write(root.path().join("items.json"), b"{}").unwrap();
        std::fs::write(data.join("world_clock/overworld.json"), b"{}").unwrap();
        let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();

        let load_err = |files: &[(String, Vec<Vec<u8>>)]| -> String {
            PackRegistryDefinitions::load(
                root.path(),
                &defaults,
                &PackOverlay::new(files),
                &Default::default(),
                &Default::default(),
                |_| true,
            )
            .err()
            .expect("malformed timeline must refuse world load")
        };

        // Non-positive period_ticks.
        let err = load_err(&[(
            "timeline/demo:bad_period".into(),
            vec![
                json!({"clock": "minecraft:overworld", "period_ticks": 0})
                    .to_string()
                    .into_bytes(),
            ],
        )]);
        assert!(
            err.contains("minecraft:timeline") && err.contains("demo:bad_period"),
            "{err}"
        );

        // Keyframe outside [0; period_ticks].
        let err = load_err(&[(
            "timeline/demo:out_of_period".into(),
            vec![
                json!({
                    "clock": "minecraft:overworld",
                    "period_ticks": 100,
                    "tracks": {
                        "minecraft:gameplay/sky_light_level": {
                            "keyframes": [{"ticks": 101, "value": 10.0}]
                        }
                    }
                })
                .to_string()
                .into_bytes(),
            ],
        )]);
        assert!(
            err.contains("minecraft:timeline") && err.contains("demo:out_of_period"),
            "{err}"
        );

        // Time marker outside [0; period_ticks).
        let err = load_err(&[(
            "timeline/demo:bad_marker".into(),
            vec![
                json!({
                    "clock": "minecraft:overworld",
                    "period_ticks": 100,
                    "time_markers": {"demo:end": 100}
                })
                .to_string()
                .into_bytes(),
            ],
        )]);
        assert!(
            err.contains("minecraft:timeline") && err.contains("demo:bad_marker"),
            "{err}"
        );

        // Unknown world_clock reference.
        let err = load_err(&[(
            "timeline/demo:missing_clock_tl".into(),
            vec![
                json!({"clock": "demo:no_such_clock"})
                    .to_string()
                    .into_bytes(),
            ],
        )]);
        assert!(
            err.contains("minecraft:timeline") && err.contains("demo:missing_clock_tl"),
            "{err}"
        );

        // Duplicate time marker across two timelines sharing a clock (`Timeline.validateRegistry`).
        let err = load_err(&[
            (
                "timeline/demo:first".into(),
                vec![
                    json!({
                        "clock": "minecraft:overworld",
                        "period_ticks": 24000,
                        "time_markers": {"demo:shared": 1000}
                    })
                    .to_string()
                    .into_bytes(),
                ],
            ),
            (
                "timeline/demo:second".into(),
                vec![
                    json!({
                        "clock": "minecraft:overworld",
                        "period_ticks": 24000,
                        "time_markers": {"demo:shared": 2000}
                    })
                    .to_string()
                    .into_bytes(),
                ],
            ),
        ]);
        assert!(
            err.contains("minecraft:timeline") && err.contains("defined multiple times"),
            "{err}"
        );
    }

    fn song(sound: &str, length: f64, description: &str) -> Vec<u8> {
        json!({"sound_event": sound, "length_in_seconds": length, "comparator_output": 3,
            "description": description})
        .to_string()
        .into_bytes()
    }

    #[test]
    fn jukebox_songs_of_the_root_mods_and_packs_load_with_a_packs_winner_over_the_root() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("items.json"), b"{}").unwrap();
        for (namespace, name, bytes) in [
            (
                "minecraft",
                "cat",
                song("minecraft:music_disc.cat", 185.0, "cat"),
            ),
            ("mymod", "tune", song("mymod:music_disc.tune", 5.0, "tune")),
        ] {
            let dir = root.path().join(format!("data/{namespace}/jukebox_song"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("{name}.json")), bytes).unwrap();
        }
        let files = [
            (
                "jukebox_song/minecraft:cat".into(),
                vec![song("minecraft:music_disc.cat", 7.0, "replaced")],
            ),
            (
                "jukebox_song/demo:pack".into(),
                vec![song("demo:note", 2.0, "pack")],
            ),
        ];
        let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
        let overlay = PackOverlay::new(&files);
        let (packed, admitted) = super::super::SyncData::stage_registry_definitions(
            root.path(),
            &defaults,
            &overlay,
            &BTreeMap::new(),
            &Default::default(),
            |_| true,
        );
        admitted.unwrap();
        let mut held: Vec<_> = packed
            .entries()
            .filter(|(registry, ..)| *registry == "minecraft:jukebox_song")
            .map(|(_, id, admitted)| (id, admitted))
            .collect();
        held.sort();
        assert_eq!(
            held,
            [
                ("demo:pack".to_owned(), true),
                ("minecraft:cat".to_owned(), true),
                ("mymod:tune".to_owned(), true),
            ]
        );
        let mut wire = Vec::new();
        packed.append_encoded("minecraft:jukebox_song", &mut wire);
        let replaced = wire
            .iter()
            .find(|((namespace, path), ..)| {
                (namespace.as_str(), path.as_str()) == ("minecraft", "cat")
            })
            .unwrap();
        assert_eq!(
            replaced.1.get("description"),
            Some(&Tag::String("replaced".into()))
        );
        assert!(replaced.2, "a pack's winner is always sent");
    }

    #[test]
    fn a_jukebox_song_failing_its_codec_refuses_the_load_and_names_the_file() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("items.json"), b"{}").unwrap();
        let files = [
            (
                "jukebox_song/demo:good".into(),
                vec![song("demo:note", 5.0, "good")],
            ),
            ("jukebox_song/demo:bad".into(), vec![b"{}".to_vec()]),
            (
                "jukebox_song/demo:badtext".into(),
                vec![
                    json!({"sound_event": "demo:note", "length_in_seconds": 5.0,
                        "comparator_output": 3,
                        "description": {"text": "x", "color": "not a color"}})
                    .to_string()
                    .into_bytes(),
                ],
            ),
        ];
        let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
        let (_, Err(error)) = super::super::SyncData::stage_registry_definitions(
            root.path(),
            &defaults,
            &PackOverlay::new(&files),
            &BTreeMap::new(),
            &Default::default(),
            |_| true,
        ) else {
            panic!("a bad song was accepted");
        };
        assert!(
            error.contains("demo:bad: data/demo/jukebox_song/bad.json: "),
            "{error}"
        );
        assert!(
            error.contains(
                "demo:badtext: data/demo/jukebox_song/badtext.json: description: invalid JSON text component"
            ),
            "{error}"
        );
        assert!(!error.contains("demo:good:"), "{error}");
    }
}
