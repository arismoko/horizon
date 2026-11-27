//! Data packs (`<world>/datapacks`: folders and zips with `pack.mcmeta`) and
//! the parts of them commands use: `.mcfunction` functions (with macros and
//! line continuation), function tags and predicates.
use crate::pack_meta::{Filter, filter_of};
use crate::resource_source::{ResourcePath, ResourceSource};
use mod_api::__private::game::{datapack_entry_id, datapack_registry_directory};
use sunrise_core::identifier::Identifier;
use sunrise_world::loot::RecipePriorityLayer;
mod slot_sources {
    //! Slot-source holder-set tags share DataStore's pack ordering and snapshot.
    use super::DataStore;
    use std::collections::BTreeSet;
    impl DataStore {
        /// TagLoader's required-member validation and first-seen membership order.
        pub fn slot_source_tag(&self, id: &str) -> Result<Vec<String>, String> {
            fn visit(
                store: &DataStore,
                id: &str,
                active: &mut BTreeSet<String>,
            ) -> Result<Vec<String>, String> {
                if !active.insert(id.to_owned()) {
                    return Err(format!("Recursive slot-source tag #{id}"));
                }
                let result = (|| {
                    let entries = store
                        .slot_source_tags
                        .get(id)
                        .ok_or_else(|| format!("Unknown slot-source tag #{id}"))?;
                    let mut out = Vec::new();
                    for (entry, required, remove) in entries {
                        // A removal (`TagLoader.EntryWithSource.remove`) takes an
                        // id or a tag's members out of the set built so far, and
                        // an unknown one removes nothing.
                        if *remove {
                            let known = |what: &str| {
                                store.slot_sources.contains_key(what)
                                    || store.slot_source_tags.contains_key(what)
                            };
                            if let Some(tag) = entry.strip_prefix('#') {
                                let mut pending = vec![tag];
                                let mut walked = BTreeSet::new();
                                let mut cyclic = false;
                                while let Some(next) = pending.pop() {
                                    if next == id {
                                        cyclic = true;
                                        break;
                                    }
                                    if !walked.insert(next) {
                                        continue;
                                    }
                                    for (target, _, removes) in
                                        store.slot_source_tags.get(next).into_iter().flatten()
                                    {
                                        if *removes && let Some(target) = target.strip_prefix('#') {
                                            pending.push(target);
                                        }
                                    }
                                }
                                if cyclic && tag > id {
                                    log::warn!(
                                        "Sunrise: tags {id} and {tag} form a removal cycle; only removal edge {id} -> {tag} is ignored"
                                    );
                                    continue;
                                }
                                let mut members = Vec::new();
                                if let Ok(found) = visit(store, tag, active) {
                                    members.extend(found);
                                }
                                out.retain(|kept| !members.contains(kept));
                            } else if known(entry) {
                                out.retain(|kept| kept != entry);
                            }
                            continue;
                        }
                        let values = if let Some(tag) = entry.strip_prefix('#') {
                            visit(store, tag, active)
                        } else if store.slot_sources.contains_key(entry) {
                            Ok(vec![entry.clone()])
                        } else {
                            Err(format!("Unknown slot source {entry}"))
                        };
                        match values {
                            Ok(values) => {
                                for value in values {
                                    if !out.contains(&value) {
                                        out.push(value);
                                    }
                                }
                            }
                            Err(error) if *required => return Err(error),
                            Err(_) => {}
                        }
                    }
                    Ok(out)
                })();
                active.remove(id);
                result
            }
            visit(self, id, &mut BTreeSet::new())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn mutual_removal_cycles_drop_the_same_named_edge_from_both_roots() {
            let mut store = DataStore::default();
            for name in ["test:one", "test:two"] {
                store
                    .slot_sources
                    .insert(name.into(), serde_json::json!({}));
            }
            store.slot_source_tags.insert(
                "test:a".into(),
                vec![
                    ("test:one".into(), true, false),
                    ("#test:b".into(), true, true),
                ],
            );
            store.slot_source_tags.insert(
                "test:b".into(),
                vec![
                    ("test:one".into(), true, false),
                    ("test:two".into(), true, false),
                    ("#test:a".into(), true, true),
                ],
            );
            for (start, expected) in [("test:a", "test:one"), ("test:b", "test:two")] {
                let (out, warnings) =
                    crate::test_log::capture(|| store.slot_source_tag(start).unwrap());
                assert_eq!(out, [expected]);
                assert!(
                    warnings.iter().any(|line| line.contains("test:a")
                        && line.contains("test:b")
                        && line.contains("only removal edge test:a -> test:b is ignored")),
                    "{warnings:?}"
                );
            }
        }

        #[test]
        fn holder_set_tags_keep_order_required_members_and_snapshot_authority() {
            let mut store = DataStore::default();
            for name in ["t:a", "t:b"] {
                store.slot_sources.insert(
                    name.into(),
                    serde_json::json!({"type":"slot_range","slots":"container.0"}),
                );
            }
            store.slot_source_tags.insert(
                "t:child".into(),
                vec![("t:b".into(), true, false), ("t:a".into(), true, false)],
            );
            store.slot_source_tags.insert(
                "t:all".into(),
                vec![
                    ("t:a".into(), true, false),
                    ("#t:child".into(), true, false),
                    ("t:absent".into(), false, false),
                ],
            );
            assert_eq!(store.slot_source_tag("t:all").unwrap(), ["t:a", "t:b"]);
            store
                .slot_source_tags
                .get_mut("t:all")
                .unwrap()
                .push(("t:absent".into(), true, false));
            assert!(store.slot_source_tag("t:all").is_err());
            store
                .slot_source_tags
                .insert("t:cycle".into(), vec![("#t:cycle".into(), true, false)]);
            assert!(store.slot_source_tag("t:cycle").is_err());
        }
    }
}
use crate::zip::Zip;
use std::{
    collections::BTreeMap,
    io::{self, Read},
    path::{Path, PathBuf},
};
use {
    sunrise_core::nbt::Tag,
    sunrise_world::worldgen::{
        overrides,
        overrides::{PackFile, WorldgenOverrides},
    },
};

/// A pack on disk.
#[derive(Clone, Debug, PartialEq)]
pub struct DataPack {
    /// `file/<name>` in `/datapack` and level.dat.
    pub id: String,
    source: ResourceSource,
    filter: Vec<Filter>,
    /// Presentation and selection supplied by an additional repository source.
    pub selection: Option<crate::pack_meta::PackInfo>,
}
impl DataPack {
    /// Whether this repository entry comes from the feature-pack source.
    pub fn is_feature(&self) -> bool {
        self.selection
            .as_ref()
            .is_some_and(|info| info.source == crate::pack_meta::PackSource::Feature)
    }
    /// A named resource layer, retaining its open source for lazy reads. The pack's
    /// `pack.mcmeta` overlays for the data pack format are composed over its root here, once,
    /// so every read and listing of the pack sees them.
    pub fn from_source(id: String, source: ResourceSource) -> Self {
        Self {
            id,
            source: Self::with_overlays(source),
            filter: Vec::new(),
            selection: None,
        }
    }
    /// The pack's effective source: its root with the overlay directories its `pack.mcmeta`
    /// names for the data pack format (`Pack.readPackMetadata`) over it, later entries winning.
    /// As in `OverlayedPackResources`, only an overlay's `data/` files take part; every other
    /// file, `pack.mcmeta` included, is the root's. A pack with no applicable overlay keeps its
    /// root as it is.
    fn with_overlays(root: ResourceSource) -> ResourceSource {
        let Some(meta) = root
            .read("pack.mcmeta")
            .ok()
            .and_then(|bytes| crate::pack_meta::parse_meta(&bytes))
        else {
            return root;
        };
        let overlays = crate::pack_meta::data_overlays(&meta);
        if overlays.is_empty() {
            return root;
        }
        let mut files = BTreeMap::new();
        let layers: Vec<_> = overlays
            .iter()
            .filter_map(|d| root.subdirectory(d).ok().map(|layer| (d, layer)))
            .collect();
        // An overlay folder's own files are not the pack's: only its `data/` joins the index.
        for name in root.list("") {
            if layers.iter().any(|(dir, _)| {
                name.strip_prefix(dir.as_str())
                    .is_some_and(|r| r.starts_with('/'))
            }) {
                continue;
            }
            files.insert(
                name.clone(),
                crate::resource_source::Resource::file(root.clone(), name),
            );
        }
        for (_, layer) in &layers {
            for name in layer.list("data/") {
                files.insert(
                    name.clone(),
                    crate::resource_source::Resource::file(layer.clone(), name),
                );
            }
        }
        ResourceSource::indexed(files)
    }
    /// Retain the pack's parsed `filter.block` section for this generation.
    pub fn with_filter(mut self, filter: Vec<Filter>) -> Self {
        self.filter = filter;
        self
    }
    pub fn source(&self) -> &ResourceSource {
        &self.source
    }
    /// Every file path under `prefix` (relative to the pack root).
    pub fn list(&self, prefix: &str) -> Vec<String> {
        self.source.list(prefix)
    }
    /// The files of `<top>/<namespace>/<kind>/` for each kind, in the order
    /// `list("<top>/")` gives them. A folder pack walks only those folders:
    /// the built-in pack holds about 10,000 other data files.
    pub fn list_under(&self, top: &str, kinds: &[&str]) -> Vec<String> {
        self.source.list_under(top, kinds)
    }
    /// The files of `data/<namespace>/<kind>/` for each kind.
    pub fn list_kinds(&self, kinds: &[&str]) -> Vec<String> {
        self.list_under("data", kinds)
    }

    /// The files of `data/<namespace>/<folder>/` in every namespace, as
    /// `list` names them; a folder pack walks only those folders.
    pub fn list_folder(&self, folder: &str) -> Vec<String> {
        self.list_kinds(&[folder])
    }
    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.source.read(path).ok()
    }

    /// The `description` of `pack.mcmeta`, as a text component.
    pub fn description(&self) -> crate::text::Text {
        crate::pack_meta::component_root(&self.mcmeta()["pack"]["description"]).0
    }
    /// The `features.enabled` flag ids of `pack.mcmeta`
    /// (`FeatureFlagsMetadataSection.CODEC` -> `FeatureFlagRegistry.codec` -> `Identifier.CODEC`):
    /// the canonical feature flags the pack asks to turn on.
    pub fn features(&self) -> Vec<String> {
        let metadata = self.mcmeta();
        let Some(enabled) = metadata["features"]["enabled"].as_array() else {
            return Vec::new();
        };
        let mut flags = Vec::new();
        for entry in enabled {
            let Some(flag) = entry.as_str().and_then(mod_api::FeatureFlag::from_id) else {
                log::warn!(
                    "Ignoring invalid features.enabled metadata in pack '{}'",
                    self.id
                );
                return Vec::new();
            };
            let id = flag.id().to_owned();
            if !flags.contains(&id) {
                flags.push(id);
            }
        }
        flags
    }
    /// The `pack.mcmeta` as JSON, empty when it is missing or unreadable.
    fn mcmeta(&self) -> serde_json::Value {
        self.read("pack.mcmeta")
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
}
impl DataPack {
    /// A mod's passive data layer, without reading the archive's pack metadata.
    pub fn mod_layer(id: String, source: &crate::pack_meta::Source) -> std::io::Result<Self> {
        Ok(Self::from_source(id, source.open()?))
    }
    /// The built-in `vanilla` pack: the game's own data directory.
    pub fn builtin(root: &Path) -> Self {
        Self::from_source(
            "vanilla".into(),
            ResourceSource::directory(root.to_path_buf()),
        )
    }
}

/// The packs of one data load as one view, as vanilla's `MultiPackResourceManager` holds them:
/// layers lowest first, each a pack with its overlays already composed. A layer's `filter` hides
/// the matching names of the layers below it, never its own, when the view is built.
#[derive(Clone)]
pub struct DataPacks {
    /// Each layer's id and source, lowest first.
    layers: Vec<(String, ResourceSource)>,
    /// `data/` of every layer: the winner per name, and every copy lowest first.
    source: ResourceSource,
    /// The index in `layers` each copy of `source`'s stack of a name came from.
    owners: BTreeMap<String, Vec<usize>>,
}

impl Default for DataPacks {
    fn default() -> Self {
        Self::new(&[])
    }
}

impl DataPacks {
    /// The view over `layers`, lowest first.
    pub fn new(layers: &[&DataPack]) -> Self {
        let mut copies: BTreeMap<String, Vec<(usize, crate::resource_source::Resource)>> =
            BTreeMap::new();
        for (index, pack) in layers.iter().enumerate() {
            if !pack.filter.is_empty() {
                copies.retain(|name, _| {
                    let Some((namespace, path)) = name
                        .strip_prefix("data/")
                        .and_then(|rest| rest.split_once('/'))
                    else {
                        return true;
                    };
                    !pack
                        .filter
                        .iter()
                        .any(|filter| filter.matches(namespace, path))
                });
            }
            for name in pack.list("data/") {
                let resource =
                    crate::resource_source::Resource::file(pack.source.clone(), name.clone());
                copies.entry(name).or_default().push((index, resource));
            }
        }
        let mut files = BTreeMap::new();
        let mut stacks = BTreeMap::new();
        let mut owners = BTreeMap::new();
        for (name, found) in copies {
            let Some((_, winner)) = found.last() else {
                continue;
            };
            files.insert(name.clone(), winner.clone());
            owners.insert(
                name.clone(),
                found.iter().map(|(index, _)| *index).collect(),
            );
            stacks.insert(
                name,
                found.into_iter().map(|(_, resource)| resource).collect(),
            );
        }
        Self {
            layers: layers
                .iter()
                .map(|pack| (pack.id.clone(), pack.source.clone()))
                .collect(),
            source: ResourceSource::indexed_with_layers(files, stacks, None),
            owners,
        }
    }
    /// Each layer's id and source, lowest first.
    pub fn layers(&self) -> &[(String, ResourceSource)] {
        &self.layers
    }
    fn name(id: &Identifier) -> String {
        format!("data/{}/{}", id.namespace(), id.path())
    }
    /// `ResourceManager.getResource`: the winning copy's bytes.
    pub fn read(&self, id: &Identifier) -> io::Result<Vec<u8>> {
        self.source.read(&Self::name(id))
    }
    /// `ResourceManager.getResourceStack`: every copy, lowest layer first, with its layer's id.
    pub fn stack(&self, id: &Identifier) -> Vec<(&str, crate::resource_source::Resource)> {
        let name = Self::name(id);
        let Some(owners) = self.owners.get(&name) else {
            return Vec::new();
        };
        owners
            .iter()
            .zip(self.source.stack(&name))
            .map(|(&index, resource)| (self.layers[index].0.as_str(), resource))
            .collect()
    }
    /// `ResourceManager.listResources`: the ids `<namespace>:<folder>/<path><suffix>` of every
    /// namespace, in `Identifier` order. A path no identifier can spell is reported and ignored.
    pub fn list(&self, folder: &str, suffix: &str) -> Vec<Identifier> {
        let mut ids = Vec::new();
        for name in self.owners.keys() {
            let Some((namespace, path)) = name
                .strip_prefix("data/")
                .and_then(|rest| rest.split_once('/'))
            else {
                continue;
            };
            if !path
                .strip_prefix(folder)
                .is_some_and(|rest| rest.starts_with('/'))
                || !path.ends_with(suffix)
            {
                continue;
            }
            if sunrise_world::json_folder::valid_resource_path(namespace, path)
                && let Ok(id) = Identifier::from_parts(namespace, path)
            {
                ids.push(id);
            } else {
                log::warn!("Sunrise: invalid path {namespace}:{path} in pack, ignored");
            }
        }
        ids.sort();
        ids
    }
}

/// Opens the winning copy of `id` across `selected` (highest priority first) and `lower`,
/// stopping when a pack's `filter` hides the resource.
pub fn open_resource<'a>(
    selected: impl IntoIterator<Item = &'a DataPack>,
    lower: impl IntoIterator<Item = &'a ResourceSource>,
    id: &Identifier,
) -> io::Result<Option<Box<dyn Read + 'a>>> {
    let name = format!("data/{}/{}", id.namespace(), id.path());
    let Some(path) = ResourcePath::new(&name) else {
        return Ok(None);
    };
    for pack in selected {
        if let Some(reader) = pack.source.open_resource(path)? {
            return Ok(Some(reader));
        }
        if pack
            .filter
            .iter()
            .any(|filter| filter.matches(id.namespace(), id.path()))
        {
            log::warn!(
                "Resource {id} not found, but was filtered by pack {}",
                pack.id
            );
            return Ok(None);
        }
    }
    for source in lower {
        if let Some(reader) = source.open_resource(path)? {
            return Ok(Some(reader));
        }
    }
    Ok(None)
}

/// Compare repository IDs as `PackRepository`'s TreeMap compares its keys.
pub fn compare_ids(a: &str, b: &str) -> std::cmp::Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// The packs in `<world>/datapacks`, sorted by name.
pub fn discover(world: &Path) -> Vec<DataPack> {
    discover_folder(&world.join("datapacks"))
}
/// `FolderRepositorySource.loadPacks`: packs in Create World's temporary
/// folder or a world's data-pack directory, with required folder metadata.
pub fn discover_folder(dir: &Path) -> Vec<DataPack> {
    discover_folder_with(dir, &crate::pack_meta::DirectoryValidator::game())
}
/// [`discover_folder`] under an explicit `allowed_symlinks.txt` validator.
pub fn discover_folder_with(
    dir: &Path,
    validator: &crate::pack_meta::DirectoryValidator,
) -> Vec<DataPack> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut packs = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        // `PackDetector.detectPackResources`: a link, or a pack holding one, is
        // not a pack unless the validator allows the target; the id stays the
        // entry's name while the contents come from the target.
        let path = match crate::pack_meta::detected_pack_path(&e.path(), validator) {
            Ok(Some(path)) => path,
            Ok(None) => continue,
            Err(err) => {
                log::warn!(
                    "Failed to read properties of '{}', ignoring: {err}",
                    e.path().display()
                );
                continue;
            }
        };
        if path.is_dir()
            && let Some(meta) = folder_metadata(&path)
        {
            packs.push(
                DataPack::from_source(format!("file/{name}"), ResourceSource::directory(path))
                    .with_filter(filter_of(&meta)),
            );
        } else if path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().ends_with(".zip"))
            && let Ok(z) = Zip::open(&path)
            && z.contains("pack.mcmeta")
        {
            let filter = match z.read("pack.mcmeta").and_then(|bytes| {
                crate::pack_meta::first_json(&bytes).map_err(std::io::Error::other)
            }) {
                Ok(meta) => {
                    if !accepted_overlays(&meta, &name) {
                        continue;
                    }
                    filter_of(&meta)
                }
                Err(error) => {
                    log::error!("Failed to read the filter section from data pack {name}: {error}");
                    Vec::new()
                }
            };
            packs.push(
                DataPack::from_source(format!("file/{name}"), ResourceSource::opened_archive(z))
                    .with_filter(filter),
            );
        }
    }
    packs.sort_by(|a, b| compare_ids(&a.id, &b.id));
    packs
}
/// The resources root's built-in feature packs (`ServerPacksSource`
/// scanning the game's `data/minecraft/datapacks`): every folder with a
/// `Pack.readPackMetadata`: even its fallback codec requires a pack section
/// and description. Keep this folder gate separate from format compatibility
/// and the existing interpretation of the description component.
fn folder_metadata(path: &Path) -> Option<serde_json::Value> {
    let meta = std::fs::read(path.join("pack.mcmeta"))
        .ok()
        .and_then(|bytes| crate::pack_meta::parse_meta(&bytes))?;
    if meta["pack"]["description"].is_null() {
        return None;
    }
    accepted_overlays(&meta, &path.display().to_string()).then_some(meta)
}
/// `Pack.readPackMetadata` reads the `overlays` section through `OverlayMetadataSection`'s
/// codec: one that fails refuses the pack, which is logged and not loaded.
fn accepted_overlays(meta: &serde_json::Value, pack: &str) -> bool {
    match crate::pack_meta::validate_overlays(meta, sunrise_core::version::DATA_PACK_FORMAT) {
        Ok(()) => true,
        Err(error) => {
            log::warn!("Failed to read pack {pack} metadata: {error}");
            false
        }
    }
}
/// `pack.mcmeta`, a feature pack (`PackSource.FEATURE`). Off until a world
/// selects them.
pub fn feature_packs(resources: &Path) -> Vec<DataPack> {
    let Ok(entries) = std::fs::read_dir(resources.join("data").join("minecraft").join("datapacks"))
    else {
        return Vec::new();
    };
    let mut packs = Vec::new();
    for e in entries.flatten() {
        let path = e.path();
        if path.join("pack.mcmeta").exists() {
            let mut pack = DataPack::from_source(
                e.file_name().to_string_lossy().into_owned(),
                ResourceSource::directory(path),
            );
            let meta = pack.mcmeta();
            pack.filter = filter_of(&meta);
            let (description, description_style) =
                crate::pack_meta::component_root(&meta["pack"]["description"]);
            let name = format!("dataPack.{}.name", pack.id);
            pack.selection = Some(crate::pack_meta::PackInfo {
                id: pack.id.clone(),
                title: if crate::lang::has(&name) {
                    crate::text::Text::translate(&name, &[])
                } else {
                    crate::text::Text::literal(pack.id.clone())
                },
                description,
                description_style,
                source: crate::pack_meta::PackSource::Feature,
                compat: crate::pack_meta::compatibility_for(
                    &meta,
                    sunrise_core::version::DATA_PACK_FORMAT,
                ),
                icon: None,
                required: false,
                position: crate::pack_meta::Position::Top,
                fixed: false,
                hidden: false,
            });
            packs.push(pack);
        }
    }
    packs.sort_by(|a, b| compare_ids(&a.id, &b.id));
    packs
}
/// The packs of `<world>/datapacks` plus the resources root's feature packs:
/// the world's whole repository, as `PackRepository.reload` sees it.
pub fn discover_with_features(world_root: &Path, resources: &Path) -> Vec<DataPack> {
    discover_folder_with_features(&world_root.join("datapacks"), resources)
}
/// [`discover_with_features`] over a pack folder itself: Create World's `mcworld-` folder holds
/// its packs directly, as its Data Packs screen lists them.
pub fn discover_folder_with_features(dir: &Path, resources: &Path) -> Vec<DataPack> {
    let mut packs = discover_folder(dir);
    packs.extend(feature_packs(resources));
    packs.sort_by(|a, b| compare_ids(&a.id, &b.id));
    packs
}

/// Built-in and contributed feature packs offered as world experiments.
pub fn feature_repository(
    resources: &Path,
    additional: &[std::sync::Arc<DataPack>],
) -> Vec<DataPack> {
    let mut packs = feature_packs(resources);
    packs.extend(
        additional
            .iter()
            .filter(|pack| {
                pack.selection
                    .as_ref()
                    .is_some_and(|info| info.source == crate::pack_meta::PackSource::Feature)
            })
            .map(|pack| (**pack).clone()),
    );
    packs
}

// ---- Create World ------------------------------------------------------------

/// The folder of a game folder whose packs every new world there starts with: a modpack's profile
/// keeps the packs of its data-only jars here (`sunrise-port-pack`). Each world gets its own copy,
/// as Create World's packs do, so a world keeps working when the folder changes. Vanilla has no
/// such folder, and without one nothing here happens.
pub const NEW_WORLD_PACKS: &str = "new-world-datapacks";

/// Create World's temporary pack folder holding `game_dir`'s [`NEW_WORLD_PACKS`], and their ids,
/// to show them selected in the Data Packs screen. `None` when there are none.
pub fn seed_create_world(game_dir: &Path) -> Option<(PathBuf, Vec<String>)> {
    let from = game_dir.join(NEW_WORLD_PACKS);
    let ids: Vec<String> = discover_folder(&from).into_iter().map(|p| p.id).collect();
    if ids.is_empty() {
        return None;
    }
    let seeded = temp_folder()
        .map_err(|e| e.to_string())
        .and_then(|dir| copy(&from, &dir).map(|()| dir));
    match seeded {
        Ok(dir) => Some((dir, ids)),
        Err(e) => {
            log::warn!("Sunrise: data packs for new worlds: {e}");
            None
        }
    }
}

/// A world about to be created without Create World (`--world`, the control socket) gets
/// `game_dir`'s [`NEW_WORLD_PACKS`]. The game turns on every pack it finds in a world.
pub fn install_new_world_packs(game_dir: &Path, world: &Path) -> Result<(), String> {
    let from = game_dir.join(NEW_WORLD_PACKS);
    let exists = |name: &str| world.join(name).exists();
    if !from.is_dir() || exists("level.dat") || exists("level.dat_old") {
        return Ok(());
    }
    copy(&from, &world.join("datapacks"))
}

/// A new empty folder for Create World's packs
/// (`Files.createTempDirectory("mcworld-")`).
pub fn temp_folder() -> std::io::Result<PathBuf> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("mcworld-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
/// `PackSelectionScreen.onFilesDrop`'s `PackDetector`: a zip is a pack; a
/// folder is one when it holds a `pack.mcmeta`, or exactly one folder that
/// does (a folder zipped by the system, which adds a wrapper). Copied into
/// `dir`; false when `path` is no pack, or when part of it could not be copied.
pub fn copy_pack(path: &Path, dir: &Path) -> bool {
    fn copy_dir(from: &Path, at: &Path) -> bool {
        let mut whole = true;
        if let Err(e) = std::fs::create_dir_all(at) {
            log::warn!("Sunrise: {} is not made: {e}", at.display());
            whole = false;
        }
        for entry in std::fs::read_dir(from).into_iter().flatten().flatten() {
            let path = entry.path();
            let to = at.join(entry.file_name());
            let copied = if path.is_dir() {
                copy_dir(&path, &to)
            } else {
                match std::fs::copy(&path, &to) {
                    Ok(_) => true,
                    Err(e) => {
                        log::warn!(
                            "Sunrise: {} is not copied into the pack: {e}",
                            path.display()
                        );
                        false
                    }
                }
            };
            whole &= copied;
        }
        whole
    }
    let name = || {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    if path.is_file() {
        return path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
            && {
                let _ = std::fs::create_dir_all(dir);
                std::fs::copy(path, dir.join(name())).is_ok()
            };
    }
    if !path.is_dir() {
        return false;
    }
    if path.join("pack.mcmeta").is_file() {
        return copy_dir(path, &dir.join(name()));
    }
    let own: Vec<PathBuf> = std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join("pack.mcmeta").is_file())
        .collect();
    match own.as_slice() {
        [one] => copy_dir(one, &dir.join(one.file_name().unwrap_or_default())),
        _ => false,
    }
}
/// What's in a folder of packs, to notice packs added or removed while the
/// screen shows it (`PackSelectionScreen.Watcher`).
pub fn folder_stamp(dir: &Path) -> Vec<(String, Option<std::time::SystemTime>)> {
    let mut stamp: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| {
            let modified = e.metadata().and_then(|m| m.modified()).ok();
            (e.file_name().to_string_lossy().into_owned(), modified)
        })
        .collect();
    stamp.sort();
    stamp
}
/// Create World's packs into the new world before it opens
/// (`copyTempDataPackDirToNewWorld`): every pack of `dir` goes to
/// `<world>/datapacks`, and `datapacks.json` enables those `selected`
/// (lowest priority first) and disables the rest.
pub fn install(dir: &Path, world: &Path, selected: &[String]) -> Result<(), String> {
    let err = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
    let target = world.join("datapacks");
    std::fs::create_dir_all(&target).map_err(|e| err(&target, e))?;
    let mut disabled = Vec::new();
    for pack in crate::pack_meta::folder_meta(dir, sunrise_core::version::DATA_PACK_FORMAT) {
        let from = match &pack.source {
            crate::pack_meta::Source::Dir(from) | crate::pack_meta::Source::Zip(from) => from,
            crate::pack_meta::Source::Opened(_) => {
                return Err(format!(
                    "{}: an open resource layer cannot be installed as a file pack",
                    pack.id
                ));
            }
        };
        copy(from, &target.join(pack.id.trim_start_matches("file/")))?;
        if !selected.contains(&pack.id) {
            disabled.push(pack.id);
        }
    }
    if !selected.iter().any(|id| id == "vanilla") {
        disabled.push("vanilla".into());
    }
    let json = serde_json::json!({ "enabled": selected, "disabled": disabled });
    let file = world.join("datapacks.json");
    std::fs::write(&file, json.to_string()).map_err(|e| err(&file, e))
}
/// Re-Create World's packs from a saved world: a new folder holding a copy of
/// its `datapacks` (`CreateWorldScreen.createTempDataPackDirFromExistingWorld`; `None` when the
/// world has none), and the selection its `datapacks.json` makes, lowest
/// priority first (`WorldOpenFlows.recreateWorldData` keeps the world's `DataConfiguration`).
/// The built-in pack is selected unless the world turned it off.
pub fn for_recreate(world: &Path) -> (Option<PathBuf>, Vec<String>) {
    let packs = discover(world);
    // No flags here: the recreated world keeps the original's
    // `enabled_features`, and the flag-aware load re-adds the feature packs
    // it asks for (`for_recreate`'s selection only seeds the new world's
    // `datapacks.json`).
    let mut selected = enabled_ids(world, &packs, &[]);
    if !vanilla_disabled(world) {
        selected.insert(0, "vanilla".into());
    }
    if packs.is_empty() {
        return (None, selected);
    }
    let dir = temp_folder().ok().and_then(|dir| {
        if copy(&world.join("datapacks"), &dir).is_ok() {
            Some(dir)
        } else {
            let _ = std::fs::remove_dir_all(&dir);
            None
        }
    });
    (dir, selected)
}
/// Whether the world's `datapacks.json` lists the built-in `vanilla` pack as
/// disabled (`DataPackConfig` keeps it in `Disabled`).
pub fn vanilla_disabled(world: &Path) -> bool {
    std::fs::read(world.join("datapacks.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v["disabled"].as_array().cloned())
        .is_some_and(|a| a.iter().any(|id| id.as_str() == Some("vanilla")))
}
/// A file, or a folder and everything in it.
fn copy(from: &Path, to: &Path) -> Result<(), String> {
    let err = |e: std::io::Error| format!("{} -> {}: {e}", from.display(), to.display());
    if !from.is_dir() {
        return std::fs::copy(from, to).map(drop).map_err(err);
    }
    std::fs::create_dir_all(to).map_err(err)?;
    for e in std::fs::read_dir(from).map_err(err)?.flatten() {
        copy(&e.path(), &to.join(e.file_name()))?;
    }
    Ok(())
}

// ---- functions ---------------------------------------------------------------

/// A `.mcfunction`: plain command lines and macro lines.
#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    pub id: String,
    pub lines: Vec<FunctionLine>,
    /// Macro parameter names in order of first use.
    pub parameters: Vec<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum FunctionLine {
    Command(String),
    /// A `$` line: text with `$(name)` placeholders.
    Macro(String),
}
impl Function {
    pub fn is_macro(&self) -> bool {
        !self.parameters.is_empty()
    }
    /// `MacroFunction.instantiate`: the command lines with the arguments
    /// substituted. The error is the translation key and its arguments.
    pub fn instantiate(
        &self,
        arguments: Option<&Tag>,
    ) -> Result<Vec<String>, (String, Vec<String>)> {
        let mut values: BTreeMap<&str, String> = BTreeMap::new();
        if self.is_macro() {
            let Some(args) = arguments else {
                return Err((
                    "commands.function.error.missing_arguments".into(),
                    vec![self.id.clone()],
                ));
            };
            for p in &self.parameters {
                let Some(v) = args.get(p) else {
                    return Err((
                        "commands.function.error.missing_argument".into(),
                        vec![self.id.clone(), p.clone()],
                    ));
                };
                values.insert(p, stringify(v));
            }
        }
        Ok(self
            .lines
            .iter()
            .map(|l| match l {
                FunctionLine::Command(c) => c.clone(),
                FunctionLine::Macro(t) => substitute(t, &values),
            })
            .collect())
    }
}
/// `MacroFunction.stringify`.
fn stringify(tag: &Tag) -> String {
    match tag {
        Tag::Float(v) => decimal(f64::from(f32::from_bits(*v))),
        Tag::Double(v) => decimal(f64::from_bits(*v)),
        Tag::Byte(v) => v.to_string(),
        Tag::Short(v) => v.to_string(),
        Tag::Long(v) => v.to_string(),
        Tag::String(s) => s.to_string_lossy(),
        other => crate::snbt::to_snbt(other),
    }
}
/// `DecimalFormat("#")` with up to 15 fraction digits.
fn decimal(v: f64) -> String {
    use sunrise_core::java_number::{DecimalPattern, format_decimal};
    // FloatTag.valueOf and DoubleTag.valueOf collapse signed zero. A negative
    // nonzero argument that later rounds to zero must still come out as -0.
    format_decimal(if v == 0.0 { 0.0 } else { v }, DecimalPattern::Macro)
}
fn substitute(template: &str, values: &BTreeMap<&str, String>) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(i) = rest.find("$(") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        match after.find(')') {
            Some(j) => {
                if let Some(v) = values.get(&after[..j]) {
                    out.push_str(v);
                }
                rest = &after[j + 1..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}
fn macro_parameters(line: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(i) = rest.find("$(") {
        let after = &rest[i + 2..];
        let Some(j) = after.find(')') else {
            return Err(format!("Unterminated macro variable in '{line}'"));
        };
        let name = &after[..j];
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!("Invalid macro variable name '{name}' in '{line}'"));
        }
        out.push(name.to_owned());
        rest = &after[j + 1..];
    }
    Ok(out)
}
/// `CommandFunction.fromLines`: comments, blank lines, `\` continuation and
/// `$` macro lines. Command syntax is checked by the caller.
pub fn parse_function(id: &str, text: &str) -> Result<Function, String> {
    let raw: Vec<&str> = text.lines().collect();
    let mut lines = Vec::new();
    let mut parameters: Vec<String> = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let number = i + 1;
        let mut line = raw[i].trim().to_owned();
        if line.ends_with('\\') {
            loop {
                i += 1;
                if i == raw.len() {
                    return Err("Line continuation at end of file".into());
                }
                line.pop();
                line.push_str(raw[i].trim());
                if !line.ends_with('\\') {
                    break;
                }
            }
        }
        i += 1;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('/') {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+'))
                .collect();
            return Err(if rest.starts_with('/') {
                format!(
                    "Unknown or invalid command '{line}' on line {number} (if you intended to make a comment, use '#' not '//')"
                )
            } else {
                format!(
                    "Unknown or invalid command '{line}' on line {number} (did you mean '{name}'? Do not use a preceding forwards slash.)"
                )
            });
        }
        if let Some(m) = line.strip_prefix('$') {
            let names = macro_parameters(m)?;
            // `StringTemplate.fromString`: a `$` line with no `$(…)` is not a
            // macro, and fails the whole file rather than running unchanged.
            if names.is_empty() {
                return Err(format!(
                    "Can't parse function line {number}: '{m}': No variables in macro"
                ));
            }
            for p in names {
                if !parameters.contains(&p) {
                    parameters.push(p);
                }
            }
            lines.push(FunctionLine::Macro(m.to_owned()));
        } else {
            lines.push(FunctionLine::Command(line));
        }
    }
    Ok(Function {
        id: id.to_owned(),
        lines,
        parameters,
    })
}

/// The data of every enabled pack: functions, function tags, predicates.
#[derive(Default)]
pub struct DataStore {
    pub functions: BTreeMap<String, Function>,
    /// Function tag id -> entries (`ns:f`, `#ns:tag`), with `required` flags.
    pub function_tags: BTreeMap<String, Vec<TagEntryOf>>,
    pub predicates: BTreeMap<String, serde_json::Value>,
    /// `item_modifier` files, for `/item modify`.
    pub item_modifiers: BTreeMap<String, serde_json::Value>,
    /// `slot_source` definitions, evaluated against a command's live accessor.
    pub slot_sources: BTreeMap<String, serde_json::Value>,
    /// Slot-source tags belong to the same loaded snapshot as their definitions.
    pub slot_source_tags: BTreeMap<String, Vec<TagEntryOf>>,
    /// The content tables' raw files from the packs after the built-in one:
    /// Server JSON resources (recipes, loot, enchantments, songs, instruments,
    /// predicates, modifiers, advancements, registries, world clocks, data maps and tags), by
    /// `<kind-dir>/<ns>:<path>`, each id's files in pack order. The tables
    /// merge them over what they read from the resource root.
    pub files: BTreeMap<String, Vec<Vec<u8>>>,
    /// All copies of the recipe-priority resource in pack order, with their original source.
    pub priority_layers: Vec<RecipePriorityLayer>,
    /// Failed reads of the highest-priority resources, keyed like `files`.
    pub file_errors: BTreeMap<String, String>,
    /// Functions that failed to parse (id, message), for logging.
    pub errors: Vec<(String, String)>,
}
/// `<kind-dir>/<ns>:<path>` for a pack file the content tables read:
/// `recipe/`, `recipe_filter/`, `loot_table/`, `advancement/`, `trade_set/`,
/// `villager_trade/`, `dialog/`, `damage_type/`, `dimension_type/`, `chat_type/`, `data_maps/`,
/// `test_environment/`, `test_instance/`, enchantments, enchantment providers, songs,
/// instruments, painting variants, banner patterns, predicates, item modifiers, `world_clock/`,
/// `worldgen/biome/` or `tags/<sub>/`, where the id keeps the subpath for nested tag kinds.
fn content_id(file: &str) -> Option<String> {
    let rest = file.strip_prefix("data/")?;
    let (ns, rest) = rest.split_once('/')?;
    let (kind, rest) = rest.split_once('/')?;
    let kind = match kind {
        "recipe"
        | "recipe_filter"
        | "loot_table"
        | "advancement"
        | "trade_set"
        | "villager_trade"
        | "dialog"
        | "damage_type"
        | "dimension_type"
        | "chat_type"
        | "trim_pattern"
        | "trim_material"
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
        | "data_maps"
        | "enchantment"
        | "enchantment_provider"
        | "jukebox_song"
        | "instrument"
        | "painting_variant"
        | "banner_pattern"
        | "predicate"
        | "item_modifier"
        | "test_environment"
        | "test_instance"
        | "slot_source"
        | "world_clock" => kind,
        "worldgen" => {
            return resource_id(file, "worldgen/biome", ".json")
                .map(|id| format!("worldgen/biome/{id}"));
        }
        // A tag id is `{ns}:{path-under-tags}` (`tags/worldgen/biome/
        // test:sub/mytag`), the whole path after the namespace, so nested
        // registries keep their directory (`TagLoader`'s id shape).
        "tags" => {
            return Some(format!("tags/{ns}:{}", rest.strip_suffix(".json")?));
        }
        kind if crate::providers::NUMBER_PROVIDER_KINDS.contains(&kind) => kind,
        _ => return None,
    };
    Some(format!("{kind}/{ns}:{}", rest.strip_suffix(".json")?))
}
/// `ns:path` for a file `data/<ns>/<kind>/<path>.<ext>`.
fn resource_id(file: &str, kind: &str, ext: &str) -> Option<String> {
    let rest = file.strip_prefix("data/")?;
    let (ns, rest) = rest.split_once('/')?;
    let path = rest
        .strip_prefix(kind)?
        .strip_prefix('/')?
        .strip_suffix(ext)?;
    Some(format!("{ns}:{path}"))
}
/// The winning pack id and bytes at each physical resource path.
pub type RegistryFiles = BTreeMap<String, (String, Vec<u8>)>;

/// Like `ResourceManagerRegistryLoadTask.load`, read only the winning files for the
/// registry keys in `registries`. `packs` is lowest priority first. A file under
/// more than one registry directory still has only one winning source. An unreadable
/// winner fails the load and identifies its pack and file.
pub fn registry_files(
    packs: &[&DataPack],
    registries: &[&str],
) -> Result<RegistryFiles, Box<mod_api::DatapackLoadError>> {
    let directories: Vec<_> = registries
        .iter()
        .filter_map(|&key| {
            let directory = datapack_registry_directory(key)?;
            // PathPackResources.listResources rejects an undecomposable directory
            // on its own. Do not let it invalidate the combined filesystem walk.
            if !sunrise_assets::resource_source::stays_inside(&directory) {
                log::error!("Invalid datapack directory {directory:?} for registry {key}");
                return None;
            }
            Some((key, directory))
        })
        .collect();
    let mut kinds: Vec<_> = directories
        .iter()
        .map(|(_, directory)| directory.as_ref())
        .collect();
    if kinds.is_empty() {
        return Ok(BTreeMap::new());
    }
    kinds.sort_unstable();
    kinds.dedup();
    let mut sources: BTreeMap<String, _> = BTreeMap::new();
    for pack in packs {
        if !pack.filter.is_empty() {
            sources.retain(|file, _| {
                let (namespace, path) = file
                    .strip_prefix("data/")
                    .and_then(|path| path.split_once('/'))
                    .expect("paths came from registry directories");
                !pack
                    .filter
                    .iter()
                    .any(|filter| filter.matches(namespace, path))
            });
        }
        for file in pack.list_kinds(&kinds) {
            // The first match is only for I/O diagnostics. Each registry interprets
            // the winner independently when it decodes its entries.
            if let Some((registry, entry)) = directories.iter().find_map(|(key, directory)| {
                datapack_entry_id(directory, &file).map(|entry| (*key, entry))
            }) {
                sources.insert(file, (pack, registry, entry));
            }
        }
    }
    sources
        .into_iter()
        .map(|(file, (pack, registry, entry))| {
            let bytes = pack.source.read(&file).map_err(|error| {
                Box::new(mod_api::DatapackLoadError {
                    registry: registry.to_owned(),
                    entry,
                    file: file.clone(),
                    pack: pack.id.clone(),
                    reason: mod_api::DatapackLoadReason::ReadFile(error.to_string()),
                })
            })?;
            Ok((file, (pack.id.clone(), bytes)))
        })
        .collect()
}

/// Read the singleton at NeoForge's resource location. An absent file is not a layer;
/// I/O errors on a supplied file retain the identity used by the recipe loader's warning.
pub fn read_recipe_priority_layer(
    pack: &str,
    source: &ResourceSource,
) -> Option<RecipePriorityLayer> {
    let path = sunrise_mod_package::content::RECIPE_PRIORITIES_PATH;
    let contents = match source.read(path) {
        Ok(bytes) => Ok(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !source.contains(path) => {
            return None;
        }
        Err(error) => Err(error.to_string()),
    };
    Some(RecipePriorityLayer {
        pack: pack.to_owned(),
        path,
        contents,
    })
}

impl DataStore {
    /// Every function, tag, predicate and content file of the merged view, as the
    /// reload listeners read them: each name's copies from the lowest layer up.
    pub fn load(data: &DataPacks) -> Self {
        let kinds: Vec<_> = [
            "advancement",
            "banner_pattern",
            "cat_sound_variant",
            "cat_variant",
            "chat_type",
            "chicken_sound_variant",
            "chicken_variant",
            "cow_sound_variant",
            "cow_variant",
            "damage_type",
            "data_maps",
            "dialog",
            "dimension_type",
            "enchantment",
            "enchantment_provider",
            "frog_variant",
            "function",
            "instrument",
            "item_modifier",
            "jukebox_song",
            "loot_table",
            "painting_variant",
            "pig_sound_variant",
            "pig_variant",
            "predicate",
            "recipe",
            "recipe_filter",
            "slot_source",
            "tags",
            "test_environment",
            "test_instance",
            "trade_set",
            "trim_material",
            "trim_pattern",
            "villager_trade",
            "wolf_sound_variant",
            "wolf_variant",
            "world_clock",
            "worldgen/biome",
            "zombie_nautilus_variant",
        ]
        .into_iter()
        .chain(crate::providers::NUMBER_PROVIDER_KINDS)
        .collect();
        let mut s = Self::default();
        for (layer, source) in data.layers() {
            // The prepared root supplies the baseline to the recipe loader directly.
            if layer.as_str() != "vanilla" {
                let sources = source.named_sources();
                if sources.is_empty() {
                    s.priority_layers
                        .extend(read_recipe_priority_layer(layer, source));
                } else {
                    s.priority_layers.extend(
                        sources
                            .iter()
                            .filter_map(|(id, source)| read_recipe_priority_layer(id, source)),
                    );
                }
            }
        }
        // `tags` walks the whole tags tree, so a name can be listed by more than one kind:
        // handle every name once.
        let listed: std::collections::BTreeSet<Identifier> =
            kinds.iter().flat_map(|kind| data.list(kind, "")).collect();
        for name in listed {
            let f = format!("data/{}/{}", name.namespace(), name.path());
            for (layer, resource) in data.stack(&name) {
                let builtin = layer == "vanilla";
                // Preserve JSON stacks independently of the command views
                // below. World-table readers evaluate the winning resource's
                // conditions using their captured server inventory.
                if !builtin && let Some(id) = content_id(&f) {
                    match resource.read() {
                        Ok(bytes) => {
                            s.file_errors.remove(&id);
                            s.files.entry(id).or_default().push(bytes);
                        }
                        Err(error) => {
                            s.file_errors.insert(id, format!("{layer}: {error}"));
                        }
                    }
                }
                if let Some(id) = resource_id(&f, "function", ".mcfunction")
                    && let Some(bytes) = resource.read().ok()
                {
                    match parse_function(&id, &String::from_utf8_lossy(&bytes)) {
                        Ok(func) => {
                            s.functions.insert(id, func);
                        }
                        Err(e) => s.errors.push((id, e)),
                    }
                } else if let Some(id) = resource_id(&f, "tags/function", ".json")
                    && let Some(bytes) = resource.read().ok()
                    && let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes)
                {
                    let mut entries = tag_entries(&v);
                    if let Some(removes) = v["remove"].as_array() {
                        entries.extend(
                            tag_entries(&serde_json::json!({ "values": removes }))
                                .into_iter()
                                .map(|(id, required, _)| (id, required, true)),
                        );
                    }
                    let slot = s.function_tags.entry(id).or_default();
                    if v["replace"].as_bool() == Some(true) {
                        slot.clear();
                    }
                    slot.extend(entries);
                } else if let Some(id) = resource_id(&f, "tags/slot_source", ".json")
                    && let Some(bytes) = resource.read().ok()
                    && let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes)
                {
                    let mut entries = tag_entries(&v);
                    if let Some(removes) = v["remove"].as_array() {
                        entries.extend(
                            tag_entries(&serde_json::json!({ "values": removes }))
                                .into_iter()
                                .map(|(id, required, _)| (id, required, true)),
                        );
                    }
                    let slot = s.slot_source_tags.entry(id).or_default();
                    if v["replace"].as_bool() == Some(true) {
                        slot.clear();
                    }
                    slot.extend(entries);
                }
            }
        }
        s
    }
    /// `ServerFunctionLibrary.getTag`: only valid function tags expand
    /// (nested tags included, each function once).
    pub fn tag_functions(&self, id: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen_fns = std::collections::HashSet::new();
        let mut seen_tags = std::collections::HashSet::new();
        let invalid = self.invalid_function_tags(id);
        self.expand(id, &mut out, &mut seen_fns, &mut seen_tags, &invalid);
        out
    }
    /// `TagLoader.tryBuildTag`: missing required members invalidate their tag and
    /// required parents. Optional invalid children contribute nothing.
    /// Resolve against the current accepted function map, without caching a
    /// candidate store or changing the existing expansion's cycle handling.
    fn invalid_function_tags<'a>(&'a self, id: &'a str) -> std::collections::HashSet<&'a str> {
        let mut invalid = std::collections::HashSet::new();
        let mut parents: std::collections::HashMap<&str, Vec<&str>> =
            std::collections::HashMap::new();
        let mut seen = std::collections::HashSet::new();
        let mut pending = vec![id];
        while let Some(tag) = pending.pop() {
            if !seen.insert(tag) {
                continue;
            }
            let Some(entries) = self.function_tags.get(tag) else {
                invalid.insert(tag);
                continue;
            };
            for (entry, required, remove) in entries {
                if *remove {
                    continue;
                }
                if let Some(child) = entry.strip_prefix('#') {
                    pending.push(child);
                    if *required {
                        parents.entry(child).or_default().push(tag);
                    }
                } else if *required && !self.functions.contains_key(entry) {
                    invalid.insert(tag);
                }
            }
        }
        pending.extend(invalid.iter().copied());
        while let Some(child) = pending.pop() {
            for &parent in parents.get(child).into_iter().flatten() {
                if invalid.insert(parent) {
                    pending.push(parent);
                }
            }
        }
        invalid
    }
    /// Depth-first walk keeping first-seen order. Each tag is expanded at most
    /// once, so reference cycles end and nesting depth is not capped.
    fn expand<'a>(
        &'a self,
        id: &'a str,
        out: &mut Vec<String>,
        seen_fns: &mut std::collections::HashSet<&'a str>,
        seen_tags: &mut std::collections::HashSet<&'a str>,
        invalid: &std::collections::HashSet<&'a str>,
    ) {
        if invalid.contains(id) || !seen_tags.insert(id) {
            return;
        }
        let Some(entries) = self.function_tags.get(id) else {
            return;
        };
        for (e, required, remove) in entries {
            // A removal (a `remove` list's entry, after the file's values)
            // takes an id or a tag's members out of the set built so far; it
            // is optional: an unknown id removes nothing and never
            // invalidates the tag.
            if *remove {
                match e.strip_prefix('#') {
                    Some(inner) => {
                        let mut members = Vec::new();
                        self.expand(
                            inner,
                            &mut members,
                            &mut Default::default(),
                            seen_tags,
                            invalid,
                        );
                        out.retain(|f| !members.contains(f));
                        seen_fns.retain(|f| !members.iter().any(|member| member.as_str() == *f));
                    }
                    None => {
                        out.retain(|f| f.as_str() != e.as_str());
                        seen_fns.remove(e.as_str());
                    }
                }
                continue;
            }
            let _ = required;
            match e.strip_prefix('#') {
                Some(inner) => {
                    let mut members = Vec::new();
                    let mut member_ids = Default::default();
                    self.expand(inner, &mut members, &mut member_ids, seen_tags, invalid);
                    for member in members {
                        if let Some(&id) = member_ids.get(member.as_str())
                            && seen_fns.insert(id)
                        {
                            out.push(member);
                        }
                    }
                }
                None => {
                    if self.functions.contains_key(e) && seen_fns.insert(e) {
                        out.push(e.clone());
                    }
                }
            }
        }
        seen_tags.remove(id);
    }
}
fn normalize(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}
/// An entry of a tag file's `values` or `remove`: its id, whether it is
/// required, and whether it removes (`TagLoader.EntryWithSource.remove`).
type TagEntryOf = (String, bool, bool);

fn tag_entries(v: &serde_json::Value) -> Vec<TagEntryOf> {
    v["values"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| match e {
            serde_json::Value::String(s) => Some((hash_normalize(s), true, false)),
            serde_json::Value::Object(o) => Some((
                hash_normalize(o.get("id")?.as_str()?),
                o.get("required")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true),
                o.get("remove").and_then(serde_json::Value::as_bool) == Some(true),
            )),
            _ => None,
        })
        .collect()
}
fn hash_normalize(s: &str) -> String {
    match s.strip_prefix('#') {
        Some(rest) => format!("#{}", normalize(rest)),
        None => normalize(s),
    }
}

/// Whether every flag `pack` asks for (`Pack.getRequestedFeatures`) is among
/// the world's `enabled_features`.
fn features_enabled(pack: &DataPack, enabled_features: &[String]) -> bool {
    pack.features()
        .iter()
        .all(|f| enabled_features.iter().any(|e| e == f))
}

/// The ids of `packs` (a world's, from `discover`) that are enabled, lowest
/// priority first: those `<world>/datapacks.json` lists as enabled, in its
/// order, then every pack it lists neither way, in name order.
///
/// `enabled_features` are the world's feature flags
/// (`WorldDataConfiguration.enabledFeatures`): a pack a pack file selected
/// while asking for missing features comes off
/// (`MinecraftServer.configurePackRepository`), and a feature pack whose
/// features the world enables is forced on however the world's selection
/// feels about it (`MinecraftServer.enableForcedFeaturePacks`).
pub fn enabled_ids(world: &Path, packs: &[DataPack], enabled_features: &[String]) -> Vec<String> {
    let saved: Option<serde_json::Value> = std::fs::read(world.join("datapacks.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    let list = |key: &str| -> Vec<String> {
        saved
            .as_ref()
            .and_then(|v| v[key].as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    let disabled = list("disabled");
    let mut enabled: Vec<String> = list("enabled")
        .into_iter()
        .filter(|id| packs.iter().any(|p| &p.id == id))
        .filter(|id| {
            packs
                .iter()
                .find(|p| &p.id == id)
                .is_none_or(|p| features_enabled(p, enabled_features))
        })
        .collect();
    for p in packs {
        // A pack nobody mentions comes on (`PackRepository.reload` selects
        // every new pack), but a feature pack stays off until the world
        // enabled its flag (`PackSource.FEATURE` starts unselected).
        if !enabled.contains(&p.id)
            && !disabled.contains(&p.id)
            && p.id.starts_with("file/")
            && features_enabled(p, enabled_features)
        {
            enabled.push(p.id.clone());
        }
    }
    for p in packs {
        if p.is_feature()
            && !p.features().is_empty()
            && features_enabled(p, enabled_features)
            && !enabled.contains(&p.id)
        {
            enabled.push(p.id.clone());
        }
    }
    selected_ids(packs.iter(), &enabled)
}

/// Rebuild a data-pack selection, preserving its order and inserting required packs.
pub fn selected_ids<'a>(
    packs: impl Iterator<Item = &'a DataPack>,
    selected: &[String],
) -> Vec<String> {
    // Existing selections keep their order; mandatory additions use the same
    // insertion rules as the pack screen.
    let available = packs
        .map(|pack| {
            pack.selection.as_ref().map_or_else(
                || crate::pack_meta::Selectable::optional(&pack.id, true),
                crate::pack_meta::PackInfo::selectable,
            )
        })
        .collect();
    crate::pack_meta::SelectionModel::new(available, selected)
        .selected
        .into_iter()
        .rev()
        .map(|pack| pack.id)
        .collect()
}

/// The level stems of the merged view (`data/<namespace>/dimension/<path>.json`, the winning
/// copy of each name): the id `namespace:path` and the parsed JSON. This is
/// `context.datapackDimensions()` of `WorldOpenFlows.loadWorldStem`.
pub fn dimension_stems(data: &DataPacks) -> BTreeMap<String, serde_json::Value> {
    let mut out = BTreeMap::new();
    for name in data.list("dimension", ".json") {
        let Some(path) = name
            .path()
            .strip_prefix("dimension/")
            .and_then(|p| p.strip_suffix(".json"))
        else {
            continue;
        };
        // An id that is no safe folder name (`..`, an empty segment) would open a level
        // outside the world's `dimensions` folder.
        let id = format!("{}:{path}", name.namespace());
        if !sunrise_core::level_id::valid_level_id(&id) {
            continue;
        }
        if let Some(json) = data
            .read(&name)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
        {
            out.insert(id, json);
        }
    }
    out
}

// ---- worldgen ----------------------------------------------------------------

/// The worldgen entries of the merged view `data` (every layer but the prepared root,
/// `vanilla`: the prepared bundles are it) for the chunk generator, and what it leaves out, for
/// the log. Every biome a pack gives is kept, as `RegistryDataLoader` loads them. A file whose
/// `neoforge:conditions` fail is left out; one that fails to decode is recorded in `conditions`.
pub fn worldgen_overrides(
    data: &DataPacks,
    conditions: &sunrise_world::conditions::LoadConditions,
) -> (WorldgenOverrides, Vec<String>) {
    worldgen_overrides_over(overrides::mod_dimensions(), data, conditions)
}

/// [`worldgen_overrides`] over the levels `mods` registered, which sit below
/// every pack.
pub fn worldgen_overrides_over(
    mods: &[overrides::ModDimension],
    data: &DataPacks,
    conditions: &sunrise_world::conditions::LoadConditions,
) -> (WorldgenOverrides, Vec<String>) {
    let mut out = WorldgenOverrides::default();
    for dimension in mods {
        out.add_mod_dimension(dimension);
    }
    let mut not_read: BTreeMap<String, usize> = BTreeMap::new();
    let mut warnings = Vec::new();
    for folder in overrides::PACK_KINDS {
        for id in data.list(folder, "") {
            let file = DataPacks::name(&id);
            // `getResourceStack`: every copy, lowest layer first, so tags merge in layer order.
            for (layer, resource) in data.stack(&id) {
                if layer == "vanilla" {
                    continue;
                }
                if let Some(template) = overrides::template_id(&file) {
                    out.set_template(template, resource);
                    continue;
                }
                let read = || String::from_utf8(resource.read().ok()?).ok();
                match out.add_file(layer, &file, read) {
                    PackFile::Read | PackFile::Other => {}
                    PackFile::NotRead(kind) => {
                        *not_read.entry(format!("{layer}: {kind}")).or_default() += 1
                    }
                    PackFile::Unreadable => warnings.push(format!("{layer}: {file}: unreadable")),
                }
            }
        }
    }
    // After every pack, so a failing higher file never uncovers a lower one. A condition that
    // fails to decode is recorded in `conditions`: the owning registry load refuses on
    // `check_errors`, as `ConditionalOps` fails the registry's decode.
    out.admit_conditions(|file, kind, id| conditions.allows_id(file, kind, id));
    for (kind, files) in not_read {
        warnings.push(format!("{kind}: {files} files the generator does not read"));
    }
    (out, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn memory_pack(id: &str, files: &[(&str, &str)]) -> DataPack {
        use crate::resource_source::Resource;
        DataPack::from_source(
            id.into(),
            ResourceSource::indexed(
                files
                    .iter()
                    .map(|(name, text)| {
                        (name.to_string(), Resource::bytes(text.as_bytes().to_vec()))
                    })
                    .collect(),
            ),
        )
    }
    fn function(name: &str) -> Identifier {
        Identifier::parse(&format!("test:function/{name}.mcfunction")).unwrap()
    }
    fn texts(data: &DataPacks, id: &Identifier) -> Vec<(String, String)> {
        data.stack(id)
            .into_iter()
            .map(|(layer, resource)| {
                (
                    layer.to_owned(),
                    String::from_utf8(resource.read().unwrap()).unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn a_layers_filter_hides_the_names_below_it_but_not_its_own() {
        let low = memory_pack(
            "low",
            &[
                ("data/test/function/x.mcfunction", "low x"),
                ("data/test/function/y.mcfunction", "low y"),
                ("data/test/function/z.mcfunction", "low z"),
            ],
        );
        let meta = serde_json::json!({"filter": {"block": [
            {"namespace": "^test$", "path": "^function/[xz][.]mcfunction$"},
        ]}});
        let high = memory_pack("high", &[("data/test/function/x.mcfunction", "high x")])
            .with_filter(filter_of(&meta));
        let data = DataPacks::new(&[&low, &high]);
        assert_eq!(
            texts(&data, &function("x")),
            [("high".to_owned(), "high x".to_owned())]
        );
        assert_eq!(data.read(&function("x")).unwrap(), b"high x");
        assert!(texts(&data, &function("z")).is_empty());
        assert!(data.read(&function("z")).is_err());
        assert_eq!(
            texts(&data, &function("y")),
            [("low".to_owned(), "low y".to_owned())]
        );
        let listed: Vec<_> = data
            .list("function", ".mcfunction")
            .iter()
            .map(Identifier::to_string)
            .collect();
        assert_eq!(
            listed,
            ["test:function/x.mcfunction", "test:function/y.mcfunction"]
        );
    }

    #[test]
    fn the_stack_runs_from_the_prepared_root_through_mods_and_contributions_to_the_packs() {
        let layers = [
            memory_pack("vanilla", &[("data/test/function/x.mcfunction", "vanilla")]),
            memory_pack("mod_a", &[("data/test/function/x.mcfunction", "mod")]),
            memory_pack(
                "mod_files",
                &[("data/test/function/x.mcfunction", "contribution")],
            ),
            memory_pack(
                "file/a",
                &[("data/test/function/x.mcfunction", "first pack")],
            ),
            memory_pack(
                "file/b",
                &[("data/test/function/x.mcfunction", "second pack")],
            ),
        ];
        let data = DataPacks::new(&layers.iter().collect::<Vec<_>>());
        let expected = [
            ("vanilla", "vanilla"),
            ("mod_a", "mod"),
            ("mod_files", "contribution"),
            ("file/a", "first pack"),
            ("file/b", "second pack"),
        ]
        .map(|(layer, text)| (layer.to_owned(), text.to_owned()));
        assert_eq!(texts(&data, &function("x")), expected);
        assert_eq!(data.read(&function("x")).unwrap(), b"second pack");
    }

    #[test]
    fn a_pack_written_as_vanilla_writes_one_reads_through_the_view() {
        let scratch = tempfile::tempdir().unwrap();
        let low = scratch.path().join("a_low");
        let high = scratch.path().join("b_high");
        for (pack, files) in [
            (
                &low,
                [
                    ("data/test/function/gone.mcfunction", "low gone"),
                    ("data/test/function/kept.mcfunction", "low kept"),
                    ("data/test/function/over.mcfunction", "low over"),
                ]
                .as_slice(),
            ),
            (
                &high,
                [
                    ("data/test/function/over.mcfunction", "high root"),
                    ("fmt/data/test/function/over.mcfunction", "high overlay"),
                ]
                .as_slice(),
            ),
        ] {
            for (name, text) in files {
                let path = pack.join(name);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, text).unwrap();
            }
        }
        fs::write(
            low.join("pack.mcmeta"),
            br#"{"pack":{"description":"low","min_format":[121,0],"max_format":[121,0]}}"#,
        )
        .unwrap();
        fs::write(
            high.join("pack.mcmeta"),
            br#"{
                "pack": {"description": "high", "min_format": [121, 0], "max_format": [121, 0]},
                "overlays": {"entries": [
                    {"directory": "fmt", "min_format": [121, 0], "max_format": [121, 0]}
                ]},
                "filter": {"block": [{"namespace": "^test$", "path": "^function/gone[.]mcfunction$"}]}
            }"#,
        )
        .unwrap();
        let mut packs = discover_folder(scratch.path());
        packs.sort_by(|a, b| compare_ids(&a.id, &b.id));
        let data = DataPacks::new(&packs.iter().collect::<Vec<_>>());
        assert!(data.stack(&function("gone")).is_empty());
        assert_eq!(
            texts(&data, &function("kept")),
            [("file/a_low".to_owned(), "low kept".to_owned())]
        );
        assert_eq!(data.read(&function("over")).unwrap(), b"high overlay");
        let layers: Vec<_> = data
            .stack(&function("over"))
            .into_iter()
            .map(|(layer, _)| layer.to_owned())
            .collect();
        assert_eq!(layers, ["file/a_low", "file/b_high"]);
    }

    #[test]
    fn selected_data_pack_filter_hides_lower_files_but_not_its_own() {
        use crate::resource_source::Resource;
        use std::io::Write;
        let scratch = tempfile::tempdir().unwrap();
        let low = scratch.path().join("low");
        let high = scratch.path().join("high");
        for pack in [&low, &high] {
            fs::create_dir_all(pack.join("data/demo/value")).unwrap();
        }
        fs::write(
            low.join("pack.mcmeta"),
            br#"{"pack":{"description":"low"}}"#,
        )
        .unwrap();
        let meta = serde_json::json!({
            "pack": {"description": "high"},
            "filter": {"block": [
                {"namespace": "^demo$", "path": "^value/"},
                {"namespace": "^minecraft$", "path": "^test[.]txt$"},
            ]},
        })
        .to_string();
        let own = "data/demo/value/own.json";
        fs::write(high.join("pack.mcmeta"), &meta).unwrap();
        fs::write(low.join("data/demo/value/hidden.json"), b"lower world pack").unwrap();
        fs::write(high.join(own), b"higher world pack").unwrap();
        let content = ResourceSource::indexed(BTreeMap::from([
            (
                "data/demo/value/mod.json".into(),
                Resource::bytes(b"mod pack".to_vec()),
            ),
            (
                "data/demo/test.txt".into(),
                Resource::bytes(b"unfiltered".to_vec()),
            ),
        ]));
        let vanilla = ResourceSource::indexed(BTreeMap::from([(
            "data/minecraft/test.txt".into(),
            Resource::bytes(b"vanilla pack".to_vec()),
        )]));
        let read = |selected: &[&DataPack], id: &str| {
            let mut reader = open_resource(
                selected.iter().copied(),
                [&content, &vanilla],
                &Identifier::parse(id).unwrap(),
            )
            .unwrap()?;
            let mut text = String::new();
            reader.read_to_string(&mut text).unwrap();
            Some(text)
        };
        let check_selected = || {
            let selected = discover_folder(scratch.path());
            let [high, low] = selected.as_slice() else {
                panic!("expected both world packs")
            };
            for (id, expected) in [
                ("demo:value/hidden.json", "lower world pack"),
                ("demo:value/mod.json", "mod pack"),
                ("minecraft:test.txt", "vanilla pack"),
            ] {
                assert_eq!(read(&[low], id).as_deref(), Some(expected));
                assert!(read(&[high, low], id).is_none());
            }
            assert_eq!(
                read(&[high, low], "demo:value/own.json").as_deref(),
                Some("higher world pack")
            );
            assert_eq!(
                read(&[high, low], "demo:test.txt").as_deref(),
                Some("unfiltered")
            );
        };
        check_selected();
        // A selected ZIP has the same filter semantics as a folder.
        fs::remove_dir_all(high).unwrap();
        let mut zip =
            zip::ZipWriter::new(fs::File::create(scratch.path().join("high.zip")).unwrap());
        for (name, bytes) in [
            ("pack.mcmeta", meta.as_bytes()),
            (own, b"higher world pack"),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        check_selected();
    }

    #[test]
    fn neoforge_recipe_priorities_keep_pack_order_and_read_errors() {
        use sunrise_mod_package::content::RECIPE_PRIORITIES_PATH;
        let root =
            std::env::temp_dir().join(format!("sunrise-priority-packs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for pack in ["a", "b", "c", "other_namespace"] {
            let dir = root.join(format!("datapacks/{pack}"));
            fs::create_dir_all(dir.join("data/neoforge")).unwrap();
            fs::write(
                dir.join("pack.mcmeta"),
                r#"{"pack":{"description":"","pack_format":15}}"#,
            )
            .unwrap();
            let path = dir.join(RECIPE_PRIORITIES_PATH);
            match pack {
                "a" => fs::write(path, br#"{"entries":{"test:b":5}}"#).unwrap(),
                "b" => fs::create_dir(path).unwrap(), // The path exists but cannot be read as a file.
                "c" => fs::write(path, br#"{"replace":true}"#).unwrap(),
                _ => {
                    fs::create_dir_all(dir.join("data/test")).unwrap();
                    fs::write(dir.join("data/test/recipe_priorities.json"), b"{").unwrap();
                }
            }
        }
        let packs = discover(&root);
        let store = DataStore::load(&DataPacks::new(&packs.iter().collect::<Vec<_>>()));
        let layers = store.priority_layers;
        assert_eq!(
            layers
                .iter()
                .map(|layer| layer.pack.as_str())
                .collect::<Vec<_>>(),
            ["file/a", "file/b", "file/c"]
        );
        assert!(
            layers
                .iter()
                .all(|layer| layer.path == RECIPE_PRIORITIES_PATH)
        );
        assert_eq!(
            layers[0].contents.as_ref().unwrap(),
            br#"{"entries":{"test:b":5}}"#
        );
        assert!(layers[1].contents.is_err());
        assert_eq!(layers[2].contents.as_ref().unwrap(), br#"{"replace":true}"#);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_pack_level_outside_the_worlds_folder_is_not_a_level() {
        let dir = std::env::temp_dir().join(format!("sunrise-pack-ids-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pack = dir.join("datapacks/p");
        for path in [
            "data/test/dimension/pocket.json",
            "data/test/dimension/a/b.json",
            "data/test/dimension/my pocket.json",
            "data/test/dimension/Pocket.json",
            "data/Test/dimension/pocket.json",
        ] {
            let file = pack.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, "{}").unwrap();
        }
        std::fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"","pack_format":15}}"#,
        )
        .unwrap();
        let packs = discover(&dir);
        let refs: Vec<&DataPack> = packs.iter().collect();
        let ids: Vec<String> = dimension_stems(&DataPacks::new(&refs))
            .into_keys()
            .collect();
        assert_eq!(ids, ["test:a/b", "test:pocket"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_filter_hides_a_lower_packs_worldgen_file_from_the_generator() {
        let low = memory_pack(
            "low",
            &[
                ("data/test/worldgen/noise/hidden.json", "{\"low\":1}"),
                ("data/test/worldgen/noise/kept.json", "{\"low\":2}"),
                ("data/test/dimension/hidden.json", "{}"),
            ],
        );
        let meta = serde_json::json!({"filter": {"block": [
            {"namespace": "^test$", "path": "^(worldgen/noise|dimension)/hidden[.]json$"},
        ]}});
        let high = memory_pack("high", &[]).with_filter(filter_of(&meta));
        let data = DataPacks::new(&[&low, &high]);
        let (o, warnings) = worldgen_overrides_over(
            &[],
            &data,
            &sunrise_world::conditions::LoadConditions::vanilla(),
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            o.entries("noises").collect::<Vec<_>>(),
            [("test:kept", "{\"low\":2}")]
        );
        assert!(dimension_stems(&data).is_empty());
        let unfiltered = DataPacks::new(&[&low]);
        assert_eq!(
            worldgen_overrides_over(
                &[],
                &unfiltered,
                &sunrise_world::conditions::LoadConditions::vanilla()
            )
            .0
            .entries("noises")
            .count(),
            2
        );
        assert_eq!(dimension_stems(&unfiltered).len(), 1);
    }

    #[test]
    fn a_pack_replaces_a_mods_level() {
        let root = std::env::temp_dir().join(format!("sunrise-mod-level-{}", std::process::id()));
        let world = root.join("world");
        let pack = world.join("datapacks/over");
        let types = pack.join("data/test/dimension_type");
        std::fs::create_dir_all(&types).unwrap();
        std::fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"","pack_format":15}}"#,
        )
        .unwrap();
        std::fs::write(
            types.join("pocket.json"),
            r#"{"min_y":0,"height":64,"logical_height":64}"#,
        )
        .unwrap();
        let packs = discover(&world);
        let mods = [overrides::ModDimension {
            id: "test:pocket".into(),
            stem: r#"{"type":"test:pocket"}"#.into(),
            dimension_type: r#"{"min_y":-128,"height":448,"logical_height":448}"#.into(),
            sea_level: None,
        }];
        let height = |packs: &[&DataPack]| {
            let (o, _) = worldgen_overrides_over(
                &mods,
                &DataPacks::new(packs),
                &sunrise_world::conditions::LoadConditions::vanilla(),
            );
            o.level_height("test:pocket").unwrap().height
        };
        assert_eq!(height(&[]), 448);
        assert_eq!(height(&[&packs[0]]), 64);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn structure_templates_read_archive_generations_without_extraction() {
        use std::io::Write;
        use {
            sunrise_core::nbt::{Limits, NamedTag},
            sunrise_world::{
                registry::BlockRegistry,
                worldgen::structure::{StateTransforms, StructureResources},
            },
        };
        let root =
            std::env::temp_dir().join(format!("sunrise-template-source-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        struct Scratch(PathBuf);
        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let scratch = Scratch(root);
        let archive = scratch.0.join("templates.jar");
        let write_archive = |width| {
            let tag = Tag::compound([
                (
                    "size",
                    Tag::List(vec![Tag::Int(width), Tag::Int(3), Tag::Int(4)]),
                ),
                (
                    "palette",
                    Tag::List(vec![Tag::compound([(
                        "Name",
                        Tag::String("minecraft:stone".into()),
                    )])]),
                ),
                (
                    "blocks",
                    Tag::List(vec![Tag::compound([
                        (
                            "pos",
                            Tag::List(vec![Tag::Int(1), Tag::Int(0), Tag::Int(1)]),
                        ),
                        ("state", Tag::Int(0)),
                    ])]),
                ),
                ("entities", Tag::List(Vec::new())),
            ]);
            let bytes = sunrise_core::nbt::write_named(
                &NamedTag {
                    name: "".into(),
                    tag,
                },
                Limits::templates(),
            )
            .unwrap();
            let mut gzip =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            gzip.write_all(&bytes).unwrap();
            let temporary = scratch.0.join("replacement.jar");
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&temporary).unwrap());
            zip.start_file(
                "data/example/structure/nested/piece.nbt",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(&gzip.finish().unwrap()).unwrap();
            zip.finish().unwrap();
            std::fs::rename(temporary, &archive).unwrap();
        };
        write_archive(2);
        let old =
            DataPack::from_source("passive".into(), ResourceSource::archive(&archive).unwrap());
        let (before, warnings) = worldgen_overrides(
            &DataPacks::new(&[&old]),
            &sunrise_world::conditions::LoadConditions::vanilla(),
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        write_archive(5);
        let new =
            DataPack::from_source("passive".into(), ResourceSource::archive(&archive).unwrap());
        let (after, warnings) = worldgen_overrides(
            &DataPacks::new(&[&new]),
            &sunrise_world::conditions::LoadConditions::vanilla(),
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_ne!(
            before, after,
            "a reopened archive invalidates the generator cache"
        );
        let registry = Box::leak(Box::new(
            BlockRegistry::from_nbt(&Tag::compound([
                ("DataVersion", Tag::Int(sunrise_world::DATA_VERSION)),
                (
                    "states",
                    Tag::List(vec![
                        Tag::compound([("id", Tag::String("minecraft:air".into()))]),
                        Tag::compound([("id", Tag::String("minecraft:stone".into()))]),
                    ]),
                ),
                (
                    "defaults",
                    Tag::compound([
                        ("minecraft:air", Tag::Int(0)),
                        ("minecraft:stone", Tag::Int(1)),
                    ]),
                ),
            ]))
            .unwrap(),
        ));
        let manager = |overrides: &WorldgenOverrides| {
            let transforms = StateTransforms::from_json(
                r#"{"rotate":[[0,0,0],[1,1,1]],"mirror":[[0,0],[1,1]],"full_block":[false,true]}"#,
                2,
            )
            .unwrap();
            StructureResources::new(
                registry,
                transforms,
                &scratch.0.join("unpacked-templates"),
                BTreeMap::new(),
                &BTreeMap::new(),
            )
            .with_templates(overrides.templates().clone())
        };
        // Neither generation has read its NBT yet; the old one must still
        // read from the reader retained before the archive was replaced.
        let first = manager(&before).template("example:nested/piece").unwrap();
        let second = manager(&after).template("example:nested/piece").unwrap();
        assert_eq!(first.size, [2, 3, 4]);
        assert_eq!(second.size, [5, 3, 4]);
        let blocks = second.palettes()[0].blocks();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].pos, [1, 0, 1]);
        assert_eq!(blocks[0].state.0, 1);
        assert!(!scratch.0.join("data").exists());
        assert!(!scratch.0.join("unpacked-templates").exists());
    }

    #[test]
    fn folder_metadata_is_required_before_contents_are_discovered() {
        let root =
            std::env::temp_dir().join(format!("sunrise-folder-metadata-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (name, metadata) in [
            ("broken", Some("{")),
            ("root_empty", Some("{}")),
            ("root_array", Some("[]")),
            ("pack_empty", Some(r#"{"pack":{}}"#)),
            ("pack_string", Some(r#"{"pack":"bad"}"#)),
            ("missing", None),
            (
                "full",
                Some(
                    r#"{"pack":{"description":"full","pack_format":121,"min_format":[121,0],"max_format":[121,0]}}"#,
                ),
            ),
            (
                "description",
                Some(r#"{"pack":{"description":"description only"}}"#),
            ),
            ("numeric", Some(r#"{"pack":{"description":42}}"#)),
            ("null", Some(r#"{"pack":{"description":null}}"#)),
            ("bom", Some("\u{feff}{\"pack\":{\"description\":\"BOM\"}}")),
            (
                "comment",
                Some("{\"pack\":{\"description\":\"comment\"}}\n// trailing"),
            ),
        ] {
            let dir = root.join(name);
            std::fs::create_dir_all(dir.join("data/h146/function")).unwrap();
            std::fs::write(
                dir.join(format!("data/h146/function/{name}.mcfunction")),
                "scoreboard players set admitted h146 1\n",
            )
            .unwrap();
            if let Some(metadata) = metadata {
                std::fs::write(dir.join("pack.mcmeta"), metadata).unwrap();
            }
        }
        let packs = discover_folder(&root);
        assert_eq!(
            packs.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            [
                "file/bom",
                "file/comment",
                "file/description",
                "file/full",
                "file/numeric"
            ]
        );
        let store = DataStore::load(&DataPacks::new(&packs.iter().collect::<Vec<_>>()));
        assert_eq!(
            store
                .functions
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [
                "h146:bom",
                "h146:comment",
                "h146:description",
                "h146:full",
                "h146:numeric"
            ]
        );
        let numeric = packs.iter().find(|p| p.id == "file/numeric").unwrap();
        let raw: serde_json::Value =
            serde_json::from_slice(&numeric.read("pack.mcmeta").unwrap()).unwrap();
        assert_eq!(raw["pack"]["description"], 42);
        // Builtin and feature construction still have their independent paths.
        let resources = root.join("resources");
        let feature = resources.join("data/minecraft/datapacks/feature");
        std::fs::create_dir_all(&feature).unwrap();
        std::fs::write(
            feature.join("pack.mcmeta"),
            r#"{"pack":{"description":"feature"}}"#,
        )
        .unwrap();
        assert_eq!(
            feature_packs(&resources)
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>(),
            ["feature"]
        );
        assert_eq!(DataPack::builtin(&resources).id, "vanilla");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn feature_packs_select_at_the_top() {
        let resources =
            std::env::temp_dir().join(format!("feature-position-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&resources);
        let feature = resources.join("data/minecraft/datapacks/feature");
        std::fs::create_dir_all(&feature).unwrap();
        std::fs::write(
            feature.join("pack.mcmeta"),
            r#"{"pack":{"description":"feature"}}"#,
        )
        .unwrap();
        let packs = feature_packs(&resources);
        let selection = packs[0].selection.as_ref().unwrap();
        assert!(selection.position == crate::pack_meta::Position::Top);
        assert!(!selection.required && !selection.fixed);
        std::fs::remove_dir_all(resources).unwrap();
    }

    #[test]
    fn data_store_admits_only_valid_content_resource_ids() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("data/example/recipe/sub");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("value.json"), "{}").unwrap();
        std::fs::write(folder.join("Invalid_Name.json"), "{}").unwrap();
        let folder = root.path().join("data/bad namespace/recipe/sub");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("other.json"), "{}").unwrap();
        let pack = DataPack::from_source(
            "file/registry-fixture".into(),
            ResourceSource::directory(root.path().to_owned()),
        );
        let store = DataStore::load(&DataPacks::new(&[&pack]));
        assert_eq!(
            store.files.keys().map(String::as_str).collect::<Vec<_>>(),
            ["recipe/example:sub/value"]
        );
    }

    #[test]
    fn data_map_files_are_content_files_keyed_by_map_namespace_and_registry() {
        assert_eq!(
            content_id("data/mymod/data_maps/item/fuel.json").as_deref(),
            Some("data_maps/mymod:item/fuel")
        );
        assert_eq!(content_id("data/mymod/data_maps/item/fuel.txt"), None);
    }

    #[test]
    fn a_pack_that_cannot_be_copied_is_reported() {
        let root = std::env::temp_dir().join(format!("hz-datapack-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let pack = root.join("pack");
        std::fs::create_dir_all(pack.join("data")).unwrap();
        std::fs::write(pack.join("pack.mcmeta"), "{}").unwrap();
        std::fs::write(pack.join("data/functions.mcfunction"), "").unwrap();
        // A file where the pack's folder goes: every copy under it fails.
        let blocked = root.join("blocked");
        std::fs::write(&blocked, "").unwrap();
        assert!(!copy_pack(&pack, &blocked));
        let dir = root.join("packs");
        assert!(copy_pack(&pack, &dir));
        assert!(dir.join("pack/pack.mcmeta").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_game_folder_s_new_world_packs_go_into_new_worlds_only() {
        let root = std::env::temp_dir().join(format!("hz-new-world-packs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (game, world, old) = (root.join("game"), root.join("new"), root.join("old"));
        assert!(seed_create_world(&game).is_none());
        assert_eq!(install_new_world_packs(&game, &world), Ok(()));
        assert!(!world.join("datapacks").exists());
        for pack in ["void", "islands"] {
            let dir = game.join(NEW_WORLD_PACKS).join(pack);
            std::fs::create_dir_all(dir.join("data/v/function")).unwrap();
            std::fs::write(dir.join("pack.mcmeta"), r#"{"pack":{"description":""}}"#).unwrap();
            std::fs::write(dir.join("data/v/function/a.mcfunction"), "say a").unwrap();
        }
        let (temp, ids) = seed_create_world(&game).unwrap();
        assert_eq!(ids, ["file/islands", "file/void"]);
        assert!(temp.join("void/data/v/function/a.mcfunction").is_file());
        let _ = std::fs::remove_dir_all(&temp);
        install_new_world_packs(&game, &world).unwrap();
        let ids: Vec<String> = discover(&world).into_iter().map(|p| p.id).collect();
        assert_eq!(ids, ["file/islands", "file/void"]);
        // A world that exists already keeps the packs it has.
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("level.dat"), "").unwrap();
        install_new_world_packs(&game, &old).unwrap();
        assert!(discover(&old).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn enabled_packs_override_worldgen_in_order() {
        let dir = std::env::temp_dir().join(format!("hz-datapack-worldgen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let write = |pack: &str, p: &str, t: &str| {
            let path = dir.join("datapacks").join(pack).join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, t).unwrap();
        };
        for pack in ["a", "b", "off"] {
            write(pack, "pack.mcmeta", r#"{"pack":{"description":"fixture"}}"#);
            write(pack, "data/minecraft/worldgen/noise/cave.json", pack);
            write(pack, "data/p/structure/room/a.nbt", pack);
            write(
                pack,
                "data/minecraft/tags/worldgen/biome/is_hill.json",
                "{\"values\":[]}",
            );
        }
        write("a", "data/minecraft/worldgen/biome/new_peaks.json", "{}");
        write("a", "data/minecraft/worldgen/biome/plains.json", "{}");
        write(
            "a",
            "data/minecraft/tags/worldgen/structure/village.json",
            "{}",
        );
        write(
            "b",
            "data/minecraft/dimension_type/overworld.json",
            "{\"min_y\":-64,\"height\":2096}",
        );
        // `b` is enabled first, so `a` (enabled after it) wins.
        std::fs::write(
            dir.join("datapacks.json"),
            "{\"enabled\":[\"file/b\",\"file/a\"],\"disabled\":[\"file/off\"]}",
        )
        .unwrap();
        let packs = discover(&dir);
        let enabled = enabled_ids(&dir, &packs, &[]);
        assert_eq!(enabled, ["file/b", "file/a"]);
        let enabled: Vec<&DataPack> = enabled
            .iter()
            .map(|id| packs.iter().find(|p| &p.id == id).unwrap())
            .collect();
        let (o, warnings) = worldgen_overrides(
            &DataPacks::new(&enabled),
            &sunrise_world::conditions::LoadConditions::vanilla(),
        );
        assert_eq!(
            o.entries("noises").collect::<Vec<_>>(),
            [("minecraft:cave", "a")]
        );
        assert_eq!(o.templates().len(), 1);
        assert_eq!(o.templates()["p:room/a"].read().unwrap(), b"a");
        assert_eq!(
            o.entries("biomes").collect::<Vec<_>>(),
            [("minecraft:new_peaks", "{}"), ("minecraft:plains", "{}")]
        );
        let tag_packs: Vec<&str> = o.tags("biome_tags").map(|(_, p, _)| p).collect();
        assert_eq!(tag_packs, ["file/b", "file/a"]);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("tags/worldgen/structure"));
        assert_eq!(
            o.level_height("minecraft:overworld").map(|h| h.height),
            Some(2096)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
    use sunrise_core::nbt::parse_snbt;

    #[test]
    fn parses_comments_continuations_and_macros() {
        let f = parse_function(
            "a:b",
            "# a comment\n\nsay hi\n  say \\\n  there  \n$say $(who) is $(n)\n$tell $(who) $(who)\n",
        )
        .unwrap();
        assert_eq!(
            f.lines,
            vec![
                FunctionLine::Command("say hi".into()),
                FunctionLine::Command("say there".into()),
                FunctionLine::Macro("say $(who) is $(n)".into()),
                FunctionLine::Macro("tell $(who) $(who)".into()),
            ]
        );
        assert_eq!(f.parameters, ["who", "n"]);
        assert!(
            parse_function("a:b", "/say hi")
                .unwrap_err()
                .contains("did you mean 'say'")
        );
        assert!(
            parse_function("a:b", "//x")
                .unwrap_err()
                .contains("use '#'")
        );
        assert!(parse_function("a:b", "say \\").is_err());
        assert!(parse_function("a:b", "$say $(bad name)").is_err());
    }

    #[test]
    fn macros_substitute_vanilla_text() {
        let f = parse_function("a:b", "$say $(s) $(i) $(d) $(l) $(b) $(c)\nsay plain").unwrap();
        let args = parse_snbt("{s:\"hi\",i:5,d:1.5d,l:7L,b:1b,c:{x:1}}")
            .unwrap()
            .0;
        let out = f.instantiate(Some(&args)).unwrap();
        assert_eq!(out, ["say hi 5 1.5 7 1 {x:1}", "say plain"]);
        let args = parse_snbt("{s:\"x\",i:1,d:3.0d,l:1L,b:0b,c:{}}").unwrap().0;
        assert_eq!(f.instantiate(Some(&args)).unwrap()[0], "say x 1 3 1 0 {}");
        assert_eq!(
            f.instantiate(None).unwrap_err().0,
            "commands.function.error.missing_arguments"
        );
        let partial = parse_snbt("{s:\"x\"}").unwrap().0;
        assert_eq!(
            f.instantiate(Some(&partial)).unwrap_err().0,
            "commands.function.error.missing_argument"
        );
        // A plain function needs no arguments.
        let plain = parse_function("a:p", "say 1").unwrap();
        assert_eq!(plain.instantiate(None).unwrap(), ["say 1"]);
    }

    #[test]
    fn macro_arguments_use_decimal_format_not_fixed_precision() {
        // Temurin 25 DecimalFormat("#") with 15 fractional places.
        for (tag, expected) in [
            (Tag::Float(0.1_f32.to_bits()), ".100000001490116"),
            (Tag::Double(0.5_f64.to_bits()), ".5"),
            (Tag::Double((-0.0_f64).to_bits()), "0"),
            (Tag::Double((-1e-16_f64).to_bits()), "-0"),
            (Tag::Double(f64::INFINITY.to_bits()), "∞"),
            (Tag::Double(1e23_f64.to_bits()), "99999999999999990000000"),
        ] {
            assert_eq!(stringify(&tag), expected);
        }
    }

    /// `StringTemplate.fromString` throws "No variables in macro" for a `$`
    /// line with no `$(…)`, so `FunctionBuilder.addMacro` fails and
    /// `ServerFunctionLibrary.reload` leaves the whole file out: `/function`
    /// reports it unknown instead of running the line unchanged.
    #[test]
    fn a_macro_line_without_a_variable_drops_the_whole_file() {
        let dir = std::env::temp_dir().join(format!("hz-datapack-macro-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pack = dir.join("datapacks/macros");
        let write = |p: &str, t: &str| {
            let path = pack.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, t).unwrap();
        };
        write(
            "pack.mcmeta",
            "{\"pack\":{\"description\":\"Macros\",\"pack_format\":1}}",
        );
        // A plain line before and after does not save the file.
        write(
            "data/t/function/novar.mcfunction",
            "say before\n$say hello\nsay after\n",
        );
        write("data/t/function/lone.mcfunction", "$\n");
        // A `$` not followed by `(` is text, not a variable.
        write(
            "data/t/function/dollar.mcfunction",
            "$say $(who) costs $5\n",
        );
        write("data/t/function/plain.mcfunction", "say plain\n");
        let packs = discover(&dir);
        let store = DataStore::load(&DataPacks::new(&[&packs[0]]));
        for id in ["t:novar", "t:lone"] {
            assert!(!store.functions.contains_key(id), "{id} was registered");
            assert!(
                store
                    .errors
                    .iter()
                    .any(|(e, m)| e == id && m.contains("No variables in macro")),
                "{id} has no load error: {:?}",
                store.errors
            );
        }
        let dollar = &store.functions["t:dollar"];
        assert_eq!(dollar.parameters, ["who"]);
        assert!(store.functions.contains_key("t:plain"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn packs_provide_functions_and_tags() {
        let dir = std::env::temp_dir().join(format!("hz-datapack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pack = dir.join("datapacks/mypack");
        let write = |p: &str, t: &str| {
            let path = pack.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, t).unwrap();
        };
        write(
            "pack.mcmeta",
            "{\"pack\":{\"description\":\"Hello\",\"pack_format\":1}}",
        );
        write(
            "data/mine/function/a.mcfunction",
            "say a\nfunction mine:sub/b\n",
        );
        write("data/mine/function/sub/b.mcfunction", "say b");
        write(
            "data/minecraft/tags/function/tick.json",
            "{\"values\":[\"mine:a\",\"#mine:more\",{\"id\":\"mine:gone\",\"required\":false}]}",
        );
        write(
            "data/mine/tags/function/more.json",
            "{\"values\":[\"mine:sub/b\",\"mine:a\"]}",
        );
        write(
            "data/mine/predicate/p.json",
            "{\"type\":\"minecraft:random_chance\",\"chance\":1}",
        );
        write(
            "data/mine/dialog/config/points.json",
            "{\"type\":\"minecraft:notice\",\"title\":\"Points\"}",
        );
        write("data/mine/dimension_type/pocket.json", "{\"min_y\":-128}");
        let packs = discover(&dir);
        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0].id, "file/mypack");
        assert_eq!(packs[0].description().plain(), "Hello");
        let store = DataStore::load(&DataPacks::new(&[&packs[0]]));
        assert_eq!(store.functions.len(), 2);
        assert!(store.functions.contains_key("mine:sub/b"));
        assert_eq!(
            store.tag_functions("minecraft:tick"),
            ["mine:a", "mine:sub/b"]
        );
        assert!(store.files.contains_key("predicate/mine:p"));
        // Dialogs reach the synchronized registry through the content files.
        assert_eq!(
            store.files["dialog/mine:config/points"],
            [b"{\"type\":\"minecraft:notice\",\"title\":\"Points\"}".to_vec()]
        );
        // So do dimension types: a client takes a level's build height from them.
        assert_eq!(
            store.files["dimension_type/mine:pocket"],
            [b"{\"min_y\":-128}".to_vec()]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn function_removals_use_finished_members_and_ignore_missing_ids() {
        let mut store = DataStore::default();
        store.functions.insert(
            "t:f".into(),
            Function {
                id: "t:f".into(),
                lines: Vec::new(),
                parameters: Vec::new(),
            },
        );
        store.function_tags.insert(
            "t:child".into(),
            vec![("t:f".into(), true, false), ("t:f".into(), true, true)],
        );
        store.function_tags.insert(
            "t:parent".into(),
            vec![
                ("t:f".into(), true, false),
                ("#t:child".into(), true, true),
                ("t:missing".into(), true, true),
                ("#t:missing".into(), true, true),
            ],
        );
        assert_eq!(store.tag_functions("t:parent"), ["t:f"]);
        store
            .function_tags
            .insert("t:child".into(), vec![("t:f".into(), true, false)]);
        assert!(store.tag_functions("t:parent").is_empty());
    }

    #[test]
    fn tag_resolution_dedupes_fast_ends_cycles_and_nests_deeply() {
        let mut store = DataStore::default();
        // Every referenced function exists; missing required ones invalidate a tag.
        for id in (0..25_000)
            .map(|i| format!("t:f{i}"))
            .chain(["t:x", "t:y", "t:deep"].map(str::to_owned))
        {
            store.functions.insert(
                id.clone(),
                Function {
                    id,
                    lines: Vec::new(),
                    parameters: Vec::new(),
                },
            );
        }
        let big: Vec<TagEntryOf> = (0..50_000)
            .map(|i| (format!("t:f{}", i % 25_000), true, false))
            .collect();
        store.function_tags.insert("t:big".into(), big);
        let start = std::time::Instant::now();
        let got = store.tag_functions("t:big");
        assert_eq!(got.len(), 25_000);
        assert_eq!(got[0], "t:f0");
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        // A cycle terminates and keeps first-seen order.
        store.function_tags.insert(
            "t:a".into(),
            vec![("#t:b".into(), true, false), ("t:x".into(), true, false)],
        );
        store.function_tags.insert(
            "t:b".into(),
            vec![("#t:a".into(), true, false), ("t:y".into(), true, false)],
        );
        assert_eq!(store.tag_functions("t:a"), ["t:y", "t:x"]);
        // Nesting deeper than the old cap of 16 still resolves.
        for i in 0..40 {
            store.function_tags.insert(
                format!("d:{i}"),
                vec![(format!("#d:{}", i + 1), true, false)],
            );
        }
        store
            .function_tags
            .insert("d:40".into(), vec![("t:deep".into(), true, false)]);
        assert_eq!(store.tag_functions("d:0"), ["t:deep"]);
    }

    #[test]
    fn server_json_resources_keep_ordered_stacks_for_conditional_readers() {
        let root =
            std::env::temp_dir().join(format!("sunrise-server-json-stacks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let kinds = [
            "enchantment",
            "jukebox_song",
            "instrument",
            "predicate",
            "item_modifier",
            "slot_source",
        ];
        let mut packs = Vec::new();
        for name in ["low", "high"] {
            let folder = root.join(name);
            for kind in kinds {
                let file = folder.join(format!("data/demo/{kind}/nested/entry.json"));
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                let mut entry = serde_json::json!({"layer":name,"neoforge:conditions":[{"type":"unsupported"}]});
                // A 26.3 predicate names its condition under `type`.
                if kind == "predicate" {
                    entry["type"] = "minecraft:random_chance".into();
                    entry["chance"] = 1.into();
                }
                std::fs::write(file, entry.to_string()).unwrap();
            }
            packs.push(DataPack::from_source(
                name.into(),
                ResourceSource::directory(folder),
            ));
        }
        let store = DataStore::load(&DataPacks::new(&packs.iter().collect::<Vec<_>>()));
        for kind in kinds {
            let versions = &store.files[&format!("{kind}/demo:nested/entry")];
            assert_eq!(versions.len(), 2);
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&versions[0]).unwrap()["layer"],
                "low"
            );
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&versions[1]).unwrap()["layer"],
                "high"
            );
        }
        // Predicates, item modifiers and slot sources are loaded solely through
        // LootData + LoadConditions, never duplicated in DataStore::load.
        assert!(store.predicates.is_empty());
        assert!(store.item_modifiers.is_empty());
        assert!(store.slot_sources.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// #1381: predicates and item modifiers carry no element conditions: a
    /// nested key stays, unevaluated.
    #[test]
    fn predicates_and_item_modifiers_keep_list_elements_conditions_as_data() {
        let root =
            std::env::temp_dir().join(format!("sunrise-1381-datapack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let nested = serde_json::json!([{"type": "absent:condition"}]);
        let predicate = serde_json::json!({"type": "minecraft:all_of", "terms": [
            {"type": "minecraft:random_chance", "chance": 0.5, "neoforge:conditions": nested}]});
        let modifier = serde_json::json!([
            {"type": "minecraft:set_count", "count": 2, "neoforge:conditions": nested}]);
        for (kind, value) in [("predicate", &predicate), ("item_modifier", &modifier)] {
            let file = root.join(format!("data/demo/{kind}/entry.json"));
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, value.to_string()).unwrap();
        }
        let pack = DataPack::from_source("pack".into(), ResourceSource::directory(root.clone()));
        let store = DataStore::load(&DataPacks::new(&[&pack]));
        let files: Vec<_> = store.files.into_iter().collect();
        let overlay = sunrise_world::loot::PackOverlay::new(&files);
        let empty_root = root.join("empty");
        std::fs::create_dir_all(&empty_root).unwrap();
        std::fs::write(empty_root.join("items.json"), "{}").unwrap();
        let defaults = sunrise_world::item_defaults::read(&empty_root).unwrap();
        let loot = sunrise_world::loot::LootData::load(&empty_root, &overlay, &defaults).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(loot.predicates().get("demo:entry"), Some(&predicate));
        assert_eq!(loot.item_modifiers().get("demo:entry"), Some(&modifier));
    }

    #[test]
    fn function_tags_ignore_top_level_load_conditions() {
        let root =
            std::env::temp_dir().join(format!("sunrise-command-conditions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut packs = Vec::new();
        for name in ["low", "high"] {
            let folder = root.join(name);
            let conditions = if name == "high" {
                serde_json::json!([{"type":"neoforge:mod_loaded","modid":"provider"}])
            } else {
                serde_json::json!([])
            };
            let dir = folder.join("data/demo/tags/function");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("load.json"), serde_json::json!({"replace":true,"values":[format!("demo:{name}")],"neoforge:conditions":conditions}).to_string()).unwrap();
            std::fs::write(
                dir.join("unknown.json"),
                r#"{"neoforge:conditions":[{"type":"unsupported"}],"replace":true}"#,
            )
            .unwrap();
            packs.push(DataPack::from_source(
                name.into(),
                ResourceSource::directory(folder),
            ));
        }
        let store = DataStore::load(&DataPacks::new(&packs.iter().collect::<Vec<_>>()));
        assert_eq!(
            store.function_tags["demo:load"],
            [("demo:high".into(), true, false)]
        );
        assert!(store.function_tags["demo:unknown"].is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn newly_discovered_packs_follow_utf16_after_saved_selections() {
        let world = temp_folder().unwrap();
        for name in ["z-saved", "a-new", "\u{e000}", "\u{10000}"] {
            let dir = world.join("datapacks").join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("pack.mcmeta"),
                r#"{"pack":{"description":"order"}}"#,
            )
            .unwrap();
        }
        std::fs::write(
            world.join("datapacks.json"),
            r#"{"enabled":["file/z-saved"]}"#,
        )
        .unwrap();
        let packs = discover(&world);
        let selected = enabled_ids(&world, &packs, &[]);
        std::fs::remove_dir_all(world).unwrap();
        // Temurin 25.0.5 natural String order for the three newly added IDs.
        assert_eq!(
            selected,
            [
                "file/z-saved",
                "file/a-new",
                "file/\u{10000}",
                "file/\u{e000}"
            ]
        );
    }

    #[test]
    fn folder_packs_list_the_needed_kinds_in_walk_order() {
        let dir = std::env::temp_dir().join(format!("hz-datapack-kinds-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pack = dir.join("datapacks/p");
        let files = [
            "data/b/function/z.mcfunction",
            "data/b/function/a/y.mcfunction",
            "data/b/loot_table/l.json",
            "data/b/tags/block/t.json",
            "data/b/tags/function/load.json",
            "data/b/predicate/p.json",
            "data/a/item_modifier/m.json",
            "data/a/function/f.mcfunction",
            "data/a/functions/old.mcfunction",
            "data/a/recipe/r.json",
            "data/a/tags/function/tick.json",
            "data/c/worldgen/noise/n.json",
        ];
        for f in files {
            let path = pack.join(f);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "{}").unwrap();
        }
        std::fs::write(pack.join("pack.mcmeta"), "{}").unwrap();
        let p = DataPack::builtin(&pack);
        let kinds = ["function", "item_modifier", "predicate", "tags/function"];
        let wanted: Vec<String> = p
            .list("data/")
            .into_iter()
            .filter(|f| {
                let rest = f.splitn(3, '/').nth(2).unwrap_or_default();
                kinds.iter().any(|k| rest.starts_with(&format!("{k}/")))
            })
            .collect();
        assert_eq!(wanted.len(), 7);
        assert_eq!(p.list_kinds(&kinds), wanted);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A mod's registry folders are listed per pack, lowest pack first, and only the declared
    /// registries' `.json` files, in any entry namespace.
    #[test]
    fn registry_files_list_the_declared_folders_in_pack_order() {
        let root =
            std::env::temp_dir().join(format!("sunrise-registry-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut packs = Vec::new();
        for name in ["low", "high"] {
            let folder = root.join(name);
            for file in [
                "data/example/example/spell/fire.json",
                "data/addon/example/spell/nested/ice.json",
                "data/addon/worldgen/spell/frost.json",
                "data/addon/minecraft/worldgen/spell/skip.json",
                "data/addon/example/other/skip.json",
                "data/addon/example/spell/note.txt",
                "data/addon/example/spellbook/skip.json",
            ] {
                let path = folder.join(file);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, name).unwrap();
            }
            packs.push(DataPack::from_source(
                name.into(),
                ResourceSource::directory(folder),
            ));
        }
        let refs = packs.iter().collect::<Vec<_>>();
        let keys = [
            "example:spell",
            "minecraft:example/spell/nested",
            "minecraft:worldgen/spell",
            "example:../missing",
        ];
        let path = "data/example/example/spell/fire.json";
        let mut wide = vec![b' '; 1 << 20];
        wide.extend_from_slice(br#"{"power":7}"#);
        std::fs::write(root.join("high").join(path), &wide).unwrap();
        let files = registry_files(&refs, &keys).unwrap();
        assert_eq!(
            files.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                "data/addon/example/spell/nested/ice.json",
                "data/addon/worldgen/spell/frost.json",
                "data/example/example/spell/fire.json"
            ]
        );
        assert_eq!(files[path], ("high".to_owned(), wide));
        assert_eq!(
            files["data/addon/example/spell/nested/ice.json"],
            ("high".to_owned(), b"high".to_vec())
        );
        assert!(registry_files(&refs, &[]).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn registry_files_names_the_unreadable_winner_without_falling_back() {
        use sunrise_assets::resource_source::Resource;

        let path = "data/addon/example/spell/nested/fire.json";
        let readable = DataPack::from_source(
            "readable".into(),
            ResourceSource::indexed(BTreeMap::from([(
                path.to_owned(),
                Resource::bytes(br#"{"power":7}"#.to_vec()),
            )])),
        );
        let unreadable = DataPack::from_source(
            "unreadable".into(),
            ResourceSource::indexed(BTreeMap::from([(
                path.to_owned(),
                Resource::file(ResourceSource::default(), path.to_owned()),
            )])),
        );
        let keys = ["example:spell"];
        let error = registry_files(&[&readable, &unreadable], &keys).unwrap_err();
        assert_eq!(error.registry, "example:spell");
        assert_eq!(error.entry, "addon:nested/fire");
        assert_eq!(error.pack, "unreadable");
        assert_eq!(error.file, path);
        assert!(
            matches!(error.reason, mod_api::DatapackLoadReason::ReadFile(message) if !message.is_empty())
        );

        let files = registry_files(&[&unreadable, &readable], &keys).unwrap();
        assert_eq!(
            files[path],
            ("readable".to_owned(), br#"{"power":7}"#.to_vec())
        );
    }

    /// A data pack's applicable overlay composes with its root: a file an overlay replaces reads the
    /// overlay's bytes, an overlay-only file exists, and an overlay for another format stays out.
    #[test]
    fn a_data_packs_active_overlays_replace_and_extend_its_root() {
        let scratch = tempfile::tempdir().unwrap();
        let pack = scratch.path().join("overlaid");
        for (path, bytes) in [
            (
                "pack.mcmeta",
                br#"{"pack":{"description":"o"},"overlays":{"entries":[
                    {"directory":"now","min_format":121,"max_format":121},
                    {"directory":"later","min_format":999,"max_format":999}]}}"#
                    .as_slice(),
            ),
            ("data/demo/value/root.json", b"root"),
            ("data/demo/value/both.json", b"from the root"),
            ("now/data/demo/value/both.json", b"from the overlay"),
            ("now/data/demo/value/only.json", b"overlay only"),
            ("later/data/demo/value/never.json", b"other format"),
        ] {
            let at = pack.join(path);
            fs::create_dir_all(at.parent().unwrap()).unwrap();
            fs::write(at, bytes).unwrap();
        }
        let packs = discover_folder(scratch.path());
        let source = packs[0].source();
        assert_eq!(source.read("data/demo/value/root.json").unwrap(), b"root");
        assert_eq!(
            source.read("data/demo/value/both.json").unwrap(),
            b"from the overlay"
        );
        assert_eq!(
            source.read("data/demo/value/only.json").unwrap(),
            b"overlay only"
        );
        assert!(!source.contains("data/demo/value/never.json"));
        assert_eq!(
            packs[0].list("data/demo/value/"),
            [
                "data/demo/value/both.json",
                "data/demo/value/only.json",
                "data/demo/value/root.json"
            ]
        );
    }

    /// A folder pack whose `pack.mcmeta` names an overlay in range of the data pack format and
    /// one out of range: reads take the in-range overlay's file, a listing holds the overlay's
    /// extra file once, the out-of-range overlay is ignored, and a loader built on the pack
    /// sees the overlay's function.
    #[test]
    fn a_folder_pack_s_in_range_overlay_serves_reads_listings_and_the_data_store() {
        let scratch = tempfile::tempdir().unwrap();
        let pack = scratch.path().join("overlaid");
        let format = sunrise_core::version::DATA_PACK_FORMAT.0;
        let meta = format!(
            r#"{{"pack":{{"description":"root"}},"overlays":{{"entries":[
                {{"directory":"in_range","min_format":[{format},0],"max_format":[{format},0]}},
                {{"directory":"out_of_range","min_format":[{0},0],"max_format":[{0},0]}}]}}}}"#,
            format + 1000
        );
        for (path, text) in [
            ("pack.mcmeta", meta.as_str()),
            ("data/test/function/a.mcfunction", "say root"),
            (
                "in_range/pack.mcmeta",
                r#"{"pack":{"description":"overlay"}}"#,
            ),
            ("in_range/data/test/function/a.mcfunction", "say overlay"),
            ("in_range/data/test/function/b.mcfunction", "say b"),
            ("out_of_range/data/test/function/a.mcfunction", "say never"),
            ("out_of_range/data/test/function/c.mcfunction", "say never"),
        ] {
            let at = pack.join(path);
            fs::create_dir_all(at.parent().unwrap()).unwrap();
            fs::write(at, text).unwrap();
        }
        let packs = discover_folder(scratch.path());
        assert_eq!(packs.len(), 1);
        assert_eq!(
            packs[0].read("data/test/function/a.mcfunction").unwrap(),
            b"say overlay"
        );
        assert_eq!(packs[0].description(), "root".into());
        // The overlay folder's own files (its `pack.mcmeta`, its `data/` under its name) are not
        // paths of the pack.
        assert!(
            packs[0]
                .list("")
                .iter()
                .all(|name| !name.starts_with("in_range/"))
        );
        assert_eq!(
            packs[0].list_kinds(&["function"]),
            [
                "data/test/function/a.mcfunction",
                "data/test/function/b.mcfunction"
            ]
        );
        let store = DataStore::load(&DataPacks::new(&[&packs[0]]));
        assert_eq!(store.functions.len(), 2);
        assert!(store.functions.contains_key("test:b"));
        assert!(!store.functions.contains_key("test:c"));
        assert_eq!(
            store.functions["test:a"],
            parse_function("test:a", "say overlay").unwrap()
        );
    }

    fn overlaid_zip(path: &Path, meta: &str) {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        for (name, bytes) in [
            ("pack.mcmeta", meta),
            ("data/test/function/a.mcfunction", "say root"),
            ("in_range/data/test/function/a.mcfunction", "say overlay"),
            ("in_range/data/test/function/b.mcfunction", "say b"),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    /// The same composition for a zip pack, found by `discover_folder` and opened directly
    /// through `DataPack::from_source`, the route feature packs and mod layers take.
    #[test]
    fn a_zip_pack_s_in_range_overlay_serves_reads_and_listings() {
        let scratch = tempfile::tempdir().unwrap();
        let format = sunrise_core::version::DATA_PACK_FORMAT.0;
        let meta = format!(
            r#"{{"pack":{{"description":"root"}},"overlays":{{"entries":[
                {{"directory":"in_range","min_format":[{format},0],"max_format":[{format},0]}}]}}}}"#
        );
        let path = scratch.path().join("overlaid.zip");
        overlaid_zip(&path, &meta);
        let found = discover_folder(scratch.path());
        let direct =
            DataPack::from_source("direct".into(), ResourceSource::archive(&path).unwrap());
        for pack in [&found[0], &direct] {
            assert_eq!(
                pack.read("data/test/function/a.mcfunction").unwrap(),
                b"say overlay"
            );
            assert_eq!(
                pack.list_kinds(&["function"]),
                [
                    "data/test/function/a.mcfunction",
                    "data/test/function/b.mcfunction"
                ]
            );
            assert!(
                pack.list("")
                    .iter()
                    .all(|name| !name.starts_with("in_range/"))
            );
        }
        assert_eq!(found.len(), 1);
    }

    /// `Pack.readPackMetadata` refuses a pack whose `overlays` section fails its codec, folder
    /// or zip alike; a well-formed section next to the same files loads.
    #[test]
    fn a_pack_with_a_bad_overlays_section_is_not_loaded() {
        let format = sunrise_core::version::DATA_PACK_FORMAT.0;
        let good =
            format!(r#"{{"directory":"ok","min_format":[{format},0],"max_format":[{format},0]}}"#);
        let wrap = |entries: &str| {
            format!(r#"{{"pack":{{"description":"d"}},"overlays":{{"entries":[{entries}]}}}}"#)
        };
        let scratch = tempfile::tempdir().unwrap();
        for (name, meta, loads) in [
            ("good", wrap(&good), true),
            (
                "nested_directory",
                wrap(r#"{"directory":"a/b","min_format":[200,0],"max_format":[200,0]}"#),
                false,
            ),
            (
                "bad_formats",
                wrap(&format!(r#"{good},{{"directory":"x","formats":"all"}}"#)),
                false,
            ),
            (
                "formats_only_above_81",
                wrap(&format!(
                    r#"{{"directory":"x","formats":[{format},{format}]}}"#
                )),
                false,
            ),
            (
                "half_range",
                wrap(r#"{"directory":"x","min_format":[200,0]}"#),
                false,
            ),
            (
                "inverted_range",
                wrap(r#"{"directory":"x","min_format":[200,0],"max_format":[199,0]}"#),
                false,
            ),
            (
                "no_entries",
                r#"{"pack":{"description":"d"},"overlays":{}}"#.to_owned(),
                false,
            ),
        ] {
            let dir = scratch.path().join(name);
            fs::create_dir_all(dir.join("data")).unwrap();
            fs::write(dir.join("pack.mcmeta"), &meta).unwrap();
            overlaid_zip(&scratch.path().join(format!("{name}_z.zip")), &meta);
            let found: Vec<String> = discover_folder(scratch.path())
                .into_iter()
                .map(|p| p.id)
                .collect();
            assert_eq!(
                found.contains(&format!("file/{name}")),
                loads,
                "folder {name}"
            );
            assert_eq!(
                found.contains(&format!("file/{name}_z.zip")),
                loads,
                "zip {name}"
            );
            fs::remove_dir_all(&dir).unwrap();
            fs::remove_file(scratch.path().join(format!("{name}_z.zip"))).unwrap();
        }
    }
}
