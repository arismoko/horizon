//! The 26.3 client item definitions (`assets/<ns>/items/*.json`): the model
//! tree (`model`, `composite`, `condition`, `select`, `range_dispatch`,
//! `special`, `neoforge:fluid_container`, `bundle/selected_item`, `empty`), its
//! properties and tints, and the resolver that turns one stack into render layers
//! (`ItemModelResolver.updateForTopItem`).
//!
//! Baked geometry lives in [`crate::item_bake`]; this module decides *which*
//! model, tints, foil and transformation each layer gets.
use crate::{
    json::{DecodeError, error as err},
    math::{Mat4, normalize_quaternion, quaternion_from_axis_angle},
};
use serde_json::{Map, Value};
use std::{
    collections::BTreeMap,
    ops::ControlFlow,
    sync::{Arc, Mutex},
};

mod component_predicate;
mod fluid_container;
mod foreign;
mod mod_properties;
mod special;
pub use fluid_container::{FluidAppearance, FluidContainer, FluidContainerTextures};
pub use mod_properties::{is_skull_kind, register_skull_kinds};
pub use special::{SpecialKind, SpecialModel};

type Result<T> = std::result::Result<T, DecodeError>;

/// `BuiltInRegistries.DATA_COMPONENT_TYPE`, which `has_component` and the
/// `component` select validate against (checked by the harness).
pub const DATA_COMPONENT_TYPES: [&str; 122] = [
    "minecraft:additional_trade_cost",
    "minecraft:attack_animation",
    "minecraft:attack_range",
    "minecraft:attribute_modifiers",
    "minecraft:axolotl/variant",
    "minecraft:banner_patterns",
    "minecraft:base_color",
    "minecraft:bees",
    "minecraft:block_entity_data",
    "minecraft:block_state",
    "minecraft:block_transformer",
    "minecraft:blocks_attacks",
    "minecraft:break_sound",
    "minecraft:brewing_fuel",
    "minecraft:bucket_entity_data",
    "minecraft:bundle_contents",
    "minecraft:can_break",
    "minecraft:can_place_on",
    "minecraft:cat/collar",
    "minecraft:cat/sound_variant",
    "minecraft:cat/variant",
    "minecraft:charged_projectiles",
    "minecraft:chicken/sound_variant",
    "minecraft:chicken/variant",
    "minecraft:compostable",
    "minecraft:consumable",
    "minecraft:container",
    "minecraft:container_loot",
    "minecraft:cooking_fuel",
    "minecraft:cow/sound_variant",
    "minecraft:cow/variant",
    "minecraft:creative_slot_lock",
    "minecraft:cushion/color",
    "minecraft:custom_data",
    "minecraft:custom_model_data",
    "minecraft:custom_name",
    "minecraft:damage",
    "minecraft:damage_resistant",
    "minecraft:damage_type",
    "minecraft:death_protection",
    "minecraft:debug_stick_state",
    "minecraft:dye",
    "minecraft:dyed_color",
    "minecraft:enchantable",
    "minecraft:enchantment_glint_override",
    "minecraft:enchantments",
    "minecraft:entity_data",
    "minecraft:equippable",
    "minecraft:firework_explosion",
    "minecraft:fireworks",
    "minecraft:food",
    "minecraft:fox/variant",
    "minecraft:frog/variant",
    "minecraft:glider",
    "minecraft:horse/variant",
    "minecraft:instrument",
    "minecraft:intangible_projectile",
    "minecraft:interact_animation",
    "minecraft:item_model",
    "minecraft:item_name",
    "minecraft:jukebox_playable",
    "minecraft:kinetic_weapon",
    "minecraft:llama/variant",
    "minecraft:lock",
    "minecraft:lodestone_tracker",
    "minecraft:lore",
    "minecraft:map_decorations",
    "minecraft:map_id",
    "minecraft:map_post_processing",
    "minecraft:max_damage",
    "minecraft:max_stack_size",
    "minecraft:minimum_attack_charge",
    "minecraft:mob_visibility",
    "minecraft:mooshroom/variant",
    "minecraft:note_block_sound",
    "minecraft:ominous_bottle_amplifier",
    "minecraft:painting/variant",
    "minecraft:parrot/variant",
    "minecraft:piercing_weapon",
    "minecraft:pig/sound_variant",
    "minecraft:pig/variant",
    "minecraft:pot_decorations",
    "minecraft:potion_contents",
    "minecraft:potion_duration_scale",
    "minecraft:profile",
    "minecraft:provides_banner_patterns",
    "minecraft:provides_pottery_pattern",
    "minecraft:provides_trim_material",
    "minecraft:rabbit/variant",
    "minecraft:rarity",
    "minecraft:recipes",
    "minecraft:repair_cost",
    "minecraft:repairable",
    "minecraft:salmon/size",
    "minecraft:sheep/color",
    "minecraft:shulker/color",
    "minecraft:sign_text_back",
    "minecraft:sign_text_front",
    "minecraft:stored_enchantments",
    "minecraft:sulfur_cube_content",
    "minecraft:suspicious_stew_effects",
    "minecraft:tool",
    "minecraft:tooltip_display",
    "minecraft:tooltip_style",
    "minecraft:trim",
    "minecraft:tropical_fish/base_color",
    "minecraft:tropical_fish/pattern",
    "minecraft:tropical_fish/pattern_color",
    "minecraft:unbreakable",
    "minecraft:use_cooldown",
    "minecraft:use_effects",
    "minecraft:use_remainder",
    "minecraft:villager/variant",
    "minecraft:villager_food",
    "minecraft:waxed",
    "minecraft:weapon",
    "minecraft:wolf/collar",
    "minecraft:wolf/sound_variant",
    "minecraft:wolf/variant",
    "minecraft:writable_book_content",
    "minecraft:written_book_content",
    "minecraft:zombie_nautilus/variant",
];

// ---------------------------------------------------------------------------
// Display contexts
// ---------------------------------------------------------------------------

/// `ItemDisplayContext`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DisplayContext {
    None,
    ThirdPersonLeftHand,
    ThirdPersonRightHand,
    FirstPersonLeftHand,
    FirstPersonRightHand,
    Head,
    Gui,
    Ground,
    Fixed,
    OnShelf,
}
impl DisplayContext {
    pub const ALL: [Self; 10] = [
        Self::None,
        Self::ThirdPersonLeftHand,
        Self::ThirdPersonRightHand,
        Self::FirstPersonLeftHand,
        Self::FirstPersonRightHand,
        Self::Head,
        Self::Gui,
        Self::Ground,
        Self::Fixed,
        Self::OnShelf,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ThirdPersonLeftHand => "thirdperson_lefthand",
            Self::ThirdPersonRightHand => "thirdperson_righthand",
            Self::FirstPersonLeftHand => "firstperson_lefthand",
            Self::FirstPersonRightHand => "firstperson_righthand",
            Self::Head => "head",
            Self::Gui => "gui",
            Self::Ground => "ground",
            Self::Fixed => "fixed",
            Self::OnShelf => "on_shelf",
        }
    }
    pub fn by_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == name)
    }
    /// `ItemDisplayContext.leftHand`.
    pub fn left_hand(self) -> bool {
        matches!(self, Self::ThirdPersonLeftHand | Self::FirstPersonLeftHand)
    }
}

// ---------------------------------------------------------------------------
// Decoding helpers (DataFixerUpper `JsonOps` semantics)
// ---------------------------------------------------------------------------

fn object(v: &Value) -> Result<&Map<String, Value>> {
    v.as_object().ok_or_else(|| err("expected object"))
}
/// A present, non-null field (`JsonOps` maps `null` to an absent key).
fn get<'a>(m: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    m.get(key).filter(|v| !v.is_null())
}
fn required<'a>(m: &'a Map<String, Value>, key: &str) -> Result<&'a Value> {
    get(m, key).ok_or_else(|| err(&format!("missing {key}")))
}
/// `Codec.FLOAT`: numbers, and booleans as 1 and 0.
fn float(v: &Value) -> Result<f32> {
    match v {
        Value::Number(n) => n
            .to_string()
            .parse()
            .or_else(|_| n.as_f64().map(|f| f as f32).ok_or(()))
            .map_err(|()| err("invalid float")),
        _ => Err(err("expected float")),
    }
}
/// `Codec.INT`: `Number.intValue`, truncating toward zero and saturating
/// like a Java narrowing of a double (longs keep their low 32 bits).
fn int(v: &Value) -> Result<i32> {
    match v {
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i as i32)
            } else if let Some(u) = n.as_u64() {
                Ok(u as i32)
            } else {
                Ok(n.as_f64().ok_or_else(|| err("invalid int"))? as i32)
            }
        }
        _ => Err(err("expected int")),
    }
}
fn boolean(v: &Value) -> Result<bool> {
    v.as_bool().ok_or_else(|| err("expected boolean"))
}
fn string(v: &Value) -> Result<String> {
    v.as_str()
        .map(str::to_owned)
        .ok_or_else(|| err("expected string"))
}
fn list(v: &Value) -> Result<&Vec<Value>> {
    v.as_array().ok_or_else(|| err("expected list"))
}
/// `Identifier.CODEC`: a lowercase `namespace:path`, defaulting the namespace.
pub fn identifier(name: &str) -> Result<String> {
    let (namespace, path) = match name.split_once(':') {
        Some((n, p)) => (if n.is_empty() { "minecraft" } else { n }, p),
        None => ("minecraft", name),
    };
    let valid = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c);
    if path.contains(':')
        || !namespace.bytes().all(valid)
        || !path.bytes().all(|c| valid(c) || c == b'/')
    {
        return Err(err("invalid resource identifier"));
    }
    Ok(format!("{namespace}:{path}"))
}
fn ident(v: &Value) -> Result<String> {
    identifier(&string(v)?)
}
fn component_type(v: &Value) -> Result<String> {
    let id = ident(v)?;
    if DATA_COMPONENT_TYPES.contains(&id.as_str()) {
        Ok(id)
    } else {
        Err(err("unknown data component type"))
    }
}
fn vector<const N: usize>(v: &Value) -> Result<[f32; N]> {
    let values = list(v)?;
    if values.len() != N {
        return Err(err("wrong vector length"));
    }
    let mut out = [0.0; N];
    for (o, v) in out.iter_mut().zip(values) {
        *o = float(v)?;
    }
    Ok(out)
}
fn non_negative(v: &Value) -> Result<i32> {
    let i = int(v)?;
    if i < 0 {
        return Err(err("value must be non-negative"));
    }
    Ok(i)
}
/// `ExtraCodecs.RGB_COLOR_CODEC`: an int, or three floats in 0..1.
fn rgb(v: &Value) -> Result<i32> {
    if v.is_array() {
        let [r, g, b] = vector::<3>(v)?;
        return Ok(argb(255, channel(r), channel(g), channel(b)));
    }
    int(v)
}
fn argb(a: i32, r: i32, g: i32, b: i32) -> i32 {
    (a & 255) << 24 | (r & 255) << 16 | (g & 255) << 8 | (b & 255)
}
fn opaque(color: i32) -> i32 {
    color | (0xFF00_0000u32 as i32)
}

/// `Transformation.EXTENDED_CODEC`.
fn transformation(v: &Value) -> Result<Mat4> {
    if v.is_array() {
        return Ok(Mat4::from_row_major(vector::<16>(v)?));
    }
    let m = object(v)?;
    let quaternion = |v: &Value| -> Result<[f32; 4]> {
        if v.is_array() {
            return Ok(normalize_quaternion(vector::<4>(v)?));
        }
        let o = object(v)?;
        Ok(quaternion_from_axis_angle(
            float(required(o, "angle")?)?,
            vector::<3>(required(o, "axis")?)?,
        ))
    };
    let translation = vector::<3>(required(m, "translation")?)?;
    let left = quaternion(required(m, "left_rotation")?)?;
    let scale = vector::<3>(required(m, "scale")?)?;
    let right = quaternion(required(m, "right_rotation")?)?;
    Ok(Mat4::translation(translation)
        .rotate(left)
        .scale(scale)
        .rotate(right))
}
pub fn optional_transformation(m: &Map<String, Value>) -> Result<Option<Mat4>> {
    get(m, "transformation").map(transformation).transpose()
}
/// `ARGB.as8BitChannel`: `Mth.floor(value * 255.0F)`.
fn channel(v: f32) -> i32 {
    sunrise_core::mth::floor_f32(v * 255.0)
}

// ---------------------------------------------------------------------------
// The model tree
// ---------------------------------------------------------------------------

/// Engine callback for a special renderer's model object, called while loading the item
/// definition. `kind` is a validated `Identifier` (bare paths default to `minecraft:`), and
/// `base` is canonical. `Ok(None)` lets the builtin decoder handle the type; `Err` rejects the
/// enclosing definition, even if the type is also builtin.
pub type SpecialModelLoader<'a> = dyn Fn(
        &sunrise_core::identifier::Identifier,
        &str,
        &Map<String, Value>,
    ) -> std::result::Result<Option<RegisteredSpecialModel>, String>
    + 'a;

/// Engine callback for a registered item tint source, called while loading the item definition.
/// `kind` is canonical. `Ok(None)` lets the built-in decoder handle the type; `Err` rejects the
/// enclosing definition, even if the type is also built-in.
pub type TintSourceLoader<'a> = dyn Fn(&str, &Map<String, Value>) -> std::result::Result<Option<RegisteredTintSource>, String>
    + 'a;

/// Result returned by [`ItemModelLoader`] when a registered decoder handles an item model node.
#[derive(Clone, Debug)]
pub struct DecodedItemModel {
    pub model: RegisteredItemModel,
    pub transformation: Option<Mat4>,
    pub models: Vec<(String, ItemModelNode)>,
}

/// Engine callback for a registered item model node type, called while loading the item definition.
/// `kind` is canonical. `decode_child` decodes any nested item model JSON value using the same
/// provider tables. `Ok(None)` lets the built-in decoder handle the type; `Err` refuses the
/// enclosing definition, even if the type is also built-in.
pub type ItemModelLoader<'a> = dyn Fn(
        &str,
        &Map<String, Value>,
        &dyn Fn(&Value) -> std::result::Result<ItemModelNode, DecodeError>,
    ) -> std::result::Result<Option<DecodedItemModel>, String>
    + 'a;

/// Engine callback for a registered `range_dispatch` numeric property, called while loading the
/// item definition. `kind` is canonical. `Ok(None)` lets the built-in decoder handle the property;
/// `Err` rejects the enclosing definition, even if the property is also built-in.
pub type RangePropertyLoader<'a> = dyn Fn(&str, &Map<String, Value>) -> std::result::Result<Option<RegisteredRangeProperty>, String>
    + 'a;

/// Engine callback for a registered `select` categorical property, called while loading the item
/// definition. `kind` is canonical. `Ok(None)` lets the built-in decoder handle the property;
/// `Err` rejects the enclosing definition, even if the property is also built-in.
pub type SelectPropertyLoader<'a> = dyn Fn(&str, &Map<String, Value>) -> std::result::Result<Option<RegisteredSelectProperty>, String>
    + 'a;

/// A registered special renderer's baked model. The payload is opaque to assets; the
/// baker and draw host agree on its type. Clones retain the same model identity.
#[derive(Clone)]
pub struct RegisteredSpecialModel(pub Arc<dyn std::any::Any + Send + Sync>);
impl std::fmt::Debug for RegisteredSpecialModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RegisteredSpecialModel")
    }
}
impl PartialEq for RegisteredSpecialModel {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// A registered item tint source bound at decode time. The payload is opaque to assets; the
/// decoder and draw host agree on its type. Clones retain the same instance identity.
#[derive(Clone)]
pub struct RegisteredTintSource(pub Arc<dyn std::any::Any + Send + Sync>);
impl std::fmt::Debug for RegisteredTintSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RegisteredTintSource")
    }
}
impl PartialEq for RegisteredTintSource {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// A registered item model node bound at decode time. The payload is opaque to assets; the
/// decoder and draw host agree on its type. Clones retain the same instance identity.
#[derive(Clone)]
pub struct RegisteredItemModel(pub Arc<dyn std::any::Any + Send + Sync>);
impl std::fmt::Debug for RegisteredItemModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RegisteredItemModel")
    }
}
impl PartialEq for RegisteredItemModel {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// A registered `range_dispatch` numeric property bound at decode time. The payload is opaque to
/// assets; the decoder and draw host agree on its type. Clones retain the same instance identity.
#[derive(Clone)]
pub struct RegisteredRangeProperty(pub Arc<dyn std::any::Any + Send + Sync>);
impl std::fmt::Debug for RegisteredRangeProperty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RegisteredRangeProperty")
    }
}
impl PartialEq for RegisteredRangeProperty {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// A type-erased `when` case value. Only its provider may compare the underlying value,
/// inside its callback boundary.
#[derive(Clone, Debug)]
pub struct RegisteredSelectKey(Arc<dyn std::any::Any + Send + Sync>);

impl RegisteredSelectKey {
    pub fn new<T: Send + Sync + 'static>(value: T) -> Self {
        Self(Arc::new(value))
    }

    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        self.0.downcast_ref::<T>()
    }
}

/// Only a codec mismatch may try the scalar branch of the compact-list codec.
#[derive(Debug)]
pub enum SelectCaseDecodeError {
    Mismatch(String),
    Callback(String),
}

impl From<SelectCaseDecodeError> for DecodeError {
    fn from(error: SelectCaseDecodeError) -> Self {
        match error {
            SelectCaseDecodeError::Mismatch(reason) | SelectCaseDecodeError::Callback(reason) => {
                Self(reason)
            }
        }
    }
}

type RegisteredSelectCaseDecoder =
    dyn Fn(&Value) -> std::result::Result<RegisteredSelectKey, SelectCaseDecodeError> + Send + Sync;
type RegisteredSelectCaseEquality = dyn Fn(&RegisteredSelectKey, &RegisteredSelectKey) -> std::result::Result<bool, String>
    + Send
    + Sync;

/// A registered `select` categorical property bound at decode time. The payload is opaque to
/// assets; the decoder and draw host agree on its type, and `decode_case` validates and decodes
/// each `when` entry into a [`RegisteredSelectKey`]. Clones retain the same instance identity.
#[derive(Clone)]
pub struct RegisteredSelectProperty {
    pub payload: Arc<dyn std::any::Any + Send + Sync>,
    decode_case: Arc<RegisteredSelectCaseDecoder>,
    case_eq: Arc<RegisteredSelectCaseEquality>,
}
impl RegisteredSelectProperty {
    pub fn new(
        payload: Arc<dyn std::any::Any + Send + Sync>,
        decode_case: impl Fn(&Value) -> std::result::Result<RegisteredSelectKey, SelectCaseDecodeError>
        + Send
        + Sync
        + 'static,
        case_eq: impl Fn(
            &RegisteredSelectKey,
            &RegisteredSelectKey,
        ) -> std::result::Result<bool, String>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            payload,
            decode_case: Arc::new(decode_case),
            case_eq: Arc::new(case_eq),
        }
    }

    pub fn decode_case(
        &self,
        value: &Value,
    ) -> std::result::Result<RegisteredSelectKey, SelectCaseDecodeError> {
        (self.decode_case)(value)
    }
}
impl std::fmt::Debug for RegisteredSelectProperty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RegisteredSelectProperty")
    }
}
impl PartialEq for RegisteredSelectProperty {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.payload, &other.payload)
    }
}

/// Optional decode-time provider callbacks for item definitions, checked ahead of built-in
/// decoders so registered overrides take effect.
#[derive(Clone, Copy, Default)]
pub struct ItemModelDecoders<'a> {
    pub special: Option<&'a SpecialModelLoader<'a>>,
    pub tint: Option<&'a TintSourceLoader<'a>>,
    pub item_model: Option<&'a ItemModelLoader<'a>>,
    pub range_property: Option<&'a RangePropertyLoader<'a>>,
    pub select_property: Option<&'a SelectPropertyLoader<'a>>,
}

/// One `items/<name>.json` file: a model tree plus the client properties.
#[derive(Clone, Debug)]
pub struct ClientItem {
    pub model: ItemModelNode,
    pub hand_animation_on_swap: bool,
    pub oversized_in_gui: bool,
    pub swap_animation_scale: f32,
}
impl ClientItem {
    pub const DEFAULT_PROPERTIES: (bool, bool, f32) = (true, false, 1.0);
    pub fn from_json(text: &str) -> Result<Self> {
        Self::from_json_with_decoders(text, &ItemModelDecoders::default())
    }
    /// Decode an item with the engine's decode-time provider registrations, including child nodes.
    pub fn from_json_with_decoders(text: &str, decoders: &ItemModelDecoders<'_>) -> Result<Self> {
        let value: Value = serde_json::from_str(text.strip_prefix('\u{feff}').unwrap_or(text))?;
        Self::decode_with(&value, decoders)
    }
    pub fn decode(value: &Value) -> Result<Self> {
        Self::decode_with(value, &ItemModelDecoders::default())
    }
    pub fn decode_with(value: &Value, decoders: &ItemModelDecoders<'_>) -> Result<Self> {
        let m = object(value)?;
        Ok(Self {
            model: ItemModelNode::decode_with(required(m, "model")?, decoders)?,
            hand_animation_on_swap: get(m, "hand_animation_on_swap").map_or(Ok(true), boolean)?,
            oversized_in_gui: get(m, "oversized_in_gui").map_or(Ok(false), boolean)?,
            swap_animation_scale: get(m, "swap_animation_scale").map_or(Ok(1.0), float)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelLeaf {
    pub model: String,
    pub transformation: Option<Mat4>,
    pub tints: Vec<TintSource>,
}
#[derive(Clone, Debug)]
pub enum ItemModelNode {
    Empty,
    Model(ModelLeaf),
    Composite {
        models: Vec<ItemModelNode>,
        transformation: Option<Mat4>,
    },
    Condition {
        transformation: Option<Mat4>,
        property: ConditionProperty,
        on_true: Box<ItemModelNode>,
        on_false: Box<ItemModelNode>,
    },
    Select {
        transformation: Option<Mat4>,
        property: SelectProperty,
        cases: Vec<SwitchCase>,
        fallback: Option<Box<ItemModelNode>>,
    },
    RangeDispatch {
        transformation: Option<Mat4>,
        property: NumericProperty,
        scale: f32,
        /// Sorted by threshold (a stable sort, as `bake` does).
        entries: Vec<(f32, ItemModelNode)>,
        fallback: Option<Box<ItemModelNode>>,
    },
    Special {
        base: String,
        transformation: Option<Mat4>,
        model: SpecialModel,
    },
    /// A `minecraft:special` model decoded by a registered renderer, with `base`'s display
    /// transforms, particle and GUI light.
    RegisteredSpecial {
        base: Arc<str>,
        transformation: Option<Mat4>,
        model: RegisteredSpecialModel,
    },
    FluidContainer(FluidContainer),
    BundleSelectedItem,
    /// A node decoded by a registered item model provider (`register_item_models`): per stack, its
    /// bound provider answers which of `models`, by name, draw.
    Registered {
        model: RegisteredItemModel,
        transformation: Option<Mat4>,
        /// The node's `models` object: named nodes decoded and baked as any other.
        models: Vec<(String, ItemModelNode)>,
    },
}
/// One `when` value on a [`SwitchCase`]: canonical JSON for built-in properties, or a typed
/// [`RegisteredSelectKey`] decoded by a registered select property's case decoder.
#[derive(Clone, Debug)]
pub enum SwitchCaseValue {
    Json(Value),
    Registered(RegisteredSelectKey),
}

#[derive(Clone, Debug)]
pub struct SwitchCase {
    /// Property values, in their canonical form.
    pub values: Vec<SwitchCaseValue>,
    pub model: ItemModelNode,
}

impl ItemModelNode {
    pub fn decode(v: &Value) -> Result<Self> {
        Self::decode_with(v, &ItemModelDecoders::default())
    }

    pub fn decode_with(v: &Value, decoders: &ItemModelDecoders<'_>) -> Result<Self> {
        let decode = |node: &Value| Self::decode_with(node, decoders);
        let m = object(v)?;
        let kind = ident(required(m, "type")?)?;
        // Registered item model decoders run ahead of built-ins (including vanilla arms and
        // `neoforge:fluid_container`) so overrides take effect. An error refuses the enclosing
        // item definition and never falls back to a built-in.
        if let Some(load) = decoders.item_model
            && let Some(decoded) = load(&kind, m, &decode).map_err(DecodeError)?
        {
            return Ok(Self::Registered {
                model: decoded.model,
                transformation: decoded.transformation,
                models: decoded.models,
            });
        }
        if kind == "minecraft:condition" {
            let property = ident(required(m, "property")?)?;
            if !property.starts_with("minecraft:") {
                return Ok(Self::Condition {
                    transformation: optional_transformation(m)?,
                    property: ConditionProperty::Custom(property),
                    on_true: Box::new(decode(required(m, "on_true")?)?),
                    on_false: Box::new(decode(required(m, "on_false")?)?),
                });
            }
        }
        Ok(match kind.as_str() {
            "minecraft:empty" => Self::Empty,
            "minecraft:model" => Self::Model(ModelLeaf {
                model: ident(required(m, "model")?)?,
                transformation: optional_transformation(m)?,
                tints: match get(m, "tints") {
                    None => Vec::new(),
                    Some(t) => list(t)?
                        .iter()
                        .map(|tint| TintSource::decode_with(tint, decoders))
                        .collect::<Result<_>>()?,
                },
            }),
            "minecraft:composite" => Self::Composite {
                models: list(required(m, "models")?)?
                    .iter()
                    .map(decode)
                    .collect::<Result<_>>()?,
                transformation: optional_transformation(m)?,
            },
            "minecraft:condition" if foreign::property_is_foreign(m)? => foreign::wrap(
                decode(required(m, "on_false")?)?,
                optional_transformation(m)?,
            ),
            "minecraft:condition" => Self::Condition {
                transformation: optional_transformation(m)?,
                property: ConditionProperty::decode(m)?,
                on_true: Box::new(decode(required(m, "on_true")?)?),
                on_false: Box::new(decode(required(m, "on_false")?)?),
            },
            "minecraft:select" => {
                let Some(property) = SelectProperty::decode_with(m, decoders)? else {
                    // No case can match an unregistered foreign property.
                    let fallback = get(m, "fallback").map(decode).transpose()?;
                    return Ok(foreign::wrap(
                        fallback.unwrap_or(Self::Empty),
                        optional_transformation(m)?,
                    ));
                };
                let transformation = optional_transformation(m)?;
                let cases = list(required(m, "cases")?)?
                    .iter()
                    .map(|c| SwitchCase::decode(&property, c, decoders))
                    .collect::<Result<Vec<_>>>()?;
                if cases.is_empty() {
                    return Err(err("empty case list"));
                }
                let mut seen: Vec<&SwitchCaseValue> = Vec::new();
                for value in cases.iter().flat_map(|c| &c.values) {
                    for &previous in &seen {
                        let equal = match (previous, value, &property) {
                            (SwitchCaseValue::Json(a), SwitchCaseValue::Json(b), _) => a == b,
                            (
                                SwitchCaseValue::Registered(a),
                                SwitchCaseValue::Registered(b),
                                SelectProperty::Registered(bound),
                            ) => (bound.case_eq)(a, b).map_err(DecodeError)?,
                            _ => unreachable!("all cases of a select use the same decoder"),
                        };
                        if equal {
                            return Err(err("duplicate case conditions"));
                        }
                    }
                    seen.push(value);
                }
                Self::Select {
                    transformation,
                    property,
                    cases,
                    fallback: get(m, "fallback")
                        .map(|f| decode(f).map(Box::new))
                        .transpose()?,
                }
            }
            "minecraft:range_dispatch" => {
                let property = NumericProperty::decode_with(m, decoders)?;
                let mut entries = list(required(m, "entries")?)?
                    .iter()
                    .map(|e| {
                        let e = object(e)?;
                        Ok((
                            float(required(e, "threshold")?)?,
                            decode(required(e, "model")?)?,
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?;
                // `Comparator.comparingDouble`: Double.compare, a total order.
                entries.sort_by(|a, b| f64::from(a.0).total_cmp(&f64::from(b.0)));
                let Some(property) = property else {
                    // An unregistered foreign property reads 0.
                    let fallback = get(m, "fallback").map(decode).transpose()?;
                    let chosen = last_index_less_or_equal(&entries, 0.0)
                        .map(|i| entries.swap_remove(i).1)
                        .or(fallback);
                    return Ok(foreign::wrap(
                        chosen.unwrap_or(Self::Empty),
                        optional_transformation(m)?,
                    ));
                };
                Self::RangeDispatch {
                    transformation: optional_transformation(m)?,
                    property,
                    scale: get(m, "scale").map_or(Ok(1.0), float)?,
                    entries,
                    fallback: get(m, "fallback")
                        .map(|f| decode(f).map(Box::new))
                        .transpose()?,
                }
            }
            "minecraft:special" => {
                let model = required(m, "model")?;
                let base = ident(required(m, "base")?)?;
                let transformation = optional_transformation(m)?;
                let fields = object(model)?;
                let kind = sunrise_core::identifier::Identifier::parse(&string(required(
                    fields, "type",
                )?)?)
                .map_err(|_| err("invalid resource identifier"))?;
                // A registration may replace a vanilla decoder. A refusal is an error, not a miss.
                let registered = match decoders.special {
                    Some(load) => load(&kind, &base, fields).map_err(DecodeError)?,
                    None => None,
                };
                match registered {
                    Some(model) => Self::RegisteredSpecial {
                        base: base.into(),
                        transformation,
                        model,
                    },
                    None => Self::Special {
                        base,
                        transformation,
                        model: SpecialModel::decode_builtin(&kind.to_string(), fields)?,
                    },
                }
            }
            "minecraft:bundle/selected_item" => Self::BundleSelectedItem,
            "neoforge:fluid_container" => Self::FluidContainer(FluidContainer::decode(m)?),
            other => return Err(err(&format!("unknown item model type {other}"))),
        })
    }
    /// Every model resource id the tree can ask for.
    pub fn dependencies(&self, out: &mut Vec<String>) {
        match self {
            Self::Empty | Self::BundleSelectedItem | Self::FluidContainer(_) => {}
            Self::Model(leaf) => out.push(leaf.model.clone()),
            Self::Composite { models, .. } => models.iter().for_each(|m| m.dependencies(out)),
            Self::Condition {
                on_true, on_false, ..
            } => {
                on_true.dependencies(out);
                on_false.dependencies(out);
            }
            Self::Select {
                cases, fallback, ..
            } => {
                cases.iter().for_each(|c| c.model.dependencies(out));
                fallback.iter().for_each(|f| f.dependencies(out));
            }
            Self::RangeDispatch {
                entries, fallback, ..
            } => {
                entries.iter().for_each(|(_, m)| m.dependencies(out));
                fallback.iter().for_each(|f| f.dependencies(out));
            }
            Self::Special { base, .. } => out.push(base.clone()),
            Self::RegisteredSpecial { base, .. } => out.push(base.to_string()),
            Self::Registered { models, .. } => {
                models.iter().for_each(|(_, m)| m.dependencies(out));
            }
        }
    }
}
impl SwitchCase {
    fn decode(
        property: &SelectProperty,
        v: &Value,
        decoders: &ItemModelDecoders<'_>,
    ) -> Result<Self> {
        let m = object(v)?;
        let when = required(m, "when")?;
        // `compactListCodec` tries the list codec first, then the original value as one key.
        // An empty list decodes successfully and is then rejected by the nonempty check.
        let decode = |value: &Value| match property {
            SelectProperty::Registered(bound) => {
                bound.decode_case(value).map(SwitchCaseValue::Registered)
            }
            _ => property
                .decode_value(value)
                .map_err(|e| SelectCaseDecodeError::Mismatch(e.0)),
        };
        let values = if let Value::Array(array) = when {
            match array
                .iter()
                .map(decode)
                .collect::<std::result::Result<Vec<_>, _>>()
            {
                Ok(values) if values.is_empty() => return Err(err("list must have contents")),
                Ok(values) => values,
                Err(SelectCaseDecodeError::Mismatch(_)) => vec![decode(when)?],
                Err(error) => return Err(error.into()),
            }
        } else {
            vec![decode(when)?]
        };
        Ok(Self {
            values,
            model: ItemModelNode::decode_with(required(m, "model")?, decoders)?,
        })
    }
}

// ---------------------------------------------------------------------------
// Tint sources
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum TintSource {
    Constant(i32),
    CustomModelData {
        index: i32,
        default: i32,
    },
    Dye(i32),
    Grass {
        temperature: f32,
        downfall: f32,
    },
    Firework(i32),
    Potion(i32),
    Team(i32),
    /// A source bound by a registered tint decoder (`register_item_tint_sources`), read through
    /// [`ItemEnv::item_tint_source`].
    Registered(RegisteredTintSource),
}
impl TintSource {
    fn decode_with(v: &Value, decoders: &ItemModelDecoders<'_>) -> Result<Self> {
        let m = object(v)?;
        let kind = ident(required(m, "type")?)?;
        // Registered tint decoders run ahead of built-ins so vanilla overrides take effect.
        // An error rejects the enclosing item definition and never falls back to a built-in.
        if let Some(load) = decoders.tint
            && let Some(bound) = load(&kind, m).map_err(DecodeError)?
        {
            return Ok(Self::Registered(bound));
        }
        let default = || rgb(required(m, "default")?);
        let unit = |v: &Value| -> Result<f32> {
            let f = float(v)?;
            if (0.0..=1.0).contains(&f) {
                Ok(f)
            } else {
                Err(err("value must be within range [0;1]"))
            }
        };
        Ok(match kind.as_str() {
            "minecraft:constant" => Self::Constant(opaque(rgb(required(m, "value")?)?)),
            "minecraft:custom_model_data" => Self::CustomModelData {
                index: get(m, "index").map_or(Ok(0), non_negative)?,
                default: default()?,
            },
            "minecraft:dye" => Self::Dye(default()?),
            "minecraft:grass" => Self::Grass {
                temperature: unit(required(m, "temperature")?)?,
                downfall: unit(required(m, "downfall")?)?,
            },
            "minecraft:firework" => Self::Firework(default()?),
            "minecraft:potion" => Self::Potion(default()?),
            "minecraft:team" => Self::Team(default()?),
            // A mod's own tint source that no mod registered: untinted here.
            other if foreign::is_foreign(other) => Self::Constant(-1),
            _ => return Err(err("unknown item tint source")),
        })
    }
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum ConditionProperty {
    Custom(String),
    CustomModelData(i32),
    UsingItem,
    Broken,
    Damaged,
    FishingRodCast,
    HasComponent {
        component: String,
        ignore_default: bool,
    },
    BundleHasSelectedItem,
    Selected,
    Carried,
    ExtendedView,
    KeybindDown(String),
    ViewEntity,
    /// A single `component` predicate; external predicate types use the host.
    Component(Value),
}
impl ConditionProperty {
    fn decode(m: &Map<String, Value>) -> Result<Self> {
        Ok(match ident(required(m, "property")?)?.as_str() {
            "minecraft:custom_model_data" => {
                Self::CustomModelData(get(m, "index").map_or(Ok(0), non_negative)?)
            }
            "minecraft:using_item" => Self::UsingItem,
            "minecraft:broken" => Self::Broken,
            "minecraft:damaged" => Self::Damaged,
            "minecraft:fishing_rod/cast" => Self::FishingRodCast,
            "minecraft:has_component" => Self::HasComponent {
                component: component_type(required(m, "component")?)?,
                ignore_default: get(m, "ignore_default").map_or(Ok(false), boolean)?,
            },
            "minecraft:bundle/has_selected_item" => Self::BundleHasSelectedItem,
            "minecraft:selected" => Self::Selected,
            "minecraft:carried" => Self::Carried,
            "minecraft:extended_view" => Self::ExtendedView,
            "minecraft:keybind_down" => Self::KeybindDown(string(required(m, "keybind")?)?),
            "minecraft:view_entity" => Self::ViewEntity,
            "minecraft:component" => Self::Component(component_predicate::decode(m)?),
            _ => return Err(err("unknown condition property")),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SelectProperty {
    /// A property bound by a registered select decoder (`register_select_item_model_property`),
    /// read through [`ItemEnv::item_model_select_property`]; its `when` values are strings.
    Registered(RegisteredSelectProperty),
    CustomModelData(i32),
    MainHand,
    ChargeType,
    TrimMaterial,
    BlockState(String),
    DisplayContext,
    LocalTime {
        pattern: String,
        locale: String,
        time_zone: Option<String>,
    },
    ContextEntityType,
    ContextDimension,
    Component(String),
}
impl SelectProperty {
    /// Decode a `minecraft:select` property, checking registered decoders ahead of built-ins.
    /// Returns `Ok(None)` only for an unregistered foreign property, which collapses to its
    /// fallback.
    fn decode_with(
        m: &Map<String, Value>,
        decoders: &ItemModelDecoders<'_>,
    ) -> Result<Option<Self>> {
        let property = ident(required(m, "property")?)?;
        if let Some(load) = decoders.select_property
            && let Some(bound) = load(&property, m).map_err(DecodeError)?
        {
            return Ok(Some(Self::Registered(bound)));
        }
        if foreign::is_foreign(&property) {
            return Ok(None);
        }
        Ok(Some(match property.as_str() {
            "minecraft:custom_model_data" => {
                Self::CustomModelData(get(m, "index").map_or(Ok(0), non_negative)?)
            }
            "minecraft:main_hand" => Self::MainHand,
            "minecraft:charge_type" => Self::ChargeType,
            "minecraft:trim_material" => Self::TrimMaterial,
            "minecraft:block_state" => {
                Self::BlockState(string(required(m, "block_state_property")?)?)
            }
            "minecraft:display_context" => Self::DisplayContext,
            "minecraft:local_time" => Self::LocalTime {
                pattern: string(required(m, "pattern")?)?,
                locale: get(m, "locale").map_or(Ok(String::new()), string)?,
                time_zone: get(m, "time_zone").map(string).transpose()?,
            },
            "minecraft:context_entity_type" => Self::ContextEntityType,
            "minecraft:context_dimension" => Self::ContextDimension,
            "minecraft:component" => Self::Component(component_type(required(m, "component")?)?),
            _ => return Err(err("unknown select property")),
        }))
    }
    /// A built-in `when` value in canonical form (identifiers normalised).
    fn decode_value(&self, v: &Value) -> Result<SwitchCaseValue> {
        Ok(SwitchCaseValue::Json(match self {
            Self::Registered(_) => unreachable!(),
            Self::CustomModelData(_) | Self::BlockState(_) | Self::LocalTime { .. } => {
                Value::String(string(v)?)
            }
            Self::MainHand => match string(v)?.as_str() {
                s @ ("left" | "right") => Value::String(s.to_owned()),
                _ => return Err(err("invalid arm")),
            },
            Self::ChargeType => match string(v)?.as_str() {
                s @ ("none" | "rocket" | "arrow") => Value::String(s.to_owned()),
                _ => return Err(err("invalid charge type")),
            },
            Self::TrimMaterial | Self::ContextEntityType | Self::ContextDimension => {
                Value::String(ident(v)?)
            }
            Self::DisplayContext => {
                let s = string(v)?;
                DisplayContext::by_name(&s).ok_or_else(|| err("invalid display context"))?;
                Value::String(s)
            }
            Self::Component(_) => v.clone(),
        }))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompassTarget {
    None,
    Lodestone,
    Spawn,
    Recovery,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeSource {
    Random,
    Daytime,
    MoonPhase,
}
/// A `NeedleDirectionHelper.Wobbler`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Wobbler {
    pub rotation: f32,
    pub delta_rotation: f32,
    pub last_update_tick: i64,
    /// Without wobble the needle jumps straight to its target.
    pub target: f32,
}
impl Wobbler {
    pub fn should_update(&self, wobble: bool, tick: i64) -> bool {
        !wobble || self.last_update_tick != tick
    }
    pub fn update(&mut self, wobble: bool, factor: f32, tick: i64, target_rotation: f32) {
        if !wobble {
            self.target = target_rotation;
            return;
        }
        self.last_update_tick = tick;
        let temp = positive_modulo(target_rotation - self.rotation + 0.5, 1.0) - 0.5;
        self.delta_rotation += temp * 0.1;
        self.delta_rotation *= factor;
        self.rotation = positive_modulo(self.rotation + self.delta_rotation, 1.0);
    }
    pub fn value(&self, wobble: bool) -> f32 {
        if wobble { self.rotation } else { self.target }
    }
}
/// `Mth.positiveModulo(float, float)`.
pub fn positive_modulo(input: f32, modulus: f32) -> f32 {
    (input % modulus + modulus) % modulus
}
/// The mutable state a baked `compass` or `time` property keeps.
#[derive(Debug, Default)]
pub struct NeedleState(Mutex<[Wobbler; 2]>);
impl Clone for NeedleState {
    fn clone(&self) -> Self {
        Self::default()
    }
}
impl PartialEq for NeedleState {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum NumericProperty {
    /// A property bound by a registered range decoder (`register_range_select_item_model_property`),
    /// read through [`ItemEnv::item_model_range_property`].
    Registered(RegisteredRangeProperty),
    BundleFullness,
    Compass {
        wobble: bool,
        target: CompassTarget,
        state: Arc<NeedleState>,
    },
    Cooldown,
    Count {
        normalize: bool,
    },
    CrossbowPull,
    CustomModelData(i32),
    Damage {
        normalize: bool,
    },
    Time {
        wobble: bool,
        source: TimeSource,
        state: Arc<NeedleState>,
    },
    UseCycle(f32),
    UseDuration {
        remaining: bool,
    },
}
impl NumericProperty {
    /// Decode a `minecraft:range_dispatch` property, checking registered decoders ahead of
    /// built-ins. Returns `Ok(None)` only for an unregistered foreign property, which reads 0.
    fn decode_with(
        m: &Map<String, Value>,
        decoders: &ItemModelDecoders<'_>,
    ) -> Result<Option<Self>> {
        let normalize = || get(m, "normalize").map_or(Ok(true), boolean);
        let property = ident(required(m, "property")?)?;
        if let Some(load) = decoders.range_property
            && let Some(bound) = load(&property, m).map_err(DecodeError)?
        {
            return Ok(Some(Self::Registered(bound)));
        }
        if foreign::is_foreign(&property) {
            return Ok(None);
        }
        Ok(Some(match property.as_str() {
            "minecraft:custom_model_data" => {
                Self::CustomModelData(get(m, "index").map_or(Ok(0), non_negative)?)
            }
            "minecraft:bundle/fullness" => Self::BundleFullness,
            "minecraft:damage" => Self::Damage {
                normalize: normalize()?,
            },
            "minecraft:cooldown" => Self::Cooldown,
            "minecraft:time" => Self::Time {
                wobble: get(m, "wobble").map_or(Ok(true), boolean)?,
                source: match string(required(m, "source")?)?.as_str() {
                    "random" => TimeSource::Random,
                    "daytime" => TimeSource::Daytime,
                    "moon_phase" => TimeSource::MoonPhase,
                    _ => return Err(err("invalid time source")),
                },
                state: Arc::default(),
            },
            "minecraft:compass" => Self::Compass {
                wobble: get(m, "wobble").map_or(Ok(true), boolean)?,
                target: match string(required(m, "target")?)?.as_str() {
                    "none" => CompassTarget::None,
                    "lodestone" => CompassTarget::Lodestone,
                    "spawn" => CompassTarget::Spawn,
                    "recovery" => CompassTarget::Recovery,
                    _ => return Err(err("invalid compass target")),
                },
                state: Arc::default(),
            },
            "minecraft:crossbow/pull" => Self::CrossbowPull,
            "minecraft:use_cycle" => {
                let period = get(m, "period").map_or(Ok(1.0), float)?;
                if period.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
                    return Err(err("value must be positive"));
                }
                Self::UseCycle(period)
            }
            "minecraft:use_duration" => Self::UseDuration {
                remaining: get(m, "remaining").map_or(Ok(false), boolean)?,
            },
            "minecraft:count" => Self::Count {
                normalize: normalize()?,
            },
            _ => return Err(err("unknown range dispatch property")),
        }))
    }
}

// ---------------------------------------------------------------------------
// Stacks and the world around them
// ---------------------------------------------------------------------------

/// The item components a stack carries: a prototype plus an explicit patch
/// (`None` removes a prototype component).
pub type Components = BTreeMap<String, Value>;

/// A stack's components read in place: its patch first, then its item's defaults. Reading one copies
/// nothing, where [`ItemModels::components`] clones the defaults for a stack with a patch (an enchanted
/// sword's are a few dozen JSON values, on every draw of it).
#[derive(Clone, Copy)]
pub struct ComponentsView<'a> {
    prototype: &'a Components,
    patch: &'a BTreeMap<String, Option<Value>>,
}

impl<'a> ComponentsView<'a> {
    pub fn get(&self, key: &str) -> Option<&'a Value> {
        match self.patch.get(key) {
            Some(patched) => patched.as_ref(),
            None => self.prototype.get(key),
        }
    }
    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }
    /// Components with no patch: `components` as they are.
    pub fn of(components: &'a Components) -> Self {
        static NO_PATCH: BTreeMap<String, Option<Value>> = BTreeMap::new();
        Self {
            prototype: components,
            patch: &NO_PATCH,
        }
    }
    /// The merged map, for what needs one whole.
    pub fn to_map(&self) -> Components {
        let mut out = self.prototype.clone();
        for (key, value) in self.patch {
            match value {
                Some(v) => out.insert(key.clone(), v.clone()),
                None => out.remove(key),
            };
        }
        out
    }
}

/// An item stack as the resolver reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StackView {
    pub item: String,
    pub count: i32,
    pub patch: BTreeMap<String, Option<Value>>,
    /// `BundleContents.selectedItem`, a runtime-only index (-1: none).
    pub bundle_selected: i32,
}
impl StackView {
    pub fn new(item: &str, count: i32) -> Self {
        Self {
            item: item.to_owned(),
            count,
            patch: BTreeMap::new(),
            bundle_selected: -1,
        }
    }
}

/// `ArmorTrim`/`LodestoneTracker`-style positions are plain data.
#[derive(Clone, Debug, PartialEq)]
pub struct GlobalPos {
    pub dimension: String,
    pub pos: [i32; 3],
}
/// The living entity holding the item (`ItemOwner`).
#[derive(Clone, Debug, Default)]
pub struct OwnerState {
    pub position: [f64; 3],
    pub visual_rotation_y_degrees: f32,
    pub entity_type: String,
    pub main_arm_left: bool,
    /// `isUsingItem() && getUseItem() == stack`.
    pub using_this_item: bool,
    pub use_item_remaining_ticks: i32,
    /// `stack.getUseDuration(owner)`.
    pub use_duration: i32,
    /// `CrossbowItem.getChargeDuration(stack, owner)`.
    pub crossbow_charge_duration: i32,
    pub fishing_cast_with_this_item: bool,
    pub is_selected: bool,
    pub is_carried: bool,
    pub is_view_entity: bool,
    pub team_color: Option<i32>,
    /// `ItemCooldowns.getCooldownPercent(stack, 0)`.
    pub cooldown_percent: f32,
    pub is_local_player: bool,
    pub last_death_location: Option<GlobalPos>,
}
/// The `ClientLevel` an item is drawn in.
#[derive(Clone, Debug, Default)]
pub struct LevelState {
    pub dimension: String,
    pub game_time: i64,
    /// `EnvironmentAttributes.SUN_ANGLE` at the owner, in degrees.
    pub sun_angle: f32,
    /// `EnvironmentAttributes.MOON_PHASE` index at the owner.
    pub moon_phase: i32,
    pub respawn: Option<GlobalPos>,
    /// `tickRateManager().runsNormally()`.
    pub runs_normally: bool,
}
/// Everything about the surroundings that is not in the stack.
#[derive(Clone, Debug)]
pub struct Context<'a> {
    pub display: DisplayContext,
    pub owner: Option<&'a OwnerState>,
    pub level: Option<&'a LevelState>,
    pub seed: i32,
}

/// Host services the resolver cannot derive from data alone.
pub trait ItemEnv {
    /// A mod's namespaced conditional property, false when no mod supplies it.
    fn item_model_property(&self, _property: &str, _stack: &StackView) -> bool {
        false
    }
    /// A mod's bound numeric `range_dispatch` property, 0 when no mod answers.
    fn item_model_range_property(
        &self,
        _property: &RegisteredRangeProperty,
        _stack: &StackView,
        _ctx: &Context,
    ) -> f32 {
        0.0
    }
    /// Evaluate and compare a bound categorical `select` property in its callback boundary.
    /// Return the selected case index or `None` to use the fallback.
    fn item_model_select_property(
        &self,
        _property: &RegisteredSelectProperty,
        _stack: &StackView,
        _ctx: &Context,
        _keys: &mut dyn Iterator<Item = (usize, &RegisteredSelectKey)>,
    ) -> Option<usize> {
        None
    }
    /// A mod's bound tint source for `stack`: ARGB, white when no mod answers.
    fn item_tint_source(&self, _source: &RegisteredTintSource, _stack: &StackView) -> i32 {
        -1
    }
    /// The fluid `stack` holds (`FluidUtil.getFirstStackContained`), or `None` when it
    /// holds none. The game answers for vanilla's buckets and asks a mod's own items
    /// (`contained_fluid`); a container nothing answers for draws its declared fluid.
    fn stack_fluid(&self, _stack: &StackView) -> Option<&str> {
        None
    }
    /// What `fluid` draws with in an item: its still sprite and the tint
    /// `FluidContentsTint` gives a stack holding it. `None` for a fluid the host has no
    /// model for, `minecraft:empty` included; the resolver draws the missing sprite for
    /// those, except the empty fluid, which draws nothing.
    fn fluid_appearance(&self, _fluid: &str) -> Option<FluidAppearance> {
        None
    }
    /// A mod's bound node type for `stack` and the surroundings `ItemModel.update` reads (`ctx`):
    /// push the layers to draw, in order, each naming one of the node's `models`. None draw when
    /// no mod answers.
    fn item_model_update(
        &self,
        _model: &RegisteredItemModel,
        _stack: &StackView,
        _ctx: &Context,
        _layers: &mut Vec<ModelChoice>,
    ) {
    }
    /// `PotionContents.getColorOr` for the potion/effects part: the mixed
    /// effect colour, or `None` when the contents have no visible effects.
    fn potion_effect_color(&self, _potion_contents: &Value) -> Option<i32> {
        None
    }
    /// `GrassColor.get(temperature, downfall)`.
    fn grass_color(&self, _temperature: f64, _downfall: f64) -> i32 {
        -65281
    }
    /// `BundleItem.getFullnessDisplay`.
    fn bundle_fullness(&self, _stack: &StackView, _components: &Components) -> f32 {
        0.0
    }
    /// Additional component predicate types supplied by the host.
    fn component_matches(&self, _stack: &StackView, _predicate: &Value) -> bool {
        false
    }
    /// Membership in the level's enchantment registry tags.
    fn enchantment_in_tag(&self, _enchantment: &str, _tag: &str) -> bool {
        false
    }
    fn is_keybind_down(&self, _keybind: &str) -> bool {
        false
    }
    fn has_shift_down(&self) -> bool {
        false
    }
    /// `LocalTime`: the formatted current time for a pattern.
    fn local_time(&self, _pattern: &str, _locale: &str, _time_zone: Option<&str>) -> String {
        String::new()
    }
    /// A random float for `time` (random source) and the spinning compass.
    fn random_float(&self) -> f32 {
        0.0
    }
}
/// An [`ItemEnv`] with the trait defaults.
pub struct NoEnv;
impl ItemEnv for NoEnv {}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoilType {
    None,
    Standard,
    Special,
}
#[derive(Clone, Debug, PartialEq)]
pub enum LayerContent {
    /// A cuboid model resource, baked by [`crate::item_bake`].
    Model { model: String },
    /// One `item/generated` layer of a sprite, its pixels kept only where `mask`'s
    /// are opaque: NeoForge's `bakeItemMaskQuads`.
    Sprite {
        sprite: String,
        mask: Option<String>,
    },
    /// `MissingItemModel`.
    Missing,
    /// A registered special model; the renderer reads it with the stack that selected this layer.
    RegisteredSpecial { model: RegisteredSpecialModel },
    Special {
        base: String,
        model: SpecialModel,
        /// What `extractArgument` reads from the stack.
        argument: Value,
    },
}
/// Whose display transform a layer is drawn under: the layer owns the choice, not the model it
/// draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayerTransform {
    /// The layer's own model's transform for the display context.
    #[default]
    Model,
    /// None: only the centring every item transform ends with
    /// ([`crate::item_bake::item_transform_matrix`] of no transform).
    None,
}
/// What a mod's registered node answers for a stack: one of the node's `models`, by name, and
/// whose display transform it draws under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelChoice {
    pub model: String,
    pub transform: LayerTransform,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub content: LayerContent,
    pub tints: Vec<i32>,
    pub foil: FoilType,
    pub transform: Mat4,
    pub display_transform: LayerTransform,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResolvedItem {
    pub layers: Vec<Layer>,
    pub oversized_in_gui: bool,
}

/// All item definitions and the default components of every item.
#[derive(Clone, Default)]
pub struct ItemModels {
    pub definitions: BTreeMap<String, ClientItem>,
    pub prototypes: Arc<BTreeMap<String, Components>>,
}

impl ItemModels {
    /// The stack's components: its item's defaults with the stack's patch on top. A stack
    /// with no patch (most stacks a screen draws) borrows the defaults instead of copying them.
    pub fn components(&self, stack: &StackView) -> std::borrow::Cow<'_, Components> {
        static NONE: Components = BTreeMap::new();
        let prototype = self.prototypes.get(&stack.item).unwrap_or(&NONE);
        if stack.patch.is_empty() {
            return std::borrow::Cow::Borrowed(prototype);
        }
        std::borrow::Cow::Owned(self.components_view(stack).to_map())
    }
    /// The stack's components, read in place (see [`ComponentsView`]).
    pub fn components_view<'a>(&'a self, stack: &'a StackView) -> ComponentsView<'a> {
        static NONE: Components = BTreeMap::new();
        ComponentsView {
            prototype: self.prototypes.get(&stack.item).unwrap_or(&NONE),
            patch: &stack.patch,
        }
    }
    /// `ItemModelResolver.updateForTopItem`.
    pub fn resolve(&self, stack: &StackView, ctx: &Context, env: &dyn ItemEnv) -> ResolvedItem {
        let mut out = ResolvedItem::default();
        if stack.count > 0 && stack.item != "minecraft:air" {
            self.append_layers(&mut out, stack, ctx, env);
        }
        out
    }
    /// `ItemModelResolver.appendItemLayers`.
    pub fn append_layers(
        &self,
        out: &mut ResolvedItem,
        stack: &StackView,
        ctx: &Context,
        env: &dyn ItemEnv,
    ) {
        if let ControlFlow::Continue(Some(oversized_in_gui)) =
            self.visit_layers(stack, ctx, env, LayerTransform::Model, &mut |layer, _| {
                out.layers.push(layer);
                ControlFlow::Continue(())
            })
        {
            out.oversized_in_gui = oversized_in_gui;
        }
    }
    /// Visit each layer in draw order with the stack it resolved from (which may be a bundle's
    /// selected stack). An inherited [`LayerTransform::None`] suppresses the model transform
    /// for every descendant. A visitor break stops traversal, including within composites and
    /// selected stacks. On completion returns the final `oversized_in_gui`, or `None` if the
    /// stack has no `minecraft:item_model` string. Like [`Self::append_layers`], this does not
    /// filter empty stacks.
    pub fn visit_layers(
        &self,
        stack: &StackView,
        ctx: &Context,
        env: &dyn ItemEnv,
        display_transform: LayerTransform,
        visit: &mut impl FnMut(Layer, &StackView) -> ControlFlow<()>,
    ) -> ControlFlow<(), Option<bool>> {
        let components = self.components_view(stack);
        let Some(Value::String(model_id)) = components.get("minecraft:item_model") else {
            return ControlFlow::Continue(None);
        };
        let id = identifier(model_id).unwrap_or_else(|_| model_id.clone());
        let definition = self.definitions.get(&id);
        let mut state = Resolve {
            models: self,
            stack,
            components,
            ctx,
            env,
            visit,
            oversized_in_gui: definition.is_some_and(|d| d.oversized_in_gui),
        };
        match definition {
            Some(d) => state.node(&d.model, &Mat4::IDENTITY, display_transform),
            None => state.missing(&Mat4::IDENTITY, display_transform),
        }?;
        ControlFlow::Continue(Some(state.oversized_in_gui))
    }
    /// `ItemModelResolver.shouldPlaySwapAnimation`.
    pub fn hand_animation_on_swap(&self, stack: &StackView) -> bool {
        self.properties(stack).0
    }
    /// `ItemModelResolver.swapAnimationScale`.
    pub fn swap_animation_scale(&self, stack: &StackView) -> f32 {
        self.properties(stack).2
    }
    /// The skull kind a `minecraft:head` model at the top of `item`'s default item model names
    /// (`minecraft:item_model`): how a native mod's skull item, whose block the game does not
    /// know as a skull, names the kind it registered (`create_skull_models`). None for an item
    /// whose model is anything else.
    pub fn head_kind(&self, item: &str) -> Option<&str> {
        let Some(Value::String(id)) = self.prototypes.get(item)?.get("minecraft:item_model") else {
            return None;
        };
        // Asked each frame for every worn non-vanilla item: a namespaced id is already the
        // definitions' canonical key, so only a bare one pays for `identifier`'s allocation.
        let definition = match id.split_once(':') {
            Some((namespace, _)) if !namespace.is_empty() => self.definitions.get(id.as_str())?,
            _ => self.definitions.get(&identifier(id).ok()?)?,
        };
        match &definition.model {
            ItemModelNode::Special {
                model:
                    SpecialModel {
                        kind: SpecialKind::Head { kind, .. },
                    },
                ..
            } => Some(kind),
            _ => None,
        }
    }
    fn properties(&self, stack: &StackView) -> (bool, bool, f32) {
        match self.components_view(stack).get("minecraft:item_model") {
            Some(Value::String(id)) => identifier(id)
                .ok()
                .and_then(|id| self.definitions.get(&id))
                .map_or(ClientItem::DEFAULT_PROPERTIES, |d| {
                    (
                        d.hand_animation_on_swap,
                        d.oversized_in_gui,
                        d.swap_animation_scale,
                    )
                }),
            _ => ClientItem::DEFAULT_PROPERTIES,
        }
    }
}

struct Resolve<'a, 'visit, F> {
    models: &'a ItemModels,
    stack: &'a StackView,
    components: ComponentsView<'a>,
    ctx: &'a Context<'a>,
    env: &'a dyn ItemEnv,
    visit: &'visit mut F,
    oversized_in_gui: bool,
}

/// A `Transformation` of scale `(1, 1, z)` as `FaceBakery.bakeVertex` applies a model
/// state's: about `BLOCK_MIDDLE` (0.5, 0.5, 0.5), in the model's 0..1 space.
fn depth_scale(z: f32) -> Mat4 {
    Mat4::IDENTITY
        .translate([0.5; 3])
        .scale([1.0, 1.0, z])
        .translate([-0.5; 3])
}

fn compose(parent: &Mat4, own: &Option<Mat4>) -> Mat4 {
    own.as_ref().map_or(*parent, |t| parent.mul(t))
}

impl<F: FnMut(Layer, &StackView) -> ControlFlow<()>> Resolve<'_, '_, F> {
    fn component(&self, key: &str) -> Option<&Value> {
        self.components.get(key)
    }
    fn has_foil(&self) -> bool {
        if let Some(v) = self.component("minecraft:enchantment_glint_override") {
            return v.as_bool().unwrap_or(false);
        }
        let enchanted = self
            .component("minecraft:enchantments")
            .is_some_and(enchantments_present);
        enchanted
            || (self.stack.item == "minecraft:compass"
                && self.component("minecraft:lodestone_tracker").is_some())
    }
    fn damageable(&self) -> bool {
        self.component("minecraft:max_damage").is_some()
            && self.component("minecraft:unbreakable").is_none()
            && self.component("minecraft:damage").is_some()
    }
    fn max_damage(&self) -> i32 {
        self.component("minecraft:max_damage")
            .and_then(|v| int(v).ok())
            .unwrap_or(0)
    }
    fn damage_value(&self) -> i32 {
        let d = self
            .component("minecraft:damage")
            .and_then(|v| int(v).ok())
            .unwrap_or(0);
        // `Mth.clamp(int, 0, max)`
        if d < 0 { 0 } else { d.min(self.max_damage()) }
    }
    fn max_stack_size(&self) -> i32 {
        self.component("minecraft:max_stack_size")
            .and_then(|v| int(v).ok())
            .unwrap_or(1)
    }
    fn emit(&mut self, layer: Layer) -> ControlFlow<()> {
        (self.visit)(layer, self.stack)
    }
    fn missing(&mut self, transform: &Mat4, display_transform: LayerTransform) -> ControlFlow<()> {
        self.emit(Layer {
            content: LayerContent::Missing,
            tints: Vec::new(),
            foil: FoilType::None,
            transform: *transform,
            display_transform,
        })
    }
    fn node(
        &mut self,
        node: &ItemModelNode,
        parent: &Mat4,
        display_transform: LayerTransform,
    ) -> ControlFlow<()> {
        match node {
            ItemModelNode::Empty => ControlFlow::Continue(()),
            ItemModelNode::Model(leaf) => {
                let transform = compose(parent, &leaf.transformation);
                let foil = if self.has_foil() {
                    if matches!(self.stack.item.as_str(), "minecraft:clock")
                        || self.item_in_compass_tag()
                    {
                        FoilType::Special
                    } else {
                        FoilType::Standard
                    }
                } else {
                    FoilType::None
                };
                let tints = leaf.tints.iter().map(|t| self.tint(t)).collect();
                self.emit(Layer {
                    content: LayerContent::Model {
                        model: leaf.model.clone(),
                    },
                    tints,
                    foil,
                    transform,
                    display_transform,
                })
            }
            ItemModelNode::Composite {
                models,
                transformation,
            } => {
                let child = compose(parent, transformation);
                for m in models {
                    self.node(m, &child, display_transform)?;
                }
                ControlFlow::Continue(())
            }
            ItemModelNode::Condition {
                transformation,
                property,
                on_true,
                on_false,
            } => {
                let child = compose(parent, transformation);
                let chosen = if self.condition(property) {
                    on_true
                } else {
                    on_false
                };
                self.node(chosen, &child, display_transform)
            }
            ItemModelNode::Select {
                transformation,
                property,
                cases,
                fallback,
            } => {
                let child = compose(parent, transformation);
                let chosen = if let SelectProperty::Registered(property) = property {
                    let mut keys = cases.iter().enumerate().flat_map(|(index, case)| {
                        case.values.iter().filter_map(move |value| match value {
                            SwitchCaseValue::Registered(key) => Some((index, key)),
                            SwitchCaseValue::Json(_) => None,
                        })
                    });
                    self.env
                        .item_model_select_property(property, self.stack, self.ctx, &mut keys)
                        .and_then(|index| cases.get(index))
                } else {
                    self.select_value(property).and_then(|v| {
                        cases.iter().find(|c| {
                            c.values.iter().any(
                                |key| matches!(key, SwitchCaseValue::Json(value) if value == &v),
                            )
                        })
                    })
                }
                .map(|case| &case.model);
                match (chosen, fallback) {
                    (Some(m), _) => self.node(m, &child, display_transform),
                    (None, Some(f)) => self.node(f, &child, display_transform),
                    (None, None) => self.missing(&child, display_transform),
                }
            }
            ItemModelNode::RangeDispatch {
                transformation,
                property,
                scale,
                entries,
                fallback,
            } => {
                let child = compose(parent, transformation);
                let value = self.numeric(property) * scale;
                let chosen = if value.is_nan() {
                    None
                } else {
                    last_index_less_or_equal(entries, value).map(|i| &entries[i].1)
                };
                match (chosen, fallback) {
                    (Some(m), _) => self.node(m, &child, display_transform),
                    (None, Some(f)) => self.node(f, &child, display_transform),
                    (None, None) => self.missing(&child, display_transform),
                }
            }
            ItemModelNode::Special {
                base,
                transformation,
                model,
            } => {
                let transform = compose(parent, transformation);
                self.emit(Layer {
                    content: LayerContent::Special {
                        base: base.clone(),
                        model: model.clone(),
                        argument: model.argument(&|k| self.component(k).cloned()),
                    },
                    tints: Vec::new(),
                    foil: if self.has_foil() {
                        FoilType::Standard
                    } else {
                        FoilType::None
                    },
                    transform,
                    display_transform,
                })
            }
            ItemModelNode::RegisteredSpecial {
                transformation,
                model,
                ..
            } => self.emit(Layer {
                content: LayerContent::RegisteredSpecial {
                    model: model.clone(),
                },
                tints: Vec::new(),
                foil: if self.has_foil() {
                    FoilType::Standard
                } else {
                    FoilType::None
                },
                transform: compose(parent, transformation),
                display_transform,
            }),
            ItemModelNode::FluidContainer(container) => {
                self.fluid_container(container, parent, display_transform)
            }
            ItemModelNode::BundleSelectedItem => {
                if let Some(selected) = self.bundle_selected()
                    && let Some(oversized_in_gui) = self.models.visit_layers(
                        &selected,
                        self.ctx,
                        self.env,
                        display_transform,
                        self.visit,
                    )?
                {
                    self.oversized_in_gui = oversized_in_gui;
                }
                ControlFlow::Continue(())
            }
            ItemModelNode::Registered {
                model,
                transformation,
                models,
            } => {
                let mut drawn = Vec::new();
                self.env
                    .item_model_update(model, self.stack, self.ctx, &mut drawn);
                let child = compose(parent, transformation);
                for choice in &drawn {
                    if let Some((_, model)) = models.iter().find(|(key, _)| *key == choice.model) {
                        // The model's own transform is kept only where no layer above refused it.
                        let display_transform = if display_transform == LayerTransform::None {
                            display_transform
                        } else {
                            choice.transform
                        };
                        self.node(model, &child, display_transform)?;
                    }
                }
                ControlFlow::Continue(())
            }
        }
    }
    /// `DynamicFluidContainerModel.bakeModelForFluid`: the base layer, then the held
    /// fluid's still sprite through the `fluid` mask, then the cover. The fluid layer is
    /// baked under `FLUID_TRANSFORM` and the cover under `COVER_TRANSFORM` (offsets 403-415,
    /// 617-629), Z scales of 1.002 and 1.004 (`<clinit>` 0-83) that `FaceBakery` applies
    /// about the block's middle: each sits a little further out than the one below it.
    fn fluid_container(
        &mut self,
        container: &FluidContainer,
        parent: &Mat4,
        display_transform: LayerTransform,
    ) -> ControlFlow<()> {
        let textures = &container.textures;
        // `update`: the fluid the stack holds, else the model's own (offsets 0-29).
        let held = self.env.stack_fluid(self.stack);
        let drawn = held.unwrap_or(&container.fluid);
        if let Some(base) = &textures.base {
            self.sprite_layer(base, None, Vec::new(), parent, display_transform)?;
        }
        // `fluid != EMPTY ? FluidStateModelSet.get(fluid).stillMaterial() : null` (offsets
        // 129-145): a fluid the host has no model for gets `ModelBakery.MissingModels.fluid`,
        // the missing sprite with no tint source, so it draws `missingno` untinted.
        let appearance = self.env.fluid_appearance(drawn).or_else(|| {
            (drawn != "minecraft:empty").then(|| FluidAppearance {
                still: crate::item_bake::MISSING_SPRITE.to_owned(),
                tint: -1,
            })
        });
        if let (Some(mask), Some(appearance)) = (&textures.fluid, appearance) {
            // `FluidContentsTint` colours the layer from the fluid the stack holds, not the one
            // drawn: a stack holding none gets the empty fluid's model, which has no tint source,
            // so -1, even while the declared fluid is drawn. A stack that holds one draws that
            // same fluid, so its tint is the appearance already looked up.
            let tint = if held.is_some() { appearance.tint } else { -1 };
            let parent = parent.mul(&depth_scale(1.002));
            self.sprite_layer(
                &appearance.still,
                Some(mask),
                vec![tint],
                &parent,
                display_transform,
            )?;
        }
        // The cover is always the mask (`bakeItemMaskQuads(baker, 0, cover, drawn, ...)`, offsets
        // 631-639). With `cover_is_mask` it picks out the parts of `base` drawn again over the
        // fluid, so it needs a base (offsets 147-164); otherwise the cover draws itself.
        if let Some(cover) = &textures.cover {
            let drawn = if container.cover_is_mask {
                textures.base.as_deref()
            } else {
                Some(cover.as_str())
            };
            if let Some(drawn) = drawn {
                let parent = parent.mul(&depth_scale(1.004));
                self.sprite_layer(drawn, Some(cover), Vec::new(), &parent, display_transform)?;
            }
        }
        ControlFlow::Continue(())
    }
    /// One `item/generated` layer, which [`crate::item_bake::bake_sprite_layer`] extrudes.
    fn sprite_layer(
        &mut self,
        sprite: &str,
        mask: Option<&str>,
        tints: Vec<i32>,
        parent: &Mat4,
        display_transform: LayerTransform,
    ) -> ControlFlow<()> {
        self.emit(Layer {
            content: LayerContent::Sprite {
                sprite: sprite.to_owned(),
                mask: mask.map(str::to_owned),
            },
            tints,
            foil: if self.has_foil() {
                FoilType::Standard
            } else {
                FoilType::None
            },
            transform: *parent,
            display_transform,
        })
    }
    fn item_in_compass_tag(&self) -> bool {
        // `ItemTags.COMPASSES`: the compass and the recovery compass.
        matches!(
            self.stack.item.as_str(),
            "minecraft:compass" | "minecraft:recovery_compass"
        )
    }
    /// `BundleItem.getSelectedItem`: the template at the selected index.
    fn bundle_selected(&self) -> Option<StackView> {
        let items = self.component("minecraft:bundle_contents")?.as_array()?;
        let template = items.get(usize::try_from(self.stack.bundle_selected).ok()?)?;
        stack_from_template(template)
    }
    fn condition(&self, property: &ConditionProperty) -> bool {
        let owner = self.ctx.owner;
        match property {
            ConditionProperty::Custom(property) => {
                self.env.item_model_property(property, self.stack)
            }
            ConditionProperty::CustomModelData(i) => {
                self.component("minecraft:custom_model_data")
                    .and_then(|d| custom_data_at(d, "flags", *i))
                    .and_then(|v| v.as_bool())
                    == Some(true)
            }
            ConditionProperty::UsingItem => owner.is_some_and(|o| o.using_this_item),
            ConditionProperty::Broken => {
                self.damageable() && self.damage_value() >= self.max_damage() - 1
            }
            ConditionProperty::Damaged => self.damageable() && self.damage_value() > 0,
            ConditionProperty::FishingRodCast => {
                owner.is_some_and(|o| o.fishing_cast_with_this_item)
            }
            ConditionProperty::HasComponent {
                component,
                ignore_default,
            } => {
                if *ignore_default {
                    self.stack.patch.contains_key(component)
                } else {
                    self.components.contains_key(component)
                }
            }
            ConditionProperty::BundleHasSelectedItem => self.bundle_selected().is_some(),
            ConditionProperty::Selected => {
                owner.is_some_and(|o| o.is_local_player && o.is_selected)
            }
            ConditionProperty::Carried => owner.is_some_and(|o| o.is_local_player && o.is_carried),
            ConditionProperty::ExtendedView => {
                self.ctx.display == DisplayContext::Gui && self.env.has_shift_down()
            }
            ConditionProperty::KeybindDown(key) => self.env.is_keybind_down(key),
            ConditionProperty::ViewEntity => owner.is_some_and(|o| o.is_view_entity),
            ConditionProperty::Component(predicate) => {
                component_predicate::matches(predicate, self.components, self.env)
                    .unwrap_or_else(|| self.env.component_matches(self.stack, predicate))
            }
        }
    }
    fn select_value(&self, property: &SelectProperty) -> Option<Value> {
        let owner = self.ctx.owner;
        match property {
            SelectProperty::Registered(_) => unreachable!(),
            SelectProperty::CustomModelData(i) => self
                .component("minecraft:custom_model_data")
                .and_then(|d| custom_data_at(d, "strings", *i))
                .cloned(),
            SelectProperty::MainHand => owner
                .map(|o| Value::String(if o.main_arm_left { "left" } else { "right" }.to_owned())),
            SelectProperty::ChargeType => {
                let projectiles = self
                    .component("minecraft:charged_projectiles")
                    .and_then(Value::as_array);
                let kind = match projectiles {
                    None => "none",
                    Some(p) if p.is_empty() => "none",
                    Some(p) => {
                        if p.iter()
                            .any(|t| template_id(t) == Some("minecraft:firework_rocket"))
                        {
                            "rocket"
                        } else {
                            "arrow"
                        }
                    }
                };
                Some(Value::String(kind.to_owned()))
            }
            SelectProperty::TrimMaterial => {
                let material = self
                    .component("minecraft:trim")?
                    .get("material")?
                    .as_str()?;
                identifier(material).ok().map(Value::String)
            }
            SelectProperty::BlockState(name) => self
                .component("minecraft:block_state")
                .and_then(|v| v.get("properties").or(Some(v)))
                .and_then(|p| p.get(name))
                .map(|v| Value::String(v.as_str().map_or_else(|| v.to_string(), str::to_owned))),
            SelectProperty::DisplayContext => {
                Some(Value::String(self.ctx.display.name().to_owned()))
            }
            SelectProperty::LocalTime {
                pattern,
                locale,
                time_zone,
            } => Some(Value::String(self.env.local_time(
                pattern,
                locale,
                time_zone.as_deref(),
            ))),
            SelectProperty::ContextEntityType => {
                owner.map(|o| Value::String(o.entity_type.clone()))
            }
            SelectProperty::ContextDimension => {
                self.ctx.level.map(|l| Value::String(l.dimension.clone()))
            }
            SelectProperty::Component(component) => self.component(component).cloned(),
        }
    }
    fn numeric(&self, property: &NumericProperty) -> f32 {
        let owner = self.ctx.owner;
        match property {
            NumericProperty::Registered(property) => self
                .env
                .item_model_range_property(property, self.stack, self.ctx),
            NumericProperty::BundleFullness => self
                .env
                .bundle_fullness(self.stack, &self.components.to_map()),
            NumericProperty::Compass {
                wobble,
                target,
                state,
            } => self.compass(*wobble, *target, state),
            NumericProperty::Cooldown => owner.map_or(0.0, |o| o.cooldown_percent),
            NumericProperty::Count { normalize } => {
                let count = self.stack.count as f32;
                let max = self.max_stack_size() as f32;
                if *normalize {
                    clamp(count / max, 0.0, 1.0)
                } else {
                    clamp(count, 0.0, max)
                }
            }
            NumericProperty::CrossbowPull => {
                let Some(o) = owner else { return 0.0 };
                if self
                    .component("minecraft:charged_projectiles")
                    .and_then(Value::as_array)
                    .is_some_and(|p| !p.is_empty())
                {
                    return 0.0;
                }
                (o.use_duration - o.use_item_remaining_ticks) as f32
                    / o.crossbow_charge_duration as f32
            }
            NumericProperty::CustomModelData(i) => self
                .component("minecraft:custom_model_data")
                .and_then(|d| custom_data_at(d, "floats", *i))
                .and_then(|v| float(v).ok())
                .unwrap_or(0.0),
            NumericProperty::Damage { normalize } => {
                let damage = self.damage_value() as f32;
                let max = self.max_damage() as f32;
                if *normalize {
                    clamp(damage / max, 0.0, 1.0)
                } else {
                    clamp(damage, 0.0, max)
                }
            }
            NumericProperty::Time {
                wobble,
                source,
                state,
            } => self.time(*wobble, *source, state),
            NumericProperty::UseCycle(period) => match owner {
                Some(o) if o.using_this_item => (o.use_item_remaining_ticks as f32) % period,
                _ => 0.0,
            },
            NumericProperty::UseDuration { remaining } => match owner {
                Some(o) if o.using_this_item => {
                    if *remaining {
                        o.use_item_remaining_ticks as f32
                    } else {
                        (o.use_duration - o.use_item_remaining_ticks) as f32
                    }
                }
                _ => 0.0,
            },
        }
    }
    /// `Time.calculate` (through `NeedleDirectionHelper.get`).
    fn time(&self, wobble: bool, source: TimeSource, state: &NeedleState) -> f32 {
        let (Some(_), Some(level)) = (self.ctx.owner, self.ctx.level) else {
            return 0.0;
        };
        let target = match source {
            TimeSource::Random => self.env.random_float(),
            TimeSource::Daytime => level.sun_angle / 360.0,
            TimeSource::MoonPhase => level.moon_phase as f32 / 8.0,
        };
        let mut wobblers = state.0.lock().unwrap_or_else(|e| e.into_inner());
        let w = &mut wobblers[0];
        if w.should_update(wobble, level.game_time) {
            w.update(wobble, 0.9, level.game_time, target);
        }
        w.value(wobble)
    }
    /// `CompassAngleState.calculate`.
    fn compass(&self, wobble: bool, target: CompassTarget, state: &NeedleState) -> f32 {
        let (Some(owner), Some(level)) = (self.ctx.owner, self.ctx.level) else {
            return 0.0;
        };
        let target_pos: Option<GlobalPos> = match target {
            CompassTarget::None => None,
            CompassTarget::Lodestone => self
                .component("minecraft:lodestone_tracker")
                .and_then(|t| t.get("target"))
                .and_then(|t| {
                    Some(GlobalPos {
                        dimension: identifier(t.get("dimension")?.as_str()?).ok()?,
                        pos: {
                            let p = t.get("pos")?.as_array()?;
                            [
                                int(p.first()?).ok()?,
                                int(p.get(1)?).ok()?,
                                int(p.get(2)?).ok()?,
                            ]
                        },
                    })
                }),
            CompassTarget::Spawn => level.respawn.clone(),
            CompassTarget::Recovery => owner.last_death_location.clone(),
        };
        let valid = target_pos.as_ref().filter(|t| {
            t.dimension == level.dimension && {
                let c = [
                    f64::from(t.pos[0]) + 0.5,
                    f64::from(t.pos[1]) + 0.5,
                    f64::from(t.pos[2]) + 0.5,
                ];
                let d = [
                    c[0] - owner.position[0],
                    c[1] - owner.position[1],
                    c[2] - owner.position[2],
                ];
                // `distToCenterSqr(position) < 1.0E-5F`
                let dist = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                dist.is_nan() || dist >= f64::from(1.0E-5_f32)
            }
        });
        let mut wobblers = state.0.lock().unwrap_or_else(|e| e.into_inner());
        let game_time = level.game_time;
        let Some(target) = valid else {
            // `getRandomlySpinningRotation`
            let no_target = &mut wobblers[1];
            if no_target.should_update(wobble, game_time) {
                let r = self.env.random_float();
                no_target.update(wobble, 0.8, game_time, r);
            }
            let hash = self.ctx.seed.wrapping_mul(1_327_217_883);
            let rotation = no_target.value(wobble) + hash as f32 / 2.147_483_6E9;
            return positive_modulo(rotation, 1.0);
        };
        let center = [
            f64::from(target.pos[0]) + 0.5,
            f64::from(target.pos[2]) + 0.5,
        ];
        let angle_to_target = ((center[1] - owner.position[2]).atan2(center[0] - owner.position[0])
            / f64::from(std::f32::consts::PI * 2.0)) as f32;
        let owner_y = positive_modulo(owner.visual_rotation_y_degrees / 360.0, 1.0);
        let rotation = if owner.is_local_player && level.runs_normally {
            let w = &mut wobblers[0];
            if w.should_update(wobble, game_time) {
                w.update(wobble, 0.8, game_time, 0.5 - (owner_y - 0.25));
            }
            angle_to_target + w.value(wobble)
        } else {
            0.5 - (owner_y - 0.25 - angle_to_target)
        };
        positive_modulo(rotation, 1.0)
    }
    fn tint(&self, source: &TintSource) -> i32 {
        match source {
            TintSource::Constant(v) => *v,
            TintSource::CustomModelData { index, default } => self
                .component("minecraft:custom_model_data")
                .and_then(|d| custom_data_at(d, "colors", *index))
                .and_then(|v| rgb(v).ok())
                .map_or(opaque(*default), opaque),
            TintSource::Dye(default) => match self.component("minecraft:dyed_color") {
                Some(v) => opaque(rgb(v.get("rgb").unwrap_or(v)).unwrap_or(0)),
                None => *default,
            },
            TintSource::Grass {
                temperature,
                downfall,
            } => self
                .env
                .grass_color(f64::from(*temperature), f64::from(*downfall)),
            TintSource::Firework(default) => {
                let colors: Vec<i32> = self
                    .component("minecraft:firework_explosion")
                    .and_then(|e| e.get("colors"))
                    .and_then(Value::as_array)
                    .map(|c| c.iter().filter_map(|v| int(v).ok()).collect())
                    .unwrap_or_default();
                match colors.len() {
                    0 => *default,
                    1 => opaque(colors[0]),
                    n => {
                        let n = n as i32;
                        let sum = |shift: u32| -> i32 {
                            colors.iter().map(|c| (c >> shift) & 255).sum::<i32>() / n
                        };
                        argb(255, sum(16), sum(8), sum(0))
                    }
                }
            }
            TintSource::Potion(default) => match self.component("minecraft:potion_contents") {
                Some(contents) => {
                    let custom = contents.get("custom_color").and_then(|c| int(c).ok());
                    opaque(
                        custom
                            .or_else(|| self.env.potion_effect_color(contents))
                            .unwrap_or(*default),
                    )
                }
                None => opaque(*default),
            },
            TintSource::Team(default) => opaque(
                self.ctx
                    .owner
                    .and_then(|o| o.team_color)
                    .unwrap_or(*default),
            ),
            TintSource::Registered(source) => self.env.item_tint_source(source, self.stack),
        }
    }
}

/// `Mth.clamp(float, min, max)` with `Math.min`'s NaN propagation.
fn clamp(v: f32, min: f32, max: f32) -> f32 {
    if v < min { min } else { java_min(v, max) }
}
fn java_min(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        a
    } else if b.is_nan() {
        b
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_negative() { a } else { b }
    } else if a < b {
        a
    } else {
        b
    }
}
fn enchantments_present(v: &Value) -> bool {
    // `ItemEnchantments` is a level map, optionally wrapped as {"levels": {...}}.
    let map = v.get("levels").unwrap_or(v);
    map.as_object().is_some_and(|m| !m.is_empty())
}
fn custom_data_at<'a>(data: &'a Value, list: &str, index: i32) -> Option<&'a Value> {
    data.get(list)?
        .as_array()?
        .get(usize::try_from(index).ok()?)
}
fn template_id(t: &Value) -> Option<&str> {
    t.get("id")?.as_str()
}
/// `ItemStackTemplate.create`: id, count and a component patch.
fn stack_from_template(template: &Value) -> Option<StackView> {
    let id = identifier(template.get("id")?.as_str()?).ok()?;
    let count = template.get("count").map_or(Some(1), |c| int(c).ok())?;
    let mut stack = StackView::new(&id, count);
    if let Some(components) = template.get("components").and_then(Value::as_object) {
        for (key, value) in components {
            match key.strip_prefix('!') {
                Some(removed) => {
                    stack.patch.insert(identifier(removed).ok()?, None);
                }
                None => {
                    stack
                        .patch
                        .insert(identifier(key).ok()?, Some(value.clone()));
                }
            }
        }
    }
    Some(stack)
}

/// `RangeSelectItemModel.lastIndexLessOrEqual`.
fn last_index_less_or_equal(entries: &[(f32, ItemModelNode)], needle: f32) -> Option<usize> {
    let found = if entries.len() < 16 {
        entries
            .iter()
            .position(|e| e.0 > needle)
            .map_or(entries.len() as i64 - 1, |i| i as i64 - 1)
    } else {
        match binary_search(entries, needle) {
            Ok(index) => index as i64,
            Err(insertion) => insertion as i64 - 1,
        }
    };
    usize::try_from(found).ok()
}
/// `Arrays.binarySearch(float[], float)`, including its handling of equal
/// values (-0.0 below 0.0, NaN above everything).
fn binary_search(entries: &[(f32, ItemModelNode)], key: f32) -> std::result::Result<usize, usize> {
    let (mut low, mut high) = (0i64, entries.len() as i64 - 1);
    let bits = |v: f32| -> i32 {
        if v.is_nan() {
            0x7fc0_0000
        } else {
            v.to_bits() as i32
        }
    };
    while low <= high {
        let mid = (low + high) >> 1;
        let value = entries[mid as usize].0;
        if value < key {
            low = mid + 1;
        } else if value > key {
            high = mid - 1;
        } else {
            let (mid_bits, key_bits) = (bits(value), bits(key));
            if mid_bits == key_bits {
                return Ok(mid as usize);
            } else if mid_bits < key_bits {
                low = mid + 1;
            } else {
                high = mid - 1;
            }
        }
    }
    Err(low as usize)
}

#[cfg(test)]
mod fluid_container_tests;
#[cfg(test)]
mod foreign_tests;
#[cfg(test)]
mod tests;
