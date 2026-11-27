//! Data-driven enchantments (`data/minecraft/enchantment/*.json`): the
//! registry, ordered enchantment tags, `EnchantmentHelper` selection and cost
//! rules used by the enchanting table, display names, and evaluation of the
//! value/attribute effect components (`LevelBasedValue`, `EnchantmentValueEffect`
//! and the loot-condition subset that enchantment requirements use).
mod holder_sets;
mod providers;
use crate::item_data::{CodecBool, Levels};
use crate::items::{ItemStack, Items};
pub use holder_sets::HolderSetBinding;
pub use providers::{EnchantmentProvider, EnchantmentSet};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use sunrise_core::nbt::Tag;
use sunrise_core::random::{LegacyRandom, RandomSource};

impl Items {
    /// Vanilla's enchantment-provider tag gives 1.0; non-members give 0.0.
    /// Parts may override this answer; tag membership is not a separate particle-provider test.
    pub fn default_enchant_power(&self, block: &str) -> f32 {
        if self.block_in_tag("minecraft:enchantment_power_provider", block) {
            1.0
        } else {
            0.0
        }
    }
}

use mod_vocab::__private::{
    BorrowedRandom, BoundEnchantmentEntityEffect, BoundEnchantmentValueEffect,
    EnchantmentEffectRegistry, builtin_enchantment_entity_kind, builtin_enchantment_value_kind,
};

static EFFECT_TYPES: std::sync::OnceLock<EnchantmentEffectRegistry> = std::sync::OnceLock::new();

/// Freeze the two effect type registries before loading any enchantment definitions.
pub fn install_effect_types(types: EnchantmentEffectRegistry) {
    assert!(
        EFFECT_TYPES.set(types).is_ok(),
        "enchantment effect types already registered"
    );
}

const VALUE_LISTS: &[&str] = &[
    "minecraft:damage_protection",
    "minecraft:damage",
    "minecraft:smash_damage_per_fallen_block",
    "minecraft:knockback",
    "minecraft:armor_effectiveness",
    "minecraft:item_damage",
    "minecraft:equipment_drops",
    "minecraft:ammo_use",
    "minecraft:projectile_piercing",
    "minecraft:projectile_spread",
    "minecraft:projectile_count",
    "minecraft:trident_return_acceleration",
    "minecraft:fishing_time_reduction",
    "minecraft:fishing_luck_bonus",
    "minecraft:block_experience",
    "minecraft:mob_experience",
    "minecraft:repair_with_xp",
];
const SINGLE_VALUES: &[&str] = &[
    "minecraft:crossbow_charge_time",
    "minecraft:trident_spin_attack_strength",
];
const ENTITY_LISTS: &[&str] = &[
    "minecraft:post_attack",
    "minecraft:post_piercing_attack",
    "minecraft:hit_block",
    "minecraft:tick",
    "minecraft:projectile_spawned",
];

#[derive(Clone)]
enum ValueEffect {
    Builtin(Value),
    AllOf(Vec<Self>),
    Native(BoundEnchantmentValueEffect),
}

/// One decoded entity-effect node, including ordered composition.
#[derive(Clone)]
pub enum EntityEffect {
    /// An engine effect and its data.
    Builtin(Value),
    /// Consecutive child effects on the same target.
    AllOf(Vec<Self>),
    /// A native codec's bound behavior.
    Native(BoundEnchantmentEntityEffect),
}

fn effect_kind(effect: &Value) -> Result<std::borrow::Cow<'_, str>, String> {
    let kind = effect["type"]
        .as_str()
        .ok_or("enchantment effect is missing its type")?;
    Ok(if kind.contains(':') {
        kind.into()
    } else {
        format!("minecraft:{kind}").into()
    })
}

fn effect_children(effect: &Value) -> Result<&[Value], String> {
    effect["effects"]
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| "all_of.effects must be a list".into())
}

impl ValueEffect {
    fn decode(effect: &Value, types: &EnchantmentEffectRegistry) -> Result<Self, String> {
        let kind = effect_kind(effect)?;
        match kind.as_ref() {
            "minecraft:all_of" => effect_children(effect)?
                .iter()
                .map(|child| Self::decode(child, types))
                .collect::<Result<_, _>>()
                .map(Self::AllOf),
            kind if builtin_enchantment_value_kind(kind) => Ok(Self::Builtin(effect.clone())),
            kind => types.decode_value(kind, effect).map(Self::Native),
        }
    }

    fn process(&self, level: u32, random: &mut LegacyRandom, value: f32) -> f32 {
        match self {
            Self::Builtin(effect) => apply_value_effect(effect, level, random, value),
            Self::AllOf(nodes) => nodes
                .iter()
                .fold(value, |value, node| node.process(level, random, value)),
            Self::Native(effect) => effect.process(level, &mut BorrowedRandom(random), value),
        }
    }
}

impl EntityEffect {
    /// Test only engine-owned payloads; native fields were validated by their codec.
    pub fn all_builtins_match(&self, predicate: fn(&Value) -> bool) -> bool {
        match self {
            Self::Builtin(effect) => predicate(effect),
            Self::AllOf(effects) => effects
                .iter()
                .all(|effect| effect.all_builtins_match(predicate)),
            Self::Native(_) => true,
        }
    }

    fn decode(effect: &Value, types: &EnchantmentEffectRegistry) -> Result<Self, String> {
        let kind = effect_kind(effect)?;
        match kind.as_ref() {
            "minecraft:all_of" => effect_children(effect)?
                .iter()
                .map(|child| Self::decode(child, types))
                .collect::<Result<_, _>>()
                .map(Self::AllOf),
            kind if builtin_enchantment_entity_kind(kind) => Ok(Self::Builtin(effect.clone())),
            kind => types.decode_entity(kind, effect).map(Self::Native),
        }
    }
}

#[derive(Clone, Default)]
struct BoundEffects {
    values: HashMap<&'static str, Vec<ValueEffect>>,
    entities: HashMap<&'static str, Vec<EntityEffect>>,
}

impl BoundEffects {
    fn decode(effects: &Value, types: &EnchantmentEffectRegistry) -> Result<Self, String> {
        let entries = |component: &str| -> Result<&[Value], String> {
            effects[component]
                .as_array()
                .map(Vec::as_slice)
                .ok_or_else(|| format!("{component} expects a list of effects"))
        };
        let mut bound = Self::default();
        for &component in VALUE_LISTS {
            if effects.get(component).is_some() {
                let nodes = entries(component)?
                    .iter()
                    .map(|entry| ValueEffect::decode(&entry["effect"], types))
                    .collect::<Result<_, _>>()
                    .map_err(|err| format!("{component}: {err}"))?;
                bound.values.insert(component, nodes);
            }
        }
        for &component in SINGLE_VALUES {
            if let Some(effect) = effects.get(component) {
                let effect = ValueEffect::decode(effect, types)
                    .map_err(|err| format!("{component}: {err}"))?;
                bound.values.insert(component, vec![effect]);
            }
        }
        for &component in ENTITY_LISTS {
            if effects.get(component).is_some() {
                let nodes = entries(component)?
                    .iter()
                    .map(|entry| EntityEffect::decode(&entry["effect"], types))
                    .collect::<Result<_, _>>()
                    .map_err(|err| format!("{component}: {err}"))?;
                bound.entities.insert(component, nodes);
            }
        }
        Ok(bound)
    }
}

/// Levels as numbers in enchantment names ("Sharpness 5"): Sunrise's PvP & HUD option, off by
/// default (vanilla's numerals).
static NUMERIC_LEVELS: AtomicBool = AtomicBool::new(false);
pub fn set_numeric_levels(on: bool) {
    NUMERIC_LEVELS.store(on, Ordering::Relaxed);
}

/// `Enchantment.Cost`: base + per_level_above_first * (level - 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cost {
    pub base: i32,
    pub per_level: i32,
}
impl Cost {
    fn decoded_fields(v: &Value) -> (Option<i32>, Option<i32>) {
        let int = |name| v[name].as_i64().and_then(|n| i32::try_from(n).ok());
        (int("base"), int("per_level_above_first"))
    }

    fn parse(v: &Value) -> Self {
        let (base, per_level) = Self::decoded_fields(v);
        Self {
            base: base.unwrap_or(1),
            per_level: per_level.unwrap_or(0),
        }
    }
    /// `Enchantment.Cost.calculate`: arithmetic wraps like Java `int`.
    pub fn calculate(self, level: u32) -> i32 {
        self.base
            .wrapping_add(self.per_level.wrapping_mul((level as i32).wrapping_sub(1)))
    }
}
#[derive(Clone, Debug)]
pub struct Enchantment {
    pub id: Arc<str>,
    /// The name the pinned English map gave at load: the fallback for a
    /// language that has no entry for the key below.
    pub description: String,
    /// `description`'s translation key (`enchantment.minecraft.<name>`).
    pub description_key: String,
    /// `Enchantment.description`, the definition's component as written: a translatable key
    /// with its fallback and arguments, or a literal, with its style.
    pub description_text: sunrise_game_data::text::Text,
    pub max_level: u32,
    pub weight: u32,
    pub anvil_cost: u32,
    pub min_cost: Cost,
    pub max_cost: Cost,
    /// `supported_items` / `primary_items`: item ids or `#tags`.
    pub supported: Vec<String>,
    pub primary: Option<Vec<String>>,
    pub exclusive: HashSet<Arc<str>>,
    pub slots: Vec<String>,
    pub effects: Value,
}
/// Item components the enchanting, anvil and smithing rules read.
#[derive(Clone, Debug, Default)]
pub struct ItemProps {
    /// `minecraft:enchantable.value`.
    pub enchantable: u32,
    /// `minecraft:repairable.items` (ids or a `#tag`).
    pub repairable: Vec<String>,
    /// The prototype override; absence differs from an explicit false.
    pub glint: Option<bool>,
    pub rarity: String,
    /// `minecraft:provides_trim_material`.
    pub trim_material: Option<String>,
    /// `minecraft:brewing_fuel` present.
    pub brewing_fuel: bool,
}
#[derive(Clone, Default)]
pub struct Enchantments {
    pub list: Vec<Enchantment>,
    index: BTreeMap<Arc<str>, usize>,
    bound_effects: HashMap<Arc<str>, BoundEffects>,
    /// Enchantment tags in vanilla `HolderSet` order (nested tags expanded in place).
    pub tags: HashMap<String, Vec<Arc<str>>>,
    pub entity_tags: Arc<HashMap<String, HashSet<String>>>,
    pub damage_tags: HashMap<String, HashSet<String>>,
    pub props: HashMap<Arc<str>, ItemProps>,
    lang: HashMap<String, String>,
    /// The mods' `isPrimaryItemFor` answers resolved for this snapshot
    /// (`ItemPart::is_primary_item_for`).
    primary_answers: sunrise_world::enchantment_answers::AnswerTable,
    /// The item's support answer, resolved at install for this registry snapshot.
    support_answers: sunrise_world::enchantment_answers::AnswerTable,
    /// `minecraft:enchantment_provider` registry entries frozen for this world.
    providers: BTreeMap<String, EnchantmentProvider>,
}

fn string_list(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => vec![s.clone()],
        Value::Array(a) => a
            .iter()
            .filter_map(|e| e.as_str().map(str::to_owned))
            .collect(),
        _ => vec![],
    }
}
/// Roman numerals as in `enchantment.level.N` (falls back to the number).
pub fn level_name(level: u32) -> String {
    const NAMES: [&str; 10] = ["I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X"];
    NAMES
        .get(level.wrapping_sub(1) as usize)
        .map_or_else(|| level.to_string(), |s| (*s).to_owned())
}

/// `Enchantment.getFullname`: append the level unless both it and the holder's maximum are one.
pub fn format_full_name(
    mut description: String,
    level: u32,
    max_level: u32,
    suffix: impl FnOnce(u32) -> String,
) -> String {
    if level != 1 || max_level != 1 {
        description.push(' ');
        description.push_str(&suffix(level));
    }
    description
}

/// Decode a winning pack definition's top-level fields for registry sync.
/// Holder sets and effect payloads are bound after the registry phase.
pub fn decode_pack_definition(mut value: Value) -> Result<Value, String> {
    let fields = value
        .as_object_mut()
        .ok_or("expected an enchantment object")?;
    for (name, range) in [
        ("max_level", 1..=255),
        ("weight", 1..=1024),
        ("anvil_cost", 0..=i64::from(i32::MAX)),
    ] {
        let n = fields
            .get(name)
            .and_then(Value::as_i64)
            .filter(|n| range.contains(n))
            .ok_or_else(|| format!("{name} must be an integer in {range:?}"))?;
        fields.insert(name.into(), Value::from(n));
    }
    for name in ["min_cost", "max_cost"] {
        let cost = fields
            .get(name)
            .ok_or_else(|| format!("{name} is required"))?;
        let (base, per_level) = Cost::decoded_fields(cost);
        let cost = Cost {
            base: base.ok_or_else(|| format!("{name}.base must be an integer"))?,
            per_level: per_level
                .ok_or_else(|| format!("{name}.per_level_above_first must be an integer"))?,
        };
        fields.insert(
            name.into(),
            serde_json::json!({"base": cost.base,"per_level_above_first": cost.per_level}),
        );
    }
    for name in ["supported_items", "primary_items", "exclusive_set"] {
        match fields.get(name) {
            Some(Value::String(_) | Value::Array(_) | Value::Object(_)) => {}
            None | Some(Value::Null) if name != "supported_items" => {
                fields.remove(name);
            }
            _ => return Err(format!("{name} must be a holder set")),
        }
    }
    let slots = fields
        .get("slots")
        .and_then(Value::as_array)
        .ok_or("slots must be a list of equipment slot groups")?;
    for slot in slots {
        if !matches!(
            slot.as_str(),
            Some(
                "any"
                    | "mainhand"
                    | "offhand"
                    | "hand"
                    | "feet"
                    | "legs"
                    | "chest"
                    | "head"
                    | "armor"
                    | "body"
                    | "saddle"
            )
        ) {
            return Err("slots must contain equipment slot groups".into());
        }
    }
    match fields.get("effects") {
        Some(Value::Null) => {
            fields.remove("effects");
        }
        Some(Value::Object(_)) | None => {}
        _ => return Err("effects must be a component map".into()),
    }
    fields.retain(|name, _| {
        matches!(
            name.as_str(),
            "description"
                | "max_level"
                | "weight"
                | "anvil_cost"
                | "min_cost"
                | "max_cost"
                | "supported_items"
                | "primary_items"
                | "exclusive_set"
                | "slots"
                | "effects"
        )
    });
    Ok(value)
}

impl Enchantment {
    /// Whether `Enchantment.matchingSlot` admits this equipment slot.
    pub fn matches_slot(&self, slot: &str) -> bool {
        self.slots.iter().any(|group| match group.as_str() {
            "any" => true,
            "armor" => matches!(slot, "head" | "chest" | "legs" | "feet" | "body"),
            "hand" => matches!(slot, "mainhand" | "offhand"),
            group => group == slot,
        })
    }
}

/// One registry entry from its definition (`Enchantment.DIRECT_CODEC`).
fn parse_entry(
    id: &str,
    v: &Value,
    tags: &HashMap<String, Vec<Arc<str>>>,
    lang: &HashMap<String, String>,
) -> crate::Result<Enchantment> {
    if let Some(field) = ["supported_items", "primary_items", "exclusive_set"]
        .into_iter()
        .find(|field| v[*field].is_object())
    {
        return Err(format!(
            "enchantment {id} {field}: holder-set object has no owning registry binding"
        )
        .into());
    }
    let exclusive = match &v["exclusive_set"] {
        Value::String(s) if s.starts_with('#') => tags.get(&s[1..]).cloned().unwrap_or_default(),
        other => string_list(other)
            .into_iter()
            .map(|s| Arc::from(s.as_str()))
            .collect(),
    };
    let item_holders = |value| {
        string_list(value)
            .iter()
            .map(|holder| sunrise_world::enchantment_answers::canonical_item_holder(holder))
            .collect()
    };
    let key = v["description"]["translate"].as_str().unwrap_or("");
    Ok(Enchantment {
        id: Arc::from(id),
        description: lang.get(key).cloned().unwrap_or_else(|| {
            // A data pack's entry may name itself with a literal component
            // rather than a translation key.
            if key.is_empty() && !v["description"].is_null() {
                sunrise_game_data::scoreboard::comp_text(&v["description"]).plain()
            } else {
                id.trim_start_matches("minecraft:").replace('_', " ")
            }
        }),
        description_key: key.to_owned(),
        description_text: if v["description"].is_null() {
            sunrise_game_data::text::Text::literal(id)
        } else {
            sunrise_game_data::scoreboard::comp_text(&v["description"])
        },
        max_level: v["max_level"].as_u64().unwrap_or(1) as u32,
        weight: v["weight"].as_u64().unwrap_or(1) as u32,
        anvil_cost: v["anvil_cost"].as_u64().unwrap_or(0) as u32,
        min_cost: Cost::parse(&v["min_cost"]),
        max_cost: Cost::parse(&v["max_cost"]),
        supported: item_holders(&v["supported_items"]),
        primary: v.get("primary_items").map(item_holders),
        exclusive: exclusive.into_iter().collect(),
        slots: string_list(&v["slots"]),
        effects: v["effects"].clone(),
    })
}

impl Enchantments {
    /// Use the same frozen registry and resolved tags as player damage.
    pub fn bind_damage_catalog(&mut self, catalog: &sunrise_world::damage_source::Catalog) {
        self.damage_tags = catalog
            .tags()
            .map(|tag| {
                (
                    tag.to_owned(),
                    catalog.tag_members(tag).map(str::to_owned).collect(),
                )
            })
            .collect();
    }
    /// Load and bind the selected enchantment effects; decode errors name the definition.
    pub fn load(
        root: &Path,
        defaults: &sunrise_world::item_defaults::ItemDefaults,
    ) -> crate::Result<Self> {
        Self::load_with(root, &sunrise_world::loot::PackOverlay::NONE, defaults)
    }
    /// `RegistryDataLoader.load`: load from a resource root; `overlay` (world-pack content
    /// files) overrides definitions and joins enchantment tags in pack order, a
    /// `replace` file dropping the lower sources' (`TagLoader.load`).
    pub fn load_with(
        root: &Path,
        overlay: &sunrise_world::loot::PackOverlay,
        defaults: &sunrise_world::item_defaults::ItemDefaults,
    ) -> crate::Result<Self> {
        let mut enchantments = Self::load_definitions(root, overlay, defaults)?;
        use crate::items::ingredient_defaults::{LoadEntityTags, component_tables};
        let (_, tags) = component_tables(root, overlay, LoadEntityTags::Prepared)?;
        enchantments.entity_tags = tags.members().clone();
        Ok(enchantments)
    }

    pub(crate) fn load_definitions(
        root: &Path,
        overlay: &sunrise_world::loot::PackOverlay,
        defaults: &sunrise_world::item_defaults::ItemDefaults,
    ) -> crate::Result<Self> {
        // The registry lists entries sorted by id, like the data-pack loader.
        let mut files: BTreeMap<String, Value> =
            crate::items::data_files(root, "enchantment", overlay)
                .into_iter()
                .collect();
        if files.values().any(HolderSetBinding::needed) {
            let binding =
                HolderSetBinding::load(root, overlay, defaults, files.keys().cloned().collect())?;
            for (id, value) in &mut files {
                binding.bind(id, value)?;
            }
        }
        let mut enchantments = Self::load_selected(
            root,
            overlay,
            defaults,
            files.iter().map(|(id, value)| (id.as_str(), value)),
        )?;
        enchantments.install_providers(providers::load_from_resources(root, overlay)?);
        Ok(enchantments)
    }

    /// Assemble gameplay behavior from the selected, holder-bound definitions in id order.
    pub fn load_selected<'a>(
        root: &Path,
        overlay: &sunrise_world::loot::PackOverlay,
        defaults: &sunrise_world::item_defaults::ItemDefaults,
        files: impl IntoIterator<Item = (&'a str, &'a Value)>,
    ) -> crate::Result<Self> {
        let files: Vec<_> = files.into_iter().collect();
        let admitted: HashSet<_> = files.iter().map(|&(id, _)| id).collect();
        let tags = sunrise_world::tag_layers::load(root, "enchantment", overlay, |id| {
            admitted.contains(id)
        })?
        .into_iter()
        .map(|(name, ids)| (name, ids.into_iter().map(Arc::from).collect()))
        .collect();
        let damage_tags = match sunrise_world::damage_source::Catalog::load(root, overlay) {
            Ok(catalog) => catalog
                .tags()
                .map(|tag| {
                    (
                        tag.to_owned(),
                        catalog.tag_members(tag).map(str::to_owned).collect(),
                    )
                })
                .collect(),
            // Small tools/fixtures may not carry the prepared catalog.
            Err(_) => crate::items::load_tags(root, "damage_type", overlay)?,
        };
        let lang: HashMap<String, String> =
            sunrise_core::prepared_files::read(&root.join("assets/minecraft/lang/en_us.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<HashMap<String, String>>(&b).ok())
                .unwrap_or_default()
                .into_iter()
                .filter(|(k, _)| {
                    k.starts_with("enchantment.")
                        || k.starts_with("attribute.")
                        || k.starts_with("item.modifiers.")
                        || k.starts_with("effect.")
                        || k.starts_with("item.minecraft.potion")
                        || k.starts_with("item.minecraft.splash_potion")
                        || k.starts_with("item.minecraft.lingering_potion")
                        || k.starts_with("item.minecraft.tipped_arrow")
                        || k.starts_with("item.minecraft.firework_rocket.")
                        || k.starts_with("item.minecraft.firework_star.")
                        || k.starts_with("trim_")
                        || k.starts_with("potion.")
                        || k.starts_with("container.")
                        || k.starts_with("merchant.")
                        || k.starts_with("entity.minecraft.")
                        || k.starts_with("gui.recipebook.")
                        || k.starts_with("gui.togglable_slot")
                        || k.starts_with("block.minecraft.banner.")
                        || k.starts_with("recipe.toast.")
                })
                .collect();
        // Mods' English names for their enchantments (`assets/<ns>/lang/`),
        // under vanilla's, which win a clash.
        let mut lang = lang;
        for ns in sunrise_world::json_folder::namespaces(&root.join("data"))
            .into_iter()
            .filter(|ns| ns != "minecraft")
        {
            let path = root.join("assets").join(ns).join("lang/en_us.json");
            let Some(map) = sunrise_core::prepared_files::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<HashMap<String, String>>(&b).ok())
            else {
                continue;
            };
            for (k, v) in map {
                if k.starts_with("enchantment.") {
                    lang.entry(k).or_insert(v);
                }
            }
        }
        let mut bound_effects = HashMap::new();
        let empty = EnchantmentEffectRegistry::default();
        let types = EFFECT_TYPES.get().unwrap_or(&empty);
        let list = files
            .into_iter()
            .map(|(id, value)| {
                let entry = parse_entry(id, value, &tags, &lang)?;
                let effects = BoundEffects::decode(&entry.effects, types)
                    .map_err(|err| format!("enchantment {id}: {err}"))?;
                bound_effects.insert(entry.id.clone(), effects);
                Ok(entry)
            })
            .collect::<crate::Result<Vec<_>>>()?;
        let index = list
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id.clone(), i))
            .collect();
        let mut props = HashMap::new();
        for (id, c) in defaults.entries() {
            props.insert(
                Arc::from(id.as_str()),
                ItemProps {
                    enchantable: c["minecraft:enchantable"]["value"].as_u64().unwrap_or(0) as u32,
                    repairable: string_list(&c["minecraft:repairable"]["items"]),
                    glint: c["minecraft:enchantment_glint_override"].codec_bool(),
                    rarity: c["minecraft:rarity"]
                        .as_str()
                        .unwrap_or("common")
                        .to_owned(),
                    trim_material: c["minecraft:provides_trim_material"]
                        .as_str()
                        .map(str::to_owned),
                    brewing_fuel: c.get("minecraft:brewing_fuel").is_some(),
                },
            );
        }
        Ok(Self {
            list,
            index,
            bound_effects,
            tags,
            entity_tags: Default::default(),
            damage_tags,
            props,
            lang,
            primary_answers: Default::default(),
            support_answers: Default::default(),
            providers: BTreeMap::new(),
        })
    }
    /// Replace the `minecraft:enchantment_provider` table for this snapshot.
    pub fn install_providers(&mut self, providers: BTreeMap<String, EnchantmentProvider>) {
        self.providers = providers;
    }
    /// Look up one `minecraft:enchantment_provider` by canonical id.
    pub fn provider(&self, id: &str) -> Option<&EnchantmentProvider> {
        self.providers.get(id)
    }
    /// `EnchantmentHelper.enchantItemFromProvider`: look up `provider_id` in
    /// the `minecraft:enchantment_provider` registry and compute the
    /// `(enchantment, level)` upgrades for `stack`. Returns empty (drawing
    /// nothing from `random`) if the provider is not registered.
    pub fn enchant_from_provider<R: RandomSource>(
        &self,
        provider_id: &str,
        items: &Items,
        random: &mut R,
        stack: &ItemStack,
        special_multiplier: f32,
    ) -> Levels {
        let Some(provider) = self.providers.get(provider_id) else {
            return Vec::new();
        };
        provider.enchant(self, items, random, stack, special_multiplier)
    }
    /// The `isPrimaryItemFor` answers the server resolved for this snapshot when its parts
    /// installed; selection reads them.
    pub fn primary_answers(&self) -> &sunrise_world::enchantment_answers::AnswerTable {
        &self.primary_answers
    }
    /// The support answers resolved for this snapshot when its parts installed.
    pub fn support_answers(&self) -> &sunrise_world::enchantment_answers::AnswerTable {
        &self.support_answers
    }
    pub fn get(&self, id: &str) -> Option<&Enchantment> {
        self.index.get(id).map(|&i| &self.list[i])
    }
    /// The accepted registry IDs in lexical order, independent of gameplay's slot order.
    pub fn registry_ids(&self) -> impl Iterator<Item = &str> {
        self.index.keys().map(AsRef::as_ref)
    }
    /// The table a client plays a server with: this one and the server's
    /// entries it lacks (a data pack's), from the definitions the registry
    /// sync sent; `None` when there are none. The names then come from the
    /// entries' own `description`, as `Enchantment.getFullname` reads it.
    pub fn with_entries<'a>(
        &self,
        entries: impl IntoIterator<Item = (&'a str, &'a Value)>,
    ) -> crate::Result<Option<Self>> {
        let mut table = None::<Self>;
        for (id, v) in entries {
            if self.index.contains_key(id) {
                continue;
            }
            let t = table.get_or_insert_with(|| self.clone());
            // A remote definition supplies display data; only the authoritative
            // side binds and invokes its native handlers.
            let e = parse_entry(id, v, &t.tags, &t.lang)?;
            t.index.insert(e.id.clone(), t.list.len());
            t.list.push(e);
        }
        Ok(table)
    }
    /// The longest `minecraft:ignite` the attacker's `minecraft:post_attack`
    /// effects (Fire Aspect) set on the victim, in seconds: the effects whose
    /// requirements hold for `ctx` (0 when none).
    pub fn ignite_seconds(
        &self,
        enchants: &[(Arc<str>, u32)],
        ctx: &EffectContext<'_>,
        random: &mut LegacyRandom,
    ) -> f32 {
        let mut seconds = 0.0f32;
        for (id, level) in enchants {
            let Some(def) = self.get(id) else { continue };
            for effect in def.effects["minecraft:post_attack"]
                .as_array()
                .into_iter()
                .flatten()
            {
                let e = &effect["effect"];
                let is_ignite = matches!(e["type"].as_str(), Some("minecraft:ignite" | "ignite"));
                if !is_ignite
                    || effect["enchanted"] != "attacker"
                    || effect["affected"] != "victim"
                    || !effect
                        .get("requirements")
                        .is_none_or(|r| self.condition(r, *level, ctx, random))
                {
                    continue;
                }
                seconds = seconds.max(level_value(&e["duration"], *level));
            }
        }
        seconds
    }
    pub fn tag(&self, name: &str) -> &[Arc<str>] {
        self.tags
            .get(name.trim_start_matches('#'))
            .map_or(&[], Vec::as_slice)
    }
    pub fn in_tag(&self, name: &str, id: &str) -> bool {
        self.tag(name).iter().any(|e| &**e == id)
    }
    pub fn is_curse(&self, id: &str) -> bool {
        self.in_tag("minecraft:curse", id)
    }
    pub fn translate(&self, key: &str) -> Option<&str> {
        self.lang.get(key).map(String::as_str)
    }
    pub fn props(&self, item: &str) -> Option<&ItemProps> {
        self.props.get(item)
    }
    pub fn enchantable(&self, item: &str) -> u32 {
        self.props(item).map_or(0, |p| p.enchantable)
    }
    /// `ItemStack.get(DataComponents.ENCHANTABLE)`: the stack's component,
    /// else the item's default; a removed component leaves none.
    pub fn enchantable_of(&self, stack: &ItemStack) -> u32 {
        let data = stack.data();
        if data.is_some_and(|d| d.other("!minecraft:enchantable").is_some()) {
            return 0;
        }
        if let Some(tag) = data.and_then(|d| d.other("minecraft:enchantable")) {
            return match tag.get("value") {
                Some(Tag::Byte(v)) => u32::try_from(i32::from(*v)),
                Some(Tag::Short(v)) => u32::try_from(i32::from(*v)),
                Some(Tag::Int(v)) => u32::try_from(*v),
                _ => Ok(0),
            }
            .unwrap_or(0);
        }
        self.enchantable(&stack.item)
    }
    /// Whether `item` matches an id / `#tag` list.
    pub fn item_in(items: &Items, specs: &[String], item: &str) -> bool {
        specs.iter().any(|spec| {
            crate::properties::item_matches(spec, item, |tag| items.item_in_tag(tag, item))
        })
    }
    fn supports(&self, items: &Items, e: &Enchantment, item: &str) -> bool {
        self.support_answers
            .answer(item, &e.id)
            .unwrap_or_else(|| Self::item_in(items, &e.supported, item))
    }
    /// Support for `AnvilMenu.createResult`: only the anvil treats a left-hand
    /// enchanted book as supported without an explicit part answer.
    pub fn can_enchant_in_anvil(&self, items: &Items, e: &Enchantment, item: &str) -> bool {
        if item == "minecraft:enchanted_book" {
            self.support_answers.answer(item, &e.id).unwrap_or(true)
        } else {
            self.supports(items, e, item)
        }
    }
    /// Explicit install-time support, or vanilla `Enchantment.isSupportedItem`.
    pub fn can_enchant(&self, items: &Items, id: &str, item: &str) -> bool {
        self.get(id).is_some_and(|e| self.supports(items, e, item))
    }
    /// The `isPrimaryItemFor` default: a plain book, or support before the primary tag.
    pub fn is_primary(&self, items: &Items, e: &Enchantment, item: &str) -> bool {
        item == "minecraft:book"
            || (self.supports(items, e, item)
                && e.primary
                    .as_ref()
                    .is_none_or(|p| Self::item_in(items, p, item)))
    }
    /// `Enchantment.areCompatible`.
    pub fn compatible(&self, a: &str, b: &str) -> bool {
        if a == b {
            return false;
        }
        let excl = |x: &str, y: &str| self.get(x).is_some_and(|e| e.exclusive.contains(y));
        !excl(a, b) && !excl(b, a)
    }
    /// `Enchantment.getFullname` as plain text and whether it is a curse (red).
    pub fn full_name(&self, id: &str, level: u32) -> (String, bool) {
        let language = sunrise_game_data::lang::current();
        self.full_name_in(language.as_deref(), id, level)
    }
    /// `Enchantment.getFullname` against `language`; `None` (no language loaded: a
    /// dedicated server, the probes) keeps the English names loaded above.
    pub fn full_name_in(
        &self,
        language: Option<&sunrise_game_data::lang::Language>,
        id: &str,
        level: u32,
    ) -> (String, bool) {
        let numeric = NUMERIC_LEVELS.load(Ordering::Relaxed);
        self.full_name_with(language, id, level, numeric)
    }
    /// [`Enchantments::full_name_in`] with the level as a number when `numeric`.
    fn full_name_with(
        &self,
        language: Option<&sunrise_game_data::lang::Language>,
        id: &str,
        level: u32,
        numeric: bool,
    ) -> (String, bool) {
        let tr = |key: &str| language.and_then(|l| l.get(key).map(str::to_owned));
        let Some(e) = self.get(id) else {
            return (format!("{id} {}", level_name(level)), false);
        };
        let description = tr(&e.description_key).unwrap_or_else(|| e.description.clone());
        let name = format_full_name(description, level, e.max_level, |level| {
            if numeric {
                return level.to_string();
            }
            let key = format!("enchantment.level.{level}");
            tr(&key)
                .or_else(|| self.lang.get(&key).cloned())
                .unwrap_or_else(|| {
                    if (1..=10).contains(&level) {
                        level_name(level)
                    } else {
                        key
                    }
                })
        });
        (name, self.is_curse(id))
    }
    /// Tooltip lines for a level list, in `#tooltip_order` then remaining order.
    pub fn tooltip(&self, levels: &[(Arc<str>, u32)]) -> Vec<(String, bool)> {
        let order = self.tag("minecraft:tooltip_order");
        let mut out = Vec::new();
        for id in order {
            if let Some((_, l)) = levels.iter().find(|(e, _)| e == id) {
                out.push(self.full_name(id, *l));
            }
        }
        for (id, l) in levels {
            if !order.contains(id) {
                out.push(self.full_name(id, *l));
            }
        }
        out
    }

    // ---- Enchanting table (EnchantmentHelper) ----

    /// `EnchantmentHelper.getEnchantmentCost`.
    pub fn enchantment_cost<R: RandomSource>(
        &self,
        random: &mut R,
        slot: usize,
        bookcases: i32,
        item: &str,
    ) -> i32 {
        self.enchantment_cost_with(random, slot, bookcases, self.enchantable(item))
    }
    /// `EnchantmentHelper.getEnchantmentCost` for a stack's own `ENCHANTABLE`.
    pub fn enchantment_cost_of<R: RandomSource>(
        &self,
        random: &mut R,
        slot: usize,
        bookcases: i32,
        stack: &ItemStack,
    ) -> i32 {
        self.enchantment_cost_with(random, slot, bookcases, self.enchantable_of(stack))
    }
    fn enchantment_cost_with<R: RandomSource>(
        &self,
        random: &mut R,
        slot: usize,
        bookcases: i32,
        enchantable: u32,
    ) -> i32 {
        if enchantable == 0 {
            return 0;
        }
        let bookcases = bookcases.min(15);
        let selected = next_int(random, 8) + 1 + (bookcases >> 1) + next_int(random, bookcases + 1);
        match slot {
            0 => (selected / 3).max(1),
            1 => selected * 2 / 3 + 1,
            _ => selected.max(bookcases * 2),
        }
    }
    /// `EnchantmentHelper.getAvailableEnchantmentResults` offers only enchantments for which
    /// the item is primary. A plain book qualifies by default; other items need support and
    /// membership in `primary_items` if present. This snapshot's `ItemPart::is_primary_item_for`
    /// answers can override that default for any item, including books ([`Self::primary_answers`]).
    pub fn available_results(
        &self,
        items: &Items,
        value: i32,
        item: &str,
        pool: &[Arc<str>],
    ) -> Vec<(Arc<str>, u32)> {
        let mut out = Vec::new();
        for id in pool {
            let Some(e) = self.get(id) else { continue };
            if !self
                .primary_answers
                .answer(item, &e.id)
                .unwrap_or_else(|| self.is_primary(items, e, item))
            {
                continue;
            }
            for level in (1..=e.max_level).rev() {
                if value >= e.min_cost.calculate(level) && value <= e.max_cost.calculate(level) {
                    out.push((e.id.clone(), level));
                    break;
                }
            }
        }
        out
    }
    fn weighted<R: RandomSource>(
        &self,
        random: &mut R,
        list: &[(Arc<str>, u32)],
    ) -> Option<(Arc<str>, u32)> {
        let total: i64 = list
            .iter()
            .map(|(id, _)| self.get(id).map_or(0, |e| i64::from(e.weight)))
            .sum();
        if total <= 0 {
            return None;
        }
        // Preserve vanilla's int draw. Oversized pack totals use one unsigned
        // long modulo the total, a bounded, slightly biased fallback.
        let mut index = if total <= i64::from(i32::MAX) {
            i64::from(next_int(random, total as i32))
        } else {
            ((random.next_long() as u64) % total as u64) as i64
        };
        for (id, l) in list {
            index -= self.get(id).map_or(0, |e| i64::from(e.weight));
            if index < 0 {
                return Some((id.clone(), *l));
            }
        }
        None
    }
    /// Select enchantments using the stack's enchantable component.
    pub fn select<R: RandomSource>(
        &self,
        items: &Items,
        random: &mut R,
        stack: &ItemStack,
        cost: i32,
        pool: &[Arc<str>],
    ) -> Levels {
        self.select_with_value(
            items,
            random,
            &stack.item,
            cost,
            pool,
            self.enchantable_of(stack),
        )
    }
    /// Select with the enchantability of the stack after its component patch.
    pub fn select_with_value<R: RandomSource>(
        &self,
        items: &Items,
        random: &mut R,
        item: &str,
        cost: i32,
        pool: &[Arc<str>],
        value: u32,
    ) -> Levels {
        let mut results = Vec::new();
        let Ok(value) = i32::try_from(value) else {
            return results;
        };
        if value == 0 {
            return results;
        }
        let bound = value / 4 + 1;
        let mut cost = cost
            .wrapping_add(1)
            .wrapping_add(next_int(random, bound))
            .wrapping_add(next_int(random, bound));
        let span = (random.next_float() + random.next_float() - 1.0) * 0.15;
        cost = ((cost as f32 + cost as f32 * span).round() as i32).max(1);
        let mut available = self.available_results(items, cost, item, pool);
        if !available.is_empty() {
            if let Some(first) = self.weighted(random, &available) {
                results.push(first);
            }
            while next_int(random, 50) <= cost {
                if let Some((last, _)) = results.last() {
                    let last = last.clone();
                    available.retain(|(e, _)| self.compatible(&last, e));
                }
                if available.is_empty() {
                    break;
                }
                if let Some(next) = self.weighted(random, &available) {
                    results.push(next);
                }
                cost /= 2;
            }
        }
        results
    }

    // ---- Effect components ----

    /// Run `component` value effects of every enchantment on `stack` over `value`.
    pub fn modify_value(
        &self,
        stack: Option<&ItemStack>,
        component: &str,
        ctx: &EffectContext<'_>,
        random: &mut LegacyRandom,
        value: f32,
    ) -> f32 {
        let Some(stack) = stack else { return value };
        let mut value = value;
        for (id, level) in stack.all_enchantments().iter() {
            value = self.modify_value_with(id, *level, component, ctx, random, value);
        }
        value
    }
    pub fn modify_value_with(
        &self,
        id: &str,
        level: u32,
        component: &str,
        ctx: &EffectContext<'_>,
        random: &mut LegacyRandom,
        mut value: f32,
    ) -> f32 {
        for (index, entry) in self.conditional_value_entries(id, component).enumerate() {
            if entry
                .get("requirements")
                .is_none_or(|req| self.condition(req, level, ctx, random))
            {
                value = self.process_value(
                    id,
                    component,
                    index,
                    &entry["effect"],
                    level,
                    random,
                    value,
                );
            }
        }
        value
    }

    fn conditional_value_entries<'a>(
        &'a self,
        id: &str,
        component: &str,
    ) -> impl Iterator<Item = &'a Value> {
        self.get(id)
            .and_then(|enchantment| enchantment.effects[component].as_array())
            .into_iter()
            .flatten()
    }

    /// Process a value node selected by its component's vanilla iteration.
    #[allow(clippy::too_many_arguments)]
    pub fn process_value(
        &self,
        id: &str,
        component: &str,
        index: usize,
        effect: &Value,
        level: u32,
        random: &mut LegacyRandom,
        value: f32,
    ) -> f32 {
        match self
            .bound_effects
            .get(id)
            .and_then(|items| items.values.get(component))
            .and_then(|nodes| nodes.get(index))
        {
            Some(node) => node.process(level, random, value),
            None => apply_value_effect(effect, level, random, value),
        }
    }

    /// Find the effect bound for this selected definition, component and entry.
    pub fn entity_effect(&self, id: &str, component: &str, index: usize) -> Option<&EntityEffect> {
        self.bound_effects
            .get(id)?
            .entities
            .get(component)?
            .get(index)
    }

    /// Read the vanilla portions of a value component without invoking a native callback.
    /// The player's equipment display cache carries no live damage source or entity random.
    pub fn preview_value(
        &self,
        stack: &ItemStack,
        component: &str,
        ctx: &EffectContext<'_>,
        random: &mut LegacyRandom,
    ) -> f32 {
        let mut value = 0.0;
        for (id, level) in stack.enchantments() {
            for entry in self.conditional_value_entries(id, component) {
                if entry
                    .get("requirements")
                    .is_none_or(|req| self.condition(req, *level, ctx, random))
                {
                    value = apply_value_effect(&entry["effect"], *level, random, value);
                }
            }
        }
        value
    }

    /// Check requirements on the level stream and apply on the subject's own stream.
    pub fn modify_entity_value(
        &self,
        levels: &[(Arc<str>, u32)],
        component: &str,
        ctx: &EffectContext<'_>,
        level_random: &mut LegacyRandom,
        entity_random: &mut LegacyRandom,
        mut value: f32,
    ) -> f32 {
        for (id, level) in levels {
            for (index, entry) in self.conditional_value_entries(id, component).enumerate() {
                if entry
                    .get("requirements")
                    .is_none_or(|req| self.condition(req, *level, ctx, level_random))
                {
                    value = self.process_value(
                        id,
                        component,
                        index,
                        &entry["effect"],
                        *level,
                        entity_random,
                        value,
                    );
                }
            }
        }
        value
    }
    /// Chain `damage_protection` from one equipped stack in vanilla slot order.
    pub fn equipment_protection(
        &self,
        stack: &ItemStack,
        slot: &str,
        ctx: &EffectContext<'_>,
        level_random: &mut LegacyRandom,
        entity_random: &mut LegacyRandom,
        mut value: f32,
    ) -> f32 {
        for (id, level) in stack.all_enchantments().iter() {
            if self.get(id).is_some_and(|def| def.matches_slot(slot)) {
                value = self.modify_entity_value(
                    std::slice::from_ref(&(id.clone(), *level)),
                    "minecraft:damage_protection",
                    ctx,
                    level_random,
                    entity_random,
                    value,
                );
            }
        }
        value
    }

    /// `EnchantmentHelper.has`: whether `stack` has any enchantment providing
    /// `component`. It reads the stack's own component.
    pub fn has_component(&self, stack: &ItemStack, component: &str) -> bool {
        stack.enchantments().iter().any(|(id, _)| {
            self.get(id)
                .is_some_and(|e| e.effects.get(component).is_some())
        })
    }
    /// `EnchantmentHelper.has(stack, PREVENT_ARMOR_CHANGE)`: whether an enchantment on `stack`
    /// keeps it on as worn armour (Curse of Binding, or a data pack's own).
    pub fn prevents_armor_change(&self, stack: &ItemStack) -> bool {
        self.has_component(stack, "minecraft:prevent_armor_change")
    }
    /// `minecraft:attributes` contributions of `stack`: (attribute, id, amount, operation).
    pub fn attributes(&self, stack: &ItemStack) -> Vec<(String, String, f64, String)> {
        let mut out = Vec::new();
        for (id, level) in stack.all_enchantments().iter() {
            let Some(e) = self.get(id) else { continue };
            for a in e.effects["minecraft:attributes"]
                .as_array()
                .into_iter()
                .flatten()
            {
                out.push((
                    a["attribute"].as_str().unwrap_or("").to_owned(),
                    a["id"].as_str().unwrap_or("").to_owned(),
                    f64::from(level_value(&a["amount"], *level)),
                    a["operation"].as_str().unwrap_or("add_value").to_owned(),
                ));
            }
        }
        out
    }
    /// `EnchantmentHelper.forEachModifier` as an item's tooltip lists them: the
    /// `minecraft:attributes` effects of `stack`'s enchantments, one row per
    /// slot group the enchantment lists.
    pub fn modifier_rows(&self, stack: &ItemStack) -> Vec<crate::item_data::ModifierRow> {
        let mut rows = Vec::new();
        for (id, level) in stack.all_enchantments().iter() {
            let Some(e) = self.get(id) else { continue };
            for a in e.effects["minecraft:attributes"]
                .as_array()
                .into_iter()
                .flatten()
            {
                for slot in &e.slots {
                    rows.push(crate::item_data::ModifierRow::new(
                        a["attribute"].as_str().unwrap_or(""),
                        a["id"].as_str().unwrap_or(""),
                        f64::from(level_value(&a["amount"], *level)),
                        a["operation"].as_str().unwrap_or("add_value"),
                        slot,
                    ));
                }
            }
        }
        rows
    }
    /// Sum of `add_value` amounts of one attribute from enchantments on `stack`.
    pub fn attribute_add(&self, stack: Option<&ItemStack>, attribute: &str) -> f64 {
        stack.map_or(0.0, |s| {
            self.attributes(s)
                .iter()
                .filter(|a| a.0 == attribute && a.3 == "add_value")
                .map(|a| a.2)
                .sum()
        })
    }
    /// A resolved entity-type tag from the same pack stack as these enchantments.
    pub fn entity_in_tag(&self, entity: &str, tag: &str) -> bool {
        self.entity_tags
            .get(tag)
            .is_some_and(|set| set.contains(entity))
    }

    /// `EnchantmentHelper.runIterationOnEquipment`, restricted to enchantments
    /// that can answer `isImmuneToDamage`.
    pub fn damage_immunities<'a>(
        &'a self,
        equipment: impl IntoIterator<Item = (&'a [(Arc<str>, u32)], &'a str)> + 'a,
    ) -> impl Iterator<Item = (&'a Arc<str>, u32)> + 'a {
        equipment.into_iter().flat_map(move |(levels, slot)| {
            levels.iter().filter_map(move |(id, level)| {
                let enchantment = self.get(id)?;
                (enchantment.matches_slot(slot)
                    && enchantment
                        .effects
                        .get("minecraft:damage_immunity")
                        .is_some())
                .then_some((id, *level))
            })
        })
    }

    /// `Enchantment.isImmuneToDamage`: a registered, active enchantment must
    /// match the current source, not just declare a damage immunity effect.
    pub fn is_damage_immune<'a>(
        &self,
        active: impl IntoIterator<Item = (&'a Arc<str>, u32)>,
        context: &EffectContext<'_>,
        random: &mut LegacyRandom,
    ) -> bool {
        active.into_iter().any(|(id, level)| {
            self.get(id).is_some_and(|enchantment| {
                enchantment.effects["minecraft:damage_immunity"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|entry| {
                        entry.get("requirements").is_none_or(|requirement| {
                            self.condition(requirement, level, context, random)
                        })
                    })
            })
        })
    }

    fn entity_matches(&self, spec: &Value, entity: Option<&str>) -> bool {
        let Some(entity) = entity else { return false };
        match spec.as_str() {
            Some(s) => match s.strip_prefix('#') {
                Some(tag) => self.entity_in_tag(entity, tag),
                None => s == entity,
            },
            None => string_list(spec).iter().any(|s| s == entity),
        }
    }
    fn entity_predicate_matches(
        &self,
        predicate: &Value,
        entity: Option<&str>,
        motion: Option<(f64, bool)>,
    ) -> bool {
        let spec = predicate
            .get("minecraft:entity_type")
            .unwrap_or(&predicate["type"]);
        entity.is_some()
            && (spec.is_null() || self.entity_matches(spec, entity))
            && movement_matches(predicate, motion)
    }

    /// The loot-condition subset enchantment `requirements` use.
    pub fn condition(
        &self,
        c: &Value,
        level: u32,
        ctx: &EffectContext<'_>,
        random: &mut LegacyRandom,
    ) -> bool {
        match c["type"].as_str().or(c["condition"].as_str()).unwrap_or("") {
            "minecraft:inverted" => !self.condition(&c["term"], level, ctx, random),
            "minecraft:all_of" => c["terms"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|t| self.condition(t, level, ctx, random)),
            "minecraft:any_of" => c["terms"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|t| self.condition(t, level, ctx, random)),
            "minecraft:match_tool" => {
                let Some(tool) = ctx.tool else { return false };
                let items = &c["predicate"]["items"];
                if items.is_null() {
                    return true;
                }
                string_list(items)
                    .iter()
                    .any(|s| match s.strip_prefix('#') {
                        Some(tag) => ctx.items.is_some_and(|i| {
                            i.item_tags.get(tag).is_some_and(|t| t.contains(tool))
                        }),
                        None => s == tool,
                    })
            }
            "minecraft:damage_source_properties" => {
                let Some(damage_type) = ctx.damage_type else {
                    return false;
                };
                let p = &c["predicate"];
                p["tags"].as_array().into_iter().flatten().all(|t| {
                    let tag = t["id"].as_str().unwrap_or("").trim_start_matches('#');
                    let has = self
                        .damage_tags
                        .get(tag)
                        .is_some_and(|s| s.contains(damage_type));
                    has == t["expected"].as_bool().unwrap_or(true)
                }) && p.get("direct_entity").is_none_or(|predicate| {
                    self.entity_predicate_matches(predicate, ctx.direct_attacker, ctx.direct_motion)
                }) && p.get("source_entity").is_none_or(|predicate| {
                    self.entity_predicate_matches(predicate, ctx.attacker, ctx.attacker_motion)
                }) && p["is_direct"]
                    .as_bool()
                    .is_none_or(|direct| direct == ctx.is_direct)
            }
            "minecraft:entity_properties" => {
                let (entity, motion) = match c["entity"].as_str().unwrap_or("this") {
                    "this" => (ctx.this, ctx.this_motion),
                    "attacker" => (ctx.attacker, ctx.attacker_motion),
                    "direct_attacker" => (ctx.direct_attacker, ctx.direct_motion),
                    _ => (None, None),
                };
                let predicate = &c["predicate"];
                if !self.entity_predicate_matches(predicate, entity, motion) {
                    return false;
                }
                if c["entity"].as_str().unwrap_or("this") != "this" {
                    return true;
                }
                let location = &predicate["minecraft:location"];
                let grounded = predicate["minecraft:flags"]["is_on_ground"].as_bool();
                let speed = &predicate["minecraft:movement"]["horizontal_speed"];
                let affected_by = &predicate["minecraft:movement_affected_by"]["block"];
                let period = predicate["minecraft:periodic_tick"].as_u64();
                location["can_see_sky"]
                    .as_bool()
                    .is_none_or(|sky| ctx.sees_sky == Some(sky))
                    && grounded.is_none_or(|grounded| ctx.grounded == Some(grounded))
                    && (speed.is_null()
                        || ctx
                            .horizontal_speed
                            .is_some_and(|v| matches_double(speed, v)))
                    && (affected_by.is_null()
                        || effect_block_matches(affected_by, ctx.movement_block, ctx.items))
                    && period.is_none_or(|period| {
                        period != 0 && ctx.tick.is_some_and(|tick| u64::from(tick) % period == 0)
                    })
            }
            "minecraft:weather_check" => ctx.weather.is_some_and(|(raining, thundering)| {
                c["raining"].as_bool().is_none_or(|v| v == raining)
                    && c["thundering"].as_bool().is_none_or(|v| v == thundering)
            }),
            "minecraft:location_check" => {
                let predicate = &c["predicate"];
                predicate["can_see_sky"]
                    .as_bool()
                    .is_none_or(|sky| ctx.sees_sky == Some(sky))
                    && (predicate["block"].is_null()
                        || effect_block_matches(&predicate["block"], ctx.hit_block, ctx.items))
            }
            "minecraft:block_state_property" => ctx.hit_state.is_some_and(|state| {
                c["block"]
                    .as_str()
                    .is_some_and(|name| state.name.as_str() == name)
                    && c["properties"].as_object().is_none_or(|properties| {
                        properties.iter().all(|(key, value)| {
                            state
                                .properties
                                .get(key)
                                .is_some_and(|actual| value.as_str() == Some(actual.as_str()))
                        })
                    })
            }),
            "minecraft:random_chance" => {
                let chance = match &c["chance"] {
                    Value::Object(o)
                        if o.get("type").and_then(Value::as_str)
                            == Some("minecraft:enchantment_level") =>
                    {
                        level_value(&c["chance"]["amount"], level)
                    }
                    v => v
                        .as_f64()
                        .map_or_else(|| level_value(v, level), |f| f as f32),
                };
                random.next_float() < chance
            }
            _ => false,
        }
    }
}
/// What enchantment requirement predicates can see.
#[derive(Clone, Copy, Default)]
pub struct EffectContext<'a> {
    pub items: Option<&'a Items>,
    /// The tool for `match_tool` (the enchanted item, or the ammo for ammo_use).
    pub tool: Option<&'a str>,
    /// Damage type id (e.g. `minecraft:fall`).
    pub damage_type: Option<&'a str>,
    pub is_direct: bool,
    /// Entity type ids: the victim/user ("this"), attacker and direct attacker.
    pub this: Option<&'a str>,
    pub attacker: Option<&'a str>,
    pub direct_attacker: Option<&'a str>,
    /// The roles' fall distance and flying state, where the game knows them.
    pub this_motion: Option<(f64, bool)>,
    pub attacker_motion: Option<(f64, bool)>,
    pub direct_motion: Option<(f64, bool)>,
    /// Weather and sky at the context's origin, where available.
    pub weather: Option<(bool, bool)>,
    pub sees_sky: Option<bool>,
    pub hit_block: Option<&'a str>,
    pub hit_state: Option<&'a sunrise_world::chunk::BlockState>,
    pub movement_block: Option<&'a str>,
    pub grounded: Option<bool>,
    pub horizontal_speed: Option<f64>,
    pub tick: Option<u32>,
}

fn effect_block_matches(predicate: &Value, block: Option<&str>, items: Option<&Items>) -> bool {
    block.is_some_and(|block| {
        let blocks = &predicate["blocks"];
        blocks.is_null()
            || string_list(blocks).iter().any(|name| {
                name.strip_prefix('#').map_or_else(
                    || name == block,
                    |tag| items.is_some_and(|items| items.block_in_tag(tag, block)),
                )
            })
    })
}

fn matches_double(bounds: &Value, value: f64) -> bool {
    if let Some(exact) = bounds.as_f64() {
        return exact == value;
    }
    bounds["min"].as_f64().is_none_or(|min| min <= value)
        && bounds["max"].as_f64().is_none_or(|max| value <= max)
}
/// `EntityPredicate`'s `minecraft:movement` fall distance and
/// `minecraft:flags` is_flying (the parts enchantment requirements use); an
/// entity whose state is unknown matches neither.
fn movement_matches(predicate: &Value, state: Option<(f64, bool)>) -> bool {
    let fall = &predicate["minecraft:movement"]["fall_distance"];
    let flying = predicate["minecraft:flags"]["is_flying"].as_bool();
    if fall.is_null() && flying.is_none() {
        return true;
    }
    let Some((distance, is_flying)) = state else {
        return false;
    };
    // MinMaxBounds.Doubles: a number is an exact bound.
    let bound = |key: &str| {
        fall.as_f64()
            .or_else(|| fall.get(key).and_then(Value::as_f64))
    };
    // MinMaxBounds.Doubles.matches
    bound("min").is_none_or(|min| min <= distance)
        && bound("max").is_none_or(|max| distance <= max)
        && flying.is_none_or(|f| f == is_flying)
}
fn next_int<R: RandomSource>(random: &mut R, bound: i32) -> i32 {
    random.next_int_bound(bound.max(1)).unwrap_or(0)
}
/// `LevelBasedValue.calculate`.
pub fn level_value(v: &Value, level: u32) -> f32 {
    let l = level as f32;
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(0.0) as f32,
        Value::Object(o) => {
            let f = |k: &str| o.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32;
            match o.get("type").and_then(Value::as_str).unwrap_or("") {
                "minecraft:linear" => f("base") + f("per_level_above_first") * (l - 1.0),
                "minecraft:levels_squared" => l * l + f("added"),
                "minecraft:clamped" => level_value(&o["value"], level).clamp(f("min"), f("max")),
                "minecraft:fraction" => {
                    let d = level_value(&o["denominator"], level);
                    if d == 0.0 {
                        0.0
                    } else {
                        level_value(&o["numerator"], level) / d
                    }
                }
                "minecraft:exponent" => {
                    level_value(&o["base"], level).powf(level_value(&o["power"], level))
                }
                "minecraft:lookup" => {
                    let values = o.get("values").and_then(Value::as_array);
                    match values.and_then(|a| a.get(level as usize - 1)) {
                        Some(v) => v.as_f64().unwrap_or(0.0) as f32,
                        None => level_value(&o["fallback"], level),
                    }
                }
                _ => f("value"),
            }
        }
        _ => 0.0,
    }
}
/// `EnchantmentValueEffect.process`.
pub fn apply_value_effect(
    effect: &Value,
    level: u32,
    random: &mut LegacyRandom,
    value: f32,
) -> f32 {
    let kind = effect["type"].as_str().unwrap_or("");
    match kind.strip_prefix("minecraft:").unwrap_or(kind) {
        "add" => value + level_value(&effect["value"], level),
        "multiply" => value * level_value(&effect["factor"], level),
        "set" => level_value(&effect["value"], level),
        "exponential" => {
            (f64::from(value)
                * sunrise_core::jmath::pow(
                    f64::from(level_value(&effect["base"], level)),
                    f64::from(level_value(&effect["exponent"], level)),
                )) as f32
        }
        "remove_binomial" => {
            let p = level_value(&effect["chance"], level);
            let n = value;
            let mean = n * p;
            let other = 1.0 - p;
            use std::cmp::Ordering::{Equal, Less};
            let removed = if !matches!(n.partial_cmp(&128.0), Some(Less | Equal))
                && mean.partial_cmp(&20.0) != Some(Less)
                && (n * other).partial_cmp(&20.0) != Some(Less)
            {
                let mu = f64::from(mean).floor();
                let sigma = f64::from(mean * other).sqrt();
                (sunrise_core::jmath::round(mu + random.next_gaussian() * sigma) as i32)
                    .clamp(0, n as i32)
            } else {
                let mut removed = 0_i32;
                let mut draw = 0_i32;
                while (draw as f32) < n {
                    if random.next_float() < p {
                        removed = removed.wrapping_add(1);
                    }
                    draw = draw.wrapping_add(1);
                }
                removed
            };
            n - removed as f32
        }
        "all_of" => effect["effects"]
            .as_array()
            .into_iter()
            .flatten()
            .fold(value, |v, e| apply_value_effect(e, level, random, v)),
        _ => value,
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn synced_unknown_damage_retains_enchantment_definition() {
        let definition = json!({"description":"Remote", "effects":{"minecraft:damage":[
            {"effect":{"type":"remote:damage"}}
        ]}});
        let table = sample()
            .with_entries([("remote:enchantment", &definition)])
            .unwrap()
            .unwrap();
        assert!(table.get("remote:enchantment").is_some());
    }

    #[test]
    fn damage_requirements_and_value_effects_advance_different_owners() {
        let mut enchantments = sample();
        let index = enchantments.index["minecraft:sharpness"];
        enchantments.list[index].effects = json!({"minecraft:damage":[{
            "effect":{"type":"minecraft:remove_binomial", "chance":0.5},
            "requirements":{"condition":"minecraft:random_chance", "chance":1.0}
        }]});
        let mut level = LegacyRandom::new(42);
        let mut victim = LegacyRandom::new(0);
        let value = enchantments.modify_entity_value(
            &[("minecraft:sharpness".into(), 1)],
            "minecraft:damage",
            &EffectContext::default(),
            &mut level,
            &mut victim,
            2.0,
        );
        // Java Random(0) first two floats are greater than 0.5.
        assert_eq!(value, 2.0);
        assert_eq!(level.next_int(), 234_785_527);
        assert_eq!(victim.next_int(), 1_033_096_058);
    }

    #[test]
    fn unknown_nested_effect_identifies_its_component_and_kind() {
        for component in ["minecraft:damage", "minecraft:post_attack"] {
            let effects = json!({(component): [{"effect":{"type":"all_of", "effects":[{"type":"missing:effect"}]}}]});
            let error = BoundEffects::decode(&effects, &EnchantmentEffectRegistry::default())
                .err()
                .unwrap();
            assert!(error.starts_with(component));
            assert!(error.ends_with("missing:effect"));
        }
    }

    #[test]
    fn binomial_draws_for_fractional_and_unordered_inputs() {
        let effect = json!({"type":"remove_binomial", "chance":0.0});
        let mut random = LegacyRandom::new(42);
        assert_eq!(apply_value_effect(&effect, 1, &mut random, 0.5), 0.5);
        assert_eq!(random.next_int(), 234_785_527);
        let mut random = LegacyRandom::new(42);
        let mut gaussian = random.clone();
        gaussian.next_gaussian();
        assert!(apply_value_effect(&effect, 1, &mut random, f32::NAN).is_nan());
        assert_eq!(random.next_int(), gaussian.next_int());
        let effect = json!({"type":"exponential", "base":4, "exponent":0.5});
        assert_eq!(
            apply_value_effect(&effect, 1, &mut random, 2_000.0),
            4_000.0
        );
    }

    #[test]
    fn damage_without_custom_nodes_preserves_vanilla_values_and_random() {
        let enchantments = sample();
        let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
        for (levels, expected) in [
            (vec![], 7.0),
            (vec![("minecraft:sharpness".into(), 3)], 9.0),
            (vec![("minecraft:smite".into(), 2)], 12.0),
        ] {
            stack.data_mut().enchantments = levels;
            let context = EffectContext {
                this: Some("minecraft:zombie"),
                ..Default::default()
            };
            let mut random = LegacyRandom::new(42);
            assert_eq!(
                enchantments.modify_entity_value(
                    &stack.all_enchantments(),
                    "minecraft:damage",
                    &context,
                    &mut random,
                    &mut LegacyRandom::new(0),
                    7.0,
                ),
                expected
            );
            assert_eq!(random.next_int(), -1_170_105_035);
        }
    }

    #[test]
    fn world_pack_definitions_join_catalog_in_overlay_priority() {
        let root = std::env::temp_dir().join(format!(
            "sunrise-enchantment-overlay-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let write = |path: &str, value: &Value| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
            std::fs::read(path).unwrap()
        };
        let definition = |description: &str, max_level| {
            json!({
                "description": description, "supported_items": "minecraft:diamond_sword",
                "weight": 1, "max_level": max_level,
                "min_cost": {"base": 1, "per_level_above_first": 0},
                "max_cost": {"base": 1, "per_level_above_first": 0},
                "anvil_cost": 1, "slots": ["mainhand"],
                "effects": {"minecraft:damage": [{"effect": {"type": "minecraft:add", "value": 2.0}}]}
            })
        };
        write(
            "data/minecraft/enchantment/mending.json",
            &definition("Vanilla retained", 1),
        );
        write(
            "data/minecraft/enchantment/sharpness.json",
            &definition("Vanilla base", 5),
        );
        write(
            "data/minecraft/tags/enchantment/fixture.json",
            &json!({"values": ["minecraft:mending"]}),
        );
        write(
            "data/minecraft/tags/damage_type/fixture.json",
            &json!({"values": ["minecraft:generic"]}),
        );
        write(
            "data/minecraft/tags/entity_type/fixture.json",
            &json!({"values": ["minecraft:pig"]}),
        );
        let custom = write(
            "packs/custom/data/wpenchant/enchantment/loaded.json",
            &definition("Custom loaded", 1),
        );
        let low = write(
            "packs/low/data/minecraft/enchantment/sharpness.json",
            &definition("Lower override", 2),
        );
        let high = write(
            "packs/high/data/minecraft/enchantment/sharpness.json",
            &definition("Higher override", 3),
        );
        let tag = write(
            "packs/custom/data/wpenchant/tags/enchantment/fixture.json",
            &json!({"values": ["wpenchant:loaded"]}),
        );
        let files = vec![
            ("enchantment/wpenchant:loaded".to_owned(), vec![custom]),
            (
                "enchantment/minecraft:sharpness".to_owned(),
                vec![low, high],
            ),
            ("tags/wpenchant:enchantment/fixture".to_owned(), vec![tag]),
        ];
        write("items.json", &json!({}));
        write("entity-types.json", &json!({"minecraft:pig": {}}));
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let base = Enchantments::load(&root, &defaults).unwrap();
        let loaded = Enchantments::load_with(
            &root,
            &sunrise_world::loot::PackOverlay {
                files: &files,
                ..sunrise_world::loot::PackOverlay::default()
            },
            &defaults,
        )
        .unwrap();
        assert_eq!(
            loaded.get("minecraft:mending").unwrap().description,
            "Vanilla retained"
        );
        assert_eq!(loaded.damage_tags, base.damage_tags);
        assert_eq!(loaded.entity_tags, base.entity_tags);
        assert_eq!(
            loaded.tag("minecraft:fixture"),
            base.tag("minecraft:fixture")
        );
        assert!(loaded.in_tag("wpenchant:fixture", "wpenchant:loaded"));
        assert!(loaded.get("wpenchant:missing").is_none());
        let custom = loaded
            .get("wpenchant:loaded")
            .expect("actual overlay definition must enter the catalog");
        assert_eq!(custom.description, "Custom loaded");
        assert_eq!(custom.max_level, 1);
        assert_eq!(
            loaded.get("minecraft:sharpness").unwrap().description,
            "Higher override"
        );
        assert_eq!(loaded.get("minecraft:sharpness").unwrap().max_level, 3);
        assert_eq!(
            loaded.modify_value_with(
                "wpenchant:loaded",
                1,
                "minecraft:damage",
                &EffectContext::default(),
                &mut LegacyRandom::new(0),
                1.0
            ),
            3.0
        );
        assert!(
            Enchantments::load(&root, &defaults)
                .unwrap()
                .get("wpenchant:loaded")
                .is_none()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A content mod's enchantments (`data/<ns>/enchantment/`, copied into the
    /// resource root beside vanilla's) join the catalog with their own tags and
    /// English names, nested files keep their path, and a world's pack still
    /// replaces one.
    #[test]
    fn mod_namespace_definitions_join_catalog() {
        let root = std::env::temp_dir().join(format!(
            "sunrise-enchantment-mods-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let write = |path: &str, value: &Value| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
            std::fs::read(path).unwrap()
        };
        std::fs::create_dir_all(root.join("data/harvestmod/enchantment/tools")).unwrap();
        std::fs::write(
            root.join("data/harvestmod/enchantment/tools/broken.json"),
            b"{",
        )
        .unwrap();
        let definition = |description: Value, max_level| {
            json!({
                "description": description, "supported_items": "minecraft:diamond_hoe",
                "weight": 1, "max_level": max_level,
                "min_cost": {"base": 1, "per_level_above_first": 0},
                "max_cost": {"base": 1, "per_level_above_first": 0},
                "anvil_cost": 1, "slots": ["mainhand"],
                "exclusive_set": "#harvestmod:exclusive/area"
            })
        };
        write(
            "data/minecraft/enchantment/mending.json",
            &json!({"description": "Mending", "supported_items": "minecraft:diamond_hoe",
                "max_level": 1, "weight": 2, "anvil_cost": 4, "slots": ["any"],
                "min_cost": {"base": 1, "per_level_above_first": 0},
                "max_cost": {"base": 1, "per_level_above_first": 0}}),
        );
        write(
            "data/harvestmod/enchantment/scythe.json",
            &definition(json!({"translate": "enchantment.harvestmod.scythe"}), 3),
        );
        write(
            "data/harvestmod/enchantment/tools/sweep.json",
            &definition(json!("Sweep"), 1),
        );
        write(
            "data/harvestmod/tags/enchantment/exclusive/area.json",
            &json!({"values": ["harvestmod:scythe", "harvestmod:tools/sweep"]}),
        );
        write(
            "data/minecraft/tags/enchantment/in_enchanting_table.json",
            &json!({"values": ["minecraft:mending", "harvestmod:scythe"]}),
        );
        write(
            "assets/harvestmod/lang/en_us.json",
            &json!({"enchantment.harvestmod.scythe": "Scythe", "item.harvestmod.x": "X"}),
        );
        write("items.json", &json!({}));
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let loaded = Enchantments::load(&root, &defaults).unwrap();
        let scythe = loaded
            .get("harvestmod:scythe")
            .expect("a mod's own enchantment must enter the catalog");
        assert_eq!(scythe.description, "Scythe");
        assert_eq!(scythe.max_level, 3);
        assert!(scythe.exclusive.contains("harvestmod:tools/sweep"));
        assert_eq!(
            loaded.get("harvestmod:tools/sweep").unwrap().description,
            "Sweep"
        );
        assert!(loaded.in_tag("harvestmod:exclusive/area", "harvestmod:scythe"));
        assert!(loaded.in_tag("minecraft:in_enchanting_table", "harvestmod:scythe"));
        assert!(loaded.get("minecraft:mending").is_some());
        assert!(loaded.get("minecraft:scythe").is_none());
        assert!(loaded.get("harvestmod:tools/broken").is_none());

        // A world's pack replaces the mod's definition for its id.
        let pack = write(
            "packs/p/data/harvestmod/enchantment/scythe.json",
            &definition(json!("Pack scythe"), 5),
        );
        let files = vec![("enchantment/harvestmod:scythe".to_owned(), vec![pack])];
        let packed = Enchantments::load_with(
            &root,
            &sunrise_world::loot::PackOverlay::new(&files),
            &defaults,
        )
        .unwrap();
        let scythe = packed.get("harvestmod:scythe").unwrap();
        assert_eq!(scythe.description, "Pack scythe");
        assert_eq!(scythe.max_level, 5);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A small registry built from literal JSON matching the vanilla files.
    pub fn sample() -> Enchantments {
        let defs = [
            (
                "minecraft:sharpness",
                json!({"anvil_cost":1,"description":{"translate":"enchantment.minecraft.sharpness"},"exclusive_set":"#minecraft:exclusive_set/damage","max_cost":{"base":21,"per_level_above_first":11},"max_level":5,"min_cost":{"base":1,"per_level_above_first":11},"primary_items":"#minecraft:enchantable/melee_weapon","supported_items":"#minecraft:enchantable/sharp_weapon","weight":10,"slots":["mainhand"],"effects":{"minecraft:damage":[{"effect":{"type":"minecraft:add","value":{"type":"minecraft:linear","base":1.0,"per_level_above_first":0.5}}}]}}),
            ),
            (
                "minecraft:smite",
                json!({"anvil_cost":2,"exclusive_set":"#minecraft:exclusive_set/damage","max_cost":{"base":25,"per_level_above_first":8},"max_level":5,"min_cost":{"base":5,"per_level_above_first":8},"primary_items":"#minecraft:enchantable/melee_weapon","supported_items":"#minecraft:enchantable/weapon","weight":5,"effects":{"minecraft:damage":[{"effect":{"type":"minecraft:add","value":{"type":"minecraft:linear","base":2.5,"per_level_above_first":2.5}},"requirements":{"type":"minecraft:entity_properties","entity":"this","predicate":{"minecraft:entity_type":"#minecraft:sensitive_to_smite"}}}]}}),
            ),
            (
                "minecraft:unbreaking",
                json!({"anvil_cost":2,"max_cost":{"base":55,"per_level_above_first":8},"max_level":3,"min_cost":{"base":5,"per_level_above_first":8},"supported_items":"#minecraft:enchantable/durability","weight":5,"effects":{"minecraft:item_damage":[{"effect":{"type":"minecraft:remove_binomial","chance":{"type":"minecraft:fraction","denominator":{"type":"minecraft:linear","base":2.0,"per_level_above_first":1.0},"numerator":{"type":"minecraft:linear","base":1.0,"per_level_above_first":1.0}}}}]}}),
            ),
            (
                "minecraft:mending",
                json!({"anvil_cost":4,"max_cost":{"base":75,"per_level_above_first":25},"max_level":1,"min_cost":{"base":25,"per_level_above_first":25},"supported_items":"#minecraft:enchantable/durability","weight":2}),
            ),
            (
                "minecraft:feather_falling",
                json!({"anvil_cost":2,"max_cost":{"base":11,"per_level_above_first":6},"max_level":4,"min_cost":{"base":5,"per_level_above_first":6},"supported_items":"#minecraft:enchantable/foot_armor","weight":5,"effects":{"minecraft:damage_protection":[{"effect":{"type":"minecraft:add","value":{"type":"minecraft:linear","base":3.0,"per_level_above_first":3.0}},"requirements":{"type":"minecraft:damage_source_properties","predicate":{"tags":[{"expected":true,"id":"#minecraft:is_fall"},{"expected":false,"id":"#minecraft:bypasses_invulnerability"}]}}}]}}),
            ),
            (
                "minecraft:binding_curse",
                json!({"anvil_cost":8,"description":{"translate":"enchantment.minecraft.binding_curse"},"effects":{"minecraft:prevent_armor_change":{}},"max_cost":{"base":50,"per_level_above_first":0},"max_level":1,"min_cost":{"base":25,"per_level_above_first":0},"slots":["armor"],"supported_items":"#minecraft:enchantable/equippable","weight":1}),
            ),
            // A data pack's own enchantment with Curse of Binding's effect under another id.
            (
                "test:clinging",
                json!({"anvil_cost":8,"description":"Clinging","effects":{"minecraft:prevent_armor_change":{}},"max_cost":{"base":50,"per_level_above_first":0},"max_level":1,"min_cost":{"base":25,"per_level_above_first":0},"slots":["armor"],"supported_items":"#minecraft:enchantable/equippable","weight":1}),
            ),
        ];
        let list: Vec<Enchantment> = defs
            .iter()
            .map(|(id, v)| Enchantment {
                id: Arc::from(*id),
                description: id.trim_start_matches("minecraft:").into(),
                description_key: v["description"]["translate"].as_str().unwrap_or("").into(),
                description_text: sunrise_game_data::scoreboard::comp_text(&v["description"]),
                max_level: v["max_level"].as_u64().unwrap() as u32,
                weight: v["weight"].as_u64().unwrap() as u32,
                anvil_cost: v["anvil_cost"].as_u64().unwrap() as u32,
                min_cost: Cost::parse(&v["min_cost"]),
                max_cost: Cost::parse(&v["max_cost"]),
                supported: string_list(&v["supported_items"]),
                primary: v.get("primary_items").map(string_list),
                exclusive: if v["exclusive_set"].is_null() {
                    HashSet::new()
                } else {
                    [
                        Arc::from("minecraft:sharpness"),
                        Arc::from("minecraft:smite"),
                    ]
                    .into_iter()
                    .collect()
                },
                slots: vec![],
                effects: v["effects"].clone(),
            })
            .collect();
        let index = list
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id.clone(), i))
            .collect();
        let pool: Vec<Arc<str>> = [
            "minecraft:feather_falling",
            "minecraft:sharpness",
            "minecraft:smite",
            "minecraft:unbreaking",
            "minecraft:mending",
        ]
        .iter()
        .map(|s| Arc::from(*s))
        .collect();
        let mut tags = HashMap::new();
        tags.insert(
            "minecraft:in_enchanting_table".to_owned(),
            pool[..4].to_vec(),
        );
        tags.insert(
            "minecraft:tooltip_order".to_owned(),
            vec![
                Arc::from("minecraft:sharpness"),
                Arc::from("minecraft:unbreaking"),
            ],
        );
        let mut entity_tags = HashMap::new();
        entity_tags.insert(
            "minecraft:sensitive_to_smite".to_owned(),
            ["minecraft:zombie".to_owned()].into_iter().collect(),
        );
        let mut damage_tags = HashMap::new();
        damage_tags.insert(
            "minecraft:is_fall".to_owned(),
            ["minecraft:fall".to_owned()].into_iter().collect(),
        );
        let mut props = HashMap::new();
        for (item, value) in [
            ("minecraft:diamond_sword", 10),
            ("minecraft:iron_sword", 14),
            ("minecraft:book", 1),
            ("minecraft:iron_boots", 9),
        ] {
            props.insert(
                Arc::from(item),
                ItemProps {
                    enchantable: value,
                    ..Default::default()
                },
            );
        }
        Enchantments {
            list,
            index,
            bound_effects: HashMap::new(),
            tags,
            entity_tags: Arc::new(entity_tags),
            damage_tags,
            props,
            lang: HashMap::new(),
            primary_answers: Default::default(),
            support_answers: Default::default(),
            providers: BTreeMap::new(),
        }
    }
    pub fn sample_items() -> Items {
        let mut item_tags: HashMap<String, HashSet<String>> = HashMap::new();
        let swords = ["minecraft:diamond_sword", "minecraft:iron_sword"];
        for tag in [
            "enchantable/melee_weapon",
            "enchantable/sharp_weapon",
            "enchantable/weapon",
            "enchantable/durability",
        ] {
            item_tags.insert(
                format!("minecraft:{tag}"),
                swords.iter().map(|s| (*s).to_owned()).collect(),
            );
        }
        item_tags
            .get_mut("minecraft:enchantable/durability")
            .unwrap()
            .insert("minecraft:iron_boots".into());
        item_tags.insert(
            "minecraft:enchantable/foot_armor".into(),
            ["minecraft:iron_boots".to_owned()].into_iter().collect(),
        );
        Items::for_tests(item_tags)
    }

    #[test]
    fn support_precedes_primary_tags_and_primary_answers_can_withhold_books() {
        let (mut e, items) = (sample(), sample_items());
        let sharp: Arc<str> = "minecraft:sharpness".into();
        let index = e.index[&sharp];
        e.list[index].primary = Some(vec![
            "minecraft:stick".into(),
            "minecraft:iron_sword".into(),
        ]);
        let at_30 =
            |item: &str| e.available_results(&items, 30, item, std::slice::from_ref(&sharp));
        assert!(at_30("minecraft:stick").is_empty());
        assert_eq!(at_30("minecraft:iron_sword"), [(sharp.clone(), 3)]);
        e.support_answers().install(HashMap::from([
            (
                "minecraft:stick".into(),
                HashMap::from([(sharp.clone(), true)]),
            ),
            (
                "minecraft:glass".into(),
                HashMap::from([(sharp.clone(), true)]),
            ),
            (
                "minecraft:iron_sword".into(),
                HashMap::from([(sharp.clone(), false)]),
            ),
        ]));
        assert_eq!(
            at_30("minecraft:stick"),
            [(sharp.clone(), 3)],
            "support can admit an item in primary_items but outside supported_items"
        );
        assert!(
            at_30("minecraft:glass").is_empty(),
            "support does not bypass primary_items"
        );
        assert!(
            at_30("minecraft:iron_sword").is_empty(),
            "primary_items does not bypass support"
        );
        assert_eq!(at_30("minecraft:book"), [(sharp.clone(), 3)]);
        e.primary_answers().install(HashMap::from([(
            "minecraft:book".into(),
            HashMap::from([(sharp.clone(), false)]),
        )]));
        assert!(
            at_30("minecraft:book").is_empty(),
            "a primary answer can withhold the plain book's default eligibility"
        );
    }

    #[test]
    fn items_random_enchantment_reads_support_without_filtering_unrestricted_rolls() {
        let mut items = sample_items();
        items.enchantments = sample().into();
        for (item, supported) in [
            ("minecraft:diamond_sword", true),
            ("minecraft:enchanted_book", false),
        ] {
            for compatible in [true, false] {
                let table = json!({"pools":[{"rolls":1,"entries":[{
                    "type":"minecraft:item", "name":item, "modifier":[{
                        "type":"minecraft:enchant_randomly", "options":"minecraft:mending", "only_compatible":compatible
                    }]
                }]}]});
                let roll = || {
                    items.inline_loot_table(&table, &mut sunrise_core::random::LegacyRandom::new(1))
                        [0]
                    .crafting_enchantments()
                    .to_vec()
                };
                let answers = items.enchantments.support_answers();
                for answer in [None, Some(false), Some(true), None] {
                    match answer {
                        Some(value) => answers.install(HashMap::from([(
                            item.into(),
                            HashMap::from([("minecraft:mending".into(), value)]),
                        )])),
                        None => answers.clear(),
                    }
                    if !compatible || answer.unwrap_or(supported) {
                        assert_eq!(roll(), [(Arc::from("minecraft:mending"), 1)]);
                    } else {
                        assert!(roll().is_empty(), "{item}, {answer:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn level_based_values() {
        assert_eq!(
            level_value(
                &json!({"type":"minecraft:linear","base":1.0,"per_level_above_first":0.5}),
                5
            ),
            3.0
        );
        assert_eq!(
            level_value(&json!({"type":"minecraft:levels_squared","added":1.0}), 3),
            10.0
        );
        assert_eq!(
            level_value(
                &json!({"type":"minecraft:fraction","numerator":{"type":"minecraft:linear","base":1.0,"per_level_above_first":1.0},"denominator":{"type":"minecraft:linear","base":2.0,"per_level_above_first":1.0}}),
                3
            ),
            0.75
        );
        assert_eq!(level_name(4), "IV");
        assert_eq!(
            Cost {
                base: 1,
                per_level: 11
            }
            .calculate(5),
            45
        );
    }

    #[test]
    fn enchantment_cost_wraps_like_java_int() {
        let cost = Cost {
            base: i32::MAX,
            per_level: 1,
        };
        assert_eq!(cost.calculate(2), i32::MIN);
    }

    #[test]
    fn enchanting_cost_matches_vanilla_formula() {
        let e = sample();
        // Replays EnchantmentHelper.getEnchantmentCost draws by hand.
        for seed in [0i64, 1, 42, -7, 123456] {
            for bookcases in [0, 5, 15, 20] {
                let mut a = LegacyRandom::new(seed);
                let mut b = LegacyRandom::new(seed);
                let costs: Vec<i32> = (0..3)
                    .map(|slot| {
                        e.enchantment_cost(&mut a, slot, bookcases, "minecraft:diamond_sword")
                    })
                    .collect();
                let bc = bookcases.min(15);
                let expect: Vec<i32> = (0..3)
                    .map(|slot| {
                        let s = b.next_int_bound(8).unwrap()
                            + 1
                            + (bc >> 1)
                            + b.next_int_bound(bc + 1).unwrap();
                        match slot {
                            0 => (s / 3).max(1),
                            1 => s * 2 / 3 + 1,
                            _ => s.max(bc * 2),
                        }
                    })
                    .collect();
                assert_eq!(costs, expect);
                assert!(costs[2] <= 30);
            }
        }
        assert_eq!(
            e.enchantment_cost(&mut LegacyRandom::new(0), 0, 15, "minecraft:stick"),
            0
        );
    }

    #[test]
    fn selection_respects_costs_and_exclusivity() {
        let e = sample();
        let items = sample_items();
        let pool = e.tag("minecraft:in_enchanting_table").to_vec();
        for seed in 0..200 {
            let mut r = LegacyRandom::new(seed);
            let picked = e.select(
                &items,
                &mut r,
                &ItemStack::new("minecraft:diamond_sword", 1),
                30,
                &pool,
            );
            assert!(!picked.is_empty());
            let has = |id: &str| picked.iter().any(|(p, _)| &**p == id);
            assert!(
                !(has("minecraft:sharpness") && has("minecraft:smite")),
                "{picked:?}"
            );
            assert!(!has("minecraft:feather_falling"));
            for (id, l) in &picked {
                assert!(*l >= 1 && *l <= e.get(id).unwrap().max_level);
            }
        }
        // Books accept every enchantment in the pool.
        let mut r = LegacyRandom::new(3);
        let available = e.available_results(&items, 20, "minecraft:book", &pool);
        assert!(
            available
                .iter()
                .any(|(id, _)| &**id == "minecraft:feather_falling")
        );
        assert!(
            e.select(
                &items,
                &mut r,
                &ItemStack::new("minecraft:stick", 1),
                30,
                &pool
            )
            .is_empty()
        );
    }

    #[test]
    fn damage_source_requirements_match_each_entity_role_and_refuse_absent_entities() {
        let mut enchantments = sample();
        let id: Arc<str> = "minecraft:sharpness".into();
        let requirements = json!({"condition":"minecraft:damage_source_properties","predicate":{
            "source_entity":{"minecraft:entity_type":"minecraft:skeleton"},
            "direct_entity":{"minecraft:entity_type":"minecraft:arrow"},
            "is_direct":false
        }});
        let index = enchantments.index[id.as_ref()];
        enchantments.list[index].slots = vec!["hand".into()];
        enchantments.list[index].effects = json!({
            "minecraft:damage_immunity":[{"requirements":requirements}],
            "minecraft:damage":[{"requirements":requirements,"effect":{"type":"minecraft:add","value":3}}]
        });
        let levels = [(id, 1)];
        let arrow = EffectContext {
            damage_type: Some("minecraft:arrow"),
            attacker: Some("minecraft:skeleton"),
            direct_attacker: Some("minecraft:arrow"),
            ..Default::default()
        };
        for (ctx, matches) in [
            (arrow, true),
            (
                EffectContext {
                    attacker: None,
                    ..arrow
                },
                false,
            ),
            (
                EffectContext {
                    direct_attacker: None,
                    ..arrow
                },
                false,
            ),
            (
                EffectContext {
                    attacker: arrow.direct_attacker,
                    direct_attacker: arrow.attacker,
                    ..arrow
                },
                false,
            ),
            (
                EffectContext {
                    damage_type: Some("minecraft:cactus"),
                    is_direct: true,
                    ..Default::default()
                },
                false,
            ),
        ] {
            let mut random = LegacyRandom::new(0);
            assert_eq!(
                enchantments.is_damage_immune(
                    enchantments.damage_immunities([(&levels[..], "mainhand")]),
                    &ctx,
                    &mut random,
                ),
                matches
            );
            assert_eq!(
                enchantments.modify_entity_value(
                    &levels,
                    "minecraft:damage",
                    &ctx,
                    &mut random,
                    &mut LegacyRandom::new(0),
                    2.0,
                ),
                if matches { 5.0 } else { 2.0 }
            );
        }
    }

    #[test]
    fn value_effects_and_requirements() {
        let e = sample();
        let items = sample_items();
        let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
        sword.enchant("minecraft:sharpness", 5);
        let mut r = LegacyRandom::new(1);
        let ctx = EffectContext {
            items: Some(&items),
            this: Some("minecraft:zombie"),
            ..Default::default()
        };
        assert_eq!(
            e.modify_value(Some(&sword), "minecraft:damage", &ctx, &mut r, 0.0),
            3.0
        );
        let mut smite = ItemStack::new("minecraft:diamond_sword", 1);
        smite.enchant("minecraft:smite", 2);
        assert_eq!(
            e.modify_value(Some(&smite), "minecraft:damage", &ctx, &mut r, 0.0),
            5.0
        );
        let pig = EffectContext {
            this: Some("minecraft:pig"),
            ..ctx
        };
        assert_eq!(
            e.modify_value(Some(&smite), "minecraft:damage", &pig, &mut r, 0.0),
            0.0
        );
        let mut boots = ItemStack::new("minecraft:iron_boots", 1);
        boots.enchant("minecraft:feather_falling", 4);
        let fall = EffectContext {
            damage_type: Some("minecraft:fall"),
            ..Default::default()
        };
        assert_eq!(
            e.modify_value(
                Some(&boots),
                "minecraft:damage_protection",
                &fall,
                &mut r,
                0.0
            ),
            12.0
        );
        let lava = EffectContext {
            damage_type: Some("minecraft:lava"),
            ..Default::default()
        };
        assert_eq!(
            e.modify_value(
                Some(&boots),
                "minecraft:damage_protection",
                &lava,
                &mut r,
                0.0
            ),
            0.0
        );
        // Unbreaking III keeps about 1/4 of durability hits.
        let mut tool = ItemStack::new("minecraft:diamond_sword", 1);
        tool.enchant("minecraft:unbreaking", 3);
        let kept: f32 = (0..4000)
            .map(|_| e.modify_value(Some(&tool), "minecraft:item_damage", &ctx, &mut r, 1.0))
            .sum();
        assert!((800.0..1200.0).contains(&kept), "{kept}");
    }

    #[test]
    fn names_and_tooltip_order() {
        let e = sample();
        assert_eq!(e.full_name("minecraft:mending", 1).0, "mending");
        assert_eq!(e.full_name("minecraft:sharpness", 1).0, "sharpness I");
        // PvP & HUD's numbers: any level, past vanilla's numerals too; a one-level
        // enchantment still has none.
        let number = |id, level| e.full_name_with(None, id, level, true).0;
        assert_eq!(number("minecraft:sharpness", 5), "sharpness 5");
        assert_eq!(number("minecraft:sharpness", 255), "sharpness 255");
        assert_eq!(number("minecraft:mending", 1), "mending");
        let levels = vec![
            (Arc::from("minecraft:unbreaking"), 3),
            (Arc::from("minecraft:mending"), 1),
            (Arc::from("minecraft:sharpness"), 2),
        ];
        let lines: Vec<String> = e.tooltip(&levels).into_iter().map(|l| l.0).collect();
        assert_eq!(lines, ["sharpness II", "unbreaking III", "mending"]);
        assert!(!e.compatible("minecraft:sharpness", "minecraft:smite"));
        assert!(e.compatible("minecraft:sharpness", "minecraft:unbreaking"));
    }

    #[test]
    fn high_enchantment_names_keep_the_untranslated_key() {
        let mut e = sample();
        assert_eq!(
            e.full_name_in(None, "minecraft:sharpness", 255).0,
            "sharpness enchantment.level.255"
        );
        // A pack may supply names beyond vanilla's I through X, including
        // the English fallback used when no client language is loaded.
        e.lang.insert("enchantment.level.11".into(), "XI".into());
        assert_eq!(
            e.full_name_in(None, "minecraft:sharpness", 11).0,
            "sharpness XI"
        );
    }

    /// A server's data-pack enchantment the game lacks is named by the
    /// `description` it sent (`Enchantment.getFullname`), with the level
    /// numeral unless both the level and the maximum are 1; the game's own
    /// entries keep their tables.
    #[test]
    fn synced_entries_name_themselves() {
        let e = sample();
        assert_eq!(e.full_name("enchex:drill", 2).0, "enchex:drill II");
        let drill = json!({"description":{"text":"Drill"},"max_level":3,"supported_items":"#minecraft:enchantable/mining"});
        let hex = json!({"description":"Hex","max_level":1});
        let other = json!({"description":{"text":"Other"},"max_level":5});
        let synced = e
            .with_entries([
                ("enchex:drill", &drill),
                ("enchex:hex", &hex),
                ("minecraft:sharpness", &other),
            ])
            .unwrap()
            .unwrap();
        assert_eq!(synced.full_name("enchex:drill", 2).0, "Drill II");
        assert_eq!(synced.full_name("enchex:drill", 1).0, "Drill I");
        assert_eq!(synced.full_name("enchex:hex", 1).0, "Hex");
        assert_eq!(synced.full_name("minecraft:sharpness", 1).0, "sharpness I");
        assert_eq!(synced.list.len(), e.list.len() + 2);
        // Nothing new: the game's table stays in use.
        assert!(
            e.with_entries([("minecraft:sharpness", &other)])
                .unwrap()
                .is_none()
        );
    }

    /// The enchantment's name and its level come from the selected language,
    /// with the English name loaded above as the fallback.
    #[test]
    fn names_follow_the_language() {
        struct Mem(HashMap<String, Vec<u8>>);
        impl sunrise_game_data::lang::ResourceSource for Mem {
            fn read(&self, path: &str) -> Option<Vec<u8>> {
                self.0.get(path).cloned()
            }
        }
        let ru = Mem(HashMap::from([(
            "assets/minecraft/lang/ru_ru.json".to_owned(),
            r#"{"enchantment.minecraft.sharpness":"Острота","enchantment.level.5":"V"}"#
                .as_bytes()
                .to_vec(),
        )]));
        let ru = sunrise_game_data::lang::Language::load(&[&ru], "ru_ru");
        let e = sample();
        assert_eq!(
            e.full_name_in(Some(&ru), "minecraft:sharpness", 5).0,
            "Острота V"
        );
        // mending carries no key in the fixture: the English name stays.
        assert_eq!(
            e.full_name_in(Some(&ru), "minecraft:mending", 1).0,
            "mending"
        );
    }
}

#[cfg(test)]
mod ignite_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn false_conditions_do_not_suppress_enchantment_tag_operations() {
        let root = std::env::temp_dir().join(format!(
            "sunrise-enchantment-tag-conditions-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("data/minecraft/tags/enchantment")).unwrap();
        let entries = root.join("data/minecraft/enchantment");
        std::fs::create_dir_all(&entries).unwrap();
        for id in ["sharpness", "smite"] {
            std::fs::write(entries.join(format!("{id}.json")), "{}").unwrap();
        }
        std::fs::write(root.join("items.json"), "{}").unwrap();
        std::fs::write(root.join("data/minecraft/tags/enchantment/conditional.json"), json!({"neoforge:conditions":[{"type":"neoforge:never"}],"values":["minecraft:sharpness"]}).to_string()).unwrap();
        let files = vec![("tags/minecraft:enchantment/conditional".into(), vec![json!({"neoforge:conditions":[{"type":"neoforge:never"}],"values":["minecraft:smite"],"remove":["minecraft:sharpness"]}).to_string().into_bytes()])];
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let base = Enchantments::load(&root, &defaults).unwrap();
        assert_eq!(
            base.tag("minecraft:conditional")
                .iter()
                .map(|id| id.as_ref())
                .collect::<Vec<_>>(),
            ["minecraft:sharpness"]
        );
        let loaded = Enchantments::load_with(
            &root,
            &sunrise_world::loot::PackOverlay::new(&files),
            &defaults,
        )
        .unwrap();
        assert_eq!(
            loaded
                .tag("minecraft:conditional")
                .iter()
                .map(|id| id.as_ref())
                .collect::<Vec<_>>(),
            ["minecraft:smite"]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Fire Aspect as the data pack has it: 4 seconds per level.
    fn registry() -> Enchantments {
        let fire_aspect = Enchantment {
            id: Arc::from("minecraft:fire_aspect"),
            description: "fire_aspect".into(),
            description_key: String::new(),
            description_text: sunrise_game_data::text::Text::literal("fire_aspect"),
            max_level: 2,
            weight: 2,
            anvil_cost: 4,
            min_cost: Cost::parse(&json!({"base":10,"per_level_above_first":20})),
            max_cost: Cost::parse(&json!({"base":60,"per_level_above_first":20})),
            supported: vec![],
            primary: None,
            exclusive: HashSet::new(),
            slots: vec![],
            effects: json!({"minecraft:post_attack":[{"effect":{"type":"minecraft:ignite","duration":{"type":"minecraft:linear","base":4.0,"per_level_above_first":4.0}},"enchanted":"attacker","affected":"victim"}]}),
        };
        let index = [(fire_aspect.id.clone(), 0)].into_iter().collect();
        Enchantments {
            list: vec![fire_aspect],
            index,
            ..Default::default()
        }
    }

    #[test]
    fn weight_totals_enchantments() {
        for weights in [
            [0, 0, 0],
            [1, 2, 3],
            [7, 0, 11],
            [i32::MAX as u32; 3],
            [u32::MAX; 3],
        ] {
            let mut reg = registry();
            let template = reg.list[0].clone();
            reg.list.clear();
            reg.index.clear();
            let mut list = vec![];
            for (i, weight) in weights.into_iter().enumerate() {
                let mut e = template.clone();
                e.id = Arc::from(format!("test:{i}"));
                e.weight = weight;
                list.push((e.id.clone(), 1));
                reg.index.insert(e.id.clone(), i);
                reg.list.push(e);
            }
            let total: i64 = weights.iter().map(|&w| i64::from(w)).sum();
            for seed in 0..64 {
                let mut expected = LegacyRandom::new(seed);
                let pick = if total == 0 {
                    None
                } else {
                    let mut at = if total <= i64::from(i32::MAX) {
                        i64::from(expected.next_int_bound(total as i32).unwrap())
                    } else {
                        ((expected.next_long() as u64) % total as u64) as i64
                    };
                    weights
                        .iter()
                        .position(|&w| {
                            at -= i64::from(w);
                            at < 0
                        })
                        .map(|i| list[i].clone())
                };
                let mut random = LegacyRandom::new(seed);
                assert_eq!(reg.weighted(&mut random, &list), pick);
                assert_eq!(random.next_long(), expected.next_long());
            }
            assert!(reg.weighted(&mut LegacyRandom::new(0), &[]).is_none());
        }
    }

    #[test]
    fn damage_source_properties_match_source_and_direct_entities() {
        let e = super::tests::sample();
        let mut random = LegacyRandom::new(0);
        let predicate = |field: &str, kind: &str| {
            json!({
                "type": "minecraft:damage_source_properties",
                "predicate": {field: {"minecraft:entity_type": kind}}
            })
        };
        let source_zombie = predicate("source_entity", "minecraft:zombie");
        let direct_player = predicate("direct_entity", "minecraft:player");
        let none = EffectContext {
            damage_type: Some("minecraft:generic"),
            ..Default::default()
        };
        assert!(
            !e.condition(&source_zombie, 1, &none, &mut random),
            "source_entity requires an attacker"
        );
        assert!(
            !e.condition(&direct_player, 1, &none, &mut random),
            "direct_entity requires a direct attacker"
        );
        let hit = EffectContext {
            damage_type: Some("minecraft:generic"),
            attacker: Some("minecraft:zombie"),
            direct_attacker: Some("minecraft:player"),
            ..Default::default()
        };
        assert!(e.condition(&source_zombie, 1, &hit, &mut random));
        assert!(e.condition(&direct_player, 1, &hit, &mut random));
        assert!(
            !e.condition(
                &predicate("direct_entity", "minecraft:zombie"),
                1,
                &hit,
                &mut random
            ),
            "the direct entity is evaluated separately from the source"
        );
        let legacy = json!({
            "type": "minecraft:damage_source_properties",
            "predicate": {"source_entity": {"type": "minecraft:zombie"}}
        });
        assert!(
            e.condition(&legacy, 1, &hit, &mut random),
            "the legacy `type` key still names the entity type"
        );
    }

    #[test]
    fn damage_immunity_requires_the_matching_attacker() {
        use std::sync::Arc;

        let definition = json!({
            "description": "Zombie ward",
            "max_level": 1,
            "weight": 1,
            "anvil_cost": 0,
            "supported_items": "#minecraft:enchantable/foot_armor",
            "effects": {"minecraft:damage_immunity": [{
                "requirements": {
                    "type": "minecraft:damage_source_properties",
                    "predicate": {"source_entity": {
                        "minecraft:entity_type": "minecraft:zombie"
                    }}
                }
            }]}
        });
        let table = super::tests::sample()
            .with_entries([("test:zombie_ward", &definition)])
            .unwrap()
            .unwrap();
        let active = [(Arc::<str>::from("test:zombie_ward"), 1)];
        let active = active.iter().map(|(id, level)| (id, *level));
        let mut random = LegacyRandom::new(0);
        let zombie = EffectContext {
            damage_type: Some("minecraft:generic"),
            attacker: Some("minecraft:zombie"),
            ..Default::default()
        };
        assert!(table.is_damage_immune(active, &zombie, &mut random));
        let active = [(Arc::<str>::from("test:zombie_ward"), 1)];
        let active = active.iter().map(|(id, level)| (id, *level));
        let skeleton = EffectContext {
            damage_type: Some("minecraft:generic"),
            attacker: Some("minecraft:skeleton"),
            ..Default::default()
        };
        assert!(!table.is_damage_immune(active, &skeleton, &mut random));
        let active = [(Arc::<str>::from("test:zombie_ward"), 1)];
        let active = active.iter().map(|(id, level)| (id, *level));
        let absent = EffectContext {
            damage_type: Some("minecraft:generic"),
            ..Default::default()
        };
        assert!(
            !table.is_damage_immune(active, &absent, &mut random),
            "a source_entity requirement fails when the attacker is absent"
        );
    }

    #[test]
    fn a_source_entity_requirement_gates_a_damage_effect() {
        let definition = json!({
            "description": "Zombie bane",
            "max_level": 1,
            "weight": 1,
            "anvil_cost": 0,
            "supported_items": "#minecraft:enchantable/sword",
            "effects": {"minecraft:damage": [{
                "effect": {"type": "minecraft:add", "value": 2.0},
                "requirements": {
                    "type": "minecraft:damage_source_properties",
                    "predicate": {"source_entity": {
                        "minecraft:entity_type": "minecraft:zombie"
                    }}
                }
            }]}
        });
        let table = super::tests::sample()
            .with_entries([("test:zombie_bane", &definition)])
            .unwrap()
            .unwrap();
        let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
        sword.enchant("test:zombie_bane", 1);
        let mut random = LegacyRandom::new(0);
        let zombie = EffectContext {
            damage_type: Some("minecraft:mob_attack"),
            attacker: Some("minecraft:zombie"),
            ..Default::default()
        };
        assert_eq!(
            table.modify_value(Some(&sword), "minecraft:damage", &zombie, &mut random, 0.0),
            2.0
        );
        let skeleton = EffectContext {
            damage_type: Some("minecraft:mob_attack"),
            attacker: Some("minecraft:skeleton"),
            ..Default::default()
        };
        assert_eq!(
            table.modify_value(
                Some(&sword),
                "minecraft:damage",
                &skeleton,
                &mut random,
                0.0
            ),
            0.0
        );
        let absent = EffectContext::default();
        assert_eq!(
            table.modify_value(Some(&sword), "minecraft:damage", &absent, &mut random, 0.0),
            0.0
        );
    }

    #[test]
    fn fire_aspect_ignites_for_four_seconds_a_level() {
        let reg = registry();
        let mut random = LegacyRandom::new(0);
        let ctx = EffectContext {
            this: Some("minecraft:player"),
            ..Default::default()
        };
        let mut fire = |level| {
            let held = [(Arc::from("minecraft:fire_aspect"), level)];
            reg.ignite_seconds(&held, &ctx, &mut random)
        };
        assert_eq!(fire(1) * 20.0, 80.0);
        assert_eq!(fire(2) * 20.0, 160.0);
        assert_eq!(reg.ignite_seconds(&[], &ctx, &mut random), 0.0);
    }
}
