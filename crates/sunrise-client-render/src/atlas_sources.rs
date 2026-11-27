//! Atlas sprite sources (`SpriteSourceList`): the `atlases/*.json`
//! definitions listing which sprites an atlas holds. `directory`, `single`,
//! `filter`, `unstitch` and `paletted_permutations` sources, and the types mods
//! register ([`crate::mod_sprite_sources`]), run in order against the resource
//! tree; a later source replaces an earlier sprite of the same id and a filter
//! removes the matching ones.
use mod_api::{Identifier, ResourceReader, SpriteSourceOutput};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use sunrise_assets::{animation::AnimationMetadata, palette::apply_palette};
use sunrise_core::nbt::Tag;
use sunrise_game_data::pattern::Pattern;

/// One region of an `unstitch` source, in units of the divisors.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub sprite: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Clone, Debug)]
pub enum SpriteSource {
    Directory {
        source: String,
        prefix: String,
    },
    Single {
        resource: String,
        sprite: Option<String>,
    },
    Filter {
        namespace: Option<Pattern>,
        path: Option<Pattern>,
    },
    Unstitch {
        resource: String,
        regions: Vec<Region>,
        divisor_x: f64,
        divisor_y: f64,
    },
    PalettedPermutations {
        textures: Vec<String>,
        palette_key: String,
        permutations: BTreeMap<String, String>,
        separator: String,
    },
    /// An entry of a type a mod registered: its source answers it when the atlas is listed.
    Registered {
        /// The type id (`mymod:checker`).
        kind: String,
        /// The entry's object, converted once at load.
        config: Arc<Tag>,
    },
}
/// Where a listed sprite's pixels come from.
#[derive(Clone, Debug, PartialEq)]
pub enum SpriteOrigin {
    /// A whole texture (`textures/<path>.png`), by texture identifier.
    File(String),
    /// `Unstitcher.RegionInstance`: part of a texture.
    Region {
        texture: String,
        region: Region,
        divisor_x: f64,
        divisor_y: f64,
    },
    /// `PalettedSpriteSupplier`: a texture with its colours remapped.
    Paletted {
        texture: String,
        palette_key: String,
        palette: String,
    },
    /// Pixels a mod's sprite source made.
    Pixels(Image),
}

/// `Identifier.parse` default namespace: bare names and leading-colon names (`:single`) resolve
/// into `minecraft:`.
pub fn qualified(id: &str) -> String {
    match id.split_once(':') {
        Some(("", path)) => format!("minecraft:{path}"),
        Some(_) => id.to_owned(),
        None => format!("minecraft:{id}"),
    }
}
fn string(v: &Value, key: &str) -> Option<String> {
    v.get(key)?.as_str().map(str::to_owned)
}
fn number(v: &Value, key: &str, default: f64) -> Option<f64> {
    match v.get(key) {
        None => Some(default),
        Some(n) => n.as_f64(),
    }
}

/// Why an atlas definition can't be decoded ([`decode`]).
#[derive(Debug)]
pub enum DefinitionError {
    /// The file isn't JSON.
    Json(serde_json::Error),
    /// It has no `sources` list.
    NoSources,
    /// The entry at `index` doesn't decode as its type.
    Malformed { index: usize },
    /// The entry at `index` is of a type neither built in nor registered by a mod.
    UnknownType { index: usize, kind: String },
}
impl std::fmt::Display for DefinitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(error) => error.fmt(f),
            Self::NoSources => f.write_str("no `sources` list"),
            Self::Malformed { index } => write!(f, "sources[{index}]: malformed source"),
            Self::UnknownType { index, kind } => {
                write!(f, "sources[{index}]: unknown sprite source type {kind}")
            }
        }
    }
}
impl std::error::Error for DefinitionError {}

/// The source types native mods registered and what their sources answer
/// ([`crate::mod_sprite_sources::Registered`] in the game).
pub trait ModSources: Sync {
    /// Whether a mod registered `kind` as a sprite source type.
    fn is_registered(&self, kind: &str) -> bool;
    /// What a mod's source answers for an entry of its type in `atlas`, reading the load's
    /// `resources`.
    fn ask(
        &self,
        atlas: &Identifier,
        kind: &str,
        config: &Tag,
        resources: &Arc<dyn ResourceReader>,
    ) -> Vec<SpriteSourceOutput>;
}

/// `SpriteSources.FILE_CODEC`: the `sources` list of an atlas definition.
/// A malformed source fails the whole file, as the codec does; the error names
/// the first one, and its type when no source has it.
pub fn decode(text: &str, mods: &dyn ModSources) -> Result<Vec<SpriteSource>, DefinitionError> {
    let json: Value = serde_json::from_str(text).map_err(DefinitionError::Json)?;
    let sources = json
        .get("sources")
        .and_then(Value::as_array)
        .ok_or(DefinitionError::NoSources)?;
    sources
        .iter()
        .enumerate()
        .map(|(index, source)| {
            let kind = source
                .get("type")
                .and_then(Value::as_str)
                .and_then(|raw| Identifier::parse(raw).ok())
                .map(|id| id.to_string());
            decode_source(source, mods).ok_or_else(|| match kind {
                Some(kind) if !known(&kind, mods) => DefinitionError::UnknownType { index, kind },
                _ => DefinitionError::Malformed { index },
            })
        })
        .collect()
}
/// Whether `kind` is one of vanilla's built-in sprite source types (`SpriteSources.bootstrap`).
pub(crate) fn is_builtin_type(kind: &str) -> bool {
    matches!(
        kind,
        "minecraft:directory"
            | "minecraft:single"
            | "minecraft:filter"
            | "minecraft:unstitch"
            | "minecraft:paletted_permutations"
    )
}
/// Whether a source of type `kind` can be decoded: a built-in one or one a mod registered.
fn known(kind: &str, mods: &dyn ModSources) -> bool {
    is_builtin_type(kind) || mods.is_registered(kind)
}
fn decode_source(s: &Value, mods: &dyn ModSources) -> Option<SpriteSource> {
    let kind = Identifier::parse(s.get("type")?.as_str()?)
        .ok()?
        .to_string();
    if mods.is_registered(&kind) {
        return Some(SpriteSource::Registered {
            kind,
            config: Arc::new(sunrise_core::json_nbt::json_to_tag(s).ok()?),
        });
    }
    Some(match kind.as_str() {
        "minecraft:directory" => SpriteSource::Directory {
            source: string(s, "source")?,
            prefix: string(s, "prefix")?,
        },
        "minecraft:single" => SpriteSource::Single {
            resource: qualified(&string(s, "resource")?),
            sprite: match s.get("sprite") {
                None => None,
                Some(v) => Some(qualified(v.as_str()?)),
            },
        },
        "minecraft:filter" => {
            let p = s.get("pattern")?;
            let pattern = |key: &str| -> Option<Option<Pattern>> {
                match p.get(key) {
                    None => Some(None),
                    Some(v) => Pattern::new(v.as_str()?).map(Some),
                }
            };
            SpriteSource::Filter {
                namespace: pattern("namespace")?,
                path: pattern("path")?,
            }
        }
        "minecraft:unstitch" => {
            let regions = s
                .get("regions")?
                .as_array()?
                .iter()
                .map(|r| {
                    Some(Region {
                        sprite: qualified(&string(r, "sprite")?),
                        x: r.get("x")?.as_f64()?,
                        y: r.get("y")?.as_f64()?,
                        width: r.get("width")?.as_f64()?,
                        height: r.get("height")?.as_f64()?,
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            if regions.is_empty() {
                return None;
            }
            SpriteSource::Unstitch {
                resource: qualified(&string(s, "resource")?),
                regions,
                divisor_x: number(s, "divisor_x", 1.0)?,
                divisor_y: number(s, "divisor_y", 1.0)?,
            }
        }
        "minecraft:paletted_permutations" => SpriteSource::PalettedPermutations {
            textures: s
                .get("textures")?
                .as_array()?
                .iter()
                .map(|t| t.as_str().map(qualified))
                .collect::<Option<_>>()?,
            palette_key: qualified(&string(s, "palette_key")?),
            permutations: s
                .get("permutations")?
                .as_object()?
                .iter()
                .map(|(k, v)| Some((k.clone(), qualified(v.as_str()?))))
                .collect::<Option<_>>()?,
            separator: match s.get("separator") {
                None => "_".to_owned(),
                Some(v) => v.as_str()?.to_owned(),
            },
        },
        _ => return None,
    })
}

/// The texture files of a resource tree: `namespace:path` for every
/// `assets/<namespace>/textures/<path>.png`.
pub fn list_textures(source: &crate::resource_source::ResourceSource) -> Vec<String> {
    source
        .list_under("assets", &["textures"])
        .into_iter()
        .filter_map(|file| {
            let (namespace, texture) = file.strip_prefix("assets/")?.split_once("/textures/")?;
            Some(format!("{namespace}:{}", texture.strip_suffix(".png")?))
        })
        .collect()
}
/// The file of a texture identifier. A Custom Item Textures id (`cit_assets`) names its
/// resource path, wherever the folder: `directory` is not used for it. An id whose path would
/// leave the root (a pack's model may write any) names a file that is never there.
pub fn texture_path(root: &Path, texture: &str, directory: &str) -> PathBuf {
    root.join(texture_resource(texture, directory))
}
/// Resource name of a texture, including Custom Item Textures outside `textures/`.
pub fn texture_resource(texture: &str, directory: &str) -> String {
    // No file can have a NUL in its name, so this one is never there.
    const NOWHERE: &str = "assets/minecraft/textures/sunrise/\0refused.png";
    if directory == "textures" && texture.starts_with("sunrise_cit:") {
        return crate::cit_assets::resource(texture, "png").unwrap_or_else(|| NOWHERE.to_owned());
    }
    let (namespace, path) = texture.split_once(':').unwrap_or(("minecraft", texture));
    let rel = format!("assets/{namespace}/{directory}/{path}.png");
    if sunrise_optifine::paths::valid(&rel) {
        rel
    } else {
        NOWHERE.to_owned()
    }
}

/// What a mod's source answers for an entry of its type: the type and the entry's object.
pub type Ask<'a> = dyn FnMut(&str, &Tag) -> Vec<SpriteSourceOutput> + 'a;

/// The sprites one atlas's definition lists.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Listed {
    /// Each sprite id and where its pixels come from.
    pub sprites: BTreeMap<String, SpriteOrigin>,
    /// The ids whose origin a mod's source set: its pixels, or the texture it named.
    pub by_mods: BTreeSet<String>,
    /// The ids a filter or a mod's `Remove` took out and no later source listed again: listed
    /// once, absent from the final list.
    pub removed: BTreeSet<String>,
}
impl Listed {
    /// `id`'s origin replaced by the built-in source that lists it now.
    fn put(&mut self, id: String, origin: SpriteOrigin) {
        self.by_mods.remove(&id);
        self.removed.remove(&id);
        self.sprites.insert(id, origin);
    }
}

/// `SpriteSourceList.list`: every sprite id of the atlas and its origin.
/// `textures` are the available texture identifiers ([`list_textures`]);
/// `ask` answers the entries of a type a mod registered, each once.
pub fn list(sources: &[SpriteSource], textures: &[String], ask: &mut Ask<'_>) -> Listed {
    let mut out = Listed::default();
    let exists = |t: &str| textures.binary_search_by(|x| x.as_str().cmp(t)).is_ok();
    for source in sources {
        match source {
            SpriteSource::Directory { source, prefix } => {
                // FileToIdConverter("textures/" + source, ".png").
                let dir = format!("{source}/");
                for t in textures {
                    let (namespace, path) = t.split_once(':').unwrap_or(("minecraft", t));
                    if let Some(rest) = path.strip_prefix(&dir) {
                        out.put(
                            format!("{namespace}:{prefix}{rest}"),
                            SpriteOrigin::File(t.clone()),
                        );
                    }
                }
            }
            SpriteSource::Single { resource, sprite } => {
                if exists(resource) {
                    out.put(
                        sprite.clone().unwrap_or_else(|| resource.clone()),
                        SpriteOrigin::File(resource.clone()),
                    );
                }
            }
            SpriteSource::Filter { namespace, path } => {
                let kept = |id: &String| {
                    let (ns, p) = id.split_once(':').unwrap_or(("minecraft", id));
                    !(namespace.as_ref().is_none_or(|m| m.is_match(ns))
                        && path.as_ref().is_none_or(|m| m.is_match(p)))
                };
                let gone: Vec<String> =
                    out.sprites.keys().filter(|id| !kept(id)).cloned().collect();
                out.sprites.retain(|id, _| kept(id));
                out.by_mods.retain(kept);
                out.removed.extend(gone);
            }
            SpriteSource::Unstitch {
                resource,
                regions,
                divisor_x,
                divisor_y,
            } => {
                if exists(resource) {
                    for region in regions {
                        out.put(
                            region.sprite.clone(),
                            SpriteOrigin::Region {
                                texture: resource.clone(),
                                region: region.clone(),
                                divisor_x: *divisor_x,
                                divisor_y: *divisor_y,
                            },
                        );
                    }
                }
            }
            SpriteSource::PalettedPermutations {
                textures: bases,
                palette_key,
                permutations,
                separator,
            } => {
                for base in bases {
                    if !exists(base) {
                        continue;
                    }
                    for (suffix, palette) in permutations {
                        out.put(
                            format!("{base}{separator}{suffix}"),
                            SpriteOrigin::Paletted {
                                texture: base.clone(),
                                palette_key: palette_key.clone(),
                                palette: palette.clone(),
                            },
                        );
                    }
                }
            }
            SpriteSource::Registered { kind, config } => {
                for output in ask(kind, config) {
                    apply(kind, output, &exists, &mut out);
                }
            }
        }
    }
    out
}

/// One step of a mod's source on the listed sprites, as `SpriteSource.Output` takes it. What
/// can't be a sprite is logged and skipped by itself.
fn apply(kind: &str, output: SpriteSourceOutput, exists: &dyn Fn(&str) -> bool, out: &mut Listed) {
    let mut make = |sprite: Identifier, origin| {
        let id = sprite.to_string();
        out.by_mods.insert(id.clone());
        out.removed.remove(&id);
        out.sprites.insert(id, origin);
    };
    match output {
        SpriteSourceOutput::Texture { sprite, texture } => {
            let texture = texture.to_string();
            if exists(&texture) {
                make(sprite, SpriteOrigin::File(texture));
            } else {
                log::warn!("sprite source {kind}: sprite {sprite}: no texture {texture}; skipped");
            }
        }
        SpriteSourceOutput::Pixels {
            sprite,
            width,
            height,
            rgba,
        } => {
            // A size from mod code: checked arithmetic, so no input can overflow or panic.
            let length = (width as usize)
                .checked_mul(height as usize)
                .and_then(|pixels| pixels.checked_mul(4));
            if width == 0 || height == 0 {
                log::warn!("sprite source {kind}: sprite {sprite} has no pixels; skipped");
            } else if length != Some(rgba.len()) {
                log::warn!(
                    "sprite source {kind}: sprite {sprite}: {} bytes for {width}x{height} pixels \
                     (4 a pixel); skipped",
                    rgba.len()
                );
            } else if i64::from(width.max(height)) > MAX_SPRITE_SIDE {
                log::warn!(
                    "sprite source {kind}: sprite {sprite}: {width}x{height} exceeds the \
                     {MAX_SPRITE_SIDE}-pixel sprite side; skipped"
                );
            } else {
                make(sprite, SpriteOrigin::Pixels((width, height, rgba)));
            }
        }
        SpriteSourceOutput::Remove { sprite } => {
            let id = sprite.to_string();
            if out.sprites.remove(&id).is_some() {
                out.removed.insert(id.clone());
            }
            out.by_mods.remove(&id);
        }
    }
}

/// An RGBA image.
pub type Image = (u32, u32, Vec<u8>);

/// The largest side of a sprite an `unstitch` region may cut or a mod's source may make, in
/// pixels. Vanilla's `Unstitcher` and `SpriteSource.Output` have no limit of their own, but its
/// stitcher can't place a sprite larger than the device's maximum texture size, and the renderer
/// asks for the adapter's own limits. 32768 is the largest `max_texture_dimension_2d` current
/// drivers report (NVIDIA's Vulkan; Direct3D 12 and Metal stop at 16384), so no sprite vanilla
/// would stitch is refused. A larger declared sprite is skipped with a logged reason, never
/// allocated.
pub use sunrise_level::pack_image::MAX_SPRITE_SIDE;

fn argb(rgba: &[u8]) -> Vec<u32> {
    rgba.as_chunks::<4>()
        .0
        .iter()
        .map(|p| u32::from_be_bytes([p[3], p[0], p[1], p[2]]))
        .collect()
}
/// The pixels of a generated sprite (`Region`, `Paletted` or a mod's `Pixels`);
/// `None` for a plain file, which loads as itself. `load` takes `(id, directory)`.
pub fn generate(
    origin: &SpriteOrigin,
    load: &dyn Fn(&str, &str) -> Option<Image>,
) -> Option<Image> {
    match origin {
        SpriteOrigin::File(_) => None,
        SpriteOrigin::Pixels(image) => Some(image.clone()),
        SpriteOrigin::Region {
            texture,
            region,
            divisor_x,
            divisor_y,
        } => {
            let (w, h, pixels) = load(texture, "textures")?;
            let x_scale = f64::from(w) / divisor_x;
            let y_scale = f64::from(h) / divisor_y;
            // A float-to-integer cast saturates (NaN is 0), so every value here is in range.
            let x = (region.x * x_scale).floor() as i64;
            let y = (region.y * y_scale).floor() as i64;
            let width = (region.width * x_scale).floor() as i64;
            let height = (region.height * y_scale).floor() as i64;
            if width <= 0 || height <= 0 {
                return None;
            }
            // The whole rectangle must lie inside the sheet, as `SpriteContents.createSubImage`
            // takes it: a cut that doesn't fits nothing, and the sprite is absent. Checked before
            // anything is sized, so numbers a pack made up allocate nothing.
            if x < 0
                || y < 0
                || x.saturating_add(width) > i64::from(w)
                || y.saturating_add(height) > i64::from(h)
            {
                log::warn!(
                    "atlas region {} of {texture}: outside the {w}x{h} sheet; skipped",
                    region.sprite
                );
                return None;
            }
            // The sheet is a decoded image, so its side is within the sprite side already.
            let (x, y, width, height) = (x as usize, y as usize, width as usize, height as usize);
            let mut out = Vec::new();
            if out.try_reserve_exact(width * height * 4).is_err() {
                log::warn!(
                    "atlas region {} of {texture}: {width}x{height} could not be allocated; skipped",
                    region.sprite
                );
                return None;
            }
            for row in 0..height {
                let from = ((y + row) * w as usize + x) * 4;
                out.extend_from_slice(&pixels[from..from + width * 4]);
            }
            Some((width as u32, height as u32, out))
        }
        SpriteOrigin::Paletted {
            texture,
            palette_key,
            palette,
        } => {
            let (w, h, pixels) = load(texture, "textures")?;
            // `Palette.ID_CONVERTER` resolves palettes under `textures/palettes/`.
            let (_, _, key) = load(palette_key, "textures/palettes")?;
            let (_, _, target) = load(palette, "textures/palettes")?;
            let mut mapped = argb(&pixels);
            let key = argb(&key);
            let target = argb(&target);
            apply_palette(&mut mapped, &key, &target)?;
            let out: Vec<u8> = mapped
                .iter()
                .flat_map(|p| {
                    let [a, r, g, b] = p.to_be_bytes();
                    [r, g, b, a]
                })
                .collect();
            drop(mapped);
            Some((w, h, out))
        }
    }
}

/// The definition `assets/<namespace>/atlases/<path>.json` of atlas `definition`, decoded;
/// `None` (logged) where there is none or it doesn't decode.
pub fn definition(
    source: &crate::resource_source::ResourceSource,
    mods: &dyn ModSources,
    definition: &Identifier,
) -> Option<Vec<SpriteSource>> {
    let file = format!(
        "assets/{}/atlases/{}.json",
        definition.namespace(),
        definition.path()
    );
    match decode(&source.read_to_string(&file).ok()?, mods) {
        Ok(sources) => Some(sources),
        Err(error) => {
            log::warn!("Sunrise: atlas {definition} skipped: {error}");
            None
        }
    }
}

/// The resources of the load being listed, for one callback of a mod's source: it answers only
/// until the call returns, then lets go of the load, so a reader a source keeps reads nothing and
/// holds nothing.
struct LoadResources(Mutex<Option<crate::resource_source::ResourceSource>>);
impl LoadResources {
    fn source(&self) -> Option<crate::resource_source::ResourceSource> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}
impl ResourceReader for LoadResources {
    /// The last visible layer's own bytes: the topmost pack's file, never the merged document
    /// the game's own readers take of a resource every pack adds to.
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.source()?.stack(path).last()?.read().ok()
    }
    fn read_stack(&self, path: &str) -> Vec<Vec<u8>> {
        self.source().map_or_else(Vec::new, |source| {
            source
                .stack(path)
                .iter()
                .filter_map(|layer| layer.read().ok())
                .collect()
        })
    }
    fn list(&self, prefix: &str) -> Vec<String> {
        self.source()
            .map_or_else(Vec::new, |source| source.list(prefix))
    }
}

/// The sprites `sources` list for `atlas`, each entry of a registered type asked once with a
/// reader of the load's resources that lasts that one call.
fn list_atlas(
    source: &crate::resource_source::ResourceSource,
    textures: &[String],
    mods: &dyn ModSources,
    atlas: &Identifier,
    sources: &[SpriteSource],
) -> Listed {
    list(sources, textures, &mut |kind, config| {
        let resources = Arc::new(LoadResources(Mutex::new(Some(source.clone()))));
        let reader: Arc<dyn ResourceReader> = resources.clone();
        let outputs = mods.ask(atlas, kind, config, &reader);
        *resources.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
        outputs
    })
}

/// The sorted texture identifiers `list` searches (their identifiers sort differently from their
/// paths: `a0:` before `a:`).
fn sorted_textures(source: &crate::resource_source::ResourceSource) -> Vec<String> {
    let mut textures = list_textures(source);
    textures.sort_unstable();
    textures
}

/// The sprites of the atlases a mod registered, each atlas listed from its own definition and
/// asked of the mods under its own id. An atlas whose definition doesn't decode is left out.
/// Once per client prepare, by the staging of the mods' atlases; the game's atlases are
/// [`Listing`]'s.
pub fn registered(
    source: &crate::resource_source::ResourceSource,
    mods: &dyn ModSources,
    atlases: &[(&Identifier, &Identifier)],
) -> BTreeMap<Identifier, Listed> {
    let textures = sorted_textures(source);
    atlases
        .iter()
        .filter_map(|(atlas, definition_id)| {
            let sources = definition(source, mods, definition_id)?;
            Some((
                (*atlas).clone(),
                list_atlas(source, &textures, mods, atlas, &sources),
            ))
        })
        .collect()
}

/// Registration order and atlas ownership of sprites staged by `register_texture_atlases` for
/// one resource generation, preserved through `HudAtlas` placement so an oversized atlas is
/// diagnosed and isolated without affecting the base HUD or other mod atlases.
#[derive(Debug, Default)]
pub struct StagedTextureAtlases {
    /// Admitted registrations in order: `(atlas_id, mod_id)`.
    pub order: Vec<(String, &'static str)>,
    /// Staged sprite identifier (`<namespace>:gui/atlases/...`) mapped to its owning index in
    /// [`Self::order`].
    pub sprite_atlas: HashMap<String, usize>,
}

/// Built-in engine atlas paths under `minecraft:` (`AtlasManager.KNOWN_ATLASES` in vanilla 26.3).
pub const KNOWN_ATLASES: &[&str] = &[
    "banner_patterns",
    "blocks",
    "items",
    "chests",
    "decorated_pot",
    "gui",
    "map_decorations",
    "paintings",
    "particles",
    "shield_patterns",
    "shulker_boxes",
    "celestials",
];

/// Whether `id` names one of vanilla's built-in engine atlases (`AtlasManager.KNOWN_ATLASES`).
pub fn is_builtin_atlas(id: &Identifier) -> bool {
    id.namespace() == "minecraft" && KNOWN_ATLASES.contains(&id.path())
}

/// Sprite listings of the engine atlases (`AtlasManager.KNOWN_ATLASES`). A definition on its own
/// does not register an atlas. Mod-registered atlases are listed under their own IDs by
/// [`registered`].
#[derive(Debug, Default)]
pub struct Listing {
    atlases: BTreeMap<String, Listed>,
}
impl Listing {
    /// The listing of `source`, asking `mods` for each entry of a registered type.
    pub fn build(source: &crate::resource_source::ResourceSource, mods: &dyn ModSources) -> Self {
        let textures = sorted_textures(source);
        let atlases = KNOWN_ATLASES
            .iter()
            .copied()
            .filter_map(|atlas| {
                let id = Identifier::parse(atlas).expect("known engine atlas id");
                let sources = definition(source, mods, &id)?;
                Some((
                    id.to_string(),
                    list_atlas(source, &textures, mods, &id, &sources),
                ))
            })
            .collect();
        Self { atlases }
    }
    /// The sprites `atlas` lists; none for an atlas without a definition.
    pub fn sprites(&self, atlas: &str) -> &BTreeMap<String, SpriteOrigin> {
        static NONE: BTreeMap<String, SpriteOrigin> = BTreeMap::new();
        self.atlases
            .get(atlas)
            .map_or(&NONE, |listed| &listed.sprites)
    }
    /// The sprites a filter or a mod's source took out of `atlas`'s list.
    pub fn removed(&self, atlas: &str) -> impl Iterator<Item = &String> {
        self.atlases
            .get(atlas)
            .into_iter()
            .flat_map(|listed| &listed.removed)
    }
    /// The sprites a mod's source made in `atlas`.
    pub fn by_mods(&self, atlas: &str) -> impl Iterator<Item = &String> {
        self.atlases
            .get(atlas)
            .into_iter()
            .flat_map(|listed| &listed.by_mods)
    }
    /// Whether `atlas` has a definition that decodes.
    pub fn defines(&self, atlas: &str) -> bool {
        self.atlases.contains_key(atlas)
    }
    /// Every atlas and its sprites, in atlas order.
    pub fn atlases(&self) -> impl Iterator<Item = (&String, &BTreeMap<String, SpriteOrigin>)> {
        self.atlases
            .iter()
            .map(|(atlas, listed)| (atlas, &listed.sprites))
    }
    /// Identifies the whole listing, mods' answers included: what a cache of results derived
    /// from it keys by, so code that answers differently never meets an old result.
    pub fn fingerprint(&self) -> u64 {
        let mut hash = DefaultHasher::new();
        for (atlas, listed) in &self.atlases {
            atlas.hash(&mut hash);
            for (id, origin) in &listed.sprites {
                id.hash(&mut hash);
                match origin {
                    SpriteOrigin::File(texture) => (0u8, texture).hash(&mut hash),
                    SpriteOrigin::Region {
                        texture,
                        region,
                        divisor_x,
                        divisor_y,
                    } => (
                        1u8,
                        texture,
                        &region.sprite,
                        [
                            region.x,
                            region.y,
                            region.width,
                            region.height,
                            *divisor_x,
                            *divisor_y,
                        ]
                        .map(f64::to_bits),
                    )
                        .hash(&mut hash),
                    SpriteOrigin::Paletted {
                        texture,
                        palette_key,
                        palette,
                    } => (2u8, texture, palette_key, palette).hash(&mut hash),
                    SpriteOrigin::Pixels(image) => (3u8, image).hash(&mut hash),
                }
            }
        }
        hash.finish()
    }
}

/// The listing of this load of `source`, made once: the first caller asks `mods`, every later one
/// (and every source extended from this one) takes that answer.
pub fn shared(
    source: &crate::resource_source::ResourceSource,
    mods: &dyn ModSources,
) -> Arc<Listing> {
    source.cached(|| Listing::build(source, mods))
}

/// [`shared`] with the native mods' sources: what the game's readers of the atlases take.
pub fn listing(source: &crate::resource_source::ResourceSource) -> Arc<Listing> {
    shared(source, &crate::mod_sprite_sources::Registered)
}

/// The blocks and items atlases as a model material sees them: a material names no atlas, and
/// the baker looks in the items atlas first, then the blocks atlas, then uses the missing sprite.
pub struct Materials {
    listing: Arc<Listing>,
    /// Whether either atlas has a definition file, however it decodes.
    defined: bool,
}

/// A finished sprite: its pixels, its source texture file and animation metadata whose frame
/// size fits the image. Generated sprites have neither a texture file nor animation metadata.
pub struct Material {
    pub image: Image,
    pub resource: Option<String>,
    pub animation: Option<AnimationMetadata>,
}

/// Whether a texture of `size` tiles into the frames its animation declares (the whole image
/// when it has none): or the sprite is no sprite.
pub fn frames_fit((width, height): (u32, u32), animation: Option<&AnimationMetadata>) -> bool {
    let [frame_width, frame_height] = animation
        .map_or([width as i32, height as i32], |animation| {
            animation.frame_size(width as i32, height as i32)
        });
    u32::try_from(frame_width)
        .ok()
        .zip(u32::try_from(frame_height).ok())
        .is_some_and(|(w, h)| w > 0 && h > 0 && width % w == 0 && height % h == 0)
}

/// The finished sprite of one origin: a file whose frames fit its size, with the resource it was
/// read from, or the pixels a cut, a palette or a mod made. None where it can't be finished.
pub fn finish_origin(
    source: &crate::resource_source::ResourceSource,
    origin: &SpriteOrigin,
) -> Option<Material> {
    let decode = |texture: &str, directory: &str| {
        crate::hud::decode_resource(source, &texture_resource(texture, directory)).ok()
    };
    match origin {
        SpriteOrigin::File(texture) => {
            let resource = texture_resource(texture, "textures");
            let image = decode(texture, "textures")?;
            let mcmeta = source.read_to_string(&format!("{resource}.mcmeta")).ok();
            let animation = crate::hud::animation_of(mcmeta.as_deref());
            frames_fit((image.0, image.1), animation.as_ref()).then_some(Material {
                image,
                resource: Some(resource),
                animation,
            })
        }
        generated => Some(Material {
            image: generate(generated, &decode)?,
            resource: None,
            animation: None,
        }),
    }
}

impl Materials {
    /// The materials of this load of `source` ([`listing`]).
    pub fn new(source: &crate::resource_source::ResourceSource) -> Self {
        Self::over(listing(source), source)
    }
    /// The materials of `listing`, a listing of `source`.
    pub fn over(listing: Arc<Listing>, source: &crate::resource_source::ResourceSource) -> Self {
        Self {
            listing,
            defined: ["blocks", "items"]
                .iter()
                .any(|atlas| source.contains(&format!("assets/minecraft/atlases/{atlas}.json"))),
        }
    }
    /// The listing the materials come from: every atlas.
    pub fn listing(&self) -> &Listing {
        &self.listing
    }
    fn items(&self) -> &BTreeMap<String, SpriteOrigin> {
        self.listing.sprites("minecraft:items")
    }
    fn blocks(&self) -> &BTreeMap<String, SpriteOrigin> {
        self.listing.sprites("minecraft:blocks")
    }
    /// Whether neither atlas has a definition: a bare resource tree, whose sprites are its files.
    pub fn is_bare(&self) -> bool {
        !self.defined
    }
    /// Whether the items atlas lists `id`: its model material is that atlas's, which may not be
    /// the blocks atlas's sprite of the name.
    pub fn in_items(&self, id: &str) -> bool {
        self.items().contains_key(id)
    }
    /// Every id either atlas lists.
    pub fn ids(&self) -> BTreeSet<&str> {
        self.items()
            .keys()
            .chain(self.blocks().keys())
            .map(String::as_str)
            .collect()
    }
    /// The ids a mod's sprite source made in the blocks atlas, by pixels or by naming a texture:
    /// sprites to stitch even when no model names them, for the mod that draws them.
    pub fn mod_sprites(&self) -> Vec<String> {
        self.listing.by_mods("minecraft:blocks").cloned().collect()
    }
    /// `finish` of the first atlas's origin for `id` that gives a sprite, items first: an
    /// origin that fails (a cut outside its sheet, a missing palette) leaves the blocks atlas's
    /// sprite of that id to be found. None where no atlas lists `id`, or none finishes.
    pub fn resolve<T>(
        &self,
        id: &str,
        finish: impl FnMut(&SpriteOrigin) -> Option<T>,
    ) -> Option<T> {
        [self.items(), self.blocks()]
            .into_iter()
            .filter_map(|atlas| atlas.get(id))
            .find_map(finish)
    }
    /// The same, for what names the blocks atlas itself (the fluid renderer).
    pub fn resolve_in_blocks<T>(
        &self,
        id: &str,
        finish: impl FnMut(&SpriteOrigin) -> Option<T>,
    ) -> Option<T> {
        self.blocks().get(id).and_then(finish)
    }
    /// The finished model material `id`: a bare tree reads it as a file.
    pub fn finish(
        &self,
        source: &crate::resource_source::ResourceSource,
        id: &str,
    ) -> Option<Material> {
        if self.is_bare() || id.starts_with("sunrise_cit:") {
            return finish_origin(source, &SpriteOrigin::File(id.to_owned()));
        }
        self.resolve(id, |origin| finish_origin(source, origin))
    }
}

/// Source types a test registers for itself: no process-wide registry is touched.
#[cfg(test)]
pub(crate) mod test_mods {
    use super::*;
    use std::sync::Mutex;

    type Answer = dyn Fn(&Identifier, &str, &Tag, &Arc<dyn ResourceReader>) -> Vec<SpriteSourceOutput>
        + Send
        + Sync;
    pub struct Mods {
        types: Vec<String>,
        answer: Box<Answer>,
        /// Each question the sources were asked, as `(atlas, type)`.
        pub asked: Mutex<Vec<(String, String)>>,
    }
    impl Mods {
        /// No registered type.
        pub fn none() -> Self {
            Self::new(&[], |_, _, _, _| Vec::new())
        }
        /// The types `types`, answered by `answer`.
        pub fn new(
            types: &[&str],
            answer: impl Fn(
                &Identifier,
                &str,
                &Tag,
                &Arc<dyn ResourceReader>,
            ) -> Vec<SpriteSourceOutput>
            + Send
            + Sync
            + 'static,
        ) -> Self {
            Self {
                types: types.iter().map(|t| (*t).to_owned()).collect(),
                answer: Box::new(answer),
                asked: Mutex::default(),
            }
        }
    }
    impl ModSources for Mods {
        fn is_registered(&self, kind: &str) -> bool {
            self.types.iter().any(|t| t == kind)
        }
        fn ask(
            &self,
            atlas: &Identifier,
            kind: &str,
            config: &Tag,
            resources: &Arc<dyn ResourceReader>,
        ) -> Vec<SpriteSourceOutput> {
            self.asked
                .lock()
                .unwrap()
                .push((atlas.to_string(), kind.to_owned()));
            (self.answer)(atlas, kind, config, resources)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_mods::Mods;
    use super::*;
    fn no_mods(_: &str, _: &Tag) -> Vec<SpriteSourceOutput> {
        Vec::new()
    }
    #[test]
    fn discovers_archive_textures_and_nested_atlas_definitions_in_place() {
        use std::io::{Cursor, Write};
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in [
            ("assets/demo/textures/item/x.png", b"png".as_slice()),
            ("assets/zzz/textures/item/z.png", b"png"),
            ("assets/demo/atlases/custom/nested.json", br#"{"sources":[{"type":"single","resource":"demo:item/x","sprite":"demo:renamed"}]}"#),
            ("assets/demo/atlases/ignored.txt", b"ignored"),
        ] {
            writer.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            writer.write_all(bytes).unwrap();
        }
        let source = crate::resource_source::ResourceSource::opened_archive(
            crate::zip::Zip::from_bytes(writer.finish().unwrap().into_inner()).unwrap(),
        );
        let textures = list_textures(&source);
        assert_eq!(textures, ["demo:item/x", "zzz:item/z"]);
        let atlas = Identifier::parse("demo:atlas").unwrap();
        let definition = Identifier::parse("demo:custom/nested").unwrap();
        let atlases = registered(&source, &Mods::none(), &[(&atlas, &definition)]);
        assert!(
            matches!(atlases[&atlas].sprites.get("demo:renamed"), Some(SpriteOrigin::File(texture)) if texture == "demo:item/x")
        );
    }

    fn textures() -> Vec<String> {
        let mut t: Vec<String> = [
            "minecraft:item/apple",
            "minecraft:item/stick",
            "minecraft:trims/items/boots_trim",
            "minecraft:block/stone",
            "minecraft:entity/sheet",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        t.sort();
        t
    }
    #[test]
    fn sources_run_in_order() {
        let sources = decode(
            r#"{"sources":[
                {"type":"directory","source":"item","prefix":"item/"},
                {"type":"minecraft:single","resource":"block/stone","sprite":"item/stone"},
                {"type":"filter","pattern":{"path":"stick"}},
                {"type":"unstitch","resource":"entity/sheet","divisor_x":2,"regions":[{"sprite":"a","x":1,"y":0,"width":1,"height":1}]},
                {"type":"paletted_permutations","textures":["trims/items/boots_trim"],"palette_key":"trims/color_palettes/trim_palette","permutations":{"quartz":"trims/color_palettes/quartz"}}
            ]}"#,
            &Mods::none(),
        )
        .unwrap();
        let listed = list(&sources, &textures(), &mut no_mods).sprites;
        let ids: Vec<&str> = listed.keys().map(String::as_str).collect();
        assert_eq!(
            ids,
            [
                "minecraft:a",
                "minecraft:item/apple",
                "minecraft:item/stone",
                "minecraft:trims/items/boots_trim_quartz"
            ]
        );
        assert_eq!(
            listed["minecraft:item/stone"],
            SpriteOrigin::File("minecraft:block/stone".into())
        );
    }
    #[test]
    fn malformed_sources_fail_the_file() {
        assert_eq!(
            decode(
                r#"{"sources":[{"type":"directory","source":"item"}]}"#,
                &Mods::none()
            )
            .unwrap_err()
            .to_string(),
            "sources[0]: malformed source"
        );
        assert_eq!(
            decode(r#"{"sources":[{"type":"directory","source":"a","prefix":""},{"type":"unstitch","resource":"x","regions":[]}]}"#, &Mods::none())
                .unwrap_err()
                .to_string(),
            "sources[1]: malformed source"
        );
        assert_eq!(
            decode(r#"{"sources":[{"type":"sunrise"}]}"#, &Mods::none())
                .unwrap_err()
                .to_string(),
            "sources[0]: unknown sprite source type minecraft:sunrise"
        );
    }
    #[test]
    fn unstitch_cuts_by_divisor() {
        let pixels: Vec<u8> = (0..16u8).flat_map(|i| [i, i, i, 255]).collect();
        let load = |_: &str, _: &str| Some((4u32, 4u32, pixels.clone()));
        let origin = SpriteOrigin::Region {
            texture: "t".into(),
            region: Region {
                sprite: "s".into(),
                x: 1.0,
                y: 1.0,
                width: 1.0,
                height: 1.0,
            },
            divisor_x: 2.0,
            divisor_y: 2.0,
        };
        let (w, h, out) = generate(&origin, &load).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(out[0], 10);
        assert_eq!(out[4], 11);
        assert_eq!(out[8], 14);
    }

    #[test]
    fn paletted_permutations_load_palettes_from_textures_palettes() {
        let origin = SpriteOrigin::Paletted {
            texture: "minecraft:trims/items/boots_trim".into(),
            palette_key: "minecraft:trim_base".into(),
            palette: "minecraft:trim/quartz".into(),
        };
        let load = |id: &str, dir: &str| match (id, dir) {
            ("minecraft:trims/items/boots_trim", "textures") => Some((1, 1, vec![10, 20, 30, 255])),
            ("minecraft:trim_base", "textures/palettes") => Some((1, 1, vec![10, 20, 30, 255])),
            ("minecraft:trim/quartz", "textures/palettes") => {
                Some((1, 1, vec![220, 210, 200, 255]))
            }
            _ => None,
        };
        assert_eq!(
            generate(&origin, &load),
            Some((1, 1, vec![220, 210, 200, 255]))
        );
    }

    /// An unstitch region over a 4x4 texture with divisors 1: the region's numbers are pixels.
    fn region_of(x: f64, y: f64, width: f64, height: f64) -> Option<Image> {
        let pixels: Vec<u8> = (0..16u8).flat_map(|i| [i, i, i, 255]).collect();
        let load = |_: &str, _: &str| Some((4u32, 4u32, pixels.clone()));
        let origin = SpriteOrigin::Region {
            texture: "t".into(),
            region: Region {
                sprite: "s".into(),
                x,
                y,
                width,
                height,
            },
            divisor_x: 4.0,
            divisor_y: 4.0,
        };
        generate(&origin, &load)
    }

    /// A pack's declared region can't overflow the size or ask for a huge allocation: past the
    /// stated side it is skipped, whatever the numbers, and the copy's offsets can't overflow.
    #[test]
    fn a_huge_declared_region_is_refused_not_allocated() {
        for (width, height) in [
            (f64::MAX, f64::MAX),
            (i64::MAX as f64, 2.0),
            (2.0, i64::MAX as f64),
            (3_000_000_000.0, 3_000_000_000.0),
            ((MAX_SPRITE_SIDE + 1) as f64, 1.0),
            (f64::INFINITY, 1.0),
        ] {
            assert_eq!(region_of(0.0, 0.0, width, height), None, "{width}x{height}");
        }
        // An offset near the integer limit is outside the sheet.
        assert_eq!(
            region_of(i64::MAX as f64, -(i64::MAX as f64), 2.0, 2.0),
            None
        );
    }

    /// A cut that is not wholly inside the sheet is absent, beside one that is.
    #[test]
    fn a_cut_outside_the_sheet_is_absent() {
        // Rows of the 4x4 sheet hold 0..3, 4..7, ... in every channel.
        let (w, h, out) = region_of(1.0, 2.0, 2.0, 2.0).unwrap();
        assert_eq!((w, h), (2, 2));
        let reds: Vec<u8> = out.chunks(4).map(|p| p[0]).collect();
        assert_eq!(reds, [9, 10, 13, 14]);
        for (x, y, width, height) in [
            (3.0, 0.0, 2.0, 1.0),
            (0.0, 3.0, 1.0, 2.0),
            (-1.0, 0.0, 2.0, 2.0),
        ] {
            assert_eq!(
                region_of(x, y, width, height),
                None,
                "{x},{y} {width}x{height}"
            );
        }
    }

    #[test]
    fn listed_finds_textures_of_namespaces_that_share_a_prefix() {
        let source = crate::resource_source::ResourceSource::indexed(
            [
                ("assets/a/textures/block/sheet.png", b"png".as_slice()),
                ("assets/a0/textures/block/other.png", b"png"),
                (
                    "assets/minecraft/atlases/blocks.json",
                    br#"{"sources":[{"type":"single","resource":"a:block/sheet","sprite":"x:one"},
                        {"type":"single","resource":"a0:block/other","sprite":"x:two"}]}"#,
                ),
            ]
            .map(|(name, bytes)| {
                (
                    name.to_owned(),
                    sunrise_assets::resource_source::Resource::bytes(bytes.to_vec()),
                )
            })
            .into(),
        );
        let listing = Listing::build(&source, &Mods::none());
        assert_eq!(
            listing
                .sprites("minecraft:blocks")
                .keys()
                .collect::<Vec<_>>(),
            ["x:one", "x:two"]
        );
    }

    fn pixels(sprite: &str, rgba: [u8; 4]) -> SpriteSourceOutput {
        SpriteSourceOutput::Pixels {
            sprite: Identifier::parse(sprite).unwrap(),
            width: 1,
            height: 1,
            rgba: rgba.to_vec(),
        }
    }
    fn definitions(atlases: &[&str]) -> crate::resource_source::ResourceSource {
        use sunrise_assets::resource_source::Resource;
        let own = r#"{"sources":[{"type":"srcgame:own"}]}"#;
        crate::resource_source::ResourceSource::indexed(
            atlases
                .iter()
                .map(|atlas| {
                    (
                        format!("assets/minecraft/atlases/{atlas}.json"),
                        Resource::bytes(own.as_bytes().to_vec()),
                    )
                })
                .collect(),
        )
    }

    /// A source type is asked once for each atlas whose definition lists it, however many
    /// readers take the load's listing (and a source extended from it), and each atlas keeps its
    /// own sprites: a blocks and an items sprite of one id are two sprites.
    #[test]
    fn a_load_asks_each_entry_once_and_every_reader_takes_the_same_answer() {
        let mods = Mods::new(&["srcgame:own"], |atlas, kind, _, _| {
            assert_eq!(kind, "srcgame:own");
            vec![
                pixels("srcgame:same", [atlas.path().len() as u8, 0, 0, 255]),
                pixels(&format!("srcgame:only_{}", atlas.path()), [9, 9, 9, 255]),
            ]
        });
        let source = definitions(&["blocks", "items", "gui"]);
        let first = shared(&source, &mods);
        let again = shared(&source.clone(), &mods);
        let extended = shared(&source.extended(Default::default()), &mods);
        assert!(Arc::ptr_eq(&first, &again) && Arc::ptr_eq(&first, &extended));
        let mut asked = mods.asked.lock().unwrap().clone();
        asked.sort();
        assert_eq!(
            asked,
            [
                ("minecraft:blocks".to_owned(), "srcgame:own".to_owned()),
                ("minecraft:gui".to_owned(), "srcgame:own".to_owned()),
                ("minecraft:items".to_owned(), "srcgame:own".to_owned()),
            ]
        );
        let materials = Materials::over(first, &source);
        let red = |origin: &SpriteOrigin| match origin {
            SpriteOrigin::Pixels((_, _, rgba)) => Some(rgba[0]),
            _ => None,
        };
        // "blocks" and "items" differ in length: 6 and 5.
        assert_eq!(
            materials.resolve("srcgame:same", red),
            Some(5),
            "items first"
        );
        assert_eq!(materials.resolve_in_blocks("srcgame:same", red), Some(6));
        assert_eq!(
            materials.resolve_in_blocks("srcgame:only_items", red),
            None,
            "an items sprite is not a blocks sprite"
        );
        assert_eq!(
            materials.mod_sprites(),
            ["srcgame:only_blocks", "srcgame:same"],
            "the blocks atlas's, though no model names them"
        );
    }

    /// A source reads the load's resources while it is asked: the winning pack's own copy (not
    /// the merged document of a file every pack adds to), every layer lowest first, and the paths
    /// under a prefix. The reader lasts one callback: one a source keeps reads nothing in the
    /// next entry's callback or after, and lets go of the load.
    #[test]
    fn a_source_reads_the_loads_resources_for_the_length_of_one_call() {
        use sunrise_assets::resource_source::Resource;
        let bytes = |text: &str| Resource::bytes(text.as_bytes().to_vec());
        let name = "assets/minecraft/textures/block/probe.png";
        let merged = "assets/minecraft/lang/en_us.json";
        let source = crate::resource_source::ResourceSource::indexed_with_layers(
            [
                (name.to_owned(), bytes("top")),
                (merged.to_owned(), bytes("merged")),
                (
                    "assets/minecraft/atlases/blocks.json".to_owned(),
                    bytes(r#"{"sources":[{"type":"srcgame:own"},{"type":"srcgame:own"}]}"#),
                ),
            ]
            .into(),
            [
                (
                    name.to_owned(),
                    vec![bytes("vanilla"), bytes("mod"), bytes("top")],
                ),
                (merged.to_owned(), vec![bytes("one"), bytes("two")]),
            ]
            .into(),
            None,
        );
        let seen = Arc::new(Mutex::new(Vec::new()));
        let kept: Arc<Mutex<Option<Arc<dyn ResourceReader>>>> = Arc::default();
        let (record, keep) = (seen.clone(), kept.clone());
        let mods = Mods::new(&["srcgame:own"], move |_, _, _, resources| {
            let mut kept = keep.lock().unwrap();
            // The second entry's callback: the first entry's reader is already closed.
            let earlier = kept.as_ref().map(|reader| {
                (
                    reader.read(name),
                    reader.read_stack(name),
                    reader.list("assets/"),
                    Arc::strong_count(reader),
                )
            });
            record.lock().unwrap().push((
                resources.read(name),
                resources.read_stack(name),
                resources.list("assets/minecraft/textures/"),
                resources.read(merged),
                earlier,
            ));
            *kept = Some(resources.clone());
            Vec::new()
        });
        Listing::build(&source, &mods);
        let seen = seen.lock().unwrap();
        let first = (
            Some(b"top".to_vec()),
            vec![b"vanilla".to_vec(), b"mod".to_vec(), b"top".to_vec()],
            vec![name.to_owned()],
            Some(b"two".to_vec()),
        );
        assert_eq!(seen.len(), 2, "each entry is asked once");
        assert_eq!(
            seen[0],
            (
                first.0.clone(),
                first.1.clone(),
                first.2.clone(),
                first.3.clone(),
                None
            )
        );
        assert_eq!(
            seen[1],
            (
                first.0,
                first.1,
                first.2,
                first.3,
                Some((None, Vec::new(), Vec::new(), 1))
            ),
            "the first entry's reader reads nothing in the second entry's call"
        );
        let kept = kept.lock().unwrap().take().unwrap();
        assert!(kept.read(name).is_none() && kept.read_stack(name).is_empty());
    }

    /// An atlas a mod registered is listed under its own id from its definition. Two atlases
    /// sharing one definition are each asked, but the definition is not itself an engine atlas.
    #[test]
    fn registered_atlases_are_asked_by_their_own_id() {
        use sunrise_assets::resource_source::Resource;
        let source = crate::resource_source::ResourceSource::indexed(
            ["minecraft:pack_icons", "demo:unregistered"]
                .map(|definition| {
                    let (namespace, path) = definition.split_once(':').unwrap();
                    (
                        format!("assets/{namespace}/atlases/{path}.json"),
                        Resource::bytes(br#"{"sources":[{"type":"srcreg:own"}]}"#.to_vec()),
                    )
                })
                .into(),
        );
        let mods = Mods::new(&["srcreg:own"], |atlas, _, _, _| {
            vec![pixels(
                &format!("demo:from_{}", atlas.path()),
                [atlas.path().len() as u8, 0, 0, 255],
            )]
        });
        let id = |text: &str| Identifier::parse(text).unwrap();
        let (first, second, definition) = (
            id("demo:first"),
            id("demo:second"),
            id("minecraft:pack_icons"),
        );
        let listed = registered(
            &source,
            &mods,
            &[(&first, &definition), (&second, &definition)],
        );
        assert_eq!(
            listed[&first].sprites.keys().collect::<Vec<_>>(),
            ["demo:from_first"]
        );
        assert_eq!(
            listed[&second].sprites.keys().collect::<Vec<_>>(),
            ["demo:from_second"]
        );
        assert!(Listing::build(&source, &mods).atlases().next().is_none());
        let mut asked = mods.asked.lock().unwrap().clone();
        asked.sort();
        assert_eq!(
            asked,
            [
                ("demo:first".to_owned(), "srcreg:own".to_owned()),
                ("demo:second".to_owned(), "srcreg:own".to_owned())
            ]
        );
    }

    /// What identifies a listing follows what the sources answered and what they removed: a
    /// cache of results derived from it never serves an answer the code no longer gives.
    #[test]
    fn the_listing_fingerprint_follows_the_mods_answers() {
        let source = definitions(&["blocks"]);
        let answer = |outputs: Vec<SpriteSourceOutput>| {
            let outputs = Arc::new(outputs);
            Mods::new(&["srcgame:own"], move |_, _, _, _| outputs.to_vec())
        };
        let fingerprint = |mods: &Mods| Listing::build(&source, mods).fingerprint();
        let red = fingerprint(&answer(vec![pixels("srcgame:a", [1, 0, 0, 255])]));
        let green = fingerprint(&answer(vec![pixels("srcgame:a", [0, 1, 0, 255])]));
        let none = fingerprint(&answer(Vec::new()));
        assert!(red != green && red != none && green != none);
        assert_eq!(
            red,
            fingerprint(&answer(vec![pixels("srcgame:a", [1, 0, 0, 255])]))
        );
    }

    /// A later source of the same id takes a sprite back from a mod's, and a filter drops it.
    #[test]
    fn a_sprite_a_built_in_source_lists_later_is_no_longer_the_mods() {
        let source = crate::resource_source::ResourceSource::indexed(
            [
                (
                    "assets/minecraft/atlases/blocks.json".to_owned(),
                    sunrise_assets::resource_source::Resource::bytes(
                        br#"{"sources":[{"type":"srcgame:own"},
                            {"type":"single","resource":"srcgame:block/real","sprite":"srcgame:a"},
                            {"type":"filter","pattern":{"path":"gone"}}]}"#
                            .to_vec(),
                    ),
                ),
                (
                    "assets/srcgame/textures/block/real.png".to_owned(),
                    sunrise_assets::resource_source::Resource::bytes(b"png".to_vec()),
                ),
            ]
            .into(),
        );
        let mods = Mods::new(&["srcgame:own"], |_, _, _, _| {
            vec![
                pixels("srcgame:a", [1, 1, 1, 255]),
                pixels("srcgame:b", [2, 2, 2, 255]),
                pixels("srcgame:gone", [3, 3, 3, 255]),
            ]
        });
        let listing = Listing::build(&source, &mods);
        assert_eq!(
            listing
                .sprites("minecraft:blocks")
                .keys()
                .collect::<Vec<_>>(),
            ["srcgame:a", "srcgame:b"]
        );
        assert_eq!(
            listing.by_mods("minecraft:blocks").collect::<Vec<_>>(),
            ["srcgame:b"]
        );
    }

    /// A registered sprite source type checked ahead of vanilla's built-ins overrides that
    /// built-in during atlas decode and listing (matching NeoForge's `LateBoundIdMapper`).
    #[test]
    fn a_registered_source_overrides_a_builtin_type_when_decoding() {
        let json = r#"{"sources":[{"type":"single","resource":"minecraft:item/apple"}]}"#;
        assert!(matches!(
            decode(json, &Mods::none()).unwrap().as_slice(),
            [SpriteSource::Single { resource, .. }] if resource == "minecraft:item/apple"
        ));
        let overriding = Mods::new(&["minecraft:single"], |_, _, _, _| {
            vec![pixels("override:custom", [42, 43, 44, 255])]
        });
        assert!(matches!(
            decode(json, &overriding).unwrap().as_slice(),
            [SpriteSource::Registered { kind, .. }] if kind == "minecraft:single"
        ));
        let source = crate::resource_source::ResourceSource::indexed(
            [
                (
                    "assets/minecraft/atlases/blocks.json".to_owned(),
                    sunrise_assets::resource_source::Resource::bytes(json.as_bytes().to_vec()),
                ),
                (
                    "assets/minecraft/textures/item/apple.png".to_owned(),
                    sunrise_assets::resource_source::Resource::bytes(b"png".to_vec()),
                ),
            ]
            .into(),
        );
        let listing = Listing::build(&source, &overriding);
        assert_eq!(
            listing
                .sprites("minecraft:blocks")
                .keys()
                .collect::<Vec<_>>(),
            ["override:custom"]
        );
    }
}
