//! Items from vanilla's default-component reports, block<->item mapping,
//! tool mining rules and block loot tables (`LootTable`/`LootPool` semantics
//! for the condition, entry and modifier types used by block drops).
mod entity_type_tags;
#[path = "ingredient_defaults.rs"]
pub(crate) mod ingredient_defaults;
pub use entity_type_tags::EntityTypeTags;
#[path = "loot_query.rs"]
mod loot_query;
use crate::{Result, blocks::BlockFacts};
pub use ingredient_defaults::LoadEntityTags;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
    sync::Arc,
};
use sunrise_core::{nbt::Tag, random::RandomSource};
use sunrise_world::loot::number::int_value;
use sunrise_world::registry::{BlockRegistry, BlockStateId};
use sunrise_world::{
    item_defaults::ItemDefaults,
    loot::{ItemTagNames, MAX_LOOT_LEVEL, PackOverlay},
    tag_layers,
};

#[cfg(test)]
#[path = "loot_copy_tests.rs"]
mod copy_tests;
mod loot_conditions;

fn tool_number(value: &sunrise_core::nbt::Tag) -> Option<f32> {
    sunrise_world::datafix::dynamic::as_number(value).map(|n| n.float_value())
}

/// A fire starter's use sound: `SoundEvents.FIRECHARGE_USE` or
/// `FLINTANDSTEEL_USE`.
pub fn fire_starter_sound(fire_charge: bool) -> &'static str {
    if fire_charge {
        "minecraft:item.firecharge.use"
    } else {
        "minecraft:item.flintandsteel.use"
    }
}

/// The pitch of a fire starter's use sound, from the level's random:
/// `FlintAndSteelItem.useOn`'s `nextFloat() * 0.4 + 0.8`, or
/// `FireChargeItem.playSound`'s `(nextFloat() - nextFloat()) * 0.2 + 1.0`.
pub fn fire_starter_pitch(fire_charge: bool, random: &mut impl RandomSource) -> f32 {
    if fire_charge {
        (random.next_float() - random.next_float()) * 0.2 + 1.0
    } else {
        random.next_float() * 0.4 + 0.8
    }
}

#[derive(Clone, Debug)]
pub struct ToolRule {
    pub blocks: Vec<String>,
    pub speed: Option<f32>,
    pub correct: Option<bool>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ConsumeEffectSpec {
    pub id: String,
    pub duration: i32,
    pub amplifier: i32,
    /// `MobEffectInstance`'s `ambient` (default false) and `show_particles` (default true).
    pub ambient: bool,
    pub show_particles: bool,
    /// `show_icon`, which defaults to `show_particles`.
    pub show_icon: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConsumeEffect {
    ApplyEffects {
        effects: Vec<ConsumeEffectSpec>,
        probability: f32,
    },
    ClearAllEffects,
    RemoveEffects(Vec<String>),
    /// `TeleportRandomlyConsumeEffect`.
    TeleportRandomly(sunrise_world::teleport_consume::TeleportConsume),
    PlaySound(ConsumeSound),
    /// A mod's effect and its decode-time dispatch binding.
    Native(NativeConsumeEffect),
}

/// `SoundEvent.CODEC`: a registry reference or an inline event with its own range.
#[derive(Clone, Debug, PartialEq)]
pub enum ConsumeSound {
    Registered(String),
    Direct { id: String, range: Option<f32> },
}

// no wire: the callback binding is local to the installed native mods.
#[derive(Clone, Debug)]
pub struct NativeConsumeEffect {
    pub kind: Arc<str>,
    pub data: Arc<sunrise_core::nbt::Tag>,
    pub registered: Option<sunrise_world::component_value::consume_effects::Admitted>,
    pub binding: Option<mod_vocab::__private::ConsumeEffectBinding>,
}

impl PartialEq for NativeConsumeEffect {
    fn eq(&self, other: &Self) -> bool {
        // Component equality is about the saved data, not the installed callable.
        self.kind == other.kind && self.data == other.data && self.registered == other.registered
    }
}

/// Typed consumable shared by item prototypes and stack component patches.
pub type ItemConsumable = sunrise_world::consumable::Consumable<String, ConsumeEffect>;

/// Decode `Consumable.CODEC` defaults and ordered effects at the load boundary.
/// Opaque mod payloads are converted once to retained NBT; invalid payloads refuse admission.
pub fn parse_consumable(
    cons: &Value,
) -> std::result::Result<ItemConsumable, sunrise_core::json_nbt::JsonNbtError> {
    let mut effects = Vec::new();
    for effect in cons["on_consume_effects"].as_array().into_iter().flatten() {
        let Some(kind) = effect["type"].as_str() else {
            continue;
        };
        if let Some(builtin) = builtin_consume_effect_path(kind) {
            if let Ok(tag) = sunrise_core::json_nbt::json_to_tag(effect)
                && let Some(effect) = builtin_consume_effect(builtin, &tag)
            {
                effects.push(effect);
            }
        } else {
            effects.push(ConsumeEffect::Native(NativeConsumeEffect {
                kind: Arc::from(kind),
                data: Arc::new(sunrise_core::json_nbt::json_to_tag(effect)?),
                registered: None,
                binding: mod_vocab::__private::ConsumeEffectBinding::resolve(kind),
            }));
        }
    }
    Ok(ItemConsumable {
        seconds: cons["consume_seconds"].as_f64().map_or(1.6, |s| s as f32),
        animation: crate::use_animation::consumable_animation(cons),
        sound: cons["sound"]
            .as_str()
            .or_else(|| cons["sound"]["sound_id"].as_str())
            .unwrap_or("minecraft:entity.generic.eat")
            .to_owned(),
        particles: cons["has_consume_particles"].as_bool().unwrap_or(true),
        effects,
    })
}

fn builtin_consume_effect_path(kind: &str) -> Option<&str> {
    // `Identifier.bySeparator`: a bare or empty namespace is minecraft's.
    match kind.split_once(':') {
        Some(("minecraft" | "", path)) => Some(path),
        None => Some(kind),
        Some(_) => None,
    }
}

/// Decode a built-in `ConsumeEffect.CODEC` entry from its tagged fields.
fn builtin_consume_effect(builtin: &str, effect: &Tag) -> Option<ConsumeEffect> {
    use crate::{item_data::component_bool, properties::component_float};

    match builtin {
        "apply_effects" => Some(ConsumeEffect::ApplyEffects {
            effects: match effect.get("effects") {
                Some(Tag::List(list)) => list.iter().filter_map(consume_effect_spec).collect(),
                _ => Vec::new(),
            },
            // `ApplyStatusEffectsConsumeEffect.CODEC`: floatRange(0, 1).
            probability: match effect.get("probability") {
                Some(value) => component_float(value).filter(|p| (0.0..=1.0).contains(p))?,
                None => 1.0,
            },
        }),
        "clear_all_effects" => Some(ConsumeEffect::ClearAllEffects),
        "remove_effects" => Some(ConsumeEffect::RemoveEffects(match effect.get("effects") {
            Some(Tag::String(id)) => vec![id.to_string_lossy()],
            Some(Tag::List(ids)) => ids
                .iter()
                .filter_map(|id| match id {
                    Tag::String(id) => Some(id.to_string_lossy()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        })),
        "teleport_randomly" => Some(ConsumeEffect::TeleportRandomly(
            sunrise_world::teleport_consume::TeleportConsume {
                // `TeleportRandomlyConsumeEffect.CODEC`: ExtraCodecs.POSITIVE_FLOAT.
                diameter: match effect.get("diameter") {
                    Some(value) => component_float(value).filter(|d| *d > 0.0 && d.is_finite())?,
                    None => 16.0,
                },
                directional_particles: effect
                    .get("directional_particles")
                    .and_then(component_bool)
                    .unwrap_or(true),
            },
        )),
        "play_sound" => {
            use sunrise_core::identifier::Identifier;
            let sound = effect.get("sound")?;
            let holder = match sound {
                Tag::String(id) => ConsumeSound::Registered(
                    Identifier::parse(&id.to_string_lossy()).ok()?.to_string(),
                ),
                Tag::Compound(_) => {
                    let Tag::String(id) = sound.get("sound_id")? else {
                        return None;
                    };
                    ConsumeSound::Direct {
                        id: Identifier::parse(&id.to_string_lossy()).ok()?.to_string(),
                        // SoundEvent.DIRECT_CODEC uses lenientOptionalFieldOf("range").
                        range: sound.get("range").and_then(component_float),
                    }
                }
                _ => return None,
            };
            Some(ConsumeEffect::PlaySound(holder))
        }
        _ => None,
    }
}

/// Admit custom effects from saved NBT, or install effects already decoded by
/// the connection's Wire codecs. Decode built-ins directly from their tagged fields.
pub(crate) fn stack_effects(
    raw: &mut sunrise_core::nbt::Tag,
    key: &str,
    received: Option<&[sunrise_world::component_value::consume_effects::Admitted]>,
    mut refused: Option<&mut bool>,
) -> std::result::Result<Vec<ConsumeEffect>, String> {
    use sunrise_world::component_value::consume_effects::admit;

    let Tag::Compound(_) = raw else {
        return Err("expected effect component map".into());
    };
    let list = match raw.get_mut(key) {
        None => return Ok(Vec::new()),
        Some(Tag::List(list)) => list,
        Some(Tag::ByteArray(a)) if a.is_empty() => return Ok(Vec::new()),
        Some(Tag::IntArray(a)) if a.is_empty() => return Ok(Vec::new()),
        Some(Tag::LongArray(a)) if a.is_empty() => return Ok(Vec::new()),
        _ => return Err("expected consume effect list".into()),
    };
    let mut effects = Vec::with_capacity(list.len());
    for entry in list {
        let Some(Tag::String(kind)) = entry.get("type") else {
            continue;
        };
        let kind = kind.to_string_lossy();
        if let Some(builtin) = builtin_consume_effect_path(&kind) {
            effects.extend(builtin_consume_effect(builtin, entry));
        } else {
            let Tag::Compound(fields) = entry else {
                return Err("expected consume effect map".into());
            };
            let registered = match received {
                Some(received) => received
                    .iter()
                    .find(|effect| effect.saved.as_ref() == entry)
                    .cloned(),
                None => match admit(&kind, fields) {
                    Ok(effect) => effect,
                    Err(error) => {
                        // The registered codec already reported the error. Preserve this
                        // saved entry, but don't let it run or discard its siblings.
                        let Some(refused) = refused.as_deref_mut() else {
                            return Err(error);
                        };
                        *refused = true;
                        continue;
                    }
                },
            };
            let data = if let Some(effect) = &registered {
                *entry = (*effect.saved).clone();
                effect.saved.clone()
            } else {
                // An unregistered kind keeps its saved NBT as written: bytes, floats and
                // any UTF-16 string reach the hook unchanged, as a mod's own codec reads
                // them. Only a non-finite number or a nested end tag (which the NBT
                // writer refuses, so the stack could not be saved) is refused, like a
                // codec error: the entry is kept in the saved tag but does not run, and
                // its siblings do.
                if !admissible_payload(entry) {
                    let Some(refused) = refused.as_deref_mut() else {
                        return Err(
                            "non-finite number or nested end tag in consume effect payload".into(),
                        );
                    };
                    *refused = true;
                    continue;
                }
                Arc::new(entry.clone())
            };
            let binding = mod_vocab::__private::ConsumeEffectBinding::resolve(&kind);
            effects.push(ConsumeEffect::Native(NativeConsumeEffect {
                kind: Arc::from(kind),
                data,
                registered,
                binding,
            }));
        }
    }
    Ok(effects)
}

/// Whether every float and double in `tag` is finite and no end tag is nested in
/// it (the payload refusals left for an unregistered consume-effect kind; the NBT
/// writer refuses a nested end tag).
fn admissible_payload(tag: &sunrise_core::nbt::Tag) -> bool {
    let mut work = vec![tag];
    while let Some(tag) = work.pop() {
        match tag {
            Tag::End => return false,
            Tag::Float(bits) => {
                if !f32::from_bits(*bits).is_finite() {
                    return false;
                }
            }
            Tag::Double(bits) => {
                if !f64::from_bits(*bits).is_finite() {
                    return false;
                }
            }
            Tag::List(values) => work.extend(values),
            Tag::Compound(fields) => work.extend(fields.iter().map(|(_, value)| value)),
            _ => {}
        }
    }
    true
}

#[derive(Clone, Debug)]
pub struct ItemInfo {
    pub id: Arc<str>,
    /// Default components decoded once, for schema-aware saved patch equality.
    pub component_prototype: ItemStack,
    pub max_stack: u32,
    pub max_damage: u32,
    /// Prototype damage, cached when default components are loaded.
    pub damage: i32,
    pub block: Option<BlockStateId>,
    pub tool: Vec<ToolRule>,
    pub tool_default_speed: f32,
    /// The `tool` component's `damage_per_block`, when the item has the
    /// component (a mace or trident has one without rules).
    pub tool_damage_per_block: Option<u32>,
    /// `tool.can_destroy_blocks_in_creative` (defaults to true).
    pub can_destroy_blocks_in_creative: bool,
    pub food: Option<(u32, f32)>,
    pub can_always_eat: bool,
    /// `consumable`: its timing, sound, particles and `on_consume_effects`.
    pub consumable: Option<Arc<ItemConsumable>>,
    /// Death protection effects decoded at definition load.
    pub death_protection: Option<Vec<ConsumeEffect>>,
    /// Kinetic pose timing decoded once from the shared item prototype.
    pub kinetic_timing: Option<crate::kinetic_weapon::UseTiming>,
    /// `use_cooldown`: the cooldown a use starts (`UseCooldown`).
    pub use_cooldown: Option<crate::cooldowns::UseCooldown>,
    pub model: String,
    /// Main-hand `attack_damage` / `attack_speed` modifier amounts.
    pub attack_damage: f32,
    pub attack_speed: f32,
    /// `cooking_fuel`: burn-time and speed-multiplier provider references.
    pub fuel: Option<(Value, Value)>,
    /// `brewing_fuel`: uses and speed-multiplier provider references.
    pub brewing_fuel: Option<(Value, Value)>,
    /// `compostable.layers`: the layers provider (`ComposterBlock.addLayer`).
    pub compostable: Option<Value>,
    /// Item left behind after crafting/fuel use (e.g. bucket).
    pub remainder: Option<String>,
    /// `equippable.slot` (head, chest, legs, feet, ...).
    pub equip_slot: Option<String>,
    /// `equippable.asset_id`: the equipment asset drawn when worn.
    pub equip_asset: Option<String>,
    /// Texture overlaid when worn in first person.
    pub equip_camera_overlay: Option<String>,
    /// All `attribute_modifiers`.
    pub modifiers: Vec<crate::item_data::ModifierRow>,
    /// `blocks_attacks` (shields).
    pub blocks_attacks: Option<BlocksAttacks>,
    /// The arm pose a mod item declares for a humanoid holding it (`arm_pose`).
    pub arm_pose: Option<sunrise_core::arm_pose::ArmPose>,
    /// `weapon.disable_blocking_for_seconds` (axes).
    pub disable_blocking_seconds: f32,
    /// `weapon.item_damage_per_attack`, when the item is a weapon.
    pub weapon_damage: Option<u32>,
    /// `villager_food.nutrition`: what a villager gets from eating it.
    pub villager_food: Option<i32>,
    /// `use_effects.speed_multiplier` (UseEffects.DEFAULT: 0.2).
    pub use_speed: f32,
    /// `use_effects.can_sprint` (default false): sprinting may start while the
    /// item is in use.
    pub can_sprint: bool,
    /// `use_effects.interact_vibrations` (default true).
    pub interact_vibrations: bool,
    /// `glider` (elytra).
    pub glider: bool,
    /// `equippable.swappable` and `equip_sound`.
    pub swappable: bool,
    /// `equippable.dispensable` (dispensers may put it on an entity).
    pub equip_dispensable: bool,
    /// `equippable.damage_on_hurt`: the item wears when its wearer is hurt
    /// (false for the elytra).
    pub equip_damage_on_hurt: bool,
    pub equip_sound: Option<String>,
    /// `jukebox_playable`: the jukebox song it plays.
    pub jukebox_song: Option<String>,
    /// `crafting_remainder`: what stays in the grid when the item is used in a recipe.
    pub crafting_remainder: Option<String>,
    /// `dye`: the dye colour name (dyes, and anything else with the component).
    pub dye: Option<String>,
    /// `cushion/color`: the DyeColor name of a cushion item.
    pub cushion_color: Option<String>,
    /// `block_transformer`: the registry entry an axe, hoe or shovel uses on blocks.
    pub block_transformer: Option<String>,
    /// Registered as a `BundleItem` (`Items.BUNDLE`, `Items.DYED_BUNDLE`, or a
    /// content mod item templated on one).
    pub bundle: bool,
    /// The ids of the item's default components (its prototype's keys).
    pub components: Vec<String>,
    /// Opaque native component defaults, keyed by their registered name.
    pub custom_components: std::collections::BTreeMap<String, sunrise_core::nbt::Tag>,
    /// A content mod's item: the vanilla item it copies (its `template`), whose
    /// behaviour it has where Sunrise follows the template (`ItemInfo::is`).
    /// None for vanilla's items.
    pub template: Option<Arc<str>>,
}
/// Vanilla 26.3 `BundleItem` item ids (`Items.BUNDLE` and all 16 `Items.DYED_BUNDLE`).
pub const VANILLA_BUNDLE_ITEMS: [&str; 17] = [
    "minecraft:bundle",
    "minecraft:black_bundle",
    "minecraft:blue_bundle",
    "minecraft:brown_bundle",
    "minecraft:cyan_bundle",
    "minecraft:gray_bundle",
    "minecraft:green_bundle",
    "minecraft:light_blue_bundle",
    "minecraft:light_gray_bundle",
    "minecraft:lime_bundle",
    "minecraft:magenta_bundle",
    "minecraft:orange_bundle",
    "minecraft:pink_bundle",
    "minecraft:purple_bundle",
    "minecraft:red_bundle",
    "minecraft:white_bundle",
    "minecraft:yellow_bundle",
];

impl ItemInfo {
    /// Whether this item is the vanilla item `vanilla`, or a content mod's item
    /// copied from it: what vanilla's `instanceof` on the item's class asks
    /// (a content item copies its template's class, as a NeoForge mod's
    /// `new FishingRodItem(...)` would).
    pub fn is(&self, vanilla: &str) -> bool {
        &*self.id == vanilla || self.template.as_deref() == Some(vanilla)
    }

    /// Whether this item has vanilla's `BundleItem` class: `minecraft:bundle`, any of the 16 dyed
    /// bundles, or a content mod's item copied from one via `template` (`ItemInfo::is`).
    ///
    /// Unlike [`ItemInfo::bundle`], which checks only for a default `minecraft:bundle_contents`
    /// component, this matches vanilla 26.3's item-class dispatch for
    /// `BundleItem.overrideStackedOnOther` / `BundleItem.overrideOtherStackedOnMe`.
    pub fn is_bundle_item(&self) -> bool {
        VANILLA_BUNDLE_ITEMS.iter().any(|&v| self.is(v))
    }
}
/// Vanilla items registered as `BundleItem` (`Items.BUNDLE` and `Items.DYED_BUNDLE`).
fn is_vanilla_bundle(id: &str) -> bool {
    matches!(
        id,
        "minecraft:bundle"
            | "minecraft:white_bundle"
            | "minecraft:orange_bundle"
            | "minecraft:magenta_bundle"
            | "minecraft:light_blue_bundle"
            | "minecraft:yellow_bundle"
            | "minecraft:lime_bundle"
            | "minecraft:pink_bundle"
            | "minecraft:gray_bundle"
            | "minecraft:light_gray_bundle"
            | "minecraft:cyan_bundle"
            | "minecraft:purple_bundle"
            | "minecraft:blue_bundle"
            | "minecraft:brown_bundle"
            | "minecraft:green_bundle"
            | "minecraft:red_bundle"
            | "minecraft:black_bundle"
    )
}
/// `Item.Properties.craftRemainder` (not a data component, so not in the
/// default-component report): what an item leaves in the grid when crafted with.
pub const CRAFT_REMAINDERS: [(&str, &str); 5] = [
    ("minecraft:water_bucket", "minecraft:bucket"),
    ("minecraft:lava_bucket", "minecraft:bucket"),
    ("minecraft:milk_bucket", "minecraft:bucket"),
    ("minecraft:dragon_breath", "minecraft:glass_bottle"),
    ("minecraft:honey_bottle", "minecraft:glass_bottle"),
];
/// In a root built by `mod_content::build`: the content mods' items' crafting
/// remainders, by item. Other roots have none.
pub const MOD_CRAFT_REMAINDERS: &str = "sunrise-mod-craft-remainders.json";
/// In a root built by `mod_content::build`: the content mods' items' vanilla
/// templates, by item (`ItemInfo::template`). Other roots have none.
pub const MOD_ITEM_TEMPLATES: &str = "sunrise-mod-item-templates.json";
/// The mod items' declared arm poses (`arm_pose`): item id to the pose's name.
pub const MOD_ITEM_ARM_POSES: &str = "sunrise-mod-item-arm-poses.json";
/// Explicit item-to-block mappings from content mods (not block drops).
pub const MOD_ITEM_BLOCKS: &str = "sunrise-mod-item-blocks.json";
/// A reduction of `blocks_attacks.damage_reductions`.
#[derive(Clone, Debug)]
pub struct BlockReduction {
    pub horizontal_angle: f32,
    /// Damage type ids or `#tags`; None blocks every type.
    pub types: Option<Vec<String>>,
    pub base: f32,
    pub factor: f32,
}
/// The `blocks_attacks` component.
#[derive(Clone, Debug)]
pub struct BlocksAttacks {
    pub delay_seconds: f32,
    pub disable_scale: f32,
    pub reductions: Vec<BlockReduction>,
    /// item_damage: (threshold, base, factor)
    pub item_damage: (f32, f32, f32),
    pub bypassed_by: Option<String>,
    pub block_sound: Option<String>,
    pub disabled_sound: Option<String>,
}
impl BlocksAttacks {
    /// The `blocks_attacks` `stack` has (`ItemStack.get(DataComponents.BLOCKS_ATTACKS)`):
    /// its own component when it carries one (`/give @s diamond_sword[blocks_attacks={}]`,
    /// or a server's item), else its item's (`item`, a shield's).
    pub fn of_stack<'a>(
        stack: &ItemStack,
        item: Option<&'a ItemInfo>,
    ) -> Option<std::borrow::Cow<'a, Self>> {
        if let Some(own) = stack
            .data()
            .and_then(|d| d.other("minecraft:blocks_attacks"))
        {
            let json = sunrise_game_data::scoreboard::tag_to_json(own);
            return Self::parse(&json).map(std::borrow::Cow::Owned);
        }
        item?
            .blocks_attacks
            .as_ref()
            .map(std::borrow::Cow::Borrowed)
    }
    /// `blocks_attacks={}`: the codec's defaults, what a stack that only a mod's item
    /// ability makes block attacks (`shield_block`) blocks with.
    pub fn bare() -> &'static Self {
        static DEFAULT: std::sync::LazyLock<BlocksAttacks> = std::sync::LazyLock::new(|| {
            BlocksAttacks::parse(&Value::Object(Default::default())).expect("an object parses")
        });
        &DEFAULT
    }
    fn parse(v: &Value) -> Option<Self> {
        if !v.is_object() {
            return None;
        }
        let f = |v: &Value, d: f64| v.as_f64().unwrap_or(d) as f32;
        let text = |v: &Value| v.as_str().map(str::to_owned);
        let reductions = match v["damage_reductions"].as_array() {
            Some(list) => list
                .iter()
                .map(|r| BlockReduction {
                    horizontal_angle: f(&r["horizontal_blocking_angle"], 90.0),
                    types: match &r["type"] {
                        Value::String(s) => Some(vec![s.clone()]),
                        Value::Array(a) => Some(
                            a.iter()
                                .filter_map(|t| t.as_str().map(str::to_owned))
                                .collect(),
                        ),
                        _ => None,
                    },
                    base: f(&r["base"], 0.0),
                    factor: f(&r["factor"], 0.0),
                })
                .collect(),
            // BlocksAttacks.CODEC default: block everything within 90 degrees.
            None => vec![BlockReduction {
                horizontal_angle: 90.0,
                types: None,
                base: 0.0,
                factor: 1.0,
            }],
        };
        let d = &v["item_damage"];
        Some(Self {
            delay_seconds: f(&v["block_delay_seconds"], 0.0),
            disable_scale: f(&v["disable_cooldown_scale"], 1.0),
            reductions,
            item_damage: if d.is_object() {
                (
                    f(&d["threshold"], 1.0),
                    f(&d["base"], 0.0),
                    f(&d["factor"], 1.0),
                )
            } else {
                (1.0, 0.0, 1.0)
            },
            bypassed_by: text(&v["bypassed_by"]),
            block_sound: text(&v["block_sound"]),
            disabled_sound: text(&v["disabled_sound"]),
        })
    }
}
#[derive(Clone, Debug, PartialEq, Hash)]
pub struct ItemStack {
    pub item: Arc<str>,
    pub count: u32,
    pub damage: u32,
    /// Components beyond count and damage (see `item_data`); None when default.
    pub extra: Option<Box<crate::item_data::StackData>>,
}
/// `order`'s items in the registry's order (`command-registries.json`'s
/// `items`, exported from the game); `order` itself when that is missing.
fn registry_order(root: &Path, order: &[Arc<str>]) -> Vec<Arc<str>> {
    let listed: Vec<String> = fs::read(root.join("command-registries.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| serde_json::from_value(v["items"].clone()).ok())
        .unwrap_or_default();
    if listed.is_empty() {
        return order.to_vec();
    }
    let known: HashSet<&str> = order.iter().map(|i| &**i).collect();
    listed
        .iter()
        .filter(|i| known.contains(i.as_str()))
        .map(|i| Arc::from(i.as_str()))
        .collect()
}
/// Item ids standing for a loot table's `minecraft:dynamic` entries.
pub(crate) const DYNAMIC: &str = "#dynamic:";
/// The item id standing for a chosen `minecraft:loot_table` entry, whose
/// table then rolls in full (`NestedLootTable`).
const NESTED: &str = "#loot_table";
thread_local! {
    /// The nested tables rolling now, outermost first: a table never rolls
    /// inside itself (`LootContext.pushVisitedElement`).
    static NESTED_ROLLS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}
struct NestedRoll;
impl NestedRoll {
    fn enter(name: &str) -> Option<Self> {
        NESTED_ROLLS.with(|visited| {
            let mut visited = visited.borrow_mut();
            if visited.iter().any(|current| current == name) {
                return None;
            }
            visited.push(name.to_owned());
            Some(Self)
        })
    }
}
impl Drop for NestedRoll {
    fn drop(&mut self) {
        NESTED_ROLLS.with(|visited| {
            visited.borrow_mut().pop();
        });
    }
}

/// The block tags the mob code asks about by name, as a bit per block state:
/// built once from [`Items::block_tags`], so a mob tick or a pathfinder step
/// tests a bit instead of hashing the tag and the block name.
#[derive(Clone, Debug, Default)]
pub struct BlockTagFlags {
    /// Every block state's flags, indexed by `BlockStateId.0`.
    bits: Vec<u16>,
    /// The states' names by id, in the registry the table was built for:
    /// what a rebuild tests `block_tags` against. The map is replaced by a
    /// load, a `/reload`, a pack's classes and a server's tags.
    names: Arc<[Arc<str>]>,
}
impl BlockTagFlags {
    /// `minecraft:climbable` (`BlockTags.CLIMBABLE`).
    pub const CLIMBABLE: u16 = 1 << 0;
    /// `minecraft:fire` (`BlockTags.FIRE`).
    pub const FIRE: u16 = 1 << 1;
    /// `minecraft:cauldrons` (`BlockTags.CAULDRONS`).
    pub const CAULDRONS: u16 = 1 << 2;
    /// `minecraft:can_glide_through` (`BlockTags.CAN_GLIDE_THROUGH`).
    pub const CAN_GLIDE_THROUGH: u16 = 1 << 3;
    /// `minecraft:cats_can_lie_on`.
    pub const CATS_CAN_LIE_ON: u16 = 1 << 4;
    /// `minecraft:cats_can_sit_on`.
    pub const CATS_CAN_SIT_ON: u16 = 1 << 5;
    /// `minecraft:entities_can_teleport_to`.
    pub const ENTITIES_CAN_TELEPORT_TO: u16 = 1 << 6;
    /// `minecraft:dangerous_for_teleportation`.
    pub const DANGEROUS_FOR_TELEPORTATION: u16 = 1 << 7;
    /// Every flag with the tag it stands for: the table's whole contract.
    pub const ALL: [(u16, &'static str); 8] = [
        (Self::CLIMBABLE, "minecraft:climbable"),
        (Self::FIRE, "minecraft:fire"),
        (Self::CAULDRONS, "minecraft:cauldrons"),
        (Self::CAN_GLIDE_THROUGH, "minecraft:can_glide_through"),
        (Self::CATS_CAN_LIE_ON, "minecraft:cats_can_lie_on"),
        (Self::CATS_CAN_SIT_ON, "minecraft:cats_can_sit_on"),
        (
            Self::ENTITIES_CAN_TELEPORT_TO,
            "minecraft:entities_can_teleport_to",
        ),
        (
            Self::DANGEROUS_FOR_TELEPORTATION,
            "minecraft:dangerous_for_teleportation",
        ),
    ];

    /// `tags` as one flag set per state of `names` (in `BlockStateId` order).
    /// A member of several flags is marked with all of them, as testing each
    /// tag's set does.
    pub fn build(tags: &HashMap<String, HashSet<String>>, names: Arc<[Arc<str>]>) -> Self {
        let mut by_name: HashMap<&str, u16> = HashMap::new();
        for (flag, tag) in Self::ALL {
            if let Some(members) = tags.get(tag) {
                for member in members {
                    *by_name.entry(member.as_str()).or_default() |= flag;
                }
            }
        }
        let bits = names
            .iter()
            .map(|name| by_name.get(&**name).copied().unwrap_or(0))
            .collect();
        Self { bits, names }
    }
    /// The state's flags: 0 for an id the table has no name for.
    #[inline]
    pub fn flags(&self, id: BlockStateId) -> u16 {
        self.bits.get(id.0 as usize).copied().unwrap_or(0)
    }
    /// Whether the state carries `flag`.
    #[inline]
    pub fn has(&self, id: BlockStateId, flag: u16) -> bool {
        self.flags(id) & flag != 0
    }
    /// The flags by state id, for a view that keeps them directly.
    pub fn bits(&self) -> &[u16] {
        &self.bits
    }
    /// The states' names by id.
    pub fn state_names(&self) -> &[Arc<str>] {
        &self.names
    }
}
/// The states' names of `facts`, by `BlockStateId`: what a [`BlockTagFlags`]
/// is built against.
pub fn block_tag_state_names(facts: &BlockFacts) -> Arc<[Arc<str>]> {
    facts
        .states
        .iter()
        .map(|state| Arc::clone(&state.name))
        .collect()
}

type StewEffects = Arc<[(Arc<str>, i32)]>;

#[derive(Default, serde::Deserialize)]
struct BlockBindings {
    #[serde(default)]
    items: HashMap<String, BlockItemBinding>,
    #[serde(default)]
    blocks: Vec<BlockBinding>,
}

#[derive(serde::Deserialize)]
struct BlockItemBinding {
    #[serde(default)]
    class: Vec<String>,
    #[serde(default)]
    fields: BlockItemFields,
}

#[derive(Default, serde::Deserialize)]
struct BlockItemFields {
    block: Option<String>,
}

#[derive(serde::Deserialize)]
struct BlockBinding {
    name: String,
    suspicious_stew_effects: Option<Vec<sunrise_mod_package::content::StewEffect>>,
}

impl BlockBindings {
    fn load(root: &Path) -> Result<Self> {
        match fs::read(root.join("block-behaviours.json")) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }

    fn item_blocks(&self) -> impl Iterator<Item = (&str, &str)> {
        self.items.iter().filter_map(|(item, binding)| {
            let block = binding.fields.block.as_deref()?;
            binding
                .class
                .iter()
                .any(|name| name == "BlockItem")
                .then_some((item.as_str(), block))
        })
    }

    fn stew_effects_by_item(&self) -> HashMap<String, StewEffects> {
        let mut holders: HashMap<&str, StewEffects> = crate::flowers::FLOWER_STEW_EFFECTS
            .iter()
            .map(|&(block, id, ticks)| (block, Arc::from([(Arc::from(id), ticks)])))
            .collect();
        holders.extend(self.blocks.iter().filter_map(|block| {
            let effects = block
                .suspicious_stew_effects
                .as_ref()?
                .iter()
                .map(|effect| (Arc::from(effect.id.as_str()), effect.duration))
                .collect();
            Some((block.name.as_str(), effects))
        }));
        self.item_blocks()
            .filter_map(|(item, block)| Some((item.to_owned(), holders.get(block)?.clone())))
            .collect()
    }
}

/// An empty catalog is useful for synthetic worlds without registered items;
/// normal worlds load their catalog from the resource snapshot.
#[derive(Clone, Default)]
pub struct Items {
    pub items: HashMap<Arc<str>, ItemInfo>,
    /// Shared Block.getItem snapshot, including blocks sharing another block's item.
    pub block_items: Arc<HashMap<String, String>>,
    // The item -> its BlockItem block -> effect holder lookup, resolved once at load.
    suspicious_stew_effects: Arc<HashMap<String, StewEffects>>,
    /// Registry snapshot used to validate ingredient component payloads.
    pub component_registry: Arc<sunrise_game_data::registries::Registries>,
    /// Loaded component prototypes for concrete ingredient predicates.
    pub component_defaults: Arc<HashMap<String, sunrise_core::nbt::Compound>>,
    /// Default `minecraft:mob_visibility` by item id, with the `equippable`
    /// slot it applies in (`LivingEntity.getVisibilityPercent`).
    mob_visibility: HashMap<Arc<str>, (String, MobVisibility)>,
    pub order: Vec<Arc<str>>,
    /// The same items in item registry (network id) order, as vanilla lists
    /// them; sorted by name when the registry export is missing.
    pub registry_order: Vec<Arc<str>>,
    /// Block tag name (without '#') -> member block names.
    pub block_tags: HashMap<String, HashSet<String>>,
    /// The tags of `block_tags` the mob code asks about, as a bit per block
    /// state ([`BlockTagFlags`]): a tick or a pathfinder step tests one bit
    /// instead of hashing the tag and the block name.
    pub block_tag_flags: Arc<BlockTagFlags>,
    pub item_tags: HashMap<String, HashSet<String>>,
    /// Fluid tag name -> member fluid names (`minecraft:water` holds `minecraft:water` and
    /// `minecraft:flowing_water`).
    pub fluid_tags: HashMap<String, HashSet<String>>,
    /// Item tags in vanilla's order (`TagLoader`: file order, nested tags in
    /// place, first occurrence kept), the order `TagSlotDisplay` resolves.
    pub item_tag_order: HashMap<String, Vec<String>>,
    pub(crate) item_tag_names: Option<ItemTagNames>,
    tags_loaded: [bool; 10],
    /// Block tags in the same order.
    pub block_tag_order: HashMap<String, Vec<String>>,
    /// Fluid tags in the same order.
    pub fluid_tag_order: HashMap<String, Vec<String>>,
    /// Biome tags in `TagLoader` order (`worldgen/biome`).
    pub biome_tag_order: HashMap<String, Vec<String>>,
    /// Enchantment tags in `TagLoader` order (`enchantment`).
    pub enchantment_tag_order: HashMap<String, Vec<String>>,
    /// Structure tags in `TagLoader` order (`worldgen/structure`).
    pub structure_tag_order: HashMap<String, Vec<String>>,
    /// Potion tags in `TagLoader` order (`potion`).
    pub potion_tag_order: HashMap<String, Vec<String>>,
    /// Damage type tags in `TagLoader` order (`damage_type`).
    pub damage_type_tag_order: HashMap<String, Vec<String>>,
    /// World clock tags in `TagLoader` order (`world_clock`).
    pub world_clock_tag_order: HashMap<String, Vec<String>>,
    pub entity_type_tags: Arc<EntityTypeTags>,
    /// The bound `game_event` tags (`GameEventTags`), which the vibration listeners read.
    pub game_event_tags: Arc<HashMap<String, HashSet<String>>>,
    loot: Arc<sunrise_world::loot::LootTables>,
    loot_bindings: sunrise_world::loot::types::Bindings,
    loot_modifiers: Arc<HashMap<String, Value>>,
    global_modifiers: Arc<sunrise_world::loot::global_modifiers::GlobalLootModifiers>,
    /// `Registries.PREDICATE`: what a table's string condition names, which a
    /// pack replaces or adds to (`LootItemCondition.CODEC`).
    predicates: Arc<HashMap<String, Value>>,
    /// The `jukebox_song` registry.
    pub songs: HashMap<String, Song>,
    /// The `instrument` registry (goat horns).
    pub instruments: HashMap<String, Instrument>,
    /// The enchantment registry: loot functions enchant the items they roll
    /// (`EnchantWithLevelsFunction`, `EnchantRandomlyFunction`), and the
    /// station menus share this copy.
    pub enchantments: Arc<crate::enchant::Enchantments>,
}
impl Items {
    /// This table with a server's block and item tags in place of its own
    /// (`ClientboundUpdateTagsPacket`): each tag's members in the order sent,
    /// which is `TagLoader`'s. `None` keeps this table's tags of that kind.
    pub fn with_tags(
        &self,
        block_tags: Option<HashMap<String, Vec<String>>>,
        item_tags: Option<HashMap<String, Vec<String>>>,
    ) -> Self {
        let mut items = self.clone();
        if let Some(tags) = block_tags {
            items.set_tags(TagRegistry::Block, tags);
        }
        if let Some(tags) = item_tags {
            items.set_tags(TagRegistry::Item, tags);
        }
        items
    }

    /// Replace the generation seen by both entity queries and enchantment effects.
    pub fn set_entity_type_tags(&mut self, tags: EntityTypeTags) {
        let ids = Arc::make_mut(&mut self.component_registry)
            .by_registry
            .entry(EntityTypeTags::REGISTRY.into())
            .or_default();
        ids.tags = tags.ordered().keys().cloned().collect();
        ids.tags.sort();
        Arc::make_mut(&mut self.enchantments).entity_tags = tags.members().clone();
        self.entity_type_tags = Arc::new(tags);
        self.tags_loaded[TagRegistry::EntityType as usize] = true;
    }

    /// The states' names [`Self::block_tag_flags`] was built against.
    pub fn block_state_names(&self) -> Arc<[Arc<str>]> {
        Arc::clone(&self.block_tag_flags.names)
    }
    /// Rebuild [`Self::block_tag_flags`] from `block_tags` against the states'
    /// `names`: every path that replaces the map (a load, a `/reload`, a
    /// pack's class tags, a server's tags) rebuilds the table with it.
    pub fn rebuild_block_tag_flags(&mut self, names: Arc<[Arc<str>]>) {
        self.block_tag_flags = Arc::new(BlockTagFlags::build(&self.block_tags, names));
    }
}
/// A `JukeboxSong`.
#[derive(Clone, Debug)]
pub struct Song {
    pub sound: String,
    pub length_ticks: i64,
    pub comparator: i32,
    pub description: Value,
}
impl Song {
    /// `JukeboxSong.DIRECT_CODEC`: every field is required: a `sound_event`
    /// (a sound id or an inline definition), a `description` (a text
    /// component), a positive `length_in_seconds` and a `comparator_output`
    /// from 0 to 15.
    pub fn decode(v: &Value) -> std::result::Result<Self, String> {
        let event = &v["sound_event"];
        let sound = match event {
            Value::String(id) => id.as_str(),
            Value::Object(_) => event["sound_id"]
                .as_str()
                .ok_or("sound_event needs a sound_id")?,
            _ => return Err("sound_event must be a sound id or definition".into()),
        };
        let sound = sunrise_core::identifier::Identifier::parse(sound)
            .map_err(|error| format!("sound_event: {error}"))?
            .to_string();
        let description = v["description"].clone();
        if description.is_null() {
            return Err("description is required".into());
        }
        let length = v["length_in_seconds"]
            .as_f64()
            .map(|n| n as f32)
            .filter(|n| *n > 0.0 && *n <= f32::MAX)
            .ok_or("length_in_seconds must be positive and finite")?;
        // `Codec.INT` reads `Number.intValue`, which truncates.
        let comparator = v["comparator_output"]
            .as_f64()
            .map(|n| n as i32)
            .filter(|n| (0..=15).contains(n))
            .ok_or("comparator_output must be an integer from 0 to 15")?;
        Ok(Self {
            sound,
            length_ticks: i64::from((length * 20.0).ceil() as i32),
            comparator,
            description,
        })
    }
    /// `hasFinished`: the length plus one second.
    pub fn finished(&self, ticks: i64) -> bool {
        ticks >= self.length_ticks + 20
    }
}
/// An `Instrument`: what a goat horn plays.
#[derive(Clone, Debug)]
pub struct Instrument {
    pub sound: String,
    /// `None` names a registry holder; `Some` keeps an inline sound's optional fixed range.
    pub inline_sound_range: Option<Option<f32>>,
    /// Seconds the horn stays up, and its cooldown.
    pub use_duration: f32,
    /// How far it carries, in blocks (`range / 16` is its volume).
    pub range: f32,
    pub durability_damage: i32,
}
impl Instrument {
    /// `InstrumentItem.getUseDuration`: `Mth.floor(useDuration * 20)`.
    pub fn use_ticks(&self) -> i32 {
        instrument_use_ticks(self.use_duration)
    }
}
/// `InstrumentItem.getUseDuration`: floor after the float multiplication.
pub fn instrument_use_ticks(seconds: f32) -> i32 {
    (seconds * 20.0).floor() as i32
}
/// The id of a goat horn's instrument (`InstrumentItem.getInstrument`): the
/// stack's own `instrument` component, else the item's `ponder_goat_horn`.
/// None for an inline instrument (see `inline_instrument`).
pub fn instrument_id(stack: &ItemStack) -> Option<String> {
    if &*stack.item != "minecraft:goat_horn" {
        return None;
    }
    match stack.data().and_then(|d| d.other("minecraft:instrument")) {
        Some(sunrise_core::nbt::Tag::String(id)) => Some(id.to_string_lossy()),
        Some(sunrise_core::nbt::Tag::Compound(_)) => None,
        _ => Some("minecraft:ponder_goat_horn".to_owned()),
    }
}
/// A goat horn's inline `instrument` component (`Instrument.CODEC` also
/// takes the instrument itself instead of a registry id).
pub fn inline_instrument(stack: &ItemStack) -> Option<&sunrise_core::nbt::Tag> {
    if &*stack.item != "minecraft:goat_horn" {
        return None;
    }
    match stack.data().and_then(|d| d.other("minecraft:instrument")) {
        Some(tag @ sunrise_core::nbt::Tag::Compound(_)) => Some(tag),
        _ => None,
    }
}
impl Instrument {
    /// `Instrument.DIRECT_CODEC`: a sound holder or inline sound, duration,
    /// audible range, and optional durability cost used by instrument gameplay.
    pub fn decode(v: &Value) -> std::result::Result<Self, String> {
        let event = &v["sound_event"];
        let (sound, inline_sound_range) = if let Some(id) = event.as_str() {
            (id, None)
        } else {
            let id = event["sound_id"]
                .as_str()
                .ok_or("sound_event must be a sound id or definition")?;
            let fixed = match &event["range"] {
                Value::Null => None,
                range => Some(range.as_f64().ok_or("sound_event.range must be a number")? as f32),
            };
            (id, Some(fixed))
        };
        let sound = sunrise_core::identifier::Identifier::parse(sound)
            .map_err(|error| format!("sound_event: {error}"))?
            .to_string();
        let use_duration = v["use_duration"]
            .as_f64()
            .map(|n| n as f32)
            .filter(|n| (0.0..=f32::MAX).contains(n))
            .ok_or("use_duration must be non-negative and finite")?;
        let range = v["range"]
            .as_f64()
            .map(|n| n as f32)
            .filter(|n| *n > 0.0 && *n <= f32::MAX)
            .ok_or("range must be positive and finite")?;
        let durability_damage = match &v["durability_damage"] {
            Value::Null => 0,
            n => n
                .as_u64()
                .and_then(|n| i32::try_from(n).ok())
                .ok_or("durability_damage must be a non-negative integer")?,
        };
        Ok(Self {
            sound,
            inline_sound_range,
            use_duration,
            range,
            durability_damage,
        })
    }
}
#[path = "condition_files.rs"]
mod condition_files;
pub(crate) use condition_files::data_files;
fn load_instruments(root: &Path, overlay: &PackOverlay) -> HashMap<String, Instrument> {
    data_files(root, "instrument", overlay)
        .into_iter()
        .filter_map(|(id, v)| {
            Instrument::decode(&v)
                .ok()
                .map(|definition| (id, definition))
        })
        .collect()
}
/// The jukebox songs of the root's own files: the built-ins a client holds itself. A
/// client of a server resolves a known-pack entry the server sent without data against
/// them (`RegistryDataLoader` on the client's known packs); the world's table is installed
/// from the staged registry definitions. One definition that fails `DIRECT_CODEC` fails the
/// whole load, and every failure is reported.
fn load_songs(root: &Path) -> std::result::Result<HashMap<String, Song>, String> {
    let mut songs = HashMap::new();
    let mut errors = Vec::new();
    for (id, v) in data_files(root, "jukebox_song", &PackOverlay::NONE) {
        match Song::decode(&v) {
            Ok(song) => {
                songs.insert(id, song);
            }
            Err(error) => errors.push(format!("jukebox_song {id}: {error}")),
        }
    }
    if errors.is_empty() {
        Ok(songs)
    } else {
        errors.sort();
        Err(errors.join("; "))
    }
}
pub fn load_tags(
    root: &Path,
    kind: &str,
    overlay: &PackOverlay,
) -> Result<HashMap<String, HashSet<String>>> {
    let registries = tag_layers::builtin_ids(root)?;
    let ids = registries.get(kind);
    Ok(tag_layers::load(root, kind, overlay, |id| {
        ids.is_none_or(|ids| ids.contains(id))
    })?
    .into_iter()
    .map(|(k, v)| (k, v.into_iter().collect()))
    .collect())
}
/// The ordered fluid tags used by recipe ingredients.
pub fn fluid_tag_list(root: &Path, overlay: &PackOverlay) -> Result<HashMap<String, Vec<String>>> {
    let registries = tag_layers::builtin_ids(root)?;
    let ids = registries.get("fluid");
    Ok(tag_layers::load(root, "fluid", overlay, |id| {
        ids.map_or_else(
            || sunrise_world::fluid::FluidKind::from_name(id).is_some(),
            |ids| ids.contains(id),
        )
    })?)
}
/// The registries whose tags a mod can list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagRegistry {
    Block,
    Item,
    Fluid,
    EntityType,
    Biome,
    Enchantment,
    Structure,
    Potion,
    DamageType,
    WorldClock,
}
/// An entity type id or `#tag` as the set of entity type ids it names.
fn entity_spec(tags: &HashMap<String, HashSet<String>>, spec: &str) -> HashSet<String> {
    match spec.strip_prefix('#') {
        Some(tag) => tags.get(tag).cloned().unwrap_or_default(),
        None => HashSet::from([spec.to_owned()]),
    }
}
/// `MobVisibility`: entity types a worn item hides its wearer from, and the
/// factor `LivingEntity.getVisibilityPercent` multiplies by.
#[derive(Clone, Debug)]
pub struct MobVisibility {
    pub targeting: HashSet<String>,
    pub visibility: f32,
}
/// `minecraft:mob_visibility` (`targeting_entity_types` is one id, a `#tag`,
/// or a list) when the object is present and names at least one type.
fn parse_mob_visibility(
    v: &Value,
    tags: &HashMap<String, HashSet<String>>,
) -> Option<MobVisibility> {
    if !v.is_object() {
        return None;
    }
    let visibility = v.get("visibility")?.as_f64()? as f32;
    let targeting = match v.get("targeting_entity_types")? {
        Value::String(spec) => entity_spec(tags, spec),
        Value::Array(list) => list
            .iter()
            .filter_map(Value::as_str)
            .flat_map(|spec| entity_spec(tags, spec))
            .collect(),
        _ => return None,
    };
    if targeting.is_empty() {
        return None;
    }
    Some(MobVisibility {
        targeting,
        visibility,
    })
}
/// The item's `equippable.slot` and `mob_visibility`, when both are set.
/// `getVisibilityPercent` multiplies only when the stack is in that slot.
fn worn_mob_visibility(
    components: &Value,
    tags: &HashMap<String, HashSet<String>>,
) -> Option<(String, MobVisibility)> {
    let slot = components
        .get("minecraft:equippable")?
        .get("slot")?
        .as_str()?
        .to_owned();
    Some((
        slot,
        parse_mob_visibility(components.get("minecraft:mob_visibility")?, tags)?,
    ))
}

impl Items {
    /// `ItemStack.has(DataComponents.TOOL)`: the stack's own patch first (a `tool` it sets, or
    /// one it removes), then the item's default components (`AbstractPiglin.isHoldingMeleeWeapon`
    /// asks it of the main hand).
    pub fn has_tool(&self, stack: &ItemStack) -> bool {
        if let Some(data) = stack.data() {
            if data.other("!minecraft:tool").is_some() {
                return false;
            }
            if data.other("minecraft:tool").is_some() {
                return true;
            }
        }
        self.get(&stack.item)
            .is_some_and(|info| info.tool_damage_per_block.is_some())
    }
    /// Load item data; bind the world's damage catalog before preparing recipes.
    pub fn load(
        root: &Path,
        registry: &BlockRegistry,
        facts: &BlockFacts,
        overlay: &PackOverlay,
        defaults: &ItemDefaults,
    ) -> Result<Self> {
        let mut items = Self::load_registry(
            root,
            registry,
            facts,
            overlay,
            defaults,
            LoadEntityTags::Prepared,
        )?;
        items.songs = load_songs(root)?;
        items.load_resource_registries(root, overlay, defaults)?;
        items.load_table_loot(root, overlay, defaults)?;
        Ok(items)
    }

    /// Load the item registry and tags before admitting dependent registries.
    /// The owning load must then install songs, instruments, enchantments and loot.
    /// When the game already owns mob types, use their frozen registry order.
    pub fn load_registry(
        root: &Path,
        registry: &BlockRegistry,
        facts: &BlockFacts,
        overlay: &PackOverlay,
        defaults: &ItemDefaults,
        entity_tags: LoadEntityTags<'_>,
    ) -> Result<Self> {
        let mut block_for_item: HashMap<&str, BlockStateId> = HashMap::new();
        for (i, s) in facts.states.iter().enumerate() {
            let default = registry.default_state(&s.name);
            if default == Some(BlockStateId(i as u32)) && &*s.item != "minecraft:air" {
                block_for_item
                    .entry(&s.item)
                    .or_insert(BlockStateId(i as u32));
            }
        }
        // Block.getItem is not an inverse of BlockItem.getBlock: several items
        // may place the same block. Read their normal compiled bindings too.
        let bindings = BlockBindings::load(root)?;
        for (item, block) in bindings.item_blocks() {
            if let Some(state) = registry.default_state(block) {
                block_for_item.entry(item).or_insert(state);
            }
        }
        let mut items = Self::load_with(
            root,
            &block_for_item,
            &bindings,
            overlay,
            defaults,
            entity_tags,
            |id| registry.default_state(id).is_some(),
        )?;
        items.block_items = Arc::new(
            facts
                .states
                .iter()
                .map(|state| (state.name.to_string(), state.item.to_string()))
                .collect(),
        );
        let path = root.join(MOD_ITEM_BLOCKS);
        match fs::read(&path) {
            Ok(bytes) => {
                let mappings: HashMap<String, String> = serde_json::from_slice(&bytes)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                for (item, block) in mappings {
                    let state = registry
                        .default_state(&block)
                        .ok_or_else(|| format!("{item}: block {block:?} is not a block"))?;
                    let info = items
                        .items
                        .get_mut(item.as_str())
                        .ok_or_else(|| format!("{item}: not an item"))?;
                    info.block = Some(state);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", path.display()).into()),
        }
        // The mob code's tags as a bit per state, against this registry.
        items.rebuild_block_tag_flags(block_tag_state_names(facts));
        Ok(items)
    }
    /// `SuspiciousEffectHolder.tryGet`: the effects supplied by the block this item places.
    pub fn suspicious_stew_effects(&self, item: &str) -> Option<&[(Arc<str>, i32)]> {
        self.suspicious_stew_effects.get(item).map(AsRef::as_ref)
    }

    /// Every item's defaults, tags and block-to-item identities, without
    /// placement state ids (for headless tools that never place blocks).
    pub fn catalog(root: &Path, overlay: &PackOverlay, defaults: &ItemDefaults) -> Result<Self> {
        let mut items = Self::catalog_items(root, overlay, defaults, |items| {
            items.load_resource_registries(root, overlay, defaults)
        })?;
        items.load_table_loot(root, overlay, defaults)?;
        ingredient_defaults::load_component_damage_registry(root, overlay, &mut items)?;
        Ok(items)
    }
    /// Build the item catalog for recipe sync against the world's loaded damage
    /// registry. The world loads loot separately; loading it here would reopen
    /// damage definitions instead of respecting the frozen registry on reload.
    pub fn catalog_with_damage_registry(
        root: &Path,
        overlay: &PackOverlay,
        defaults: &ItemDefaults,
        damage: sunrise_game_data::registries::RegistryIds,
        load_registries: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<Self> {
        let mut items = Self::catalog_items(root, overlay, defaults, load_registries)?;
        items.component_defaults = ingredient_defaults::component_defaults(defaults)?;
        Arc::make_mut(&mut items.component_registry)
            .by_registry
            .insert("minecraft:damage_type".into(), damage);
        Ok(items)
    }
    fn catalog_items(
        root: &Path,
        overlay: &PackOverlay,
        defaults: &ItemDefaults,
        load_registries: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<Self> {
        let block_items = ingredient_defaults::block_items(root)?;
        let bindings = BlockBindings::load(root)?;
        let mut items = Self::load_with(
            root,
            &HashMap::new(),
            &bindings,
            overlay,
            defaults,
            LoadEntityTags::Prepared,
            |id| block_items.contains_key(id),
        )?;
        items.songs = load_songs(root)?;
        load_registries(&mut items)?;
        items.block_items = Arc::new(block_items);
        Ok(items)
    }

    fn load_with(
        root: &Path,
        block_for_item: &HashMap<&str, BlockStateId>,
        bindings: &BlockBindings,
        overlay: &PackOverlay,
        defaults: &ItemDefaults,
        entity_tags: LoadEntityTags<'_>,
        contains_block: impl Fn(&str) -> bool,
    ) -> Result<Self> {
        let (component_registry, entity_type_tags) =
            ingredient_defaults::component_tables(root, overlay, entity_tags)?;
        let entity_tags = entity_type_tags.members();
        let by_item = |name: &str| -> Result<HashMap<String, String>> {
            let path = root.join(name);
            match fs::read(&path) {
                Ok(bytes) => Ok(serde_json::from_slice(&bytes)
                    .map_err(|e| format!("{}: {e}", path.display()))?),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
                Err(e) => Err(format!("{}: {e}", path.display()).into()),
            }
        };
        let mod_remainders = by_item(MOD_CRAFT_REMAINDERS)?;
        let mod_templates = by_item(MOD_ITEM_TEMPLATES)?;
        let path = root.join(MOD_ITEM_ARM_POSES);
        let mod_arm_poses: HashMap<String, sunrise_core::arm_pose::ArmPose> =
            match fs::read(&path) {
                Ok(bytes) => serde_json::from_slice(&bytes)
                    .map_err(|e| format!("{}: {e}", path.display()))?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
                Err(e) => return Err(format!("{}: {e}", path.display()).into()),
            };
        let mut items = HashMap::new();
        let mut order = Vec::new();
        let mut mob_visibility = HashMap::new();
        for (name, c) in defaults.entries() {
            let id: Arc<str> = Arc::from(name.as_str());
            let tool = c["minecraft:tool"]["rules"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|r| ToolRule {
                    blocks: match &r["blocks"] {
                        Value::String(s) => vec![s.clone()],
                        Value::Array(a) => a
                            .iter()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect(),
                        _ => vec![],
                    },
                    speed: r["speed"].as_f64().map(|v| v as f32),
                    correct: r["correct_for_drops"].as_bool(),
                })
                .collect();
            let modifier = |kind: &str| -> f32 {
                c["minecraft:attribute_modifiers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|m| {
                        m["type"].as_str() == Some(kind)
                            && m["operation"].as_str() == Some("add_value")
                    })
                    .filter_map(|m| m["amount"].as_f64())
                    .sum::<f64>() as f32
            };
            let food = c.get("minecraft:food").map(|f| {
                (
                    f["nutrition"].as_u64().unwrap_or(0) as u32,
                    f["saturation"].as_f64().unwrap_or(0.0) as f32,
                )
            });
            let can_always_eat = c
                .get("minecraft:food")
                .and_then(|f| f.get("can_always_eat"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let prototype = sunrise_core::json_nbt::json_to_tag(c)?;
            let (damage, extra) = crate::item_data::decode_components(Some(&prototype));
            let component_prototype = ItemStack {
                item: id.clone(),
                count: 1,
                damage,
                extra,
            };
            let villager_food = component_prototype.villager_food(None);
            let consumable = component_prototype.consumable(None).cloned();
            let death_protection = component_prototype
                .death_protection(None)
                .map(<[_]>::to_vec);
            if (c.get("minecraft:consumable").is_some() && consumable.is_none())
                || (c.get("minecraft:death_protection").is_some() && death_protection.is_none())
            {
                return Err(format!("{name}: invalid consume effect component").into());
            }
            let template: Option<Arc<str>> = mod_templates.get(name).map(|t| Arc::from(t.as_str()));
            let bundle =
                is_vanilla_bundle(name) || template.as_deref().is_some_and(is_vanilla_bundle);
            items.insert(
                id.clone(),
                ItemInfo {
                    id: id.clone(),
                    component_prototype,
                    max_stack: c["minecraft:max_stack_size"].as_u64().unwrap_or(64) as u32,
                    max_damage: c["minecraft:max_damage"].as_u64().unwrap_or(0) as u32,
                    damage: c["minecraft:damage"].as_i64().unwrap_or(0).max(0) as i32,
                    block: block_for_item.get(name.as_str()).copied(),
                    tool,
                    tool_default_speed: c["minecraft:tool"]["default_mining_speed"]
                        .as_f64()
                        .unwrap_or(1.0) as f32,
                    tool_damage_per_block: c
                        .get("minecraft:tool")
                        .map(|t| t["damage_per_block"].as_u64().unwrap_or(1) as u32),
                    can_destroy_blocks_in_creative:
                        c["minecraft:tool"]["can_destroy_blocks_in_creative"]
                            .as_bool()
                            .unwrap_or(true),
                    food,
                    can_always_eat,
                    kinetic_timing: c
                        .get("minecraft:kinetic_weapon")
                        .and_then(crate::kinetic_weapon::UseTiming::from_json),
                    death_protection,
                    consumable,
                    use_cooldown: c
                        .get("minecraft:use_cooldown")
                        .and_then(crate::cooldowns::UseCooldown::from_json),
                    model: c["minecraft:item_model"]
                        .as_str()
                        .unwrap_or(name)
                        .to_owned(),
                    glider: c["minecraft:glider"].is_object(),
                    swappable: c["minecraft:equippable"]["swappable"]
                        .as_bool()
                        .unwrap_or(true),
                    equip_dispensable: c["minecraft:equippable"]["dispensable"]
                        .as_bool()
                        .unwrap_or(true),
                    equip_damage_on_hurt: c["minecraft:equippable"]["damage_on_hurt"]
                        .as_bool()
                        .unwrap_or(true),
                    equip_sound: c["minecraft:equippable"]["equip_sound"]
                        .as_str()
                        .map(str::to_owned),
                    jukebox_song: c["minecraft:jukebox_playable"].as_str().map(str::to_owned),
                    dye: c["minecraft:dye"].as_str().map(str::to_owned),
                    cushion_color: c["minecraft:cushion/color"].as_str().map(str::to_owned),
                    block_transformer: c["minecraft:block_transformer"].as_str().map(str::to_owned),
                    bundle,
                    components: c
                        .as_object()
                        .map(|c| c.keys().cloned().collect())
                        .unwrap_or_default(),
                    custom_components: c
                        .as_object()
                        .into_iter()
                        .flatten()
                        .filter(|(name, _)| !name.starts_with("minecraft:"))
                        .map(|(name, value)| {
                            (
                                name.clone(),
                                sunrise_game_data::scoreboard::json_to_tag(value),
                            )
                        })
                        .collect(),
                    crafting_remainder: CRAFT_REMAINDERS
                        .iter()
                        .find(|(item, _)| *item == name)
                        .map(|(_, rest)| (*rest).to_owned())
                        .or_else(|| mod_remainders.get(name).cloned()),
                    template,
                    villager_food,
                    use_speed: c["minecraft:use_effects"]["speed_multiplier"]
                        .as_f64()
                        .unwrap_or(0.2) as f32,
                    can_sprint: c["minecraft:use_effects"]["can_sprint"]
                        .as_bool()
                        .unwrap_or(false),
                    interact_vibrations: c["minecraft:use_effects"]["interact_vibrations"]
                        .as_bool()
                        .unwrap_or(true),
                    blocks_attacks: BlocksAttacks::parse(&c["minecraft:blocks_attacks"]),
                    arm_pose: mod_arm_poses.get(name).copied(),
                    disable_blocking_seconds: c["minecraft:weapon"]["disable_blocking_for_seconds"]
                        .as_f64()
                        .unwrap_or(0.0) as f32,
                    weapon_damage: c["minecraft:weapon"].is_object().then(|| {
                        c["minecraft:weapon"]["item_damage_per_attack"]
                            .as_u64()
                            .unwrap_or(1) as u32
                    }),
                    equip_asset: c["minecraft:equippable"]["asset_id"]
                        .as_str()
                        .map(str::to_owned),
                    equip_camera_overlay: c["minecraft:equippable"]["camera_overlay"]
                        .as_str()
                        .map(str::to_owned),
                    equip_slot: c["minecraft:equippable"]["slot"]
                        .as_str()
                        .map(str::to_owned),
                    modifiers: c["minecraft:attribute_modifiers"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|m| {
                            Some(crate::item_data::ModifierRow::new(
                                m["type"].as_str()?,
                                m["id"].as_str()?,
                                m["amount"].as_f64()?,
                                m["operation"].as_str().unwrap_or("add_value"),
                                m["slot"].as_str().unwrap_or("any"),
                            ))
                        })
                        .collect(),
                    fuel: c
                        .get("minecraft:cooking_fuel")
                        .map(|f| (f["burn_time"].clone(), f["speed_multiplier"].clone())),
                    brewing_fuel: c
                        .get("minecraft:brewing_fuel")
                        .map(|f| (f["uses"].clone(), f["speed_multiplier"].clone())),
                    compostable: c.get("minecraft:compostable").map(|f| f["layers"].clone()),
                    remainder: c["minecraft:use_remainder"]["id"]
                        .as_str()
                        .or(c["minecraft:crafting_remainder"]["id"].as_str())
                        .or(c["minecraft:crafting_remainder"].as_str())
                        .map(str::to_owned),
                    attack_damage: modifier("minecraft:attack_damage"),
                    attack_speed: modifier("minecraft:attack_speed"),
                },
            );
            if let Some(parsed) = worn_mob_visibility(c, entity_tags) {
                mob_visibility.insert(id.clone(), parsed);
            }
            order.push(id);
        }
        let registry_order = registry_order(root, &order);
        let mut table = Self {
            items,
            block_items: Default::default(),
            suspicious_stew_effects: Arc::new(bindings.stew_effects_by_item()),
            component_registry,
            entity_type_tags,
            mob_visibility,
            order,
            registry_order,
            ..Self::default()
        };
        table.bind_tags(root, overlay, defaults, contains_block)?;
        Ok(table)
    }

    fn bind_tags(
        &mut self,
        root: &Path,
        overlay: &PackOverlay,
        defaults: &ItemDefaults,
        contains_block: impl Fn(&str) -> bool,
    ) -> Result<()> {
        self.set_tags(
            TagRegistry::Block,
            tag_layers::load(root, "block", overlay, contains_block)?,
        );
        self.set_tags(
            TagRegistry::Item,
            tag_layers::load(root, "item", overlay, |id| {
                defaults.entries().contains_key(id)
            })?,
        );
        self.set_tags(TagRegistry::Fluid, fluid_tag_list(root, overlay)?);
        self.game_event_tags = Arc::new(load_tags(root, "game_event", overlay)?);
        self.tags_loaded[TagRegistry::EntityType as usize] = true;
        let builtin = tag_layers::builtin_ids(root)?;
        for (registry, kind, reg_key) in [
            (TagRegistry::Potion, "potion", "minecraft:potion"),
            (
                TagRegistry::Structure,
                "worldgen/structure",
                "minecraft:worldgen/structure",
            ),
            (
                TagRegistry::WorldClock,
                "world_clock",
                "minecraft:world_clock",
            ),
            (
                TagRegistry::Biome,
                "worldgen/biome",
                "minecraft:worldgen/biome",
            ),
        ] {
            let exported = self.component_registry.ids(reg_key);
            let fallback = builtin.get(kind);
            let tags = tag_layers::load(root, kind, overlay, |id| {
                if !exported.is_empty() {
                    exported
                        .binary_search_by(|known| known.as_str().cmp(id))
                        .is_ok()
                } else if let Some(ids) = fallback {
                    ids.contains(id)
                } else {
                    false
                }
            })?;
            self.set_tags(registry, tags);
        }
        // Enchantment tags bind once the enchantments are installed (`bind_enchantment_tags`,
        // which every load that installs them runs), never against the export here: it lists
        // vanilla's alone, so a tag naming a mod's enchantment failed to bind (#6129).
        Ok(())
    }

    /// Rebind one registry's ordered tags against an authoritative world-owned predicate.
    pub fn bind_registry_tags(
        &mut self,
        root: &Path,
        overlay: &PackOverlay,
        registry: TagRegistry,
        kind: &str,
        contains: impl Fn(&str) -> bool,
    ) -> Result<()> {
        self.set_tags(registry, tag_layers::load(root, kind, overlay, contains)?);
        Ok(())
    }

    /// Bind `enchantment` tags against the currently installed enchantment definitions.
    pub fn bind_enchantment_tags(&mut self, root: &Path, overlay: &PackOverlay) -> Result<()> {
        let known: HashSet<&str> = self
            .enchantments
            .list
            .iter()
            .map(|entry| entry.id.as_ref())
            .collect();
        let tags = tag_layers::load(root, "enchantment", overlay, |id| known.contains(id))?;
        self.set_tags(TagRegistry::Enchantment, tags);
        Ok(())
    }

    /// Load from resource files when there is no staged world registry snapshot.
    pub fn load_resource_registries(
        &mut self,
        root: &Path,
        overlay: &PackOverlay,
        defaults: &ItemDefaults,
    ) -> Result<()> {
        self.install_registries(
            crate::enchant::Enchantments::load_definitions(root, overlay, defaults)?,
            load_instruments(root, overlay),
        );
        self.bind_enchantment_tags(root, overlay)?;
        Ok(())
    }

    /// Install the dependent registries and the component decoder's matching ids.
    pub fn install_registries(
        &mut self,
        mut enchantments: crate::enchant::Enchantments,
        instruments: HashMap<String, Instrument>,
    ) {
        enchantments.entity_tags = self.entity_type_tags.members().clone();
        Arc::make_mut(&mut self.component_registry)
            .set_enchantment_ids(enchantments.list.iter().map(|entry| entry.id.to_string()));
        self.enchantments = Arc::new(enchantments);
        self.instruments = instruments;
    }

    fn load_table_loot(
        &mut self,
        root: &Path,
        overlay: &PackOverlay,
        defaults: &ItemDefaults,
    ) -> Result<()> {
        let data = sunrise_world::loot::LootData::load_table_data(root, overlay, defaults)?;
        self.component_defaults = data.component_defaults().clone();
        self.set_loot_data(&data);
        Ok(())
    }

    /// Bind tables, predicates, modifiers and prepared instances from the same candidate.
    pub fn set_loot_data(&mut self, data: &sunrise_world::loot::LootData) {
        self.loot = data.tables().clone();
        self.loot_bindings = data.bindings().clone();
        self.loot_modifiers = Arc::clone(data.modifiers());
        self.predicates = data.predicates().clone();
        self.global_modifiers = data.global_modifiers().clone();
    }
    /// Use the server's loaded loot-table snapshot for item and block drops.
    pub fn set_loot_tables(&mut self, tables: Arc<sunrise_world::loot::LootTables>) {
        self.loot = tables;
    }

    pub fn get(&self, id: &str) -> Option<&ItemInfo> {
        self.items.get(id)
    }
    /// `item`'s `villager_food` nutrition (`DataComponents.VILLAGER_FOOD`) in
    /// this registry: 0 for an item without the component, `None` for an item
    /// the registry lacks.
    pub fn villager_food(&self, item: &str) -> Option<i32> {
        self.get(item).map(|i| i.villager_food.unwrap_or(0))
    }
    /// `ItemStack.get(DataComponents.MOB_VISIBILITY).visibility()` when the
    /// prototype's equippable slot is `slot` and `targeting` is in
    /// `targetingEntityTypes`. None otherwise (`getVisibilityPercent` skips it).
    pub fn worn_visibility(&self, item: &str, slot: &str, targeting: &str) -> Option<f32> {
        let (equip, vis) = self.mob_visibility.get(item)?;
        if equip == slot && vis.targeting.contains(targeting) {
            Some(vis.visibility)
        } else {
            None
        }
    }
    /// The instrument a goat horn plays: its `instrument` component's
    /// registry entry or inline instrument.
    pub fn instrument(&self, stack: &ItemStack) -> Option<Instrument> {
        if let Some(tag) = inline_instrument(stack) {
            return Instrument::decode(&sunrise_game_data::scoreboard::tag_to_json(tag)).ok();
        }
        self.instruments.get(&instrument_id(stack)?).cloned()
    }
    /// A table with nothing in it: the tags a client was sent go in with `set_tags`.
    pub fn empty() -> Self {
        Self::default()
    }
    /// An item table with only tags, for unit tests of item rules.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_tests(item_tags: HashMap<String, HashSet<String>>) -> Self {
        let mut table = Self::empty();
        table.set_tags(
            TagRegistry::Item,
            item_tags
                .into_iter()
                .map(|(tag, values)| (tag, values.into_iter().collect()))
                .collect(),
        );
        table
    }
    /// `TagLoader.loadPendingTags`: no successfully built tags means no update
    /// for that registry. A built tag with no members still replaces the map.
    pub fn retain_unbound_tags(&mut self, previous: &Self) {
        use sunrise_world::level::classes::is_generated_tag;
        if self.block_tag_order.is_empty() {
            self.set_tags(
                TagRegistry::Block,
                previous
                    .block_tag_order
                    .iter()
                    // These are installed from the current block facts, not the pack stack.
                    .filter(|(tag, _)| !is_generated_tag(tag))
                    .map(|(tag, members)| (tag.clone(), members.clone()))
                    .collect(),
            );
        }
        if self.item_tag_order.is_empty() {
            self.set_tags(TagRegistry::Item, previous.item_tag_order.clone());
        }
        if self.fluid_tag_order.is_empty() {
            self.set_tags(TagRegistry::Fluid, previous.fluid_tag_order.clone());
        }
        for (registry, current, prev) in [
            (
                TagRegistry::Biome,
                self.biome_tag_order.is_empty(),
                &previous.biome_tag_order,
            ),
            (
                TagRegistry::Enchantment,
                self.enchantment_tag_order.is_empty(),
                &previous.enchantment_tag_order,
            ),
            (
                TagRegistry::Structure,
                self.structure_tag_order.is_empty(),
                &previous.structure_tag_order,
            ),
            (
                TagRegistry::Potion,
                self.potion_tag_order.is_empty(),
                &previous.potion_tag_order,
            ),
            (
                TagRegistry::DamageType,
                self.damage_type_tag_order.is_empty(),
                &previous.damage_type_tag_order,
            ),
            (
                TagRegistry::WorldClock,
                self.world_clock_tag_order.is_empty(),
                &previous.world_clock_tag_order,
            ),
        ] {
            if current && previous.tags_loaded(registry) {
                self.set_tags(registry, prev.clone());
            }
        }
    }

    /// Ordered bindings for data loaders, keyed by the vanilla registry directory.
    pub fn ordered_tags(&self) -> [(&'static str, &sunrise_world::loot::RegistryTags); 4] {
        [
            ("block", &self.block_tag_order),
            ("item", &self.item_tag_order),
            ("fluid", &self.fluid_tag_order),
            ("entity_type", self.entity_type_tags.ordered()),
        ]
    }

    /// Replace one registry's tags with `tags`, each tag's members in order.
    pub fn set_tags(&mut self, registry: TagRegistry, tags: HashMap<String, Vec<String>>) {
        match registry {
            TagRegistry::Block | TagRegistry::Item | TagRegistry::Fluid => {
                let (sets, order) = match registry {
                    TagRegistry::Block => (&mut self.block_tags, &mut self.block_tag_order),
                    TagRegistry::Item => {
                        self.item_tag_names = Some(ItemTagNames::bind(&tags));
                        (&mut self.item_tags, &mut self.item_tag_order)
                    }
                    TagRegistry::Fluid => (&mut self.fluid_tags, &mut self.fluid_tag_order),
                    _ => unreachable!(),
                };
                *sets = tags
                    .iter()
                    .map(|(tag, members)| (tag.clone(), members.iter().cloned().collect()))
                    .collect();
                *order = tags;
                self.tags_loaded[registry as usize] = true;
                if registry == TagRegistry::Block {
                    let names = self.block_state_names();
                    self.rebuild_block_tag_flags(names);
                }
            }
            TagRegistry::EntityType => {
                self.set_entity_type_tags(EntityTypeTags::from_ordered(
                    tags,
                    self.entity_type_tags.ids().clone(),
                ));
            }
            TagRegistry::Biome => {
                self.biome_tag_order = tags;
                self.tags_loaded[registry as usize] = true;
            }
            TagRegistry::Enchantment => {
                self.enchantment_tag_order = tags;
                self.tags_loaded[registry as usize] = true;
            }
            TagRegistry::Structure => {
                self.structure_tag_order = tags;
                self.tags_loaded[registry as usize] = true;
            }
            TagRegistry::Potion => {
                self.potion_tag_order = tags;
                self.tags_loaded[registry as usize] = true;
            }
            TagRegistry::DamageType => {
                self.damage_type_tag_order = tags;
                self.tags_loaded[registry as usize] = true;
            }
            TagRegistry::WorldClock => {
                self.world_clock_tag_order = tags;
                self.tags_loaded[registry as usize] = true;
            }
        }
    }
    /// Whether `block` is in the block tag `tag` (`minecraft:fire`, no `#`).
    pub fn block_in_tag(&self, tag: &str, block: &str) -> bool {
        self.block_tags.get(tag).is_some_and(|s| s.contains(block))
    }
    /// Whether `fluid` is in the fluid tag `tag` (`minecraft:water`, no `#`): `FluidState.is(TagKey)`.
    pub fn fluid_in_tag(&self, tag: &str, fluid: &str) -> bool {
        self.fluid_tags.get(tag).is_some_and(|s| s.contains(fluid))
    }
    /// Whether this registry's authoritative tag table has been installed, even if empty.
    pub fn tags_loaded(&self, registry: TagRegistry) -> bool {
        self.tags_loaded[registry as usize]
    }
    /// A loaded tag's members in `TagLoader` order, or `None` for a tag that isn't loaded.
    pub fn tag_members(&self, registry: TagRegistry, tag: &str) -> Option<&[String]> {
        let order = match registry {
            TagRegistry::Block => &self.block_tag_order,
            TagRegistry::Item => &self.item_tag_order,
            TagRegistry::Fluid => &self.fluid_tag_order,
            TagRegistry::EntityType => self.entity_type_tags.ordered(),
            TagRegistry::Biome => &self.biome_tag_order,
            TagRegistry::Enchantment => &self.enchantment_tag_order,
            TagRegistry::Structure => &self.structure_tag_order,
            TagRegistry::Potion => &self.potion_tag_order,
            TagRegistry::DamageType => &self.damage_type_tag_order,
            TagRegistry::WorldClock => &self.world_clock_tag_order,
        };
        order.get(tag).map(Vec::as_slice)
    }
    /// Whether an item belongs to a loaded item tag.
    pub fn item_in_tag(&self, tag: &str, item: &str) -> bool {
        let normalized;
        let tag = if !tag.contains(':') || tag.starts_with(':') {
            normalized = format!("minecraft:{}", tag.strip_prefix(':').unwrap_or(tag));
            normalized.as_str()
        } else {
            tag
        };
        self.item_tags.get(tag).is_some_and(|s| s.contains(item))
    }
    fn block_matches(&self, spec: &str, block: &str) -> bool {
        match spec.strip_prefix('#') {
            Some(tag) => self.block_in_tag(tag, block),
            None => spec == block,
        }
    }
    /// `Item.canDestroyBlock`: a tool may forbid block destruction with instabuild.
    pub fn can_destroy_block(&self, held: Option<&ItemStack>, creative: bool) -> bool {
        if !creative {
            return true;
        }
        let Some(held) = held.filter(|s| s.count > 0 && &*s.item != "minecraft:air") else {
            return true;
        };
        if held
            .data()
            .is_some_and(|d| d.other("!minecraft:tool").is_some())
        {
            return true;
        }
        if let Some(tool) = held.data().and_then(|d| d.other("minecraft:tool")) {
            return tool
                .get("can_destroy_blocks_in_creative")
                .and_then(|v| match v {
                    sunrise_core::nbt::Tag::Byte(v) => Some(*v),
                    _ => None,
                })
                .is_none_or(|v| v != 0);
        }
        self.get(&held.item)
            .is_none_or(|info| info.can_destroy_blocks_in_creative)
    }
    /// `Tool.getMiningSpeed` for the held item (1.0 without a tool component).
    pub fn mining_speed(&self, held: Option<&ItemStack>, block: &str) -> f32 {
        let held = held.filter(|s| s.count > 0 && &*s.item != "minecraft:air");
        if let Some(stack) = held {
            if stack
                .data()
                .is_some_and(|d| d.other("!minecraft:tool").is_some())
            {
                return 1.0;
            }
            if let Some(tool) = stack.data().and_then(|d| d.other("minecraft:tool")) {
                return self
                    .stack_tool_rule(tool, block, "speed")
                    .and_then(tool_number)
                    .or_else(|| tool.get("default_mining_speed").and_then(tool_number))
                    .unwrap_or(1.0);
            }
        }
        let Some(info) = held.and_then(|s| self.get(&s.item)) else {
            return 1.0;
        };
        for rule in &info.tool {
            if let Some(speed) = rule.speed
                && rule.blocks.iter().any(|b| self.block_matches(b, block))
            {
                return speed;
            }
        }
        if info.tool.is_empty() {
            1.0
        } else {
            info.tool_default_speed
        }
    }
    /// `Tool.isCorrectForDrops`.
    pub fn correct_for_drops(&self, held: Option<&ItemStack>, block: &str) -> bool {
        let held = held.filter(|s| s.count > 0 && &*s.item != "minecraft:air");
        if let Some(stack) = held {
            if stack
                .data()
                .is_some_and(|d| d.other("!minecraft:tool").is_some())
            {
                return false;
            }
            if let Some(tool) = stack.data().and_then(|d| d.other("minecraft:tool")) {
                return sunrise_world::datafix::dynamic::as_bool(
                    self.stack_tool_rule(tool, block, "correct_for_drops"),
                    false,
                );
            }
        }
        let Some(info) = held.and_then(|s| self.get(&s.item)) else {
            return false;
        };
        for rule in &info.tool {
            if let Some(correct) = rule.correct
                && rule.blocks.iter().any(|b| self.block_matches(b, block))
            {
                return correct;
            }
        }
        false
    }
    fn stack_tool_rule<'a>(
        &self,
        tool: &'a sunrise_core::nbt::Tag,
        block: &str,
        field: &str,
    ) -> Option<&'a sunrise_core::nbt::Tag> {
        use sunrise_core::nbt::Tag;
        let Tag::List(rules) = tool.get("rules")? else {
            return None;
        };
        rules.iter().find_map(|rule| {
            let value = rule.get(field)?;
            let matches = |value: &Tag| match value {
                Tag::String(spec) => spec.to_string().is_ok_and(|s| {
                    let tag = s.starts_with('#');
                    let id = s.strip_prefix('#').unwrap_or(&s);
                    sunrise_world::datafix::identifier(id).is_some_and(|(namespace, path)| {
                        let id = format!("{namespace}:{path}");
                        if tag {
                            self.block_in_tag(&id, block)
                        } else {
                            id == block
                        }
                    })
                }),
                _ => false,
            };
            match rule.get("blocks")? {
                Tag::List(blocks) if blocks.iter().any(matches) => Some(value),
                spec if matches(spec) => Some(value),
                _ => None,
            }
        })
    }
    /// The stack's TOOL component, including its default mining wear.
    pub fn mining_damage(&self, stack: &ItemStack) -> Option<u32> {
        if stack.count == 0 || &*stack.item == "minecraft:air" {
            return None;
        }
        if stack
            .data()
            .is_some_and(|d| d.other("!minecraft:tool").is_some())
        {
            return None;
        }
        if let Some(tool) = stack.data().and_then(|d| d.other("minecraft:tool")) {
            return Some(
                sunrise_world::datafix::dynamic::as_int(tool.get("damage_per_block"), 1).max(0)
                    as u32,
            );
        }
        self.get(&stack.item)?.tool_damage_per_block
    }
    /// `Weapon.disableBlockingForSeconds`: the stack's `weapon` component,
    /// else the item's prototype; zero without one.
    pub fn weapon_disable_blocking(&self, stack: &ItemStack) -> f32 {
        if stack.count == 0 || &*stack.item == "minecraft:air" {
            return 0.0;
        }
        if stack
            .data()
            .is_some_and(|d| d.other("!minecraft:weapon").is_some())
        {
            return 0.0;
        }
        if let Some(weapon) = stack.data().and_then(|d| d.other("minecraft:weapon")) {
            return sunrise_world::datafix::dynamic::as_float(
                weapon.get("disable_blocking_for_seconds"),
                0.0,
            );
        }
        self.get(&stack.item)
            .map_or(0.0, |i| i.disable_blocking_seconds)
    }

    /// `Weapon.itemDamagePerAttack`: the stack's `weapon` component, else the
    /// item's prototype. A removed component leaves no weapon.
    pub fn weapon_damage(&self, stack: &ItemStack) -> Option<u32> {
        if stack.count == 0 || &*stack.item == "minecraft:air" {
            return None;
        }
        if stack
            .data()
            .is_some_and(|d| d.other("!minecraft:weapon").is_some())
        {
            return None;
        }
        if let Some(weapon) = stack.data().and_then(|d| d.other("minecraft:weapon")) {
            return Some(
                sunrise_world::datafix::dynamic::as_int(weapon.get("item_damage_per_attack"), 1)
                    .max(0) as u32,
            );
        }
        self.get(&stack.item)?.weapon_damage
    }
    /// The stack's damage capacity (`crate::durability::stack_max_damage`).
    pub fn stack_max_damage(&self, stack: &ItemStack) -> u32 {
        crate::durability::stack_max_damage(self, stack)
    }
    /// The damage capacity mining wear uses (`stack_max_damage`).
    pub fn mining_damage_capacity(&self, stack: &ItemStack) -> u32 {
        self.stack_max_damage(stack)
    }
    /// Whether a loot table (`minecraft:blocks/stone`, ...) exists.
    pub fn has_loot_table(&self, id: &str) -> bool {
        self.loot.contains_key(id)
    }
    /// Every loot table's id: the root's (vanilla's and the content mods')
    /// and the packs' over them, in lexical order.
    pub fn loot_table_ids(&self) -> impl Iterator<Item = &str> {
        self.loot.ids()
    }
    /// `LootTable.getRandomItems` of an inline table.
    pub fn inline_loot_table<R: RandomSource>(
        &self,
        table: &Value,
        random: &mut R,
    ) -> Vec<ItemStack> {
        self.inline_loot_table_with_tool(table, None, random)
    }
    /// `LootCommand.dropFishingLoot`: evaluate an inline table with its TOOL.
    pub fn inline_loot_table_with_tool<R: RandomSource>(
        &self,
        table: &Value,
        held: Option<&ItemStack>,
        random: &mut R,
    ) -> Vec<ItemStack> {
        self.inline_loot_table_for_command(table, held, random, false)
    }
    /// Prepare an inline loot table at the argument boundary before drawing.
    pub fn prepare_inline_loot_table(
        &self,
        table: &Value,
    ) -> std::io::Result<sunrise_world::loot::types::PreparedInlineTable> {
        self.loot_bindings.prepare_inline_table(table)
    }
    /// Roll an already-prepared inline loot table; replacement keeps emitted empties.
    pub fn roll_prepared_inline_loot_table_for_command<R: RandomSource>(
        &self,
        table: &sunrise_world::loot::types::PreparedInlineTable,
        held: Option<&ItemStack>,
        random: &mut R,
        keep_empty: bool,
    ) -> Vec<ItemStack> {
        let empty = sunrise_world::chunk::BlockProperties::new();
        let ctx = LootContext {
            query: sunrise_world::loot::query::current_query(),
            place: sunrise_world::loot::current_native_loot_place(),
            items: self,
            local_bindings: Some(table.rules()),
            block: "",
            properties: &empty,
            held,
            killed_by_player: false,
            explosion_radius: None,
            this_entity: false,
            entity_levels: None,
            block_levels: None,
            target: None,
            target_entity: None,
            interacting_entity: None,
            location: None,
            keep_empty,
        };
        ctx.roll_root(None, table.value(), random)
    }
    /// `LootCommand.dropFishingLoot`: output-list adapter; replacement keeps emitted empties.
    pub fn inline_loot_table_for_command<R: RandomSource>(
        &self,
        table: &Value,
        held: Option<&ItemStack>,
        random: &mut R,
        keep_empty: bool,
    ) -> Vec<ItemStack> {
        let prepared = match self.prepare_inline_loot_table(table) {
            Ok(prepared) => prepared,
            Err(error) => {
                log::warn!("inline loot table refused: {error}");
                return Vec::new();
            }
        };
        self.roll_prepared_inline_loot_table_for_command(&prepared, held, random, keep_empty)
    }
    /// The id of the block's loot table (`minecraft:blocks/stone`), when it
    /// has one. `Blocks.wallVariant`: wall signs, banners, heads, fans and
    /// torches use the standing block's table.
    pub fn block_loot_table(
        &self,
        registry: &BlockRegistry,
        state: BlockStateId,
    ) -> Option<String> {
        let block = registry.state(state)?;
        let (namespace, name) = block
            .name
            .split_once(':')
            .unwrap_or(("minecraft", &block.name));
        let own = format!("{namespace}:blocks/{name}");
        if self.loot.contains_key(&own) {
            return Some(own);
        }
        let standing = name.replacen("_wall_", "_", 1);
        let standing = standing.strip_prefix("wall_").unwrap_or(&standing);
        let standing = standing.replacen("wall_", "", 1);
        let standing = format!("{namespace}:blocks/{standing}");
        self.loot.contains_key(&standing).then_some(standing)
    }
    /// `BlockPredicate` state matching shared with block loot conditions.
    pub fn matches_block_state(
        &self,
        c: &Value,
        block: &str,
        properties: &sunrise_world::chunk::BlockProperties,
    ) -> bool {
        let blocks_ok = match &c["blocks"].as_str().or(c["block"].as_str()) {
            Some(spec) => self.block_matches(spec, block),
            None => true,
        };
        blocks_ok
            && c["state"]
                .as_object()
                .into_iter()
                .flatten()
                .chain(c["properties"].as_object().into_iter().flatten())
                .all(|(k, v)| {
                    let have = properties.get(k).map(String::as_str);
                    match v {
                        Value::String(s) => have == Some(s.as_str()),
                        Value::Object(r) => {
                            let have: i64 = have.and_then(|h| h.parse().ok()).unwrap_or(0);
                            r.get("min")
                                .and_then(Value::as_i64)
                                .is_none_or(|m| have >= m)
                                && r.get("max")
                                    .and_then(Value::as_i64)
                                    .is_none_or(|m| have <= m)
                        }
                        _ => true,
                    }
                })
    }
    /// `Block.getDrops`: the block's loot table rolled with `roll`, drawing
    /// from the table's random sequence when it has one (`sequences`: the
    /// world's sequences and seed), else from `random`.
    pub fn block_drops(
        &self,
        registry: &BlockRegistry,
        state: BlockStateId,
        roll: BlockLootRoll<'_>,
        random: &mut impl RandomSource,
        sequences: Option<(&mut crate::random_sequences::RandomSequences, i64)>,
    ) -> Vec<ItemStack> {
        self.scoped_block_roll(registry, state, roll, random, sequences)
    }

    fn roll_block_table<R: RandomSource>(
        &self,
        id: &str,
        table: &Value,
        block: &str,
        properties: &sunrise_world::chunk::BlockProperties,
        roll: BlockLootRoll<'_>,
        random: &mut R,
    ) -> Vec<ItemStack> {
        let ask = |_: &mut R, id: &str, level: u32| roll.levels.map_or(level, |ask| ask(id, level));
        let ctx = LootContext {
            query: sunrise_world::loot::query::current_query(),
            place: sunrise_world::loot::current_native_loot_place(),
            items: self,
            local_bindings: None,
            block,
            properties,
            held: roll.held,
            killed_by_player: false,
            explosion_radius: roll.explosion,
            this_entity: roll.breaker,
            entity_levels: None,
            block_levels: roll.levels.map(|_| &ask as &LootLevels<'_, R>),
            target: None,
            target_entity: None,
            interacting_entity: None,
            location: roll.location,
            keep_empty: roll.keep_empty,
        };
        let raw = ctx.evaluate_candidates(table, random);
        ctx.complete_root(Some(id), raw, random, Some(roll.dynamic))
    }
    /// Evaluate an entity's loot table (`minecraft:entities/<name>`).
    /// Evaluate any loot table by id (e.g. `minecraft:gameplay/chicken_lay`).
    pub fn loot_table<R: RandomSource>(
        &self,
        id: &str,
        killed_by_player: bool,
        held: Option<&ItemStack>,
        random: &mut R,
    ) -> Vec<ItemStack> {
        self.loot_table_for_command(id, killed_by_player, held, random, false)
    }
    /// `LootCommand.dropChestLoot`: output-list adapter; replacement keeps emitted empties.
    pub fn loot_table_for_command<R: RandomSource>(
        &self,
        id: &str,
        killed_by_player: bool,
        held: Option<&ItemStack>,
        random: &mut R,
        keep_empty: bool,
    ) -> Vec<ItemStack> {
        self.table_root(id, killed_by_player, held, None, None, random, keep_empty)
            .unwrap_or_default()
    }
    /// `LootTable.getRandomItems(LootParams)` of table `id` with an ORIGIN,
    /// `held` as the TOOL and, when `this` is given, that entity as THIS_ENTITY
    /// (the origin is the place the caller set). The stacks come
    /// after the stack splitter, each at most its item's stack size, empties
    /// left out. None when there is no such table.
    pub fn roll_loot_table<R: RandomSource>(
        &self,
        id: &str,
        held: Option<&ItemStack>,
        this: Option<EntityTarget<'_>>,
        location: Option<sunrise_world::loot::LootLocation<'_>>,
        random: &mut R,
    ) -> Option<Vec<ItemStack>> {
        self.table_root(id, false, held, this, location, random, false)
    }
    /// A named root with no block; command replacement may retain emitted empties.
    fn table_root<R: RandomSource>(
        &self,
        id: &str,
        killed_by_player: bool,
        held: Option<&ItemStack>,
        target: Option<EntityTarget<'_>>,
        location: Option<sunrise_world::loot::LootLocation<'_>>,
        random: &mut R,
        keep_empty: bool,
    ) -> Option<Vec<ItemStack>> {
        let table = self.loot.get(id)?;
        let empty = sunrise_world::chunk::BlockProperties::new();
        let ctx = LootContext {
            query: sunrise_world::loot::query::current_query(),
            place: sunrise_world::loot::current_native_loot_place(),
            items: self,
            local_bindings: None,
            block: "",
            properties: &empty,
            held,
            killed_by_player,
            explosion_radius: None,
            this_entity: false,
            entity_levels: None,
            block_levels: None,
            target,
            target_entity: None,
            interacting_entity: None,
            location,
            keep_empty,
        };
        // LootTable.getRandomItemsRaw visits the table itself first.
        let raw = {
            let _guard = NestedRoll::enter(id)?;
            ctx.evaluate_candidates(table, random)
        };
        Some(ctx.complete_root(Some(id), raw, random, None))
    }
    /// The server random sequence a loot table draws from (`random_sequence`):
    /// vanilla's `LootContext.Builder.create` uses it instead of the level random.
    pub fn loot_sequence(&self, id: &str) -> Option<String> {
        self.loot.get(id)?["random_sequence"]
            .as_str()
            .map(str::to_owned)
    }
    #[allow(clippy::too_many_arguments)]
    fn interact_roll<R: RandomSource>(
        &self,
        id: &str,
        table: &Value,
        block: &str,
        properties: &sunrise_world::chunk::BlockProperties,
        interacting: Option<EntityTarget<'_>>,
        held: Option<&ItemStack>,
        location: Option<sunrise_world::loot::LootLocation<'_>>,
        random: &mut R,
    ) -> Vec<ItemStack> {
        let ctx = LootContext {
            query: sunrise_world::loot::query::current_query(),
            place: sunrise_world::loot::current_native_loot_place(),
            items: self,
            local_bindings: None,
            block,
            properties,
            held,
            killed_by_player: false,
            explosion_radius: None,
            this_entity: false,
            entity_levels: None,
            block_levels: None,
            target: None,
            target_entity: None,
            interacting_entity: interacting,
            location,
            keep_empty: false,
        };
        ctx.roll_root(Some(id), table, random)
    }
    /// `loot_table` drawing from any random (a random sequence).
    pub fn loot_table_from(
        &self,
        id: &str,
        killed_by_player: bool,
        held: Option<&ItemStack>,
        random: &mut impl RandomSource,
    ) -> Vec<ItemStack> {
        let Some(table) = self.loot.get(id) else {
            return vec![];
        };
        let empty = sunrise_world::chunk::BlockProperties::new();
        let ctx = LootContext {
            query: sunrise_world::loot::query::current_query(),
            place: sunrise_world::loot::current_native_loot_place(),
            items: self,
            local_bindings: None,
            block: "",
            properties: &empty,
            held,
            killed_by_player,
            explosion_radius: None,
            this_entity: false,
            entity_levels: None,
            block_levels: None,
            target: None,
            target_entity: None,
            interacting_entity: None,
            location: None,
            keep_empty: false,
        };
        ctx.roll_root(Some(id), table, random)
    }
    pub fn entity_drops<R: RandomSource>(
        &self,
        entity: &str,
        killed_by_player: bool,
        held: Option<&ItemStack>,
        random: &mut R,
    ) -> Vec<ItemStack> {
        self.entity_drops_with(entity, killed_by_player, held, random, None, None, false)
    }
    /// Command output retains emitted empty positions when replacing slots.
    pub fn entity_drops_for_command<R: RandomSource>(
        &self,
        entity: &str,
        killed_by_player: bool,
        held: Option<&ItemStack>,
        random: &mut R,
        keep_empty: bool,
    ) -> Vec<ItemStack> {
        self.entity_drops_with(
            entity,
            killed_by_player,
            held,
            random,
            None,
            None,
            keep_empty,
        )
    }
    /// [`Self::entity_drops`] with `level` asked each time
    /// `enchanted_count_increase` or `random_chance_with_enchanted_bonus`
    /// reads the attacker's level.
    pub fn entity_drops_asked<R: RandomSource>(
        &self,
        entity: &str,
        killed_by_player: bool,
        held: Option<&ItemStack>,
        random: &mut R,
        level: &mut sunrise_world::loot::EnchantedLevel<'_, R>,
        keep_empty: bool,
    ) -> Vec<ItemStack> {
        let level = std::cell::RefCell::new(level);
        let levels = |random: &mut R, id: &str, read: u32| {
            let read = read.min(i32::MAX as u32) as i32;
            (level.borrow_mut())(random, id, read).max(0) as u32
        };
        self.entity_drops_with(
            entity,
            killed_by_player,
            held,
            random,
            Some(&levels),
            None,
            keep_empty,
        )
    }
    /// Command drops with a live entity predicate context.
    #[allow(clippy::too_many_arguments)]
    pub fn entity_drops_with_target<R: RandomSource>(
        &self,
        entity: &str,
        killed_by_player: bool,
        held: Option<&ItemStack>,
        target: &sunrise_world::loot::LootEntity,
        matcher: &dyn Fn(&Value, &sunrise_world::loot::LootEntity) -> bool,
        random: &mut R,
        keep_empty: bool,
    ) -> Vec<ItemStack> {
        self.entity_drops_with(
            entity,
            killed_by_player,
            held,
            random,
            None,
            Some((target, matcher)),
            keep_empty,
        )
    }
    /// Command drops with a live target and an answer at each enchanted-level read.
    #[allow(clippy::too_many_arguments)]
    pub fn entity_drops_asked_with_target<R: RandomSource>(
        &self,
        entity: &str,
        killed_by_player: bool,
        held: Option<&ItemStack>,
        target: &sunrise_world::loot::LootEntity,
        matcher: &dyn Fn(&Value, &sunrise_world::loot::LootEntity) -> bool,
        random: &mut R,
        level: &mut sunrise_world::loot::EnchantedLevel<'_, R>,
        keep_empty: bool,
    ) -> Vec<ItemStack> {
        let level = std::cell::RefCell::new(level);
        let levels = |random: &mut R, id: &str, read: u32| {
            let read = read.min(i32::MAX as u32) as i32;
            (level.borrow_mut())(random, id, read).max(0) as u32
        };
        self.entity_drops_with(
            entity,
            killed_by_player,
            held,
            random,
            Some(&levels),
            Some((target, matcher)),
            keep_empty,
        )
    }
    fn entity_drops_with<R: RandomSource>(
        &self,
        entity: &str,
        killed_by_player: bool,
        held: Option<&ItemStack>,
        random: &mut R,
        levels: Option<&LootLevels<'_, R>>,
        target: Option<EntityTarget<'_>>,
        keep_empty: bool,
    ) -> Vec<ItemStack> {
        let name = entity.trim_start_matches("minecraft:");
        let Some(table) = self.loot.get(&format!("minecraft:entities/{name}")) else {
            return vec![];
        };
        let empty = sunrise_world::chunk::BlockProperties::new();
        let ctx = LootContext {
            query: sunrise_world::loot::query::current_query(),
            place: sunrise_world::loot::current_native_loot_place(),
            items: self,
            local_bindings: None,
            block: "",
            properties: &empty,
            held,
            killed_by_player,
            explosion_radius: None,
            this_entity: false,
            entity_levels: levels,
            block_levels: None,
            target,
            target_entity: None,
            interacting_entity: None,
            location: None,
            keep_empty,
        };
        ctx.roll_root(Some(&format!("minecraft:entities/{name}")), table, random)
    }
}
/// `DataComponents.BLOCK_STATE`.
const BLOCK_STATE: &str = "minecraft:block_state";
impl<R: RandomSource> LootContext<'_, R> {
    /// The entity a condition's `entity` field (a `LootContext.EntityTarget`)
    /// names, when the roll carries it.
    fn named_entity(&self, name: &str) -> Option<&EntityTarget<'_>> {
        match name {
            "this" => self.target.as_ref(),
            "target_entity" => self.target_entity.as_ref(),
            "interacting_entity" => self.interacting_entity.as_ref(),
            _ => None,
        }
    }
    /// Raw evaluation: nested tables share this context, without splitting
    /// output or completing the queried table id.
    #[cfg(test)]
    fn evaluate(&self, table: &Value, random: &mut R) -> Vec<ItemStack> {
        self.evaluate_candidates(table, random)
            .into_iter()
            .map(|candidate| candidate.into_item_stack(self.items))
            .collect()
    }
    fn evaluate_candidates(
        &self,
        table: &Value,
        random: &mut R,
    ) -> Vec<crate::loot_candidate::LootCandidate> {
        let ctx = self;
        let mut out = Vec::new();
        for pool in table["pools"].as_array().into_iter().flatten() {
            if !ctx.conditions(pool, random) {
                continue;
            }
            // LootPool: rolls.getInt (Mth.randomBetweenInclusive for uniform rolls).
            let rolls = Self::int_number(&pool["rolls"], random).max(0) as u32;
            for _ in 0..rolls {
                let mut produced = Vec::new();
                ctx.choose(
                    pool["entries"].as_array().map_or(&[][..], Vec::as_slice),
                    random,
                    &mut produced,
                );
                for (mut stack, emitted) in produced {
                    ctx.modifiers(&pool["modifier"], &mut stack, random);
                    ctx.modifiers(&table["modifier"], &mut stack, random);
                    if stack.count() > 0 || (ctx.keep_empty && emitted) {
                        out.push(stack);
                    }
                }
            }
        }
        out
    }
}
/// What a roll asks when its table reads an enchantment level: given the
/// roll's random, the enchantment and the level, the level to use. The random
/// is the roll's own, so whoever answers can keep draws made meanwhile in order.
pub type LootLevels<'a, R> = dyn Fn(&mut R, &str, u32) -> u32 + 'a;
/// The game's say on an enchantment level a block's table reads: the
/// enchantment and the tool's level in, the level to use out. A block's
/// listeners make no draws, so the roll's random isn't theirs.
pub type BlockLootLevels<'a> = dyn Fn(&str, u32) -> u32 + 'a;
/// What a block's table rolls with, beside the block (`LootParams` as
/// `BlockState.getDrops` is given them).
#[derive(Clone, Copy, Default)]
pub struct BlockLootRoll<'a> {
    /// `LootContextParams.ORIGIN` (the block's centre) and the level its location checks
    /// read; without one every `location_check` fails.
    pub location: Option<sunrise_world::loot::LootLocation<'a>>,
    /// Retain emitted empty slots for command replacement output.
    pub keep_empty: bool,
    /// `LootContextParams.TOOL`; none is the empty stack.
    pub held: Option<&'a ItemStack>,
    /// What the `minecraft:dynamic` entries become
    /// (`LootParams.withDynamicDrop`: a decorated pot's sherds).
    pub dynamic: &'a [ItemStack],
    /// `LootContextParams.EXPLOSION_RADIUS`: the radius of an explosion that
    /// decays drops (`DESTROY_WITH_DECAY`).
    pub explosion: Option<f32>,
    /// `LootContextParams.THIS_ENTITY` is set (a player broke the block).
    pub breaker: bool,
    /// Asked at each level `ApplyBonusCount` and `BonusLevelTableCondition`
    /// read, in the roll's order; none keeps the tool's.
    pub levels: Option<&'a BlockLootLevels<'a>>,
}
/// An entity a roll names as `this`, with the predicate matcher for its conditions.
pub type EntityTarget<'a> = (
    &'a sunrise_world::loot::LootEntity,
    &'a dyn Fn(&Value, &sunrise_world::loot::LootEntity) -> bool,
);
struct LootContext<'a, R> {
    query: std::rc::Rc<sunrise_world::loot::query::LootQuery>,
    place: Option<Arc<sunrise_world::loot::LootPlace>>,
    items: &'a Items,
    local_bindings: Option<&'a sunrise_world::loot::types::RuleMap>,
    block: &'a str,
    properties: &'a sunrise_world::chunk::BlockProperties,
    held: Option<&'a ItemStack>,
    killed_by_player: bool,
    /// `LootContextParams.EXPLOSION_RADIUS`: an explosion that decays drops.
    explosion_radius: Option<f32>,
    /// `LootContextParams.THIS_ENTITY` is set (a player broke the block).
    this_entity: bool,
    /// Asked only at entity enchantment readers; None keeps vanilla's level.
    entity_levels: Option<&'a LootLevels<'a, R>>,
    /// Asked only at block enchantment readers; None keeps vanilla's level.
    block_levels: Option<&'a LootLevels<'a, R>>,
    target: Option<EntityTarget<'a>>,
    /// `LootContextParams.TARGET_ENTITY`.
    target_entity: Option<EntityTarget<'a>>,
    /// `LootContextParams.INTERACTING_ENTITY`.
    interacting_entity: Option<EntityTarget<'a>>,
    /// `ORIGIN` and the level, for `location_check`; None without an origin.
    location: Option<sunrise_world::loot::LootLocation<'a>>,
    /// Command replacement consumes the emitted list positions, including empty stacks.
    keep_empty: bool,
}
impl<R: RandomSource> LootContext<'_, R> {
    /// Level of an enchantment on the tool / killer's weapon.
    fn enchantment_level(&self, id: &str) -> u32 {
        self.held.map_or(0, |h| h.enchantment_level(id))
    }
    /// The level `EnchantedCountIncreaseFunction` and
    /// `LootItemRandomChanceWithEnchantedBonusCondition` use: the attacker's
    /// (a player's, 0 without one), as `levels` leaves it, up to what a table
    /// reads.
    fn attacker_level(&self, id: &str, random: &mut R) -> u32 {
        let level = if self.killed_by_player && self.block.is_empty() {
            self.enchantment_level(id)
        } else {
            0
        };
        let level = self
            .entity_levels
            .map_or(level, |levels| levels(random, id, level));
        level.min(MAX_LOOT_LEVEL as u32)
    }
    /// The level `ApplyBonusCount.run` and `BonusLevelTableCondition.test`
    /// use: the tool's, as the game leaves it, up to what a table reads.
    fn bonus_level(&self, id: &str, random: &mut R) -> i32 {
        let level = self.enchantment_level(id);
        let level = self
            .block_levels
            .map_or(level, |levels| levels(random, id, level));
        level.min(MAX_LOOT_LEVEL as u32) as i32
    }
    fn number(v: &Value, random: &mut R) -> f32 {
        match v {
            Value::Number(n) => n.as_f64().unwrap_or(0.0) as f32,
            Value::Object(o) => match o
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("minecraft:uniform")
            {
                "minecraft:constant" => {
                    o.get("value").and_then(Value::as_f64).unwrap_or(0.0) as f32
                }
                "minecraft:uniform" => {
                    let min = Self::number(o.get("min").unwrap_or(&Value::Null), random);
                    let max = Self::number(o.get("max").unwrap_or(&Value::Null), random);
                    // UniformGenerator.getFloat / getInt (Mth.nextInt) differ; use int for counts.
                    if max <= min {
                        min
                    } else {
                        min + random.next_float() * (max - min)
                    }
                }
                "minecraft:binomial" => {
                    let n =
                        Self::int_number(o.get("n").unwrap_or(&Value::Null), random).max(0) as u32;
                    let p = Self::number(o.get("p").unwrap_or(&Value::Null), random);
                    (0..n).filter(|_| random.next_float() < p).count() as f32
                }
                // An unknown type gives 0, as in loot.rs `int` and `float`.
                _ => 0.0,
            },
            _ => 0.0,
        }
    }
    fn int_number(v: &Value, random: &mut R) -> i32 {
        if let Some(number) = v.as_number() {
            return int_value(number) as i32;
        }
        if v["type"].as_str() == Some("minecraft:constant") {
            return Self::int_number(&v["value"], random);
        }
        if let Value::Object(o) = v
            && o.get("type")
                .and_then(Value::as_str)
                .unwrap_or("minecraft:uniform")
                == "minecraft:uniform"
        {
            let min = Self::int_number(o.get("min").unwrap_or(&Value::Null), random);
            let max = Self::int_number(o.get("max").unwrap_or(&Value::Null), random);
            return if max <= min {
                min
            } else {
                random.next_int_between_inclusive(min, max).unwrap_or(min)
            };
        }
        Self::number(v, random).round() as i32
    }
    fn condition(&self, c: &Value, random: &mut R) -> bool {
        self.condition_with_bindings(c, random, self.local_bindings)
    }
    fn condition_with_bindings(
        &self,
        c: &Value,
        random: &mut R,
        bindings: Option<&sunrise_world::loot::types::RuleMap>,
    ) -> bool {
        if let Some(holds) = self.stock_condition(c) {
            return holds;
        }
        let (kind, c) = match c {
            Value::Null => return true,
            Value::Array(list) => {
                return list
                    .iter()
                    .all(|c| self.condition_with_bindings(c, random, bindings));
            }
            // A string names a `Registries.PREDICATE` id (`LootItemCondition.CODEC`).
            Value::String(id) => return self.named_condition(id, random),
            Value::Object(o) => (o.get("type").and_then(Value::as_str).unwrap_or(""), c),
            _ => return false,
        };
        match kind {
            // ExplosionCondition: 1 / radius under a decaying explosion.
            "minecraft:survives_explosion" => self
                .explosion_radius
                .is_none_or(|r| random.next_float() <= 1.0 / r),
            "minecraft:inverted" => !self.condition_with_bindings(&c["term"], random, bindings),
            "minecraft:any_of" => c["terms"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|t| self.condition_with_bindings(t, random, bindings)),
            "minecraft:all_of" => c["terms"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|t| self.condition_with_bindings(t, random, bindings)),
            "minecraft:random_chance" => random.next_float() < Self::number(&c["chance"], random),
            "minecraft:table_bonus" => {
                // BonusLevelTableCondition: chances[min(level, len - 1)].
                let level =
                    self.bonus_level(c["enchantment"].as_str().unwrap_or(""), random) as usize;
                let chances = c["chances"].as_array().map_or(&[][..], Vec::as_slice);
                let chance = chances
                    .get(level.min(chances.len().saturating_sub(1)))
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0) as f32;
                random.next_float() < chance
            }
            "minecraft:block_state_property" => {
                self.items
                    .matches_block_state(c, self.block, self.properties)
            }
            // `MatchBlock.test`: the BLOCK_STATE first, then the predicate's NBT and
            // component tests against the BLOCK_ENTITY the roll supplied.
            "minecraft:match_block" => {
                !self.block.is_empty()
                    && self
                        .items
                        .matches_block_state(c, self.block, self.properties)
                    && (!sunrise_world::loot::reads_block_entity(c) || {
                        let place = sunrise_world::loot::current_native_loot_place();
                        let place = place.as_deref();
                        let item_tags = |registry: &str, tag: &str, id: &str| {
                            registry == "item"
                                && self
                                    .items
                                    .item_tags
                                    .get(tag)
                                    .is_some_and(|t| t.contains(id))
                        };
                        sunrise_world::loot::block_entity_matches(
                            c,
                            place.and_then(|p| p.block_entity.as_ref()),
                            || place.and_then(|p| p.block_components.clone()),
                            &item_tags,
                        )
                    })
            }
            "minecraft:match_tool" => {
                let items = &c["predicate"]["items"];
                let items_ok = match (self.held, items) {
                    (_, Value::Null) => self.held.is_some(),
                    (Some(h), Value::String(s)) => match s.strip_prefix('#') {
                        Some(tag) => self
                            .items
                            .item_tags
                            .get(tag)
                            .is_some_and(|t| t.contains(&*h.item)),
                        None => **s == *h.item,
                    },
                    _ => false,
                };
                // ItemPredicate "minecraft:enchantments": each entry needs a level in range.
                let enchants_ok = c["predicate"]["predicates"]["minecraft:enchantments"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .all(|e| {
                        let level =
                            self.enchantment_level(e["enchantments"].as_str().unwrap_or(""));
                        let min = e["levels"]["min"].as_number().map(int_value).unwrap_or(1);
                        let max = e["levels"]["max"]
                            .as_number()
                            .map(int_value)
                            .unwrap_or(i64::from(i32::MAX));
                        i64::from(level) >= min.max(1) && i64::from(level) <= max
                    });
                let custom_ok = c["predicate"]["predicates"]
                    .get("minecraft:custom_data")
                    .is_none_or(|pattern| {
                        sunrise_world::loot::custom_data_matches(
                            pattern,
                            self.held.and_then(ItemStack::custom_data),
                        )
                    });
                items_ok && enchants_ok && custom_ok
            }
            "minecraft:killed_by_player" => self.killed_by_player,
            // TimeCheck: the level's clock, in the range (bounds as written; none is open).
            "minecraft:time_check" => sunrise_world::loot::time_check_ticks(
                self.place.as_deref(),
                c,
            )
            .is_some_and(|time| {
                let range = &c["value"];
                let bound = |key: &str| range.get(key).and_then(Value::as_number).map(int_value);
                match range {
                    Value::Number(n) => int_value(n) == time,
                    _ => {
                        bound("min").is_none_or(|min| time >= min)
                            && bound("max").is_none_or(|max| time <= max)
                    }
                }
            }),
            "minecraft:random_chance_with_enchanted_bonus" => {
                let level = self.attacker_level(c["enchantment"].as_str().unwrap_or(""), random);
                let chance = if level > 0 {
                    crate::enchant::level_value(&c["enchanted_chance"], level)
                } else {
                    c["unenchanted_chance"].as_f64().unwrap_or(0.0) as f32
                };
                random.next_float() < chance
            }
            // `this` with no checks (snow, chorus flower) holds when an entity
            // broke the block (LootItemEntityPropertyCondition needs the entity).
            "minecraft:entity_properties" => {
                let name = c["entity"].as_str().unwrap_or("");
                match self.named_entity(name) {
                    Some((target, matcher)) => matcher(&c["predicate"], target),
                    None => {
                        self.this_entity
                            && name == "this"
                            && c["predicate"].as_object().is_none_or(|p| p.is_empty())
                    }
                }
            }
            // Entity flags, equipment and damage-source predicates: none hold
            // for an unenchanted player kill without fire or lightning.
            "minecraft:damage_source_properties" => false,
            // `LocationCheck.test`: false without an origin. Unknown types fail, as in
            // loot.rs.
            "minecraft:location_check" => self.location.is_some_and(|at| at.check(c)),
            // A registered condition answers using the current roll context.
            k if k.contains(':') && !k.starts_with("minecraft:") => self
                .items
                .loot_bindings
                .condition(bindings, k, c)
                .is_some_and(|bound| bound.test(&self.roll_facts(), random, self.items)),
            _ => false,
        }
    }
    /// Expand entries; returns whether an entry succeeded (for alternatives).
    /// The last candidate field distinguishes emitted items from empty/deferred
    /// placeholders, without changing existing selection/modifier draws.
    fn expand<'v>(
        &self,
        e: &'v Value,
        random: &mut R,
        out: &mut Vec<(i32, crate::loot_candidate::LootCandidate, &'v Value, bool)>,
    ) -> bool {
        if !self.conditions(e, random) {
            return false;
        }
        match e["type"].as_str().unwrap_or("") {
            "minecraft:item" => {
                out.push((
                    e["weight"].as_number().map(int_value).unwrap_or(1) as i32,
                    crate::loot_candidate::LootCandidate::from_item(
                        e["name"].as_str().unwrap_or("minecraft:air"),
                        1,
                    ),
                    &e["modifier"],
                    true,
                ));
                true
            }
            "minecraft:alternatives" => {
                for child in e["children"].as_array().into_iter().flatten() {
                    if self.expand(child, random, out) {
                        return true;
                    }
                }
                false
            }
            "minecraft:group" => {
                for child in e["children"].as_array().into_iter().flatten() {
                    self.expand(child, random, out);
                }
                true
            }
            "minecraft:sequence" => {
                for child in e["children"].as_array().into_iter().flatten() {
                    if !self.expand(child, random, out) {
                        break;
                    }
                }
                true
            }
            // DynamicLoot: what the block entity adds (a cracked decorated
            // pot's sherds), filled in by `block_drops`.
            "minecraft:dynamic" => {
                out.push((
                    e["weight"].as_number().map(int_value).unwrap_or(1) as i32,
                    crate::loot_candidate::LootCandidate::from_item(
                        format!("{DYNAMIC}{}", e["name"].as_str().unwrap_or("")),
                        1,
                    ),
                    &Value::Null,
                    false,
                ));
                true
            }
            // NestedLootTable: chosen by its weight like an item; `choose`
            // then rolls its table (an id or inline) and applies its functions.
            "minecraft:loot_table" => {
                out.push((
                    e["weight"].as_number().map(int_value).unwrap_or(1) as i32,
                    crate::loot_candidate::LootCandidate::from_item(NESTED, 1),
                    e,
                    true,
                ));
                true
            }
            "minecraft:empty" => {
                out.push((
                    e["weight"].as_number().map(int_value).unwrap_or(1) as i32,
                    crate::loot_candidate::LootCandidate::from_item("minecraft:air", 0),
                    &Value::Null,
                    false,
                ));
                true
            }
            _ => false,
        }
    }
    fn choose(
        &self,
        entries: &[Value],
        random: &mut R,
        produced: &mut Vec<(crate::loot_candidate::LootCandidate, bool)>,
    ) {
        let mut candidates = Vec::new();
        for e in entries {
            self.expand(e, random, &mut candidates);
        }
        if candidates.is_empty() {
            return;
        }
        let pick = if candidates.len() == 1 {
            0
        } else {
            let total: i32 = candidates.iter().map(|c| c.0.max(0)).sum();
            let mut r = random.next_int_bound(total.max(1)).unwrap_or(0);
            let mut chosen = 0;
            for (i, c) in candidates.iter().enumerate() {
                r -= c.0.max(0);
                if r < 0 {
                    chosen = i;
                    break;
                }
            }
            chosen
        };
        let (_, mut stack, entry_or_modifier, emitted) = candidates.swap_remove(pick);
        if stack.item() == NESTED {
            self.nested(entry_or_modifier, random, produced);
            return;
        }
        self.modifiers(entry_or_modifier, &mut stack, random);
        produced.push((stack, emitted));
    }
    /// `NestedLootTable.createItemStack`: the entry's table rolls in full
    /// (its pools and its own functions), then the entry's functions apply to
    /// each stack. A table already rolling further out adds nothing.
    fn nested(
        &self,
        entry: &Value,
        random: &mut R,
        produced: &mut Vec<(crate::loot_candidate::LootCandidate, bool)>,
    ) {
        let (name, table) = match &entry["value"] {
            Value::String(name) => (name.as_str(), self.items.loot.get(name.as_str())),
            inline @ Value::Object(_) => ("<inline>", Some(inline)),
            _ => return,
        };
        let Some(table) = table else { return };
        let stacks = {
            let Some(_guard) = NestedRoll::enter(name) else {
                return;
            };
            self.evaluate_candidates(table, random)
        };
        for mut stack in stacks {
            self.modifiers(&entry["modifier"], &mut stack, random);
            produced.push((stack, true));
        }
    }
    /// An enchantment function's `options` (`HolderSet`): a tag, a list or
    /// one id; every enchantment when absent (`registryAccess().listElements`).
    fn enchant_pool(e: &crate::enchant::Enchantments, options: &Value) -> Vec<Arc<str>> {
        match options {
            Value::String(s) if s.starts_with('#') => e.tag(s).to_vec(),
            Value::String(s) => vec![Arc::from(s.as_str())],
            Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(Arc::from)).collect(),
            _ => e.list.iter().map(|x| Arc::clone(&x.id)).collect(),
        }
    }
    fn effective_custom_data<'s>(
        &'s self,
        stack: &'s ItemStack,
    ) -> Option<std::borrow::Cow<'s, sunrise_core::nbt::Compound>> {
        let prototype = self.items.component_defaults.get(stack.item.as_ref());
        if stack
            .data()
            .is_some_and(|data| data.other("!minecraft:custom_data").is_some())
        {
            None
        } else if let Some(data) = stack.custom_data() {
            Some(std::borrow::Cow::Borrowed(data))
        } else {
            prototype
                .and_then(|values| values.get(&"minecraft:custom_data".into()))
                .and_then(|value| sunrise_world::custom_data::borrow(value).ok())
        }
    }

    fn modifiers(
        &self,
        m: &Value,
        stack: &mut crate::loot_candidate::LootCandidate,
        random: &mut R,
    ) {
        let list = match m {
            Value::Array(a) => a.as_slice(),
            value => std::slice::from_ref(value),
        };
        for f in list {
            match f {
                // SequenceFunction composes each holder, including named modifiers.
                Value::String(id) => {
                    if let Some(modifier) = self.items.loot_modifiers.get(id) {
                        self.modifiers(modifier, stack, random);
                    }
                    continue;
                }
                Value::Object(_) => {}
                _ => continue,
            }
            if self
                .items
                .loot_bindings
                .conditional(f["type"].as_str().unwrap_or(""))
                && !self.conditions(f, random)
            {
                continue;
            }
            match f["type"].as_str().or(f["function"].as_str()).unwrap_or("") {
                "minecraft:sequence" => {
                    for function in f["functions"].as_array().into_iter().flatten() {
                        self.modifiers(function, stack, random);
                    }
                }
                "minecraft:copy_custom_data" => {
                    stack.with_resident_mut(self.items, |stack| {
                        let original = self.effective_custom_data(stack);
                        let place = sunrise_world::loot::current_native_loot_place();
                        if let Some(data) = sunrise_world::loot::copy::copy_custom_data(
                            f,
                            original.as_deref(),
                            |source| {
                                sunrise_world::loot::copy::nbt_source(
                                    source,
                                    place.as_deref(),
                                    |target| self.named_entity(target).map(|(entity, _)| *entity),
                                )
                            },
                        ) {
                            apply_component_patch(
                                stack,
                                [(
                                    "minecraft:custom_data".into(),
                                    sunrise_core::nbt::Tag::Compound(data),
                                )]
                                .into_iter()
                                .collect(),
                            );
                        }
                    });
                }
                "minecraft:set_custom_data" => {
                    // Keep flattened SNBT for the shared custom-data codec;
                    // JSON compounds use the same JsonOps conversion as world loot.
                    let encoded = match &f["tag"] {
                        Value::String(text) => {
                            Some(sunrise_core::nbt::Tag::String(text.as_str().into()))
                        }
                        value => sunrise_world::loot::nbt_predicate_tag(value),
                    };
                    let Some(encoded) = encoded else {
                        continue;
                    };
                    let Ok(tag) = sunrise_world::custom_data::borrow(&encoded) else {
                        continue;
                    };
                    stack.with_resident_mut(self.items, |stack| {
                        let mut data = self
                            .effective_custom_data(stack)
                            .map(std::borrow::Cow::into_owned)
                            .unwrap_or_default();
                        sunrise_world::nbt_path::merge_compound(&mut data, &tag);
                        let id = if data.is_empty() {
                            "!minecraft:custom_data"
                        } else {
                            "minecraft:custom_data"
                        };
                        apply_component_patch(
                            stack,
                            [(id.into(), sunrise_core::nbt::Tag::Compound(data))]
                                .into_iter()
                                .collect(),
                        );
                    });
                }
                "minecraft:copy_components" => {
                    let place = sunrise_world::loot::current_native_loot_place();
                    let mut patch = sunrise_core::nbt::Compound::new();
                    sunrise_world::loot::copy::copy_components(
                        f,
                        |source| match source {
                            "block_entity" => place
                                .as_ref()?
                                .block_components
                                .as_ref()
                                .map(std::borrow::Cow::Borrowed),
                            "tool" => self.held.map(|tool| {
                                std::borrow::Cow::Owned(
                                    sunrise_world::loot::copy::components_with_patch(
                                        self.items.component_defaults.get(tool.item.as_ref()),
                                        crate::item_data::component_snapshot(tool).as_ref(),
                                    ),
                                )
                            }),
                            target => self.named_entity(target).map(|(entity, _)| {
                                sunrise_world::loot::copy::entity_components(entity)
                            }),
                        },
                        |id, value| {
                            patch.insert(id.as_str().into(), value.clone());
                        },
                    );
                    if !patch.is_empty() {
                        stack.with_resident_mut(self.items, |stack| {
                            apply_component_patch(stack, patch);
                        });
                    }
                }
                "minecraft:filtered" => {
                    let pass = sunrise_world::loot::item_predicates::matches(
                        &f["item_filter"],
                        stack,
                        |registry, tag, id| match registry {
                            "item" => self.items.item_in_tag(tag, id),
                            "enchantment" => self.items.enchantments.in_tag(tag, id),
                            _ => false,
                        },
                    );
                    self.modifiers(
                        if pass { &f["on_pass"] } else { &f["on_fail"] },
                        stack,
                        random,
                    );
                }
                "minecraft:set_count" => {
                    let n = Self::int_number(&f["count"], random);
                    let base = if f["add"].as_bool() == Some(true) {
                        stack.count()
                    } else {
                        0
                    };
                    let total = base.saturating_add(n).max(0);
                    stack.set_signed_count(total);
                }
                // SetItemDamageFunction.run: check damageability before the
                // float provider, so ineligible stacks do not draw randomness.
                "minecraft:set_damage" => {
                    stack.with_resident_mut(self.items, |stack| {
                        let component = |id| stack.data().and_then(|data| data.other(id));
                        if stack.count == 0
                            || component("minecraft:unbreakable").is_some()
                            || component("!minecraft:max_damage").is_some()
                            || component("!minecraft:damage").is_some()
                        {
                            return;
                        }
                        let max = match component("minecraft:max_damage") {
                            Some(sunrise_core::nbt::Tag::Int(value)) => {
                                u32::try_from(*value).unwrap_or(0)
                            }
                            _ => self
                                .items
                                .get(&stack.item)
                                .map_or(0, |info| info.max_damage),
                        };
                        if max == 0 {
                            return;
                        }
                        let base = if f["add"].as_bool() == Some(true) {
                            1.0 - stack.damage as f32 / max as f32
                        } else {
                            0.0
                        };
                        let remaining = (Self::number(&f["damage"], random) + base).clamp(0.0, 1.0);
                        stack.damage = ((1.0 - remaining) * max as f32).floor() as u32;
                    });
                }
                "minecraft:apply_bonus" => {
                    let level = self.bonus_level(f["enchantment"].as_str().unwrap_or(""), random);
                    let count = stack.count();
                    let params = &f["parameters"];
                    let next = match f["formula"].as_str().unwrap_or("") {
                        "minecraft:ore_drops" if level > 0 => {
                            let bound = level.saturating_add(2);
                            let bonus = (random.next_int_bound(bound).unwrap_or(0) - 1).max(0);
                            count.saturating_mul(bonus + 1)
                        }
                        "minecraft:uniform_bonus_count" => {
                            let multiplier = params["bonusMultiplier"]
                                .as_number()
                                .map(int_value)
                                .unwrap_or(1) as i32;
                            let bound = multiplier.saturating_mul(level).saturating_add(1).max(1);
                            count.saturating_add(random.next_int_bound(bound).unwrap_or(0))
                        }
                        "minecraft:binomial_with_bonus_count" => {
                            let (extra, p) = (
                                params["extra"].as_number().map(int_value).unwrap_or(0) as i32,
                                params["probability"].as_f64().unwrap_or(0.0) as f32,
                            );
                            let mut n = count;
                            for _ in 0..level.saturating_add(extra) {
                                if random.next_float() < p {
                                    n = n.saturating_add(1);
                                }
                            }
                            n
                        }
                        _ => count,
                    }
                    .max(0);
                    stack.set_signed_count(next);
                }
                "minecraft:enchanted_count_increase" => {
                    // Attacker's level x count, rounded (looting).
                    let level =
                        self.attacker_level(f["enchantment"].as_str().unwrap_or(""), random);
                    if level > 0 {
                        let add = level as f32 * Self::number(&f["count"], random);
                        // ItemStack.grow: an int sum, which wraps as in Java.
                        let mut count = stack.count().wrapping_add(add.round() as i32).max(0);
                        if let Some(limit) =
                            f["limit"].as_number().map(int_value).filter(|l| *l > 0)
                        {
                            count = count.min(limit.min(i64::from(i32::MAX)) as i32);
                        }
                        stack.set_signed_count(count);
                    }
                }
                // CopyBlockState: the listed properties of the broken block
                // become the item's `block_state` component.
                "minecraft:copy_state" => {
                    stack.with_resident_mut(self.items, |stack| {
                        use sunrise_core::nbt::Tag;
                        let mut copied = match stack.data().and_then(|d| d.other(BLOCK_STATE)) {
                            Some(Tag::Compound(c)) => c.clone(),
                            _ => Default::default(),
                        };
                        for name in f["properties"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                        {
                            if let Some(value) = self.properties.get(name) {
                                copied.insert(name.into(), Tag::String(value.as_str().into()));
                            }
                        }
                        if !copied.is_empty() {
                            stack
                                .data_mut()
                                .set_other(BLOCK_STATE, Some(Tag::Compound(copied)));
                        }
                    });
                }
                // ApplyExplosionDecay: each item survives with 1 / radius.
                "minecraft:explosion_decay" => {
                    if let Some(radius) = self.explosion_radius {
                        let probability = 1.0 / radius;
                        let kept = (0..stack.count().max(0))
                            .filter(|_| random.next_float() <= probability)
                            .count() as i32;
                        stack.set_signed_count(kept);
                    }
                }
                // SetPotionFunction (the barter's fire resistance potions).
                "minecraft:set_potion" => {
                    if let Some(id) = f["id"].as_str() {
                        stack.with_resident_mut(self.items, |stack| {
                            *stack = std::mem::replace(stack, ItemStack::new("minecraft:air", 0))
                                .with_potion(id);
                        });
                    }
                }
                // EnchantWithLevelsFunction: EnchantmentHelper.enchantItem on
                // the context's random (the End cities' gear).
                "minecraft:enchant_with_levels" => {
                    let cost = Self::int_number(&f["levels"], random);
                    let e = &self.items.enchantments;
                    let pool = Self::enchant_pool(e, &f["options"]);
                    stack.with_resident_mut(self.items, |stack| {
                        let chosen = e.select(self.items, random, stack, cost, &pool);
                        // EnchantmentHelper.enchantItem: a book becomes one
                        // enchanted book (the fresh stack's count).
                        if &*stack.item == "minecraft:book" {
                            *stack = ItemStack::new("minecraft:enchanted_book", 1);
                        }
                        for (id, level) in chosen {
                            stack.enchant(&id, level);
                        }
                    });
                }
                // EnchantRandomlyFunction: one enchantment the item accepts,
                // at a level between its min and max.
                "minecraft:enchant_randomly" => {
                    stack.with_resident_mut(self.items, |stack| {
                        let e = &self.items.enchantments;
                        let book = &*stack.item == "minecraft:book";
                        let only_compatible = f["only_compatible"].as_bool().unwrap_or(true);
                        let options: Vec<Arc<str>> = Self::enchant_pool(e, &f["options"])
                            .into_iter()
                            .filter(|id| {
                                book || !only_compatible
                                    || e.can_enchant(self.items, id, &stack.item)
                            })
                            .collect();
                        if options.is_empty() {
                            return;
                        }
                        let id = options
                            [random.next_int_bound(options.len() as i32).unwrap_or(0) as usize]
                            .clone();
                        let max = e.get(&id).map_or(1, |d| d.max_level).max(1);
                        // `Mth.nextInt(random, 1, max)`: a single-level
                        // enchantment (Mending) returns 1 without drawing, so the
                        // rolls after this one keep vanilla's sequence.
                        let level = if max <= 1 {
                            1
                        } else {
                            1 + random.next_int_bound(max as i32).unwrap_or(0) as u32
                        };
                        if book {
                            *stack = ItemStack::new("minecraft:enchanted_book", 1);
                        }
                        stack.enchant(&id, level);
                    });
                }
                "minecraft:limit_count" => {
                    let limit = &f["limit"];
                    let mut count = stack.count();
                    if let Some(min) = limit.get("min").filter(|value| !value.is_null()) {
                        count = count.max(Self::int_number(min, random));
                    }
                    if let Some(max) = limit.get("max").filter(|value| !value.is_null()) {
                        count = count.min(Self::int_number(max, random));
                    }
                    stack.set_signed_count(count.max(0));
                }
                // SetNameFunction.run: the component the target names
                // (custom_name by default), set as a chest's roll sets it. As in
                // the main loot engine, an `entity` resolver is not modelled.
                "minecraft:set_name" if !f["name"].is_null() => {
                    let target = f["target"].as_str().unwrap_or("custom_name");
                    let id = format!("minecraft:{target}");
                    stack.with_resident_mut(self.items, |stack| {
                        stack.data_mut().set_loot_name(&id, &f["name"]);
                        stack.tidy();
                    });
                }
                "minecraft:set_components" => {
                    stack.with_resident_mut(self.items, |stack| {
                        apply_components(stack, &f["components"]);
                    });
                }
                k if k.contains(':') && !k.starts_with("minecraft:") => {
                    if let Some(bound) =
                        self.items.loot_bindings.function(self.local_bindings, k, f)
                    {
                        let facts = self.roll_facts();
                        bound.apply(&facts, random, self.items, stack);
                    }
                }
                _ => {}
            }
        }
    }
}

/// `SetComponentsFunction.run`: `ItemStack.applyComponents` with the patch
/// (`!id` removes a component), read back as a saved stack's components are,
/// so a modelled one (a potion's contents, a book's pages) lands in its field.
fn apply_components(stack: &mut ItemStack, patch: &Value) {
    let patch = patch
        .as_object()
        .into_iter()
        .flatten()
        .map(|(id, value)| {
            if value.is_null() {
                (
                    format!("!{}", id.strip_prefix('!').unwrap_or(id))
                        .as_str()
                        .into(),
                    sunrise_core::nbt::Tag::Compound(Default::default()),
                )
            } else {
                (
                    id.as_str().into(),
                    sunrise_game_data::scoreboard::json_to_tag(value),
                )
            }
        })
        .collect();
    apply_component_patch(stack, patch);
}

/// One complete patch enters the component codec together, preserving component
/// interactions (including the ominous banner) and the source's NBT types.
fn apply_component_patch(stack: &mut ItemStack, patch: sunrise_core::nbt::Compound) {
    use sunrise_core::nbt::Tag;
    let mut saved = match crate::item_data::component_snapshot(stack) {
        Some(Tag::Compound(c)) => c,
        _ => Default::default(),
    };
    for (key, value) in patch {
        let key = key.to_string_lossy();
        let (removed, id) = key
            .strip_prefix('!')
            .map_or((false, key.as_str()), |id| (true, id));
        let id = if id.contains(':') {
            id.to_owned()
        } else {
            format!("minecraft:{id}")
        };
        saved.remove(&id.as_str().into());
        saved.remove(&format!("!{id}").as_str().into());
        if removed {
            saved.insert(
                format!("!{id}").as_str().into(),
                Tag::compound(Vec::<(&str, Tag)>::new()),
            );
        } else {
            saved.insert(id.as_str().into(), value);
        }
    }
    (stack.damage, stack.extra) = crate::item_data::decode_components(Some(&Tag::Compound(saved)));
    stack.tidy();
}

/// `prototype_has` over `items`: whether an item's prototype carries a
/// component, or `None` for an item `items` doesn't know.
pub(crate) fn prototypes_of(items: &Items) -> impl Fn(&str, &str) -> Option<bool> + '_ {
    |item, id| {
        items
            .get(item)
            .map(|info| info.components.iter().any(|c| c == id))
    }
}

/// `DispensibleContainerItem`: shared contents for hand use and dispensers.
pub fn bucket_contents(
    item: &str,
) -> Option<(sunrise_world::level::BucketContent, &str, Option<&str>)> {
    use sunrise_world::{fluid::FluidKind, level::BucketContent as C};
    // A bucket of a fluid empties with that fluid's sound.
    let fluid = |kind| {
        Some((
            C::Fluid(kind),
            sunrise_world::level::fluid_empty_sound(kind)?,
            None,
        ))
    };
    Some(match item {
        _ if !item.starts_with("minecraft:") => fluid(FluidKind::Mod(
            sunrise_world::fluid::mod_fluid_of_bucket(item)?,
        ))?,
        "minecraft:water_bucket" => fluid(FluidKind::Water)?,
        "minecraft:lava_bucket" => fluid(FluidKind::Lava)?,
        "minecraft:powder_snow_bucket" => {
            (C::Powder, "minecraft:item.bucket.empty_powder_snow", None)
        }
        "minecraft:cod_bucket" => (C::MobWater, "minecraft:item.bucket.empty_fish", Some("cod")),
        "minecraft:salmon_bucket" => (
            C::MobWater,
            "minecraft:item.bucket.empty_fish",
            Some("salmon"),
        ),
        "minecraft:pufferfish_bucket" => (
            C::MobWater,
            "minecraft:item.bucket.empty_fish",
            Some("pufferfish"),
        ),
        "minecraft:tropical_fish_bucket" => (
            C::MobWater,
            "minecraft:item.bucket.empty_fish",
            Some("tropical_fish"),
        ),
        "minecraft:axolotl_bucket" => (
            C::MobWater,
            "minecraft:item.bucket.empty_axolotl",
            Some("axolotl"),
        ),
        "minecraft:tadpole_bucket" => (
            C::MobWater,
            "minecraft:item.bucket.empty_tadpole",
            Some("tadpole"),
        ),
        "minecraft:sulfur_cube_bucket" => (
            C::Nothing,
            "minecraft:item.bucket.empty_sulfur_cube",
            Some("sulfur_cube"),
        ),
        _ => return None,
    })
}

/// One `MobEffectInstance` of an `apply_effects` consume effect (its codec's defaults).
/// `DeathProtection.CODEC`: the `death_effects` of a `minecraft:death_protection`
/// component, in order (`applyEffects` runs each one). A missing or empty
/// list, as in `death_protection={}`, has none.
pub fn parse_death_effects(
    component: &Value,
) -> std::result::Result<Vec<ConsumeEffect>, sunrise_core::json_nbt::JsonNbtError> {
    parse_consumable(&serde_json::json!({"on_consume_effects": component.get("death_effects").cloned().unwrap_or_else(|| serde_json::json!([]))})).map(|value| value.effects)
}

#[cfg(test)]
mod death_effects_tests {
    use super::*;
    #[test]
    fn the_totems_component_reads_clear_then_three_effects() {
        let totem = serde_json::json!({"death_effects": [
            {"type": "minecraft:clear_all_effects"},
            {"type": "minecraft:apply_effects", "effects": [
                {"amplifier": 1, "duration": 900, "id": "minecraft:regeneration", "show_icon": true},
                {"amplifier": 1, "duration": 100, "id": "minecraft:absorption", "show_icon": true},
                {"duration": 800, "id": "minecraft:fire_resistance", "show_icon": true}
            ]}
        ]});
        let effects = parse_death_effects(&totem).unwrap();
        assert_eq!(effects.len(), 2);
        assert_eq!(effects[0], ConsumeEffect::ClearAllEffects);
        let ConsumeEffect::ApplyEffects {
            effects,
            probability,
        } = &effects[1]
        else {
            panic!("second effect is apply_effects");
        };
        assert_eq!(*probability, 1.0);
        let ids: Vec<_> = effects
            .iter()
            .map(|e| (e.id.as_str(), e.duration, e.amplifier))
            .collect();
        assert_eq!(
            ids,
            [
                ("minecraft:regeneration", 900, 1),
                ("minecraft:absorption", 100, 1),
                ("minecraft:fire_resistance", 800, 0)
            ]
        );
    }
    #[test]
    fn an_empty_component_has_no_effects() {
        assert!(
            parse_death_effects(&serde_json::json!({}))
                .unwrap()
                .is_empty()
        );
    }
}

fn consume_effect_spec(e: &Tag) -> Option<ConsumeEffectSpec> {
    let Tag::String(id) = e.get("id")? else {
        return None;
    };
    let flag = |key, default| {
        e.get(key)
            .and_then(crate::item_data::component_bool)
            .unwrap_or(default)
    };
    let number = |key| {
        e.get(key)
            .and_then(crate::properties::component_int)
            .unwrap_or(0)
    };
    let show_particles = flag("show_particles", true);
    Some(ConsumeEffectSpec {
        id: id.to_string_lossy(),
        duration: number("duration"),
        // `MobEffectInstance.Details.MAP_CODEC` reads `ExtraCodecs.UNSIGNED_BYTE`.
        amplifier: i32::from(number("amplifier") as u8),
        ambient: flag("ambient", false),
        show_particles,
        show_icon: flag("show_icon", show_particles),
    })
}

#[cfg(test)]
mod bucket_use_tests {
    use super::*;
    #[test]
    fn every_filled_bucket_has_the_native_content_and_sound() {
        use sunrise_world::level::BucketContent as C;
        assert_eq!(
            bucket_contents("minecraft:powder_snow_bucket"),
            Some((C::Powder, "minecraft:item.bucket.empty_powder_snow", None))
        );
        assert_eq!(
            bucket_contents("minecraft:tadpole_bucket"),
            Some((
                C::MobWater,
                "minecraft:item.bucket.empty_tadpole",
                Some("tadpole")
            ))
        );
        assert_eq!(
            bucket_contents("minecraft:sulfur_cube_bucket"),
            Some((
                C::Nothing,
                "minecraft:item.bucket.empty_sulfur_cube",
                Some("sulfur_cube")
            ))
        );
        for id in ["cod", "salmon", "pufferfish", "tropical_fish", "axolotl"] {
            assert!(bucket_contents(&format!("minecraft:{id}_bucket")).is_some());
        }
        assert!(bucket_contents("minecraft:bucket").is_none());
        assert!(bucket_contents("minecraft:milk_bucket").is_none());
    }
}

/// The items of the prepared resources, when they are on this machine, for
/// tests: the block registry and facts as the client's `Assets::load` reads them.
#[cfg(test)]
pub(crate) fn prepared_items() -> Option<Items> {
    let root = std::env::var_os("SUNRISE_TEST_RESOURCES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/sunrise-client")
                .join(sunrise_core::VANILLA_VERSION)
        });
    if !root.join("items.json").exists() {
        return None;
    }
    let (registry, _) =
        BlockRegistry::from_facts_json_with_flags(fs::File::open(root.join("facts.json")).ok()?)
            .ok()?;
    let facts = BlockFacts::load(&root, &registry).ok()?;
    let defaults = sunrise_world::item_defaults::read(&root).ok()?;
    Items::load(&root, &registry, &facts, &PackOverlay::NONE, &defaults).ok()
}

#[cfg(test)]
mod consume_effect_tests {
    use super::*;

    #[test]
    fn effect_details_decode_byte_numbers_and_boolean_defaults() {
        let quiet = Tag::compound([
            ("id", Tag::String("minecraft:haste".into())),
            ("duration", Tag::Byte(100)),
            ("amplifier", Tag::Byte(-56)),
            ("show_particles", Tag::Byte(0)),
        ]);
        let s = consume_effect_spec(&quiet).unwrap();
        assert_eq!(
            (s.id.as_str(), s.duration, s.amplifier),
            ("minecraft:haste", 100, 200)
        );
        assert!(!s.ambient && !s.show_particles && !s.show_icon);
        let plain = Tag::compound([("id", Tag::String("minecraft:regeneration".into()))]);
        let plain = consume_effect_spec(&plain).unwrap();
        assert!(!plain.ambient && plain.show_particles && plain.show_icon);
    }

    #[test]
    fn saved_consume_effects_reject_out_of_range_parameters() {
        for (kind, field, invalid) in [
            ("apply_effects", "probability", 1.5_f32),
            ("teleport_randomly", "diameter", 0.0),
            ("teleport_randomly", "diameter", f32::NAN),
        ] {
            let tag = Tag::compound([
                ("type", Tag::String(kind.into())),
                (field, Tag::Float(invalid.to_bits())),
            ]);
            let mut raw = Tag::compound([("on_consume_effects", Tag::List(vec![tag]))]);
            assert!(
                stack_effects(&mut raw, "on_consume_effects", None, None)
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn invalid_native_consume_payload_refuses_component() {
        let bad =
            serde_json::json!({"on_consume_effects": [{"type": "mymod:charge", "data": null}]});
        assert!(parse_consumable(&bad).is_err());
        let good =
            serde_json::json!({"on_consume_effects": [{"type": "mymod:charge", "amount": 42}]});
        let value = parse_consumable(&good).unwrap();
        let copy = value.clone();
        let (ConsumeEffect::Native(a), ConsumeEffect::Native(b)) =
            (&value.effects[0], &copy.effects[0])
        else {
            panic!("native payload")
        };
        assert!(Arc::ptr_eq(&a.data, &b.data));
    }

    #[test]
    fn a_mods_own_consume_effect_kind_is_kept() {
        let effect = |value: Value| {
            parse_consumable(&serde_json::json!({"on_consume_effects": [value]}))
                .unwrap()
                .effects
                .pop()
        };
        let native = effect(serde_json::json!({"type": "mymod:energy_boost", "amount": 42}));
        assert!(matches!(&native, Some(ConsumeEffect::Native(effect))
            if effect.kind.as_ref() == "mymod:energy_boost" && effect.data.get("amount") == Some(&Tag::Byte(42))));
        // A bare or empty namespace resolves to minecraft's own type.
        for kind in ["apply_effects", "minecraft:apply_effects", ":apply_effects"] {
            assert!(matches!(
                effect(serde_json::json!({
                    "type": kind,
                    "effects": [{"id": "minecraft:speed", "duration": 100}],
                })),
                Some(ConsumeEffect::ApplyEffects { .. })
            ));
        }
        assert!(effect(serde_json::json!({"type": "minecraft:future"})).is_none());
        assert!(effect(serde_json::json!({"amount": 42})).is_none());
    }

    #[test]
    fn play_sound_consume_effect_uses_lenient_optional_range() {
        let sound_effect = |s: &str| {
            let tag = sunrise_core::json_nbt::parse_json_tag(s).unwrap();
            builtin_consume_effect("play_sound", &tag)
        };
        assert_eq!(
            sound_effect(r#"{"type":"minecraft:play_sound","sound":"minecraft:entity.fox.eat"}"#),
            Some(ConsumeEffect::PlaySound(ConsumeSound::Registered(
                "minecraft:entity.fox.eat".into(),
            )))
        );
        assert_eq!(
            sound_effect(
                r#"{"type":"minecraft:play_sound","sound":{"sound_id":"demo:chime","range":24.5}}"#,
            ),
            Some(ConsumeEffect::PlaySound(ConsumeSound::Direct {
                id: "demo:chime".into(),
                range: Some(24.5),
            }))
        );
        assert_eq!(
            sound_effect(r#"{"type":"minecraft:play_sound","sound":{"sound_id":"demo:chime"}}"#),
            Some(ConsumeEffect::PlaySound(ConsumeSound::Direct {
                id: "demo:chime".into(),
                range: None,
            }))
        );
        assert_eq!(
            sound_effect(
                r#"{"type":"minecraft:play_sound","sound":{"sound_id":"demo:chime","range":"oops"}}"#,
            ),
            Some(ConsumeEffect::PlaySound(ConsumeSound::Direct {
                id: "demo:chime".into(),
                range: None,
            }))
        );
    }

    #[test]
    fn unregistered_native_effect_keeps_small_numbers_in_item_prototype() {
        // The item loader's path: JSON components -> NBT -> decoded stack data.
        let components = serde_json::json!({"minecraft:consumable": {"on_consume_effects": [
            {"type": "mymod:set_fire", "seconds": 15},
            {"type": "mymod:give_item", "item": "mymod:cheese_ingot", "count": 3},
            {"type": "mymod:wide", "short": 300, "flag": false},
        ]}});
        let prototype = sunrise_core::json_nbt::json_to_tag(&components).unwrap();
        let (damage, extra) = crate::item_data::decode_components(Some(&prototype));
        let stack = ItemStack {
            item: "mymod:magma_bottle".into(),
            count: 1,
            damage,
            extra,
        };
        let consumable = stack.consumable(None).expect("consumable");
        let field = |i: usize, key: &str| match &consumable.effects[i] {
            ConsumeEffect::Native(effect) => effect.data.get(key).cloned(),
            other => panic!("expected a native effect, got {other:?}"),
        };
        assert_eq!(field(0, "seconds"), Some(Tag::Byte(15)));
        assert_eq!(field(1, "count"), Some(Tag::Byte(3)));
        assert_eq!(
            field(1, "item"),
            Some(Tag::String("mymod:cheese_ingot".into()))
        );
        assert_eq!(field(2, "short"), Some(Tag::Short(300)));
        assert_eq!(field(2, "flag"), Some(Tag::Byte(0)));
    }

    fn saved_stack(effects: Vec<Tag>) -> ItemStack {
        let saved = Tag::compound([(
            "minecraft:consumable",
            Tag::compound([("on_consume_effects", Tag::List(effects))]),
        )]);
        let (damage, extra) = crate::item_data::decode_components(Some(&saved));
        ItemStack {
            item: "mymod:snack".into(),
            count: 1,
            damage,
            extra,
        }
    }

    #[test]
    fn unregistered_native_effect_keeps_a_saved_float_exactly() {
        let stack = saved_stack(vec![Tag::compound([
            ("type", Tag::String("mymod:chill".into())),
            ("strength", Tag::Float(0.1f32.to_bits())),
        ])]);
        let consumable = stack.consumable(None).expect("consumable");
        let [ConsumeEffect::Native(effect)] = consumable.effects.as_slice() else {
            panic!("expected one native effect, got {:?}", consumable.effects)
        };
        assert_eq!(
            effect.data.get("strength"),
            Some(&Tag::Float(0.1f32.to_bits()))
        );
    }

    #[test]
    fn unregistered_native_effect_with_a_lone_surrogate_keeps_its_siblings() {
        let lone = sunrise_core::nbt::JavaString(vec![0x61, 0xD800, 0x62]);
        let stack = saved_stack(vec![
            Tag::compound([
                ("type", Tag::String("mymod:named".into())),
                ("name", Tag::String(lone.clone())),
            ]),
            Tag::compound([("type", Tag::String("minecraft:clear_all_effects".into()))]),
        ]);
        let data = stack.data().expect("saved data");
        assert!(!data.refused_consume_effect_component("minecraft:consumable"));
        let consumable = stack.consumable(None).expect("consumable");
        let [
            ConsumeEffect::Native(effect),
            ConsumeEffect::ClearAllEffects,
        ] = consumable.effects.as_slice()
        else {
            panic!(
                "expected the custom and built-in effects, got {:?}",
                consumable.effects
            )
        };
        assert_eq!(effect.kind.as_ref(), "mymod:named");
        assert_eq!(effect.data.get("name"), Some(&Tag::String(lone)));
    }
}

#[cfg(test)]
mod block_tag_flag_tests {
    use super::*;

    /// The table's own rule: a state carries each flag whose tag lists its
    /// name, a block in two tags carries both, and an id the table has no
    /// name for carries none.
    #[test]
    fn build_marks_the_states_of_every_tags_members() {
        let tags = HashMap::from([
            (
                "minecraft:climbable".to_owned(),
                HashSet::from(["minecraft:ladder".to_owned(), "minecraft:vine".to_owned()]),
            ),
            (
                // A fire block cats may also lie on.
                "minecraft:fire".to_owned(),
                HashSet::from(["minecraft:fire".to_owned()]),
            ),
            (
                "minecraft:cats_can_lie_on".to_owned(),
                HashSet::from(["minecraft:fire".to_owned()]),
            ),
            ("minecraft:cauldrons".to_owned(), HashSet::new()),
        ]);
        let names: Arc<[Arc<str>]> = [
            "minecraft:ladder",
            "minecraft:fire",
            "minecraft:vine",
            "minecraft:stone",
        ]
        .iter()
        .map(|name| Arc::<str>::from(*name))
        .collect();
        let flags = BlockTagFlags::build(&tags, names);
        assert_eq!(flags.flags(BlockStateId(0)), BlockTagFlags::CLIMBABLE);
        assert_eq!(
            flags.flags(BlockStateId(1)),
            BlockTagFlags::FIRE | BlockTagFlags::CATS_CAN_LIE_ON
        );
        assert_eq!(flags.flags(BlockStateId(2)), BlockTagFlags::CLIMBABLE);
        assert_eq!(flags.flags(BlockStateId(3)), 0);
        assert!(flags.has(BlockStateId(0), BlockTagFlags::CLIMBABLE));
        assert!(!flags.has(BlockStateId(3), BlockTagFlags::CLIMBABLE));
        // No name for this id: no flags, and no panic.
        assert_eq!(flags.flags(BlockStateId(4)), 0);
        assert!(!flags.has(BlockStateId(u32::MAX), BlockTagFlags::FIRE));
        assert_eq!(flags.bits(), &[1u16, 18, 1, 0][..]);
        assert_eq!(&*flags.state_names()[2], "minecraft:vine");
    }

    /// Every state's flags are exactly what the tag map answers for its name:
    /// `has(id, flag)` equals `block_tags[tag].contains(name)` for every state
    /// and every flag. This is the equivalence the mob code relies on when it
    /// tests a bit where it used to hash the tag and the block name.
    ///
    /// Needs the prepared resources; skipped without them like the crate's
    /// other resource tests (`prepared_items`).
    #[test]
    fn every_states_flags_match_the_tag_map() {
        let Some(items) = prepared_items() else {
            return;
        };
        let names = items.block_state_names();
        assert!(!names.is_empty(), "a loaded catalog names its states");
        let mut seen = 0u16;
        for (id, name) in names.iter().enumerate() {
            let state = BlockStateId(id as u32);
            for (flag, tag) in BlockTagFlags::ALL {
                let expected = items
                    .block_tags
                    .get(tag)
                    .is_some_and(|members| members.contains(&**name));
                assert_eq!(
                    items.block_tag_flags.has(state, flag),
                    expected,
                    "{name} / {tag}"
                );
                if expected {
                    seen |= flag;
                }
            }
        }
        // A tag with members must reach at least one state: an empty table
        // fails here instead of answering false everywhere in silence.
        for (flag, tag) in BlockTagFlags::ALL {
            if items.block_tags.get(tag).is_some_and(|m| !m.is_empty()) {
                assert!(
                    seen & flag != 0,
                    "{tag} has members but no state carries it"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    mod enchant_select_tests {
        include!("enchant_select_tests.rs");
    }
    use super::*;
    use serde_json::json;
    use sunrise_core::random::LegacyRandom;

    /// The ids `/loot ... loot <id>` takes: a content mod's table in the
    /// root and a world pack's table over it.
    #[test]
    fn weapon_damage_comes_from_the_stack() {
        let items = Items::for_tests(HashMap::new());
        let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
        sword.data_mut().set_other(
            "minecraft:weapon",
            Some(sunrise_core::nbt::Tag::compound([(
                "item_damage_per_attack",
                sunrise_core::nbt::Tag::Int(3),
            )])),
        );
        assert_eq!(items.weapon_damage(&sword), Some(3));
        sword.data_mut().set_other(
            "!minecraft:weapon",
            Some(sunrise_core::nbt::Tag::compound(Vec::<(
                &str,
                sunrise_core::nbt::Tag,
            )>::new())),
        );
        assert_eq!(items.weapon_damage(&sword), None);
    }

    #[test]
    fn weapon_disable_blocking_comes_from_the_stack() {
        let root = std::env::temp_dir().join(format!(
            "sunrise-weapon-disable-blocking-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("items.json"),
            r#"{"minecraft:stick":{},"minecraft:diamond_axe":{"minecraft:weapon":{"disable_blocking_for_seconds":5.0}}}"#,
        )
        .unwrap();
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let items = Items::catalog(&root, &PackOverlay::NONE, &defaults).unwrap();
        let mut axe = ItemStack::new("minecraft:diamond_axe", 1);
        assert_eq!(items.weapon_disable_blocking(&axe), 5.0);
        for item in ["minecraft:stick", "test:unknown", "minecraft:air"] {
            assert_eq!(items.weapon_disable_blocking(&ItemStack::new(item, 1)), 0.0);
        }

        // A replacement component uses the codec's zero default, not the axe's.
        let empty = crate::item_data::command_components("[weapon={}]").unwrap();
        (axe.damage, axe.extra) = crate::item_data::decode_components(Some(&empty));
        assert_eq!(items.weapon_disable_blocking(&axe), 0.0);

        let custom =
            crate::item_data::command_components("[weapon={disable_blocking_for_seconds:1.5f}]")
                .unwrap();
        (axe.damage, axe.extra) = crate::item_data::decode_components(Some(&custom));
        assert_eq!(items.weapon_disable_blocking(&axe), 1.5);
        let mut stick = ItemStack::new("minecraft:stick", 1);
        (stick.damage, stick.extra) = crate::item_data::decode_components(Some(&custom));
        assert_eq!(items.weapon_disable_blocking(&stick), 1.5);
        stick.count = 0;
        assert_eq!(items.weapon_disable_blocking(&stick), 0.0);
        stick.count = 1;
        stick.item = "minecraft:air".into();
        assert_eq!(items.weapon_disable_blocking(&stick), 0.0);

        axe.data_mut().set_other(
            "!minecraft:weapon",
            Some(sunrise_core::nbt::Tag::compound(Vec::<(
                &str,
                sunrise_core::nbt::Tag,
            )>::new())),
        );
        assert_eq!(items.weapon_disable_blocking(&axe), 0.0);
        let removal = crate::item_data::command_components("[!weapon]").unwrap();
        (axe.damage, axe.extra) = crate::item_data::decode_components(Some(&removal));
        let restored = crate::item_nbt::item_from_nbt(&crate::item_nbt::item_to_nbt(
            &axe,
            &Default::default(),
        ))
        .unwrap();
        assert_eq!(items.weapon_disable_blocking(&restored), 0.0);
        fs::remove_dir_all(root).unwrap();
    }

    /// One effective component for every reader: a bundle that only inherits its prototype's
    /// contents spills them on destruction, selects and extracts a non-first entry, and clearing
    /// the last slot of a command edit leaves an effective empty component.
    #[test]
    fn inherited_bundle_contents_are_the_effective_component_everywhere() {
        use crate::bundle::{Contents, contents_of, destroyed, set_contents, toggle_selected};

        let root =
            std::env::temp_dir().join(format!("sunrise-bundle-inherited-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("items.json"),
            r#"{
                "minecraft:blue_bundle": {"minecraft:max_stack_size": 1, "minecraft:bundle_contents": [{"id": "minecraft:stone", "count": 8}, {"id": "minecraft:dirt", "count": 4}]},
                "minecraft:stone": {"minecraft:max_stack_size": 64},
                "minecraft:dirt": {"minecraft:max_stack_size": 64}
            }"#,
        )
        .unwrap();
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let items = Items::catalog(&root, &PackOverlay::NONE, &defaults).unwrap();
        fs::remove_dir_all(root).unwrap();
        let counts = |bundle: &ItemStack| -> Vec<(String, u32)> {
            contents_of(&items, bundle)
                .unwrap()
                .iter()
                .map(|s| (s.item.to_string(), s.count))
                .collect()
        };

        // Selecting a non-first inherited entry needs no override, and removal takes it.
        let mut bundle = ItemStack::new("minecraft:blue_bundle", 1);
        toggle_selected(&items, &mut bundle, 1);
        let mut contents = Contents::of(&items, &bundle).unwrap();
        assert_eq!(contents.selected(), 1);
        let taken = contents.remove_one(&items).unwrap();
        assert_eq!((&*taken.item, taken.count), ("minecraft:dirt", 4));
        // An index outside the effective contents selects nothing.
        toggle_selected(&items, &mut bundle, 2);
        assert_eq!(bundle.data().unwrap().bundle_selected.0, -1);

        // A command edit of the middle of the list keeps the inherited entries it never touched,
        // and clearing the final slot does not show the prototype again.
        let mut edited = ItemStack::new("minecraft:blue_bundle", 1);
        let mut inside: Vec<_> = contents_of(&items, &edited).unwrap().to_vec();
        inside[1].count = 2;
        set_contents(&items, &mut edited, inside);
        assert_eq!(
            counts(&edited),
            [("minecraft:stone".into(), 8), ("minecraft:dirt".into(), 2)]
        );
        set_contents(&items, &mut edited, Vec::new());
        assert!(counts(&edited).is_empty());

        // Destruction spills inherited contents and writes an effective empty component.
        let mut dropped = ItemStack::new("minecraft:blue_bundle", 1);
        let spilled = destroyed(&items, &mut dropped);
        assert_eq!(spilled.iter().map(|s| s.count).collect::<Vec<_>>(), [8, 4]);
        assert!(counts(&dropped).is_empty());
        assert!(destroyed(&items, &mut dropped).is_empty());
        // Explicitly empty and removed contents spill nothing.
        let mut removed = ItemStack::new("minecraft:blue_bundle", 1);
        removed.data_mut().set_other(
            "!minecraft:bundle_contents",
            Some(sunrise_core::nbt::Tag::compound(Vec::<(
                &str,
                sunrise_core::nbt::Tag,
            )>::new())),
        );
        assert!(destroyed(&items, &mut removed).is_empty());
    }

    #[test]
    fn bundle_weight_refuses_prototype_cycles_without_masking_stack_patches() {
        use crate::bundle::{Contents, stack_bar, tooltip_layout};
        use sunrise_core::nbt::Tag;

        let root =
            std::env::temp_dir().join(format!("sunrise-bundle-cycles-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("items.json"),
            r#"{
                "minecraft:black_bundle": {"minecraft:max_stack_size": 1, "minecraft:bundle_contents": [{"id": "minecraft:black_bundle"}]},
                "minecraft:white_bundle": {"minecraft:max_stack_size": 1, "minecraft:bundle_contents": [{"id": "minecraft:orange_bundle"}]},
                "minecraft:orange_bundle": {"minecraft:max_stack_size": 1, "minecraft:bundle_contents": [{"id": "minecraft:white_bundle"}]},
                "minecraft:blue_bundle": {"minecraft:max_stack_size": 1, "minecraft:bundle_contents": [{"id": "minecraft:stone", "count": 16}]},
                "minecraft:stone": {"minecraft:max_stack_size": 64}
            }"#,
        )
        .unwrap();
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let items = Items::catalog(&root, &PackOverlay::NONE, &defaults).unwrap();
        fs::remove_dir_all(root).unwrap();

        let stone = ItemStack::new("minecraft:stone", 1);
        let mut empty = Contents::from_items(&items, Vec::new()).unwrap();
        for id in ["minecraft:black_bundle", "minecraft:white_bundle"] {
            let mut cyclic = ItemStack::new(id, 1);
            assert!(Contents::of(&items, &cyclic).is_none());
            assert_eq!(Contents::room_in_stack(&items, &cyclic, &stone), None);
            assert!(tooltip_layout(&items, &cyclic, 0.0, 0.0, 96.0, 2).is_none());
            let bar = stack_bar(&items, &cyclic).unwrap();
            assert_eq!((bar.width, bar.color), (13, 0xFF5454));
            assert_eq!(empty.try_insert(&items, &mut cyclic, true), 0);
            assert_eq!(cyclic.count, 1);
        }
        assert!(empty.items().is_empty());

        let mut child = ItemStack::new("minecraft:white_bundle", 1);
        child
            .data_mut()
            .set_other("minecraft:bundle_contents", Some(Tag::List(Vec::new())));
        let mut parent = ItemStack::new("minecraft:red_bundle", 1);
        parent.data_mut().bundle_contents = vec![child.clone()];
        assert_eq!(Contents::room_in_stack(&items, &parent, &stone), Some(60));

        child
            .data_mut()
            .set_other("minecraft:bundle_contents", None);
        child.data_mut().set_other(
            "!minecraft:bundle_contents",
            Some(Tag::Compound(Default::default())),
        );
        assert_eq!(Contents::room_in_stack(&items, &child, &stone), None);
        parent.data_mut().bundle_contents = vec![child];
        assert_eq!(Contents::room_in_stack(&items, &parent, &stone), Some(0));

        // Both siblings can independently inherit the same nonempty prototype.
        parent.data_mut().bundle_contents = vec![ItemStack::new("minecraft:blue_bundle", 1); 2];
        assert_eq!(Contents::room_in_stack(&items, &parent, &stone), Some(24));
    }

    #[test]
    fn loot_table_ids_list_the_roots_and_the_packs_tables() {
        let root = std::env::temp_dir().join(format!("sunrise-loot-ids-{}", std::process::id()));
        fs::create_dir_all(root.join("data/mymod/loot_table/rooms")).unwrap();
        fs::write(root.join("items.json"), "{}").unwrap();
        fs::write(
            root.join("data/mymod/loot_table/rooms/a.json"),
            r#"{"pools":[]}"#,
        )
        .unwrap();
        let pack = [(
            "loot_table/mypack:b".to_owned(),
            vec![br#"{"pools":[]}"#.to_vec()],
        )];
        let items = Items::catalog(
            &root,
            &PackOverlay::new(&pack),
            &sunrise_world::item_defaults::read(&root).unwrap(),
        )
        .unwrap();
        let mut ids: Vec<&str> = items.loot_table_ids().collect();
        ids.sort();
        assert_eq!(ids, ["mymod:rooms/a", "mypack:b"]);
        fs::remove_dir_all(root).unwrap();
    }

    /// `ExtraCodecs.POSITIVE_FLOAT` is exclusive of 0 and takes any float above
    /// it; `intRange(0, 15)` reads `Number.intValue`, which truncates.
    #[test]
    fn song_numbers_decode_like_the_codec() {
        let song = |length: f64, comparator: f64| {
            Song::decode(
                &json!({"sound_event": "demo:tune", "length_in_seconds": length,
                "comparator_output": comparator, "description": "tune"}),
            )
        };
        assert!(song(1e-40, 1.0).is_ok());
        assert!(song(0.0, 1.0).is_err());
        assert!(song(-1.0, 1.0).is_err());
        assert_eq!(song(5.0, 3.5).unwrap().comparator, 3);
        assert_eq!(song(5.0, 15.0).unwrap().comparator, 15);
        assert!(song(5.0, 16.0).is_err());
        assert!(song(5.0, -1.0).is_err());
    }

    /// #6129: a mod's biome, structure and enchantment, merged into the export after vanilla's
    /// and added to vanilla tags, leave every vanilla tag bound, with the mod members in them.
    #[test]
    fn mod_entries_merged_into_the_export_keep_vanilla_tags_bound() {
        let root = std::env::temp_dir().join(format!("sunrise-6129-tags-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("items.json"), "{}").unwrap();
        // The order the mod content load leaves: vanilla's sorted ids, then the mods'.
        let export = json!({
            "enchantments": ["minecraft:binding_curse"],
            "by_registry": {
                "minecraft:worldgen/biome": {"ids": [
                    "minecraft:badlands", "minecraft:wooded_badlands", "ae2:spatial_storage",
                ]},
                "minecraft:worldgen/structure": {"ids": [
                    "minecraft:village_plains", "minecraft:village_taiga", "ae2:meteorite",
                ]},
                "minecraft:enchantment": {"ids": ["minecraft:binding_curse"]},
            },
        });
        fs::write(root.join("command-registries.json"), export.to_string()).unwrap();
        let tag = |values: &[&str]| vec![json!({ "values": values }).to_string().into_bytes()];
        let enchantment = || {
            vec![
                json!({"description": "curse", "max_level": 1})
                    .to_string()
                    .into_bytes(),
            ]
        };
        let files = vec![
            (
                "tags/minecraft:worldgen/biome/is_badlands".to_owned(),
                tag(&[
                    "minecraft:badlands",
                    "minecraft:wooded_badlands",
                    "ae2:spatial_storage",
                ]),
            ),
            (
                "tags/minecraft:worldgen/structure/village".to_owned(),
                tag(&["minecraft:village_taiga", "ae2:meteorite"]),
            ),
            (
                "tags/minecraft:enchantment/curse".to_owned(),
                tag(&["minecraft:binding_curse", "enderio:soulbound"]),
            ),
            (
                "enchantment/minecraft:binding_curse".to_owned(),
                enchantment(),
            ),
            ("enchantment/enderio:soulbound".to_owned(), enchantment()),
        ];
        let overlay = PackOverlay {
            files: &files,
            ..PackOverlay::default()
        };
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let items = Items::tags_only(&root, &overlay, &defaults).unwrap();
        fs::remove_dir_all(&root).unwrap();
        let members = |registry, tag| items.tag_members(registry, tag).map(<[String]>::to_vec);
        assert_eq!(
            members(TagRegistry::Biome, "minecraft:is_badlands"),
            Some(vec![
                "minecraft:badlands".to_owned(),
                "minecraft:wooded_badlands".to_owned(),
                "ae2:spatial_storage".to_owned(),
            ])
        );
        assert_eq!(
            members(TagRegistry::Structure, "minecraft:village"),
            Some(vec![
                "minecraft:village_taiga".to_owned(),
                "ae2:meteorite".to_owned(),
            ])
        );
        assert_eq!(
            members(TagRegistry::Enchantment, "minecraft:curse"),
            Some(vec![
                "minecraft:binding_curse".to_owned(),
                "enderio:soulbound".to_owned(),
            ])
        );
    }

    #[test]
    fn server_json_conditions_filter_selected_instruments_and_enchantments() {
        let root = std::env::temp_dir().join(format!("sunrise-server-json-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mut files = Vec::new();
        for kind in ["instrument", "enchantment"] {
            let dir = root.join("data/demo").join(kind);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("entry.json"), "{}").unwrap();
            files.push((format!("{kind}/demo:entry"), vec![
                b"{}".to_vec(),
                json!({"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"provider"}],"description":"winning","max_level":7,"sound_event":"demo:note","use_duration":4.0,"range":16.0,"length_in_seconds":4.0,"comparator_output":1}).to_string().into_bytes(),
            ]));
            files.push((
                format!("{kind}/demo:unknown"),
                vec![
                    json!({"neoforge:conditions":[{"type":"unsupported"}]})
                        .to_string()
                        .into_bytes(),
                ],
            ));
        }
        let off = PackOverlay {
            files: &files,
            ..PackOverlay::default()
        };
        let enabled = sunrise_world::conditions::LoadConditions::new(["provider"]);
        let on = PackOverlay {
            files: &files,
            conditions: &enabled,
            ..PackOverlay::default()
        };
        for kind in ["instrument", "enchantment"] {
            assert!(data_files(&root, kind, &off).is_empty());
            let selected = data_files(&root, kind, &on);
            assert_eq!(selected.len(), 1);
            assert_eq!(selected["demo:entry"]["description"], "winning");
        }
        assert!(load_instruments(&root, &off).is_empty());
        assert!(load_instruments(&root, &on).contains_key("demo:entry"));
        fs::write(root.join("items.json"), "{}").unwrap();
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        assert!(
            crate::enchant::Enchantments::load_with(&root, &off, &defaults)
                .unwrap()
                .list
                .is_empty()
        );
        let enchantments = crate::enchant::Enchantments::load_with(&root, &on, &defaults).unwrap();
        assert_eq!(enchantments.list.len(), 1);
        assert_eq!(enchantments.list[0].max_level, 7);
        fs::remove_dir_all(root).unwrap();
    }

    /// #1381: the `data_files` kinds (instruments, songs, enchantments) carry
    /// no element conditions: a nested key stays, unevaluated.
    #[test]
    fn data_files_keep_a_list_elements_conditions_as_data() {
        let root =
            std::env::temp_dir().join(format!("sunrise-1381-data-files-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let dir = root.join("data/demo/instrument");
        fs::create_dir_all(&dir).unwrap();
        let file = json!({"description": "horn", "sounds": [
            {"id": "demo:a", "neoforge:conditions": [{"type": "absent:condition"}]}]});
        fs::write(dir.join("horn.json"), file.to_string()).unwrap();
        let conditions = sunrise_world::conditions::LoadConditions::new([]);
        let overlay = PackOverlay {
            conditions: &conditions,
            ..PackOverlay::default()
        };
        let loaded = data_files(&root, "instrument", &overlay);
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(loaded.get("demo:horn"), Some(&file));
        assert_eq!(conditions.check_errors(), Ok(()));
    }

    #[test]
    fn tag_conditions_use_enabled_server_ids_before_each_replace_and_reload() {
        use mod_vocab::Side;
        use sunrise_mod_package::enabled::EnabledMods;
        let root = std::env::temp_dir().join(format!(
            "sunrise-conditional-tags-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let dir = root.join("data/visuals/tags/block");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            root.join(crate::content_namespaces::MARKER),
            r#"["visuals"]"#,
        )
        .unwrap();
        let condition = json!([{"type":"neoforge:mod_loaded","modid":"code_only"}]);
        fs::write(
            dir.join("base.json"),
            serde_json::to_vec(&json!({
                "neoforge:conditions": condition,
                "values": ["minecraft:stone"]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            dir.join("namespace.json"),
            serde_json::to_vec(&json!({
                "neoforge:conditions": [{"type":"neoforge:mod_loaded","modid":"visuals"}],
                "values": ["minecraft:stone"]
            }))
            .unwrap(),
        )
        .unwrap();
        let bytes = |value: Value| serde_json::to_vec(&value).unwrap();
        let files = vec![
            (
                "tags/visuals:block/layers".to_owned(),
                vec![
                    bytes(json!({"values":["minecraft:stone"]})),
                    bytes(json!({"neoforge:conditions": condition, "replace":true,
                    "values":["minecraft:dirt"]})),
                    bytes(json!({"neoforge:conditions":[{"type":"unknown:condition"}],
                    "replace":true,"values":["minecraft:iron_block"]})),
                    bytes(json!({"values":["minecraft:gold_block"]})),
                ],
            ),
            (
                "tags/visuals:block/absent".to_owned(),
                vec![bytes(json!({
                    "neoforge:conditions":[{"type":"neoforge:not","value":{"type":"neoforge:always"}}],
                    "replace":true,"values":["minecraft:iron_block"]
                }))],
            ),
        ];
        let overlay = PackOverlay::new(&files);
        for side in [None, Some(Side::Client), Some(Side::Both), None] {
            let inventory = EnabledMods::new(side.map(|side| ("code_only".into(), side)));
            fs::write(
                root.join(sunrise_mod_package::enabled::MARKER),
                serde_json::to_vec(&inventory).unwrap(),
            )
            .unwrap();
            let loaded = tag_layers::load(&root, "block", &overlay, |_| true).unwrap();
            assert_eq!(
                loaded["visuals:layers"],
                ["minecraft:iron_block", "minecraft:gold_block"]
            );
            assert_eq!(loaded["visuals:base"], ["minecraft:stone"]);
            assert!(
                loaded.contains_key("visuals:namespace"),
                "tag conditions are not evaluated"
            );
            assert!(
                loaded.contains_key("visuals:absent"),
                "tag conditions do not skip layers"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_higher_packs_remove_takes_entries_out_of_a_tag() {
        let root = std::env::temp_dir().join(format!("sunrise-tag-remove-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let bytes = |value: Value| serde_json::to_vec(&value).unwrap();
        let files = vec![
            (
                "tags/test:item/things".to_owned(),
                vec![
                    bytes(json!({"values": ["minecraft:stone", "minecraft:dirt", "#test:other"]})),
                    bytes(json!({"values": [], "remove": ["minecraft:dirt", "#test:other"]})),
                    bytes(json!({"neoforge:conditions": [{"type": "neoforge:not",
                    "value": {"type": "neoforge:always"}}], "values": [], "remove": ["minecraft:stone"]})),
                ],
            ),
            (
                "tags/test:item/other".to_owned(),
                vec![bytes(json!({"values": ["minecraft:sand"]}))],
            ),
        ];
        let loaded = tag_layers::load(&root, "item", &PackOverlay::new(&files), |_| true).unwrap();
        assert!(loaded["test:things"].is_empty());
        assert_eq!(loaded["test:other"], ["minecraft:sand"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn attribute_modifiers_come_from_the_stack() {
        let items = Items::for_tests(HashMap::new());
        let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
        let attack = sunrise_core::nbt::Tag::compound([
            (
                "type",
                sunrise_core::nbt::Tag::String("minecraft:attack_damage".into()),
            ),
            (
                "id",
                sunrise_core::nbt::Tag::String("minecraft:test".into()),
            ),
            ("amount", sunrise_core::nbt::Tag::Double(0.1f64.to_bits())),
            (
                "operation",
                sunrise_core::nbt::Tag::String("add_value".into()),
            ),
            ("slot", sunrise_core::nbt::Tag::String("mainhand".into())),
        ]);
        sword.data_mut().set_other(
            "minecraft:attribute_modifiers",
            Some(sunrise_core::nbt::Tag::List(vec![attack])),
        );
        let modifiers = sword.default_attribute_modifiers(&items);
        assert_eq!(modifiers.len(), 1);
        assert_eq!(modifiers[0].attribute, "minecraft:attack_damage");
        // A double, as `AttributeModifier.amount`: not rounded through a float.
        assert_eq!(modifiers[0].amount, 0.1);
        drop(modifiers);
        sword.data_mut().set_other(
            "!minecraft:attribute_modifiers",
            Some(sunrise_core::nbt::Tag::compound(Vec::<(
                &str,
                sunrise_core::nbt::Tag,
            )>::new())),
        );
        assert!(sword.default_attribute_modifiers(&items).is_empty());
    }

    #[test]
    fn admitted_tool_rules_override_defaults_and_removals_survive_saving() {
        let root =
            std::env::temp_dir().join(format!("sunrise-tool-components-{}", std::process::id()));
        fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
        fs::write(root.join("items.json"), r#"{"minecraft:stick":{},"minecraft:diamond_pickaxe":{"minecraft:max_damage":1561,"minecraft:tool":{"rules":[{"blocks":"minecraft:stone","speed":8.0,"correct_for_drops":true}],"damage_per_block":1}}}"#).unwrap();
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let items = Items::catalog(&root, &PackOverlay::NONE, &defaults).unwrap();
        let mut pick = ItemStack::new("minecraft:diamond_pickaxe", 1);
        assert_eq!(items.mining_speed(Some(&pick), "minecraft:stone"), 8.0);
        assert!(items.correct_for_drops(Some(&pick), "minecraft:stone"));
        let removal = crate::item_data::command_components("[!tool]").unwrap();
        (pick.damage, pick.extra) = crate::item_data::decode_components(Some(&removal));
        let restored = crate::item_nbt::item_from_nbt(&crate::item_nbt::item_to_nbt(
            &pick,
            &Default::default(),
        ))
        .unwrap();
        assert_eq!(items.mining_speed(Some(&restored), "minecraft:stone"), 1.0);
        assert!(!items.correct_for_drops(Some(&restored), "minecraft:stone"));
        assert_eq!(items.mining_damage(&restored), None);
        let mut stick = ItemStack::new("minecraft:stick", 1);
        let tool = crate::item_data::command_components(r#"[tool={rules:[{blocks:"stone",correct_for_drops:false},{blocks:["stone","dirt"],speed:7.0f,correct_for_drops:true},{blocks:"stone",speed:90.0f}],default_mining_speed:2.0f,damage_per_block:3}]"#).unwrap();
        (stick.damage, stick.extra) = crate::item_data::decode_components(Some(&tool));
        assert_eq!(items.mining_speed(Some(&stick), "minecraft:stone"), 7.0);
        assert!(!items.correct_for_drops(Some(&stick), "minecraft:stone"));
        assert_eq!(items.mining_speed(Some(&stick), "minecraft:cobweb"), 2.0);
        assert_eq!(items.mining_damage(&stick), Some(3));
        assert_eq!(items.mining_damage_capacity(&stick), 0);
        stick.data_mut().set_other(
            "minecraft:max_damage",
            Some(sunrise_core::nbt::Tag::Int(20)),
        );
        assert_eq!(
            items.mining_damage_capacity(&stick),
            0,
            "MAX_DAMAGE alone does not supply DAMAGE"
        );
        stick.count = 0;
        assert_eq!(items.mining_speed(Some(&stick), "minecraft:stone"), 1.0);
        assert!(!items.correct_for_drops(Some(&stick), "minecraft:stone"));
        assert_eq!(items.mining_damage(&stick), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn command_loot_keeps_zero_positions_without_changing_filtered_rolls() {
        let mut items = Items::for_tests(HashMap::new());
        let zero = json!({"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:iron_ingot",
            "modifier":{"type":"minecraft:set_count","count":0}}]});
        let positive =
            json!({"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:iron_ingot"}]});
        let flat = json!({"pools":[zero, positive]});
        let empty = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:empty"}]}]});
        let nested =
            json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:loot_table","value":flat}]}]});
        let random = json!({"pools":[zero, {"rolls":{"type":"minecraft:uniform","min":1,"max":3},
            "entries":[{"type":"minecraft:item","name":"minecraft:iron_ingot"},
                       {"type":"minecraft:item","name":"minecraft:gold_ingot"}]}]});
        let summary = |stacks: Vec<ItemStack>| {
            stacks
                .into_iter()
                .map(|s| (s.item.to_string(), s.count))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            summary(items.inline_loot_table_for_command(
                &flat,
                None,
                &mut LegacyRandom::new(39),
                true
            )),
            vec![
                ("minecraft:iron_ingot".into(), 0),
                ("minecraft:iron_ingot".into(), 1)
            ]
        );
        assert!(
            items
                .inline_loot_table_for_command(&empty, None, &mut LegacyRandom::new(39), true)
                .is_empty(),
            "EmptyLootItem emits nothing, rather than an empty item position"
        );
        let deferred = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:dynamic","name":"minecraft:contents"}],
            "modifier":{"type":"minecraft:set_count","count":0}}]});
        assert!(
            items
                .inline_loot_table_for_command(&deferred, None, &mut LegacyRandom::new(39), true)
                .is_empty(),
            "deferred dynamic placeholders are not emitted items"
        );
        for table in [flat, empty, nested, random, deferred] {
            items.loot = Arc::new(
                [
                    ("test:table".into(), table.clone()),
                    ("minecraft:entities/pig".into(), table.clone()),
                ]
                .into_iter()
                .collect(),
            );
            for seed in 0..32 {
                let mut natural_rng = LegacyRandom::new(seed);
                let natural =
                    summary(items.inline_loot_table_with_tool(&table, None, &mut natural_rng));
                let continuation = natural_rng.next_long();
                let mut command_rng = LegacyRandom::new(seed);
                let command =
                    items.inline_loot_table_for_command(&table, None, &mut command_rng, true);
                assert_eq!(
                    summary(command.into_iter().filter(|s| s.count > 0).collect()),
                    natural
                );
                assert_eq!(
                    command_rng.next_long(),
                    continuation,
                    "output policy consumes no randomness"
                );
                let mut named_rng = LegacyRandom::new(seed);
                let named =
                    items.loot_table_for_command("test:table", false, None, &mut named_rng, true);
                assert_eq!(
                    summary(named.into_iter().filter(|s| s.count > 0).collect()),
                    natural
                );
                assert_eq!(named_rng.next_long(), continuation);
                let mut entity_rng = LegacyRandom::new(seed);
                let entity = items.entity_drops_for_command(
                    "minecraft:pig",
                    false,
                    None,
                    &mut entity_rng,
                    true,
                );
                assert_eq!(
                    summary(entity.into_iter().filter(|s| s.count > 0).collect()),
                    natural
                );
                assert_eq!(entity_rng.next_long(), continuation);
            }
        }
    }

    /// `LootCommand.dropFishingLoot` carries TOOL even for an inline holder.
    #[test]
    fn inline_fishing_keeps_the_actual_tool_context() {
        let mut items = Items::for_tests(HashMap::new());
        let table = |tool| {
            json!({"type":"minecraft:fishing","pools":[{"rolls":1,"entries":[{
                "type":"minecraft:item","name":"minecraft:diamond",
                "condition":{"type":"minecraft:match_tool","predicate":{"items":tool}}
            }]}]})
        };
        let rod_table = table("minecraft:fishing_rod");
        let stick_table = table("minecraft:stick");
        let unconditioned = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]});
        items.loot = Arc::new(
            [
                ("test:rod".to_owned(), rod_table.clone()),
                ("test:stick".to_owned(), stick_table.clone()),
                ("test:plain".to_owned(), unconditioned.clone()),
            ]
            .into_iter()
            .collect(),
        );
        let rod = ItemStack::new("minecraft:fishing_rod", 1);
        let stick = ItemStack::new("minecraft:stick", 1);
        let summary = |drops: Vec<ItemStack>| {
            drops
                .into_iter()
                .map(|s| (s.item.to_string(), s.count))
                .collect::<Vec<_>>()
        };
        for (id, table, matches) in [
            ("test:rod", &rod_table, [false, true, false]),
            ("test:stick", &stick_table, [false, false, true]),
            ("test:plain", &unconditioned, [true, true, true]),
        ] {
            for (held, matches) in [None, Some(&rod), Some(&stick)].into_iter().zip(matches) {
                let expected = if matches {
                    vec![("minecraft:diamond".to_owned(), 1)]
                } else {
                    vec![]
                };
                let mut named_rng = LegacyRandom::new(41);
                assert_eq!(
                    summary(items.loot_table(id, false, held, &mut named_rng)),
                    expected
                );
                let mut inline_rng = LegacyRandom::new(41);
                assert_eq!(
                    summary(items.inline_loot_table_with_tool(table, held, &mut inline_rng)),
                    expected
                );
                let mut command_rng = LegacyRandom::new(41);
                assert_eq!(
                    summary(items.inline_loot_table_for_command(
                        table,
                        held,
                        &mut command_rng,
                        true
                    )),
                    expected
                );
                let continuation = named_rng.next_long();
                assert_eq!(
                    inline_rng.next_long(),
                    continuation,
                    "same evaluator draw order"
                );
                assert_eq!(
                    command_rng.next_long(),
                    continuation,
                    "command output keeps TOOL and draw order"
                );
            }
            let mut random = LegacyRandom::new(41);
            let legacy = summary(items.inline_loot_table(table, &mut random));
            assert_eq!(
                legacy,
                if matches[0] {
                    vec![("minecraft:diamond".to_owned(), 1)]
                } else {
                    vec![]
                }
            );
        }
    }

    #[test]
    fn a_stacks_own_blocks_attacks_component_counts() {
        let plain = ItemStack::new("minecraft:diamond_sword", 1);
        assert!(BlocksAttacks::of_stack(&plain, None).is_none());
        let mut sword = plain.clone();
        let component = json!({
            "block_delay_seconds": 0.0,
            "disable_cooldown_scale": 0.0,
            "damage_reductions": [{"horizontal_blocking_angle": 180.0, "base": -0.5, "factor": 0.5}],
            "item_damage": {"threshold": 1.0e9, "base": 0.0, "factor": 0.0},
            "bypassed_by": "#minecraft:bypasses_armor",
        });
        sword.data_mut().set_other(
            "minecraft:blocks_attacks",
            Some(sunrise_game_data::scoreboard::json_to_tag(&component)),
        );
        let b = BlocksAttacks::of_stack(&sword, None).expect("the stack's own component");
        assert_eq!(b.delay_seconds, 0.0);
        assert_eq!(b.disable_scale, 0.0);
        assert_eq!(b.reductions.len(), 1);
        assert_eq!(b.reductions[0].horizontal_angle, 180.0);
        assert_eq!((b.reductions[0].base, b.reductions[0].factor), (-0.5, 0.5));
        assert_eq!(b.item_damage.0, 1.0e9);
        assert_eq!(b.bypassed_by.as_deref(), Some("#minecraft:bypasses_armor"));
        // An empty compound is the codec's defaults: a shield's 90 degrees.
        let mut bare = plain.clone();
        bare.data_mut().set_other(
            "minecraft:blocks_attacks",
            Some(sunrise_game_data::scoreboard::json_to_tag(&json!({}))),
        );
        let b = BlocksAttacks::of_stack(&bare, None).expect("defaults");
        assert_eq!(b.reductions[0].horizontal_angle, 90.0);
        assert_eq!(b.disable_scale, 1.0);
    }

    #[test]
    fn loaded_block_and_item_tags_answer_membership() {
        let mut items = super::Items::for_tests(std::collections::HashMap::from([(
            "test:items".into(),
            ["test:apple".into()].into_iter().collect(),
        )]));
        items.block_tags.insert(
            "test:blocks".into(),
            ["test:stone".into()].into_iter().collect(),
        );
        assert!(items.block_in_tag("test:blocks", "test:stone"));
        assert!(!items.block_in_tag("test:blocks", "test:dirt"));
        assert!(!items.block_in_tag("test:missing", "test:stone"));
        assert!(items.item_in_tag("test:items", "test:apple"));
        assert!(!items.item_in_tag("test:items", "test:stone"));
        assert!(!items.item_in_tag("test:missing", "test:apple"));
    }

    #[test]
    fn fractional_and_exponent_entry_weights_reach_the_roll() {
        for (token, at_seed_three) in [("2.0", "test:control"), ("1e1", "test:weighted")] {
            let mut items = Items::for_tests(HashMap::new());
            let weight: Value = serde_json::from_str(token).unwrap();
            items.loot = Arc::new(
                [(
                    "test:weights".to_owned(),
                    json!({"pools": [{"rolls": 1, "entries": [
                        {"type": "minecraft:item", "name": "test:weighted", "weight": weight},
                        {"type": "minecraft:item", "name": "test:control", "weight": 1}
                    ]}]}),
                )]
                .into_iter()
                .collect(),
            );
            // Independent JDK LCG first draws: seed 0 gives 0 modulo 3 or 11;
            // seed 3 gives 2 modulo 3 and 6 modulo 11. Thus weights 2 and 10
            // choose different entries at seed 3, and neither is default 1.
            for (seed, expected) in [(0, "test:weighted"), (3, at_seed_three)] {
                let stacks =
                    items.loot_table("test:weights", false, None, &mut LegacyRandom::new(seed));
                assert_eq!(stacks.len(), 1, "{token}, seed {seed}");
                assert_eq!(
                    (&*stacks[0].item, stacks[0].count),
                    (expected, 1),
                    "{token}, seed {seed}"
                );
            }
        }
    }

    /// `LootPool.CODEC` reads a pool's `condition` and has no `conditions` field (#5378): the
    /// list pre-26.3 data writes is ignored, so its pool rolls, while a pool's `condition` that
    /// fails stops it.
    #[test]
    fn a_pools_old_conditions_list_is_ignored_and_its_condition_is_not() {
        let mut items = Items::for_tests(HashMap::new());
        let never = json!({"type":"minecraft:random_chance","chance":0.0});
        let diamond = json!([{"type":"minecraft:item","name":"minecraft:diamond"}]);
        items.loot = Arc::new(
            [
                (
                    "test:old",
                    json!({"pools":[{"rolls":1,"conditions":[never],"entries":diamond}]}),
                ),
                (
                    "test:new",
                    json!({"pools":[{"rolls":1,"condition":never,"entries":diamond}]}),
                ),
            ]
            .map(|(id, t)| (id.to_owned(), t))
            .into_iter()
            .collect(),
        );
        let roll = |id: &str| {
            items
                .loot_table(id, false, None, &mut LegacyRandom::new(1))
                .len()
        };
        assert_eq!(roll("test:old"), 1);
        assert_eq!(roll("test:new"), 0);
    }

    /// `NestedLootTable`: a `loot_table` entry, by id or inline, rolls that
    /// table in full and applies its own functions to each stack; a table
    /// never rolls inside itself.
    #[test]
    fn loot_table_entries_roll_their_table() {
        let mut items = Items::for_tests(HashMap::new());
        let diamond = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]});
        items.loot = Arc::new(
            [
                ("test:inner", diamond.clone()),
                ("test:outer", json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:loot_table",
                    "value":"test:inner","modifier":[{"type":"minecraft:set_count","count":3}]}]}]})),
                ("test:inline", json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:loot_table",
                    "value":diamond}]}]})),
                ("test:loop", json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:loot_table",
                    "value":"test:loop"}]},{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:stick"}]}]})),
            ]
            .map(|(id, t)| (id.to_owned(), t))
            .into_iter()
            .collect(),
        );
        let roll = |id: &str| {
            items
                .loot_table(id, false, None, &mut LegacyRandom::new(1))
                .into_iter()
                .map(|s| format!("{} {}", s.item, s.count))
                .collect::<Vec<_>>()
        };
        // The empty fixture registry gives an unknown item a stack limit of one.
        assert_eq!(roll("test:outer"), ["minecraft:diamond 1"; 3]);
        assert_eq!(roll("test:inline"), ["minecraft:diamond 1"]);
        // `test:loop` is already rolling: its own entry adds nothing.
        assert_eq!(roll("test:loop"), ["minecraft:stick 1"]);
    }

    /// `Blocks.wallVariant`: a wall block without a table of its own drops
    /// from its standing block's (vanilla's tables: `blocks/torch` for
    /// `wall_torch`, `blocks/oak_sign` for `oak_wall_sign`...).
    #[test]
    fn wall_blocks_use_the_standing_blocks_table() {
        let facts = br#"{"defaults":{"minecraft:air":0,"minecraft:wall_torch":1,
            "minecraft:oak_wall_sign":2,"minecraft:skeleton_wall_skull":3,
            "minecraft:dead_brain_coral_wall_fan":4,"test:wall_lamp":5,"minecraft:cobblestone_wall":6},
            "states":[
            ["air",true,false,"","minecraft:air"],
            ["a",false,false,"","minecraft:wall_torch"],
            ["b",false,false,"","minecraft:oak_wall_sign"],
            ["c",false,false,"","minecraft:skeleton_wall_skull"],
            ["d",false,false,"","minecraft:dead_brain_coral_wall_fan"],
            ["e",false,false,"","test:wall_lamp"],
            ["f",false,false,"","minecraft:cobblestone_wall"]
        ]}"#;
        let registry = BlockRegistry::from_facts_json(&facts[..]).unwrap();
        let mut items = Items::for_tests(HashMap::new());
        items.loot = Arc::new(
            [
                "minecraft:blocks/torch",
                "minecraft:blocks/oak_sign",
                "minecraft:blocks/skeleton_skull",
                "minecraft:blocks/dead_brain_coral_fan",
                "test:blocks/wall_lamp",
                "test:blocks/lamp",
                "minecraft:blocks/cobblestone_wall",
            ]
            .map(|id| (id.to_owned(), json!({"pools": []})))
            .into_iter()
            .collect(),
        );
        let table = |state| items.block_loot_table(&registry, BlockStateId(state));
        assert_eq!(table(1).as_deref(), Some("minecraft:blocks/torch"));
        assert_eq!(table(2).as_deref(), Some("minecraft:blocks/oak_sign"));
        assert_eq!(table(3).as_deref(), Some("minecraft:blocks/skeleton_skull"));
        assert_eq!(
            table(4).as_deref(),
            Some("minecraft:blocks/dead_brain_coral_fan")
        );
        // A table of its own wins over the standing block's.
        assert_eq!(table(5).as_deref(), Some("test:blocks/wall_lamp"));
        assert_eq!(
            table(6).as_deref(),
            Some("minecraft:blocks/cobblestone_wall")
        );
        assert_eq!(table(0), None);
    }

    #[test]
    fn content_mod_blocks_keep_their_namespace_in_loot_table_ids() {
        let facts = br#"{"defaults":{"minecraft:air":0,"loot_repro:glowing_amethyst":1},
            "states":[
            ["air",true,false,"","minecraft:air"],
            ["amethyst",false,false,"","loot_repro:glowing_amethyst"]
        ]}"#;
        let registry = BlockRegistry::from_facts_json(&facts[..]).unwrap();
        let mut items = Items::for_tests(HashMap::new());
        items.loot = Arc::new(
            [(
                "loot_repro:blocks/glowing_amethyst".to_owned(),
                json!({"pools": []}),
            )]
            .into_iter()
            .collect(),
        );

        assert_eq!(
            items
                .block_loot_table(&registry, BlockStateId(1))
                .as_deref(),
            Some("loot_repro:blocks/glowing_amethyst")
        );
    }

    #[test]
    fn entity_target_context_leaves_unaffected_rolls_and_rng_continuation_unchanged() {
        let mut items = Items::for_tests(HashMap::new());
        items.loot = Arc::new([("minecraft:entities/test".into(), json!({"pools":[{
            "rolls":{"type":"minecraft:uniform","min":1,"max":3},
            "condition":{"type":"minecraft:all_of","terms":[
                {"type":"minecraft:killed_by_player"},
                {"type":"minecraft:match_tool","predicate":{"items":"minecraft:stick"}}]},
            "entries":[{"type":"minecraft:item","name":"minecraft:diamond","weight":1},
                       {"type":"minecraft:item","name":"minecraft:iron_ingot","weight":3}],
            "modifier":[{"type":"minecraft:set_count","count":{"type":"minecraft:uniform","min":1,"max":4}}]
        }]}))].into_iter().collect());
        let target = sunrise_world::loot::LootEntity::new("minecraft:sheep");
        let held = ItemStack::new("minecraft:stick", 1);
        let unused = |_: &Value, _: &sunrise_world::loot::LootEntity| -> bool {
            panic!("no entity predicate on this unaffected path")
        };
        for seed in [0, 1, 99, i64::MAX] {
            for by_player in [false, true] {
                let mut old = LegacyRandom::new(seed);
                let mut bound = old.clone();
                let expected =
                    items.entity_drops("minecraft:test", by_player, Some(&held), &mut old);
                let actual = items.entity_drops_with_target(
                    "minecraft:test",
                    by_player,
                    Some(&held),
                    &target,
                    &unused,
                    &mut bound,
                    false,
                );
                assert_eq!(actual, expected);
                assert_eq!(bound.next_long(), old.next_long());
                assert_eq!(bound.next_int_bound(137), old.next_int_bound(137));
            }
        }
    }

    #[test]
    fn entity_target_context_reuses_the_loaded_matcher_only_for_this() {
        use sunrise_world::loot::{LootData, LootEntity};
        let root = std::env::temp_dir().join(format!("h036-items-matcher-{}", std::process::id()));
        std::fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
        std::fs::write(root.join("items.json"), "{}").unwrap();
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let data = LootData::load(&root, &PackOverlay::NONE, &defaults).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        let matcher = |p: &Value, e: &LootEntity| data.entity_matches(p, Some(e));
        let mut items = Items::for_tests(HashMap::new());
        for who in ["this", "attacker"] {
            items.loot = Arc::new(
                [(
                    "minecraft:entities/sheep".into(),
                    json!({"pools":[{"rolls":1,"entries":[{
                        "type":"minecraft:item","name":"minecraft:white_wool",
                        "condition":{"type":"minecraft:entity_properties","entity":who,"predicate":{
                            "minecraft:components":{"minecraft:sheep/color":"white"},
                            "minecraft:type_specific/sheep":{"sheared":false}}}
                    }]}]}),
                )]
                .into_iter()
                .collect(),
            );
            for (color, sheared) in [("white", false), ("white", true), ("red", false)] {
                let mut target = LootEntity::new("minecraft:sheep");
                target.sheep = Some((color.into(), sheared));
                let mut random = LegacyRandom::new(36);
                let actual = items.entity_drops_with_target(
                    "minecraft:sheep",
                    true,
                    None,
                    &target,
                    &matcher,
                    &mut random,
                    false,
                );
                assert_eq!(
                    !actual.is_empty(),
                    who == "this" && color == "white" && !sheared
                );
                assert!(
                    items
                        .entity_drops("minecraft:sheep", true, None, &mut LegacyRandom::new(36))
                        .is_empty()
                );
            }
        }
    }

    #[test]
    fn location_context_preserves_unaffected_rolls_and_rng_continuation() {
        let registry = BlockRegistry::from_facts_json(&br#"{"defaults":{"minecraft:stone":0},"states":[["stone",false,false,"","minecraft:stone"]]}"#[..]).unwrap();
        let mut items = Items::for_tests(HashMap::new());
        items.loot = Arc::new([("minecraft:blocks/stone".into(), json!({"pools":[{
            "rolls":{"type":"minecraft:uniform","min":1,"max":3},
            "condition":{"type":"minecraft:match_tool","predicate":{"items":"minecraft:stick"}},
            "entries":[{"type":"minecraft:item","name":"minecraft:diamond","weight":1},
                       {"type":"minecraft:item","name":"minecraft:emerald","weight":3}],
            "modifier":[{"type":"minecraft:set_count","count":{"type":"minecraft:uniform","min":1,"max":4}}]
        }]}))].into_iter().collect());
        let unused = |_: &Value, _: [f64; 3]| -> bool {
            panic!("unaffected conditions must not invoke location reader")
        };
        for seed in [0, 1, 79, i64::MAX] {
            for held in [None, Some(ItemStack::new("minecraft:stick", 1))] {
                let mut old = LegacyRandom::new(seed);
                let mut contextual = old.clone();
                assert_eq!(
                    items.block_drops(
                        &registry,
                        BlockStateId(0),
                        BlockLootRoll {
                            held: held.as_ref(),
                            ..Default::default()
                        },
                        &mut old,
                        None
                    ),
                    items.block_drops(
                        &registry,
                        BlockStateId(0),
                        BlockLootRoll {
                            held: held.as_ref(),
                            location: Some(sunrise_world::loot::LootLocation {
                                origin: [0.5; 3],
                                level: &unused,
                            }),
                            ..Default::default()
                        },
                        &mut contextual,
                        None,
                    )
                );
                assert_eq!(old.next_long(), contextual.next_long());
                assert_eq!(old.next_int_bound(137), contextual.next_int_bound(137));
            }
        }
    }

    /// `LocationCheck.test`: no origin fails, an origin with no predicate holds, and
    /// otherwise the level answers at the offset position.
    #[test]
    fn location_check_needs_an_origin_and_asks_the_level() {
        let items = Items::for_tests(HashMap::new());
        let properties = sunrise_world::chunk::BlockProperties::new();
        let drops = |condition: Value, location: Option<sunrise_world::loot::LootLocation<'_>>| {
            let table = json!({"pools":[{"rolls":1,"condition":condition,"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]});
            LootContext {
                location,
                ..ctx(&items, None, &properties, "minecraft:stone")
            }
            .evaluate(&table, &mut LegacyRandom::new(79))
            .len()
        };
        let check = json!({"type":"minecraft:location_check","offsetY":-1,"predicate":{"block":{"blocks":"minecraft:stone"}}});
        assert_eq!(drops(check.clone(), None), 0);
        assert_eq!(drops(json!({"type":"minecraft:location_check"}), None), 0);
        for answer in [false, true] {
            let level = |_: &Value, at: [f64; 3]| {
                assert_eq!(at, [0.5, 63.5, 0.5]);
                answer
            };
            let location = sunrise_world::loot::LootLocation {
                origin: [0.5, 64.5, 0.5],
                level: &level,
            };
            assert_eq!(drops(check.clone(), Some(location)), usize::from(answer));
            assert_eq!(
                drops(json!({"type":"minecraft:location_check"}), Some(location)),
                1
            );
        }
    }

    fn ctx<'a>(
        items: &'a Items,
        held: Option<&'a ItemStack>,
        props: &'a sunrise_world::chunk::BlockProperties,
        block: &'a str,
    ) -> LootContext<'a, LegacyRandom> {
        LootContext {
            query: sunrise_world::loot::query::current_query(),
            place: sunrise_world::loot::current_native_loot_place(),
            items,
            local_bindings: None,
            block,
            properties: props,
            held,
            killed_by_player: block.is_empty(),
            explosion_radius: None,
            this_entity: false,
            entity_levels: None,
            block_levels: None,
            target: None,
            target_entity: None,
            interacting_entity: None,
            location: None,
            keep_empty: false,
        }
    }

    /// A mod's kind nobody claims fails; a reference by name isn't a kind,
    /// and fails too.
    #[test]
    fn an_unclaimed_condition_of_a_mods_kind_fails() {
        let items = Items::for_tests(HashMap::new());
        let props = sunrise_world::chunk::BlockProperties::new();
        let table = |condition: Value| {
            json!({"pools":[{"rolls":1,"entries":[
                {"type":"minecraft:item","name":"minecraft:stone","condition":condition}]}]})
        };
        let mut random = LegacyRandom::new(7);
        let mut drops = |condition: Value, block: &str| {
            ctx(&items, None, &props, block)
                .evaluate(&table(condition), &mut random)
                .len()
        };
        assert_eq!(drops(json!({"type":"mymod:always"}), "minecraft:stone"), 0);
        assert_eq!(drops(json!({"type":"mymod:always"}), ""), 0);
        assert_eq!(drops(json!("mymod:always"), "minecraft:stone"), 0);
        assert_eq!(drops(Value::Null, "minecraft:stone"), 1);
    }

    /// A mod's function kind changes the stack the items engine drops: what
    /// the hook leaves in the stack comes back (item, count, a component),
    /// through the conversion the block-drop path uses.
    #[test]
    fn a_mod_function_changes_the_stack_the_items_engine_drops() {
        use sunrise_world::loot::LootStackEditAccess;
        let items = Items::for_tests(HashMap::new());
        let props = sunrise_world::chunk::BlockProperties::new();
        let mut held = ItemStack::new("minecraft:diamond_pickaxe", 1);
        held.enchant("minecraft:fortune", 3);
        let block = ctx(&items, Some(&held), &props, "minecraft:stone");
        let native = block.roll_facts();
        let mut candidate =
            crate::loot_candidate::LootCandidate::from_stack(ItemStack::new("minecraft:stone", 1));
        let tool = native.tool.unwrap();
        assert_eq!(tool.item_id(), "minecraft:diamond_pickaxe");
        assert_eq!(tool.enchantment_at(0), Some(("minecraft:fortune", 3)));
        assert!(tool.enchantment_at(1).is_none());
        candidate.set_item_id("minecraft:diamond");
        candidate.set_count(7);
        candidate.push_component(
            "minecraft:custom_data".to_owned(),
            Some(sunrise_core::json_nbt::json_to_tag(&json!({"machine": "furnace"})).unwrap()),
        );
        let stack = candidate.into_item_stack(&items);
        assert_eq!(&*stack.item, "minecraft:diamond");
        assert_eq!(stack.count, 7);
        assert_eq!(
            stack.data().and_then(|d| d.other("minecraft:custom_data")),
            Some(&sunrise_game_data::scoreboard::json_to_tag(
                &json!({"machine": "furnace"})
            ))
        );
    }

    #[test]
    fn two_ordered_custom_functions_share_live_signed_count_map_exploration_and_components() {
        use std::sync::{Arc, Mutex};
        use sunrise_core::nbt::Tag;
        use sunrise_world::loot::{
            ApiExplorationMap, LootStackEditAccess,
            types::{
                BoundCondition, BoundFunction, DecodedCondition, DecodedFunction, Registry,
                RollRandom, Types,
            },
        };
        struct First;
        impl BoundFunction for First {
            fn apply(
                &self,
                _: &sunrise_world::loot::LootContext<'_>,
                _: &mut dyn RollRandom,
                _: &dyn sunrise_world::loot::LootTags,
                stack: &mut dyn LootStackEditAccess,
            ) -> std::result::Result<(), String> {
                stack.set_count(-2);
                stack.set_map(true);
                stack.set_exploration(Some(ApiExplorationMap::new(
                    "minecraft:on_woodland_explorer_maps".into(),
                    "minecraft:mansion".into(),
                    3,
                    75,
                    false,
                )));
                stack.push_component(
                    "minecraft:potion_contents".into(),
                    Some(Tag::compound([
                        ("potion", Tag::String("minecraft:healing".into())),
                        ("custom_color", Tag::Int(0x186cb8)),
                    ])),
                );
                stack.push_component(
                    "minecraft:written_book_content".into(),
                    Some(Tag::compound([
                        (
                            "title",
                            Tag::compound([("raw", Tag::String("Notes".into()))]),
                        ),
                        ("author", Tag::String("Alice".into())),
                        ("generation", Tag::Int(1)),
                        (
                            "pages",
                            Tag::List(vec![Tag::compound([("raw", Tag::String("Hello".into()))])]),
                        ),
                    ])),
                );
                Ok(())
            }
        }
        struct Second(Arc<Mutex<Option<(i32, bool, Option<String>)>>>);
        impl BoundFunction for Second {
            fn apply(
                &self,
                _: &sunrise_world::loot::LootContext<'_>,
                _: &mut dyn RollRandom,
                _: &dyn sunrise_world::loot::LootTags,
                stack: &mut dyn LootStackEditAccess,
            ) -> std::result::Result<(), String> {
                let view = stack.as_view();
                *self.0.lock().unwrap() = Some((
                    view.count(),
                    view.map(),
                    view.exploration().map(|m| m.destination().to_owned()),
                ));
                assert!(
                    view.components().iter().any(|(id, value)| id
                        == "minecraft:written_book_content"
                        && value.is_some())
                );
                assert_eq!(view.potion(), Some("minecraft:healing"));
                let next = view.count() + 5;
                stack.set_count(next);
                stack.set_potion_id(Some("minecraft:swiftness".into()));
                assert_eq!(stack.as_view().potion(), Some("minecraft:swiftness"));
                Ok(())
            }
        }
        struct TestRegistry(Arc<Mutex<Option<(i32, bool, Option<String>)>>>);
        impl Registry for TestRegistry {
            fn contains(&self, kind: &str, function: bool) -> bool {
                function && matches!(kind, "test:first" | "test:second")
            }
            fn conditional(&self, _: &str) -> bool {
                false
            }
            fn decode_function(
                &self,
                kind: &str,
                _: &Value,
            ) -> std::result::Result<DecodedFunction, String> {
                let rule: Arc<dyn BoundFunction> = match kind {
                    "test:first" => Arc::new(First),
                    "test:second" => Arc::new(Second(self.0.clone())),
                    _ => return Err(format!("unknown {kind}")),
                };
                Ok(DecodedFunction {
                    rule,
                    referenced_params: Vec::new(),
                })
            }
            fn decode_condition(
                &self,
                kind: &str,
                _: &Value,
            ) -> std::result::Result<DecodedCondition, String> {
                let _: Option<Arc<dyn BoundCondition>> = None;
                Err(format!("unknown {kind}"))
            }
        }
        let observed = Arc::new(Mutex::new(None));
        let table = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:written_book","modifier":[{"type":"test:first"},{"type":"test:second"}]}]}]});
        let tables = Arc::new(sunrise_world::loot::LootTables::from_iter([(
            "test:live".to_owned(),
            table,
        )]));
        let bindings = Types::new(Arc::new(TestRegistry(observed.clone())))
            .bind_generation(
                tables.clone(),
                Arc::default(),
                Arc::default(),
                Vec::new(),
                Default::default(),
            )
            .unwrap();
        let mut items = Items::for_tests(HashMap::new());
        items.loot = tables;
        items.loot_bindings = bindings;
        let drops = items.loot_table("test:live", false, None, &mut LegacyRandom::new(1));
        assert_eq!(
            *observed.lock().unwrap(),
            Some((
                -2,
                true,
                Some("minecraft:on_woodland_explorer_maps".to_owned())
            ))
        );
        assert_eq!(drops.len(), 3);
        for book in &drops {
            assert_eq!(book.count, 1);
            assert_eq!(book.potion(), Some("minecraft:swiftness"));
            let data = book.data().expect("book and potion components");
            assert_eq!(
                data.potion_extra,
                vec![("custom_color".into(), Tag::Int(0x186cb8))]
            );
            let written = data.written.as_ref().expect("decoded book contents");
            assert_eq!(written.title, "Notes");
            assert_eq!(written.author, "Alice");
            assert_eq!(written.generation, 1);
            assert_eq!(written.pages, ["Hello"]);
        }
    }

    #[test]
    fn loot_patches_keep_nbt_types_removals_and_order() {
        use sunrise_core::nbt::Tag;
        use sunrise_world::loot::{LootStackAccess, LootStackEditAccess};
        let mut stack = ItemStack::new("minecraft:stone", 1);
        stack
            .data_mut()
            .set_other("test:array", Some(Tag::ByteArray(vec![7])));
        stack.data_mut().set_other("test:byte", Some(Tag::Byte(7)));
        let mut candidate = crate::loot_candidate::LootCandidate::from_stack(stack);
        assert!(
            candidate
                .components()
                .contains(&("test:array".into(), Some(Tag::ByteArray(vec![7]))))
        );
        assert!(
            candidate
                .components()
                .contains(&("test:byte".into(), Some(Tag::Byte(7))))
        );
        candidate.push_component("test:array".into(), Some(Tag::List(vec![Tag::Int(7)])));
        candidate.push_component("test:array".into(), None);
        candidate.push_component("test:array".into(), Some(Tag::ByteArray(vec![7])));
        candidate.push_component("test:byte".into(), None);
        let stack = candidate.into_item_stack_with(&|_, _| Some(false));
        assert_eq!(
            stack.data().unwrap().other("test:array"),
            Some(&Tag::ByteArray(vec![7]))
        );
        assert_eq!(stack.data().unwrap().other("test:byte"), None);
    }

    #[test]
    fn loot_trim_patch_decodes_materializes_captures_and_removes() {
        use sunrise_core::nbt::Tag;
        use sunrise_world::loot::{LootStackAccess, LootStackEditAccess};
        let mut candidate = crate::loot_candidate::LootCandidate::from_stack(ItemStack::new(
            "minecraft:iron_chestplate",
            1,
        ));
        let trim = sunrise_core::json_nbt::json_to_tag(
            &json!({"material": "minecraft:gold", "pattern": "minecraft:sentry"}),
        )
        .unwrap();
        candidate.push_component("minecraft:trim".into(), Some(trim));
        let stack = candidate.into_item_stack_with(&|_, _| Some(false));
        assert_eq!(
            stack.data().unwrap().trim.as_ref().cloned(),
            Some((
                crate::item_data::trim::Holder::Named("minecraft:gold".into()),
                crate::item_data::trim::Holder::Named("minecraft:sentry".into())
            ))
        );
        let mut captured = crate::loot_candidate::LootCandidate::from_stack(stack);
        assert_eq!(
            captured.components(),
            &[(
                "minecraft:trim".into(),
                Some(Tag::Compound(
                    [
                        ("material".into(), Tag::String("minecraft:gold".into())),
                        ("pattern".into(), Tag::String("minecraft:sentry".into())),
                    ]
                    .into()
                ))
            )]
        );
        captured.push_component("minecraft:trim".into(), None);
        let stack = captured.into_item_stack_with(&|_, _| Some(false));
        assert!(stack.data().is_none_or(|data| data.trim.is_none()));
    }

    #[test]
    fn an_unrelated_loot_patch_does_not_restore_a_captured_name() {
        use sunrise_world::loot::LootStackEditAccess;
        let mut stack = ItemStack::new("minecraft:stick", 1);
        stack
            .data_mut()
            .set_loot_name("minecraft:custom_name", &json!("Old"));
        let mut candidate = crate::loot_candidate::LootCandidate::from_stack(stack);
        candidate.push_name_value("minecraft:custom_name".into(), json!("New"));
        candidate.push_component("test:byte".into(), Some(sunrise_core::nbt::Tag::Byte(7)));
        let stack = candidate.into_item_stack_with(&|_, _| Some(false));
        assert_eq!(
            stack.data().unwrap().custom_name,
            Some(sunrise_core::nbt::Tag::String("New".into()))
        );
    }

    /// A stick whose prototype carries `minecraft:rarity`, with its own `epic` on top.
    fn epic_stick() -> ItemStack {
        let mut stack = ItemStack::new("minecraft:stick", 1);
        stack.data_mut().set_other(
            "minecraft:rarity",
            Some(sunrise_core::nbt::Tag::String("epic".into())),
        );
        stack
    }

    fn stick_has_rarity(item: &str, id: &str) -> Option<bool> {
        Some(item == "minecraft:stick" && id == "minecraft:rarity")
    }

    fn removal_marker() -> Option<sunrise_core::nbt::Tag> {
        Some(sunrise_core::nbt::Tag::Compound(Default::default()))
    }

    /// #1540: a hook that drops a component the prototype carries leaves the
    /// `!id` marker, as `SetComponentsFunction`'s `!id` does, so the
    /// prototype's value doesn't come back.
    #[test]
    fn a_hook_removing_a_prototype_component_keeps_it_removed() {
        use sunrise_world::loot::LootStackEditAccess;
        let mut candidate = crate::loot_candidate::LootCandidate::from_stack(epic_stick());
        candidate.retain_components(&mut |id, _| id != "minecraft:rarity");
        let stack = candidate.into_item_stack_with(&stick_has_rarity);
        let data = stack.data().expect("the marker is stack data");
        assert_eq!(data.other("minecraft:rarity"), None);
        assert_eq!(data.other("!minecraft:rarity").cloned(), removal_marker());
    }

    /// The same removal written as an explicit `None` entry of the patch.
    #[test]
    fn an_explicit_none_entry_for_a_prototype_component_keeps_it_removed() {
        use sunrise_world::loot::LootStackEditAccess;
        let mut candidate = crate::loot_candidate::LootCandidate::from_stack(epic_stick());
        candidate.push_component("minecraft:rarity".into(), None);
        let stack = candidate.into_item_stack_with(&stick_has_rarity);
        let data = stack.data().expect("the marker is stack data");
        assert_eq!(data.other("minecraft:rarity"), None);
        assert_eq!(data.other("!minecraft:rarity").cloned(), removal_marker());
    }

    /// A component the prototype lacks is simply dropped: no marker, as
    /// `PatchedDataComponentMap.remove` leaves the patch without the type.
    #[test]
    fn a_hook_removing_a_component_the_prototype_lacks_leaves_no_marker() {
        use sunrise_world::loot::LootStackEditAccess;
        let mut stack = ItemStack::new("minecraft:stick", 1);
        stack
            .data_mut()
            .set_other("test:byte", Some(sunrise_core::nbt::Tag::Byte(7)));
        let mut candidate = crate::loot_candidate::LootCandidate::from_stack(stack);
        candidate.retain_components(&mut |id, _| id != "test:byte");
        let stack = candidate.into_item_stack_with(&stick_has_rarity);
        assert!(stack.data().is_none_or(
            |data| data.other("test:byte").is_none() && data.other("!test:byte").is_none()
        ));
    }

    /// A value the hook sets replaces an earlier removal of the same component.
    #[test]
    fn a_hook_setting_a_removed_component_drops_the_marker() {
        use sunrise_world::loot::LootStackEditAccess;
        let mut stack = ItemStack::new("minecraft:stick", 1);
        stack
            .data_mut()
            .set_other("!minecraft:rarity", removal_marker());
        let mut candidate = crate::loot_candidate::LootCandidate::from_stack(stack);
        candidate.push_component(
            "minecraft:rarity".into(),
            Some(sunrise_core::nbt::Tag::String("rare".into())),
        );
        let stack = candidate.into_item_stack_with(&stick_has_rarity);
        let data = stack.data().expect("the value is stack data");
        assert_eq!(
            data.other("minecraft:rarity"),
            Some(&sunrise_core::nbt::Tag::String("rare".into()))
        );
        assert_eq!(data.other("!minecraft:rarity"), None);
    }

    /// A hook may write the `!id` form itself, as `set_components` encodes a
    /// removal: the stack's own value goes and the marker stays.
    #[test]
    fn a_hook_writing_the_removal_form_removes_the_component() {
        use sunrise_world::loot::LootStackEditAccess;
        let mut candidate = crate::loot_candidate::LootCandidate::from_stack(epic_stick());
        candidate.push_component("!minecraft:rarity".into(), removal_marker());
        let stack = candidate.into_item_stack_with(&stick_has_rarity);
        let data = stack.data().expect("the marker is stack data");
        assert_eq!(data.other("minecraft:rarity"), None);
        assert_eq!(data.other("!minecraft:rarity").cloned(), removal_marker());
    }

    /// One id patched twice in a row: the last entry wins, a removal with its
    /// marker and a value without one.
    #[test]
    fn ordered_entries_for_one_component_end_on_the_last() {
        use sunrise_world::loot::LootStackEditAccess;
        let rare = Some(sunrise_core::nbt::Tag::String("rare".into()));
        for (entries, value, marker) in [
            (vec![rare.clone(), None], None, removal_marker()),
            (vec![None, rare.clone()], rare.clone(), None),
        ] {
            let mut candidate = crate::loot_candidate::LootCandidate::from_stack(epic_stick());
            candidate.retain_components(&mut |id, _| id != "minecraft:rarity");
            for entry in entries {
                candidate.push_component("minecraft:rarity".into(), entry);
            }
            let stack = candidate.into_item_stack_with(&stick_has_rarity);
            let data = stack.data();
            assert_eq!(
                data.and_then(|d| d.other("minecraft:rarity")).cloned(),
                value
            );
            assert_eq!(
                data.and_then(|d| d.other("!minecraft:rarity")).cloned(),
                marker
            );
        }
    }

    /// A value the component's setter refuses (custom data that isn't a
    /// compound) changes nothing: an earlier removal and its marker survive.
    #[test]
    fn a_refused_replacement_keeps_the_removal() {
        use sunrise_world::loot::LootStackEditAccess;
        let mut stack = ItemStack::new("minecraft:stick", 1);
        stack
            .data_mut()
            .set_other("!minecraft:custom_data", removal_marker());
        let mut candidate = crate::loot_candidate::LootCandidate::from_stack(stack);
        candidate.push_component(
            "minecraft:custom_data".into(),
            Some(sunrise_core::nbt::Tag::Int(7)),
        );
        let stack = candidate.into_item_stack_with(&|_, _| Some(true));
        let data = stack.data().expect("the marker is stack data");
        assert_eq!(data.other("minecraft:custom_data"), None);
        assert_eq!(
            data.other("!minecraft:custom_data").cloned(),
            removal_marker()
        );
    }

    /// A valid empty banner-pattern list is a set component too: it replaces
    /// an earlier removal, and the stack saves and reloads the empty list.
    #[test]
    fn an_empty_banner_pattern_list_replaces_a_removal() {
        use sunrise_world::loot::LootStackEditAccess;
        let mut stack = ItemStack::new("minecraft:white_banner", 1);
        stack
            .data_mut()
            .set_other("!minecraft:banner_patterns", removal_marker());
        let mut candidate = crate::loot_candidate::LootCandidate::from_stack(stack);
        candidate.push_component(
            "minecraft:banner_patterns".into(),
            Some(sunrise_core::nbt::Tag::List(Vec::new())),
        );
        let stack = candidate.into_item_stack_with(&|_, _| Some(true));
        let saved = crate::item_data::encode_components(&stack, &Items::default());
        let Some(sunrise_core::nbt::Tag::Compound(saved)) = saved else {
            panic!("the empty list is saved");
        };
        assert_eq!(saved.get(&"!minecraft:banner_patterns".into()), None);
        assert_eq!(
            saved.get(&"minecraft:banner_patterns".into()),
            Some(&sunrise_core::nbt::Tag::List(Vec::new()))
        );
        let (_, back) =
            crate::item_data::decode_components(Some(&sunrise_core::nbt::Tag::Compound(saved)));
        assert!(back.is_some_and(|d| d.sets_component("minecraft:banner_patterns")));
    }

    /// The prototype that decides is the final item's: a hook that turns the
    /// stack into a stick and removes its rarity leaves the stick's marker.
    #[test]
    fn the_final_items_prototype_decides_a_removal() {
        use sunrise_world::loot::LootStackEditAccess;
        let mut stack = ItemStack::new("minecraft:stone", 1);
        stack.data_mut().set_other(
            "minecraft:rarity",
            Some(sunrise_core::nbt::Tag::String("epic".into())),
        );
        let mut candidate = crate::loot_candidate::LootCandidate::from_stack(stack);
        candidate.set_item_id("minecraft:stick");
        candidate.retain_components(&mut |id, _| id != "minecraft:rarity");
        let stack = candidate.into_item_stack_with(&stick_has_rarity);
        assert_eq!(&*stack.item, "minecraft:stick");
        let data = stack.data().expect("the marker is stack data");
        assert_eq!(data.other("!minecraft:rarity").cloned(), removal_marker());
    }

    /// A damage removal zeroes the scalar: no saved damage beside its `!id`.
    #[test]
    fn a_damage_removal_zeroes_the_stacks_damage() {
        use sunrise_world::loot::LootStackEditAccess;
        let mut stack = ItemStack::new("minecraft:iron_sword", 1);
        stack.damage = 5;
        let mut candidate = crate::loot_candidate::LootCandidate::from_stack(stack);
        candidate.push_component("minecraft:damage".into(), None);
        let stack = candidate.into_item_stack_with(&|item, id| {
            Some(item == "minecraft:iron_sword" && id == "minecraft:damage")
        });
        assert_eq!(stack.damage, 0);
        let saved = crate::item_data::encode_components(&stack, &Items::default());
        let Some(sunrise_core::nbt::Tag::Compound(saved)) = saved else {
            panic!("the removal is saved");
        };
        assert_eq!(saved.get(&"minecraft:damage".into()), None);
        assert_eq!(
            saved.get(&"!minecraft:damage".into()).cloned(),
            removal_marker()
        );
    }

    /// `from_loot` has no prototypes: a removal it can't judge keeps the
    /// stack's explicit marker rather than dropping it.
    #[test]
    fn from_loot_keeps_an_explicit_removal() {
        let loot = sunrise_world::loot::LootStack {
            item: "minecraft:stick".into(),
            count: 1,
            damage: 0,
            enchantments: Vec::new(),
            potion: None,
            map: false,
            exploration: None,
            ominous_amplifier: None,
            stew_effects: Vec::new(),
            names: Vec::new(),
            components: vec![
                ("!minecraft:rarity".into(), removal_marker()),
                ("minecraft:rarity".into(), None),
            ],
        };
        let stack = ItemStack::from_loot(&loot);
        assert_eq!(
            stack
                .data()
                .and_then(|d| d.other("!minecraft:rarity"))
                .cloned(),
            removal_marker()
        );
    }

    /// #1540 end to end, with the real prototypes: a mod function that removes
    /// the stick's `minecraft:rarity` leaves a stick without it.
    #[test]
    #[ignore = "daily"]
    fn a_mod_function_removing_a_prototype_component_leaves_the_stack_without_it() {
        use sunrise_world::loot::LootStackEditAccess;
        let items = prepared_items().expect("prepared item resources");
        let info = items.get("minecraft:stick").expect("stick");
        assert!(info.components.iter().any(|c| c == "minecraft:rarity"));
        let mut candidate = crate::loot_candidate::LootCandidate::from_stack(epic_stick());
        candidate.retain_components(&mut |id, _| id != "minecraft:rarity");
        let stack = candidate.into_item_stack(&items);
        assert!(!stack.carries_component(Some(info), "minecraft:rarity"));
    }

    /// A function that leaves the stack alone keeps its own form: lending the
    /// live candidate without edits changes nothing.
    #[test]
    fn a_mod_function_that_changes_nothing_keeps_the_stacks_form() {
        let items = Items::for_tests(HashMap::new());
        let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
        stack.enchant("minecraft:sharpness", 3);
        stack
            .data_mut()
            .set_loot_name("minecraft:custom_name", &json!("Excalibur"));
        let before = stack.clone();
        let candidate = crate::loot_candidate::LootCandidate::from_stack(stack);
        let stack = candidate.into_item_stack(&items);
        assert_eq!(stack, before);
    }

    /// An undeclared type on a raw fixture path does not convert or advance random.
    /// Loaded data refuses such an entry before any evaluation can reach it.
    #[test]
    fn an_unregistered_function_consumes_no_random() {
        let items = Items::for_tests(HashMap::new());
        let props = sunrise_world::chunk::BlockProperties::new();
        let vanilla = json!({"pools":[{"rolls":1,"entries":[
            {"type":"minecraft:item","name":"minecraft:stone"}]}]});
        let modded = json!({"pools":[{"rolls":1,"entries":[
            {"type":"minecraft:item","name":"minecraft:stone"}],
            "modifier":[{"type":"mymod:duplicate","factor":2}]}]});
        let mut a = LegacyRandom::new(100);
        let mut b = LegacyRandom::new(100);
        let _ = ctx(&items, None, &props, "minecraft:stone").evaluate(&vanilla, &mut a);
        let expected: Vec<i64> = (0..5).map(|_| b.next_long()).collect();
        let got: Vec<i64> = (0..5).map(|_| a.next_long()).collect();
        assert_eq!(got, expected, "a vanilla table shifted the random");

        let mut c = LegacyRandom::new(100);
        let mut d = LegacyRandom::new(100);
        let drops = ctx(&items, None, &props, "minecraft:stone").evaluate(&modded, &mut c);
        let expected: Vec<i64> = (0..5).map(|_| d.next_long()).collect();
        let got: Vec<i64> = (0..5).map(|_| c.next_long()).collect();
        assert_eq!(got, expected, "an unregistered function advanced random");
        assert_eq!(drops.len(), 1);
        assert_eq!(drops[0].count, 1);
    }

    #[test]
    fn fortune_silk_touch_and_looting() {
        let items = Items::for_tests(HashMap::new());
        let props = sunrise_world::chunk::BlockProperties::new();
        let mut pick = ItemStack::new("minecraft:diamond_pickaxe", 1);
        pick.enchant("minecraft:fortune", 3);
        let ore = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:alternatives","children":[
            {"type":"minecraft:item","condition":"minecraft:tool/can_silk_touch","name":"minecraft:diamond_ore"},
            {"type":"minecraft:item","name":"minecraft:diamond","modifier":[{"type":"minecraft:apply_bonus","enchantment":"minecraft:fortune","formula":"minecraft:ore_drops"}]}]}]}]});
        let mut random = LegacyRandom::new(7);
        let n = 20000;
        let total: u32 = (0..n)
            .map(|_| {
                ctx(&items, Some(&pick), &props, "minecraft:diamond_ore")
                    .evaluate(&ore, &mut random)
                    .iter()
                    .map(|s| s.count)
                    .sum::<u32>()
            })
            .sum();
        // Fortune III ore drops average (1 + 1 + 2 + 3 + 4) / 5 = 2.2 diamonds.
        let mean = f64::from(total) / f64::from(n);
        assert!((mean - 2.2).abs() < 0.05, "{mean}");
        let mut silk = ItemStack::new("minecraft:diamond_pickaxe", 1);
        silk.enchant("minecraft:silk_touch", 1);
        let drops =
            ctx(&items, Some(&silk), &props, "minecraft:diamond_ore").evaluate(&ore, &mut random);
        assert_eq!(&*drops[0].item, "minecraft:diamond_ore");
        let plain = ItemStack::new("minecraft:diamond_pickaxe", 1);
        let drops =
            ctx(&items, Some(&plain), &props, "minecraft:diamond_ore").evaluate(&ore, &mut random);
        assert_eq!((&*drops[0].item, drops[0].count), ("minecraft:diamond", 1));
        // Looting III adds level x uniform(0, 1) on player kills.
        let flesh = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:rotten_flesh","modifier":[
            {"type":"minecraft:set_count","count":{"type":"minecraft:uniform","min":0.0,"max":2.0}},
            {"type":"minecraft:enchanted_count_increase","count":{"type":"minecraft:uniform","min":0.0,"max":1.0},"enchantment":"minecraft:looting"}]}]}]});
        let mut sword = ItemStack::new("minecraft:iron_sword", 1);
        sword.enchant("minecraft:looting", 3);
        let looted: u32 = (0..n)
            .map(|_| {
                ctx(&items, Some(&sword), &props, "")
                    .evaluate(&flesh, &mut random)
                    .iter()
                    .map(|s| s.count)
                    .sum::<u32>()
            })
            .sum();
        let base: u32 = (0..n)
            .map(|_| {
                ctx(&items, None, &props, "")
                    .evaluate(&flesh, &mut random)
                    .iter()
                    .map(|s| s.count)
                    .sum::<u32>()
            })
            .sum();
        assert!(f64::from(looted) / f64::from(n) > f64::from(base) / f64::from(n) + 1.0);
        // Table bonus uses the chance for the tool's level.
        let leaves = json!({"pools":[{"rolls":1,"condition":{"type":"minecraft:table_bonus","enchantment":"minecraft:fortune","chances":[0.0,0.0,0.0,1.0]},"entries":[{"type":"minecraft:item","name":"minecraft:apple"}]}]});
        assert_eq!(
            ctx(&items, Some(&pick), &props, "minecraft:oak_leaves")
                .evaluate(&leaves, &mut random)
                .len(),
            1
        );
        assert!(
            ctx(&items, Some(&plain), &props, "minecraft:oak_leaves")
                .evaluate(&leaves, &mut random)
                .is_empty()
        );
    }

    #[test]
    fn set_potion_gives_the_potion_its_contents() {
        let items = Items::for_tests(HashMap::new());
        let props = sunrise_world::chunk::BlockProperties::new();
        let table = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:splash_potion",
            "modifier":{"type":"minecraft:set_potion","id":"minecraft:fire_resistance"}}]}]});
        let drops = ctx(&items, None, &props, "").evaluate(&table, &mut LegacyRandom::new(1));
        assert_eq!(drops.len(), 1);
        assert_eq!(&*drops[0].item, "minecraft:splash_potion");
        assert_eq!(drops[0].potion(), Some("minecraft:fire_resistance"));
    }

    /// What `levels` answers is what `/loot ... kill`'s two readers use, with
    /// no player at all; a level left alone rolls as if nobody were asked.
    #[test]
    fn the_answered_level_drives_a_kills_count_and_chance() {
        let items = Items::for_tests(HashMap::new());
        let props = sunrise_world::chunk::BlockProperties::new();
        let reads = std::cell::RefCell::new(Vec::new());
        let levels = |_: &mut LegacyRandom, id: &str, level: u32| {
            reads.borrow_mut().push((id.to_owned(), level));
            3
        };
        let answered = LootContext {
            killed_by_player: false,
            entity_levels: Some(&levels),
            ..ctx(&items, None, &props, "")
        };
        let count = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:bone","modifier":[{"type":"minecraft:enchanted_count_increase","count":2.0,"enchantment":"minecraft:looting"}]}]}]});
        let got = answered.evaluate(&count, &mut LegacyRandom::new(7));
        assert_eq!(got[0].count, 7);
        let chance = json!({"pools":[{"rolls":1,"condition":{"type":"minecraft:random_chance_with_enchanted_bonus","enchantment":"minecraft:looting","unenchanted_chance":0.0,"enchanted_chance":1.0},"entries":[{"type":"minecraft:item","name":"minecraft:bone"}]}]});
        assert_eq!(
            answered.evaluate(&chance, &mut LegacyRandom::new(7)).len(),
            1
        );
        let looting = ("minecraft:looting".to_owned(), 0);
        assert_eq!(*reads.borrow(), [looting.clone(), looting]);

        let mut sword = ItemStack::new("minecraft:iron_sword", 1);
        sword.enchant("minecraft:looting", 2);
        let same = |_: &mut LegacyRandom, _: &str, level: u32| level;
        let table = json!({"pools":[{"rolls":1,"condition":{"type":"minecraft:random_chance_with_enchanted_bonus","enchantment":"minecraft:looting","unenchanted_chance":0.5,"enchanted_chance":0.5},"entries":[{"type":"minecraft:item","name":"minecraft:bone","modifier":[{"type":"minecraft:enchanted_count_increase","count":{"type":"minecraft:uniform","min":0.0,"max":3.0},"enchantment":"minecraft:looting"}]}]}]});
        for seed in 0..16 {
            let (mut plain, mut asked) = (LegacyRandom::new(seed), LegacyRandom::new(seed));
            let expected = ctx(&items, Some(&sword), &props, "").evaluate(&table, &mut plain);
            let got = LootContext {
                entity_levels: Some(&same),
                ..ctx(&items, Some(&sword), &props, "")
            }
            .evaluate(&table, &mut asked);
            assert_eq!(
                got.iter().map(|s| s.count).collect::<Vec<_>>(),
                expected.iter().map(|s| s.count).collect::<Vec<_>>()
            );
            assert_eq!(asked.next_long(), plain.next_long());
        }
    }

    /// A draw the answer makes on the roll's random is a draw of the roll.
    #[test]
    fn a_kills_answer_draws_on_the_rolls_own_random() {
        let items = Items::for_tests(HashMap::new());
        let props = sunrise_world::chunk::BlockProperties::new();
        let drawing = |random: &mut LegacyRandom, _: &str, level: u32| {
            random.next_int();
            level
        };
        let context = LootContext {
            entity_levels: Some(&drawing),
            ..ctx(&items, None, &props, "")
        };
        let table = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:bone","modifier":[{"type":"minecraft:enchanted_count_increase","count":1.0,"enchantment":"minecraft:looting"}]}]}]});
        let mut random = LegacyRandom::new(7);
        context.evaluate(&table, &mut random);
        let mut expected = LegacyRandom::new(7);
        expected.next_int();
        assert_eq!(random.next_long(), expected.next_long());
    }

    /// A mod may answer any level, and it counts as `MAX_LOOT_LEVEL`.
    #[test]
    fn a_kills_largest_level_counts_as_the_most_a_table_reads() {
        let items = Items::for_tests(HashMap::new());
        let props = sunrise_world::chunk::BlockProperties::new();
        let huge = |_: &mut LegacyRandom, _: &str, _: u32| u32::MAX;
        let context = LootContext {
            entity_levels: Some(&huge),
            ..ctx(&items, None, &props, "")
        };
        let count = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:bone","modifier":[{"type":"minecraft:enchanted_count_increase","count":2.0,"enchantment":"minecraft:looting"}]}]}]});
        let got = context.evaluate(&count, &mut LegacyRandom::new(7));
        assert_eq!(got[0].count, 1 + 2 * MAX_LOOT_LEVEL as u32);
        let squared = json!({"pools":[{"rolls":1,"condition":{"type":"minecraft:random_chance_with_enchanted_bonus","enchantment":"minecraft:looting","unenchanted_chance":0.0,"enchanted_chance":{"type":"minecraft:levels_squared","added":0.0}},"entries":[{"type":"minecraft:item","name":"minecraft:bone"}]}]});
        assert_eq!(
            context.evaluate(&squared, &mut LegacyRandom::new(7)).len(),
            1
        );
    }

    #[test]
    fn unrelated_entity_reader_does_not_ask_the_block_listener() {
        let items = Items::for_tests(HashMap::new());
        let props = sunrise_world::chunk::BlockProperties::new();
        let reads = std::cell::Cell::new(0);
        let levels = |_: &str, _: u32| {
            reads.set(reads.get() + 1);
            3
        };
        let table = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:bone","modifier":[{"type":"minecraft:enchanted_count_increase","enchantment":"minecraft:looting","count":2.0}]}]}]});
        let drops = items.roll_block_table(
            "minecraft:blocks/stone",
            &table,
            "minecraft:stone",
            &props,
            BlockLootRoll {
                levels: Some(&levels),
                ..Default::default()
            },
            &mut LegacyRandom::new(7),
        );
        assert_eq!(drops.len(), 1);
        assert_eq!(drops[0].count, 1);
        assert_eq!(reads.get(), 0);
    }

    #[test]
    fn unrelated_block_reader_does_not_ask_the_entity_listener() {
        let items = Items::for_tests(HashMap::new());
        let props = sunrise_world::chunk::BlockProperties::new();
        let reads = std::cell::Cell::new(0);
        let levels = |_: &mut LegacyRandom, _: &str, _: u32| {
            reads.set(reads.get() + 1);
            3
        };
        let context = LootContext {
            entity_levels: Some(&levels),
            ..ctx(&items, None, &props, "")
        };
        let table = json!({"pools":[{"rolls":1,"condition":{"type":"minecraft:table_bonus","enchantment":"minecraft:fortune","chances":[0.0,0.0,0.0,1.0]},"entries":[{"type":"minecraft:item","name":"minecraft:bone"}]}]});
        let mut random = LegacyRandom::new(7);
        assert!(context.evaluate(&table, &mut random).is_empty());
        assert_eq!(reads.get(), 0);
        let mut expected = LegacyRandom::new(7);
        expected.next_float();
        assert_eq!(random.next_long(), expected.next_long());
    }

    /// The game's answer is the level a block table's count and condition
    /// use; the tool keeps its own, and the table's draws stay the formula's.
    #[test]
    fn a_game_level_replaces_the_tools_in_a_block_table() {
        let items = Items::for_tests(HashMap::new());
        let props = sunrise_world::chunk::BlockProperties::new();
        let mut pick = ItemStack::new("minecraft:iron_pickaxe", 1);
        pick.enchant("minecraft:fortune", 1);
        let reads = std::cell::RefCell::new(Vec::new());
        let levels = |_: &mut LegacyRandom, id: &str, level: u32| {
            reads.borrow_mut().push((id.to_owned(), level));
            3
        };
        let answered = |pick| LootContext {
            block_levels: Some(&levels),
            target: None,
            target_entity: None,
            interacting_entity: None,
            ..ctx(&items, pick, &props, "minecraft:stone")
        };
        let count = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond","modifier":[{"type":"minecraft:apply_bonus","enchantment":"minecraft:fortune","formula":"minecraft:uniform_bonus_count","parameters":{"bonusMultiplier":2}}]}]}]});
        let mut random = LegacyRandom::new(7);
        let got = answered(Some(&pick)).evaluate(&count, &mut random);
        let mut expected = LegacyRandom::new(7);
        let bonus = expected.next_int_bound(7).unwrap() as u32;
        assert_eq!(got[0].count, 1 + bonus);
        assert_eq!(random.next_int(), expected.next_int());
        assert_eq!(*reads.borrow(), [("minecraft:fortune".to_owned(), 1)]);
        assert_eq!(pick.enchantment_level("minecraft:fortune"), 1);

        let chance = json!({"pools":[{"rolls":1,"condition":{"type":"minecraft:table_bonus","enchantment":"minecraft:fortune","chances":[0.0,0.0,0.0,1.0]},"entries":[{"type":"minecraft:item","name":"minecraft:apple"}]}]});
        assert!(
            ctx(&items, None, &props, "minecraft:stone")
                .evaluate(&chance, &mut LegacyRandom::new(7))
                .is_empty()
        );
        let got = answered(None).evaluate(&chance, &mut LegacyRandom::new(7));
        assert_eq!(&*got[0].item, "minecraft:apple");
        assert_eq!(reads.borrow()[1], ("minecraft:fortune".to_owned(), 0));
    }

    /// A mod can answer any level, and it counts as `MAX_LOOT_LEVEL`: no
    /// formula overflows, and `binomial_with_bonus_count` draws once a level
    /// up to there, so its loop stays short.
    #[test]
    fn the_largest_level_counts_as_the_most_a_table_reads() {
        let items = Items::for_tests(HashMap::new());
        let props = sunrise_world::chunk::BlockProperties::new();
        let levels = |_: &mut LegacyRandom, _: &str, _: u32| u32::MAX;
        let context = LootContext {
            block_levels: Some(&levels),
            target: None,
            target_entity: None,
            interacting_entity: None,
            ..ctx(&items, None, &props, "minecraft:stone")
        };
        for bonus in [
            json!({"type":"minecraft:apply_bonus","enchantment":"minecraft:fortune","formula":"minecraft:ore_drops"}),
            json!({"type":"minecraft:apply_bonus","enchantment":"minecraft:fortune","formula":"minecraft:uniform_bonus_count","parameters":{"bonusMultiplier":2}}),
        ] {
            let table = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond","modifier":[bonus, bonus, bonus]}]}]});
            for seed in 0..32 {
                let drops = context.evaluate(&table, &mut LegacyRandom::new(seed));
                assert!(drops.iter().all(|stack| stack.count <= i32::MAX as u32));
            }
        }
        let binomial = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:wheat_seeds","modifier":[{"type":"minecraft:apply_bonus","enchantment":"minecraft:fortune","formula":"minecraft:binomial_with_bonus_count","parameters":{"extra":3,"probability":1.0}}]}]}]});
        let mut random = LegacyRandom::new(7);
        let drops = context.evaluate(&binomial, &mut random);
        assert_eq!(drops[0].count, 1 + MAX_LOOT_LEVEL as u32 + 3);
        let mut expected = LegacyRandom::new(7);
        for _ in 0..MAX_LOOT_LEVEL + 3 {
            expected.next_float();
        }
        assert_eq!(random.next_long(), expected.next_long());
    }

    /// Vanilla's levels stop at 255, below the bound: such a level reads and
    /// draws in full.
    #[test]
    fn the_largest_vanilla_level_is_read_in_full() {
        let items = Items::for_tests(HashMap::new());
        let props = sunrise_world::chunk::BlockProperties::new();
        let mut pick = ItemStack::new("minecraft:iron_pickaxe", 1);
        pick.enchant("minecraft:fortune", 255);
        let binomial = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:wheat_seeds","modifier":[{"type":"minecraft:apply_bonus","enchantment":"minecraft:fortune","formula":"minecraft:binomial_with_bonus_count","parameters":{"extra":3,"probability":1.0}}]}]}]});
        let context = ctx(&items, Some(&pick), &props, "minecraft:stone");
        let drops = context.evaluate(&binomial, &mut LegacyRandom::new(7));
        assert_eq!(drops[0].count, 1 + 255 + 3);
    }
    /// `/loot give` and every other table this engine rolls: set_name and
    /// set_components reach the stack, as they do in a chest's roll.
    #[test]
    fn set_name_and_set_components_reach_the_rolled_item() {
        let items = Items::for_tests(HashMap::new());
        let props = sunrise_world::chunk::BlockProperties::new();
        let effects = json!({"custom_effects":[{"id":"minecraft:absorption","duration":2400}]});
        let table = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:potion",
            "modifier":[
                {"type":"minecraft:set_components","components":{"minecraft:potion_contents":effects}},
                {"type":"minecraft:set_name","target":"item_name","name":{"text":"Potion of 1-Up","italic":false}},
                {"type":"minecraft:set_name","name":"Lucky"},
                {"type":"minecraft:set_name"}]}]}]});
        let drops = ctx(&items, None, &props, "").evaluate(&table, &mut LegacyRandom::new(1));
        let saved = crate::item_data::encode_components(&drops[0], &Default::default())
            .expect("components");
        let saved = sunrise_game_data::scoreboard::tag_to_json(&saved);
        assert_eq!(saved["minecraft:custom_name"], "Lucky");
        assert_eq!(saved["minecraft:item_name"]["text"], "Potion of 1-Up");
        let effect = &saved["minecraft:potion_contents"]["custom_effects"][0];
        assert_eq!(effect["id"], "minecraft:absorption");
        assert_eq!(effect["duration"], 2400);
        // The potion's effect is where drinking reads it, not only saved.
        assert_eq!(
            drops[0].custom_potion_effects(),
            [("minecraft:absorption".to_owned(), 2400, 0)]
        );
        // `!id` takes away a component the item has by default.
        let apple = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:apple",
            "modifier":{"type":"minecraft:set_components","components":{"!food":{}}}}]}]});
        let drops = ctx(&items, None, &props, "").evaluate(&apple, &mut LegacyRandom::new(1));
        let saved = crate::item_data::encode_components(&drops[0], &Default::default())
            .expect("components");
        assert!(saved.get("!minecraft:food").is_some());
    }

    /// `/loot give` and every other table this dispatcher rolls: the two
    /// enchantment functions (`EnchantWithLevelsFunction`,
    /// `EnchantRandomlyFunction`) put their enchantments on the stack.
    #[test]
    fn loot_enchant_functions_enchant_the_rolled_item() {
        let Some(items) = super::prepared_items() else {
            return;
        };
        let props = sunrise_world::chunk::BlockProperties::new();
        let mut random = LegacyRandom::new(11);
        // A sword at levels 20..39 comes out enchanted, as in an End city.
        let levels = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item",
            "name":"minecraft:diamond_sword","modifier":[
            {"type":"minecraft:enchant_with_levels","levels":{"type":"minecraft:uniform","min":20.0,"max":39.0}}]}]}]});
        let mut enchanted = 0;
        for _ in 0..40 {
            let drops = ctx(&items, None, &props, "").evaluate(&levels, &mut random);
            if drops[0].enchantments().is_empty() {
                continue;
            }
            enchanted += 1;
            assert_eq!(&*drops[0].item, "minecraft:diamond_sword");
            // Levels stay within the enchantment's range.
            for (id, level) in drops[0].enchantments() {
                let max = items
                    .enchantments
                    .get(id)
                    .expect("a known enchantment")
                    .max_level;
                assert!((1..=max).contains(level), "{id} at {level}");
            }
        }
        assert!(
            enchanted > 30,
            "only {enchanted} of 40 rolls were enchanted"
        );
        // `enchant_randomly` on a book makes one enchanted book, its levels
        // stored; the pool is the enchantment tag it names.
        let book = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item",
            "name":"minecraft:book","modifier":[
            {"type":"minecraft:enchant_randomly","options":"#minecraft:on_random_loot"}]}]}]});
        let drops = ctx(&items, None, &props, "").evaluate(&book, &mut random);
        assert_eq!(&*drops[0].item, "minecraft:enchanted_book");
        assert_eq!(drops[0].count, 1);
        assert!(drops[0].enchantments().is_empty());
        let stored = drops[0].stored_enchantments();
        assert_eq!(stored.len(), 1, "{stored:?}");
        assert!(
            items
                .enchantments
                .in_tag("minecraft:on_random_loot", &stored[0].0)
        );
        // An item no enchantment accepts keeps none of them.
        let dirt = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item",
            "name":"minecraft:dirt","modifier":[
            {"type":"minecraft:enchant_with_levels","levels":{"type":"minecraft:constant","value":30.0}}]}]}]});
        let drops = ctx(&items, None, &props, "").evaluate(&dirt, &mut random);
        assert!(drops[0].enchantments().is_empty());
    }

    /// `Mth.nextInt(random, 1, max)` returns `1` without drawing when the
    /// enchantment has one level (`EnchantRandomlyFunction.enchantItem`), so
    /// the rolls after it must not shift. Vanilla with `LegacyRandom(11)`: a
    /// Mending book, then `set_count` uniform 1..64, is 46.
    #[test]
    fn a_single_level_enchantment_draws_no_level() {
        let Some(items) = super::prepared_items() else {
            return;
        };
        let props = sunrise_world::chunk::BlockProperties::new();
        let table = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item",
            "name":"minecraft:book","modifier":[
            {"type":"minecraft:enchant_randomly","options":"minecraft:mending"},
            {"type":"minecraft:set_count","count":{"type":"minecraft:uniform","min":1.0,"max":64.0}}]}]}]});
        let drops = ctx(&items, None, &props, "").evaluate(&table, &mut LegacyRandom::new(11));
        assert_eq!(drops.len(), 1);
        assert_eq!(&*drops[0].item, "minecraft:enchanted_book");
        assert_eq!(
            drops[0].stored_enchantments(),
            [("minecraft:mending".to_owned().into(), 1)],
            "the book keeps its single-level enchantment"
        );
        // 46 is vanilla's count for this seed; an extra level draw gives 28.
        assert_eq!(drops[0].count, 46);
    }

    /// A content root's remainders reach the mods' items; vanilla's stay its own.
    #[test]
    fn mod_items_take_their_crafting_remainders() {
        let root =
            std::env::temp_dir().join(format!("sunrise-craft-remainders-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
        let items = r#"{"minecraft:bowl": {}, "minecraft:milk_bucket": {}, "ruby:stew": {}}"#;
        fs::write(root.join("items.json"), items).unwrap();
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let catalog = |root: &Path| Items::catalog(root, &PackOverlay::NONE, &defaults).unwrap();
        let remainder = |items: &Items, id: &str| items.items[id].crafting_remainder.clone();
        let vanilla = catalog(&root);
        assert_eq!(remainder(&vanilla, "ruby:stew"), None);
        fs::write(
            root.join(MOD_CRAFT_REMAINDERS),
            r#"{"ruby:stew": "minecraft:bowl"}"#,
        )
        .unwrap();
        let modded = catalog(&root);
        assert_eq!(
            remainder(&modded, "ruby:stew").as_deref(),
            Some("minecraft:bowl")
        );
        assert_eq!(
            remainder(&modded, "minecraft:milk_bucket").as_deref(),
            Some("minecraft:bucket")
        );
        assert_eq!(remainder(&modded, "minecraft:bowl"), None);
        fs::remove_dir_all(&root).unwrap();
    }

    /// A content root's templates make the mods' items what they copy (`ItemInfo::is`).
    #[test]
    fn mod_items_are_their_templates() {
        let root =
            std::env::temp_dir().join(format!("sunrise-item-templates-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
        let items = r#"{"minecraft:fishing_rod": {}, "minecraft:stick": {}, "ruby:rod": {}}"#;
        fs::write(root.join("items.json"), items).unwrap();
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let catalog = |root: &Path| Items::catalog(root, &PackOverlay::NONE, &defaults).unwrap();
        let vanilla = catalog(&root);
        assert!(!vanilla.items["ruby:rod"].is("minecraft:fishing_rod"));
        fs::write(
            root.join(MOD_ITEM_TEMPLATES),
            r#"{"ruby:rod": "minecraft:fishing_rod"}"#,
        )
        .unwrap();
        let modded = catalog(&root);
        let rod = &modded.items["ruby:rod"];
        assert_eq!(rod.template.as_deref(), Some("minecraft:fishing_rod"));
        assert!(rod.is("minecraft:fishing_rod"));
        assert!(rod.is("ruby:rod"));
        assert!(!rod.is("minecraft:stick"));
        assert!(modded.items["minecraft:fishing_rod"].is("minecraft:fishing_rod"));
        assert!(!modded.items["minecraft:stick"].is("minecraft:fishing_rod"));
        assert_eq!(modded.items["minecraft:fishing_rod"].template, None);
        fs::remove_dir_all(&root).unwrap();
    }

    /// `ItemInfo::is_bundle_item` checks vanilla's `BundleItem` class (including templated content
    /// items), not merely carrying a default `minecraft:bundle_contents` component.
    #[test]
    fn is_bundle_item_checks_bundle_item_class_not_bundle_contents_default() {
        let root =
            std::env::temp_dir().join(format!("sunrise-bundle-item-class-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
        let items = r#"{
            "minecraft:bundle": {"minecraft:bundle_contents": []},
            "minecraft:red_bundle": {"minecraft:bundle_contents": []},
            "minecraft:stick": {},
            "ruby:pouch": {"minecraft:bundle_contents": []},
            "ruby:dyed_pouch": {"minecraft:bundle_contents": []}
        }"#;
        fs::write(root.join("items.json"), items).unwrap();
        fs::write(
            root.join(MOD_ITEM_TEMPLATES),
            r#"{"ruby:dyed_pouch": "minecraft:red_bundle"}"#,
        )
        .unwrap();
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let catalog = Items::catalog(&root, &PackOverlay::NONE, &defaults).unwrap();
        assert!(catalog.items["minecraft:bundle"].is_bundle_item());
        assert!(catalog.items["minecraft:red_bundle"].is_bundle_item());
        assert!(catalog.items["ruby:dyed_pouch"].is_bundle_item());
        assert!(
            catalog.items["ruby:pouch"]
                .components
                .iter()
                .any(|component| component == "minecraft:bundle_contents")
        );
        assert!(
            !catalog.items["ruby:pouch"].is_bundle_item(),
            "a default bundle_contents component alone does not make an item a BundleItem"
        );
        assert!(!catalog.items["minecraft:stick"].is_bundle_item());
        if let Some(prepared) = prepared_items() {
            for id in VANILLA_BUNDLE_ITEMS {
                assert!(
                    prepared.get(id).is_some_and(ItemInfo::is_bundle_item),
                    "prepared catalog must recognize vanilla BundleItem {id}"
                );
            }
        }
        fs::remove_dir_all(&root).unwrap();
    }
}

/// Minecraft.handleKeybinds: spectators cannot drop from the selected slot.
pub fn take_held_drop(
    slot: &mut Option<ItemStack>,
    all: bool,
    spectator: bool,
) -> Option<ItemStack> {
    if spectator {
        return None;
    }
    let stack = slot.as_mut()?;
    if stack.count == 0 {
        return None;
    }
    let count = if all { stack.count } else { 1 };
    let thrown = ItemStack {
        count,
        ..stack.clone()
    };
    stack.count -= count;
    if stack.count == 0 {
        *slot = None;
    }
    Some(thrown)
}

/// How many of `stack` one slot holds: the `minecraft:max_stack_size` patch
/// when the data carries one (`!minecraft:max_stack_size` pinning it to 1),
/// else the item's own maximum and 64 for an item the registry does not know.
pub fn item_max_stack(items: &Items, stack: &ItemStack) -> u32 {
    let patched = stack.data().and_then(|d| {
        if d.other("!minecraft:max_stack_size").is_some() {
            return Some(1);
        }
        match d.other("minecraft:max_stack_size") {
            Some(sunrise_core::nbt::Tag::Int(n)) if (1..=99).contains(n) => Some(*n as u32),
            _ => None,
        }
    });
    patched.unwrap_or_else(|| items.get(&stack.item).map_or(64, |i| i.max_stack))
}

#[cfg(test)]
mod held_drop_tests {
    use super::*;
    #[test]
    fn one_all_and_empty_drop() {
        let mut slot = Some(ItemStack::new("minecraft:stone", 4));
        assert_eq!(take_held_drop(&mut slot, false, false).unwrap().count, 1);
        assert_eq!(slot.as_ref().unwrap().count, 3);
        assert_eq!(take_held_drop(&mut slot, true, false).unwrap().count, 3);
        assert!(slot.is_none());
        assert!(take_held_drop(&mut slot, false, false).is_none());
    }
    #[test]
    fn spectator_keeps_selected_stack_for_single_and_whole_stack_drops() {
        for all in [false, true] {
            let mut slot = Some(ItemStack::new("minecraft:stone", 4));
            assert!(take_held_drop(&mut slot, all, true).is_none());
            assert_eq!(slot.as_ref().unwrap().count, 4);
            // Leaving spectator allows dropping the same selected stack again.
            assert_eq!(
                take_held_drop(&mut slot, all, false).unwrap().count,
                if all { 4 } else { 1 }
            );
        }
    }
}

#[cfg(test)]
mod villager_food_tests {
    use super::*;

    /// A villager's food is the item's `villager_food` component, read from the registry:
    /// vanilla's four (bread 4, the rest 1) and nothing else.
    #[test]
    fn the_registry_tells_a_villagers_food() {
        let Some(items) = prepared_items() else {
            return;
        };
        let food: Vec<(&str, Option<i32>)> = [
            "minecraft:bread",
            "minecraft:carrot",
            "minecraft:potato",
            "minecraft:beetroot",
            "minecraft:apple",
        ]
        .map(|i| (i, items.get(i).and_then(|i| i.villager_food)))
        .into();
        assert_eq!(
            food,
            [
                ("minecraft:bread", Some(4)),
                ("minecraft:carrot", Some(1)),
                ("minecraft:potato", Some(1)),
                ("minecraft:beetroot", Some(1)),
                ("minecraft:apple", None),
            ]
        );
        // The registry hands its villagers the same table.
        assert_eq!(items.villager_food("minecraft:bread"), Some(4));
        assert_eq!(items.villager_food("minecraft:apple"), Some(0));
    }

    /// A catalog over a content root of its own (`name`) whose bread feeds a villager `bread`
    /// points and whose apple feeds none.
    fn bread_catalog(name: &str, bread: i32) -> Items {
        let root = std::env::temp_dir().join(format!(
            "sunrise-villager-food-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
        let items = format!(
            r#"{{"minecraft:bread": {{"minecraft:villager_food": {{"nutrition": {bread}}}}}, "minecraft:apple": {{}}}}"#
        );
        fs::write(root.join("items.json"), items).unwrap();
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let catalog = Items::catalog(&root, &PackOverlay::NONE, &defaults).unwrap();
        fs::remove_dir_all(&root).unwrap();
        catalog
    }

    /// Each registry answers from its own table: catalogs built after it, or while it is read
    /// on another thread, never change its answers (the last catalog built used to replace the
    /// table every villager read).
    #[test]
    fn other_catalogs_never_change_a_registrys_villager_food() {
        let vanilla = bread_catalog("vanilla", 4);
        let starved = bread_catalog("starved", 0);
        let answers = |items: &Items| {
            [
                items.villager_food("minecraft:bread"),
                items.villager_food("minecraft:apple"),
                items.villager_food("minecraft:carrot"),
            ]
        };
        assert_eq!(answers(&vanilla), [Some(4), Some(0), None]);
        assert_eq!(answers(&starved), [Some(0), Some(0), None]);
        std::thread::scope(|s| {
            for i in 0..4 {
                s.spawn(move || bread_catalog(&format!("fixture-{i}"), 9 + i));
            }
            for _ in 0..200 {
                assert_eq!(answers(&vanilla), [Some(4), Some(0), None]);
                assert_eq!(answers(&starved), [Some(0), Some(0), None]);
            }
        });
        assert_eq!(answers(&vanilla), [Some(4), Some(0), None]);
        assert_eq!(answers(&starved), [Some(0), Some(0), None]);
    }
}
