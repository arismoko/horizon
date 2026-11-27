//! A content mod's `assets/` and `data/` (API 0.2, docs/modpacks.md): the
//! files it adds to the game's resource tree, and the blocks, items, mob
//! effects, entity types and menu types it declares in
//! `data/<id>/sunrise/{block,item,mob_effect,entity_type,fluid,fluid_interaction,menu}/`.
//!
//! Content is data, not code: nothing here executes. What a mod
//! may add is fenced instead. Its own namespaces (its id and the ones its
//! manifest lists), plus additions to vanilla tags and to the convention
//! tags (`c`, which NeoForge and Fabric mods share). Paths are plain, files
//! and totals are capped, vanilla ids cannot be redefined, and no two mods
//! share a namespace ([`clash`]).
use crate::manifest::Manifest;
use mod_vocab::{__private::DeclarationKind, DeclarationError};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
pub use sunrise_core::arm_pose::ArmPose;

/// Content files per mod, bytes per file and bytes per mod.
pub const MAX_FILES: usize = 65_536;
pub const MAX_FILE: u64 = 16 << 20;
pub const MAX_TOTAL: u64 = 512 << 20;

/// Checks a content path: `assets/` or `data/`, then `/`-separated parts of
/// `[a-z0-9_.-]`, none of them `.` or `..` (vanilla's resource-location
/// characters, so the path means the same on every OS).
pub fn check_path(name: &str) -> Result<(), String> {
    let plain_char = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || "_.-".contains(c);
    let mut parts = name.split('/');
    let top = parts.next();
    // assets/<namespace>/<file> at least.
    let named_ok = matches!(top, Some("assets" | "data"))
        && parts.clone().count() >= 2
        && parts
            .clone()
            .all(|part| !part.is_empty() && part != "." && part != "..");
    if named_ok && parts.clone().all(|part| part.chars().all(plain_char)) {
        return Ok(());
    }
    let mut error = format!(
        "{name:?} is not a plain assets/ or data/ path: after assets/ or data/, every part is made of a-z, 0-9, _, . and - (vanilla's resource-location characters, so the path means the same on every OS), and none is . or .."
    );
    if named_ok {
        let fixed: Vec<String> = parts
            .map(|part| {
                part.to_ascii_lowercase()
                    .chars()
                    .map(|c| if plain_char(c) { c } else { '_' })
                    .collect()
            })
            .collect();
        error.push_str(&format!(
            "; rename it {:?}",
            format!("{}/{}", top.unwrap_or(""), fixed.join("/"))
        ));
    }
    Err(error)
}

/// The single resource location NeoForge's `RecipePriorityManager` reads in pack order.
pub const RECIPE_PRIORITIES_PATH: &str = "data/neoforge/recipe_priorities.json";

/// The shared resource namespace fence for folders and native archives.
pub(crate) fn check_namespace(name: &str, manifest: &Manifest) -> Result<(), String> {
    let mut parts = name.splitn(3, '/');
    let (top, namespace, rest) = (
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
    );
    let id = manifest.id.as_str();
    let own = namespace == id;
    let owned = own || manifest.namespaces.iter().any(|n| n == namespace);
    // Vanilla's own files stay vanilla's; a mod may only add to its
    // tags and atlases, and to the convention tags mods share.
    let shared = matches!(namespace, "minecraft" | "c" | "neoforge");
    let atlas = top == "assets" && namespace == "minecraft" && rest.starts_with("atlases/");
    let mut extension = false;
    if top == "data" && namespace == "sunrise" && rest.starts_with("tags/") {
        let tag = rest
            .strip_prefix("tags/")
            .and_then(|s| s.strip_suffix(".json"));
        if tag.is_some_and(|t| {
            t.split_once('/')
                .is_some_and(|(_, path)| path.starts_with("class/"))
        }) {
            return Err(format!(
                "{name}: sunrise:class/* tags are generated; attach a part through block_part_tag instead"
            ));
        }
        extension = tag
            .and_then(|t| t.split_once('/'))
            .is_some_and(|(registry, path)| {
                crate::extension_tags::ALL.iter().any(|tag| {
                    tag.registry == registry && tag.id.strip_prefix("sunrise:") == Some(path)
                })
            });
        if !extension {
            return Err(format!(
                "{name}: not a declared Sunrise extension tag; see the SDK extension_tags list and the book's extension tags reference (mod professions use secondary_blocks)"
            ));
        }
    }
    let allowed = owned
        || (top == "data" && shared && rest.starts_with("tags/"))
        || atlas
        || extension
        || name == RECIPE_PRIORITIES_PATH;
    if !allowed {
        return Err(format!(
            "{name}: a mod adds files in its own namespaces ({top}/{id}/ and those listed in namespaces), to vanilla and convention (c, neoforge) tags, to vanilla's atlases, or to declared shared resource paths"
        ));
    }

    Ok(())
}

/// A vanilla identifier, `minecraft:path`.
/// `Identifier.isValidNamespace` and `isValidPath` on an id with both parts
/// given: `[a-z0-9_.-]`, and `/` in the path too.
pub fn namespaced_id(id: &str) -> bool {
    let ok = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b);
    id.split_once(':').is_some_and(|(namespace, path)| {
        !namespace.is_empty()
            && !path.is_empty()
            && namespace.bytes().all(ok)
            && path.bytes().all(|b| ok(b) || b == b'/')
    })
}

fn vanilla_id(id: &str) -> bool {
    id.strip_prefix("minecraft:").is_some_and(|path| {
        !path.is_empty()
            && path
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_./-".contains(&b))
    })
}

/// What a declared block changes from its template; unset keeps the
/// template's value.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BlockProperties {
    /// Destroy time in seconds of hand mining (-1: unbreakable).
    pub hardness: Option<f32>,
    pub explosion_resistance: Option<f32>,
    /// Light emitted, 0 to 15.
    pub light: Option<u8>,
    /// `true`: the block's parts answer `light_emission`, the light a cell emits by where it is
    /// (`IBlockExtension.hasDynamicLightEmission`); `light` is a cell's without an answer.
    pub dynamic_light: Option<bool>,
    pub requires_tool: Option<bool>,
    pub friction: Option<f32>,
    /// A `MapColor` id, 0 to 61.
    pub map_color: Option<u8>,
    /// A vanilla block whose sounds this block uses.
    pub sounds: Option<String>,
    /// Which states tick randomly, whatever the template does (`isRandomlyTicking`).
    pub random_ticks: Option<RandomTicks>,
    /// How the block is drawn, whatever the template does (`BlockBehaviour.getRenderShape`).
    pub render_shape: Option<RenderShape>,
    /// Vanilla's piston push reaction, or the template's when unset.
    #[serde(default, deserialize_with = "deserialize_push_reaction")]
    pub push_reaction: Option<PushReaction>,
    /// `false`: the block occludes nothing, so its neighbours' faces are drawn and light passes
    /// it (`BlockBehaviour.Properties.noOcclusion`); `true`: occludes whatever the template
    /// does, using the declared outline or the template's published occlusion shape. Unset
    /// inherits the template.
    pub occludes: Option<bool>,
    /// `false`: the breaking crack (`destroy_stage_0..9`) is not drawn on the block, as for a
    /// block broken by use; unset or `true`, it is drawn. Only the overlay: the break and the
    /// destroy-progress packets are vanilla's.
    pub destroy_overlay: Option<bool>,
    /// Per-column model offset and translation of declared outline, collision and interaction shapes.
    /// Block-entity published shapes remain as published. Unset inherits the template.
    #[serde(default, deserialize_with = "deserialize_offset")]
    pub offset: Option<OffsetType>,
    /// Maximum horizontal displacement, 0 to 0.25 (default 0.25 for an explicit offset mode).
    #[serde(default, deserialize_with = "deserialize_horizontal_offset")]
    pub max_horizontal_offset: Option<f32>,
    /// Maximum downward displacement, 0 to 0.2 (default 0.2 for XYZ; XZ has no vertical offset).
    #[serde(default, deserialize_with = "deserialize_vertical_offset")]
    pub max_vertical_offset: Option<f32>,
    /// Vanilla's note-block instrument constant; unset inherits the template.
    #[serde(default, deserialize_with = "deserialize_instrument")]
    pub instrument: Option<mod_vocab::NoteInstrument>,
    /// Lava ignition flag. The per-position `ignited_by_lava` hook still answers first.
    #[serde(default, deserialize_with = "deserialize_ignited_by_lava")]
    pub ignited_by_lava: Option<bool>,
    /// Whether placement and features may replace this block; the held-own-item rule still applies.
    #[serde(default, deserialize_with = "deserialize_replaceable")]
    pub replaceable: Option<bool>,
    /// Data rules replacing vanilla `BlockBehaviour.skipRendering` for this block.
    /// `None` inherits the template's render rule; `Some(vec![])` disables it.
    #[serde(default, deserialize_with = "face_rules::deserialize_skip_rendering")]
    pub skip_rendering: Option<Vec<mod_vocab::FaceRule>>,
    /// Data rules implementing NeoForge `IBlockExtension.hidesNeighborFace`.
    #[serde(
        default,
        deserialize_with = "face_rules::deserialize_hides_neighbor_face"
    )]
    pub hides_neighbor_face: Vec<mod_vocab::FaceRule>,
    /// NeoForge `IBlockExtension.supportsExternalFaceHiding` (default `true`).
    #[serde(
        default,
        deserialize_with = "face_rules::deserialize_supports_external_face_hiding"
    )]
    pub supports_external_face_hiding: Option<bool>,
    /// Which mobs may spawn on the block (`BlockBehaviour.Properties.isValidSpawn`); unset keeps
    /// the template block's rule. Set on every state of the block, from the block alone: a
    /// `dynamic_light` block's cell light and an outline declared over the template's do not
    /// reach it (see [`SpawnRule::Default`]).
    pub spawn_rule: Option<SpawnRule>,
    /// Opt into `VegetationBlock` survival independently of the template.
    #[serde(default)]
    pub vegetation: bool,
}

/// A block's mob spawn rule (`BlockBehaviour.Properties.isValidSpawn`), the answer of
/// `BlockState.isValidSpawn` for every entity type; a polar bear follows the rule like any other mob.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpawnRule {
    /// Vanilla's default: the top face is sturdy and the block's light emission is below 14. The
    /// top face is the template's, and the emission is the declared `light` (or `state_light`),
    /// else the template's: a `dynamic_light` block is judged by that static light, since the
    /// spawn tables hold one answer per state.
    Default,
    /// Every mob spawns on the block (`Blocks::always`).
    Always,
    /// No mob spawns on the block (`Blocks::never`).
    Never,
    /// Only ocelots and parrots spawn on the block (`Blocks::ocelotOrParrot`).
    OcelotOrParrot,
}

/// Vanilla's per-column offset mode (`BlockBehaviour.OffsetType`).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum OffsetType {
    None,
    Xz,
    Xyz,
}

fn offset_float<'de, D: serde::Deserializer<'de>>(
    d: D,
    key: &str,
) -> Result<Option<f32>, D::Error> {
    Option::<f32>::deserialize(d).map_err(|e| serde::de::Error::custom(format!("{key}: {e}")))
}
fn deserialize_horizontal_offset<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<f32>, D::Error> {
    offset_float(d, "max_horizontal_offset")
}
fn deserialize_vertical_offset<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<f32>, D::Error> {
    offset_float(d, "max_vertical_offset")
}
fn deserialize_offset<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<OffsetType>, D::Error> {
    Option::<OffsetType>::deserialize(d)
        .map_err(|e| serde::de::Error::custom(format!("offset: {e}")))
}

fn deserialize_instrument<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<mod_vocab::NoteInstrument>, D::Error> {
    Option::<String>::deserialize(deserializer)
        .map_err(|e| serde::de::Error::custom(format!("instrument: {e}")))?
        .map(|name| {
            mod_vocab::NoteInstrument::ALL
                .into_iter()
                .find(|instrument| instrument.name().to_ascii_uppercase() == name)
                .ok_or_else(|| {
                    serde::de::Error::custom(format!(
                        "instrument: unknown NoteBlockInstrument constant {name}"
                    ))
                })
        })
        .transpose()
}

fn property_bool<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
    key: &str,
) -> Result<Option<bool>, D::Error> {
    Option::<bool>::deserialize(deserializer)
        .map_err(|e| serde::de::Error::custom(format!("{key}: {e}")))
}
fn deserialize_ignited_by_lava<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<bool>, D::Error> {
    property_bool(d, "ignited_by_lava")
}
fn deserialize_replaceable<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<bool>, D::Error> {
    property_bool(d, "replaceable")
}

/// Vanilla's `PushReaction`, by its constant's name.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PushReaction {
    PushPull,
    Push,
    Popped,
    Immoveable,
    IgnoreEntity,
}

impl PushReaction {
    /// The constant's name, as the block tables write it.
    pub fn name(self) -> &'static str {
        match self {
            Self::PushPull => "PUSH_PULL",
            Self::Push => "PUSH",
            Self::Popped => "POPPED",
            Self::Immoveable => "IMMOVEABLE",
            Self::IgnoreEntity => "IGNORE_ENTITY",
        }
    }
}

fn deserialize_push_reaction<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<PushReaction>, D::Error> {
    Option::<PushReaction>::deserialize(deserializer)
        .map_err(|e| serde::de::Error::custom(format!("push_reaction: {e}")))
}

/// Vanilla's `RenderShape`, by its constant's name: `MODEL` puts the block model in the chunk
/// mesh, `ENTITYBLOCK_ANIMATED` leaves it out for a block whose parts are drawn every frame
/// (its block entity's drawing), and `INVISIBLE` draws nothing.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RenderShape {
    Model,
    EntityblockAnimated,
    Invisible,
}

impl RenderShape {
    /// The constant's name, as the block tables write it.
    pub fn name(self) -> &'static str {
        match self {
            RenderShape::Model => "MODEL",
            RenderShape::EntityblockAnimated => "ENTITYBLOCK_ANIMATED",
            RenderShape::Invisible => "INVISIBLE",
        }
    }
}

/// Which of a block's states get random ticks (`BlockBehaviour.isRandomlyTicking(state)`), so a
/// crop, sapling or soil on a template that doesn't tick still grows. The sections' random tick
/// counts follow it, and `native_random_tick` fires for the states it picks.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum RandomTicks {
    /// Every state (`true`), or none (`false`, for a template that ticks).
    All(bool),
    /// The states whose own state property `property` is one of `values`.
    States {
        property: String,
        values: Vec<String>,
    },
}

impl RandomTicks {
    /// Whether a state ticks, `combo` being its values of the block's own state properties.
    pub fn applies(&self, combo: &[(String, String)]) -> bool {
        match self {
            RandomTicks::All(all) => *all,
            RandomTicks::States { property, values } => combo
                .iter()
                .any(|(name, value)| name == property && values.contains(value)),
        }
    }
}

/// Values selected by one property or an ordered tuple of state properties.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StateValues<T> {
    #[serde(default)]
    pub property: String,
    #[serde(default)]
    pub properties: Vec<String>,
    #[serde(default)]
    pub values: BTreeMap<String, T>,
    pub default: Option<T>,
}

/// Block-local boxes selected by an ordered tuple of state properties.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockShapes(pub StateValues<Vec<[f64; 6]>>);

impl<'de> Deserialize<'de> for BlockShapes {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Shapes {
            #[serde(default)]
            properties: Vec<String>,
            #[serde(default)]
            values: BTreeMap<String, Vec<[f64; 6]>>,
            default: Option<Vec<[f64; 6]>>,
        }
        let shapes = Shapes::deserialize(deserializer)?;
        Ok(Self(StateValues {
            property: String::new(),
            properties: shapes.properties,
            values: shapes.values,
            default: shapes.default,
        }))
    }
}

impl std::ops::Deref for BlockShapes {
    type Target = StateValues<Vec<[f64; 6]>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for BlockShapes {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
/// Light emitted by a state, as an integer from 0 to 15.
pub type StateLight = StateValues<u8>;

impl<T> StateValues<T> {
    pub fn property_names(&self) -> Vec<&str> {
        if self.property.is_empty() {
            self.properties.iter().map(String::as_str).collect()
        } else {
            vec![self.property.as_str()]
        }
    }

    fn check_selection(&self, label: &str) -> Result<(), String> {
        let names = self.property_names();
        if (!self.property.is_empty() && !self.properties.is_empty())
            || names.iter().any(|name| !state_name(name))
            || names.iter().collect::<BTreeSet<_>>().len() != names.len()
            || (names.is_empty() && !self.values.is_empty())
            || (self.values.is_empty() && self.default.is_none())
        {
            return Err(format!(
                "{label} needs distinct properties and values, or a default"
            ));
        }
        for key in self.values.keys() {
            let values: Vec<_> = key.split(',').collect();
            if values.len() != names.len() || values.iter().any(|value| !state_name(value)) {
                return Err(format!(
                    "invalid {label} key {key:?}: one value per property"
                ));
            }
        }
        Ok(())
    }
}

impl BlockShapes {
    fn check(&self) -> Result<(), String> {
        self.check_selection("shapes")?;
        for boxes in self.values.values().chain(self.default.iter()) {
            // Vanilla's shapes have no box limit, so neither does a declared one.
            if boxes.iter().any(|bounds| {
                bounds.iter().any(|value| !value.is_finite())
                    || (0..3).any(|axis| bounds[axis] > bounds[axis + 3])
            }) {
                return Err("shape needs finite boxes with ordered bounds".into());
            }
        }
        Ok(())
    }
}

impl StateLight {
    fn check(&self) -> Result<(), String> {
        self.check_selection("state_light")?;
        if self
            .values
            .values()
            .chain(self.default.iter())
            .any(|value| *value > 15)
        {
            return Err("state_light values are integers 0 to 15".into());
        }
        Ok(())
    }
}

/// The querying entity class, captured by the real caller. Empty and position-only
/// queries have no entity and always use the ordinary shape tables.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum ShapeEntityKind {
    Item,
    Player,
    Living,
    Other,
}
impl ShapeEntityKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Item => "item",
            Self::Player => "player",
            Self::Living => "living",
            Self::Other => "other",
        }
    }
}

/// A vanilla block's context-dependent collision a block takes (`context_collision`), whatever
/// its template.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ContextCollision {
    /// Scaffolding's (`ScaffoldingBlock.getCollisionShape`): solid on top unless the entity is
    /// descending, open below, and the bottom row's floor. Its states need scaffolding's
    /// `distance` (0 to 7) and `bottom` properties, from the template or declared.
    Scaffolding,
}

/// A state property a block declares on its own, beside its template's.
/// A block's states become the template's states times these values, so a
/// pipe can have a connection per side without a template that has one.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum StateProperty {
    /// `false` and `true`, in vanilla's spelling.
    Boolean,
    /// A list of values, in the order the state space is built in.
    Enum { values: Vec<String> },
}

impl StateProperty {
    /// Its values, in the order the state space is built in; a boolean's two
    /// come `false` first, as vanilla orders them.
    pub fn values(&self) -> Vec<&str> {
        match self {
            StateProperty::Boolean => vec!["false", "true"],
            StateProperty::Enum { values } => values.iter().map(String::as_str).collect(),
        }
    }
}

/// A vanilla state property's name, and a value's: lower case, digits and
/// underscores (`minecraft:oak_log` is `axis`, a repeater is `delay`).
fn state_name(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockFile {
    template: String,
    #[serde(default)]
    properties: BlockProperties,
    shapes: Option<BlockShapes>,
    collision_shapes: Option<BlockShapes>,
    #[serde(default)]
    outline_context_shapes: BTreeMap<ShapeEntityKind, BlockShapes>,
    #[serde(default)]
    collision_context_shapes: BTreeMap<ShapeEntityKind, BlockShapes>,
    interaction_shapes: Option<BlockShapes>,
    #[serde(default)]
    shapes_from_block_entity: bool,
    context_collision: Option<ContextCollision>,
    state_light: Option<StateLight>,
    #[serde(default)]
    state_properties: BTreeMap<String, StateProperty>,
    #[serde(default)]
    default_state: BTreeMap<String, String>,
    #[serde(default)]
    connects_to: Option<String>,
    #[serde(default)]
    thickness: Option<f32>,
    #[serde(default)]
    wood_type: Option<String>,
    #[serde(default)]
    wall_block: Option<String>,
    #[serde(default)]
    sign_text_color: Option<String>,
    #[serde(default)]
    sign_text_glow: Option<bool>,
    #[serde(default)]
    base_block: Option<String>,
    #[serde(default = "yes")]
    item: bool,
    #[serde(default = "yes")]
    use_block_description_prefix: bool,
    #[serde(default, deserialize_with = "GateSound::deserialize_open")]
    open_sound: Option<GateSound>,
    #[serde(default, deserialize_with = "GateSound::deserialize_close")]
    close_sound: Option<GateSound>,
    block_item: Option<String>,
    #[serde(default)]
    pick_item: Option<String>,
    #[serde(default)]
    components: BTreeMap<String, serde_json::Value>,
    suspicious_stew_effects: Option<Vec<StewEffect>>,
}

/// One entry of a block's suspicious stew effect list, in the item component's JSON form.
#[derive(Clone, Debug, PartialEq, serde::Serialize, Deserialize)]
pub struct StewEffect {
    pub id: String,
    /// Duration in ticks. Omitted or invalid durations use 160.
    #[serde(
        default = "stew_duration",
        deserialize_with = "deserialize_stew_duration"
    )]
    pub duration: i32,
}

fn stew_duration() -> i32 {
    160
}

fn deserialize_stew_duration<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<i32, D::Error> {
    Ok(serde_json::Value::deserialize(deserializer)?
        .as_i64()
        .and_then(|duration| i32::try_from(duration).ok())
        .unwrap_or_else(stew_duration))
}

fn yes() -> bool {
    true
}

/// A fence gate's sound event at every site, or only the named sites.
/// Missing sites keep the template wood's sounds.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum GateSound {
    All(String),
    Sites {
        #[serde(rename = "use")]
        on_use: Option<String>,
        #[serde(rename = "explosion")]
        on_explosion: Option<String>,
        #[serde(rename = "redstone")]
        on_redstone: Option<String>,
    },
}

impl GateSound {
    fn deserialize_open<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Self>, D::Error> {
        Option::<Self>::deserialize(d)
            .map_err(|e| serde::de::Error::custom(format!("open_sound: {e}")))
    }
    fn deserialize_close<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Self>, D::Error> {
        Option::<Self>::deserialize(d)
            .map_err(|e| serde::de::Error::custom(format!("close_sound: {e}")))
    }
}

/// A block the mod adds: a vanilla template's behaviour, states and shapes
/// under a new id, with `properties` applied. A mod's code declares one as a
/// `mod_api::Block` (`native_blocks`), whose `item` is true as a file's is.
/// Here `..Default::default()` leaves the rest as a file leaves it out, except
/// `item`, false: the engine's own blocks, such as a fluid's, have none.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Block {
    /// `<mod id>:<path>`.
    pub id: String,
    /// The vanilla block it copies, `minecraft:<path>`.
    pub template: String,
    pub properties: BlockProperties,
    pub shapes: Option<BlockShapes>,
    pub collision_shapes: Option<BlockShapes>,
    /// Independent overrides for actual entity queries; omitted kinds use the defaults.
    pub outline_context_shapes: BTreeMap<ShapeEntityKind, BlockShapes>,
    pub collision_context_shapes: BTreeMap<ShapeEntityKind, BlockShapes>,
    /// Ray-face override; does not replace the outline hit point or create a hit.
    pub interaction_shapes: Option<BlockShapes>,
    /// The block's block entity publishes its outline and collision shapes, which replace the
    /// declared ones until cleared. Needs a block entity type that owns the block.
    pub shapes_from_block_entity: bool,
    /// A vanilla block's context collision it takes, over its shapes for entities that have
    /// no `collision_context_shapes` of their own.
    pub context_collision: Option<ContextCollision>,
    /// Per-state emission; takes precedence over `properties.light`.
    pub state_light: Option<StateLight>,
    /// State properties of its own, which its template does not have.
    pub state_properties: BTreeMap<String, StateProperty>,
    /// State properties overlaid on the template default.
    pub default_state: BTreeMap<String, String>,
    /// Also add a block item with the same id.
    pub item: bool,
    /// The block item is named by its own key, `item.<namespace>.<path>`, not the block's
    /// `block.<namespace>.<path>`: a mod whose `Item.Properties` do not call
    /// `useBlockDescriptionPrefix`. False, the block's key, as vanilla's `Items.registerBlock`.
    pub own_item_name: bool,
    /// A fence gate's opening sounds (`WoodType.fenceGateOpen`). `All` replaces
    /// every site; `Sites` keeps the template wood's sound for each missing site.
    /// Needs a `*_fence_gate` template.
    pub open_sound: Option<GateSound>,
    /// A fence gate's closing sounds (`WoodType.fenceGateClose`), with the same
    /// per-site fallback as `open_sound`.
    pub close_sound: Option<GateSound>,
    /// The item this block is, for pick-block and `Block.asItem`, when
    /// it is not a block item of its own: an item of this mod or a vanilla
    /// one, checked to exist when the content loads. Needs `item` false.
    pub block_item: Option<String>,
    /// The item pick-block takes from this block (`getCloneItemStack`), when it is not
    /// the block's item: a mature crop picks its seed. An item of this mod or a vanilla
    /// one, checked to exist when the content loads. Absent is the block's item.
    pub pick_item: Option<String>,
    /// Its item's component patch, as an [`Item`]'s.
    pub components: BTreeMap<String, serde_json::Value>,
    /// Ordered effects supplied by this block, independent of its item's components.
    pub suspicious_stew_effects: Option<Vec<StewEffect>>,
    /// A connecting block (template `minecraft:chorus_plant`, a `PipeBlock`
    /// without the chorus plant's rules): the block tag, without `#`, whose
    /// blocks it joins besides itself (a cable's machines).
    pub connects_to: Option<String>,
    /// A connecting block's width in pixels (2 to 14): its shapes, the
    /// chorus plant's (10) when unset.
    pub thickness: Option<f32>,
    /// A sign's `WoodType` (template a vanilla sign), which names its editor's
    /// background: `<namespace>:<path>` a mod's texture, a bare name vanilla's.
    /// The template's when unset.
    pub wood_type: Option<String>,
    /// The block a standing or ceiling sign's item places on a wall
    /// (`StandingAndWallBlockItem.wallBlock`): one of this mod's blocks, whose
    /// template is the matching wall sign and which has no item. The
    /// template's when unset.
    pub wall_block: Option<String>,
    /// A sign's new text colour (`SignBlockEntity.createDefaultSignText`): a `DyeColor` name,
    /// black when unset. Applies to a sign when it is placed, front and back.
    pub sign_text_color: Option<String>,
    /// Whether a new sign's text glows, false when unset.
    pub sign_text_glow: Option<bool>,
    /// The block farmland turns into; the template's base when unset.
    pub base_block: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreativeTabFile {
    title: String,
    icon: String,
    items: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FluidFile {
    sprite: Option<String>,
    tint: Option<u32>,
    world: Option<WorldFluid>,
    bucket_item: Option<String>,
    temperature: Option<i32>,
    density: Option<i32>,
    viscosity: Option<i32>,
    light_level: Option<u8>,
}

/// A fluid's `FluidType` numbers, which mods read for one another's fluids: NeoForge's
/// `FluidType.getTemperature`, `getDensity`, `getViscosity` and `getLightLevel`. Each defaults
/// to `FluidType.Properties`' own, which are water's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FluidMeasures {
    /// How hot it is, room temperature in kelvin by default: 300.
    pub temperature: i32,
    /// How dense it is, water's kg/m³ by default: 1000. Below 0 is lighter than air.
    pub density: i32,
    /// How thick it is, 0 or more, water's by default: 1000.
    pub viscosity: i32,
    /// The light it gives, 0 to 15, water's by default: 0.
    pub light_level: u8,
}
impl Default for FluidMeasures {
    fn default() -> Self {
        Self {
            temperature: 300,
            density: 1000,
            viscosity: 1000,
            light_level: 0,
        }
    }
}

/// What makes a fluid exist in the world: a block holding it, as `minecraft:water`'s does, and
/// how it flows. Each value defaults to water's, in `data/<id>/sunrise/fluid/` as in code
/// (`Registrar::declare_world_fluid`).
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct WorldFluid {
    /// Game ticks between two flow steps (`Fluid.getTickDelay`).
    pub tick_delay: i32,
    /// What it loses per block it flows (`FlowingFluid.getDropOff`), 0 or more.
    pub drop_off: i32,
    /// How far it looks for a way down (`FlowingFluid.getSlopeFindDistance`).
    pub slope_find_distance: i32,
    /// Whether two sources make a third (`FlowingFluid.canConvertToSource`). Off by default:
    /// water's comes from a game rule, which a mod's fluid does not read.
    pub source_conversion: bool,
    /// Whether the game registers its filled bucket, a water bucket under the id the look's
    /// `bucket_item` names, `<id>_bucket` when it names none.
    pub bucket: bool,
    /// The registry name of its flowing form, in the fluid's namespace;
    /// `<namespace>:flowing_<path>` when unset.
    pub flowing: Option<String>,
    /// The registry name of its liquid block (`LiquidBlock`, NeoForge's
    /// `BaseFlowingFluid.Properties.block`), in the fluid's namespace; the fluid's own id when
    /// unset.
    pub block: Option<String>,
}
impl Default for WorldFluid {
    fn default() -> Self {
        Self {
            tick_delay: 5,
            drop_off: 1,
            slope_find_distance: 4,
            source_conversion: false,
            bucket: true,
            flowing: None,
            block: None,
        }
    }
}
impl WorldFluid {
    /// Refuses a negative `drop_off`: the level a spread computes (`FlowingFluid.getNewLiquid`,
    /// the highest neighbour less the drop-off) must stay within `FlowingFluid.LEVEL`'s 1 to 8.
    /// Every other value is NeoForge's `BaseFlowingFluid.Properties` domain, any `int`.
    pub fn check(&self) -> Result<(), String> {
        if self.drop_off < 0 {
            return Err("drop_off is 0 or more".into());
        }
        Ok(())
    }
}

/// How a fluid the mod names looks on the client (`data/<id>/sunrise/fluid/`), as
/// NeoForge's `IClientFluidTypeExtensions`: `getStillTexture` and `getTintColor`, and its
/// [`FluidMeasures`]. The fluid needs no block, so it can be a fluid only a tank holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FluidLook {
    /// `<mod id>:<path>`.
    pub id: String,
    /// An atlas sprite id (`namespace:path`); none draws a flat square of the tint.
    pub sprite: Option<String>,
    /// `0xAARRGGBB`.
    pub tint: Option<u32>,
    /// How it exists in the world; none for a fluid only a tank holds.
    pub world: Option<WorldFluid>,
    /// The item that holds a bucket of it (`Fluid.getBucket`): another item for a tank-only
    /// fluid or a world fluid without a generated bucket, the generated bucket's id otherwise.
    pub bucket_item: Option<String>,
    /// Its temperature, density, viscosity and light level.
    pub measures: FluidMeasures,
}
impl FluidLook {
    /// The registry name of its liquid block: the world fluid's `block`, else the fluid's own
    /// id.
    pub fn block_id(&self) -> String {
        self.world
            .as_ref()
            .and_then(|world| world.block.clone())
            .unwrap_or_else(|| self.id.clone())
    }
    /// The registry name of its flowing form: the world fluid's `flowing`, else
    /// `<namespace>:flowing_<path>`.
    pub fn flowing_id(&self) -> String {
        if let Some(flowing) = self.world.as_ref().and_then(|world| world.flowing.clone()) {
            return flowing;
        }
        match self.id.split_once(':') {
            Some((namespace, path)) => format!("{namespace}:flowing_{path}"),
            None => format!("flowing_{}", self.id),
        }
    }
    /// The item a bucket of it is (`Fluid.getBucket`): `bucket_item`, else `<id>_bucket` for a
    /// world fluid with a generated bucket. None for a fluid no bucket holds.
    pub fn bucket_id(&self) -> Option<String> {
        self.bucket_item.clone().or_else(|| {
            self.world
                .as_ref()
                .filter(|world| world.bucket)
                .map(|_| format!("{}_bucket", self.id))
        })
    }
    /// Refuses names and numbers the game cannot register: an id that is not
    /// `namespace:path`, a flowing form or generated bucket outside the fluid's namespace, and a
    /// light level above 15.
    pub fn check(&self) -> Result<(), String> {
        let namespace = self.id.split_once(':').map(|(namespace, _)| namespace);
        let own = |id: &str| namespaced_id(id) && id.split_once(':').map(|(n, _)| n) == namespace;
        if let Some(world) = &self.world {
            world.check().map_err(|e| format!("world: {e}"))?;
            if world
                .flowing
                .as_deref()
                .is_some_and(|flowing| !own(flowing) || flowing == self.id)
            {
                return Err(
                    "world: flowing is an id in the fluid's namespace, not the fluid's own".into(),
                );
            }
            // The resolved flowing id, the default `<ns>:flowing_<path>` included, as the code
            // registrar compares against.
            let flowing = self.flowing_id();
            if world
                .block
                .as_deref()
                .is_some_and(|block| !own(block) || block == flowing)
            {
                return Err(
                    "world: block is an id in the fluid's namespace, not its flowing form's".into(),
                );
            }
        }
        if let Some(item) = &self.bucket_item {
            let generated = self.world.as_ref().is_some_and(|world| world.bucket);
            if !namespaced_id(item) || (generated && !own(item)) {
                return Err(
                    "bucket_item is an item id, in the fluid's namespace when the game registers it"
                        .into(),
                );
            }
        }
        if self.measures.light_level > 15 {
            return Err("light_level is 0 to 15".into());
        }
        Ok(())
    }
    /// The block a world fluid is in the world: water's behaviour and states under its
    /// [`Self::block_id`].
    pub fn world_block(&self) -> Option<Block> {
        self.world.as_ref().map(|_| Block {
            id: self.block_id(),
            template: "minecraft:water".into(),
            ..Block::default()
        })
    }
    /// The item its bucket is: a water bucket's under its own id, leaving the empty bucket.
    pub fn world_bucket(&self) -> Option<Item> {
        let generated = self.world.as_ref().is_some_and(|world| world.bucket);
        self.bucket_id().filter(|_| generated).map(|id| Item {
            id,
            template: "minecraft:water_bucket".into(),
            crafting_remainder: Some("minecraft:bucket".into()),
            ..Item::default()
        })
    }
}

/// What a world fluid draws with beyond its [`FluidLook`], as NeoForge's fluid models: registered
/// by a client mod in `register_fluid_models`. An unset sprite keeps the look's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FluidModel {
    /// The world fluid's id, `<mod id>:<path>`.
    pub fluid: String,
    /// The block atlas sprite on a surface at rest; the look's `sprite` when unset.
    pub still: Option<String>,
    /// The sprite on a surface that flows, and on its sides; `still` when unset.
    pub flowing: Option<String>,
    /// The sprite on sides against glass and leaves, as water's overlay; `flowing` when unset.
    pub overlay: Option<String>,
}

/// A creative inventory tab the mod adds (`data/<id>/sunrise/creative_tab/`):
/// its title, the item on its button, and its items in order. Items no mod
/// or vanilla registers are left out when the game loads.
#[derive(Clone, Debug, PartialEq)]
pub struct CreativeTab {
    /// `<mod id>:<path>`.
    pub id: String,
    pub title: String,
    pub icon: String,
    pub items: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Rarity {
    Common,
    Uncommon,
    Rare,
    Epic,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemFile {
    template: String,
    max_stack_size: Option<u8>,
    rarity: Option<Rarity>,
    #[serde(default)]
    components: BTreeMap<String, serde_json::Value>,
    crafting_remainder: Option<String>,
    block: Option<String>,
    arm_pose: Option<ArmPose>,
}

/// An item the mod adds: a vanilla template item's components under a new
/// id, with these changed. A native mod may build one in code
/// (`native_items`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    pub id: String,
    /// The vanilla item it copies, `minecraft:<path>`.
    pub template: String,
    pub max_stack_size: Option<u8>,
    pub rarity: Option<Rarity>,
    /// A 26.3 data component patch over the template's: `!minecraft:<type>` removes one.
    pub components: BTreeMap<String, serde_json::Value>,
    /// The registered item it leaves in the crafting grid (`Item.Properties.craftRemainder`).
    /// Omit this field for no remainder; `minecraft:air` is not a stack.
    pub crafting_remainder: Option<String>,
    /// The registered block placed by this item, if any.
    pub block: Option<String>,
    /// The arm pose a humanoid takes holding it, in place of vanilla's (`ArmPose`).
    pub arm_pose: Option<ArmPose>,
}

/// `MobEffectCategory`.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EffectCategory {
    Beneficial,
    Harmful,
    Neutral,
}

/// A mob effect the mod adds: what `new MobEffect(category, color)` takes,
/// and the attribute modifiers its constructor adds.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MobEffect {
    /// `<mod id>:<path>`.
    #[serde(skip)]
    pub id: String,
    pub category: EffectCategory,
    /// RGB, `0x000000` to `0xFFFFFF`.
    pub color: u32,
    /// Where the effect sits in the HUD icons and the inventory's effect list
    /// (NeoForge's `IMobEffectExtension.getSortOrder`): compared where vanilla
    /// compares the colour, which it is when absent.
    #[serde(default)]
    pub sort_order: Option<i32>,
    /// `MobEffect.addAttributeModifier`, one per attribute: on an entity while
    /// the effect is, its amount times the amplifier plus one.
    #[serde(default)]
    pub attribute_modifiers: Vec<EffectAttributeModifier>,
}

/// One `MobEffect.addAttributeModifier(attribute, id, amount, operation)`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EffectAttributeModifier {
    /// The attribute, vanilla's or a mod's, `namespace:path`.
    pub attribute: String,
    /// The modifier's id, `namespace:path`.
    pub id: String,
    /// The amount at amplifier 0.
    pub amount: f64,
    pub operation: ModifierOperation,
}

/// `AttributeModifier.Operation`, by its codec name.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModifierOperation {
    AddValue,
    AddMultipliedBase,
    AddMultipliedTotal,
}

impl MobEffect {
    /// The checks a declaration's attribute modifiers pass at load: each
    /// attribute and id a `namespace:path` (`Identifier`'s rules, which refuse
    /// the namespace `..`), and one modifier per attribute
    /// (vanilla keys its templates by attribute). JSON has no infinite or NaN
    /// amount: one out of `f64`'s range fails to parse. Whether the attribute
    /// exists is the game's check, which knows the mods' attributes.
    fn check_attribute_modifiers(&self) -> Result<(), String> {
        let identifier =
            |id: &str| sunrise_core::identifier::Identifier::parse_namespaced(id).is_some();
        for (i, modifier) in self.attribute_modifiers.iter().enumerate() {
            let entry = format!("attribute_modifiers[{i}]");
            if !identifier(&modifier.attribute) {
                return Err(format!(
                    "{entry}: attribute {:?} is an attribute id (namespace:path)",
                    modifier.attribute
                ));
            }
            if !identifier(&modifier.id) {
                return Err(format!(
                    "{entry}: id {:?} is a modifier id (namespace:path)",
                    modifier.id
                ));
            }
            if let Some(first) = self.attribute_modifiers[..i]
                .iter()
                .position(|m| m.attribute == modifier.attribute)
            {
                return Err(format!(
                    "attribute_modifiers[{first}] ({}) and {entry} ({}) both modify {}: one modifier per attribute",
                    self.attribute_modifiers[first].id, modifier.id, modifier.attribute
                ));
            }
        }
        Ok(())
    }
}

/// Vanilla's `DyeColor` names, by id.
pub const DYE_COLORS: [&str; 16] = [
    "white",
    "orange",
    "magenta",
    "light_blue",
    "yellow",
    "lime",
    "pink",
    "gray",
    "light_gray",
    "cyan",
    "purple",
    "blue",
    "brown",
    "green",
    "red",
    "black",
];

/// the vanilla entity types a mod's type may copy: thrown item
/// projectiles (`ThrowableItemProjectile`), then living mobs
/// ([`LIVING_TEMPLATES`]).
pub const ENTITY_TEMPLATES: [&str; 9] = [
    "minecraft:snowball",
    "minecraft:cat",
    "minecraft:drowned",
    "minecraft:parrot",
    "minecraft:skeleton",
    "minecraft:villager",
    "minecraft:wandering_trader",
    "minecraft:wolf",
    "minecraft:zombie",
];

/// the [`ENTITY_TEMPLATES`] that are living mobs: a type copying one is a mob
/// of its template's class (NeoForge's `EntityType.Builder.of(Zombie::new,
/// ...)`), with its own default attributes (`EntityAttributeCreationEvent`).
/// The wandering trader is the neutral one whose behaviour is all goals,
/// which a native mod can replace (`register_goals`); a villager's is its
/// brain. The cat, the parrot and the wolf are the tameable ones
/// (`TamableAnimal`): a type copying one tames, sits and follows its owner.
/// The skeleton is the ranged one (`AbstractSkeleton`): a type copying it
/// draws and shoots a bow it holds, shuns the sun and flees wolves. The
/// drowned is the aquatic fighter: a type copying it swims, goes to water,
/// and fights with a trident or in melee.
pub const LIVING_TEMPLATES: [&str; 8] = [
    "minecraft:cat",
    "minecraft:drowned",
    "minecraft:parrot",
    "minecraft:skeleton",
    "minecraft:villager",
    "minecraft:wandering_trader",
    "minecraft:wolf",
    "minecraft:zombie",
];

pub mod entity_sounds;
mod entity_type;
pub use entity_type::EntityType;
mod face_rules;
pub use face_rules::{check_face_rule_properties, parse_face_rules};
pub mod fluid_interaction;
pub use fluid_interaction::FluidInteraction;
mod lenient;
pub use lenient::without_comments;
#[cfg(feature = "references")]
mod references;
#[cfg(feature = "references")]
pub use references::{Findings, JarLayer, check_references};

/// A block entity type declared in code, with the blocks it supports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockEntityType {
    pub id: String,
    pub blocks: Vec<String>,
}

/// a menu type the mod adds (vanilla's `MenuType`, NeoForge's
/// `IMenuTypeExtension.create`): its container's slot count, the
/// `ContainerData` count its menu syncs, and its take-only slots
/// (`FurnaceResultSlot.mayPlace`). The container slots come first, then the
/// player's 36 if included. A mod screen (`native_screen`) places them.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MenuType {
    /// `<mod id>:<path>`.
    #[serde(skip)]
    pub id: String,
    /// 0 to [`MAX_MENU_SLOTS`].
    pub slots: u16,
    /// Append the 36 player inventory slots (default: true).
    #[serde(default = "yes")]
    pub player_inventory: bool,
    /// Number of integer data slots synced by the menu (`ContainerData`), 0 by default.
    #[serde(default)]
    pub data_slots: usize,
    /// Per-slot maxima for transient slots when the menu is opened with no backing block.
    /// When present, supplies one maximum (1–99) for each container slot. Cannot be combined
    /// with take-only `results` or a `crafting` layout.
    #[serde(default)]
    pub owned_slots: Option<Vec<u8>>,
    /// slots a player takes from but never puts into, each below `slots`.
    #[serde(default)]
    pub results: Vec<u16>,
    /// Zero-based machine input slots, disjoint from results and crafting slots.
    /// This is a client layout role, not an item acceptance or transfer rule.
    #[serde(default)]
    pub inputs: Vec<u16>,
    /// a crafting grid over some of the slots, which crafts as vanilla's
    /// crafting table does (`CraftingMenu`).
    #[serde(default)]
    pub crafting: Option<MenuCrafting>,
    /// slots that show other containers' slots instead of the block
    /// entity's, as a crafting station shows the chests beside it; a native
    /// mod names the containers (`native_menu_links`).
    #[serde(default)]
    pub links: Option<MenuLinks>,
}

/// a mod menu type's linked slots (`links`): `count` slots from `from` on,
/// which show the slots of the containers a native mod links them to
/// (`native_menu_links`), in order, rather than the block entity's. The
/// block entity never holds them; a linked slot with nothing behind it holds
/// nothing and takes nothing.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MenuLinks {
    /// the first linked slot.
    pub from: u16,
    /// how many slots, 1 or more.
    pub count: u16,
}

impl MenuLinks {
    fn range(&self) -> std::ops::Range<u32> {
        u32::from(self.from)..u32::from(self.from) + u32::from(self.count)
    }
}

/// a mod menu type's crafting grid (`crafting`): `size`x`size` slots from
/// `grid` on, row by row, and the slot that shows what they craft
/// (`ResultSlot`). The block entity holds the grid; the result is the
/// menu's, never in the block entity.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MenuCrafting {
    /// the grid's first slot.
    pub grid: u16,
    /// 2 (the inventory's grid) or 3 (a crafting table's); 3 by default.
    #[serde(default = "MenuCrafting::default_size")]
    pub size: u16,
    /// the result slot, outside the grid and not one of `results`.
    pub result: u16,
}

impl MenuCrafting {
    fn default_size() -> u16 {
        3
    }
}

/// The most total item slots a mod menu type may carry (container backing slots plus
/// the 36 appended player inventory slots when `player_inventory` is true): `32,768`,
/// matching vanilla's signed 16-bit slot index range `0..=32,767` (`i16::MAX`).
pub const MAX_MENU_SLOTS: u16 = (i16::MAX as u16) + 1;

/// The most data slots carried over vanilla's signed 16-bit `ClientboundContainerSetDataPacket`
/// (`0..=32,767`, count `32,768`).
pub const MAX_SIGNED_DATA_SLOTS: u16 = (i16::MAX as u16) + 1;

/// The most data slots carried over the unsigned 16-bit `sunrise:menu_data` payload
/// (`0..=65,535`, count `65,536`).
pub const MAX_BLOCKLESS_DATA_SLOTS: usize = u16::MAX as usize + 1;

#[derive(Clone)]
enum Source {
    Dir(PathBuf),
    Zip {
        path: PathBuf,
        archive: Arc<Mutex<zip::ZipArchive<std::fs::File>>>,
    },
    /// No files: content a native mod declares in code.
    Code,
}

impl fmt::Debug for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dir(path) => f.debug_tuple("Dir").field(path).finish(),
            Self::Zip { path, .. } => f.debug_tuple("Zip").field(path).finish(),
            Self::Code => f.write_str("Code"),
        }
    }
}

/// A mod's content: its files and declarations.
pub struct Content {
    source: Source,
    from_admission: bool,
    /// Every content file (`assets/...`, `data/...`), sorted.
    pub files: Arc<[String]>,
    pub blocks: Vec<Block>,
    pub items: Vec<Item>,
    pub effects: Vec<MobEffect>,
    /// Its creative inventory tabs, by id.
    pub creative_tabs: Vec<CreativeTab>,
    /// How its fluids look, by id.
    pub fluids: Vec<FluidLook>,
    /// What its liquid blocks turn into beside another fluid or block, in file-name order.
    pub fluid_interactions: Vec<FluidInteraction>,
    pub entity_types: Vec<EntityType>,
    /// Block entity types declared by native code, not content files.
    pub block_entity_types: Vec<BlockEntityType>,
    pub menu_types: Vec<MenuType>,
    /// The namespaces it owns: the mod id, then the manifest's `namespaces`.
    pub namespaces: Vec<String>,
}

impl fmt::Debug for Content {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The release-line corpus records the declarations, not loader bookkeeping.
        f.debug_struct("Content")
            .field("source", &self.source)
            .field("files", &self.files)
            .field("blocks", &self.blocks)
            .field("items", &self.items)
            .field("effects", &self.effects)
            .field("creative_tabs", &self.creative_tabs)
            .field("fluids", &self.fluids)
            .field("fluid_interactions", &self.fluid_interactions)
            .field("entity_types", &self.entity_types)
            .field("block_entity_types", &self.block_entity_types)
            .field("menu_types", &self.menu_types)
            .field("namespaces", &self.namespaces)
            .finish()
    }
}

macro_rules! declaration_kinds {
    ($($kind:ident => $folder:literal: $file:ty),+ $(,)?) => {
        enum FileKind { $($kind),+ }

        impl FileKind {
            fn from_folder(folder: &str) -> Option<Self> {
                DECLARATION_KINDS.iter().zip([$(Self::$kind),+])
                    .find_map(|(&name, kind)| (name == folder).then_some(kind))
            }
        }

        /// Declaration folders admitted by the content reader.
        #[doc(hidden)]
        pub const DECLARATION_KINDS: &[&str] = &[$($folder),+];

        /// Exposes the declaration file types to the integration test's Serde schema probe.
        #[doc(hidden)]
        pub fn probe_declaration<'de, D: serde::Deserializer<'de>>(
            kind: &str,
            deserializer: D,
        ) -> Result<(), D::Error> {
            match FileKind::from_folder(kind) {
                $(Some(FileKind::$kind) => <$file>::deserialize(deserializer).map(drop),)+
                None => Err(serde::de::Error::custom(format_args!(
                    "unknown declaration kind {kind:?}"
                ))),
            }
        }
    };
}

declaration_kinds! {
    Block => "block": BlockFile,
    Item => "item": ItemFile,
    MobEffect => "mob_effect": MobEffect,
    CreativeTab => "creative_tab": CreativeTabFile,
    Fluid => "fluid": FluidFile,
    FluidInteraction => "fluid_interaction": FluidInteraction,
    EntityType => "entity_type": EntityType,
    Menu => "menu": MenuType,
}

/// Where a package's content files are read from: its folder or its `.smod`.
#[derive(Debug, Clone)]
pub struct ContentSource {
    source: Source,
    files: Arc<[String]>,
}

impl ContentSource {
    /// The package at `path` (a `.smod` or a mod folder), and every content file it holds
    /// (`assets/...`, `data/...`), sorted: content loading's own walk and checks, each path
    /// [`check_path`], at most [`MAX_FILE`] bytes per file, [`MAX_FILES`] files and [`MAX_TOTAL`]
    /// bytes in all.
    pub fn open(path: &Path) -> Result<Self, String> {
        let (source, sized) = if path.is_dir() {
            (Source::Dir(path.to_owned()), folder_files(path)?)
        } else {
            let mut zip = open_zip(path)?;
            let mut out = Vec::new();
            for i in 0..zip.len() {
                let entry = zip.by_index_raw(i).map_err(|e| e.to_string())?;
                let name = entry.name().map_err(|e| e.to_string())?.into_owned();
                if (name.starts_with("assets/") || name.starts_with("data/")) && !entry.is_dir() {
                    out.push((name, entry.size()));
                }
            }
            (
                Source::Zip {
                    path: path.to_owned(),
                    archive: Arc::new(Mutex::new(zip)),
                },
                out,
            )
        };
        let mut files = Vec::with_capacity(sized.len());
        let mut total = 0u64;
        for (name, size) in sized {
            check_path(&name)?;
            if size > MAX_FILE {
                return Err(format!("{name} is over {} MiB", MAX_FILE >> 20));
            }
            total += size;
            files.push(name);
        }
        if files.len() > MAX_FILES {
            return Err(format!("more than {MAX_FILES} content files"));
        }
        if total > MAX_TOTAL {
            return Err(format!("content is over {} MiB", MAX_TOTAL >> 20));
        }
        files.sort();
        files.dedup();
        Ok(Self {
            source,
            files: files.into(),
        })
    }

    /// The sorted content names captured when this source opened.
    pub fn names(&self) -> Arc<[String]> {
        self.files.clone()
    }

    /// Check the manifest's namespace fence before any registrant can see these files.
    pub fn admit(self, manifest: &Manifest) -> Result<AdmittedContentSource, String> {
        for name in self.files.iter() {
            check_namespace(name, manifest)?;
        }
        Ok(AdmittedContentSource(self))
    }

    /// Whether the file list came from an opened archive rather than a loose folder.
    pub fn is_archive(&self) -> bool {
        matches!(self.source, Source::Zip { .. })
    }

    /// A reader of its files, which checks each path and size again as it reads.
    pub fn reader(&self) -> FileReader<'_> {
        self.source.reader()
    }
}

/// A package's opened file index, already checked against its manifest's namespace fence.
#[derive(Debug)]
pub struct AdmittedContentSource(ContentSource);

impl AdmittedContentSource {
    pub fn source(&self) -> &ContentSource {
        &self.0
    }
}

/// A content file could not be read, with the loader's diagnostic.
#[derive(Debug)]
pub enum ReadError {
    /// The folder or archive has no file at this name.
    NotFound(String),
    /// The path, size or read failed for some other reason.
    Other(String),
}

impl ReadError {
    fn from_io(name: &str, error: io::Error) -> Self {
        let detail = format!("{name}: {error}");
        if error.kind() == io::ErrorKind::NotFound {
            Self::NotFound(detail)
        } else {
            Self::Other(detail)
        }
    }
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(reason) | Self::Other(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for ReadError {}

impl From<ReadError> for String {
    fn from(error: ReadError) -> Self {
        error.to_string()
    }
}

/// Reads a mod's content files ([`Content::reader`]).
pub struct FileReader<'a>(FileLocation<'a>);

enum FileLocation<'a> {
    Dir(&'a Path),
    Zip(&'a Mutex<zip::ZipArchive<std::fs::File>>),
}

impl FileReader<'_> {
    /// One content file's bytes.
    pub fn read(&self, name: &str) -> Result<Vec<u8>, ReadError> {
        self.read_head(name, MAX_FILE + 1)
    }

    /// Read no more than `max` bytes, and keep the package's own full-file bound.
    pub fn read_head(&self, name: &str, max: u64) -> Result<Vec<u8>, ReadError> {
        check_path(name).map_err(ReadError::Other)?;
        let read = |reader: &mut dyn Read| {
            let mut out = Vec::new();
            reader
                .take(max.min(MAX_FILE + 1))
                .read_to_end(&mut out)
                .map_err(|error| ReadError::from_io(name, error))?;
            if out.len() as u64 > MAX_FILE {
                return Err(ReadError::Other(format!(
                    "{name} is over {} MiB",
                    MAX_FILE >> 20
                )));
            }
            Ok(out)
        };
        match self.0 {
            FileLocation::Dir(root) => read(
                &mut std::fs::File::open(root.join(name))
                    .map_err(|error| ReadError::from_io(name, error))?,
            ),
            FileLocation::Zip(zip) => {
                let mut archive = zip
                    .lock()
                    .map_err(|e| ReadError::Other(format!("{name}: {e}")))?;
                let mut entry = archive.by_name(name).map_err(|e| {
                    let detail = format!("{name}: {e}");
                    if matches!(e, zip::result::ZipError::FileNotFound) {
                        ReadError::NotFound(detail)
                    } else {
                        ReadError::Other(detail)
                    }
                })?;
                read(&mut entry)
            }
        }
    }
    /// A content file's path on disk, checked as [`read`](Self::read) checks it (the name,
    /// the size), for a caller that copies it without reading it; `None` in an archive.
    pub fn path(&self, name: &str) -> Result<Option<std::path::PathBuf>, String> {
        check_path(name)?;
        let root = match self.0 {
            FileLocation::Dir(root) => root,
            FileLocation::Zip(_) => return Ok(None),
        };
        let path = root.join(name);
        let len = std::fs::metadata(&path)
            .map_err(|e| format!("{name}: {e}"))?
            .len();
        if len > MAX_FILE {
            return Err(format!("{name} is over {} MiB", MAX_FILE >> 20));
        }
        Ok(Some(path))
    }
}

/// Why two content mods cannot be on together: a namespace both own (one's
/// id or listed namespace is the other's), which would mix their files.
/// `a` and `b` are (mod id, owned namespaces).
pub fn clash(a: (&str, &[String]), b: (&str, &[String])) -> Option<String> {
    let shared = a.1.iter().find(|ns| b.1.contains(ns))?;
    Some(format!(
        "mods {} and {} both use the namespace {shared}; turn one of them off",
        a.0, b.0
    ))
}

impl Content {
    /// The content of the mod at `path` (a `.smod` or a mod folder).
    /// Item references are validated as identifiers here and resolved against the game's registries at load.
    pub fn read(path: &Path, manifest: &Manifest) -> Result<Content, String> {
        Self::read_inner(&ContentSource::open(path)?, manifest, false)
    }

    /// Read the declarations using the source already checked against this manifest at admission.
    pub fn read_opened(
        source: &AdmittedContentSource,
        manifest: &Manifest,
    ) -> Result<Content, String> {
        Self::read_inner(source.source(), manifest, true)
    }

    fn read_inner(
        source: &ContentSource,
        manifest: &Manifest,
        from_admission: bool,
    ) -> Result<Content, String> {
        let mut content = Content {
            source: source.source.clone(),
            from_admission,
            files: source.names(),
            blocks: Vec::new(),
            items: Vec::new(),
            effects: Vec::new(),
            creative_tabs: Vec::new(),
            fluids: Vec::new(),
            fluid_interactions: Vec::new(),
            entity_types: Vec::new(),
            block_entity_types: Vec::new(),
            menu_types: Vec::new(),
            namespaces: manifest.owned_namespaces().map(str::to_owned).collect(),
        };
        content.check(manifest)?;
        Ok(content)
    }

    /// Content with no files, for native declarations in `namespace`
    /// ([`Content::declare`]).
    pub fn declared(namespace: &str) -> Content {
        Content {
            source: Source::Code,
            from_admission: false,
            files: Arc::default(),
            blocks: Vec::new(),
            items: Vec::new(),
            effects: Vec::new(),
            creative_tabs: Vec::new(),
            fluids: Vec::new(),
            fluid_interactions: Vec::new(),
            entity_types: Vec::new(),
            block_entity_types: Vec::new(),
            menu_types: Vec::new(),
            namespaces: vec![namespace.to_owned()],
        }
    }

    /// Adds blocks and items a native mod declares in code (`native_blocks`,
    /// `native_items`), checked as declaration files are: ids in any of this
    /// mod's owned namespaces (its id and manifest `namespaces`) and declared once
    /// with what it already has, a vanilla template, and well-formed item and block references.
    /// The references resolve at game load. All or nothing: on an error nothing is added.
    pub fn declare(
        &mut self,
        blocks: &[Block],
        items: &[Item],
    ) -> Result<(), Vec<(DeclarationKind, DeclarationError)>> {
        let in_own = |id: &str| {
            if namespaced_id(id)
                && id
                    .split_once(':')
                    .is_some_and(|(ns, _)| self.namespaces.iter().any(|owned| owned == ns))
            {
                Ok(())
            } else {
                Err(format!(
                    "not an id in owned namespaces {:?}",
                    self.namespaces
                ))
            }
        };
        self.check_declaration_batch(
            blocks
                .iter()
                .map(|b| (b, in_own(&b.id).and_then(|()| check_block(b)))),
            items
                .iter()
                .map(|i| (i, in_own(&i.id).and_then(|()| check_item(i)))),
        )?;
        self.blocks.extend_from_slice(blocks);
        self.items.extend_from_slice(items);
        Ok(())
    }

    /// Joins declarations whose fields and submitting namespaces were checked at registration.
    pub fn declare_validated(
        &mut self,
        blocks: &[LocalDeclaration<Block>],
        items: &[LocalDeclaration<Item>],
    ) -> Result<(), Vec<(DeclarationKind, DeclarationError)>> {
        self.check_declaration_batch(
            blocks.iter().map(|b| (b.get(), Ok(()))),
            items.iter().map(|i| (i.get(), Ok(()))),
        )?;
        self.blocks.extend(blocks.iter().map(|b| b.get().clone()));
        self.items.extend(items.iter().map(|i| i.get().clone()));
        Ok(())
    }

    fn check_declaration_batch<'a>(
        &self,
        blocks: impl IntoIterator<Item = (&'a Block, Result<(), String>)>,
        items: impl IntoIterator<Item = (&'a Item, Result<(), String>)>,
    ) -> Result<(), Vec<(DeclarationKind, DeclarationError)>> {
        let mut block_ids: BTreeSet<&str> = self.blocks.iter().map(|b| b.id.as_str()).collect();
        let mut item_ids: BTreeSet<&str> = self
            .blocks
            .iter()
            .filter(|b| b.item)
            .map(|b| b.id.as_str())
            .chain(self.items.iter().map(|i| i.id.as_str()))
            .collect();
        let mut errors = Vec::new();
        let mut refuse = |kind, id: &str, reason| {
            errors.push((
                kind,
                DeclarationError::Rejected {
                    id: id.to_owned(),
                    reason,
                },
            ));
        };
        let mut all_blocks: Vec<&Block> = self.blocks.iter().collect();
        for (block, fields) in blocks {
            let id = block.id.as_str();
            let result = fields.and_then(|()| {
                if block_ids.contains(id) {
                    return Err("declared twice".into());
                }
                if block.item && !item_ids.insert(id) {
                    return Err("block's item is already declared".into());
                }
                block_ids.insert(id);
                Ok(())
            });
            match result {
                Ok(()) => all_blocks.push(block),
                Err(reason) => refuse(DeclarationKind::Block, id, reason),
            }
        }
        sign_wall_refusals(&all_blocks)
            .for_each(|(id, reason)| refuse(DeclarationKind::Block, id, reason));
        for (item, fields) in items {
            let id = item.id.as_str();
            if let Err(reason) = fields.and_then(|()| {
                if !item_ids.insert(id) {
                    return Err("declared twice, or is also a block's item".into());
                }
                Ok(())
            }) {
                refuse(DeclarationKind::Item, id, reason);
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        Ok(())
    }

    /// Where the files are read from: the mod's folder or its `.smod`.
    pub fn source(&self) -> &Path {
        match &self.source {
            Source::Dir(path) => path,
            Source::Zip { path, .. } => path,
            Source::Code => Path::new(""),
        }
    }

    /// No files: a mod with code only.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// The refusal of a `side = "server"` package whose files declare registry content
    /// (`data/<owned namespace>/sunrise/`), naming the mod and the first such file: every client
    /// must have that content. Assets and data vanilla syncs itself (recipes, loot, tags) are
    /// allowed. None for a package of any other side.
    pub fn server_only_refusal(&self, manifest: &Manifest) -> Option<String> {
        if manifest.side != crate::manifest::Side::Server {
            return None;
        }
        let name = self.files.iter().find(|name| {
            let mut parts = name.splitn(3, '/');
            let (top, namespace, rest) = (parts.next(), parts.next(), parts.next());
            top == Some("data")
                && rest.is_some_and(|rest| rest.starts_with("sunrise/"))
                && manifest
                    .owned_namespaces()
                    .any(|owned| Some(owned) == namespace)
        })?;
        Some(format!(
            "{}: a side = \"server\" mod carries {name}, which every client must have; \
             declare side = \"both\"",
            manifest.id
        ))
    }

    /// The fixed listing and reader captured at startup, if this content came from admission.
    pub fn admitted_source(&self) -> Option<ContentSource> {
        self.from_admission.then(|| ContentSource {
            source: self.source.clone(),
            files: self.files.clone(),
        })
    }

    /// One content file's bytes.
    pub fn file(&self, name: &str) -> Result<Vec<u8>, String> {
        Ok(self.reader().read(name)?)
    }

    /// A reader of its content files, using the same archive from which the files were listed.
    pub fn reader(&self) -> FileReader<'_> {
        self.source.reader()
    }

    /// Where content may go, and the declarations.
    fn check(&mut self, manifest: &Manifest) -> Result<(), String> {
        let id = manifest.id.as_str();
        let mut blocks = Vec::new();
        let mut items = Vec::new();
        let mut effects = Vec::new();
        let mut tabs = Vec::new();
        let mut fluids = Vec::new();
        let mut interactions = Vec::new();
        let mut types = Vec::new();
        let mut menus = Vec::new();
        for name in self.files.iter() {
            let mut parts = name.splitn(3, '/');
            let (top, namespace, rest) = (
                parts.next().unwrap_or(""),
                parts.next().unwrap_or(""),
                parts.next().unwrap_or(""),
            );
            let owned = manifest.owned_namespaces().any(|owned| owned == namespace);
            if !self.from_admission {
                check_namespace(name, manifest)?;
            }
            let Some(decl) = (owned && top == "data")
                .then(|| rest.strip_prefix("sunrise/"))
                .flatten()
            else {
                continue;
            };
            // A server package's registry declaration refuses the whole package by name
            // (`server_only_refusal`), so it is not parsed: a malformed one must not turn
            // that refusal into a skipped content error.
            if manifest.side == crate::manifest::Side::Server {
                continue;
            }
            let Some((kind, path)) = decl.split_once('/').and_then(|(folder, path)| {
                Some((FileKind::from_folder(folder)?, path.strip_suffix(".json")?))
            }) else {
                return Err(format!(
                    "{name}: data/{id}/sunrise/ holds block/, item/, mob_effect/, creative_tab/, fluid/, fluid_interaction/, entity_type/ and menu/*.json only"
                ));
            };
            let rows = match kind {
                FileKind::Block => &mut blocks,
                FileKind::Item => &mut items,
                FileKind::MobEffect => &mut effects,
                FileKind::CreativeTab => &mut tabs,
                FileKind::Fluid => &mut fluids,
                FileKind::FluidInteraction => &mut interactions,
                FileKind::EntityType => &mut types,
                FileKind::Menu => &mut menus,
            };
            rows.push((format!("{namespace}:{path}"), name.clone()));
        }
        let reader = self.source.reader();
        let mut ids = BTreeSet::new();
        for (block, name) in blocks {
            let file: BlockFile =
                serde_json::from_slice(&reader.read(&name)?).map_err(|e| format!("{name}: {e}"))?;
            if file
                .connects_to
                .as_ref()
                .is_some_and(|t| !t.starts_with('#'))
            {
                return Err(format!(
                    "{name}: connects_to is a block tag, #namespace:path"
                ));
            }
            let block = Block {
                id: block,
                template: file.template,
                properties: file.properties,
                shapes: file.shapes,
                collision_shapes: file.collision_shapes,
                outline_context_shapes: file.outline_context_shapes,
                collision_context_shapes: file.collision_context_shapes,
                interaction_shapes: file.interaction_shapes,
                shapes_from_block_entity: file.shapes_from_block_entity,
                context_collision: file.context_collision,
                state_light: file.state_light,
                state_properties: file.state_properties,
                default_state: file.default_state,
                item: file.item,
                own_item_name: !file.use_block_description_prefix,
                open_sound: file.open_sound,
                close_sound: file.close_sound,
                block_item: file.block_item,
                pick_item: file.pick_item,
                components: file.components,
                suspicious_stew_effects: file.suspicious_stew_effects,
                connects_to: file
                    .connects_to
                    .map(|t| t.trim_start_matches('#').to_owned()),
                thickness: file.thickness,
                wood_type: file.wood_type,
                wall_block: file.wall_block,
                sign_text_color: file.sign_text_color,
                sign_text_glow: file.sign_text_glow,
                base_block: file.base_block,
            };
            check_block(&block).map_err(|e| format!("{name}: {e}"))?;
            if block.item {
                ids.insert(block.id.clone());
            }
            self.blocks.push(block);
        }
        // After every block, so a wall form may be declared in a later file.
        if let Some((id, reason)) =
            sign_wall_refusals(&self.blocks.iter().collect::<Vec<_>>()).next()
        {
            return Err(format!("{id}: {reason}"));
        }
        for (item, name) in items {
            let file: ItemFile =
                serde_json::from_slice(&reader.read(&name)?).map_err(|e| format!("{name}: {e}"))?;
            let item = Item {
                id: item,
                template: file.template,
                max_stack_size: file.max_stack_size,
                rarity: file.rarity,
                components: file.components,
                crafting_remainder: file.crafting_remainder,
                block: file.block,
                arm_pose: file.arm_pose,
            };
            check_item(&item).map_err(|e| format!("{name}: {e}"))?;
            if !ids.insert(item.id.clone()) {
                return Err(format!("{name}: {} is also a block's item", item.id));
            }
            self.items.push(item);
        }
        for (effect, name) in effects {
            let file: MobEffect =
                serde_json::from_slice(&reader.read(&name)?).map_err(|e| format!("{name}: {e}"))?;
            if file.color > 0xFF_FFFF {
                return Err(format!("{name}: color is RGB, 0 to 16777215 (0xFFFFFF)"));
            }
            file.check_attribute_modifiers()
                .map_err(|e| format!("{name}: {e}"))?;
            self.effects.push(MobEffect { id: effect, ..file });
        }
        for (tab, name) in tabs {
            let file: CreativeTabFile =
                serde_json::from_slice(&reader.read(&name)?).map_err(|e| format!("{name}: {e}"))?;
            let id_like = |s: &str| {
                s.split_once(':')
                    .is_some_and(|(n, p)| !n.is_empty() && !p.is_empty())
            };
            if !id_like(&file.icon) || !file.items.iter().all(|i| id_like(i)) {
                return Err(format!(
                    "{name}: icon and items are item ids (namespace:path)"
                ));
            }
            self.creative_tabs.push(CreativeTab {
                id: tab,
                title: file.title,
                icon: file.icon,
                items: file.items,
            });
        }
        for (fluid, name) in fluids {
            let file: FluidFile =
                serde_json::from_slice(&self.file(&name)?).map_err(|e| format!("{name}: {e}"))?;
            if file.sprite.as_deref().is_some_and(|s| !namespaced_id(s)) {
                return Err(format!(
                    "{name}: sprite is an atlas sprite id (namespace:path)"
                ));
            }
            // `FluidType.Properties.viscosity` throws for a negative one; the others take any.
            if file.viscosity.is_some_and(|viscosity| viscosity < 0) {
                return Err(format!("{name}: viscosity is never negative"));
            }
            let defaults = FluidMeasures::default();
            let look = FluidLook {
                id: fluid,
                sprite: file.sprite,
                tint: file.tint,
                world: file.world,
                bucket_item: file.bucket_item,
                measures: FluidMeasures {
                    temperature: file.temperature.unwrap_or(defaults.temperature),
                    density: file.density.unwrap_or(defaults.density),
                    viscosity: file.viscosity.unwrap_or(defaults.viscosity),
                    light_level: file.light_level.unwrap_or(defaults.light_level),
                },
            };
            look.check().map_err(|e| format!("{name}: {e}"))?;
            if self.fluids.iter().any(|known| {
                known.id == look.id
                    || (known.world.is_some() && known.flowing_id() == look.id)
                    || (look.world.is_some()
                        && (look.flowing_id() == known.id
                            || (known.world.is_some() && look.flowing_id() == known.flowing_id())))
            }) {
                return Err(format!("{name}: fluid name declared twice"));
            }
            if let Some(block) = look.world_block() {
                if self.blocks.iter().any(|known| known.id == block.id) {
                    return Err(format!("{name}: block {} is declared twice", block.id));
                }
                self.blocks.push(block);
            }
            if let Some(item) = look.world_bucket() {
                if self.items.iter().any(|known| known.id == item.id) {
                    return Err(format!("{name}: item {} is declared twice", item.id));
                }
                self.items.push(item);
            }
            self.fluids.push(look);
        }
        for (_, name) in interactions {
            let file: FluidInteraction =
                serde_json::from_slice(&self.file(&name)?).map_err(|e| format!("{name}: {e}"))?;
            file.check().map_err(|e| format!("{name}: {e}"))?;
            self.fluid_interactions.push(file);
        }
        for (kind, name) in types {
            let file: EntityType =
                serde_json::from_slice(&reader.read(&name)?).map_err(|e| format!("{name}: {e}"))?;
            file.sounds.validate(&kind)?;
            if !ENTITY_TEMPLATES.contains(&file.template.as_str()) {
                return Err(format!(
                    "{name}: template {:?} is not one of {ENTITY_TEMPLATES:?}",
                    file.template
                ));
            }
            if file.is_living() {
                // a mob is spawned, not thrown: nothing throws it, and its
                // attack is its `minecraft:attack_damage`.
                if file.item.is_some() || file.damage != 0.0 {
                    return Err(format!(
                        "{name}: a living type has no item or damage (its template is {})",
                        file.template
                    ));
                }
                for (attribute, value) in &file.attributes {
                    let id_like = attribute
                        .split_once(':')
                        .is_some_and(|(n, p)| !n.is_empty() && !p.is_empty());
                    if !id_like {
                        return Err(format!(
                            "{name}: attribute {attribute:?} is an attribute id (namespace:path)"
                        ));
                    }
                    if !value.is_finite() {
                        return Err(format!("{name}: attribute {attribute} is a finite number"));
                    }
                }
                for pt in &file.passenger_attachments {
                    if !pt.iter().all(|f| f.is_finite()) {
                        return Err(format!("{name}: passenger attachments are finite numbers"));
                    }
                }
                self.entity_types.push(EntityType { id: kind, ..file });
                continue;
            }
            file.validate_thrown_overrides(&name)?;
            let Some(item) = &file.item else {
                return Err(format!(
                    "{name}: a thrown type names the item that throws it"
                ));
            };
            // its own item, so no vanilla item loses its use to it.
            if !ids.contains(item) {
                return Err(format!(
                    "{name}: item {item:?} is not one of this mod's items"
                ));
            }
            if self
                .entity_types
                .iter()
                .any(|t| t.item.as_ref() == Some(item))
            {
                return Err(format!(
                    "{name}: item {item:?} already throws another entity type"
                ));
            }
            if !(file.damage.is_finite() && file.damage >= 0.0) {
                return Err(format!("{name}: damage is 0 or more"));
            }
            self.entity_types.push(EntityType { id: kind, ..file });
        }
        for (menu, name) in menus {
            let bytes = reader.read(&name)?;
            let source = format!("{name} ({menu})");
            let file: MenuType =
                serde_json::from_slice(&bytes).map_err(|e| format!("{source}: {e}"))?;
            validate_menu_type(&file).map_err(|reason| format!("{source}: {reason}"))?;
            self.menu_types.push(MenuType { id: menu, ..file });
        }
        Ok(())
    }
}

/// Structured refusal kind for validating a mod's `menu/<name>.json` declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MenuTypeRefusal {
    TotalSlotsExceeded {
        total_slots: usize,
    },
    OwnedSlotsInvalidLimits,
    OwnedSlotsWithCrafting,
    ResultOutOfRange {
        slot: u16,
        slots: u16,
    },
    DuplicateResult(u16),
    InputOutOfRange {
        slot: u16,
        slots: u16,
    },
    DuplicateOrResultInput(u16),
    CraftingInvalidSize,
    CraftingOutOfRange {
        grid: u16,
        end: u32,
        result: u16,
        slots: u16,
    },
    CraftingResultInGrid,
    ResultsListCraftingSlot,
    InputsListCraftingSlot,
    LinksOutOfRange {
        range: std::ops::Range<u32>,
        slots: u16,
    },
    ResultsListLinkedSlot,
    LinksOverlapCrafting,
}

impl std::fmt::Display for MenuTypeRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TotalSlotsExceeded { total_slots } => {
                write!(
                    f,
                    "total menu slots ({total_slots}) exceed {MAX_MENU_SLOTS}"
                )
            }
            Self::OwnedSlotsInvalidLimits => {
                write!(
                    f,
                    "owned_slots must give one maximum in 1..=99 per container slot"
                )
            }
            Self::OwnedSlotsWithCrafting => {
                write!(f, "owned_slots cannot be combined with crafting")
            }
            Self::ResultOutOfRange { slot, slots } => {
                write!(
                    f,
                    "result slot {slot} is outside the {slots} container slots"
                )
            }
            Self::DuplicateResult(slot) => write!(f, "result {slot} is listed twice"),
            Self::InputOutOfRange { slot, slots } => {
                write!(f, "input {slot} is outside the {slots} container slots")
            }
            Self::DuplicateOrResultInput(slot) => {
                write!(f, "input {slot} is listed twice or is a result")
            }
            Self::CraftingInvalidSize => write!(f, "crafting size is 2 or 3"),
            Self::CraftingOutOfRange {
                grid,
                end,
                result,
                slots,
            } => write!(
                f,
                "crafting grid {grid}..{end} and result slot {result} must be within the {slots} container slots"
            ),
            Self::CraftingResultInGrid => write!(f, "the crafting result is in its grid"),
            Self::ResultsListCraftingSlot => {
                write!(
                    f,
                    "results lists a crafting slot (the result is take-only already)"
                )
            }
            Self::InputsListCraftingSlot => write!(f, "inputs lists a crafting slot"),
            Self::LinksOutOfRange { range, slots } => write!(
                f,
                "linked slots {range:?} must be nonempty and within the {slots} container slots"
            ),
            Self::ResultsListLinkedSlot => write!(f, "results lists a linked slot"),
            Self::LinksOverlapCrafting => write!(f, "links overlap the crafting slots"),
        }
    }
}

pub(crate) fn validate_menu_type(file: &MenuType) -> Result<(), MenuTypeRefusal> {
    let total_slots = usize::from(file.slots) + if file.player_inventory { 36 } else { 0 };
    if total_slots > usize::from(MAX_MENU_SLOTS) {
        return Err(MenuTypeRefusal::TotalSlotsExceeded { total_slots });
    }
    if let Some(limits) = &file.owned_slots
        && (limits.len() != usize::from(file.slots)
            || limits.iter().any(|limit| !(1..=99).contains(limit)))
    {
        return Err(MenuTypeRefusal::OwnedSlotsInvalidLimits);
    }
    if file.owned_slots.is_some() && file.crafting.is_some() {
        return Err(MenuTypeRefusal::OwnedSlotsWithCrafting);
    }
    let mut seen = BTreeSet::new();
    for &r in &file.results {
        if r >= file.slots {
            return Err(MenuTypeRefusal::ResultOutOfRange {
                slot: r,
                slots: file.slots,
            });
        }
        if !seen.insert(r) {
            return Err(MenuTypeRefusal::DuplicateResult(r));
        }
    }
    for &input in &file.inputs {
        if input >= file.slots {
            return Err(MenuTypeRefusal::InputOutOfRange {
                slot: input,
                slots: file.slots,
            });
        }
        if !seen.insert(input) {
            return Err(MenuTypeRefusal::DuplicateOrResultInput(input));
        }
    }
    if let Some(c) = file.crafting {
        let end = u32::from(c.grid) + u32::from(c.size) * u32::from(c.size);
        if !matches!(c.size, 2 | 3) {
            return Err(MenuTypeRefusal::CraftingInvalidSize);
        }
        if end > u32::from(file.slots) || c.result >= file.slots {
            return Err(MenuTypeRefusal::CraftingOutOfRange {
                grid: c.grid,
                end,
                result: c.result,
                slots: file.slots,
            });
        }
        if (u32::from(c.grid)..end).contains(&u32::from(c.result)) {
            return Err(MenuTypeRefusal::CraftingResultInGrid);
        }
        let is_crafting_slot =
            |&s: &u16| s == c.result || (u32::from(c.grid)..end).contains(&u32::from(s));
        if file.results.iter().any(is_crafting_slot) {
            return Err(MenuTypeRefusal::ResultsListCraftingSlot);
        }
        if file.inputs.iter().any(is_crafting_slot) {
            return Err(MenuTypeRefusal::InputsListCraftingSlot);
        }
    }
    if let Some(l) = file.links {
        let linked = l.range();
        if l.count == 0 || linked.end > u32::from(file.slots) {
            return Err(MenuTypeRefusal::LinksOutOfRange {
                range: linked,
                slots: file.slots,
            });
        }
        if file.results.iter().any(|&r| linked.contains(&u32::from(r))) {
            return Err(MenuTypeRefusal::ResultsListLinkedSlot);
        }
        if let Some(c) = file.crafting {
            let end = u32::from(c.grid) + u32::from(c.size) * u32::from(c.size);
            if linked.contains(&u32::from(c.result))
                || (u32::from(c.grid)..end).any(|s| linked.contains(&s))
            {
                return Err(MenuTypeRefusal::LinksOverlapCrafting);
            }
        }
    }
    Ok(())
}

/// Namespaced component types, removals as `{}`, and none of `fields`,
/// which the file sets with its own keys.
fn check_components(
    components: &BTreeMap<String, serde_json::Value>,
    fields: &[&str],
) -> Result<(), String> {
    for (key, value) in components {
        let (removed, kind) = match key.strip_prefix('!') {
            Some(kind) => (true, kind),
            None => (false, key.as_str()),
        };
        if !namespaced_id(kind) {
            return Err(format!("component {key:?} must be namespaced and valid"));
        }
        if let Some(own) = kind
            .strip_prefix("minecraft:")
            .filter(|k| fields.contains(k))
        {
            return Err(format!("component {key:?}: set {own} instead"));
        }
        if removed && !value.as_object().is_some_and(|o| o.is_empty()) {
            return Err(format!("component {key:?}: a removal's value is {{}}"));
        }
    }
    Ok(())
}

/// A declaration checked without consulting other entries or loaded resources.
#[derive(Clone)]
pub struct LocalDeclaration<T>(T);

impl<T> LocalDeclaration<T> {
    pub fn get(&self) -> &T {
        &self.0
    }
}

impl Block {
    pub fn validate_local(self) -> Result<LocalDeclaration<Self>, String> {
        check_block(&self)?;
        Ok(LocalDeclaration(self))
    }
}

impl Item {
    pub fn validate_local(self) -> Result<LocalDeclaration<Self>, String> {
        check_item(&self)?;
        Ok(LocalDeclaration(self))
    }
}

/// An item's fields, as a file or code gives them.
fn check_item(item: &Item) -> Result<(), String> {
    if !vanilla_id(&item.template) {
        return Err(format!(
            "template {:?} is not a vanilla item (minecraft:...)",
            item.template
        ));
    }
    if item.block.as_ref().is_some_and(|id| !namespaced_id(id)) {
        return Err("block must be a namespaced block id".into());
    }
    if item.max_stack_size.is_some_and(|n| !(1..=99).contains(&n)) {
        return Err("max_stack_size is 1 to 99".into());
    }
    if let Some(rest) = &item.crafting_remainder {
        if !namespaced_id(rest) {
            return Err(format!(
                "crafting_remainder {rest:?} must be a namespaced item id"
            ));
        }
        if rest == "minecraft:air" {
            return Err(format!(
                "crafting_remainder {rest:?} is no remainder (leave it out)"
            ));
        }
    }
    check_components(&item.components, &["max_stack_size", "rarity"])
}

/// A block's fields, as a file or code gives them (`connects_to` without
/// its `#`).
fn check_block(file: &Block) -> Result<(), String> {
    if !vanilla_id(&file.template) {
        return Err(format!(
            "template {:?} is not a vanilla block (minecraft:...)",
            file.template
        ));
    }
    for shapes in file
        .shapes
        .iter()
        .chain(file.collision_shapes.iter())
        .chain(file.outline_context_shapes.values())
        .chain(file.collision_context_shapes.values())
        .chain(file.interaction_shapes.iter())
    {
        shapes.check()?;
    }
    if let Some(light) = &file.state_light {
        light.check()?;
    }
    for (name, property) in &file.state_properties {
        if name == "waterlogged" && !matches!(property, StateProperty::Boolean) {
            return Err("state_properties.waterlogged must have type boolean".into());
        }
        if !state_name(name) {
            return Err(format!(
                "state property {name:?} is lower case letters, digits and underscores"
            ));
        }
        let values = property.values();
        if values.is_empty() {
            return Err(format!("state property {name:?} needs a value"));
        }
        let mut seen = BTreeSet::new();
        for value in values {
            if !state_name(value) {
                return Err(format!(
                    "state property {name:?}: value {value:?} is lower case letters, digits and underscores"
                ));
            }
            if !seen.insert(value) {
                return Err(format!("state property {name:?}: value {value:?} repeats"));
            }
        }
    }
    for key in file.default_state.keys() {
        if !state_name(key) {
            return Err(format!(
                "default_state key {key:?} is lower case letters, digits and underscores"
            ));
        }
    }
    let p = &file.properties;
    if let Some(RandomTicks::States { property, values }) = &p.random_ticks {
        let declared = file
            .state_properties
            .get(property)
            .map(StateProperty::values);
        if declared.is_none_or(|all| {
            values.is_empty() || values.iter().any(|v| !all.contains(&v.as_str()))
        }) {
            return Err(format!(
                "random_ticks needs values of the block's own state property {property:?} (state_properties)"
            ));
        }
    }
    for (key, value, max) in [
        ("max_horizontal_offset", p.max_horizontal_offset, 0.25),
        ("max_vertical_offset", p.max_vertical_offset, 0.2),
    ] {
        if value.is_some_and(|v| !v.is_finite() || !(0.0..=max).contains(&v)) {
            return Err(format!("{key} is a finite number from 0 to {max}"));
        }
    }
    let finite = |v: Option<f32>| v.is_none_or(f32::is_finite);
    if !finite(p.hardness) || !finite(p.explosion_resistance) || !finite(p.friction) {
        return Err("hardness, explosion_resistance and friction are numbers".into());
    }
    if p.hardness.is_some_and(|h| h < 0.0 && h != -1.0) {
        return Err("hardness is 0 or more, or -1 (unbreakable)".into());
    }
    if p.explosion_resistance.is_some_and(|r| r < 0.0) {
        return Err("explosion_resistance is 0 or more".into());
    }
    if p.light.is_some_and(|l| l > 15) {
        return Err("light is 0 to 15".into());
    }
    if p.map_color.is_some_and(|c| c > 61) {
        return Err("map_color is a MapColor id, 0 to 61".into());
    }
    if p.sounds.as_deref().is_some_and(|s| !vanilla_id(s)) {
        return Err("sounds names a vanilla block (minecraft:...)".into());
    }
    if let Some(rules) = &p.skip_rendering {
        face_rules::check_face_rules("skip_rendering", rules)?;
    }
    if !p.hides_neighbor_face.is_empty() {
        face_rules::check_face_rules("hides_neighbor_face", &p.hides_neighbor_face)?;
    }
    if !file.item && !file.components.is_empty() {
        return Err("item is false, so the block has no item to take components".into());
    }
    check_components(&file.components, &[])?;
    // A block file has no `max_stack_size` / `rarity` keys, so those two take the same checks here.
    if let Some(n) = file.components.get("minecraft:max_stack_size")
        && !n.as_u64().is_some_and(|n| (1..=99).contains(&n))
    {
        return Err("component \"minecraft:max_stack_size\" is an integer 1 to 99".into());
    }
    if let Some(r) = file.components.get("minecraft:rarity")
        && serde_json::from_value::<Rarity>(r.clone()).is_err()
    {
        return Err("component \"minecraft:rarity\" is common, uncommon, rare or epic".into());
    }
    if let Some(tag) = &file.connects_to {
        if file.template != "minecraft:chorus_plant" {
            return Err("connects_to needs template minecraft:chorus_plant (a PipeBlock)".into());
        }
        let id = tag.as_str();
        let valid = |s: &str| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_-./".contains(c))
        };
        if !id
            .split_once(':')
            .is_some_and(|(ns, path)| valid(ns) && valid(path))
        {
            return Err("connects_to is a block tag, #namespace:path".into());
        }
    }
    match file.thickness {
        Some(_) if file.connects_to.is_none() => {
            return Err("thickness is a connecting block's (connects_to)".into());
        }
        Some(t) if !(2.0..=14.0).contains(&t) => return Err("thickness is 2 to 14 pixels".into()),
        Some(_) if file.shapes.is_some() => {
            return Err("thickness sets the shapes, so a block has one or the other".into());
        }
        _ => {}
    }
    if let Some(base) = &file.base_block {
        if file.template != "minecraft:farmland" {
            return Err("base_block needs template minecraft:farmland".into());
        }
        if !namespaced_id(base) {
            return Err("base_block is a block id, namespace:path".into());
        }
    }
    // Every vanilla `SignBlock`'s id ends in `_sign`, and no other block's does.
    let sign = file.template.ends_with("_sign");
    if let Some(wood) = &file.wood_type {
        if !sign {
            return Err("wood_type needs a vanilla sign template".into());
        }
        let (namespace, path) = wood.split_once(':').unwrap_or(("minecraft", wood));
        if !namespaced_id(&format!("{namespace}:{path}"))
            || check_path(&format!("assets/{namespace}/{path}")).is_err()
        {
            return Err("wood_type is a name, or namespace:path, in lower case".into());
        }
    }
    if let Some(color) = &file.sign_text_color {
        if !sign {
            return Err("sign_text_color needs a vanilla sign template".into());
        }
        if !DYE_COLORS.contains(&color.as_str()) {
            return Err("sign_text_color is a DyeColor name, such as white or light_blue".into());
        }
    }
    if file.sign_text_glow.is_some() && !sign {
        return Err("sign_text_glow needs a vanilla sign template".into());
    }
    for (key, sound) in [
        ("open_sound", &file.open_sound),
        ("close_sound", &file.close_sound),
    ] {
        let Some(sound) = sound else { continue };
        if !file.template.ends_with("_fence_gate") {
            return Err(format!(
                "{key} needs a fence gate template (minecraft:*_fence_gate), not {}",
                file.template
            ));
        }
        let sites = match sound {
            GateSound::All(sound) => [("", Some(sound)), ("", None), ("", None)],
            GateSound::Sites {
                on_use,
                on_explosion,
                on_redstone,
            } => [
                (".use", on_use.as_ref()),
                (".explosion", on_explosion.as_ref()),
                (".redstone", on_redstone.as_ref()),
            ],
        };
        for (site, sound) in sites {
            if let Some(sound) = sound
                && !namespaced_id(sound)
            {
                return Err(format!(
                    "{key}{site} {sound:?} is a sound event id, namespace:path"
                ));
            }
        }
    }
    if let Some(item) = &file.block_item {
        if item == "none" {
            return Err("block_item is an item id; a block with no item is \"item\": false".into());
        }
        if file.item {
            return Err(
                "block_item is the item this block yields instead of its own, and item is true"
                    .into(),
            );
        }
        if !namespaced_id(item) {
            return Err("block_item is an item id, namespace:path".into());
        }
    }
    if let Some(item) = &file.pick_item
        && !namespaced_id(item)
    {
        return Err("pick_item is an item id, namespace:path".into());
    }
    if let Some(wall) = &file.wall_block {
        if !sign || file.template.contains("_wall_") {
            return Err("wall_block needs a standing or ceiling sign template".into());
        }
        if !file.item {
            return Err("wall_block is where the item places the block, and item is false".into());
        }
        if !namespaced_id(wall) {
            return Err("wall_block is a block id, namespace:path".into());
        }
    }
    Ok(())
}

/// A sign's `wall_block` is one of the mod's own blocks: the wall form of its
/// kind, with no item of its own and no other sign. Checked here, a pair that
/// doesn't fit leaves out its mod alone.
fn sign_wall_refusals<'a>(blocks: &'a [&'a Block]) -> impl Iterator<Item = (&'a str, String)> + 'a {
    let mut paired = BTreeMap::new();
    blocks.iter().filter_map(move |sign| {
        let wall = sign.wall_block.as_ref()?;
        let id = sign.id.as_str();
        let form = if sign.template.ends_with("_hanging_sign") {
            "_wall_hanging_sign"
        } else {
            "_wall_sign"
        };
        let Some(block) = blocks.iter().find(|b| b.id == *wall) else {
            return Some((
                id,
                format!("wall_block {wall} is not one of this mod's blocks"),
            ));
        };
        if !block.template.ends_with(form) {
            return Some((
                id,
                format!("wall_block {wall} needs a template ending in {form}"),
            ));
        }
        if block.block_item.is_some() {
            return Some((
                id,
                format!("wall_block {wall} has this sign's item, so it declares no block_item"),
            ));
        }
        if block.pick_item.is_some() {
            return Some((
                id,
                format!("wall_block {wall} picks this sign's item, so it declares no pick_item"),
            ));
        }
        if block.item {
            return Some((
                id,
                format!("wall_block {wall} has this sign's item, so its own \"item\" is false"),
            ));
        }
        fn text(b: &Block) -> (&str, bool) {
            (
                b.sign_text_color.as_deref().unwrap_or("black"),
                b.sign_text_glow.unwrap_or(false),
            )
        }
        if text(sign) != text(block) {
            return Some((
                id,
                format!("wall_block {wall} declares a different sign_text_color or sign_text_glow"),
            ));
        }
        match paired.entry(wall) {
            std::collections::btree_map::Entry::Occupied(other) => Some((
                id,
                format!("wall_block {wall} is already paired with {}", other.get()),
            )),
            std::collections::btree_map::Entry::Vacant(first) => {
                first.insert(id);
                None
            }
        }
    })
}

impl Source {
    fn reader(&self) -> FileReader<'_> {
        FileReader(match self {
            Self::Dir(root) => FileLocation::Dir(root),
            Self::Zip { archive, .. } => FileLocation::Zip(archive),
            Self::Code => FileLocation::Dir(Path::new("")),
        })
    }
}

fn open_zip(path: &Path) -> Result<zip::ZipArchive<std::fs::File>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    zip::ZipArchive::new(file).map_err(|e| e.to_string())
}

/// Files under `dir`, relative to `root` with `/` separators. Symbolic links
/// are not followed: a mod folder holds its own files.
pub(crate) fn folder_files(root: &Path) -> Result<Vec<(String, u64)>, String> {
    let mut out = Vec::new();
    for top in ["assets", "data"] {
        walk(root, &root.join(top), &mut out, Some(MAX_FILES))?;
    }
    Ok(out)
}

/// The files under a package folder's `runtime/`, as `(name below it, bytes)`. Symbolic links and
/// other non-files are refused, not followed.
pub(crate) fn runtime_folder_files(root: &Path) -> Result<Vec<(String, u64)>, String> {
    let runtime_root = root.join(crate::manifest::RUNTIME_DIR);
    let mut out = Vec::new();
    walk(&runtime_root, &runtime_root, &mut out, None)?;
    Ok(out)
}

fn walk(
    root: &Path,
    dir: &Path,
    out: &mut Vec<(String, u64)>,
    max_files: Option<usize>,
) -> Result<(), String> {
    match std::fs::symlink_metadata(dir) {
        Ok(m) if !m.is_dir() => {
            return Err(format!(
                "{}: unsupported file type in a content mod",
                dir.display()
            ));
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    }
    let read = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in read {
        let path = entry.map_err(|e| e.to_string())?.path();
        let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if meta.is_dir() {
            walk(root, &path, out, max_files)?;
        } else if meta.is_file() {
            let rel = path.strip_prefix(root).map_err(|e| e.to_string())?;
            let parts: Option<Vec<&str>> = rel.iter().map(|p| p.to_str()).collect();
            let name = parts.ok_or_else(|| format!("{} is not UTF-8", rel.display()))?;
            out.push((name.join("/"), meta.len()));
        } else {
            return Err(format!(
                "{}: unsupported file type in a content mod",
                path.display()
            ));
        }
        if let Some(max) = max_files
            && out.len() > max
        {
            return Err(format!("more than {max} content files"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Package;

    #[test]
    fn plain_content_files_over_the_size_limit_are_refused() {
        let dir = scratch("plain-size-limit");
        let name = "assets/ruby/textures/block/huge.png";
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::File::create(&path)
            .unwrap()
            .set_len(MAX_FILE + 1)
            .unwrap();
        let reader = FileReader(FileLocation::Dir(&dir));
        let error = reader.path(name).unwrap_err();
        assert!(error.contains("is over"), "{error}");
        let error = reader.read(name).unwrap_err();
        assert!(error.to_string().contains("is over"), "{error}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    const MANIFEST_TEXT: &str = r#"
id = "ruby"
name = "Ruby"
version = "0.1.0"
api = "0.2"
side = "both"
capabilities = ["registries"]
"#;
    const BLOCK: &[u8] = br#"{"template": "minecraft:iron_block",
        "properties": {"hardness": 4.0, "light": 3, "sounds": "minecraft:amethyst_block"}}"#;
    const ITEM: &[u8] = br#"{"template": "minecraft:diamond", "rarity": "uncommon"}"#;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sunrise-content-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
    #[test]
    fn shape_tables_validate_tuples_defaults_and_collision() {
        let content = smod("shape-tuples", MANIFEST_TEXT, &[(
            "data/ruby/sunrise/block/a.json",
            br#"{"template":"minecraft:stone","shapes":{"properties":["facing","count"],"values":{"north,2":[[0,0,0,1,0.5,1]]},"default":[[0,0,0,1,1,1]]},"collision_shapes":{"default":[]}}"#,
        )]).unwrap();
        let block = &content.blocks[0];
        assert_eq!(
            block.shapes.as_ref().unwrap().properties,
            ["facing", "count"]
        );
        assert_eq!(
            block.collision_shapes.as_ref().unwrap().default,
            Some(vec![])
        );
        assert!(
            serde_json::from_str::<BlockShapes>(r#"{"property":"axis","default":[]}"#).is_err()
        );
        for value in [
            r#"{"properties":["axis","axis"],"default":[]}"#,
            r#"{"properties":["Axis"],"default":[]}"#,
            r#"{"properties":["axis","count"],"values":{"x":[]}}"#,
            r#"{"properties":["axis"],"values":{"":[]}}"#,
            r#"{"values":{"x":[]},"default":[]}"#,
            r#"{"default":[[1,0,0,0,1,1]]}"#,
        ] {
            let shapes: BlockShapes = serde_json::from_str(value).unwrap();
            assert!(shapes.check().is_err(), "{value}");
            let declaration =
                format!(r#"{{"template":"minecraft:stone","collision_shapes":{value}}}"#);
            assert!(
                smod(
                    "bad-collision",
                    MANIFEST_TEXT,
                    &[("data/ruby/sunrise/block/a.json", declaration.as_bytes(),)]
                )
                .is_err(),
                "{value}"
            );
        }
        let many = BlockShapes(StateValues {
            property: String::new(),
            properties: vec![],
            values: BTreeMap::new(),
            default: Some(vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]; 100]),
        });
        assert_eq!(many.check(), Ok(()));
        let mut nonfinite = many;
        nonfinite.default = Some(vec![[f64::NAN; 6]]);
        assert!(nonfinite.check().is_err());
    }

    fn smod(name: &str, manifest: &str, files: &[(&str, &[u8])]) -> Result<Content, String> {
        let dir = scratch(name);
        let path = dir.join("m.smod");
        std::fs::write(&path, Package::write_files(manifest, files)?).unwrap();
        let package = Package::read(&path);
        let content = package.and_then(|p| Content::read(&path, &p.manifest));
        std::fs::remove_dir_all(&dir).unwrap();
        content
    }

    #[test]
    fn a_folder_and_its_smod_list_and_read_the_same_content_files() {
        let files: &[(&str, &[u8])] = &[
            ("data/ruby/lumen/b.luau", b"return 2"),
            ("assets/ruby/lang/en_us.json", b"{}"),
            ("data/ruby/lumen/a.luau", b"return 1"),
        ];
        let dir = scratch("content-source");
        let folder = dir.join("ruby");
        for (name, bytes) in files {
            let path = folder.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        std::fs::write(folder.join(crate::manifest::MANIFEST), MANIFEST_TEXT).unwrap();
        let archive = dir.join("ruby.smod");
        std::fs::write(
            &archive,
            Package::write_files(MANIFEST_TEXT, files).unwrap(),
        )
        .unwrap();
        for path in [&folder, &archive] {
            let source = ContentSource::open(path).unwrap();
            assert_eq!(
                source.names().as_ref(),
                [
                    "assets/ruby/lang/en_us.json",
                    "data/ruby/lumen/a.luau",
                    "data/ruby/lumen/b.luau"
                ],
                "{}",
                path.display()
            );
            let reader = source.reader();
            assert_eq!(reader.read("data/ruby/lumen/b.luau").unwrap(), b"return 2");
            assert!(reader.read("data/ruby/lumen/../lumen/a.luau").is_err());
            assert!(matches!(
                reader.read("data/ruby/lumen/c.luau"),
                Err(ReadError::NotFound(_))
            ));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_open_package_keeps_reading_the_archive_it_listed() {
        let dir = scratch("content-archive-identity");
        let path = dir.join("ruby.smod");
        let replacement = dir.join("replacement.smod");
        let name = "data/ruby/lumen/main.luau";
        std::fs::write(
            &path,
            Package::write_files(MANIFEST_TEXT, &[(name, b"old")]).unwrap(),
        )
        .unwrap();
        let source = ContentSource::open(&path).unwrap();
        assert_eq!(source.names().as_ref(), [name]);
        std::fs::write(
            &replacement,
            Package::write_files(MANIFEST_TEXT, &[(name, b"new")]).unwrap(),
        )
        .unwrap();
        std::fs::rename(replacement, &path).unwrap();
        assert_eq!(source.reader().read(name).unwrap(), b"old");
        let current = ContentSource::open(&path).unwrap();
        assert_eq!(current.reader().read(name).unwrap(), b"new");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_declared_friction_is_any_finite_number() {
        let load = |value: &str| {
            let file =
                format!(r#"{{"template":"minecraft:stone","properties":{{"friction":{value}}}}}"#);
            smod(
                "friction-range",
                MANIFEST_TEXT,
                &[("data/ruby/sunrise/block/a.json", file.as_bytes())],
            )
        };
        for (value, expected) in [("3.0", 3.0), ("-0.5", -0.5)] {
            let content = load(value).unwrap();
            assert_eq!(content.blocks[0].properties.friction, Some(expected));
        }
        // Just above the midpoint of two neighbouring f32 values: rounded once, from the decimal text.
        let content = load("1.0000000596046448").unwrap();
        let friction = content.blocks[0].properties.friction.unwrap();
        assert_eq!(friction.to_bits(), 0x3f80_0001);
        // JSON has no NaN or infinity: a literal is a parse error, and so is a number past f32's
        // range. The finite check remains for declarations built in code.
        assert!(load("NaN").is_err());
        assert!(load("1e39").is_err());
    }

    #[test]
    fn state_light_checks_ranges_selectors_and_native_declarations() {
        let load = |name: &str, table: &str| {
            let declaration = format!(
                r#"{{"template":"minecraft:stone","properties":{{"light":3}},"state_light":{table}}}"#
            );
            smod(
                name,
                MANIFEST_TEXT,
                &[("data/ruby/sunrise/block/lamp.json", declaration.as_bytes())],
            )
        };
        let content = load(
            "state-light-valid",
            r#"{"properties":["facing","lit"],"values":{"north,true":15},"default":0}"#,
        )
        .unwrap();
        assert_eq!(content.blocks[0].properties.light, Some(3));
        let table = content.blocks[0].state_light.as_ref().unwrap();
        assert_eq!(table.default, Some(0));
        assert_eq!(table.values["north,true"], 15);
        for (i, table) in [
            r#"{"default":-1}"#,
            r#"{"default":16}"#,
            r#"{"default":256}"#,
            r#"{"default":2.5}"#,
            r#"{"default":true}"#,
            r#"{"default":"15"}"#,
            r#"{"properties":["axis","axis"],"default":0}"#,
            r#"{"property":"axis","properties":["axis"],"default":0}"#,
            r#"{"properties":["axis","lit"],"values":{"x":15},"default":0}"#,
            r#"{"values":{"x":15},"default":0}"#,
            r#"{}"#,
        ]
        .iter()
        .enumerate()
        {
            assert!(
                load(&format!("state-light-invalid-{i}"), table).is_err(),
                "{table}"
            );
        }
        let mut declared = Content::declared("ruby");
        let mut lamp = content.blocks[0].clone();
        lamp.state_light.as_mut().unwrap().default = Some(16);
        assert!(declared.declare(&[lamp], &[]).is_err());
        assert!(declared.blocks.is_empty());
    }

    #[test]
    fn place_block_checks_files_and_native_declarations() {
        for (i, target) in ["ruby:frame", "minecraft:stone", "other:frame"]
            .into_iter()
            .enumerate()
        {
            let text = format!(r#"{{"template":"minecraft:stick","block":"{target}"}}"#);
            let content = smod(
                &format!("place-block-{i}"),
                MANIFEST_TEXT,
                &[("data/ruby/sunrise/item/empty_frame.json", text.as_bytes())],
            )
            .unwrap();
            assert_eq!(content.items[0].block.as_deref(), Some(target));
            let mut declared = Content::declared("ruby");
            declared.declare(&[], &content.items).unwrap();
            assert_eq!(declared.items[0].block.as_deref(), Some(target));
        }
        for (i, target) in ["stone", "ruby:Bad", "ruby:"].into_iter().enumerate() {
            let text = format!(r#"{{"template":"minecraft:stick","block":"{target}"}}"#);
            let error = smod(
                &format!("bad-place-block-{i}"),
                MANIFEST_TEXT,
                &[("data/ruby/sunrise/item/empty_frame.json", text.as_bytes())],
            )
            .err()
            .unwrap();
            assert!(
                error.contains("block must be a namespaced block id"),
                "{error}"
            );
            let mut content = Content::declared("ruby");
            let errors = content
                .declare(
                    &[],
                    &[Item {
                        id: "ruby:empty_frame".into(),
                        template: "minecraft:stick".into(),
                        block: Some(target.into()),
                        ..Default::default()
                    }],
                )
                .unwrap_err();
            assert_eq!(errors.len(), 1);
            assert!(
                errors[0]
                    .1
                    .to_string()
                    .contains("block must be a namespaced block id")
            );
            assert!(content.items.is_empty());
        }

        // Referencing another namespace does not let this package write its files there.
        let foreign_path = "data/other/sunrise/item/empty_frame.json";
        let error = smod(
            "foreign-place-block-file",
            MANIFEST_TEXT,
            &[(
                foreign_path,
                br#"{"template":"minecraft:stick","block":"other:frame"}"#,
            )],
        )
        .err()
        .unwrap();
        assert!(
            error.contains(foreign_path) && error.contains("own namespaces"),
            "{error}"
        );
    }

    #[test]
    fn block_without_an_item_can_share_an_existing_item_key() {
        let mut content = Content::declared("ruby");
        content.items.push(Item {
            id: "ruby:lamp".into(),
            template: "minecraft:stick".into(),
            ..Item::default()
        });
        let block = Block {
            id: "ruby:lamp".into(),
            template: "minecraft:stone".into(),
            item: true,
            ..Block::default()
        };
        assert!(content.declare(std::slice::from_ref(&block), &[]).is_err());
        content
            .declare(
                &[Block {
                    item: false,
                    ..block
                }],
                &[],
            )
            .unwrap();
        assert_eq!(content.items[0].id, "ruby:lamp");
        assert_eq!(content.blocks[0].id, "ruby:lamp");
    }

    #[test]
    fn blocks_and_items_declared_in_code_pass_the_files_checks() {
        let block = |id: &str| Block {
            id: id.into(),
            template: "minecraft:iron_block".into(),
            item: true,
            ..Block::default()
        };
        let item = |id: &str| Item {
            id: id.into(),
            template: "minecraft:diamond".into(),
            ..Item::default()
        };
        let mut content = Content::declared("ruby");
        assert!(content.is_empty());
        let wrench = Item {
            crafting_remainder: Some("ruby:gear".into()),
            ..item("ruby:wrench")
        };
        content
            .declare(&[block("ruby:gear"), block("ruby:casing")], &[wrench])
            .unwrap();
        assert_eq!(content.blocks.len(), 2);
        assert_eq!(content.items[0].id, "ruby:wrench");
        // Each refusal adds nothing.
        let refused = |blocks: &[Block], items: &[Item]| {
            let mut c = Content::declared("ruby");
            c.declare(&[block("ruby:gear")], &[]).unwrap();
            let e = c.declare(blocks, items).unwrap_err()[0].1.to_string();
            assert_eq!((c.blocks.len(), c.items.len()), (1, 0), "{e}");
            e
        };
        for error in [
            refused(&[block("emerald:gear")], &[]),
            refused(&[block("minecraft:gear")], &[]),
            refused(&[], &[item("ruby")]),
        ] {
            assert!(
                error.contains("owned namespaces") && error.contains("ruby"),
                "{error}"
            );
        }
        assert!(refused(&[block("ruby:gear")], &[]).contains("declared twice"));
        assert!(refused(&[], &[item("ruby:gear")]).contains("block's item"));
        let mut content = Content::declared("ruby");
        content.declare(&[block("ruby:gear")], &[]).unwrap();
        let errors = content
            .declare(
                &[block("ruby:gear"), block("ruby:lamp")],
                &[item("ruby:lamp")],
            )
            .unwrap_err();
        assert_eq!(
            errors,
            [
                (
                    DeclarationKind::Block,
                    DeclarationError::Rejected {
                        id: "ruby:gear".into(),
                        reason: "declared twice".into(),
                    }
                ),
                (
                    DeclarationKind::Item,
                    DeclarationError::Rejected {
                        id: "ruby:lamp".into(),
                        reason: "declared twice, or is also a block's item".into(),
                    }
                ),
            ]
        );
        assert_eq!(content.blocks.len(), 1);
        assert!(content.items.is_empty());
        let modded = Block {
            template: "ruby:other".into(),
            ..block("ruby:casing")
        };
        assert!(refused(&[modded], &[]).contains("not a vanilla block"));
        let dim = Block {
            properties: BlockProperties {
                light: Some(16),
                ..BlockProperties::default()
            },
            ..block("ruby:lamp")
        };
        assert!(refused(&[dim], &[]).contains("light is 0 to 15"));
        let stack = Item {
            max_stack_size: Some(100),
            ..item("ruby:ingot")
        };
        assert!(refused(&[], &[stack]).contains("max_stack_size"));
        let rest = Item {
            crafting_remainder: Some("emerald:bad dust".into()),
            ..item("ruby:dust")
        };
        assert!(refused(&[block("ruby:pipe")], &[rest]).contains("crafting_remainder"));
        // A cable's tag comes bare from code.
        let cable = Block {
            template: "minecraft:chorus_plant".into(),
            connects_to: Some("ruby:cables".into()),
            ..block("ruby:cable")
        };
        content.declare(&[cable], &[]).unwrap();
    }

    #[test]
    fn a_connecting_block_file_names_its_tag_with_a_hash() {
        let cable = br#"{"template": "minecraft:chorus_plant", "connects_to": "ruby:cables"}"#;
        let files: &[(&str, &[u8])] = &[("data/ruby/sunrise/block/cable.json", cable)];
        let e = smod("bare-tag", MANIFEST_TEXT, files).unwrap_err();
        assert!(e.contains("#namespace:path"), "{e}");
    }

    #[test]
    fn content_only_mod_round_trips() {
        let files: &[(&str, &[u8])] = &[
            ("data/ruby/sunrise/block/ruby_block.json", BLOCK),
            ("data/ruby/sunrise/item/ruby.json", ITEM),
            ("data/ruby/recipe/ruby_block.json", b"{}"),
            ("data/minecraft/tags/block/mineable/pickaxe.json", b"{}"),
            ("data/c/tags/item/gems/ruby.json", b"{}"),
            ("assets/ruby/textures/block/ruby_block.png", b"png"),
            ("assets/minecraft/atlases/blocks.json", b"{}"),
        ];
        let content = smod("ok", MANIFEST_TEXT, files).unwrap();
        assert_eq!(content.files.len(), 7);
        assert_eq!(
            content.blocks,
            [Block {
                id: "ruby:ruby_block".into(),
                template: "minecraft:iron_block".into(),
                shapes: None,
                collision_shapes: None,
                outline_context_shapes: BTreeMap::new(),
                collision_context_shapes: BTreeMap::new(),
                interaction_shapes: None,
                shapes_from_block_entity: false,
                context_collision: None,
                state_light: None,
                state_properties: BTreeMap::new(),
                default_state: BTreeMap::new(),
                properties: BlockProperties {
                    hardness: Some(4.0),
                    light: Some(3),
                    sounds: Some("minecraft:amethyst_block".into()),
                    ..BlockProperties::default()
                },
                item: true,
                own_item_name: false,
                open_sound: None,
                close_sound: None,
                block_item: None,
                pick_item: None,
                components: BTreeMap::new(),
                suspicious_stew_effects: None,
                connects_to: None,
                thickness: None,
                wood_type: None,
                wall_block: None,
                sign_text_color: None,
                sign_text_glow: None,
                base_block: None,
            }]
        );
        assert_eq!(content.items[0].id, "ruby:ruby");
        assert_eq!(content.items[0].rarity, Some(Rarity::Uncommon));
        assert!(content.items[0].components.is_empty());
        assert_eq!(content.items[0].crafting_remainder, None);
    }

    #[test]
    fn items_name_the_block_they_place() {
        let file = "data/ruby/sunrise/item/seeds.json";
        let content = smod(
            "seeds",
            MANIFEST_TEXT,
            &[(
                file,
                br#"{"template":"minecraft:wheat_seeds","block":"ruby:crop"}"#,
            )],
        )
        .unwrap();
        assert_eq!(content.items[0].block.as_deref(), Some("ruby:crop"));
        let error = smod(
            "bad-block",
            MANIFEST_TEXT,
            &[(
                file,
                br#"{"template":"minecraft:wheat_seeds","block":"crop"}"#,
            )],
        )
        .unwrap_err();
        assert!(
            error.contains("block must be a namespaced block id"),
            "{error}"
        );
    }

    #[test]
    fn items_name_their_crafting_remainder() {
        let files: &[(&str, &[u8])] = &[
            (
                "data/ruby/sunrise/item/a_stew.json",
                br#"{"template": "minecraft:mushroom_stew", "crafting_remainder": "minecraft:bowl"}"#,
            ),
            (
                "data/ruby/sunrise/item/b_flask.json",
                br#"{"template": "minecraft:potion", "crafting_remainder": "ruby:c_flask"}"#,
            ),
            ("data/ruby/sunrise/item/c_flask.json", ITEM),
        ];
        let content = smod("remainder", MANIFEST_TEXT, files).unwrap();
        let rests: Vec<_> = content
            .items
            .iter()
            .map(|i| i.crafting_remainder.as_deref())
            .collect();
        assert_eq!(rests, [Some("minecraft:bowl"), Some("ruby:c_flask"), None]);
    }

    #[test]
    fn items_name_an_arm_pose() {
        let item = |pose: &str| {
            let text = format!(r#"{{"template": "minecraft:stick", "arm_pose": "{pose}"}}"#);
            let files: &[(&str, &[u8])] = &[("data/ruby/sunrise/item/a.json", text.as_bytes())];
            smod("arm-pose", MANIFEST_TEXT, files)
        };
        assert_eq!(
            item("toot_horn").unwrap().items[0].arm_pose,
            Some(ArmPose::TootHorn)
        );
        let refused = item("bow").unwrap_err();
        assert!(
            refused.contains("a.json") && refused.contains("bow"),
            "{refused}"
        );
        let files: &[(&str, &[u8])] = &[("data/ruby/sunrise/item/a.json", ITEM)];
        let plain = smod("arm-pose", MANIFEST_TEXT, files).unwrap();
        assert_eq!(plain.items[0].arm_pose, None);
    }

    #[test]
    fn served_package_crafting_remainder_defers_registry_lookup_to_load() {
        let served =
            format!("{MANIFEST_TEXT}namespaces = [\"ruby_extra\"]\nserved_by = \"horizon_java\"\n");
        let dir = scratch("served-remainder");
        let path = dir.join("m.smod");
        for (remainder, accepted) in [
            ("ruby:host_bowl", true),
            ("ruby_extra:host_bowl", true),
            ("other:bowl", true),
            ("ruby:Bad", false),
            ("minecraft:air", false),
        ] {
            let body = format!(
                r#"{{"template":"minecraft:mushroom_stew","crafting_remainder":"{remainder}"}}"#
            );
            std::fs::write(
                &path,
                Package::write_files(
                    &served,
                    &[("data/ruby/sunrise/item/stew.json", body.as_bytes())],
                )
                .unwrap(),
            )
            .unwrap();
            let package = Package::read(&path).unwrap();
            match Content::read(&path, &package.manifest) {
                Ok(content) => {
                    assert!(accepted, "{remainder}");
                    assert_eq!(
                        content.items[0].crafting_remainder.as_deref(),
                        Some(remainder)
                    );
                }
                Err(error) => {
                    assert!(!accepted, "{error}");
                    assert!(error.contains(remainder), "{error}");
                }
            }
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn items_patch_their_template_components() {
        let sword = br#"{"template": "minecraft:iron_sword", "components": {
            "minecraft:max_damage": 900,
            "minecraft:attribute_modifiers": [{"type": "minecraft:attack_damage",
                "id": "minecraft:base_attack_damage", "amount": 7.0,
                "operation": "add_value", "slot": "mainhand"}],
            "!minecraft:repairable": {}}}"#;
        let files: &[(&str, &[u8])] = &[("data/ruby/sunrise/item/ruby_sword.json", sword)];
        let content = smod("components", MANIFEST_TEXT, files).unwrap();
        let components = &content.items[0].components;
        assert_eq!(
            components.keys().collect::<Vec<_>>(),
            [
                "!minecraft:repairable",
                "minecraft:attribute_modifiers",
                "minecraft:max_damage"
            ]
        );
        assert_eq!(components["minecraft:max_damage"], 900);
    }

    #[test]
    fn mob_effects_take_a_category_a_colour_and_a_sort_order() {
        let files: &[(&str, &[u8])] = &[
            (
                "data/ruby/sunrise/mob_effect/glow.json",
                br#"{"category": "beneficial", "color": 16711680, "sort_order": -3}"#,
            ),
            (
                "data/ruby/sunrise/mob_effect/curse/dark.json",
                br#"{"category": "harmful", "color": 0}"#,
            ),
        ];
        let content = smod("effects", MANIFEST_TEXT, files).unwrap();
        let effects: Vec<_> = content
            .effects
            .iter()
            .map(|e| (e.id.as_str(), e.category, e.color, e.sort_order))
            .collect();
        assert_eq!(
            effects,
            [
                ("ruby:curse/dark", EffectCategory::Harmful, 0, None),
                ("ruby:glow", EffectCategory::Beneficial, 0xFF0000, Some(-3)),
            ]
        );
        for (case, file, why) in [
            (
                "effect-colour",
                r#"{"category": "neutral", "color": 16777216}"#,
                "color is RGB",
            ),
            (
                "effect-category",
                r#"{"category": "good", "color": 1}"#,
                "unknown variant",
            ),
            (
                "effect-key",
                r#"{"category": "neutral", "color": 1, "id": "x"}"#,
                "unknown field",
            ),
            (
                "effect-missing",
                r#"{"category": "neutral"}"#,
                "missing field",
            ),
        ] {
            let files: &[(&str, &[u8])] =
                &[("data/ruby/sunrise/mob_effect/a.json", file.as_bytes())];
            let e = smod(case, MANIFEST_TEXT, files).unwrap_err();
            assert!(e.contains(why), "{case}: {e}");
        }
    }

    /// #4473: `attribute_modifiers` is `MobEffect.addAttributeModifier`'s
    /// arguments, one entry per attribute; without it there are none.
    #[test]
    fn mob_effects_take_attribute_modifiers_one_per_attribute() {
        let stun = br#"{"category": "harmful", "color": 1, "attribute_modifiers": [
            {"attribute": "minecraft:movement_speed", "id": "ruby:effect/stun_speed",
             "amount": -1.0, "operation": "add_multiplied_total"},
            {"attribute": "ruby:mana", "id": "ruby:effect/stun_mana",
             "amount": 2.5, "operation": "add_value"}]}"#;
        let files: &[(&str, &[u8])] = &[
            ("data/ruby/sunrise/mob_effect/stun.json", stun),
            (
                "data/ruby/sunrise/mob_effect/plain.json",
                br#"{"category": "neutral", "color": 1}"#,
            ),
        ];
        let content = smod("effect-modifiers", MANIFEST_TEXT, files).unwrap();
        assert!(content.effects[0].attribute_modifiers.is_empty());
        assert_eq!(
            content.effects[1].attribute_modifiers,
            [
                EffectAttributeModifier {
                    attribute: "minecraft:movement_speed".into(),
                    id: "ruby:effect/stun_speed".into(),
                    amount: -1.0,
                    operation: ModifierOperation::AddMultipliedTotal,
                },
                EffectAttributeModifier {
                    attribute: "ruby:mana".into(),
                    id: "ruby:effect/stun_mana".into(),
                    amount: 2.5,
                    operation: ModifierOperation::AddValue,
                },
            ]
        );
        let entry = |attribute: &str, id: &str, amount: &str, operation: &str| {
            format!(
                r#"{{"attribute": "{attribute}", "id": "{id}", "amount": {amount}, "operation": "{operation}"}}"#
            )
        };
        let speed = entry("minecraft:movement_speed", "ruby:a", "1", "add_value");
        for (case, modifiers, why) in [
            (
                "modifier-attribute",
                entry("movement_speed", "ruby:a", "1", "add_value"),
                "attribute_modifiers[0]: attribute \"movement_speed\" is an attribute id",
            ),
            (
                "modifier-id",
                entry("minecraft:movement_speed", "Ruby:A", "1", "add_value"),
                "attribute_modifiers[0]: id \"Ruby:A\" is a modifier id",
            ),
            (
                "modifier-id-dot-dot",
                entry("minecraft:movement_speed", "..:stun", "1", "add_value"),
                "attribute_modifiers[0]: id \"..:stun\" is a modifier id",
            ),
            (
                "modifier-attribute-dot-dot",
                entry("..:speed", "ruby:a", "1", "add_value"),
                "attribute_modifiers[0]: attribute \"..:speed\" is an attribute id",
            ),
            (
                "modifier-amount",
                entry("minecraft:movement_speed", "ruby:a", "1e999", "add_value"),
                "number out of range",
            ),
            (
                "modifier-operation",
                entry("minecraft:movement_speed", "ruby:a", "1", "multiply"),
                "unknown variant `multiply`",
            ),
            (
                "modifier-twice",
                format!(
                    "{speed}, {}",
                    entry("minecraft:movement_speed", "ruby:b", "2", "add_value")
                ),
                "attribute_modifiers[0] (ruby:a) and attribute_modifiers[1] (ruby:b) both modify minecraft:movement_speed",
            ),
            (
                "modifier-key",
                speed.replace('}', r#", "slot": "any"}"#),
                "unknown field `slot`",
            ),
        ] {
            let file = format!(
                r#"{{"category": "neutral", "color": 1, "attribute_modifiers": [{modifiers}]}}"#
            );
            let files: &[(&str, &[u8])] =
                &[("data/ruby/sunrise/mob_effect/a.json", file.as_bytes())];
            let e = smod(case, MANIFEST_TEXT, files).unwrap_err();
            assert!(e.contains(why), "{case}: {e}");
        }
    }

    #[test]
    fn menu_owned_slots_require_one_valid_stack_limit_per_slot() {
        let path = "data/ruby/sunrise/menu/filter.json";
        let menu = br#"{"slots":2,"owned_slots":[1,32]}"#;
        let content = smod("menu-owned", MANIFEST_TEXT, &[(path, menu)]).unwrap();
        assert_eq!(
            content.menu_types[0].owned_slots.as_deref(),
            Some(&[1, 32][..])
        );

        for (case, limits) in [
            ("too-short", "[1]"),
            ("too-long", "[1,2,3]"),
            ("zero", "[0,2]"),
            ("oversized", "[1,100]"),
        ] {
            let menu = format!(r#"{{"slots":2,"owned_slots":{limits}}}"#);
            let error = smod(case, MANIFEST_TEXT, &[(path, menu.as_bytes())]).unwrap_err();
            assert!(error.contains("owned_slots"), "{case}: {error}");
        }
    }

    #[test]
    fn menu_owned_slots_allow_results_and_reject_crafting() {
        let path = "data/ruby/sunrise/menu/filter.json";
        let processing = br#"{"slots":3,"owned_slots":[64,64,64],"inputs":[0,1],"results":[2]}"#;
        let content = smod("owned-results", MANIFEST_TEXT, &[(path, processing)]).unwrap();
        assert_eq!(
            content.menu_types[0].owned_slots.as_deref(),
            Some(&[64, 64, 64][..])
        );
        assert_eq!(content.menu_types[0].inputs, [0, 1]);
        assert_eq!(content.menu_types[0].results, [2]);

        let crafting =
            br#"{"slots":5,"owned_slots":[1,1,1,1,1],"crafting":{"grid":0,"size":2,"result":4}}"#;
        let parsed_crafting: MenuType = serde_json::from_slice(crafting).unwrap();
        assert_eq!(
            validate_menu_type(&parsed_crafting),
            Err(MenuTypeRefusal::OwnedSlotsWithCrafting)
        );
        let error = smod("owned-crafting", MANIFEST_TEXT, &[(path, crafting)]).unwrap_err();
        assert!(error.contains("ruby:filter"), "{error}");

        let overlap = br#"{"slots":3,"owned_slots":[64,64,64],"inputs":[0,2],"results":[2]}"#;
        let parsed_overlap: MenuType = serde_json::from_slice(overlap).unwrap();
        assert_eq!(
            validate_menu_type(&parsed_overlap),
            Err(MenuTypeRefusal::DuplicateOrResultInput(2))
        );
        let error = smod("owned-overlap", MANIFEST_TEXT, &[(path, overlap)]).unwrap_err();
        assert!(error.contains("ruby:filter"), "{error}");
    }

    #[test]
    fn menu_types_take_slots_data_slots_and_roles() {
        let files: &[(&str, &[u8])] = &[
            (
                "data/ruby/sunrise/menu/press.json",
                br#"{"slots": 9, "data_slots": 2, "results": [7, 8], "inputs": [0, 6]}"#,
            ),
            ("data/ruby/sunrise/menu/bag.json", br#"{"slots": 4}"#),
            (
                "data/ruby/sunrise/menu/status.json",
                br#"{"slots": 0, "player_inventory": false, "data_slots": 2}"#,
            ),
            (
                "data/ruby/sunrise/menu/station.json",
                br#"{"slots": 16, "results": [11], "inputs": [15], "crafting": {"grid": 1, "result": 0}, "links": {"from": 12, "count": 4}}"#,
            ),
        ];
        let content = smod("menus", MANIFEST_TEXT, files).unwrap();
        let menus: Vec<_> = content
            .menu_types
            .iter()
            .map(|m| {
                (
                    m.id.as_str(),
                    m.slots,
                    m.player_inventory,
                    m.data_slots,
                    m.results.as_slice(),
                    m.inputs.as_slice(),
                )
            })
            .collect();
        assert_eq!(
            menus,
            [
                ("ruby:bag", 4, true, 0, &[][..], &[][..]),
                ("ruby:press", 9, true, 2, &[7, 8][..], &[0, 6][..]),
                ("ruby:station", 16, true, 0, &[11][..], &[15][..]),
                ("ruby:status", 0, false, 2, &[][..], &[][..]),
            ]
        );
        let links: Vec<_> = content.menu_types.iter().map(|m| m.links).collect();
        assert_eq!(
            links,
            [
                None,
                None,
                Some(super::MenuLinks { from: 12, count: 4 }),
                None
            ]
        );
        let crafting: Vec<_> = content.menu_types.iter().map(|m| m.crafting).collect();
        assert_eq!(
            crafting,
            [
                None,
                None,
                Some(super::MenuCrafting {
                    grid: 1,
                    size: 3,
                    result: 0
                }),
                None,
            ]
        );
        let wide = smod(
            "menu-wide-data",
            MANIFEST_TEXT,
            &[
                (
                    "data/ruby/sunrise/menu/wide.json",
                    br#"{"slots": 300, "data_slots": 300}"#,
                ),
                (
                    "data/ruby/sunrise/menu/huge.json",
                    br#"{"slots": 1, "data_slots": 70000}"#,
                ),
            ],
        )
        .unwrap();
        // Menu declarations do not create saved containers; the item slot count can exceed 256.
        let wide_counts: Vec<_> = wide
            .menu_types
            .iter()
            .map(|m| (m.id.as_str(), m.slots, m.data_slots))
            .collect();
        assert_eq!(
            wide_counts,
            [("ruby:huge", 1, 70_000), ("ruby:wide", 300, 300)]
        );
        for (case, file, details) in [
            ("menu-large-with-inv", r#"{"slots": 32733}"#, &["32769"][..]),
            (
                "menu-large-no-inv",
                r#"{"slots": 32769, "player_inventory": false}"#,
                &["32769"][..],
            ),
            (
                "menu-result",
                r#"{"slots": 0, "results": [0]}"#,
                &["result slot 0"][..],
            ),
            (
                "menu-input",
                r#"{"slots": 0, "inputs": [0]}"#,
                &["input 0"][..],
            ),
            (
                "menu-twice",
                r#"{"slots": 3, "results": [1, 1]}"#,
                &["result 1"][..],
            ),
            ("menu-key", r#"{"slots": 3, "rows": 1}"#, &["rows"][..]),
            (
                "craft-size",
                r#"{"slots": 20, "crafting": {"grid": 0, "size": 4, "result": 19}}"#,
                &[],
            ),
            (
                "craft-past",
                r#"{"slots": 9, "crafting": {"grid": 1, "result": 0}}"#,
                &["1..10", "result slot 0"],
            ),
            (
                "craft-result-past",
                r#"{"slots": 0, "crafting": {"grid": 0, "size": 2, "result": 4}}"#,
                &["0..4", "result slot 4"],
            ),
            (
                "craft-overlap",
                r#"{"slots": 10, "crafting": {"grid": 0, "result": 4}}"#,
                &[],
            ),
            (
                "craft-results",
                r#"{"slots": 10, "results": [2], "crafting": {"grid": 0, "result": 9}}"#,
                &[],
            ),
            (
                "craft-key",
                r#"{"slots": 10, "crafting": {"grid": 0, "result": 9, "width": 3}}"#,
                &["width"],
            ),
            (
                "links-past",
                r#"{"slots": 0, "links": {"from": 0, "count": 1}}"#,
                &["0..1"],
            ),
            (
                "links-empty",
                r#"{"slots": 10, "links": {"from": 2, "count": 0}}"#,
                &["2..2"],
            ),
            (
                "links-results",
                r#"{"slots": 10, "results": [5], "links": {"from": 4, "count": 2}}"#,
                &[],
            ),
            (
                "links-grid",
                r#"{"slots": 20, "crafting": {"grid": 1, "result": 0}, "links": {"from": 9, "count": 4}}"#,
                &[],
            ),
            (
                "links-key",
                r#"{"slots": 10, "links": {"from": 2, "count": 2, "rows": 1}}"#,
                &["rows"],
            ),
            ("menu-missing", r#"{"data_slots": 1}"#, &["slots"]),
        ] {
            let files: &[(&str, &[u8])] = &[("data/ruby/sunrise/menu/a.json", file.as_bytes())];
            let error = smod(case, MANIFEST_TEXT, files).unwrap_err();
            assert!(error.contains("ruby:a"), "{case}: {error}");
            for detail in details {
                assert!(error.contains(detail), "{case}: {error}");
            }
        }
    }

    #[test]
    fn menu_types_reject_invalid_inputs() {
        for (case, file) in [
            ("input-past", r#"{"slots": 3, "inputs": [3]}"#),
            ("input-twice", r#"{"slots": 3, "inputs": [1, 1]}"#),
            (
                "input-result",
                r#"{"slots": 3, "inputs": [1], "results": [1]}"#,
            ),
            (
                "input-grid",
                r#"{"slots": 6, "inputs": [2], "crafting": {"grid": 1, "size": 2, "result": 0}}"#,
            ),
            (
                "input-craft-result",
                r#"{"slots": 6, "inputs": [0], "crafting": {"grid": 1, "size": 2, "result": 0}}"#,
            ),
        ] {
            let files: &[(&str, &[u8])] = &[("data/ruby/sunrise/menu/a.json", file.as_bytes())];
            assert!(smod(case, MANIFEST_TEXT, files).is_err(), "{case}");
        }
    }

    #[test]
    fn entity_types_copy_a_throwable_and_name_its_item() {
        let files: &[(&str, &[u8])] = &[
            ("data/ruby/sunrise/item/gem.json", ITEM),
            (
                "data/ruby/sunrise/entity_type/bolt.json",
                br#"{"template": "minecraft:snowball", "item": "ruby:gem", "damage": 2.5}"#,
            ),
            ("data/ruby/sunrise/item/dust.json", ITEM),
            (
                "data/ruby/sunrise/entity_type/puff.json",
                br#"{"template": "minecraft:snowball", "item": "ruby:dust"}"#,
            ),
        ];
        let content = smod("types", MANIFEST_TEXT, files).unwrap();
        let types: Vec<_> = content
            .entity_types
            .iter()
            .map(|t| (t.id.as_str(), t.item.as_deref(), t.damage))
            .collect();
        assert_eq!(
            types,
            [
                ("ruby:bolt", Some("ruby:gem"), 2.5),
                ("ruby:puff", Some("ruby:dust"), 0.0)
            ]
        );
        let snowball = |rest: &str| format!(r#"{{"template": "minecraft:snowball"{rest}}}"#);
        for (case, file, why) in [
            (
                "type-template",
                r#"{"template": "minecraft:egg", "item": "ruby:gem"}"#.to_owned(),
                "is not one of",
            ),
            (
                "type-vanilla-item",
                snowball(r#", "item": "minecraft:stick""#),
                "is not one of this mod's items",
            ),
            (
                "type-unknown-item",
                snowball(r#", "item": "ruby:none""#),
                "is not one of this mod's items",
            ),
            (
                "type-damage",
                snowball(r#", "item": "ruby:gem", "damage": -1"#),
                "damage is 0 or more",
            ),
            (
                "type-key",
                snowball(r#", "item": "ruby:gem", "size": 1"#),
                "unknown field",
            ),
            (
                "type-no-item",
                snowball(""),
                "names the item that throws it",
            ),
            (
                "type-thrown-attributes",
                snowball(r#", "item": "ruby:gem", "attributes": {"minecraft:max_health": 4}"#),
                "only a living type has attributes",
            ),
            (
                "type-thrown-passenger-attachments",
                snowball(r#", "item": "ruby:gem", "passenger_attachments": [0.5]"#),
                "only a living type has passenger attachments",
            ),
        ] {
            let files: &[(&str, &[u8])] = &[
                ("data/ruby/sunrise/item/gem.json", ITEM),
                ("data/ruby/sunrise/entity_type/a.json", file.as_bytes()),
            ];
            let e = smod(case, MANIFEST_TEXT, files).unwrap_err();
            assert!(e.contains(why), "{case}: {e}");
        }
        let shared = snowball(r#", "item": "ruby:gem""#);
        let files: &[(&str, &[u8])] = &[
            ("data/ruby/sunrise/item/gem.json", ITEM),
            ("data/ruby/sunrise/entity_type/a.json", shared.as_bytes()),
            ("data/ruby/sunrise/entity_type/b.json", shared.as_bytes()),
        ];
        let e = smod("type-shared", MANIFEST_TEXT, files).unwrap_err();
        assert!(e.contains("already throws another entity type"), "{e}");
    }

    #[test]
    fn living_entity_types_copy_a_mob_with_their_own_attributes() {
        let files: &[(&str, &[u8])] = &[
            (
                "data/ruby/sunrise/entity_type/ghoul.json",
                br#"{"template": "minecraft:zombie",
                     "attributes": {"minecraft:max_health": 40, "minecraft:movement_speed": 0.3}}"#,
            ),
            (
                "data/ruby/sunrise/entity_type/settler.json",
                br#"{"template": "minecraft:villager"}"#,
            ),
            (
                "data/ruby/sunrise/entity_type/merchant.json",
                br#"{"template": "minecraft:wandering_trader"}"#,
            ),
            // The tameable ones (#4031).
            (
                "data/ruby/sunrise/entity_type/hound.json",
                br#"{"template": "minecraft:wolf", "attributes": {"minecraft:max_health": 20}}"#,
            ),
            (
                "data/ruby/sunrise/entity_type/macaw.json",
                br#"{"template": "minecraft:parrot"}"#,
            ),
            (
                "data/ruby/sunrise/entity_type/tabby.json",
                br#"{"template": "minecraft:cat"}"#,
            ),
            (
                "data/ruby/sunrise/entity_type/archer.json",
                br#"{"template": "minecraft:skeleton"}"#,
            ),
            (
                "data/ruby/sunrise/entity_type/diver.json",
                br#"{"template": "minecraft:drowned"}"#,
            ),
        ];
        let content = smod("living", MANIFEST_TEXT, files).unwrap();
        let types: Vec<_> = content
            .entity_types
            .iter()
            .map(|t| {
                let attributes: Vec<_> =
                    t.attributes.iter().map(|(a, v)| (a.as_str(), *v)).collect();
                (
                    t.id.as_str(),
                    t.template.as_str(),
                    t.is_living(),
                    attributes,
                )
            })
            .collect();
        assert_eq!(
            types,
            [
                ("ruby:archer", "minecraft:skeleton", true, vec![]),
                ("ruby:diver", "minecraft:drowned", true, vec![]),
                (
                    "ruby:ghoul",
                    "minecraft:zombie",
                    true,
                    vec![
                        ("minecraft:max_health", 40.0),
                        ("minecraft:movement_speed", 0.3)
                    ]
                ),
                (
                    "ruby:hound",
                    "minecraft:wolf",
                    true,
                    vec![("minecraft:max_health", 20.0)]
                ),
                ("ruby:macaw", "minecraft:parrot", true, vec![]),
                ("ruby:merchant", "minecraft:wandering_trader", true, vec![]),
                ("ruby:settler", "minecraft:villager", true, vec![]),
                ("ruby:tabby", "minecraft:cat", true, vec![]),
            ]
        );
        let zombie = |rest: &str| format!(r#"{{"template": "minecraft:zombie"{rest}}}"#);
        for (case, file, why) in [
            (
                "living-item",
                zombie(r#", "item": "ruby:gem""#),
                "a living type has no item or damage",
            ),
            (
                "living-damage",
                zombie(r#", "damage": 2"#),
                "a living type has no item or damage",
            ),
            (
                "living-attribute-id",
                zombie(r#", "attributes": {"max_health": 40}"#),
                "is an attribute id",
            ),
            (
                "living-attribute-value",
                zombie(r#", "attributes": {"minecraft:max_health": "40"}"#),
                "invalid type",
            ),
        ] {
            let files: &[(&str, &[u8])] = &[
                ("data/ruby/sunrise/item/gem.json", ITEM),
                ("data/ruby/sunrise/entity_type/a.json", file.as_bytes()),
            ];
            let e = smod(case, MANIFEST_TEXT, files).unwrap_err();
            assert!(e.contains(why), "{case}: {e}");
        }
    }

    #[test]
    fn living_entity_types_load_passenger_attachments() {
        let files: &[(&str, &[u8])] = &[
            (
                "data/ruby/sunrise/entity_type/bird.json",
                br#"{"template": "minecraft:zombie", "passenger_attachments": [0.0, 0.7, -0.1]}"#,
            ),
            (
                "data/ruby/sunrise/entity_type/raft.json",
                br#"{"template": "minecraft:villager", "passenger_attachments": [[0.0, 0.5, 0.3], [0.0, 0.5, -0.3]]}"#,
            ),
            (
                "data/ruby/sunrise/entity_type/tall.json",
                br#"{"template": "minecraft:wandering_trader", "passenger_attachments": 1.25}"#,
            ),
        ];
        let content = smod("passengers", MANIFEST_TEXT, files).unwrap();
        assert_eq!(
            content.entity_types[0].passenger_attachments,
            vec![[0.0, 0.7, -0.1]]
        );
        assert_eq!(
            content.entity_types[1].passenger_attachments,
            vec![[0.0, 0.5, 0.3], [0.0, 0.5, -0.3]]
        );
        assert_eq!(
            content.entity_types[2].passenger_attachments,
            vec![[0.0, 1.25, 0.0]]
        );
    }

    #[test]
    fn a_block_may_occlude_nothing() {
        let block = |occludes: &str| {
            let text = format!(
                r#"{{"template": "minecraft:barrel", "properties": {{"occludes": {occludes}}}}}"#
            );
            let files: &[(&str, &[u8])] =
                &[("data/ruby/sunrise/block/panel.json", text.as_bytes())];
            smod("occludes", MANIFEST_TEXT, files)
        };
        let occludes = |c: Result<Content, String>| c.unwrap().blocks[0].properties.occludes;
        assert_eq!(occludes(block("false")), Some(false));
        assert_eq!(occludes(block("true")), Some(true));
        assert!(block(r#""no""#).is_err());
    }

    #[test]
    fn blocks_choose_their_render_shape() {
        let block = |shape: &str| {
            let text = format!(
                r#"{{"template": "minecraft:stone", "properties": {{"render_shape": {shape}}}}}"#
            );
            let files: &[(&str, &[u8])] = &[("data/ruby/sunrise/block/gear.json", text.as_bytes())];
            smod("render-shape", MANIFEST_TEXT, files)
        };
        let shape = |c: Result<Content, String>| c.unwrap().blocks[0].properties.render_shape;
        assert_eq!(shape(block(r#""MODEL""#)), Some(RenderShape::Model));
        assert_eq!(
            shape(block(r#""ENTITYBLOCK_ANIMATED""#)),
            Some(RenderShape::EntityblockAnimated)
        );
        assert_eq!(shape(block(r#""INVISIBLE""#)), Some(RenderShape::Invisible));
        // Only vanilla's constants, as written there.
        for bad in [r#""animated""#, r#""model""#, "1"] {
            assert!(block(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn block_offset_refusals_name_the_file_and_key() {
        for (properties, key) in [
            (r#""offset":"XY""#, "offset"),
            (r#""max_horizontal_offset":-0.1"#, "max_horizontal_offset"),
            (r#""max_horizontal_offset":0.3"#, "max_horizontal_offset"),
            (r#""max_horizontal_offset":"wide""#, "max_horizontal_offset"),
            (r#""max_vertical_offset":0.3"#, "max_vertical_offset"),
        ] {
            let json = format!(r#"{{"template":"minecraft:stone","properties":{{{properties}}}}}"#);
            let file = "data/ruby/sunrise/block/offset.json";
            let error =
                smod("offset-refusal", MANIFEST_TEXT, &[(file, json.as_bytes())]).unwrap_err();
            assert!(error.contains(file) && error.contains(key), "{error}");
        }
    }

    #[test]
    fn block_face_rules_parse_and_refuse_unknown_facts_and_directions() {
        let json = r##"{
            "skip_rendering": [
                {"neighbor": "self", "same": ["axis"], "faces": "horizontal"},
                {"neighbor": "#minecraft:slabs", "faces": ["north", "up"], "facts": {"can_occlude": false, "is_air": false}},
                {"neighbor": {"block": "minecraft:glass", "state": {"facing": "east"}}, "state": {"half": "bottom"}, "faces": "vertical"}
            ],
            "hides_neighbor_face": [
                {"neighbor": {"tag": "test:targets", "state": {"lit": "true"}}, "facts": {"is_solid_render": false, "liquid": false}}
            ],
            "supports_external_face_hiding": false
        }"##;
        let props: BlockProperties = serde_json::from_str(json).unwrap();
        let skip = props.skip_rendering.as_ref().unwrap();
        assert_eq!(skip.len(), 3);
        assert_eq!(props.hides_neighbor_face.len(), 1);
        assert_eq!(props.supports_external_face_hiding, Some(false));
        assert!(face_rules::check_face_rules("skip_rendering", skip).is_ok());
        assert!(
            face_rules::check_face_rules("hides_neighbor_face", &props.hides_neighbor_face).is_ok()
        );

        assert!(
            serde_json::from_str::<BlockProperties>(
                r#"{"skip_rendering":[{"facts":{"blocks_motion":false}}]}"#
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<BlockProperties>(
                r#"{"hides_neighbor_face":[{"faces":["north","sideways"]}]}"#
            )
            .is_err()
        );

        use mod_vocab::{FaceNeighbor, FaceRule};
        let bad_typed = vec![FaceRule {
            neighbor: FaceNeighbor::Block {
                id: "not_namespaced".into(),
                state: BTreeMap::new(),
            },
            ..FaceRule::default()
        }];
        assert!(face_rules::check_face_rules("skip_rendering", &bad_typed).is_err());

        let file = "data/ruby/sunrise/block/bad_rules.json";
        assert!(
            smod(
                "bad-face-rule",
                MANIFEST_TEXT,
                &[(
                    file,
                    br#"{"template":"minecraft:stone","properties":{"skip_rendering":[{"facts":{"unknown_fact":true}}]}}"#,
                )],
            )
            .is_err()
        );
    }

    #[test]
    fn face_rules_refuse_properties_the_block_does_not_declare() {
        use mod_vocab::{FaceNeighbor, FaceRule};
        let known = BTreeMap::from([(
            "axis".to_owned(),
            std::collections::BTreeSet::from(["x".to_owned(), "y".to_owned()]),
        )]);
        let rule = |state: &[(&str, &str)], same: &[&str]| FaceRule {
            state: state
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            same: same.iter().map(|k| (*k).to_owned()).collect(),
            ..FaceRule::default()
        };
        let accepted = [
            rule(&[("axis", "x")], &["axis"]),
            FaceRule {
                neighbor: FaceNeighbor::Tag {
                    id: "optional:empty".into(),
                    state: BTreeMap::from([("elsewhere".to_owned(), "1".to_owned())]),
                },
                ..FaceRule::default()
            },
        ];
        assert!(
            face_rules::check_face_rule_properties("skip_rendering", &accepted, &known).is_ok()
        );
        for bad in [
            rule(&[("axs", "x")], &[]),
            rule(&[("axis", "z")], &[]),
            rule(&[], &["axs"]),
        ] {
            assert!(
                face_rules::check_face_rule_properties("skip_rendering", &[bad], &known).is_err()
            );
        }
    }

    #[test]
    fn block_instruments_accept_every_vanilla_constant_and_name_unknown_values() {
        for instrument in mod_vocab::NoteInstrument::ALL {
            let name = instrument.name().to_ascii_uppercase();
            let properties: BlockProperties =
                serde_json::from_str(&format!(r#"{{"instrument":"{name}"}}"#)).unwrap();
            assert_eq!(properties.instrument, Some(instrument));
        }
        let error = serde_json::from_str::<BlockProperties>(r#"{"instrument":"FLUTEISH"}"#)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("instrument") && error.contains("FLUTEISH"),
            "{error}"
        );
    }

    #[test]
    fn blocks_choose_vanilla_push_reactions() {
        for name in ["PUSH_PULL", "PUSH", "POPPED", "IMMOVEABLE", "IGNORE_ENTITY"] {
            let text = format!(
                r#"{{"template":"minecraft:stone","properties":{{"push_reaction":"{name}"}}}}"#
            );
            let files: &[(&str, &[u8])] = &[("data/ruby/sunrise/block/gear.json", text.as_bytes())];
            let content = smod("push-reaction", MANIFEST_TEXT, files).unwrap();
            assert_eq!(
                content.blocks[0].properties.push_reaction.unwrap().name(),
                name
            );
        }
    }

    #[test]
    fn blocks_pick_the_states_that_tick_randomly() {
        let own = r#""state_properties": {"grown": {"type": "boolean"}}"#;
        let block = |ticks: &str| {
            let text = format!(
                r#"{{"template": "minecraft:dirt", {own}, "properties": {{"random_ticks": {ticks}}}}}"#
            );
            let files: &[(&str, &[u8])] = &[("data/ruby/sunrise/block/crop.json", text.as_bytes())];
            smod("random-ticks", MANIFEST_TEXT, files)
        };
        let ticks =
            |c: Result<Content, String>| c.unwrap().blocks[0].properties.random_ticks.clone();
        let state = |v: &str| [("grown".to_owned(), v.to_owned())];
        // Every state, whatever the template does; `false` stops a template that ticks.
        let all = ticks(block("true")).unwrap();
        assert!(all.applies(&state("true")) && all.applies(&[]));
        assert!(!ticks(block("false")).unwrap().applies(&state("false")));
        // Chosen states: a crop that stops ticking once it is grown.
        let chosen = ticks(block(r#"{"property": "grown", "values": ["false"]}"#)).unwrap();
        assert!(chosen.applies(&state("false")) && !chosen.applies(&state("true")));
        assert!(
            !chosen.applies(&[]),
            "a state without the property does not tick"
        );
        // Not the block's own property, no values, or a value it doesn't have.
        for bad in [
            r#"{"property": "age", "values": ["0"]}"#,
            r#"{"property": "grown", "values": []}"#,
            r#"{"property": "grown", "values": ["maybe"]}"#,
        ] {
            let e = block(bad).err().unwrap();
            assert!(e.contains("random_ticks"), "{bad}: {e}");
        }
        assert!(block("3").is_err());
    }

    #[test]
    fn block_items_patch_their_components() {
        let block = br#"{"template": "minecraft:iron_block", "components": {
            "minecraft:max_stack_size": 16, "!minecraft:lore": {}}}"#;
        let files: &[(&str, &[u8])] = &[("data/ruby/sunrise/block/ruby_block.json", block)];
        let content = smod("block-components", MANIFEST_TEXT, files).unwrap();
        let components = &content.blocks[0].components;
        assert_eq!(
            components.keys().collect::<Vec<_>>(),
            ["!minecraft:lore", "minecraft:max_stack_size"]
        );
        assert_eq!(components["minecraft:max_stack_size"], 16);
    }

    #[test]
    fn a_server_mod_is_refused_for_registry_files_but_not_for_assets() {
        let server = MANIFEST_TEXT.replace("side = \"both\"", "side = \"server\"");
        let manifest = Manifest::parse(&server).unwrap();
        let recipe: &[u8] = br#"{"type": "minecraft:crafting_shapeless"}"#;
        for (file, bytes) in [
            ("data/ruby/sunrise/item/gem.json", ITEM),
            ("data/ruby/sunrise/block/ruby_block.json", BLOCK),
        ] {
            let content = smod("server-refused", &server, &[(file, bytes)]).unwrap();
            assert_eq!(
                content.server_only_refusal(&manifest).unwrap(),
                format!(
                    "ruby: a side = \"server\" mod carries {file}, which every client must have; declare side = \"both\""
                )
            );
        }
        let content = smod(
            "server-loads",
            &server,
            &[
                ("assets/ruby/lang/en_us.json", &b"{}"[..]),
                ("data/ruby/recipe/gem.json", recipe),
            ],
        )
        .unwrap();
        assert_eq!(content.server_only_refusal(&manifest), None);
        assert_eq!(
            content.files.as_ref(),
            ["assets/ruby/lang/en_us.json", "data/ruby/recipe/gem.json"]
        );
    }

    #[test]
    fn folder_mod_reads_the_same() {
        let dir = scratch("folder");
        std::fs::write(dir.join(crate::manifest::MANIFEST), MANIFEST_TEXT).unwrap();
        let block = dir.join("data/ruby/sunrise/block");
        std::fs::create_dir_all(&block).unwrap();
        std::fs::write(block.join("ruby_block.json"), BLOCK).unwrap();
        let package = Package::read(&dir).unwrap();
        let content = Content::read(&dir, &package.manifest).unwrap();
        assert_eq!(
            content.files.as_ref(),
            ["data/ruby/sunrise/block/ruby_block.json"]
        );
        assert_eq!(content.file(&content.files[0]).unwrap(), BLOCK);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A connecting block names a block tag and copies a `PipeBlock`.
    #[test]
    fn connecting_blocks_copy_a_pipe_and_name_a_tag() {
        let block = |json: &str| {
            let files: &[(&str, &[u8])] =
                &[("data/ruby/sunrise/block/cable.json", json.as_bytes())];
            smod("cable", MANIFEST_TEXT, files)
        };
        let content = block(
            r##"{"template": "minecraft:chorus_plant", "connects_to": "#ruby:cable_joins"}"##,
        )
        .unwrap();
        assert_eq!(
            content.blocks[0].connects_to.as_deref(),
            Some("ruby:cable_joins")
        );
        let thin =
            r##"{"template": "minecraft:chorus_plant", "connects_to": "#ruby:j", "thickness": 4}"##;
        assert_eq!(block(thin).unwrap().blocks[0].thickness, Some(4.0));
        let e = block(r#"{"template": "minecraft:chorus_plant", "thickness": 4}"#).unwrap_err();
        assert!(e.contains("connects_to"), "{e}");
        let e = block(r##"{"template": "minecraft:chorus_plant", "connects_to": "#ruby:j", "thickness": 16}"##)
            .unwrap_err();
        assert!(e.contains("2 to 14"), "{e}");
        let e = block(r##"{"template": "minecraft:stone", "connects_to": "#ruby:cable_joins"}"##)
            .unwrap_err();
        assert!(e.contains("chorus_plant"), "{e}");
        for tag in ["ruby:cable_joins", "#cable_joins", "#Ruby:Cable"] {
            let json =
                format!(r#"{{"template": "minecraft:chorus_plant", "connects_to": "{tag}"}}"#);
            let e = block(&json).unwrap_err();
            assert!(e.contains("block tag"), "{tag}: {e}");
        }
    }

    #[test]
    fn declared_waterlogged_requires_a_boolean() {
        let parse = |template: &str, property: &str| {
            let json = format!(
                r#"{{"template":"minecraft:{template}","state_properties":{{"waterlogged":{property}}}}}"#
            );
            smod(
                "waterlogged",
                MANIFEST_TEXT,
                &[("data/ruby/sunrise/block/plant.json", json.as_bytes())],
            )
        };
        assert!(parse("tall_grass", r#"{"type":"boolean"}"#).is_ok());
        assert!(parse("stone", r#"{"type":"boolean"}"#).is_ok());
        let error = parse("stone", r#"{"type":"enum","values":["false","true"]}"#).unwrap_err();
        assert!(
            error.contains("state_properties.waterlogged") && error.contains("plant.json"),
            "{error}"
        );
    }

    #[test]
    fn gate_sounds_need_a_fence_gate_and_sound_ids() {
        let block = |json: &str| {
            smod(
                "gate",
                MANIFEST_TEXT,
                &[("data/ruby/sunrise/block/gate.json", json.as_bytes())],
            )
        };
        let parsed = block(
            r#"{"template": "minecraft:oak_fence_gate", "open_sound": "ruby:gate.open", "close_sound": "minecraft:block.chain.break"}"#,
        )
        .unwrap();
        assert_eq!(
            parsed.blocks[0].open_sound,
            Some(GateSound::All("ruby:gate.open".into()))
        );
        assert_eq!(
            parsed.blocks[0].close_sound,
            Some(GateSound::All("minecraft:block.chain.break".into()))
        );
        let sites = block(r#"{"template":"minecraft:oak_fence_gate","open_sound":{"use":"ruby:rope.open","redstone":"ruby:wire.open"},"close_sound":{"explosion":"ruby:blast.close"}}"#).unwrap();
        assert_eq!(
            sites.blocks[0].open_sound,
            Some(GateSound::Sites {
                on_use: Some("ruby:rope.open".into()),
                on_explosion: None,
                on_redstone: Some("ruby:wire.open".into()),
            })
        );
        assert_eq!(
            sites.blocks[0].close_sound,
            Some(GateSound::Sites {
                on_use: None,
                on_explosion: Some("ruby:blast.close".into()),
                on_redstone: None,
            })
        );
        let plain = block(r#"{"template": "minecraft:oak_fence_gate"}"#).unwrap();
        assert_eq!(
            (&plain.blocks[0].open_sound, &plain.blocks[0].close_sound),
            (&None, &None)
        );
        for (bad, word) in [
            (
                r#"{"template": "minecraft:oak_fence", "open_sound": "ruby:gate.open"}"#,
                "minecraft:oak_fence",
            ),
            (
                r#"{"template": "minecraft:oak_fence_gate", "close_sound": "gate"}"#,
                "namespace:path",
            ),
            (
                r#"{"template":"minecraft:oak_fence","open_sound":{"use":"ruby:open"}}"#,
                "open_sound",
            ),
            (
                r#"{"template":"minecraft:oak_fence_gate","open_sound":{"other":"ruby:open"}}"#,
                "open_sound",
            ),
            (
                r#"{"template":"minecraft:oak_fence_gate","close_sound":{"redstone":"bad"}}"#,
                "close_sound.redstone",
            ),
        ] {
            let e = block(bad).unwrap_err();
            assert!(e.contains("data/ruby/sunrise/block/gate.json"), "{e}");
            assert!(e.contains(word), "{bad}: {e}");
        }
    }

    #[test]
    fn block_item_names_an_item_and_needs_item_false() {
        let block = |json: &str| {
            smod(
                "blockitem",
                MANIFEST_TEXT,
                &[("data/ruby/sunrise/block/slab.json", json.as_bytes())],
            )
        };
        let parsed = block(
            r#"{"template": "minecraft:stone", "item": false, "block_item": "ruby:slab_item"}"#,
        )
        .unwrap();
        assert_eq!(
            parsed.blocks[0].block_item.as_deref(),
            Some("ruby:slab_item")
        );
        assert_eq!(
            block(r#"{"template": "minecraft:stone"}"#).unwrap().blocks[0].block_item,
            None
        );
        for (bad, word) in [
            (
                r#"{"template": "minecraft:stone", "block_item": "ruby:slab_item"}"#,
                "item is true",
            ),
            (
                r#"{"template": "minecraft:stone", "item": false, "block_item": "none"}"#,
                "\"item\": false",
            ),
            (
                r#"{"template": "minecraft:stone", "item": false, "block_item": "slab"}"#,
                "namespace:path",
            ),
        ] {
            let e = block(bad).unwrap_err();
            assert!(e.contains(word), "{bad}: {e}");
        }
    }

    #[test]
    fn pick_item_names_an_item_whatever_the_blocks_own_is() {
        let block = |json: &str| {
            smod(
                "pickitem",
                MANIFEST_TEXT,
                &[("data/ruby/sunrise/block/crop.json", json.as_bytes())],
            )
        };
        for json in [
            r#"{"template": "minecraft:stone", "item": false, "pick_item": "ruby:seed"}"#,
            r#"{"template": "minecraft:stone", "pick_item": "ruby:seed"}"#,
            r#"{"template": "minecraft:stone", "item": false, "block_item": "ruby:fruit", "pick_item": "ruby:seed"}"#,
        ] {
            let parsed = block(json).unwrap();
            assert_eq!(parsed.blocks[0].pick_item.as_deref(), Some("ruby:seed"));
        }
        assert_eq!(
            block(r#"{"template": "minecraft:stone"}"#).unwrap().blocks[0].pick_item,
            None
        );
        for bad in ["seed", "none", ""] {
            let e = block(&format!(
                r#"{{"template": "minecraft:stone", "pick_item": "{bad}"}}"#
            ))
            .unwrap_err();
            assert!(
                e.contains("pick_item") && e.contains("namespace:path"),
                "{bad}: {e}"
            );
        }
    }

    #[test]
    fn farmland_base_requires_farmland_and_a_block_id() {
        let block = |json: &str| {
            smod(
                "farmland",
                MANIFEST_TEXT,
                &[("data/ruby/sunrise/block/farm.json", json.as_bytes())],
            )
        };
        let parsed =
            block(r#"{"template":"minecraft:farmland","base_block":"ruby:soil"}"#).unwrap();
        assert_eq!(parsed.blocks[0].base_block.as_deref(), Some("ruby:soil"));
        for bad in [
            r#"{"template":"minecraft:stone","base_block":"ruby:soil"}"#,
            r#"{"template":"minecraft:farmland","base_block":"soil"}"#,
        ] {
            assert!(block(bad).unwrap_err().contains("base_block"));
        }
    }

    /// A sign names its wood; a standing or ceiling sign names the wall form
    /// its item places. Neither means anything on another template.
    #[test]
    fn signs_name_their_wood_and_wall_form() {
        let block = |json: &str| {
            let files: &[(&str, &[u8])] =
                &[("data/ruby/sunrise/block/notice.json", json.as_bytes())];
            smod("sign", MANIFEST_TEXT, files)
        };
        let files: &[(&str, &[u8])] = &[
            (
                "data/ruby/sunrise/block/notice.json",
                br#"{"template": "minecraft:oak_sign", "wood_type": "ruby:elm", "wall_block": "ruby:notice_wall"}"#,
            ),
            (
                "data/ruby/sunrise/block/notice_wall.json",
                br#"{"template": "minecraft:oak_wall_sign", "item": false}"#,
            ),
        ];
        let content = smod("sign-pair", MANIFEST_TEXT, files).unwrap();
        assert_eq!(content.blocks[0].wood_type.as_deref(), Some("ruby:elm"));
        assert_eq!(
            content.blocks[0].wall_block.as_deref(),
            Some("ruby:notice_wall")
        );
        // Vanilla's woods go by their bare names, as `WoodType.name` does.
        let cherry = r#"{"template": "minecraft:cherry_wall_hanging_sign", "wood_type": "cherry", "item": false}"#;
        assert_eq!(
            block(cherry).unwrap().blocks[0].wood_type.as_deref(),
            Some("cherry")
        );
        assert_eq!(
            block(r#"{"template": "minecraft:oak_sign", "wood_type": "ruby-wood:old/elm"}"#)
                .unwrap()
                .blocks[0]
                .wood_type
                .as_deref(),
            Some("ruby-wood:old/elm")
        );
        for bad in [
            r#"{"template": "minecraft:stone", "wood_type": "ruby:elm"}"#,
            r#"{"template": "minecraft:oak_sign", "wood_type": "Ruby:Elm"}"#,
            r#"{"template": "minecraft:oak_sign", "wood_type": "a:b:c"}"#,
            r#"{"template": "minecraft:oak_sign", "wood_type": "../elm"}"#,
        ] {
            assert!(block(bad).unwrap_err().contains("wood_type"), "{bad}");
        }
        let text = block(
            r#"{"template": "minecraft:oak_sign", "sign_text_color": "light_blue", "sign_text_glow": true}"#,
        )
        .unwrap();
        assert_eq!(
            text.blocks[0].sign_text_color.as_deref(),
            Some("light_blue")
        );
        assert_eq!(text.blocks[0].sign_text_glow, Some(true));
        for bad in [
            r#"{"template": "minecraft:stone", "sign_text_color": "white"}"#,
            r#"{"template": "minecraft:oak_sign", "sign_text_color": "White"}"#,
            r##"{"template": "minecraft:oak_sign", "sign_text_color": "#ffffff"}"##,
        ] {
            assert!(block(bad).unwrap_err().contains("sign_text_color"), "{bad}");
        }
        assert!(
            block(r#"{"template": "minecraft:stone", "sign_text_glow": true}"#)
                .unwrap_err()
                .contains("sign_text_glow")
        );
        for bad in [
            r#"{"template": "minecraft:stone", "wall_block": "ruby:notice_wall"}"#,
            r#"{"template": "minecraft:oak_wall_sign", "wall_block": "ruby:notice_wall", "item": false}"#,
            r#"{"template": "minecraft:oak_sign", "wall_block": "notice_wall"}"#,
            r#"{"template": "minecraft:oak_sign", "wall_block": ":wall"}"#,
            r#"{"template": "minecraft:oak_sign", "wall_block": "ruby:"}"#,
            r#"{"template": "minecraft:oak_sign", "wall_block": "Ruby:wall"}"#,
            r#"{"template": "minecraft:oak_sign", "wall_block": "ruby:wall:other"}"#,
            r#"{"template": "minecraft:oak_sign", "wall_block": "ruby:notice_wall", "item": false}"#,
        ] {
            assert!(block(bad).unwrap_err().contains("wall_block"), "{bad}");
        }
    }

    /// A sign's wall form is one of the mod's blocks, of the matching wall
    /// template, with no item and no other sign. Anything else refuses this
    /// mod when it is read, and leaves the other mods alone.
    #[test]
    fn a_signs_wall_block_is_its_own_mods_wall_form() {
        let refused = |template: &str, wall_block: &str, wall: &str, wanted: &str| {
            let sign =
                format!(r#"{{"template": "minecraft:{template}", "wall_block": "{wall_block}"}}"#);
            let second =
                format!(r#"{{"template": "minecraft:{template}", "wall_block": "ruby:wall"}}"#);
            let mut files: Vec<(&str, &[u8])> = vec![
                ("data/ruby/sunrise/block/sign.json", sign.as_bytes()),
                ("data/ruby/sunrise/block/wall.json", wall.as_bytes()),
            ];
            if wanted == "already paired" {
                files.push(("data/ruby/sunrise/block/second.json", second.as_bytes()));
            }
            let e = smod("sign-wall", MANIFEST_TEXT, &files).unwrap_err();
            assert!(e.contains(wanted), "{wanted}: {e}");
        };
        let wall = r#"{"template": "minecraft:oak_wall_sign", "item": false}"#;
        for (template, wall_template) in [
            ("oak_sign", "stone"),
            ("oak_sign", "oak_wall_hanging_sign"),
            ("oak_hanging_sign", "oak_wall_sign"),
        ] {
            let other = format!(r#"{{"template": "minecraft:{wall_template}", "item": false}}"#);
            refused(template, "ruby:wall", &other, "needs a template");
        }
        refused(
            "oak_sign",
            "ruby:wall",
            r#"{"template": "minecraft:oak_wall_sign"}"#,
            "\"item\" is false",
        );
        refused("oak_sign", "ruby:wall", wall, "already paired");
        for absent in ["ruby:elsewhere", "other:wall", "minecraft:oak_wall_sign"] {
            refused("oak_sign", absent, wall, "not one of this mod's blocks");
        }
        // A sign and the wall form it hangs as start with the same text, or it would change
        // colour on a wall.
        let pair = |sign_text: &str, wall_text: &str| {
            let sign = format!(
                r#"{{"template": "minecraft:oak_sign", "wall_block": "ruby:wall"{sign_text}}}"#
            );
            let wall =
                format!(r#"{{"template": "minecraft:oak_wall_sign", "item": false{wall_text}}}"#);
            let files: &[(&str, &[u8])] = &[
                ("data/ruby/sunrise/block/sign.json", sign.as_bytes()),
                ("data/ruby/sunrise/block/wall.json", wall.as_bytes()),
            ];
            smod("sign-text", MANIFEST_TEXT, files)
        };
        let white = r#", "sign_text_color": "white""#;
        assert!(pair(white, white).is_ok());
        assert!(
            pair(
                "",
                r#", "sign_text_color": "black", "sign_text_glow": false"#
            )
            .is_ok()
        );
        for (sign, wall) in [
            (white, ""),
            ("", white),
            (white, r#", "sign_text_color": "red""#),
        ] {
            assert!(
                pair(sign, wall)
                    .unwrap_err()
                    .contains("different sign_text")
            );
        }
        assert!(pair(r#", "sign_text_glow": true"#, "").is_err());

        // Declared in code, the same; nothing is added.
        let block = |id: &str, template: &str, item, wall_block: Option<&str>| Block {
            id: id.into(),
            template: template.into(),
            item,
            wall_block: wall_block.map(str::to_owned),
            ..Block::default()
        };
        let mut content = Content::declared("ruby");
        let sign = block("ruby:sign", "minecraft:oak_sign", true, Some("ruby:wall"));
        assert!(content.declare(std::slice::from_ref(&sign), &[]).is_err());
        assert!(content.blocks.is_empty());
        let wall = block("ruby:wall", "minecraft:oak_wall_sign", false, None);
        content.declare(&[sign, wall], &[]).unwrap();
        assert_eq!(content.blocks.len(), 2);
    }

    /// A creative tab: a title, an icon and its items, all item ids.
    #[test]
    fn world_fluid_generated_names_cannot_collide() {
        let files: &[(&str, &[u8])] = &[
            ("data/ruby/sunrise/fluid/oil.json", br#"{"world":{}}"#),
            (
                "data/ruby/sunrise/fluid/flowing_oil.json",
                br#"{"world":{}}"#,
            ),
        ];
        assert!(
            smod("fluid-collision", MANIFEST_TEXT, files)
                .unwrap_err()
                .contains("declared twice")
        );
    }

    #[test]
    fn fluid_interaction_files_load_in_name_order_and_refuse_what_cannot_resolve() {
        let file = |name: &str, body: &'static str| {
            vec![(
                format!("data/ruby/sunrise/fluid_interaction/{name}.json"),
                body,
            )]
        };
        let load = |files: Vec<(String, &'static str)>| {
            let files: Vec<(&str, &[u8])> = files
                .iter()
                .map(|(n, b)| (n.as_str(), b.as_bytes()))
                .collect();
            smod("fluid-interaction", MANIFEST_TEXT, &files)
        };
        let slag = r#"{"source":"minecraft:lava","neighbour":{"fluid_tag":"minecraft:water"},
            "below":{"block_tag":"ruby:cold"},"becomes":{"source":"ruby:slag","flowing":"ruby:slag[age=1]"}}"#;
        let mut files = file("b_slag", slag);
        files.extend(file(
            "a_glass",
            r#"{"source":"ruby:oil","neighbour":{"block":"minecraft:sand"},"becomes":{"flowing":"minecraft:glass"}}"#,
        ));
        let content = load(files).unwrap();
        let sources: Vec<_> = content
            .fluid_interactions
            .iter()
            .map(|i| i.source.as_str())
            .collect();
        assert_eq!(sources, ["ruby:oil", "minecraft:lava"], "by file name");
        for (body, refusal) in [
            (
                r#"{"source":"lava","neighbour":{"block":"minecraft:sand"},"becomes":{"source":"minecraft:glass"}}"#,
                "source",
            ),
            (
                r#"{"source":"minecraft:lava","neighbour":{"block":"sand"},"becomes":{"source":"minecraft:glass"}}"#,
                "neighbour",
            ),
            (
                r#"{"source":"minecraft:lava","neighbour":{"block":"minecraft:sand"},"becomes":{}}"#,
                "becomes",
            ),
            (
                r#"{"source":"minecraft:lava","neighbour":{"block":"minecraft:sand"},"becomes":{"source":"glass"}}"#,
                "becomes",
            ),
            (
                r#"{"source":"minecraft:lava","neighbour":{"stone":"minecraft:sand"},"becomes":{"source":"minecraft:glass"}}"#,
                "stone",
            ),
            (
                r#"{"source":"minecraft:lava","neighbour":{"block":"minecraft:sand"},"becomes":{"source":"minecraft:glass"},"when":"any"}"#,
                "when",
            ),
        ] {
            let error = load(file("bad", body)).unwrap_err();
            assert!(error.contains(refusal), "{body}: {error}");
        }
    }

    #[test]
    fn a_world_fluid_takes_neoforges_property_domain_but_no_negative_drop_off() {
        let fluid = |tick_delay, drop_off, slope_find_distance| WorldFluid {
            tick_delay,
            drop_off,
            slope_find_distance,
            ..WorldFluid::default()
        };
        assert_eq!(fluid(0, 0, 0).check(), Ok(()));
        assert_eq!(fluid(-3, 8, 12).check(), Ok(()));
        assert_eq!(
            fluid(5, -1, 4).check(),
            Err("drop_off is 0 or more".to_owned())
        );
    }

    #[test]
    fn a_world_fluid_brings_its_block_and_bucket() {
        let files: &[(&str, &[u8])] = &[
            (
                "data/ruby/sunrise/fluid/oil.json",
                br#"{"tint":4278190080,"world":{"tick_delay":20,"drop_off":2,"slope_find_distance":2}}"#,
            ),
            (
                "data/ruby/sunrise/fluid/mist.json",
                br#"{"world":{"bucket":false}}"#,
            ),
        ];
        let content = smod("world-fluid", MANIFEST_TEXT, files).unwrap();
        let oil = content.fluids.iter().find(|f| f.id == "ruby:oil").unwrap();
        assert_eq!(oil.flowing_id(), "ruby:flowing_oil");
        assert_eq!(oil.world.as_ref().unwrap().tick_delay, 20);
        assert!(!oil.world.as_ref().unwrap().source_conversion);
        let block = content.blocks.iter().find(|b| b.id == "ruby:oil").unwrap();
        assert_eq!(block.template, "minecraft:water");
        assert!(!block.item, "the fluid's block has no block item");
        let bucket = content
            .items
            .iter()
            .find(|i| i.id == "ruby:oil_bucket")
            .unwrap();
        assert_eq!(bucket.template, "minecraft:water_bucket");
        assert!(content.blocks.iter().any(|b| b.id == "ruby:mist"));
        assert!(!content.items.iter().any(|i| i.id == "ruby:mist_bucket"));
        assert_eq!(
            content
                .fluids
                .iter()
                .find(|f| f.id == "ruby:mist")
                .unwrap()
                .bucket_id(),
            None
        );
    }

    /// #6039: a world fluid names its liquid block (Blood Magic's `life_essence_block` for the
    /// fluid `life_essence_fluid`): the block is declared under that id, not the fluid's; a
    /// block outside the fluid's namespace or under its flowing form's id is refused.
    #[test]
    fn a_world_fluid_names_its_liquid_block() {
        let files: &[(&str, &[u8])] = &[(
            "data/ruby/sunrise/fluid/essence.json",
            br#"{"world":{"block":"ruby:essence_block"}}"#,
        )];
        let content = smod("world-fluid-block", MANIFEST_TEXT, files).unwrap();
        let essence = content
            .fluids
            .iter()
            .find(|f| f.id == "ruby:essence")
            .unwrap();
        assert_eq!(essence.block_id(), "ruby:essence_block");
        let block = content
            .blocks
            .iter()
            .find(|b| b.id == "ruby:essence_block")
            .unwrap();
        assert_eq!(block.template, "minecraft:water");
        assert!(!content.blocks.iter().any(|b| b.id == "ruby:essence"));
        let unnamed = smod(
            "world-fluid-plain",
            MANIFEST_TEXT,
            &[("data/ruby/sunrise/fluid/oil.json", br#"{"world":{}}"#)],
        )
        .unwrap();
        assert_eq!(
            unnamed.fluids[0].block_id(),
            "ruby:oil",
            "unset: the fluid's own id"
        );
        for refused in [
            &br#"{"world":{"block":"other:essence_block"}}"#[..],
            br#"{"world":{"block":"ruby:essence_flowing","flowing":"ruby:essence_flowing"}}"#,
            // The default flowing id, `ruby:flowing_essence`, though `flowing` is unset.
            br#"{"world":{"block":"ruby:flowing_essence"}}"#,
        ] {
            let files: &[(&str, &[u8])] = &[("data/ruby/sunrise/fluid/essence.json", refused)];
            let error = smod("world-fluid-refused", MANIFEST_TEXT, files).unwrap_err();
            assert!(error.to_string().contains("block"), "{error}");
        }
    }

    /// A port keeps its mod's ids (Sophisticated Core's `xp_still`, `xp_flowing` and
    /// `xp_bucket`): a world fluid names its flowing form and generated bucket, a tank-only
    /// fluid an item of its own as its bucket, and either states its light level.
    #[test]
    fn fluids_name_their_bucket_flowing_form_and_light() {
        let files: &[(&str, &[u8])] = &[
            (
                "data/ruby/sunrise/fluid/xp_still.json",
                br#"{"bucket_item":"ruby:xp_bucket","light_level":10,"world":{"flowing":"ruby:xp_flowing"}}"#,
            ),
            (
                "data/ruby/sunrise/fluid/essence.json",
                br#"{"bucket_item":"ruby:essence_jar","light_level":15}"#,
            ),
        ];
        let content = smod("named-fluid", MANIFEST_TEXT, files).unwrap();
        let xp = content
            .fluids
            .iter()
            .find(|f| f.id == "ruby:xp_still")
            .unwrap();
        assert_eq!(xp.flowing_id(), "ruby:xp_flowing");
        assert_eq!(xp.bucket_id().as_deref(), Some("ruby:xp_bucket"));
        assert_eq!(xp.measures.light_level, 10);
        assert!(content.items.iter().any(|i| i.id == "ruby:xp_bucket"));
        assert!(!content.items.iter().any(|i| i.id == "ruby:xp_still_bucket"));
        let essence = content
            .fluids
            .iter()
            .find(|f| f.id == "ruby:essence")
            .unwrap();
        assert_eq!(essence.bucket_id().as_deref(), Some("ruby:essence_jar"));
        assert_eq!(essence.measures.light_level, 15);
        assert!(
            !content.items.iter().any(|i| i.id == "ruby:essence_jar"),
            "a tank-only fluid's bucket is an item the mod declares"
        );
        for (body, refusal) in [
            (&br#"{"light_level":16}"#[..], "light_level"),
            (br#"{"bucket_item":"jar"}"#, "bucket_item"),
            (br#"{"bucket_item":"other:jar","world":{}}"#, "bucket_item"),
            (br#"{"world":{"flowing":"other:oil_flowing"}}"#, "flowing"),
            (br#"{"world":{"flowing":"ruby:oil"}}"#, "flowing"),
        ] {
            let bad: &[(&str, &[u8])] = &[("data/ruby/sunrise/fluid/oil.json", body)];
            let e = smod("bad-named-fluid", MANIFEST_TEXT, bad).unwrap_err();
            assert!(e.contains(refusal), "{e}");
        }
        let tank_named_elsewhere: &[(&str, &[u8])] = &[(
            "data/ruby/sunrise/fluid/oil.json",
            br#"{"bucket_item":"other:jar"}"#,
        )];
        assert!(smod("tank-bucket", MANIFEST_TEXT, tank_named_elsewhere).is_ok());
    }

    #[test]
    fn a_world_fluid_refuses_a_negative_drop_off_and_a_second_block() {
        let files: &[(&str, &[u8])] = &[(
            "data/ruby/sunrise/fluid/oil.json",
            br#"{"world":{"drop_off":-1}}"#,
        )];
        assert!(
            smod("bad-world-fluid", MANIFEST_TEXT, files)
                .unwrap_err()
                .contains("drop_off is 0 or more")
        );
        let files: &[(&str, &[u8])] = &[
            ("data/ruby/sunrise/fluid/oil.json", br#"{"world":{}}"#),
            (
                "data/ruby/sunrise/block/oil.json",
                br#"{"template":"minecraft:stone"}"#,
            ),
        ];
        assert!(
            smod("clashing-world-fluid", MANIFEST_TEXT, files)
                .unwrap_err()
                .contains("declared twice")
        );
    }

    #[test]
    fn fluids_declare_a_sprite_and_a_tint() {
        let files: &[(&str, &[u8])] = &[
            (
                "data/ruby/sunrise/fluid/steam.json",
                br#"{"sprite":"ruby:gui/fluids/steam","tint":4291343520}"#,
            ),
            ("data/ruby/sunrise/fluid/bare.json", b"{}"),
        ];
        let content = smod("fluid", MANIFEST_TEXT, files).unwrap();
        assert_eq!(
            content.fluids,
            [
                FluidLook {
                    id: "ruby:bare".into(),
                    sprite: None,
                    tint: None,
                    world: None,
                    bucket_item: None,
                    measures: FluidMeasures::default(),
                },
                FluidLook {
                    id: "ruby:steam".into(),
                    sprite: Some("ruby:gui/fluids/steam".into()),
                    tint: Some(4291343520),
                    world: None,
                    bucket_item: None,
                    measures: FluidMeasures::default(),
                },
            ]
        );
        for sprite in [
            &br#"{"sprite":"steam"}"#[..],
            br#"{"sprite":"ruby:has space"}"#,
            br#"{"sprite":"Ruby:steam"}"#,
            br#"{"sprite":"ruby:"}"#,
        ] {
            let bad: &[(&str, &[u8])] = &[("data/ruby/sunrise/fluid/x.json", sprite)];
            let e = smod("fluid", MANIFEST_TEXT, bad).unwrap_err();
            assert!(e.contains("sprite"), "{e}");
        }
    }

    /// `FluidType.Properties`: a file states its own numbers, a missing key keeps water's, and
    /// only a negative viscosity is refused.
    #[test]
    fn fluids_declare_their_temperature_density_and_viscosity() {
        let files: &[(&str, &[u8])] = &[
            (
                "data/ruby/sunrise/fluid/molten.json",
                br#"{"temperature":1500,"density":-200,"viscosity":0}"#,
            ),
            (
                "data/ruby/sunrise/fluid/cold.json",
                br#"{"temperature":-5}"#,
            ),
        ];
        let content = smod("fluid-measures", MANIFEST_TEXT, files).unwrap();
        let measures: Vec<_> = content
            .fluids
            .iter()
            .map(|look| (look.id.as_str(), look.measures))
            .collect();
        assert_eq!(
            measures,
            [
                (
                    "ruby:cold",
                    FluidMeasures {
                        temperature: -5,
                        density: 1000,
                        viscosity: 1000,
                        light_level: 0,
                    }
                ),
                (
                    "ruby:molten",
                    FluidMeasures {
                        temperature: 1500,
                        density: -200,
                        viscosity: 0,
                        light_level: 0,
                    }
                ),
            ]
        );
        let bad: &[(&str, &[u8])] = &[("data/ruby/sunrise/fluid/tar.json", br#"{"viscosity":-1}"#)];
        let e = smod("fluid-viscosity", MANIFEST_TEXT, bad).unwrap_err();
        assert!(e.contains("tar.json") && e.contains("viscosity"), "{e}");
    }

    #[test]
    fn creative_tabs_name_their_items() {
        let tab = |json: &str| {
            let files: &[(&str, &[u8])] =
                &[("data/ruby/sunrise/creative_tab/gems.json", json.as_bytes())];
            smod("tab", MANIFEST_TEXT, files)
        };
        let content =
            tab(r#"{"title": "Ruby Gems", "icon": "ruby:ruby", "items": ["ruby:ruby", "minecraft:diamond"]}"#)
                .unwrap();
        assert_eq!(
            content.creative_tabs,
            [CreativeTab {
                id: "ruby:gems".into(),
                title: "Ruby Gems".into(),
                icon: "ruby:ruby".into(),
                items: vec!["ruby:ruby".into(), "minecraft:diamond".into()],
            }]
        );
        let e = tab(r#"{"title": "Gems", "icon": "ruby", "items": []}"#).unwrap_err();
        assert!(e.contains("item ids"), "{e}");
    }

    #[test]
    fn two_packages_can_supply_the_merged_recipe_priorities_but_not_neighbouring_files() {
        let other_manifest = MANIFEST_TEXT.replace("ruby", "sapphire");
        for (name, manifest) in [("ruby", MANIFEST_TEXT), ("sapphire", &other_manifest)] {
            let content = smod(name, manifest, &[(RECIPE_PRIORITIES_PATH, b"{}")]).unwrap();
            assert!(
                content
                    .files
                    .iter()
                    .any(|path| path == RECIPE_PRIORITIES_PATH)
            );
        }
        for path in [
            "data/neoforge/recipe/unowned.json",
            "data/neoforge/other.json",
            "data/c/recipe_priorities.json",
            "data/minecraft/recipe_priorities.json",
        ] {
            let error =
                smod("nonshared-priority-path", MANIFEST_TEXT, &[(path, b"{}")]).unwrap_err();
            assert!(error.contains(path), "{error}");
        }
    }

    #[test]
    fn refuses_content_outside_its_fence() {
        let refused = |name: &str, files: &[(&str, &[u8])]| {
            let e = smod(name, MANIFEST_TEXT, files).unwrap_err();
            assert!(!e.is_empty());
            e
        };
        let e = refused("other-ns", &[("data/other/sunrise/block/a.json", BLOCK)]);
        assert!(e.contains("own namespaces"), "{e}");
        refused(
            "vanilla-asset",
            &[("assets/minecraft/textures/block/stone.png", b"x")],
        );
        refused(
            "vanilla-recipe",
            &[("data/minecraft/recipe/stone.json", b"{}")],
        );
        refused("convention-recipe", &[("data/c/recipe/stone.json", b"{}")]);
        refused("convention-asset", &[("assets/c/lang/en_us.json", b"{}")]);
        let e = refused(
            "modded-template",
            &[(
                "data/ruby/sunrise/block/a.json",
                br#"{"template": "ruby:other"}"#,
            )],
        );
        assert!(e.contains("not a vanilla block"), "{e}");
        refused(
            "light",
            &[(
                "data/ruby/sunrise/block/a.json",
                br#"{"template": "minecraft:stone", "properties": {"light": 16}}"#,
            )],
        );
        refused(
            "unknown-key",
            &[(
                "data/ruby/sunrise/block/a.json",
                br#"{"template": "minecraft:stone", "glow": true}"#,
            )],
        );
        let e = refused(
            "push-reaction",
            &[(
                "data/ruby/sunrise/block/a.json",
                br#"{"template":"minecraft:stone","properties":{"push_reaction":"NORMAL"}}"#,
            )],
        );
        assert!(e.contains("push_reaction"), "{e}");
        refused("shape-reversed", &[("data/ruby/sunrise/block/a.json", br#"{"template":"minecraft:stone","shapes":{"properties":["axis"],"values":{"x":[[1,0,0,0,1,1]]}}}"#)]);
        refused("shape-malformed", &[("data/ruby/sunrise/block/a.json", br#"{"template":"minecraft:stone","shapes":{"properties":["axis"],"values":{"x":[[0,0,0,1,1]]}}}"#)]);
        refused(
            "shape-no-values",
            &[(
                "data/ruby/sunrise/block/a.json",
                br#"{"template":"minecraft:stone","shapes":{"properties":["axis"],"values":{}}}"#,
            )],
        );
        let e = refused(
            "clash",
            &[
                ("data/ruby/sunrise/block/a.json", BLOCK),
                ("data/ruby/sunrise/item/a.json", ITEM),
            ],
        );
        assert!(e.contains("also a block's item"), "{e}");
        refused("other-kind", &[("data/ruby/sunrise/entity/a.json", b"{}")]);
        refused(
            "stack",
            &[(
                "data/ruby/sunrise/item/a.json",
                br#"{"template": "minecraft:stick", "max_stack_size": 100}"#,
            )],
        );
        for (case, components, why) in [
            (
                "bare-component",
                r#"{"max_damage": 3}"#,
                "must be namespaced",
            ),
            (
                "uppercase-path",
                r#"{"minecraft:Foo": 3}"#,
                "must be namespaced and valid",
            ),
            (
                "uppercase-namespace",
                r#"{"Example:charge": 3}"#,
                "must be namespaced and valid",
            ),
            (
                "empty-namespace",
                r#"{":charge": 3}"#,
                "must be namespaced and valid",
            ),
            (
                "empty-path",
                r#"{"example:": 3}"#,
                "must be namespaced and valid",
            ),
            (
                "invalid-removal",
                r#"{"!example:Foo": {}}"#,
                "must be namespaced and valid",
            ),
            (
                "own-key",
                r#"{"minecraft:rarity": "epic"}"#,
                "set rarity instead",
            ),
            (
                "own-removal",
                r#"{"!minecraft:max_stack_size": {}}"#,
                "set max_stack_size",
            ),
            (
                "removal-value",
                r#"{"!minecraft:food": true}"#,
                "a removal's value is {}",
            ),
        ] {
            let item = format!(r#"{{"template": "minecraft:stick", "components": {components}}}"#);
            let e = refused(case, &[("data/ruby/sunrise/item/a.json", item.as_bytes())]);
            assert!(e.contains(why), "{case}: {e}");
        }
        for (case, rest) in [
            ("bare-remainder", "bowl"),
            ("malformed-remainder", "jade:bad bowl"),
            ("air-remainder", "minecraft:air"),
        ] {
            let item =
                format!(r#"{{"template": "minecraft:stick", "crafting_remainder": "{rest}"}}"#);
            let files: &[(&str, &[u8])] = &[("data/ruby/sunrise/item/a.json", item.as_bytes())];
            let e = refused(case, files);
            assert!(e.contains("crafting_remainder"), "{case}: {e}");
        }
        for (case, block, why) in [
            (
                "block-removal-value",
                r#"{"template": "minecraft:stone", "components": {"!minecraft:lore": []}}"#,
                "a removal's value is {}",
            ),
            (
                "block-stack-zero",
                r#"{"template": "minecraft:stone", "components": {"minecraft:max_stack_size": 0}}"#,
                "integer 1 to 99",
            ),
            (
                "block-stack-100",
                r#"{"template": "minecraft:stone", "components": {"minecraft:max_stack_size": 100}}"#,
                "integer 1 to 99",
            ),
            (
                "block-stack-string",
                r#"{"template": "minecraft:stone", "components": {"minecraft:max_stack_size": "16"}}"#,
                "integer 1 to 99",
            ),
            (
                "block-rarity",
                r#"{"template": "minecraft:stone", "components": {"minecraft:rarity": "bogus"}}"#,
                "common, uncommon",
            ),
            (
                "block-without-item",
                r#"{"template": "minecraft:glass", "item": false,
                    "components": {"minecraft:max_stack_size": 16}}"#,
                "item is false",
            ),
        ] {
            let e = refused(
                case,
                &[("data/ruby/sunrise/block/a.json", block.as_bytes())],
            );
            assert!(e.contains(why), "{case}: {e}");
        }
    }

    #[test]
    fn reserved_namespace_accepts_only_declared_extension_tags() {
        for tag in crate::extension_tags::ALL {
            let path = format!(
                "data/sunrise/tags/{}/{}.json",
                tag.registry,
                tag.id.strip_prefix("sunrise:").unwrap()
            );
            let content = smod(
                "extension",
                MANIFEST_TEXT,
                &[(path.as_str(), br#"{"values":[]}"#)],
            )
            .unwrap();
            assert_eq!(content.files.len(), 1);
        }
        for (path, hint) in [
            (
                "data/sunrise/tags/block/villager_secondary_poi/custom.json",
                "secondary_blocks",
            ),
            (
                "data/sunrise/tags/item/villager_secondary_poi/farmer.json",
                "extension_tags",
            ),
            ("data/sunrise/tags/block/class/crop.json", "block_part_tag"),
            ("data/sunrise/loot_table/a.json", "own namespaces"),
            ("assets/sunrise/lang/en_us.json", "own namespaces"),
        ] {
            let error = smod("extension-refused", MANIFEST_TEXT, &[(path, b"{}")]).unwrap_err();
            assert!(error.contains(path) && error.contains(hint), "{error}");
        }
    }

    #[test]
    fn listed_namespaces_hold_content() {
        let manifest = format!("{MANIFEST_TEXT}namespaces = [\"kaisyn\"]\n");
        let files: &[(&str, &[u8])] = &[
            ("data/kaisyn/worldgen/template_pool/a.json", b"{}"),
            ("data/kaisyn/structure/a.nbt", b"nbt"),
            ("assets/kaisyn/lang/en_us.json", b"{}"),
        ];
        let content = smod("listed", &manifest, files).unwrap();
        assert_eq!(content.files.len(), 3);
        assert_eq!(content.namespaces, ["ruby", "kaisyn"]);
        // Files and code share the manifest-owned namespace fence.
        let decl: &[(&str, &[u8])] = &[("data/kaisyn/sunrise/block/a.json", BLOCK)];
        let content = smod("listed-decl", &manifest, decl).unwrap();
        assert_eq!(content.blocks[0].id, "kaisyn:a");
        let mut code = Content::declared("ruby");
        code.namespaces.push("kaisyn".into());
        code.declare(&content.blocks, &[]).unwrap();
        assert_eq!(code.blocks[0].id, "kaisyn:a");
        let mut foreign = content.blocks[0].clone();
        foreign.id = "unowned:a".into();
        let error = code.declare(&[foreign], &[]).unwrap_err()[0].1.to_string();
        assert!(
            error.contains("unowned:a")
                && error.contains("owned namespaces")
                && error.contains("ruby")
                && error.contains("kaisyn"),
            "{error}"
        );
        // A namespace it does not list is still outside.
        let other: &[(&str, &[u8])] = &[("data/other/structure/a.nbt", b"nbt")];
        assert!(smod("listed-other", &manifest, other).is_err());
    }

    #[test]
    fn mods_sharing_a_namespace_clash() {
        let t_and_t = ["t_and_t".to_owned(), "kaisyn".to_owned()];
        let kaisyn = ["kaisyn".to_owned()];
        let ruby = ["ruby".to_owned(), "gems".to_owned()];
        assert_eq!(
            clash(("t_and_t", &t_and_t), ("kaisyn", &kaisyn)).as_deref(),
            Some("mods t_and_t and kaisyn both use the namespace kaisyn; turn one of them off")
        );
        assert_eq!(clash(("t_and_t", &t_and_t), ("ruby", &ruby)), None);
    }

    #[test]
    fn paths_are_plain() {
        assert!(check_path("assets/ruby/textures/block/ruby_block.png").is_ok());
        assert!(check_path("data/ruby/loot_table/blocks/a.json").is_ok());
        for bad in [
            "assets/ruby",
            "assets/../x/y",
            "assets/ruby//a",
            "assets/Ruby/a",
            "assets/ruby/a b",
            "/assets/ruby/a",
            "other/ruby/a",
            "assets\\ruby\\a",
        ] {
            assert!(check_path(bad).is_err(), "{bad}");
        }
        let error = check_path("data/ruby/Glow-Slab.json").unwrap_err();
        assert!(error.contains("\"data/ruby/glow-slab.json\""), "{error}");
    }
}

#[cfg(test)]
mod context_shape_tests {
    use super::*;
    #[test]
    fn context_shape_selectors_and_tables_are_validated() {
        for selector in ["item", "player", "living", "other"] {
            for channel in ["outline_context_shapes", "collision_context_shapes"] {
                let text = format!(
                    r#"{{"template":"minecraft:stone","{channel}":{{"{selector}":{{"default":[]}}}}}}"#
                );
                let _: BlockFile = serde_json::from_str(&text).unwrap();
            }
        }
        for selector in ["none", "Item", "minecraft:item", "entity"] {
            let text = format!(
                r#"{{"template":"minecraft:stone","collision_context_shapes":{{"{selector}":{{"default":[]}}}}}}"#
            );
            assert!(serde_json::from_str::<BlockFile>(&text).is_err());
        }
        let mut block = Block {
            template: "minecraft:stone".into(),
            ..Default::default()
        };
        let bad = BlockShapes(StateValues {
            property: String::new(),
            properties: vec![],
            values: BTreeMap::new(),
            default: Some(vec![[1., 0., 0., 0., 1., 1.]]),
        });
        block
            .outline_context_shapes
            .insert(ShapeEntityKind::Player, bad.clone());
        assert!(check_block(&block).is_err());
        block.outline_context_shapes.clear();
        block
            .collision_context_shapes
            .insert(ShapeEntityKind::Item, bad.clone());
        assert!(check_block(&block).is_err());
        block.collision_context_shapes.clear();
        block.interaction_shapes = Some(bad);
        assert!(check_block(&block).is_err());
    }
}
