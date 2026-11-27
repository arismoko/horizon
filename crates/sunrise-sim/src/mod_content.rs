//! Content mods in the resource root (docs/modpacks.md, registries 0.2).
//!
//! Every loader reads one resource root, and most of its block and item
//! tables are exports from vanilla indexed by block-state id or keyed by
//! name. A content mod's block copies a vanilla *template*. So the mods'
//! root is an overlay, like the resource-pack one:
//! - the base root linked, with the per-state and per-name tables
//!   rewritten: each mod block gets its template's rows appended (states
//!   after vanilla's, mods and blocks in id order) with names, translation
//!   keys and in-table state ids moved over, and the declared overrides
//!   applied;
//! - each mod's own `assets/` and `data/` files;
//! - additions to tags merged into the tag files and bundles.
//!
//! The loaders need no mod code, vanilla state ids never move, and with no
//! content mod the root is the base root itself.
use crate::{
    Result,
    blocks::MAP_COLORS,
    mobs::data::{
        PartAnswers, PathComputationType, STURDY_TOP, VALID_EMPTY_SPAWN, VALID_SPAWN,
        VALID_SPAWN_ICE, VALID_SPAWN_LEAVES,
    },
};
use mod_api::{
    __private::{DeclarationKind, DeclarationRefusal},
    DeclarationError,
};
use serde::{Deserialize, Serialize, de};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    ops::Range,
    path::{Path, PathBuf},
};
use sunrise_core::geometry::{Aabb, Axis, ShapeOp, Vec3, VoxelShape};
use sunrise_core::{
    nbt,
    nbt::{Compound, JavaString, Limits, Tag},
};
use sunrise_mod_package::content::{
    Block, BlockEntityType, BlockShapes, Content, ContextCollision, CreativeTab, EntityType,
    FluidLook, Item, LocalDeclaration, MenuType, MobEffect, Rarity, SpawnRule, StateProperty,
    StateValues, check_face_rule_properties,
};
use sunrise_mod_package::manifest::{
    JarInputsError, JarRef, collect_jar_requirements, missing_jars,
};
use sunrise_world::{
    block_collision::ScaffoldingCollision,
    block_properties::BlockDefinitionProperties,
    fluid::{FluidKind, FluidState},
};

/// A content mod to put in the root: its id, its content, and the Java jars
/// it lists in manifest order — each an asset/data layer under the mod's own
/// files, its file found by hash in `jar_store` (modding book's manifest reference).
pub struct ContentMod {
    pub id: String,
    pub content: Content,
    pub jars: Vec<JarRef>,
}

/// Refuse a package whose crafting remainder does not resolve to an admitted item.
/// A refusal can invalidate a reference in another package, so repeat until no package is dropped.
pub fn drop_dangling_remainders(
    base: &Path,
    mods: &mut Vec<ContentMod>,
) -> Result<Vec<(String, String)>> {
    if !mods
        .iter()
        .flat_map(|m| &m.content.items)
        .any(|item| item.crafting_remainder.is_some())
    {
        return Ok(Vec::new());
    }
    let vanilla = builtin_item_keys(base)?;
    let mut dropped = Vec::new();
    loop {
        let admitted: BTreeSet<_> = mods
            .iter()
            .flat_map(declared_item_ids)
            .map(str::to_owned)
            .collect();
        let before = mods.len();
        mods.retain(|layer| {
            let invalid = layer.content.items.iter().find_map(|item| {
                let rest = item.crafting_remainder.as_deref()?;
                (rest == "minecraft:air" || !(vanilla.contains(rest) || admitted.contains(rest)))
                    .then(|| format!("{}: crafting_remainder {rest:?} is not an item", item.id))
            });
            if let Some(error) = invalid {
                dropped.push((layer.id.clone(), error));
                false
            } else {
                true
            }
        });
        if mods.len() == before {
            return Ok(dropped);
        }
    }
}

fn builtin_item_keys(base: &Path) -> Result<BTreeSet<String>> {
    match read_json(&base.join("items.json"))? {
        J::Obj(entries) => Ok(entries.into_iter().map(|(id, _)| id).collect()),
        _ => Err("items.json is not an object".into()),
    }
}

fn declared_item_ids(layer: &ContentMod) -> impl Iterator<Item = &str> {
    layer
        .content
        .items
        .iter()
        .map(|item| item.id.as_str())
        .chain(
            layer
                .content
                .blocks
                .iter()
                .filter(|block| block.item)
                .map(|block| block.id.as_str()),
        )
}

include!("mod_content/fluids.rs");

#[path = "mod_content_fluid_declarations.rs"]
mod fluid_declarations;
pub use fluid_declarations::{fluid_interactions, set_fluid_interactions};

/// The temperature, density and viscosity a mod's fluid file states for `id`, copied without
/// the rest of its look.
pub fn fluid_measures(id: &str) -> Option<sunrise_mod_package::content::FluidMeasures> {
    FLUID_LOOKS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
        .map(|look| look.measures)
}

/// The item a bucket of the mod fluid `id` is (`Fluid.getBucket`), a tank-only one's too.
pub fn fluid_bucket(id: &str) -> Option<String> {
    FLUID_LOOKS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)?
        .bucket_id()
}

/// How the fluid `id` looks, when a mod declares it.
pub fn fluid_look(id: &str) -> Option<FluidLook> {
    FLUID_LOOKS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
        .cloned()
}

include!("mod_content/content.rs");

/// Content entity rows preserve template numbers and carry sound selections.
mod entity_sounds;
mod light_rows;
use entity_sounds::mod_entity_row;

include!("mod_content/plan.rs");

/// The JSON and NBT tables rewritten for new blocks and items.
const JSON_TABLES: [&str; 16] = [
    "facts.json",
    "entity-types.json",
    "creative-inventory.json",
    "registries.json",
    "gameplay-facts.json",
    "block-behaviours.json",
    "model-states.json",
    "render-definitions.json",
    "landing-definitions.json",
    "items.json",
    "command-registries.json",
    "mob-data.json",
    "worldgen/block_flags.json",
    "worldgen/blocks/structure-blocks.json",
    "worldgen/blocks/survival-rules.json",
    "worldgen/blocks/survival-extra-definitions.json",
];
const NBT_TABLES: [&str; 6] = [
    "occlusion.nbt",
    "cached-collision.nbt",
    "movement-properties.nbt",
    "worldgen/blocks/block-properties.nbt",
    "worldgen/blocks/cached-support.nbt",
    "worldgen/blocks/cached-collision.nbt",
];
use sunrise_items::content_namespaces::MARKER;
pub use sunrise_items::content_namespaces::{namespaces, tag_namespaces};
/// Block states a world can hold. Chunk sections widen their global palette
/// past vanilla's 16 bits to fit the registry (crate::chunk::set_block_states);
/// this caps it at 20 bits, about 29 times vanilla's 35,723 states. Mods past
/// it are refused, not wrapped onto other blocks.
pub const MAX_STATES: u32 = 1 << 20;

/// Bundles of block tag files (tag id → the tag file's text).
const TAG_BUNDLES: [&str; 2] = ["block-tags.json", "worldgen/block_tags.json"];
/// The worldgen loader's bundle of biome tags.
const BIOME_TAGS: &str = "worldgen/biome_tags.json";
/// Worldgen registries a content mod may add to, and the bundle the
/// worldgen loader reads each from (id → the file's text): what structure
/// mods hold, and a mod's own biomes (any namespace but `minecraft`, which
/// no mod can replace). Noise and density functions change the terrain and
/// are not read from mods.
const WORLDGEN_BUNDLES: [(&str, &str); 7] = [
    ("biome", "worldgen/biomes.json"),
    ("structure", "worldgen/structures.json"),
    ("structure_set", "worldgen/structure_sets.json"),
    ("template_pool", "worldgen/template_pools.json"),
    ("processor_list", "worldgen/processor_lists.json"),
    ("feature", "worldgen/features.json"),
    ("placed_feature", "worldgen/placed_features.json"),
];

/// The registry of `command-registries.json` that each bundle's ids join,
/// for the commands that suggest them (`/locate`, `/place`).
const COMMAND_REGISTRIES: [(&str, &str); 7] = [
    ("worldgen/biomes.json", "minecraft:worldgen/biome"),
    ("worldgen/structures.json", "minecraft:worldgen/structure"),
    (
        "worldgen/structure_sets.json",
        "minecraft:worldgen/structure_set",
    ),
    (
        "worldgen/template_pools.json",
        "minecraft:worldgen/template_pool",
    ),
    (
        "worldgen/processor_lists.json",
        "minecraft:worldgen/processor_list",
    ),
    ("worldgen/features.json", "minecraft:worldgen/feature"),
    (
        "worldgen/placed_features.json",
        "minecraft:worldgen/placed_feature",
    ),
];

/// A mod file that adds to a worldgen bundle: (bundle, id).
fn worldgen_entry(file: &str) -> Option<(&'static str, String)> {
    let (ns, rest) = file.strip_prefix("data/")?.split_once('/')?;
    let (kind, path) = rest.strip_prefix("worldgen/")?.split_once('/')?;
    let (_, bundle) = WORLDGEN_BUNDLES.iter().find(|(k, _)| *k == kind)?;
    if *bundle == BIOMES && ns == "minecraft" {
        return None;
    }
    Some((bundle, format!("{ns}:{}", path.strip_suffix(".json")?)))
}

/// The key of a mod's structure set, template pool or processor list file
/// that adds its entries to other sets, pools or lists (vanilla's, another
/// mod's) instead of being one of its own. Vanilla data never has it.
const ADD_TO: &str = "sunrise:add_to";
/// An addition's place among the others (lower first; 1000 by default), as
/// Lithostitched's modifiers' `priority`.
const PRIORITY: &str = "sunrise:priority";
/// The bundles a mod may add to, and the list each entry goes into.
const ADDABLE: [(&str, &str); 3] = [
    ("worldgen/structure_sets.json", "structures"),
    ("worldgen/template_pools.json", "elements"),
    ("worldgen/processor_lists.json", "processors"),
];

/// A mod file's additions (`sunrise:add_to`): its entries appended to the
/// list of each set, pool or processor list it names, as Lithostitched's
/// `add_structure_set_entries`, `add_template_pool_elements` and
/// `add_processor_list_processors` do for NeoForge and Fabric packs.
struct Addition {
    targets: Vec<String>,
    list: &'static str,
    entries: Vec<J>,
    priority: i64,
}
impl Addition {
    /// What a mod file of `bundle` adds, if it is an addition file.
    fn read(bundle: &str, text: &str) -> Result<Option<Self>> {
        let Some((_, list)) = ADDABLE.iter().find(|(b, _)| *b == bundle) else {
            return Ok(None);
        };
        if !text.contains(ADD_TO) {
            return Ok(None);
        }
        let doc: J = serde_json::from_str(text)?;
        let Some(targets) = doc.get(ADD_TO) else {
            return Ok(None);
        };
        let targets = match targets {
            J::Str(s) => vec![s.clone()],
            J::Arr(a) => a
                .iter()
                .map(|v| match v {
                    J::Str(s) => Some(s.clone()),
                    _ => None,
                })
                .collect::<Option<_>>()
                .ok_or(format!("{ADD_TO}: not a list of ids"))?,
            _ => return Err(format!("{ADD_TO}: not an id or a list of ids").into()),
        };
        let Some(J::Arr(entries)) = doc.get(list) else {
            return Err(format!("{ADD_TO} without a {list} list").into());
        };
        let priority = match doc.get(PRIORITY) {
            None => 1000,
            Some(J::Num(n)) => n.as_i64().ok_or(format!("{PRIORITY}: not an integer"))?,
            Some(_) => return Err(format!("{PRIORITY}: not an integer").into()),
        };
        Ok(Some(Self {
            targets,
            list,
            entries: entries.clone(),
            priority,
        }))
    }
    /// Appends the entries to each target in the bundle `doc` (id → the
    /// file's text). A target nobody defines is skipped, as a tag entry
    /// naming an unknown id is.
    fn apply(&self, doc: &mut J) -> Result<()> {
        for target in &self.targets {
            let Some(J::Str(text)) = doc.get(target) else {
                continue;
            };
            let mut v: J = serde_json::from_str(text).map_err(|e| format!("{target}: {e}"))?;
            let Some(J::Arr(list)) = v.get_mut(self.list) else {
                continue;
            };
            list.extend(self.entries.iter().cloned());
            doc.set(target, J::Str(serde_json::to_string(&v)?));
        }
        Ok(())
    }
}

/// Applies `additions` to the bundle `doc` by priority (lower first); equal priorities keep the
/// order given, which is mod order, then file order.
fn apply_additions(doc: &mut J, additions: &mut [Addition]) -> Result<()> {
    additions.sort_by_key(|a| a.priority);
    additions.iter().try_for_each(|a| a.apply(doc))
}

/// The worldgen loader's bundles of biomes and of placed feature tags.
const BIOMES: &str = "worldgen/biomes.json";
const FEATURE_TAGS: &str = "worldgen/feature_tags.json";
const PLACED_FEATURES: &str = "worldgen/placed_features.json";
const CARVERS: &str = "worldgen/carvers.json";

/// Rewrites the biome bundle with the mods' biome modifiers. `biome_tags`
/// `placed` and `biome_doc` are the bundles as the mods left them (`None`: the base's).
fn place_in_biomes(
    base: &Path,
    overlay: &Overlay<'_>,
    modifiers: &[biomes::Modifier],
    modifier_sources: Vec<[String; 3]>,
    enabled_mods: Vec<String>,
    biome_tags: Option<J>,
    placed: Option<J>,
    biome_doc: Option<J>,
    tags: &BTreeMap<String, TagMerge>,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let bundle = |doc: Option<J>, name: &str| -> Result<BTreeMap<String, String>> {
        let doc = match doc {
            Some(doc) => doc,
            None if base.join(name).exists() => read_json(&base.join(name))?,
            None => return Ok(BTreeMap::new()),
        };
        let J::Obj(entries) = doc else {
            return Err(format!("{name}: not an object").into());
        };
        Ok(entries
            .into_iter()
            .filter_map(|(k, v)| match v {
                J::Str(text) => Some((k, text)),
                _ => None,
            })
            .collect())
    };
    if !base.join(BIOMES).exists() {
        return Err(format!("biome modifiers need {BIOMES}").into());
    }
    let mut doc = match biome_doc {
        Some(doc) => doc,
        None => read_json(&base.join(BIOMES))?,
    };
    let biome_tags = seed_convention_tags("worldgen/biome", bundle(biome_tags, BIOME_TAGS)?)?;
    let placed: BTreeSet<String> = bundle(placed, PLACED_FEATURES)?.into_keys().collect();
    let feature_tags = bundle(None, FEATURE_TAGS)?;
    let (entity_categories, entity_tags) = modifier_entities(base, overlay, tags)?;
    let carver_ids = biomes::carver_ids(&base.join(CARVERS))?;
    let mut saved = SavedModifiers {
        carver_ids,
        carver_tags: modifier_tags(base, overlay, tags, "worldgen/carver")?,
        enabled_mods,
        modifiers: modifier_sources,
        unmodified: BTreeMap::new(),
        feature_tags,
        entity_categories,
        entity_tags,
    };
    let before = doc.clone();
    for (id, text) in saved.apply(&mut doc, modifiers, &biome_tags, &placed, warnings)? {
        if let Some(J::Str(old)) = before.get(&id) {
            saved.unmodified.insert(id.clone(), old.clone());
        }
        if let Some((ns, path)) = id.split_once(':') {
            let file = format!("data/{ns}/worldgen/biome/{path}.json");
            overlay.write(&file, text.as_bytes())?;
        }
    }
    biomes::write_state(overlay, &saved, &doc, modifiers)
}

fn modifier_entities(
    base: &Path,
    overlay: &Overlay<'_>,
    tags: &BTreeMap<String, TagMerge>,
) -> Result<(BTreeMap<String, String>, BTreeMap<String, String>)> {
    let entities_path = out_path(overlay, "entity-types.json");
    let entity_categories: BTreeMap<String, String> = match entities_path.exists() {
        true => match read_json(&entities_path)? {
            J::Obj(entries) => entries
                .into_iter()
                .map(|(k, v)| {
                    let cat = v
                        .get("category")
                        .and_then(J::as_str)
                        .unwrap_or("misc")
                        .to_owned();
                    (k, cat)
                })
                .collect(),
            _ => return Err("entity-types.json: not an object".into()),
        },
        false => BTreeMap::new(),
    };
    let entity_tags = modifier_tags(base, overlay, tags, "entity_type")?;
    Ok((entity_categories, entity_tags))
}

fn modifier_tags(
    base: &Path,
    overlay: &Overlay<'_>,
    tags: &BTreeMap<String, TagMerge>,
    registry: &str,
) -> Result<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();
    for ns in fs::read_dir(base.join("data"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let dir = ns.path().join("tags").join(registry);
        if !dir.is_dir() {
            continue;
        }
        for file in &crate::json_folder::read(&dir)?.files {
            result.insert(
                format!("{}:{}", ns.file_name().to_string_lossy(), file.name),
                std::str::from_utf8(file.bytes()?)?.to_owned(),
            );
        }
    }
    let prefix = format!("/tags/{registry}/");
    for file in tags.keys() {
        if let Some((ns, path)) = file
            .strip_prefix("data/")
            .and_then(|s| s.split_once(&prefix))
            .and_then(|(ns, p)| Some((ns, p.strip_suffix(".json")?)))
        {
            result.insert(
                format!("{ns}:{path}"),
                fs::read_to_string(out_path(overlay, file))?,
            );
        }
    }
    seed_convention_tags(registry, result)
}

/// Layers `over` (vanilla/mod tag JSONs) on top of the built-in `c:` / `neoforge:`
/// convention tags for `registry`, matching the world's tag layering order.
fn seed_convention_tags(
    registry: &str,
    over: BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>> {
    let mut result: BTreeMap<String, String> =
        sunrise_world::tag_layers::convention::json_entries(registry)
            .iter()
            .cloned()
            .collect();
    let no_conditions = sunrise_world::conditions::LoadConditions::default();
    for (id, json) in over {
        let Some(base_json) = result.get(&id).map(String::as_str) else {
            result.insert(id, json);
            continue;
        };
        let tag: J = serde_json::from_str(&json).map_err(|e| format!("tag {id}: {e}"))?;
        if matches!(tag.get("replace"), Some(J::Bool(true))) {
            result.insert(id, json);
            continue;
        }
        let layers = [load_conditions::TagLayers {
            values: tag_list(&tag, "values"),
            remove: tag_list(&tag, "remove"),
        }];
        let merged = merge_tag(Some(base_json), &layers, false, &id, &no_conditions)?;
        result.insert(id, merged);
    }
    Ok(result)
}

#[path = "mod_content_structures.rs"]
mod structures;

/// In a content root with biome modifiers: what applying them again over a
/// world's data packs takes (`biome_modifiers_over_packs`).
const MODIFIERS: &str = "biome-modifiers.json";

#[derive(Serialize, Deserialize)]
struct SavedModifiers {
    enabled_mods: Vec<String>,
    /// Each modifier's mod, file and text, in the order they apply.
    modifiers: Vec<[String; 3]>,
    /// The biomes the modifiers changed, as they were before.
    unmodified: BTreeMap<String, String>,
    feature_tags: BTreeMap<String, String>,
    entity_categories: BTreeMap<String, String>,
    entity_tags: BTreeMap<String, String>,
    #[serde(default)]
    carver_ids: BTreeSet<String>,
    #[serde(default)]
    carver_tags: BTreeMap<String, String>,
}

impl SavedModifiers {
    fn apply(
        &self,
        doc: &mut J,
        modifiers: &[biomes::Modifier],
        biome_tags: &BTreeMap<String, String>,
        placed: &BTreeSet<String>,
        warnings: &mut Vec<String>,
    ) -> Result<Vec<(String, String)>> {
        let biome_ids: BTreeSet<String> = match &*doc {
            J::Obj(o) => o.iter().map(|(k, _)| k.clone()).collect(),
            _ => return Err(format!("{BIOMES}: not an object").into()),
        };
        let entity_ids: BTreeSet<String> = self.entity_categories.keys().cloned().collect();
        let registries = biomes::Registries::of(
            &biome_ids,
            biome_tags,
            placed,
            &self.feature_tags,
            (&entity_ids, &self.entity_tags),
            (&self.carver_ids, &self.carver_tags),
        );
        biomes::apply(
            doc,
            modifiers,
            &registries,
            &self.entity_categories,
            warnings,
        )
    }
}

/// A world's data packs (`packs`) with the content root `root`'s biome
/// modifiers applied over them. Modifiers apply once every data
/// pack is in, to the final biomes and tags: a pack's file for a biome
/// (Terralith's `minecraft:plains`) keeps the mods' features and spawns, and
/// a pack's biome its tags put under a modifier's selector gets them too.
/// `None` when the root's biomes stand: no modifiers, or nothing declared and packs that bring
/// no biome and no tag. `declared` are mods' code modifiers (mod, file, text), applied last.
pub fn biome_modifiers_over_packs(
    root: &Path,
    packs: &sunrise_world::worldgen::overrides::WorldgenOverrides,
    declared: &[[String; 3]],
) -> Result<Option<sunrise_world::worldgen::overrides::WorldgenOverrides>> {
    let Some(bytes) = biomes::saved_bytes(root, packs, declared) else {
        return Ok(None);
    };
    let mut saved: SavedModifiers =
        serde_json::from_slice(&bytes).map_err(|e| format!("{MODIFIERS}: {e}"))?;
    let read = |name: &str| -> Result<BTreeMap<String, String>> {
        match fs::read(root.join(name)) {
            Ok(b) => Ok(serde_json::from_slice(&b).map_err(|e| format!("{name}: {e}"))?),
            Err(_) => Ok(BTreeMap::new()),
        }
    };
    let builtin_sounds = biomes::load_sound_events(root);
    let registered_sound = |name: &str| {
        builtin_sounds
            .as_ref()
            .is_some_and(|b| biomes::registered_sound(b, name))
    };
    let mut warnings = Vec::new();
    let mut modifiers = Vec::new();
    let sources = || saved.modifiers.iter().chain(declared);
    biomes::check_registry(sources().map(|[owner, file, _]| (owner.as_str(), file.as_str())))?;
    for [m, file, text] in sources() {
        modifiers.extend(biomes::Modifier::read(
            m,
            file,
            text,
            &registered_sound,
            &mut warnings,
        )?);
    }
    // What the generator reads without the modifiers over the packs.
    let packed = packs.apply("biomes", read(BIOMES)?);
    let mut start = packed.clone();
    for (id, text) in &saved.unmodified {
        if packs.entries("biomes").all(|(p, _)| p != id)
            && let Some(value) = start.get_mut(id)
        {
            // Restore the pre-modifier value, not a holder refused by the world load.
            value.clone_from(text);
        }
    }
    let mut doc = J::Obj(start.into_iter().map(|(id, t)| (id, J::Str(t))).collect());
    // Pack tag files after the root's, over the built-in convention tags, as the worldgen loader merges them.
    let mut biome_tags = seed_convention_tags("worldgen/biome", read(BIOME_TAGS)?)?;
    for (id, pack, json) in packs.tags("biome_tags") {
        let tag: J = match serde_json::from_str(json) {
            Ok(tag) => tag,
            Err(e) => {
                warnings.push(format!("{pack}: tag {id}: {e}"));
                continue;
            }
        };
        let (values, remove) = (tag_list(&tag, "values"), tag_list(&tag, "remove"));
        let replaced = matches!(tag.get("replace"), Some(J::Bool(true)));
        let old = if replaced {
            None
        } else {
            biome_tags.get(id).map(String::as_str)
        };
        let layers = [load_conditions::TagLayers { values, remove }];
        let merged = merge_tag(
            old,
            &layers,
            replaced,
            id,
            &sunrise_world::conditions::LoadConditions::new(
                saved.enabled_mods.iter().map(String::as_str),
            ),
        )?;
        biome_tags.insert(id.to_owned(), merged);
    }
    let placed = packs
        .apply("placed_features", read(PLACED_FEATURES)?)
        .into_keys()
        .collect();
    carver_registry::refresh(&mut saved, packs)?;
    saved.apply(&mut doc, &modifiers, &biome_tags, &placed, &mut warnings)?;
    for warning in warnings {
        log::warn!("Sunrise: biome modifiers over data packs: {warning}");
    }
    let mut out = packs.clone();
    if let J::Obj(entries) = doc {
        for (id, text) in entries {
            if let J::Str(text) = text
                && packed.get(&id) != Some(&text)
            {
                out.set("biomes", &id, text);
            }
        }
    }
    Ok(Some(out))
}

#[path = "mod_content_carvers.rs"]
mod carver_registry;

#[path = "mod_content_biomes.rs"]
pub mod biomes;

/// The overlay being written: `out` mirrors `base` through links, and a
/// directory becomes real only when something inside it is written.
struct Overlay<'a> {
    written: std::sync::Mutex<std::collections::BTreeSet<String>>,
    base: &'a Path,
    out: &'a Path,
}
impl Overlay<'_> {
    fn new<'a>(base: &'a Path, out: &'a Path) -> Result<Overlay<'a>> {
        if fs::symlink_metadata(out).is_ok() {
            fs::remove_dir_all(out)?;
        }
        fs::create_dir_all(out)?;
        let overlay = Overlay {
            base,
            out,
            written: Default::default(),
        };
        overlay.link_children(Path::new(""))?;
        Ok(overlay)
    }
    fn link_children(&self, rel: &Path) -> Result<()> {
        let Ok(read) = fs::read_dir(self.base.join(rel)) else {
            return Ok(());
        };
        for entry in read.flatten() {
            let rel = rel.join(entry.file_name());
            // The base's files in one file: it describes the base, not this root (and
            // would be copied where links are not made).
            if rel == Path::new(sunrise_core::prepared_files::FILE_NAME) {
                continue;
            }
            link(&self.base.join(&rel), &self.out.join(&rel))?;
        }
        Ok(())
    }
    /// Makes `rel`'s directories real, each holding links to the base's.
    fn dir(&self, rel: &Path) -> Result<()> {
        let mut at = PathBuf::new();
        for part in rel.iter() {
            at.push(part);
            let path = self.out.join(&at);
            let meta = fs::symlink_metadata(&path);
            if meta
                .as_ref()
                .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
            {
                continue;
            }
            if meta.is_ok() {
                fs::remove_file(&path)?;
            }
            fs::create_dir(&path)?;
            self.link_children(&at)?;
        }
        Ok(())
    }
    fn write(&self, rel: &str, bytes: &[u8]) -> Result<()> {
        let name = rel;
        let rel = Path::new(rel);
        if let Some(parent) = rel.parent() {
            self.dir(parent)?;
        }
        let path = self.out.join(rel);
        if fs::symlink_metadata(&path).is_ok() {
            fs::remove_file(&path)?;
        }
        fs::write(path, bytes)?;
        self.written
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(name.to_owned());
        Ok(())
    }
}

fn out_path(overlay: &Overlay<'_>, rel: &str) -> PathBuf {
    overlay.out.join(rel)
}
#[cfg(unix)]
fn link(target: &Path, at: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, at)
}
#[cfg(not(unix))]
fn link(target: &Path, at: &Path) -> std::io::Result<()> {
    if target.is_dir() {
        fs::create_dir_all(at)?;
        for entry in fs::read_dir(target)?.flatten() {
            link(&entry.path(), &at.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        link_file(target, at, |target, at| fs::hard_link(target, at))
    }
}
/// A file of the overlay where symlinks need a privilege: a hard link to the base's file, which costs a name where
/// a copy costs the file's bytes (the base is ~28,000 files, 0.6 GiB), or a copy when no link can be made (another
/// volume, a filesystem without links, the NTFS limit on links to one file). A link shares the base file's data,
/// which is safe because the overlay is only ever replaced name by name (`Overlay::write` removes the name before it
/// writes), never written in place. A symlink is copied through, as before: linked, it would stay a symlink.
#[cfg(any(not(unix), test))]
fn link_file(
    target: &Path,
    at: &Path,
    hard_link: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let symlink = fs::symlink_metadata(target).is_ok_and(|m| m.file_type().is_symlink());
    if !symlink && hard_link(target, at).is_ok() {
        return Ok(());
    }
    fs::copy(target, at).map(|_| ())
}

mod root_lifetime;
use root_lifetime::prune_roots;
pub use root_lifetime::retain_root;
mod load_conditions;
use load_conditions::{TagLayers, TagMerge, merge_tag, tag_list};
pub use load_conditions::{build_with_conditions, resource_root_with_conditions};

/// The block and item ids and tags a content root has, for tag entries.
struct Known {
    blocks: std::collections::BTreeSet<String>,
    items: std::collections::BTreeSet<String>,
    /// The worldgen loader's biomes and biome tags.
    biomes: std::collections::BTreeSet<String>,
    biome_tags: std::collections::BTreeSet<String>,
    /// `data/<ns>/tags/<kind>/<path>.json` files the mods add.
    tag_files: std::collections::BTreeSet<String>,
    /// Block and item tag paths, by kind. A missing kind means the base has no folder to consult.
    vanilla_tags: BTreeMap<&'static str, BTreeSet<String>>,
}
impl Known {
    fn new(
        base: &Path,
        plan: &Plan,
        tags: &BTreeMap<String, TagMerge>,
        mod_biomes: impl Iterator<Item = String>,
    ) -> Result<Self> {
        use std::io::ErrorKind::{NotADirectory, NotFound};

        let mut blocks: std::collections::BTreeSet<String> =
            plan.defaults.keys().cloned().collect();
        blocks.extend(plan.blocks.iter().map(|b| b.decl.id.clone()));
        let mut items = std::collections::BTreeSet::new();
        if let J::Obj(o) = read_json(&base.join("items.json"))? {
            items.extend(o.into_iter().map(|(k, _)| k));
        }
        items.extend(plan.items.iter().map(|i| i.rename.to.clone()));
        let keys = |name: &str| -> Result<std::collections::BTreeSet<String>> {
            Ok(match base.join(name).exists() {
                true => match read_json(&base.join(name))? {
                    J::Obj(o) => o.into_iter().map(|(k, _)| k).collect(),
                    _ => Default::default(),
                },
                false => Default::default(),
            })
        };
        let mut biomes = keys(BIOMES)?;
        biomes.extend(mod_biomes);
        let mut vanilla_tags = BTreeMap::new();
        for kind in ["block", "item"] {
            let dir = base.join(format!("data/minecraft/tags/{kind}"));
            let names = if let Some(files) =
                sunrise_core::prepared_files::files_under(&dir).filter(|files| !files.is_empty())
            {
                files
                    .into_iter()
                    .filter_map(|name| name.strip_suffix(".json").map(str::to_owned))
                    .collect()
            } else {
                match sunrise_world::json_folder::list(&dir) {
                    Ok(files) => files.into_iter().map(|(name, _)| name).collect(),
                    Err(e) if matches!(e.kind(), NotFound | NotADirectory) => continue,
                    Err(e) => return Err(format!("{}: {e}", dir.display()).into()),
                }
            };
            vanilla_tags.insert(kind, names);
        }
        Ok(Known {
            blocks,
            items,
            biomes,
            biome_tags: keys(BIOME_TAGS)?,
            tag_files: tags.keys().cloned().collect(),
            vanilla_tags,
        })
    }
    /// Whether tag `file` keeps entry `v`: a known id or an existing tag,
    /// for block, item and biome tags (other registries are kept as they
    /// are). The worldgen loader refuses a biome tag naming what it lacks
    /// (another mod's biomes, a loader's tags), so those entries go.
    fn keeps(&self, file: &str, v: &J, warnings: &mut Vec<String>) -> bool {
        let id = match v {
            J::Str(s) => s.as_str(),
            J::Obj(_) => match v.get("id") {
                Some(J::Str(s)) => s.as_str(),
                _ => return true,
            },
            _ => return true,
        };
        let Some((ns, rest)) = file
            .strip_prefix("data/")
            .and_then(|r| r.split_once("/tags/"))
        else {
            return true;
        };
        let Some((kind, tag_path)) = rest.strip_suffix(".json").and_then(|rest| {
            rest.strip_prefix("worldgen/biome/")
                .map(|path| ("worldgen/biome", path))
                .or_else(|| rest.split_once('/'))
        }) else {
            return true;
        };
        let ids = match kind {
            "block" => &self.blocks,
            "item" => &self.items,
            "worldgen/biome" => &self.biomes,
            _ => return true,
        };
        match id.strip_prefix('#') {
            Some(tag) => {
                let (tns, path) = tag.split_once(':').unwrap_or(("minecraft", tag));
                let rel = format!("data/{tns}/tags/{kind}/{path}.json");
                if self.tag_files.contains(&rel)
                    || sunrise_world::tag_layers::convention::has_tag(
                        kind,
                        &format!("{tns}:{path}"),
                    )
                {
                    return true;
                }
                match kind {
                    "worldgen/biome" => self.biome_tags.contains(&format!("{tns}:{path}")),
                    _ if tns != "minecraft" => false,
                    _ => {
                        let known = self
                            .vanilla_tags
                            .get(kind)
                            .is_none_or(|tags| tags.contains(path));
                        if !known
                            && v.get("required") != Some(&J::Bool(false))
                            && v.get("remove") != Some(&J::Bool(true))
                        {
                            warnings.push(format!(
                                "{file}: skipped {id} in tag {ns}:{tag_path}: no such tag in the base or installed mods"
                            ));
                        }
                        known
                    }
                }
            }
            None => ids.contains(id),
        }
    }
}

/// Threads rewriting the tables at once. Each holds a whole table as a tree
/// (gameplay-facts.json's is over a gigabyte with ATM10's mods), so more
/// would mostly add memory.
const TABLE_THREADS: usize = 3;

/// The base's per-state tables, the largest first, with their directories
/// made real in the overlay: [`tables`]' threads then only write files.
fn table_jobs(base: &Path, overlay: &Overlay<'_>) -> Result<Vec<(&'static str, u64)>> {
    let mut jobs: Vec<(&str, u64)> = JSON_TABLES
        .iter()
        .chain(&NBT_TABLES)
        .filter_map(|name| Some((*name, fs::metadata(base.join(name)).ok()?.len())))
        .collect();
    jobs.sort_by_key(|(_, size)| std::cmp::Reverse(*size));
    for (name, _) in &jobs {
        if let Some(parent) = Path::new(name).parent() {
            overlay.dir(parent)?;
        }
    }
    Ok(jobs)
}

/// Rewrites the base's per-state tables (`jobs`, from [`table_jobs`]) for
/// the mods' states into the overlay. They are independent:
/// [`TABLE_THREADS`] threads take them, the largest first.
fn tables(base: &Path, plan: &Plan, overlay: &Overlay<'_>, jobs: &[(&str, u64)]) -> Result<()> {
    let table = |name: &str| -> Result<()> {
        let path = base.join(name);
        let bytes = if name == "entity-types.json" {
            plan.entity_types_json(&fs::read(&path)?)
                .map_err(|e| format!("{name}: {e}"))?
        } else if JSON_TABLES.contains(&name) {
            let mut doc = read_json(&path)?;
            plan.json(name, &mut doc)
                .map_err(|e| format!("{name}: {e}"))?;
            serde_json::to_vec(&doc)?
        } else {
            let (mut named, _) = nbt::read_named(&fs::read(&path)?, Limits::tables())
                .map_err(|e| format!("{name}: {e}"))?;
            plan.nbt(&mut named.tag);
            if name == "cached-collision.nbt" || name == "worldgen/blocks/cached-collision.nbt" {
                plan.declared_shapes(&mut named.tag, true)?;
                plan.published_states(&mut named.tag)?;
                plan.scaffolding_states(&mut named.tag)?;
            } else if name == "worldgen/blocks/cached-support.nbt" {
                plan.published_states(&mut named.tag)?;
            } else if name == "occlusion.nbt" {
                plan.declared_shapes(&mut named.tag, false)?;
            } else if name == "worldgen/blocks/block-properties.nbt" {
                let occlusion = named
                    .tag
                    .get_mut("occlusion")
                    .ok_or("block properties have no occlusion")?;
                plan.declared_shapes(occlusion, false)?;
            }
            nbt::write_named(&named, Limits::tables()).map_err(|e| format!("{name}: {e}"))?
        };
        overlay.write(name, &bytes)
    };
    let next = std::sync::atomic::AtomicUsize::new(0);
    let threads = sunrise_threads::worker_count();
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads.min(TABLE_THREADS))
            .map(|_| {
                scope.spawn(|| -> std::result::Result<(), String> {
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some((name, _)) = jobs.get(i) else {
                            return Ok(());
                        };
                        table(name).map_err(|e| e.to_string())?;
                    }
                })
            })
            .collect();
        workers.into_iter().try_for_each(|w| {
            w.join()
                .unwrap_or_else(|_| Err("a table thread panicked".into()))
        })
    })?;
    Ok(())
}

struct ModLayer<'a> {
    data: crate::datapack::DataPack,
    jar: Option<&'a JarRef>,
}

impl<'a> ModLayer<'a> {
    fn new(
        id: String,
        source: sunrise_level::pack_meta::Source,
        jar: Option<&'a JarRef>,
    ) -> Result<Self> {
        let source = source.open()?;
        let data = crate::datapack::DataPack::from_source(id, source);
        Ok(Self { data, jar })
    }
}

struct ModLayers<'a> {
    source: &'a ContentMod,
    layers: Vec<ModLayer<'a>>,
}

/// Open every input before writing any decoded output.
fn load_layers<'a>(jar_store: &Path, mods: &'a [ContentMod]) -> Result<Vec<ModLayers<'a>>> {
    let requirements =
        collect_jar_requirements(mods.iter().map(|m| (m.id.as_str(), m.jars.as_slice())))?;
    if let Some(missing) = missing_jars(jar_store, &requirements)?.into_iter().next() {
        return Err(missing.into());
    }
    let mut loaded = Vec::new();
    for m in mods {
        let mut layers = Vec::new();
        for jar in &m.jars {
            layers.push(ModLayer::new(
                format!("{}/{}", m.id, jar.id()),
                sunrise_level::pack_meta::Source::Zip(jar.cache_path(jar_store)),
                Some(jar),
            )?);
        }
        if !m.content.files.is_empty() {
            use sunrise_level::pack_meta::Source;
            let source = if let Some(content) = m.content.admitted_source() {
                use std::hash::{DefaultHasher, Hash, Hasher};
                use sunrise_assets::resource_source::ResourceSource;
                use sunrise_mod_package::content::ReadError;
                let reader = content.clone();
                Source::Opened(ResourceSource::fixed(
                    content.names(),
                    move |name, limit| {
                        let reader = reader.reader();
                        limit
                            .map_or_else(|| reader.read(name), |max| reader.read_head(name, max))
                            .map_err(|error| {
                                let kind = match error {
                                    ReadError::NotFound(_) => std::io::ErrorKind::NotFound,
                                    ReadError::Other(_) => std::io::ErrorKind::InvalidData,
                                };
                                std::io::Error::new(kind, error)
                            })
                    },
                    move |name| {
                        let mut hash = DefaultHasher::new();
                        if content.is_archive() {
                            content.reader().read(name).ok()?.hash(&mut hash);
                        } else {
                            let path = content.reader().path(name).ok()??;
                            let meta = fs::metadata(path).ok()?;
                            meta.len().hash(&mut hash);
                            meta.modified().ok()?.hash(&mut hash);
                        }
                        Some(hash.finish())
                    },
                ))
            } else {
                let source = m.content.source().to_owned();
                if source.is_dir() {
                    Source::Dir(source)
                } else {
                    Source::Zip(source)
                }
            };
            layers.push(ModLayer::new(m.id.clone(), source, None)?);
        }
        loaded.push(ModLayers { source: m, layers });
    }
    Ok(loaded)
}

/// Builds generated content tables for `mods` over `base` in `out`.
/// Asset layers remain in their sources and are resolved by `resolved_resources`.
/// Each jar's file is `<sha512>.jar` in `jar_store`, where the launcher
/// puts it. A missing file refuses the load before decoded output is written.
/// Returns warnings for shared tag replacements, missing required vanilla tag
/// references and duplicate asset paths.
pub fn build(
    base: &Path,
    jar_store: &Path,
    mods: &[ContentMod],
    out: &Path,
    inventory: &sunrise_mod_package::enabled::EnabledMods,
) -> Result<Vec<String>> {
    build_with_conditions(
        base,
        jar_store,
        mods,
        out,
        inventory,
        &sunrise_world::conditions::LoadConditions::new(inventory.server_ids()),
    )
}

const GENERATED_FILES: &str = "sunrise-generated-content-files.json";

/// Open the passive layers for one load. Assets and data retain the same jar
/// readers; only files the table builder generated override the raw data.
pub fn resolved_resources_with(
    base: &Path,
    prepared: PathBuf,
    jar_store: &Path,
    mods: &[ContentMod],
    cache: PathBuf,
    merge_assets: impl FnOnce(
        sunrise_assets::resource_source::ResourceSource,
        Vec<(String, sunrise_assets::resource_source::ResourceSource)>,
    ) -> std::io::Result<sunrise_assets::resource_source::ResourceSource>,
) -> Result<sunrise_assets::resource_source::ResourceRoot> {
    use sunrise_assets::resource_source::{Resource, ResourceRoot, ResourceSource};
    let base_source = ResourceSource::directory(base.to_path_buf());
    let loaded = load_layers(jar_store, mods)?;
    let mut data = base_source
        .list("data/")
        .into_iter()
        .map(|name| {
            let resource = Resource::file(base_source.clone(), name.clone());
            (name, resource)
        })
        .collect::<BTreeMap<_, _>>();
    let mut changes = BTreeMap::new();
    for group in &loaded {
        let declarations = format!("data/{}/sunrise/", group.source.id);
        for layer in &group.layers {
            for name in layer.data.list("data/") {
                if !name.starts_with(&declarations) {
                    changes.insert(
                        name.clone(),
                        Resource::file(layer.data.source().clone(), name),
                    );
                }
            }
        }
    }
    let mut assets_base = base_source.clone();
    if prepared != base {
        let generated: Vec<String> =
            serde_json::from_slice(&fs::read(prepared.join(GENERATED_FILES))?)?;
        let generated_source = ResourceSource::directory(prepared.clone());
        root_lifetime::retain_source(&prepared, &generated_source);
        let mut generated_assets = BTreeMap::new();
        for name in generated {
            if !sunrise_assets::resource_source::stays_inside(&name) {
                return Err(format!("invalid generated resource name: {name}").into());
            }
            let resource = Resource::file(generated_source.clone(), name.clone());
            if name.starts_with("data/") {
                changes.insert(name, resource);
            } else if name.starts_with("assets/") {
                generated_assets.insert(name, resource);
            }
        }
        if !generated_assets.is_empty() {
            let mut files = base_source
                .list("")
                .into_iter()
                .map(|name| {
                    let resource = Resource::file(base_source.clone(), name.clone());
                    (name, resource)
                })
                .collect::<BTreeMap<_, _>>();
            files.extend(generated_assets);
            assets_base = ResourceSource::indexed(files);
        }
    }
    let data_layers: std::sync::Arc<[(String, ResourceSource)]> = loaded
        .iter()
        .flat_map(|group| &group.layers)
        .map(|layer| (layer.data.id.clone(), layer.data.source().clone()))
        .collect();
    let mod_layers = data_layers
        .iter()
        .map(|(_, source)| source.clone())
        .collect();
    let layer_sources = data_layers.iter().cloned().collect::<Vec<_>>();
    // Client resource readers also consume passive registry data (for
    // example trim materials and dimension effects). Selected asset packs
    // remain above the asset names without changing the server data source.
    if !changes.is_empty() {
        let mut files = assets_base
            .list("")
            .into_iter()
            .map(|name| {
                let resource = Resource::file(assets_base.clone(), name.clone());
                (name, resource)
            })
            .collect::<BTreeMap<_, _>>();
        files.extend(changes.clone());
        assets_base = ResourceSource::indexed(files);
    }
    let assets = merge_assets(assets_base, layer_sources)?;
    data.extend(changes.clone());
    Ok(ResourceRoot::new(prepared, cache, assets)
        .with_data(
            ResourceSource::indexed(data),
            ResourceSource::indexed_with_sources(changes, data_layers),
        )
        .with_mod_layers(mod_layers))
}

/// Open the passive layers for a server load without client pack merging.
pub fn resolved_server_resources(
    base: &Path,
    prepared: PathBuf,
    jar_store: &Path,
    mods: &[ContentMod],
    cache: PathBuf,
) -> Result<sunrise_assets::resource_source::ResourceRoot> {
    use sunrise_assets::resource_source::{Resource, ResourceSource};
    resolved_resources_with(
        base,
        prepared,
        jar_store,
        mods,
        cache,
        |assets_base, layer_sources| {
            if layer_sources.is_empty() {
                return Ok(assets_base);
            }
            let mut files = assets_base
                .list("")
                .into_iter()
                .map(|name| (name.clone(), Resource::file(assets_base.clone(), name)))
                .collect::<BTreeMap<_, _>>();
            for (_, source) in layer_sources {
                for name in source.list("assets/") {
                    files.insert(name.clone(), Resource::file(source.clone(), name));
                }
            }
            Ok(ResourceSource::indexed(files))
        },
    )
}

/// The root to load: `base` itself with no content, else the mods' overlay
/// over it, built in `cache/mod-content/<stamp>`: one folder per mod set, so
/// a running game keeps reading the root it was given while a switch builds
/// another. Roots of other sets unused for a day are removed.
///
/// The root is built once and kept: later starts with the same build, base
/// and mod files reuse it. With ATM10's carried mods a build takes about
/// 25 s and peaks above 3 GB, more than everything the game keeps.
pub fn resource_root(
    base: &Path,
    jar_store: &Path,
    mods: &[ContentMod],
    cache: &Path,
    inventory: &sunrise_mod_package::enabled::EnabledMods,
) -> Result<PathBuf> {
    resource_root_with_conditions(
        base,
        jar_store,
        mods,
        cache,
        inventory,
        &sunrise_world::conditions::LoadConditions::new(inventory.server_ids()),
    )
}

/// Declared native factory assignments, sorted by block id. Validity is extended
/// after content resolution in the canonical `block_entity_types` table.
pub const BLOCK_ENTITY_TYPES: &str = "sunrise-block-entity-types.json";

/// The length of a root's folder name: the start of its stamp.
const ROOT_NAME: usize = 16;
/// How long a root no start or switch used is kept.
const KEEP_ROOTS: std::time::Duration = std::time::Duration::from_secs(24 * 3600);
/// The file beside `root` whose time is its last reuse (`<root>.used`).
fn used_path(root: &Path) -> PathBuf {
    root.with_extension("used")
}

/// In the roots folder: held while a root is checked, built or pruned there, and never deleted.
const ROOTS_LOCK: &str = ".lock";

/// Where the base really is: the root links into it, so the key and the links use this path.
fn real_base(base: &Path) -> Result<PathBuf> {
    Ok(fs::canonicalize(base).map_err(|e| format!("{}: {e}", base.display()))?)
}

/// Whether turning a content mod on or off can apply in a running game:
/// its assets and data can, through the resource reload and the server's data
/// reload. What a world numbers or generates from cannot yet: then the
/// translation key of why.
pub fn live_switch(content: &Content) -> std::result::Result<(), &'static str> {
    if !content.blocks.is_empty()
        || !content.items.is_empty()
        || !content.effects.is_empty()
        || !content.fluids.is_empty()
        || !content.entity_types.is_empty()
        || !content.menu_types.is_empty()
        || !content.creative_tabs.is_empty()
        || !content.block_entity_types.is_empty()
    {
        return Err("sunrise.mods.switch.registries");
    }
    let worldgen = |file: &&String| {
        let mut parts = file.split('/');
        parts.next() == Some("data")
            && parts.nth(1).is_some_and(|folder| {
                matches!(folder, "worldgen" | "dimension" | "dimension_type")
                    || folder == "tags" && file.contains("/tags/worldgen/")
                    || matches!(folder, "neoforge" | "forge")
                        && (file.contains("/biome_modifier/")
                            || file.contains("/structure_modifier/"))
            })
    };
    if content.files.iter().any(|f| worldgen(&f)) {
        return Err("sunrise.mods.switch.worldgen");
    }
    Ok(())
}

/// In a root built by [`build`]: its warnings, one per line.
const WARNINGS: &str = "sunrise-mod-content-warnings.txt";

/// In a root built by [`resource_root`]: what it was built from.
const STAMP: &str = "sunrise-mod-content.stamp";
const JAR_INPUTS: &str = "sunrise-mod-jar-inputs.json";

/// Check the accepted inputs of a composed resource root before server reload.
/// This reads no candidate packages and never reopens or rehashes an archive.
pub fn check_jars(root: &Path, jar_store: &Path) -> std::result::Result<(), JarInputsError> {
    let marker = root.join(MARKER);
    if !marker.try_exists().map_err(|source| JarInputsError::Io {
        path: marker,
        source,
    })? {
        return Ok(());
    }
    let path = root.join(JAR_INPUTS);
    let bytes = fs::read(&path).map_err(|source| JarInputsError::Io {
        path: path.clone(),
        source,
    })?;
    let requirements: Vec<sunrise_mod_package::manifest::JarRequirement> =
        serde_json::from_slice(&bytes)
            .map_err(|source| JarInputsError::Invalid { path, source })?;
    let missing = missing_jars(jar_store, &requirements).map_err(|source| JarInputsError::Io {
        path: jar_store.to_owned(),
        source,
    })?;
    if let Some(missing) = missing.into_iter().next() {
        return Err(JarInputsError::Missing(Box::new(missing)));
    }
    Ok(())
}

/// What a content root is built from: this build of the game, the base's
/// manifest and the tables [`build`] rewrites, and every file of every mod
/// in order. The base's tables count by path, size and modification time (the
/// root links into the base, so its real path belongs in the key). The
/// game's build (its commit, else its executable's contents) and every mod
/// file count by name and contents, not by where they sit, so identical
/// copies in other folders share one root. A jar is
/// named by its SHA-512 in `m.jars`, and counts only if it is in `jar_store`.
fn stamp(
    base: &Path,
    jar_store: &Path,
    mods: &[ContentMod],
    inventory: &sunrise_mod_package::enabled::EnabledMods,
) -> Result<String> {
    use sha2::{Digest, Sha256};
    let meta = |path: &Path| -> Result<String> {
        let m = fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let modified = m
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        Ok(format!(
            "{} {} {}",
            path.display(),
            m.len(),
            modified.as_nanos()
        ))
    };
    let hash = |path: &Path| -> Result<String> {
        let mut hasher = Sha256::new();
        let mut file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        std::io::copy(&mut file, &mut hasher).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(format!("{:x}", hasher.finalize()))
    };
    // This build of the game: its commit when the build knows it, else the
    // executable's contents (once per process, not on every stamp). Contents, not size and
    // time: copies of one executable in other folders must share a root.
    static EXE: std::sync::OnceLock<std::result::Result<String, String>> =
        std::sync::OnceLock::new();
    let exe = match crate::save::level_mods::build_commit() {
        Some(commit) => commit.to_owned(),
        None => EXE
            .get_or_init(|| {
                let exe = std::env::current_exe().map_err(|e| e.to_string())?;
                hash(&exe).map_err(|e| e.to_string())
            })
            .clone()?,
    };
    let mut parts = vec![
        "sunrise mod content 10".to_owned(),
        serde_json::to_string(inventory)?,
    ];
    parts.push(exe);
    // The root links into the base: where it really is, not how this folder reaches it.
    let base = real_base(base)?;
    parts.push(
        String::from_utf8_lossy(&fs::read(base.join("sunrise-resources.json"))?).into_owned(),
    );
    for name in JSON_TABLES.iter().chain(&NBT_TABLES).chain(&TAG_BUNDLES) {
        if base.join(name).exists() {
            parts.push(meta(&base.join(name))?);
        }
    }
    for m in mods {
        parts.push(m.id.clone());
        parts.push(format!("jars {}", serde_json::to_string(&m.jars)?));
        // A jar's file is named by its SHA-512: only whether it is still in the
        // store (and its size) counts, so a root isn't reused once one is gone.
        for jar in &m.jars {
            parts.push(match fs::metadata(jar.cache_path(jar_store)) {
                Ok(file) => format!("cached {}", file.len()),
                Err(_) => "not cached".into(),
            });
        }
        // What it declares, so blocks and items from code count too.
        parts.push(format!(
            "{:?} {:?} {:?}",
            m.content.blocks, m.content.items, m.content.block_entity_types
        ));
        if m.content.files.is_empty() {
            // Declared in code only: nothing to read.
            continue;
        }
        let source = m.content.source();
        let admitted = m.content.admitted_source();
        if let Some(content) = admitted.as_ref().filter(|content| content.is_archive()) {
            let reader = content.reader();
            for file in m.content.files.iter() {
                let digest = Sha256::digest(reader.read(file)?);
                parts.push(format!("{file} {digest:x}"));
            }
        } else if admitted.is_some() || source.is_dir() {
            for file in m.content.files.iter() {
                parts.push(format!("{file} {}", hash(&source.join(file))?));
            }
        } else {
            parts.push(hash(source)?);
        }
    }
    let mut hasher = Sha256::new();
    for part in &parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_fixtures;

#[cfg(test)]
#[path = "mod_content_tests.rs"]
mod tests;
