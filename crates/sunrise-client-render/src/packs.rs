//! Resource packs (PackRepository / FolderRepositorySource): discovery of
//! folder and zip packs in `resourcepacks/` plus the built-in Programmer Art
//! and High Contrast packs, `pack.mcmeta` metadata and format compatibility,
//! and an overlay resource root that layers the enabled packs over the
//! vanilla resources (later packs win), so every loader sees one tree.
mod cache;
mod import;
use crate::text::{Style, Text};
pub use import::copy_pack;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};
pub use sunrise_level::pack_meta::{
    ALLOWED_SYMLINKS_CONFIG_NAME, BLOCKSTATE_LAYERS, DirectoryValidator, Filter, Format, Merge,
    Position, RESOURCE_FORMAT, Selectable, SelectionModel, Source, VANILLA_LANG_DEPRECATIONS,
    append_atlas_sources, atlas_append, compatibility, compatibility_for, compatible, component,
    component_root, data_overlays, detected_pack_path, filter_of, first_json,
    is_downloaded_pack_id, is_overlay_directory, merge_kind, merge_stacks, overlay_applies,
    overlay_applies_for, overlay_directories, parse_meta, pattern, player_pack_ids, read_meta,
    validate_overlays, validate_pack_range,
};

#[derive(Clone, Debug)]
pub struct Pack {
    /// `file/<name>` for user packs, the bare name for built-in ones.
    pub id: String,
    pub source: Source,
    pub title: Text,
    pub description: Text,
    /// The description with its legacy codes, and its root style (the pack screen's).
    pub raw_description: Text,
    pub description_style: Style,
    pub compatible: bool,
    pub compatibility: crate::pack_screen::Compat,
    /// Overlay directories active for this game version, in `pack.mcmeta` order: also those a
    /// Polytone `polytone_condition` turns on.
    pub overlays: Vec<String>,
    /// The values of the pack's Polytone config entries.
    pub polytone_configs: sunrise_polytone::pack::Configs,
    /// `filter.block` entries (namespace and path patterns) hiding resources
    /// of the packs below this one.
    pub filter: Vec<Filter>,
    /// Presentation and selection supplied by an additional repository source.
    pub selection: Option<crate::pack_screen::PackInfo>,
}

fn overlays_of(meta: &serde_json::Value, configs: &sunrise_polytone::pack::Configs) -> Vec<String> {
    overlay_directories(meta, |e| {
        sunrise_polytone::pack::overlay_active(e, configs, overlay_applies_for(e, RESOURCE_FORMAT))
    })
}

impl Pack {
    /// A mod's asset layer, which has no resource-pack metadata.
    pub fn mod_layer(id: String, source: Source) -> Self {
        let polytone_configs = polytone_configs(&source);
        Self {
            title: Text::literal(&id),
            id,
            source,
            description: Text::empty(),
            raw_description: Text::empty(),
            description_style: Style::default(),
            compatible: true,
            compatibility: crate::pack_screen::Compat::Compatible,
            overlays: Vec::new(),
            polytone_configs,
            filter: Vec::new(),
            selection: None,
        }
    }

    /// The pack's `pack.png` (`PackResources.getRootResource`), for its icon.
    pub fn icon_png(&self) -> Option<Vec<u8>> {
        self.source.open().ok()?.read("pack.png").ok()
    }
}

/// A pack's Polytone config entries (`assets/<namespace>/polytone/config_entries/<name>.json`,
/// in the pack's base, not its overlays) at their default values.
pub fn polytone_configs(source: &Source) -> sunrise_polytone::pack::Configs {
    let mut configs = sunrise_polytone::pack::Configs::default();
    let id_of = |name: &str| -> Option<String> {
        let rest = name.strip_prefix("assets/")?;
        let (namespace, rest) = rest.split_once('/')?;
        let entry = rest
            .strip_prefix("polytone/config_entries/")?
            .strip_suffix(".json")?;
        Some(format!("{namespace}:{entry}"))
    };
    let Ok(source) = source.open() else {
        return configs;
    };
    for name in source.list("assets/") {
        if let Some(id) = id_of(&name)
            && let Ok(bytes) = source.read(&name)
            && let Ok(json) = serde_json::from_slice(&bytes)
        {
            configs.add_entry(id, &json);
        }
    }
    for (id, value) in polytone_choices() {
        if configs.entries().contains_key(&id) {
            configs.set(&id, value);
        }
    }
    configs
}

/// A pack's Polytone config entries and one's value, for the game's Pack Options.
pub use sunrise_polytone::pack::{ConfigValue, Configs};

/// The player's Polytone config choices (#11486): what they changed from the packs' defaults,
/// by entry id, for every pack. `polytone_configs` lays them over each pack's defaults.
static POLYTONE_CHOICES: std::sync::RwLock<BTreeMap<String, sunrise_polytone::pack::ConfigValue>> =
    std::sync::RwLock::new(BTreeMap::new());

pub fn polytone_choices() -> BTreeMap<String, sunrise_polytone::pack::ConfigValue> {
    POLYTONE_CHOICES
        .read()
        .map(|c| c.clone())
        .unwrap_or_default()
}

/// Replaces the player's Polytone config choices (packs read them when they load again).
pub fn set_polytone_choices(choices: BTreeMap<String, sunrise_polytone::pack::ConfigValue>) {
    if let Ok(mut c) = POLYTONE_CHOICES.write() {
        *c = choices;
    }
}

/// Where the choices are kept: `polytone-configs.json` beside `options.txt`.
pub fn polytone_choices_file() -> std::path::PathBuf {
    crate::logging::player_dir().join("polytone-configs.json")
}

/// Reads the choices from `path` (none when it is missing or broken).
pub fn load_polytone_choices(path: &Path) -> BTreeMap<String, sunrise_polytone::pack::ConfigValue> {
    use sunrise_polytone::pack::ConfigValue;
    let Ok(bytes) = fs::read(path) else {
        return BTreeMap::new();
    };
    let Ok(serde_json::Value::Object(map)) = serde_json::from_slice(&bytes) else {
        log::warn!("{}: not a JSON object", path.display());
        return BTreeMap::new();
    };
    map.into_iter()
        .filter_map(|(id, v)| {
            let value = match v {
                serde_json::Value::Number(n) => ConfigValue::Number(n.as_f64()?),
                serde_json::Value::String(s) => ConfigValue::Text(s),
                serde_json::Value::Bool(b) => ConfigValue::Bool(b),
                _ => return None,
            };
            Some((id, value))
        })
        .collect()
}

/// Writes the choices to `path`.
pub fn save_polytone_choices(
    path: &Path,
    choices: &BTreeMap<String, sunrise_polytone::pack::ConfigValue>,
) -> std::io::Result<()> {
    use sunrise_polytone::pack::ConfigValue;
    let map: serde_json::Map<String, serde_json::Value> = choices
        .iter()
        .map(|(id, v)| {
            let json = match v {
                ConfigValue::Number(n) => serde_json::json!(n),
                ConfigValue::Text(s) => serde_json::json!(s),
                ConfigValue::Bool(b) => serde_json::json!(b),
            };
            (id.clone(), json)
        })
        .collect();
    let text = serde_json::to_string_pretty(&serde_json::Value::Object(map))
        .map_err(std::io::Error::other)?;
    fs::write(path, text)
}
fn pack(id: String, title: Text, source: Source) -> Option<Pack> {
    pack_for(id, title, source, RESOURCE_FORMAT)
}
/// A pack whose compatibility is against `format`.
pub fn pack_for(id: String, title: Text, source: Source, format: (i64, i64)) -> Option<Pack> {
    let meta = read_meta(&source)?;
    let polytone_configs = polytone_configs(&source);
    let (raw_description, description_style) = component_root(&meta["pack"]["description"]);
    Some(Pack {
        id,
        title,
        description: raw_description.resolve_legacy(),
        raw_description,
        description_style,
        compatible: compatible(&meta),
        compatibility: compatibility_for(&meta, format),
        overlays: overlays_of(&meta, &polytone_configs),
        polytone_configs,
        filter: filter_of(&meta),
        source,
        selection: None,
    })
}

pub fn pack_from_meta(pack: sunrise_level::pack_meta::PackMeta, meta: &serde_json::Value) -> Pack {
    let sunrise_level::pack_meta::PackMeta {
        id,
        title,
        source,
        description,
        raw_description,
        description_style,
        compatible,
        compatibility,
        filter,
        selection,
        ..
    } = pack;
    let polytone_configs = polytone_configs(&source);
    Pack {
        id,
        title,
        source,
        description,
        raw_description,
        description_style,
        compatible,
        compatibility,
        overlays: overlays_of(meta, &polytone_configs),
        polytone_configs,
        filter,
        selection,
    }
}

/// Built-in and `resourcepacks/` packs (vanilla itself excluded), sorted like
/// the pack screen (built-ins first, then by name).
pub fn discover(resources: &Path, packs_dir: &Path) -> Vec<Pack> {
    let mut out = Vec::new();
    for (id, key) in [
        ("programmer_art", "resourcePack.programmer_art.name"),
        ("high_contrast", "resourcePack.high_contrast.name"),
    ] {
        let z = resources.join(format!("assets/minecraft/resourcepacks/{id}.zip"));
        if z.exists()
            && let Some(p) = pack(id.into(), Text::translate(key, &[]), Source::Zip(z))
        {
            out.push(p);
        }
    }
    out.extend(folder(packs_dir, RESOURCE_FORMAT));
    out
}
/// The packs (folders and zips) in `dir`, by name, as `FolderRepositorySource`
/// finds them; their compatibility is against `format`.
pub fn folder(dir: &Path, format: (i64, i64)) -> Vec<Pack> {
    folder_with(dir, format, &DirectoryValidator::game())
}
/// [`folder`] under an explicit `allowed_symlinks.txt` validator.
pub(crate) fn folder_with(
    dir: &Path,
    format: (i64, i64),
    validator: &DirectoryValidator,
) -> Vec<Pack> {
    let mut user: Vec<Pack> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = match detected_pack_path(&e.path(), validator) {
                Ok(Some(path)) => path,
                Ok(None) => return None,
                Err(err) => {
                    log::warn!(
                        "Failed to read properties of '{}', ignoring: {err}",
                        e.path().display()
                    );
                    return None;
                }
            };
            let source = if path.is_dir() {
                Source::Dir(path)
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
            {
                Source::Zip(path)
            } else {
                return None;
            };
            pack_for(format!("file/{name}"), Text::literal(name), source, format)
        })
        .collect();
    user.sort_by(|a, b| a.id.cmp(&b.id));
    user
}

/// A file a pack provides: its path (folder packs) and its name inside the pack.
type Provided = (
    Option<PathBuf>,
    crate::resource_source::ResourceSource,
    String,
);
/// Files a pack provides under `assets/`, keyed by their resource name; active
/// overlay directories are applied after the base, in order, later wins.
fn pack_files(
    pack: &Pack,
    source: &crate::resource_source::ResourceSource,
) -> io::Result<BTreeMap<String, Provided>> {
    let mut out = BTreeMap::new();
    for root in std::iter::once(String::new()).chain(
        pack.overlays
            .iter()
            .map(|directory| format!("{directory}/")),
    ) {
        let source = source.subdirectory(root.trim_end_matches('/'))?;
        for inner in source.list("assets/") {
            let path = match &pack.source {
                Source::Dir(directory) => Some(directory.join(&root).join(&inner)),
                Source::Zip(_) | Source::Opened(_) => None,
            };
            out.insert(inner.clone(), (path, source.clone(), inner));
        }
    }
    Ok(out)
}

#[cfg(unix)]
fn link(target: &Path, at: &Path) -> io::Result<()> {
    // Relative sources are relative to the process, not the overlay directory.
    std::os::unix::fs::symlink(std::path::absolute(target)?, at)
}
#[cfg(not(unix))]
fn link(target: &Path, at: &Path) -> io::Result<()> {
    if target.is_dir() {
        Ok(())
    } else {
        // The mirror only reads these files, and unix links them: a hardlink gives
        // the same view without writing the 639 MB prepared root again, which was
        // 33 s of a cold Windows launch with a pack on. Another volume (or a
        // filesystem without hardlinks) still copies.
        fs::hard_link(target, at).or_else(|_| fs::copy(target, at).map(|_| ()))
    }
}
/// A directory junction (`sunrise_threads::junction`); the mirror falls back to copying the
/// tree when it fails.
#[cfg(windows)]
fn junction(target: &Path, at: &Path) -> io::Result<()> {
    sunrise_threads::junction(target, at)
}
#[cfg(not(windows))]
fn junction(_: &Path, _: &Path) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}
/// Build `out` as `vanilla` with the packs' files layered on top (packs in
/// load order, last wins). Untouched directories are links into vanilla.
pub fn build_overlay(vanilla: &Path, packs: &[Pack], out: &Path) -> io::Result<()> {
    cache::build(vanilla, packs, out)
}

struct ResolvedPacks {
    files: BTreeMap<String, (usize, Provided)>,
    stacks: BTreeMap<String, Vec<(usize, Provided)>>,
    /// Every pack's copy of each file, lowest pack first, after the packs' filters: what a
    /// resource manager's `getResourceStack` gives above vanilla's own.
    layers: BTreeMap<String, Vec<(usize, Provided)>>,
    hidden: BTreeSet<String>,
}

fn resolve(packs: &[Pack]) -> io::Result<ResolvedPacks> {
    let sources: Vec<_> = packs
        .iter()
        .map(|pack| pack.source.open())
        .collect::<io::Result<_>>()?;
    // Final owner of every overridden file.
    let mut files: BTreeMap<String, (usize, Provided)> = BTreeMap::new();
    // Every pack's copy of the files vanilla reads as a resource stack.
    let mut stacks: BTreeMap<String, Vec<(usize, Provided)>> = BTreeMap::new();
    let mut layers: BTreeMap<String, Vec<(usize, Provided)>> = BTreeMap::new();
    for (i, (p, source)) in packs.iter().zip(&sources).enumerate() {
        // A pack's filter hides what the packs below it provide.
        files.retain(|name, _| !p.filter.iter().any(|f| f.blocks(name)));
        stacks.retain(|name, _| !p.filter.iter().any(|f| f.blocks(name)));
        if !p.filter.is_empty() {
            layers.retain(|name, _| !p.filter.iter().any(|f| f.blocks(name)));
        }
        for (name, provided) in pack_files(p, source)? {
            layers
                .entry(name.clone())
                .or_default()
                .push((i, provided.clone()));
            if merge_kind(&name).is_some() {
                stacks
                    .entry(name.clone())
                    .or_default()
                    .push((i, provided.clone()));
            }
            files.insert(name, (i, provided));
        }
    }
    // FallbackResourceManager: a file's `.mcmeta` counts only from the pack that
    // supplies the file or one above it; vanilla's own sits below every pack.
    let mut hidden: BTreeSet<String> = files
        .iter()
        .filter(|(name, _)| !name.ends_with(".mcmeta"))
        .filter_map(|(name, (owner, _))| {
            let meta = format!("{name}.mcmeta");
            match files.get(&meta) {
                Some((meta_owner, _)) if meta_owner >= owner => None,
                _ => Some(meta),
            }
        })
        .collect();
    // A companion from below the winning colour would describe another surface.
    // A map-only pack above it is intentional and remains valid.
    for (name, (owner, _)) in &files {
        let Some(stem) = name.strip_suffix(".png") else {
            continue;
        };
        if stem.ends_with("_n") || stem.ends_with("_s") {
            continue;
        }
        for suffix in ["_n.png", "_s.png"] {
            let map = format!("{stem}{suffix}");
            if files
                .get(&map)
                .is_none_or(|(map_owner, _)| map_owner < owner)
            {
                hidden.insert(map);
            }
        }
    }
    for name in &hidden {
        files.remove(name);
    }
    Ok(ResolvedPacks {
        files,
        stacks,
        layers,
        hidden,
    })
}

/// Resolve a pack selection without creating an overlay directory. Files keep
/// their open source; only vanilla's resource-stack merges are held as bytes.
/// Diverges from upstream: jars and zip packs remain in place during loading.
pub fn resources(
    base: crate::resource_source::ResourceSource,
    packs: &[Pack],
) -> io::Result<crate::resource_source::ResourceSource> {
    use crate::resource_source::{Resource, ResourceSource};
    if packs.is_empty() {
        return Ok(base);
    }
    let ResolvedPacks {
        files,
        stacks,
        layers,
        hidden,
    } = resolve(packs)?;
    let filters: Vec<_> = packs.iter().flat_map(|pack| &pack.filter).collect();
    let merged = merged_resources(&base, packs, &stacks);
    let index_base = |source: &ResourceSource| -> BTreeMap<_, _> {
        source
            .list("")
            .into_iter()
            .filter(|name| {
                !filters.iter().any(|filter| filter.blocks(name))
                    && name != sunrise_core::prepared_files::FILE_NAME
            })
            .map(|name| (name.clone(), Resource::file(source.clone(), name)))
            .collect()
    };
    let mut index = index_base(&base);
    // The same resolver already keeps each pack's original files in order.
    // Publish those winners before the renderer hides or synthesizes files.
    let raw_base = base.raw_pack_files();
    let mut raw = if base.same_generation(raw_base) {
        index.clone()
    } else {
        index_base(raw_base)
    };
    raw.extend(layers.iter().filter_map(|(name, provided)| {
        let (_, (_, source, inner)) = provided.last()?;
        Some((name.clone(), Resource::file(source.clone(), inner.clone())))
    }));
    index.retain(|name, _| !hidden.contains(name));
    for (name, (_, (_, source, inner))) in files {
        index.insert(name, Resource::file(source, inner));
    }
    for (name, bytes) in merged {
        index.insert(name, Resource::bytes(bytes));
    }
    let deprecated = VANILLA_LANG_DEPRECATIONS;
    // These renames come from the game, not its resource packs.
    if base.contains(deprecated) {
        index.insert(
            deprecated.to_owned(),
            Resource::file(base.clone(), deprecated.to_owned()),
        );
    } else {
        index.remove(deprecated);
    }
    // Every layer of the files the packs provide: the layers below `base` (it may be stacked
    // itself), then each pack's copy. What `base` stacked and no pack provides stays as it was.
    let mut stacked = base.layers().cloned().unwrap_or_default();
    for (name, provided) in layers {
        let below = if filters.iter().any(|filter| filter.blocks(&name)) {
            Vec::new()
        } else {
            base.stack(&name)
        };
        let copies = provided
            .into_iter()
            .map(|(_, (_, source, inner))| Resource::file(source, inner));
        stacked.insert(name, below.into_iter().chain(copies).collect());
    }
    stacked.retain(|name, _| index.contains_key(name));
    Ok(ResourceSource::indexed_with_layers(
        index,
        stacked,
        Some(ResourceSource::indexed(raw)),
    ))
}

fn merged_resources(
    base: &crate::resource_source::ResourceSource,
    packs: &[Pack],
    stacks: &BTreeMap<String, Vec<(usize, Provided)>>,
) -> BTreeMap<String, Vec<u8>> {
    let filters: Vec<_> = packs.iter().flat_map(|pack| &pack.filter).collect();
    let mut merged = BTreeMap::new();
    for (name, stack) in stacks {
        let mut layers = Vec::new();
        if !filters.iter().any(|filter| filter.blocks(name)) {
            layers.extend(base.read(name).ok());
        }
        for (owner, (_, source, inner)) in stack {
            let bytes = source.read(inner).ok();
            let bytes = match (bytes, merge_kind(name)) {
                (Some(bytes), Some(Merge::Sounds)) => Some(
                    packs
                        .get(*owner)
                        .and_then(|pack| {
                            sunrise_polytone::pack::sounds_without_unmet(
                                &bytes,
                                &pack.polytone_configs,
                            )
                        })
                        .unwrap_or(bytes),
                ),
                (bytes, _) => bytes,
            };
            layers.extend(bytes);
        }
        if let Some(kind) = merge_kind(name)
            && let Some(bytes) = merge(kind, &layers)
        {
            merged.insert(name.clone(), bytes);
        }
    }
    merged
}

fn build_overlay_uncached(
    vanilla: &Path,
    packs: &[Pack],
    out: &Path,
    dependencies: &mut cache::Dependencies,
) -> io::Result<()> {
    if out.exists() {
        fs::remove_dir_all(out)?;
    }
    fs::create_dir_all(out)?;
    let ResolvedPacks {
        files,
        stacks,
        hidden,
        ..
    } = resolve(packs)?;
    // Vanilla sits below every pack, so filters hide its files too.
    let filters: Vec<&Filter> = packs.iter().flat_map(|p| &p.filter).collect();
    for name in stacks.keys() {
        dependencies.file(&vanilla.join(name));
    }
    let merged = merged_resources(
        &crate::resource_source::ResourceSource::directory(vanilla.to_owned()),
        packs,
        &stacks,
    );
    let touched_dirs: BTreeSet<String> = files
        .keys()
        .flat_map(|f| {
            let parts: Vec<&str> = f.split('/').collect();
            (1..parts.len()).map(move |n| parts[..n].join("/"))
        })
        .collect();
    /// A zip entry to write: its pack, its name in the zip and where it goes.
    type Extract<'a> = (&'a crate::resource_source::ResourceSource, &'a str, PathBuf);
    #[allow(clippy::too_many_arguments)]
    fn mirror<'a>(
        rel: &str,
        vanilla: &Path,
        out: &Path,
        touched: &BTreeSet<String>,
        hidden: &BTreeSet<String>,
        files: &'a BTreeMap<String, (usize, Provided)>,
        merged: &BTreeMap<String, Vec<u8>>,
        filters: &[&Filter],
        dependencies: &mut cache::Dependencies,
        extract: &mut Vec<Extract<'a>>,
    ) -> io::Result<()> {
        let src = if rel.is_empty() {
            vanilla.to_path_buf()
        } else {
            vanilla.join(rel)
        };
        let dst = if rel.is_empty() {
            out.to_path_buf()
        } else {
            out.join(rel)
        };
        fs::create_dir_all(&dst)?;
        dependencies.directory(&src);
        let mut names: BTreeSet<String> = BTreeSet::new();
        if let Ok(entries) = fs::read_dir(&src) {
            names.extend(
                entries
                    .flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned()),
            );
        }
        let prefix = if rel.is_empty() {
            String::new()
        } else {
            format!("{rel}/")
        };
        // The files under this directory are one run of the sorted map: walking only that run
        // keeps a pack of tens of thousands of files in thousands of folders from being
        // scanned once per folder.
        for f in files
            .range(prefix.clone()..)
            .map_while(|(f, _)| f.strip_prefix(&prefix))
        {
            names.insert(f.split('/').next().unwrap_or("").to_owned());
        }
        for name in names {
            let child = format!("{prefix}{name}");
            if child == VANILLA_LANG_DEPRECATIONS {
                let base = vanilla.join(&child);
                if base.is_file() {
                    dependencies.file(&base);
                    link(&base, &out.join(&child))?;
                }
                continue;
            }
            // Vanilla's files in one file: it describes vanilla's root, not this mirror.
            if hidden.contains(&child) || child == sunrise_core::prepared_files::FILE_NAME {
                continue;
            }
            if let Some(bytes) = merged.get(&child) {
                fs::write(out.join(&child), bytes)?;
            } else if let Some((_, (path, source, inner))) = files.get(&child) {
                let target = out.join(&child);
                match path {
                    Some(p) => link(p, &target)?,
                    // Inflated and written after the walk, on every thread.
                    None => extract.push((source, inner.as_str(), target)),
                }
            } else if filters.is_empty()
                && !touched.contains(&child)
                && vanilla.join(&child).is_dir()
                && junction(&vanilla.join(&child), &out.join(&child)).is_ok()
            {
                // Linked whole, like the unix symlink: no file in it is visited or copied.
            } else if touched.contains(&child)
                // Without a junction Windows cannot link a directory the way unix does (`link`
                // above), so an untouched one has to be mirrored: skipping it hid textures/font
                // and the title panorama from the overlay, and a font json that names those
                // sheets then failed to load (#1688).
                || (vanilla.join(&child).is_dir() && (cfg!(not(unix)) || !filters.is_empty()))
            {
                mirror(
                    &child,
                    vanilla,
                    out,
                    touched,
                    hidden,
                    files,
                    merged,
                    filters,
                    dependencies,
                    extract,
                )?;
            } else if filters.iter().any(|f| f.blocks(&child)) {
                // Hidden by a pack's filter.
            } else {
                #[cfg(not(unix))]
                if vanilla.join(&child).is_file() {
                    dependencies.file(&vanilla.join(&child));
                }
                link(&vanilla.join(&child), &out.join(&child))?;
            }
        }
        Ok(())
    }
    let mut extract = Vec::new();
    mirror(
        "",
        vanilla,
        out,
        &touched_dirs,
        &hidden,
        &files,
        &merged,
        &filters,
        dependencies,
        &mut extract,
    )?;
    // What the zips may unpack to, all threads together: a server's pack is a download the
    // player only accepts, and a gigabyte of zeros deflates to a megabyte, so without it a pack
    // at the download limit wrote hundreds of gigabytes here. Vanilla never unpacks a pack.
    let budget = std::sync::atomic::AtomicU64::new(unpack_budget(packs));
    // Inflating and writing a big pack's files one by one was most of a cold build; its
    // folders all exist by now.
    let extracted: io::Result<()> = crate::par::par_map(&extract, |(source, inner, target)| {
        let copied = fs::File::create(target).and_then(|file| {
            let mut file = io::BufWriter::new(file);
            source.copy_to(inner, &mut file, &budget)?;
            io::Write::flush(&mut file)
        });
        match copied {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::FileTooLarge => Err(e),
            Err(e) => {
                log::warn!("Sunrise: skipping unreadable pack entry {inner}: {e}");
                let _ = fs::remove_file(target);
                Ok(())
            }
        }
    })
    .into_iter()
    .collect();
    if extracted.is_err() {
        // What was unpacked before the budget ran out goes too, not only the marker.
        let _ = fs::remove_dir_all(out);
    }
    extracted
}
/// The most the zip packs in `packs` may unpack to in one overlay: 16 times their size on disk
/// (real packs are mostly PNGs and JSON, a few times smaller zipped at most), at least 64 MiB
/// for a small pack of text, and never past 8 GiB.
fn unpack_budget(packs: &[Pack]) -> u64 {
    packs
        .iter()
        .filter_map(|p| match &p.source {
            Source::Zip(z) => fs::metadata(z).ok().map(|m| m.len()),
            // Open resource layers need not have a filesystem archive. They
            // share the absolute ceiling, not a guessed compressed size.
            Source::Opened(_) => Some(8 << 30),
            Source::Dir(_) => None,
        })
        .sum::<u64>()
        .saturating_mul(16)
        .clamp(64 << 20, 8 << 30)
}

/// One file with the effect of loading `layers` (bottom first) as vanilla
/// does. Layers that do not parse are skipped, as vanilla logs and skips them.
fn merge(kind: Merge, layers: &[Vec<u8>]) -> Option<Vec<u8>> {
    use serde_json::{Map, Value};
    if kind == Merge::Lang {
        // The client reads each original file in its own order. A non-primitive
        // ends that file, not the rest of the stack; merging JSON objects first
        // can both lose and admit the wrong entries.
        let mut strings = std::collections::HashMap::new();
        for layer in layers {
            sunrise_game_data::lang::read_lang_file(layer, &mut strings);
        }
        return serde_json::to_vec(&Map::from_iter(
            strings
                .into_iter()
                .map(|(key, value)| (key, Value::String(value))),
        ))
        .ok();
    }
    let mut parsed: Vec<Map<String, Value>> = layers
        .iter()
        // SoundManager skips a sounds.json Gson rejects, whole.
        .filter(|b| kind != Merge::Sounds || crate::sound_events::is_valid_file(b))
        .filter_map(|b| serde_json::from_slice(b).ok())
        .collect();
    let list = |key: &str, layers: &mut dyn Iterator<Item = &Map<String, Value>>| {
        let items: Vec<Value> = layers
            .filter_map(|l| l.get(key).and_then(Value::as_array))
            .flatten()
            .cloned()
            .collect();
        Value::Object(Map::from_iter([(key.to_owned(), Value::Array(items))]))
    };
    let out = match kind {
        Merge::Lang => unreachable!("language files are read before merging"),
        Merge::Sounds => {
            let mut events = Map::new();
            for (event, def) in parsed.into_iter().flatten() {
                if !events.contains_key(&event)
                    || crate::sound_events::json_bool(&def["replace"], false)
                {
                    let mut fresh = Map::new();
                    if let Some(subtitle) = def.get("subtitle") {
                        fresh.insert("subtitle".into(), subtitle.clone());
                    }
                    fresh.insert("sounds".into(), Value::Array(Vec::new()));
                    events.insert(event.clone(), Value::Object(fresh));
                }
                if let (Some(Value::Array(sounds)), Some(Value::Array(more))) = (
                    events.get_mut(&event).and_then(|e| e.get_mut("sounds")),
                    def.get("sounds"),
                ) {
                    sounds.extend(more.iter().cloned());
                }
            }
            Value::Object(events)
        }
        Merge::Font => list("providers", &mut parsed.iter().rev()),
        // SpriteSourceList: the bottom file whole, each layer's sources
        // appended to its own, from the bottom up. A layer whose sources
        // are not a list is skipped, as vanilla logs and skips the file.
        Merge::Atlas => {
            // Each definition stands or falls on its own: one whose sources don't decode is
            // dropped whole, and the others' sources still run.
            let mut layers = parsed.into_iter().filter(|l| {
                crate::atlas_sources::decode(
                    &Value::Object(l.clone()).to_string(),
                    &crate::mod_sprite_sources::Registered,
                )
                .is_ok()
            });
            let mut doc = layers.next()?;
            let added = layers
                .filter_map(|l| l.get("sources").and_then(Value::as_array).cloned())
                .flatten()
                .collect();
            atlas_append(&mut doc, added)?;
            Value::Object(doc)
        }
        // One definition is the file as it is.
        Merge::BlockStates if parsed.len() < 2 => Value::Object(parsed.pop()?),
        Merge::BlockStates => Value::Object(Map::from_iter([(
            BLOCKSTATE_LAYERS.to_owned(),
            Value::Array(parsed.into_iter().map(Value::Object).collect()),
        )])),
    };
    serde_json::to_vec(&out).ok()
}

/// `DownloadedPackSource.loadRequestedPacks`: a server's downloaded pack,
/// "World Specific Resources"; None when its `pack.mcmeta` can't be read. Its
/// id names the file, which is named after its hash.
pub fn server_pack(id: u128, path: &Path) -> Option<Pack> {
    let file = path.file_name()?.to_string_lossy();
    pack(
        format!("server/{id:032x}/{file}"),
        Text::translate("resourcePack.server.name", &[]),
        Source::Zip(path.to_path_buf()),
    )
}
/// The packs to apply for an options.txt `resourcePacks` list (bottom first).
pub fn enabled(all: &[Pack], ids: &[String]) -> Vec<Pack> {
    let available = all
        .iter()
        .map(|pack| {
            pack.selection.as_ref().map_or_else(
                || Selectable::optional(&pack.id, pack.compatible),
                crate::pack_screen::PackInfo::selectable,
            )
        })
        .collect();
    SelectionModel::new(available, ids)
        .selected
        .into_iter()
        .rev()
        .filter_map(|selected| all.iter().find(|p| p.id == selected.id).cloned())
        .collect()
}
/// NeoForge's `Minecraft.rollbackResourcePacks`: recover only when a selected
/// client pack can be disabled. Reordering required packs does not count.
pub fn has_optional_pack(selected: &[Pack]) -> bool {
    selected
        .iter()
        .any(|pack| !pack.selection.as_ref().is_some_and(|info| info.required))
}
/// Resource root for a pack selection: vanilla itself when no pack is on.
pub fn resource_root(vanilla: &Path, packs: &[Pack], cache: &Path) -> io::Result<PathBuf> {
    if packs.is_empty() {
        return Ok(vanilla.to_path_buf());
    }
    let key: String = packs
        .iter()
        .map(|p| p.id.as_str())
        .collect::<Vec<_>>()
        .join("|");
    let hash = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
    });
    let out = cache.join(format!("{hash:016x}"));
    cache::hold(&out)?;
    build_overlay(vanilla, packs, &out)?;
    // Old selections' overlays would otherwise stay for good, each a whole unpacked copy.
    cache::prune(cache);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_language_resource_is_the_winner_not_the_render_merge() {
        use crate::resource_source::{Resource, ResourcePath, ResourceSource};
        use std::io::Read;

        let name = "assets/demo/lang/en_us.json";
        let winner = b"{ \"high\": \"winner\" }\n";
        let base = ResourceSource::indexed(BTreeMap::from([(
            name.into(),
            Resource::bytes(br#"{"low":"base"}"#.to_vec()),
        )]));
        let upper = ResourceSource::indexed(BTreeMap::from([(
            name.into(),
            Resource::bytes(winner.to_vec()),
        )]));
        let composed = resources(
            base,
            &[Pack::mod_layer("top".into(), Source::Opened(upper))],
        )
        .unwrap();
        // Applying another selection over this view must not flatten its raw files.
        let selected = resources(
            composed,
            &[Pack::mod_layer(
                "empty".into(),
                Source::Opened(ResourceSource::default()),
            )],
        )
        .unwrap();
        let rendered: serde_json::Value =
            serde_json::from_slice(&selected.read(name).unwrap()).unwrap();
        assert_eq!(rendered, serde_json::json!({"low":"base", "high":"winner"}));
        let mut raw = Vec::new();
        selected
            .raw_pack_files()
            .open_resource(ResourcePath::new(name).unwrap())
            .unwrap()
            .unwrap()
            .read_to_end(&mut raw)
            .unwrap();
        assert_eq!(raw, winner);
    }

    #[test]
    fn raw_language_winner_reports_archive_read_failure_instead_of_lower_bytes() {
        use crate::resource_source::{Resource, ResourcePath, ResourceSource};
        use std::io::{Cursor, Read, Write};

        let name = "assets/demo/lang/en_us.json";
        let base = ResourceSource::indexed(BTreeMap::from([(
            name.into(),
            Resource::bytes(br#"{"low":"base"}"#.to_vec()),
        )]));
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file(name, options).unwrap();
        zip.write_all(br#"{"high":"winner"}"#).unwrap();
        let mut bytes = zip.finish().unwrap().into_inner();
        let offset = {
            let mut zip = zip::ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
            zip.by_index_raw(0).unwrap().data_start().unwrap() as usize
        };
        bytes[offset] = 0b0000_0111; // Reserved DEFLATE block type.
        let archive = ResourceSource::opened_archive(crate::zip::Zip::from_bytes(bytes).unwrap());
        let selected = resources(
            base,
            &[Pack::mod_layer("broken".into(), Source::Opened(archive))],
        )
        .unwrap();
        let rendered: serde_json::Value =
            serde_json::from_slice(&selected.read(name).unwrap()).unwrap();
        assert_eq!(rendered, serde_json::json!({"low":"base"}));
        let mut reader = selected
            .raw_pack_files()
            .open_resource(ResourcePath::new(name).unwrap())
            .unwrap()
            .unwrap();
        assert!(reader.read_to_end(&mut Vec::new()).is_err());
    }

    /// A resource manager's `getResourceStack`: every visible copy, lowest first, the packs'
    /// own files (not the merged document the game reads of a stacked kind), with a pack's
    /// filter hiding what is below it.
    #[test]
    fn a_resource_stack_keeps_every_visible_original_lowest_first() {
        use crate::resource_source::ResourceSource;
        let root = std::env::temp_dir().join(format!("sunrise-stack-packs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let write = |pack: &str, name: &str, bytes: &str| {
            let path = root.join(pack).join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        };
        let (plain, lang) = ("assets/demo/plain.txt", "assets/demo/lang/en_us.json");
        for (pack, plain_text, lang_text) in [
            ("vanilla", "base", r#"{"a":"base"}"#),
            ("one", "one", r#"{"a":"one","b":"one"}"#),
            ("two", "two", r#"{"b":"two"}"#),
        ] {
            write(pack, plain, plain_text);
            write(pack, lang, lang_text);
        }
        let one = Pack::mod_layer("one".into(), Source::Dir(root.join("one")));
        let mut two = Pack::mod_layer("two".into(), Source::Dir(root.join("two")));
        let loaded = resources(
            ResourceSource::directory(root.join("vanilla")),
            &[one.clone(), two.clone()],
        )
        .unwrap();
        let originals = |source: &ResourceSource, name: &str| -> Vec<String> {
            source
                .stack(name)
                .iter()
                .map(|layer| String::from_utf8(layer.read().unwrap()).unwrap())
                .collect()
        };
        assert_eq!(originals(&loaded, plain), ["base", "one", "two"]);
        assert_eq!(
            originals(&loaded, lang),
            [
                r#"{"a":"base"}"#,
                r#"{"a":"one","b":"one"}"#,
                r#"{"b":"two"}"#
            ],
            "the originals of a file the game merges"
        );
        assert_eq!(
            loaded.read_to_string(lang).unwrap(),
            r#"{"a":"one","b":"two"}"#,
            "while the game reads the merge"
        );
        // A stack over a stacked base keeps the base's layers below the new pack's.
        let over = resources(
            loaded.clone(),
            &[Pack::mod_layer("one".into(), Source::Dir(root.join("one")))],
        )
        .unwrap();
        assert_eq!(originals(&over, plain), ["base", "one", "two", "one"]);
        // A filter on the top pack hides every copy below it.
        two.filter = filter_of(
            &serde_json::json!({"filter":{"block":[{"namespace":"demo","path":"plain\\.txt"}]}}),
        );
        let hidden =
            resources(ResourceSource::directory(root.join("vanilla")), &[one, two]).unwrap();
        assert_eq!(originals(&hidden, plain), ["two"]);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn indexed_layers_keep_priority_filters_metadata_and_reload_without_unpacking() {
        use crate::resource_source::ResourceSource;
        use std::io::Write;
        let root =
            std::env::temp_dir().join(format!("sunrise-indexed-packs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let vanilla = root.join("vanilla");
        let low = root.join("low");
        let maps = root.join("maps");
        let write = |base: &Path, name: &str, bytes: &[u8]| {
            let path = base.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        };
        for (name, bytes) in [
            ("assets/demo/common.txt", b"vanilla".as_slice()),
            ("assets/demo/hidden.txt", b"vanilla"),
            ("assets/demo/only_base.txt", b"base-only"),
            ("assets/demo/textures/stone.png", b"base-colour"),
            ("assets/demo/textures/stone_n.png", b"base-normal"),
            ("assets/demo/textures/stone_s.png", b"base-specular"),
            ("assets/demo/textures/stone.png.mcmeta", b"old-metadata"),
            (
                "assets/demo/lang/en_us.json",
                br#"{"vanilla":"v","key":"v"}"#,
            ),
            ("tables.json", b"tables"),
        ] {
            write(&vanilla, name, bytes);
        }
        write(&low, "assets/demo/common.txt", b"folder");
        write(&low, "assets/demo/hidden.txt", b"folder");
        write(&low, "assets/demo/lang/en_us.json", br#"{"key":"folder"}"#);
        write(&maps, "assets/demo/textures/stone_n.png", b"new-normal");
        let archive_path = root.join("passive.jar");
        let archive = |path: &Path, replacement: &[u8]| {
            let mut writer = zip::ZipWriter::new(fs::File::create(path).unwrap());
            for (name, bytes) in [
                ("assets/demo/common.txt", b"jar".as_slice()),
                ("new/assets/demo/common.txt", replacement),
                ("assets/demo/hidden.txt", b"own-filtered-name"),
                ("assets/demo/textures/stone.png", b"jar-colour"),
                ("assets/demo/lang/en_us.json", br#"{"jar":"j"}"#),
                ("assets/../../escape.txt", b"unsafe"),
                ("Ignored.class", b"passive bytes"),
            ] {
                writer
                    .start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                writer.write_all(bytes).unwrap();
            }
            writer.finish().unwrap();
        };
        archive(&archive_path, b"overlay");
        let low = Pack::mod_layer("folder".into(), Source::Dir(low));
        let mut jar = Pack::mod_layer("jar".into(), Source::Zip(archive_path.clone()));
        jar.overlays = vec!["new".into()];
        jar.filter = filter_of(
            &serde_json::json!({"filter":{"block":[{"namespace":"demo","path":"hidden\\.txt"}]}}),
        );
        let base = ResourceSource::directory(vanilla);
        let before = ResourceSource::directory(root.clone()).list("");
        let loaded = resources(base.clone(), &[low.clone(), jar.clone()]).unwrap();
        assert_eq!(loaded.read("assets/demo/common.txt").unwrap(), b"overlay");
        // A pack's own files survive its filter; only lower copies are hidden.
        assert_eq!(
            loaded.read("assets/demo/hidden.txt").unwrap(),
            b"own-filtered-name"
        );
        assert_eq!(
            loaded.read("assets/demo/only_base.txt").unwrap(),
            b"base-only"
        );
        assert_eq!(loaded.read("tables.json").unwrap(), b"tables");
        let lang: serde_json::Value =
            serde_json::from_slice(&loaded.read("assets/demo/lang/en_us.json").unwrap()).unwrap();
        assert_eq!(
            lang,
            serde_json::json!({"vanilla":"v","key":"folder","jar":"j"})
        );
        for hidden in [
            "assets/demo/textures/stone_n.png",
            "assets/demo/textures/stone_s.png",
            "assets/demo/textures/stone.png.mcmeta",
            "Ignored.class",
        ] {
            assert_eq!(
                loaded.read(hidden).unwrap_err().kind(),
                io::ErrorKind::NotFound
            );
            assert!(!loaded.list("").iter().any(|name| name == hidden));
        }
        assert_eq!(ResourceSource::directory(root.clone()).list(""), before);
        assert!(!root.join("assets").exists());
        assert!(!root.join("escape.txt").exists());
        let maps = Pack::mod_layer("maps".into(), Source::Dir(maps));
        let with_maps = resources(base.clone(), &[low.clone(), jar.clone(), maps]).unwrap();
        assert_eq!(
            with_maps.read("assets/demo/textures/stone_n.png").unwrap(),
            b"new-normal"
        );
        let mut filter_only = jar.clone();
        filter_only.source = Source::Dir(root.join("empty"));
        filter_only.overlays.clear();
        let filtered = resources(base.clone(), &[low.clone(), filter_only]).unwrap();
        assert_eq!(
            filtered.read("assets/demo/hidden.txt").unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert!(
            !filtered
                .list("assets/demo/")
                .iter()
                .any(|name| name.ends_with("hidden.txt"))
        );
        let replacement = root.join("replacement.jar");
        archive(&replacement, b"reloaded");
        fs::rename(&archive_path, root.join("old.jar")).unwrap();
        fs::rename(replacement, &archive_path).unwrap();
        let reloaded = resources(base, &[low, jar]).unwrap();
        assert_eq!(
            reloaded.read("assets/demo/common.txt").unwrap(),
            b"reloaded"
        );
        assert_eq!(loaded.read("assets/demo/common.txt").unwrap(), b"overlay");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_sound_entry_whose_polytone_condition_fails_is_left_out() {
        let base =
            std::env::temp_dir().join(format!("sunrise-packs-sound-cond-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let (vanilla, dir, out) = (base.join("vanilla"), base.join("pack"), base.join("out"));
        fs::create_dir_all(vanilla.join("assets/minecraft")).unwrap();
        fs::write(
            vanilla.join("assets/minecraft/sounds.json"),
            r#"{"block.anvil.land":{"sounds":["vanilla"]}}"#,
        )
        .unwrap();
        let configs = dir.join("assets/mypack/polytone/config_entries");
        fs::create_dir_all(&configs).unwrap();
        fs::create_dir_all(dir.join("assets/minecraft")).unwrap();
        fs::write(configs.join("quiet.json"), r#"{"default_value": "OFF"}"#).unwrap();
        fs::write(
            dir.join("pack.mcmeta"),
            r#"{"pack":{"description":"p","pack_format":97}}"#,
        )
        .unwrap();
        fs::write(
            dir.join("assets/minecraft/sounds.json"),
            r#"{"block.anvil.land":{"replace":true,"polytone_condition":"config('mypack:quiet') == 'ON'","sounds":["soft"]},
                "block.anvil.use":{"sounds":["kept"]}}"#,
        )
        .unwrap();
        let pack = pack("file/p".into(), Text::literal("p"), Source::Dir(dir)).unwrap();
        build_overlay(&vanilla, &[pack], &out).unwrap();
        let merged: serde_json::Value =
            serde_json::from_slice(&fs::read(out.join("assets/minecraft/sounds.json")).unwrap())
                .unwrap();
        // The condition is false: the pack's entry doesn't replace vanilla's.
        assert_eq!(
            merged["block.anvil.land"]["sounds"],
            serde_json::json!(["vanilla"])
        );
        assert_eq!(
            merged["block.anvil.use"]["sounds"],
            serde_json::json!(["kept"])
        );
        let _ = fs::remove_dir_all(base);
    }
    #[test]
    fn mod_pack_languages_keep_each_files_primitive_prefix() {
        use crate::resource_source::{Resource, ResourceSource};
        let make_source = |data: &str| {
            ResourceSource::indexed(BTreeMap::from([(
                "assets/test/lang/en_us.json".to_owned(),
                Resource::bytes(data.as_bytes().to_vec()),
            )]))
        };
        let base = make_source(r#"{"a":"old","m":"still here"}"#);
        let lower = Pack::mod_layer(
            "lower".into(),
            Source::Opened(make_source(
                r#"{"z":1.50,"b":{"text":"not a client value"},"a":"unread"}"#,
            )),
        );
        let upper = Pack::mod_layer(
            "upper".into(),
            Source::Opened(make_source(r#"{"h":"higher pack still loads"}"#)),
        );
        let merged = resources(base, &[lower, upper]).unwrap();
        let language = sunrise_game_data::lang::Language::load(&[&merged], "en_us");
        assert_eq!(language.get("a"), Some("old"));
        assert_eq!(language.get("m"), Some("still here"));
        assert_eq!(language.get("z"), Some("1.50"));
        assert_eq!(language.get("h"), Some("higher pack still loads"));
        assert_eq!(language.get("b"), None);
    }

    #[test]
    fn stacked_resources_merge_like_vanilla() {
        let json = |v: &str| v.as_bytes().to_vec();
        let parse = |b: Vec<u8>| serde_json::from_slice::<serde_json::Value>(&b).unwrap();
        // ClientLanguage: later packs override keys, the rest stays.
        let lang = merge(
            Merge::Lang,
            &[
                json(r#"{"a":"vanilla a","b":"vanilla b"}"#),
                json(r#"{"b":"pack b"}"#),
            ],
        );
        assert_eq!(
            parse(lang.unwrap()),
            serde_json::json!({"a":"vanilla a","b":"pack b"})
        );
        // A layer with `#` comments is read leniently, not dropped.
        let lenient = merge(
            Merge::Lang,
            &[
                json(r#"{"a":"vanilla a"}"#),
                json("# a pack comment\n{\"b\":\"pack b\", # trailing\n\"a\":\"pack a\"}"),
            ],
        );
        assert_eq!(
            parse(lenient.unwrap()),
            serde_json::json!({"a":"pack a","b":"pack b"})
        );
        // SoundManager: sounds add up; `replace` starts the event over.
        let sounds = merge(
            Merge::Sounds,
            &[
                json(r#"{"x":{"subtitle":"s.x","sounds":["v1"]},"y":{"sounds":["v2"]}}"#),
                json(
                    r#"{"x":{"sounds":["p1"]},"y":{"replace":true,"subtitle":"s.y","sounds":["p2"]}}"#,
                ),
                b"not json".to_vec(),
            ],
        );
        assert_eq!(
            parse(sounds.unwrap()),
            serde_json::json!({
                "x":{"subtitle":"s.x","sounds":["v1","p1"]},
                "y":{"subtitle":"s.y","sounds":["p2"]}
            })
        );
        // FontManager: the top pack's providers come first.
        let font = merge(
            Merge::Font,
            &[json(r#"{"providers":[1,2]}"#), json(r#"{"providers":[3]}"#)],
        );
        assert_eq!(
            parse(font.unwrap()),
            serde_json::json!({"providers":[3,1,2]})
        );
        // SpriteSourceList: sources from the bottom pack up, and the bottom
        // file keeps the fields a merge must not drop.
        let directory =
            |source: &str| serde_json::json!({"type": "directory", "source": source, "prefix": ""});
        let atlas = merge(
            Merge::Atlas,
            &[
                json(
                    r#"{"sources":[{"type":"directory","source":"a","prefix":""}],"unseen":true}"#,
                ),
                json(r#"{"sources":[{"type":"directory","source":"b","prefix":""}]}"#),
            ],
        );
        assert_eq!(
            parse(atlas.unwrap()),
            serde_json::json!({"sources":[directory("a"), directory("b")],"unseen":true})
        );
        // A definition that does not decode is skipped alone: the sources of the packs
        // below and above it still run.
        let atlas = merge(
            Merge::Atlas,
            &[
                json(r#"{"sources":[{"type":"directory","source":"low","prefix":""}]}"#),
                json(r#"{"sources":[{"type":"nobody:such_source"}]}"#),
                json(r#"{"sources":[{"type":"directory","source":"high","prefix":""}]}"#),
            ],
        );
        assert_eq!(
            parse(atlas.unwrap()),
            serde_json::json!({"sources":[directory("low"), directory("high")]})
        );
        // BlockStateModelLoader: every definition kept, bottom first, for the
        // catalog to bind one by one; one that does not parse is skipped.
        let gate = merge(
            Merge::BlockStates,
            &[
                json(r#"{"variants":{"":{"model":"v"}}}"#),
                b"not json".to_vec(),
                json(r#"{"variants":{"powered=true":{"model":"p"}}}"#),
            ],
        );
        assert_eq!(
            parse(gate.unwrap()),
            serde_json::json!({BLOCKSTATE_LAYERS:[
                {"variants":{"":{"model":"v"}}},
                {"variants":{"powered=true":{"model":"p"}}}
            ]})
        );
        // A single definition stays the file as it is.
        let alone = merge(
            Merge::BlockStates,
            &[json(r#"{"variants":{"":{"model":"p"}}}"#)],
        );
        assert_eq!(
            parse(alone.unwrap()),
            serde_json::json!({"variants":{"":{"model":"p"}}})
        );
        assert_eq!(
            merge_kind("assets/minecraft/blockstates/oak_fence_gate.json"),
            Some(Merge::BlockStates)
        );
        assert_eq!(
            merge_kind("assets/minecraft/lang/en_us.json"),
            Some(Merge::Lang)
        );
        assert_eq!(
            merge_kind("assets/minecraft/sounds.json"),
            Some(Merge::Sounds)
        );
        assert_eq!(
            merge_kind("assets/minecraft/font/default.json"),
            Some(Merge::Font)
        );
        assert_eq!(merge_kind("assets/minecraft/font/unifont.zip"), None);
        assert_eq!(
            merge_kind("assets/minecraft/atlases/blocks.json"),
            Some(Merge::Atlas)
        );
        assert_eq!(merge_kind("assets/minecraft/models/block/stone.json"), None);
    }
    #[test]
    fn metadata_free_mod_and_jar_layers_keep_pack_order() {
        use std::io::Write;
        let base = std::env::temp_dir().join(format!("sunrise-mod-layers-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let vanilla = base.join("vanilla");
        let path = "assets/shared/textures/block/test.png";
        fs::create_dir_all(vanilla.join(path).parent().unwrap()).unwrap();
        fs::write(vanilla.join(path), b"vanilla").unwrap();
        let jar = |name: &str, content: &[u8]| {
            let path = base.join(name);
            let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
            for (entry, bytes) in [
                ("assets/shared/textures/block/test.png", content),
                ("data/shared/recipe/test.json", b"{}".as_slice()),
                ("Example.class", b"dummy".as_slice()),
            ] {
                zip.start_file(entry, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(bytes).unwrap();
            }
            zip.finish().unwrap();
            path
        };
        let jar_a = jar("a.jar", b"jar-a");
        let jar_b = jar("b.jar", b"jar-b");
        let mod_a = base.join("mod-a");
        let mod_b = base.join("mod-b");
        for (root, bytes) in [(&mod_a, b"mod-a".as_slice()), (&mod_b, b"mod-b".as_slice())] {
            fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
            fs::write(root.join(path), bytes).unwrap();
        }
        for (root, bytes) in [
            (&vanilla, br#"{"removed":["base.old"]}"#.as_slice()),
            (&mod_b, br#"{"removed":["pack.old"]}"#.as_slice()),
        ] {
            let file = root.join(VANILLA_LANG_DEPRECATIONS);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, bytes).unwrap();
        }
        let layers = [
            Pack::mod_layer("jar-a".into(), Source::Zip(jar_a)),
            Pack::mod_layer("mod-a".into(), Source::Dir(mod_a)),
            Pack::mod_layer("jar-b".into(), Source::Zip(jar_b)),
            Pack::mod_layer("mod-b".into(), Source::Dir(mod_b)),
        ];
        let out = base.join("overlay");
        build_overlay(&vanilla, &layers[..3], &out).unwrap();
        assert_eq!(fs::read(out.join(path)).unwrap(), b"jar-b");
        build_overlay(&vanilla, &layers, &out).unwrap();
        assert_eq!(fs::read(out.join(path)).unwrap(), b"mod-b");
        assert_eq!(
            fs::read(out.join(VANILLA_LANG_DEPRECATIONS)).unwrap(),
            br#"{"removed":["base.old"]}"#
        );
        assert!(!out.join("data/shared/recipe/test.json").exists());
        assert!(!out.join("Example.class").exists());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn companion_maps_follow_the_winning_colour_or_a_higher_map_only_pack() {
        let root =
            std::env::temp_dir().join(format!("sunrise-material-owner-{}", std::process::id()));
        let vanilla = root.join("vanilla");
        let relative = "assets/minecraft/textures/block";
        fs::create_dir_all(vanilla.join(relative)).unwrap();
        let names = ["stone.png", "stone_n.png", "stone_s.png"];
        for name in names {
            fs::write(vanilla.join(relative).join(name), b"base").unwrap();
        }
        for (pack, files) in [
            ("low", vec!["stone_n.png", "stone_s.png"]),
            ("colour", vec!["stone.png"]),
            ("maps", vec!["stone_n.png"]),
        ] {
            let dir = root.join("packs").join(pack);
            fs::create_dir_all(dir.join(relative)).unwrap();
            fs::write(
                dir.join("pack.mcmeta"),
                br#"{"pack":{"description":"fixture","pack_format":97}}"#,
            )
            .unwrap();
            for name in files {
                fs::write(dir.join(relative).join(name), pack.as_bytes()).unwrap();
            }
        }
        let all = discover(&vanilla, &root.join("packs"));
        let order = |names: &[&str]| {
            names
                .iter()
                .map(|name| {
                    all.iter()
                        .find(|p| p.id == format!("file/{name}"))
                        .unwrap()
                        .clone()
                })
                .collect::<Vec<_>>()
        };
        let out = root.join("out");
        build_overlay(&vanilla, &order(&["low", "colour"]), &out).unwrap();
        assert_eq!(
            fs::read(out.join(relative).join("stone.png")).unwrap(),
            b"colour"
        );
        assert!(!out.join(relative).join("stone_n.png").exists());
        assert!(!out.join(relative).join("stone_s.png").exists());
        build_overlay(&vanilla, &order(&["low", "colour", "maps"]), &out).unwrap();
        assert_eq!(
            fs::read(out.join(relative).join("stone_n.png")).unwrap(),
            b"maps"
        );
        assert!(!out.join(relative).join("stone_s.png").exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn overlay_layers_packs_over_vanilla() {
        // `&` would split an unquoted `cmd /C mklink` line.
        let base = std::env::temp_dir().join(format!("sunrise-packs-&{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let vanilla = base.join("vanilla");
        fs::create_dir_all(vanilla.join("assets/minecraft/textures/block")).unwrap();
        fs::create_dir_all(vanilla.join("assets/minecraft/lang")).unwrap();
        fs::write(
            vanilla.join("assets/minecraft/textures/block/stone.png"),
            b"vanilla",
        )
        .unwrap();
        fs::write(
            vanilla.join("assets/minecraft/textures/block/dirt.png"),
            b"vanilla dirt",
        )
        .unwrap();
        fs::write(
            vanilla.join("assets/minecraft/lang/en_us.json"),
            br#"{"a":"vanilla","b":"vanilla"}"#,
        )
        .unwrap();
        fs::write(vanilla.join("facts.json"), b"[]").unwrap();
        // A directory no pack touches (#1688: a font json names sheets in it).
        fs::create_dir_all(vanilla.join("assets/minecraft/textures/font")).unwrap();
        fs::write(
            vanilla.join("assets/minecraft/textures/font/ascii.png"),
            b"font",
        )
        .unwrap();
        let pack_dir = base.join("packs/Mine");
        fs::create_dir_all(pack_dir.join("assets/minecraft/textures/block")).unwrap();
        fs::write(
            pack_dir.join("pack.mcmeta"),
            br#"{"pack":{"description":"mine","pack_format":97}}"#,
        )
        .unwrap();
        fs::write(
            pack_dir.join("assets/minecraft/textures/block/stone.png"),
            b"pack",
        )
        .unwrap();
        fs::create_dir_all(pack_dir.join("assets/minecraft/lang")).unwrap();
        fs::write(
            pack_dir.join("assets/minecraft/lang/en_us.json"),
            br#"{"b":"pack"}"#,
        )
        .unwrap();
        let packs = discover(&vanilla, &base.join("packs"));
        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0].id, "file/Mine");
        let out = base.join("overlay");
        build_overlay(&vanilla, &packs, &out).unwrap();
        assert_eq!(
            fs::read(out.join("assets/minecraft/textures/block/stone.png")).unwrap(),
            b"pack"
        );
        assert_eq!(
            fs::read(out.join("assets/minecraft/textures/block/dirt.png")).unwrap(),
            b"vanilla dirt"
        );
        // A pack's language file adds to vanilla's instead of replacing it.
        assert_eq!(
            fs::read(out.join("assets/minecraft/lang/en_us.json")).unwrap(),
            br#"{"a":"vanilla","b":"pack"}"#
        );
        assert_eq!(fs::read(out.join("facts.json")).unwrap(), b"[]");
        assert_eq!(
            fs::read(out.join("assets/minecraft/textures/font/ascii.png")).unwrap(),
            b"font"
        );
        // Linked, not mirrored: a junction on Windows, a symlink on unix.
        assert!(
            fs::symlink_metadata(out.join("assets/minecraft/textures/font"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let _ = fs::remove_dir_all(base);
    }
    #[test]
    fn every_file_of_a_big_stack_comes_from_its_top_pack() {
        use std::io::Write as _;
        let base = std::env::temp_dir().join(format!("sunrise-pack-stack-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let vanilla = base.join("vanilla");
        // Folders whose names start alike (`item`, `item_extra`, `item2`) and files beside
        // folders of almost their name: what a prefix lookup could mix up.
        let names: Vec<String> = (0..400)
            .map(|i| {
                let folder = ["item", "item_extra", "item2", "item/sub", "block"][i % 5];
                format!("assets/ns{}/textures/{folder}/f{i}.png", i % 3)
            })
            .chain(["assets/ns0/textures/item.png".to_owned()])
            .collect();
        let mut expected: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        for name in names.iter().step_by(4) {
            let path = vanilla.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, format!("vanilla {name}")).unwrap();
            expected.insert(name.clone(), format!("vanilla {name}").into_bytes());
        }
        let meta = br#"{"pack":{"description":"d","pack_format":97}}"#;
        // Packs a and b are zips, c a folder; each provides a different part of the names.
        fs::create_dir_all(base.join("packs/c")).unwrap();
        fs::write(base.join("packs/c/pack.mcmeta"), meta).unwrap();
        for (pack, every) in [("a", 2), ("b", 3)] {
            let file = fs::File::create(base.join(format!("packs/{pack}.zip"))).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            zip.start_file("pack.mcmeta", zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(meta).unwrap();
            for name in names.iter().step_by(every) {
                zip.start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(format!("{pack} {name}").as_bytes()).unwrap();
                expected.insert(name.clone(), format!("{pack} {name}").into_bytes());
            }
            zip.finish().unwrap();
        }
        for name in names.iter().skip(1).step_by(5) {
            let path = base.join("packs/c").join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, format!("c {name}")).unwrap();
            expected.insert(name.clone(), format!("c {name}").into_bytes());
        }
        let all = folder(&base.join("packs"), RESOURCE_FORMAT);
        let ids = ["file/a.zip", "file/b.zip", "file/c"].map(String::from);
        let out = base.join("overlay");
        build_overlay(&vanilla, &enabled(&all, &ids), &out).unwrap();
        fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
            for entry in fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, root, out);
                } else {
                    let name = path.strip_prefix(root).unwrap().to_string_lossy();
                    out.insert(name.replace('\\', "/"), fs::read(&path).unwrap());
                }
            }
        }
        let mut built = BTreeMap::new();
        walk(&out.join("assets"), &out, &mut built);
        assert_eq!(built, expected);
        let _ = fs::remove_dir_all(base);
    }
    #[test]
    #[ignore = "daily"]
    fn a_pack_that_unpacks_past_its_budget_fails_and_leaves_nothing() {
        use std::io::Write as _;
        let base = std::env::temp_dir().join(format!("sunrise-pack-bomb-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let vanilla = base.join("vanilla");
        fs::create_dir_all(vanilla.join("assets/minecraft/textures/block")).unwrap();
        fs::write(
            vanilla.join("assets/minecraft/textures/block/stone.png"),
            b"vanilla",
        )
        .unwrap();
        fs::create_dir_all(base.join("packs")).unwrap();
        let pack = |name: &str, files: &[(&str, usize)]| {
            let path = base.join("packs").join(name);
            let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
            let deflated = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file("pack.mcmeta", deflated).unwrap();
            zip.write_all(br#"{"pack":{"description":"d","pack_format":97}}"#)
                .unwrap();
            let zeros = vec![0u8; 1 << 20];
            for (file, mib) in files {
                zip.start_file(*file, deflated).unwrap();
                for _ in 0..*mib {
                    zip.write_all(&zeros).unwrap();
                }
            }
            zip.finish().unwrap();
            fs::metadata(&path).unwrap().len()
        };
        // Three 22 MiB files of zeros: well under the per-file limit, 66 MiB in all from a zip of
        // a few hundred KiB, past the 64 MiB a small pack may unpack to.
        let size = pack(
            "bomb.zip",
            &[
                ("assets/minecraft/textures/block/a.png", 22),
                ("assets/minecraft/textures/block/b.png", 22),
                ("assets/minecraft/textures/block/c.png", 22),
            ],
        );
        assert!(size < 1 << 20, "the bomb is small: {size} bytes");
        let packs = discover(&vanilla, &base.join("packs"));
        assert_eq!(packs.len(), 1);
        let out = base.join("overlay");
        let error = build_overlay(&vanilla, &packs, &out).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::FileTooLarge, "{error}");
        assert!(
            !out.exists(),
            "what was unpacked before the budget ran out stays"
        );
        // A file within the budget unpacks whole.
        fs::remove_file(base.join("packs/bomb.zip")).unwrap();
        pack("fine.zip", &[("assets/minecraft/textures/block/a.png", 8)]);
        let packs = discover(&vanilla, &base.join("packs"));
        build_overlay(&vanilla, &packs, &out).unwrap();
        assert_eq!(
            fs::metadata(out.join("assets/minecraft/textures/block/a.png"))
                .unwrap()
                .len(),
            8 << 20
        );
        let _ = fs::remove_dir_all(&base);
    }
    #[test]
    fn zip_entries_never_leave_the_overlay() {
        use std::io::Write as _;
        let base = std::env::temp_dir().join(format!("sunrise-pack-escape-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let vanilla = base.join("vanilla");
        fs::create_dir_all(vanilla.join("assets/minecraft/textures/block")).unwrap();
        fs::write(
            vanilla.join("assets/minecraft/textures/block/stone.png"),
            b"vanilla",
        )
        .unwrap();
        fs::create_dir_all(base.join("packs")).unwrap();
        let mut zip = zip::ZipWriter::new(fs::File::create(base.join("packs/evil.zip")).unwrap());
        for (name, data) in [
            (
                "pack.mcmeta",
                &br#"{"pack":{"description":"d","pack_format":97}}"#[..],
            ),
            ("assets/minecraft/textures/block/stone.png", b"pack"),
            ("assets/../../escaped.txt", b"out"),
            ("assets/minecraft/../../../escaped_lang.json", b"{}"),
            ("assets\\..\\..\\escaped_backslash.txt", b"out"),
            ("assets/./minecraft/dot.txt", b"in"),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
        let packs = discover(&vanilla, &base.join("packs"));
        assert_eq!(packs.len(), 1);
        let out = base.join("work/overlay");
        build_overlay(&vanilla, &packs, &out).unwrap();
        assert_eq!(
            fs::read(out.join("assets/minecraft/textures/block/stone.png")).unwrap(),
            b"pack"
        );
        for escaped in [
            "escaped.txt",
            "escaped_lang.json",
            "escaped_backslash.txt",
            "work/escaped.txt",
            "work/escaped_lang.json",
            "work/escaped_backslash.txt",
        ] {
            assert!(!base.join(escaped).exists(), "{escaped} was written");
        }
        let _ = fs::remove_dir_all(base);
    }
    #[test]
    fn built_in_and_folder_packs_give_their_pack_png() {
        use std::io::Write as _;
        let base = std::env::temp_dir().join(format!("sunrise-pack-icons-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let meta = br#"{"pack":{"description":"d","pack_format":97}}"#;
        let built_in = base.join("vanilla/assets/minecraft/resourcepacks");
        fs::create_dir_all(&built_in).unwrap();
        let mut zip =
            zip::ZipWriter::new(fs::File::create(built_in.join("programmer_art.zip")).unwrap());
        for (name, data) in [("pack.mcmeta", &meta[..]), ("pack.png", b"dirt")] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
        for (name, icon) in [("Iconless", None), ("Mine", Some(&b"mine"[..]))] {
            let dir = base.join("packs").join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("pack.mcmeta"), meta).unwrap();
            if let Some(icon) = icon {
                fs::write(dir.join("pack.png"), icon).unwrap();
            }
        }
        let packs = discover(&base.join("vanilla"), &base.join("packs"));
        let icons: Vec<_> = packs
            .iter()
            .map(|p| (p.id.as_str(), p.icon_png()))
            .collect();
        assert_eq!(
            icons,
            [
                ("programmer_art", Some(b"dirt".to_vec())),
                ("file/Iconless", None),
                ("file/Mine", Some(b"mine".to_vec())),
            ]
        );
        let _ = fs::remove_dir_all(base);
    }
    #[test]
    fn polytone_conditions_turn_overlays_on_from_the_packs_configs() {
        let dir =
            std::env::temp_dir().join(format!("sunrise-packs-polytone-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let configs = dir.join("assets/mypack/polytone/config_entries");
        fs::create_dir_all(&configs).unwrap();
        fs::write(configs.join("fog.json"), r#"{"default_value": "ON"}"#).unwrap();
        fs::write(
            configs.join("birds.json"),
            r#"{"default_value": 0, "max": 300}"#,
        )
        .unwrap();
        fs::write(
            dir.join("pack.mcmeta"),
            r#"{"pack":{"description":"p","pack_format":97},"overlays":{"entries":[
                {"directory":"opt_fog","formats":[0,0],"polytone_condition":"config('mypack:fog') == 'ON'"},
                {"directory":"opt_birds","formats":[0,0],"polytone_condition":"config('mypack:birds') > 0"},
                {"directory":"opt_plain","formats":[0,0]}]}}"#,
        )
        .unwrap();
        let pack = pack(
            "file/p".into(),
            Text::literal("p"),
            Source::Dir(dir.clone()),
        )
        .unwrap();
        assert_eq!(pack.overlays, ["opt_fog"]);
        let _ = fs::remove_dir_all(dir);
    }
    #[test]
    fn opened_resources_supply_base_polytone_configs() {
        use sunrise_assets::resource_source::{Resource, ResourceSource};
        let source = Source::Opened(ResourceSource::indexed(BTreeMap::from([
            (
                "assets/example/polytone/config_entries/fog.json".into(),
                Resource::bytes(br#"{"default_value":"ON"}"#.to_vec()),
            ),
            (
                "overlay/assets/example/polytone/config_entries/fog.json".into(),
                Resource::bytes(br#"{"default_value":"OFF"}"#.to_vec()),
            ),
        ])));
        let layer = Pack::mod_layer("example".into(), source);
        let meta = serde_json::json!({"overlays":{"entries":[{
            "directory":"fog", "formats":[0,0],
            "polytone_condition":"config('example:fog') == 'ON'"
        }]}});
        assert_eq!(overlays_of(&meta, &layer.polytone_configs), ["fog"]);
    }

    #[test]
    fn overlays_and_filters() {
        let v = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap();
        let meta = v(r#"{"overlays":{"entries":[
            {"formats":[90,96],"directory":"old"},
            {"formats":{"min_inclusive":97,"max_inclusive":97},"directory":"new"},
            {"min_format":97,"max_format":[97,5],"directory":"also"},
            {"formats":50,"directory":"no"}]}}"#);
        assert_eq!(overlays_of(&meta, &Default::default()), ["new", "also"]);
        // Only one plain folder of the pack, as vanilla's DIR_VALIDATOR has it.
        let meta = v(r#"{"overlays":{"entries":[
            {"formats":97,"directory":".."},
            {"formats":97,"directory":"../outside"},
            {"formats":97,"directory":"/etc"},
            {"formats":97,"directory":"a/b"},
            {"formats":97,"directory":"C:"},
            {"formats":97,"directory":"ok_1.21-x"}]}}"#);
        assert_eq!(overlays_of(&meta, &Default::default()), ["ok_1.21-x"]);
        let base = std::env::temp_dir().join(format!("sunrise-packs-ov-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let vanilla = base.join("vanilla");
        fs::create_dir_all(vanilla.join("assets/minecraft/textures/block")).unwrap();
        fs::write(
            vanilla.join("assets/minecraft/textures/block/stone.png"),
            b"v",
        )
        .unwrap();
        fs::write(
            vanilla.join("assets/minecraft/textures/block/dirt.png"),
            b"v",
        )
        .unwrap();
        let dir = base.join("packs/P");
        fs::create_dir_all(dir.join("new/assets/minecraft/textures/block")).unwrap();
        fs::create_dir_all(dir.join("old/assets/minecraft/textures/block")).unwrap();
        fs::write(
            dir.join("pack.mcmeta"),
            br#"{"pack":{"description":"p","pack_format":97},
            "overlays":{"entries":[{"formats":97,"directory":"new"},{"formats":1,"directory":"old"}]},
            "filter":{"block":[{"path":"textures/block/dirt\\.png"}]}}"#,
        )
        .unwrap();
        fs::write(
            dir.join("new/assets/minecraft/textures/block/stone.png"),
            b"overlay",
        )
        .unwrap();
        fs::write(
            dir.join("old/assets/minecraft/textures/block/stone.png"),
            b"old",
        )
        .unwrap();
        let packs = discover(&vanilla, &base.join("packs"));
        let out = base.join("overlay");
        build_overlay(&vanilla, &packs, &out).unwrap();
        assert_eq!(
            fs::read(out.join("assets/minecraft/textures/block/stone.png")).unwrap(),
            b"overlay"
        );
        assert!(
            !out.join("assets/minecraft/textures/block/dirt.png")
                .exists()
        );
        let _ = fs::remove_dir_all(base);
    }

    /// A pack's filter that hides a vanilla GUI sprite leaves a row's check box drawing the
    /// missing texture, as vanilla does, while the sprites it left alone still draw.
    #[test]
    fn a_gui_sprite_a_pack_filters_out_draws_the_missing_texture() {
        use crate::graphics_style::{checkbox, drew_rect, drew_sprite, write_widget_resources};
        use crate::resource_source::ResourceSource;
        let base = tempfile::tempdir().unwrap();
        write_widget_resources(base.path());
        let pack_dir = tempfile::tempdir().unwrap();
        let mut pack = Pack::mod_layer("filter".into(), Source::Dir(pack_dir.path().to_owned()));
        pack.filter = filter_of(&serde_json::json!({"filter":{"block":[
            {"namespace":"minecraft","path":"textures/gui/sprites/widget/checkbox\\.png"}]}}));
        let source = resources(ResourceSource::directory(base.path().to_owned()), &[pack]).unwrap();
        let atlas = crate::hud::HudAtlas::build(&source).unwrap();
        crate::hud::take_gui_sprite_misses();
        let mut off = crate::hud::HudBatch::scratch([640, 480]);
        checkbox(&mut off, &atlas, 100.0, 0.0, false, (false, true));
        assert_eq!(crate::hud::take_gui_sprite_misses(), ["widget/checkbox"]);
        let missing = &atlas.sprites[crate::hud::MISSING_SPRITE];
        assert!(
            drew_rect(&off, missing, None),
            "the missing texture in its place"
        );
        let mut on = crate::hud::HudBatch::scratch([640, 480]);
        checkbox(&mut on, &atlas, 100.0, 0.0, true, (false, true));
        assert!(drew_sprite(&on, &atlas, "widget/checkbox_selected"));
    }
}
