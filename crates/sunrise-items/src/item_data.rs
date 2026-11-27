//! Item stack components beyond count and damage: enchantments, stored
//! (book) enchantments, `repair_cost`, `custom_name`, `potion_contents` and
//! `trim`, with their NBT form in vanilla's `components` compound.
#[path = "item_data/brewing_fuel.rs"]
pub mod brewing_fuel;
#[path = "item_data/component_values.rs"]
pub mod component_values;
#[path = "item_data/cooking_fuel.rs"]
pub mod cooking_fuel;
pub use sunrise_world::custom_data;
#[path = "item_data/custom_model_data.rs"]
pub mod custom_model_data;
#[path = "item_data/ingredient_components.rs"]
pub mod ingredient_components;
mod persistence;
use crate::banner_layer::BannerLayer;
use crate::items::{ItemInfo, ItemStack};
use persistence::encode_persistent_components;
use sunrise_core::nbt::{Compound, JavaString, Tag};

/// One item attribute modifier, as `ItemAttributeModifiers.Entry` holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct ModifierRow {
    /// The attribute's id (`type` in the component): `minecraft:armor`.
    pub attribute: String,
    /// The modifier's id.
    pub id: String,
    pub amount: f64,
    /// `add_value`, `add_multiplied_base` or `add_multiplied_total`.
    pub operation: String,
    /// The `EquipmentSlotGroup` it applies in: `mainhand`, `armor`, `any`...
    pub slot: String,
}
impl ModifierRow {
    pub fn new(attribute: &str, id: &str, amount: f64, operation: &str, slot: &str) -> Self {
        Self {
            attribute: attribute.to_owned(),
            id: id.to_owned(),
            amount,
            operation: operation.to_owned(),
            slot: slot.to_owned(),
        }
    }
}

/// `ItemAttributeModifiers.CODEC`: the entries of an `attribute_modifiers`
/// list (`type`, `id`, `amount`, `operation`, `slot` defaulting to `any`).
/// An entry missing a field is skipped.
pub fn modifier_rows(tag: &Tag) -> Vec<ModifierRow> {
    use sunrise_game_data::nbt_value::{float, text};
    let Tag::List(entries) = tag else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|e| {
            Some(ModifierRow {
                attribute: sunrise_world::datafix::dynamic::namespaced(&text(e.get("type"))?),
                id: sunrise_world::datafix::dynamic::namespaced(&text(e.get("id"))?),
                amount: float(e.get("amount"))?,
                operation: text(e.get("operation"))?,
                slot: text(e.get("slot")).unwrap_or_else(|| "any".to_owned()),
            })
        })
        .collect()
}
use std::sync::Arc;

/// Enchantment id and level, in insertion order.
pub type Levels = Vec<(Arc<str>, u32)>;
pub mod trim;
pub use trim::Trim;

/// A stack's modelled and preserved components. Start with `Default` (or
/// `ItemStack::data_mut`) and use the fields and setters; component storage may
/// gain private decoded data alongside its saved form.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct StackData {
    /// An explicit zero damage component overriding a damaged prototype.
    damage_set: bool,
    /// An explicit empty `minecraft:banner_patterns`: the component is set
    /// (and saved) though `banner_patterns` holds no layer.
    banner_patterns_set: bool,
    /// `minecraft:enchantments`.
    pub enchantments: Levels,
    /// `minecraft:stored_enchantments` (enchanted books).
    pub stored_enchantments: Levels,
    /// `minecraft:repair_cost` (anvil prior-work penalty).
    pub repair_cost: u32,
    /// An explicit `minecraft:damage` of zero. The damage is outside
    /// `StackData`, and a non-damageable item's prototype has no such
    /// component, so its presence is part of the stack's components.
    pub explicit_zero_damage: bool,
    /// The original `minecraft:custom_name` component; plain text is derived
    /// when gameplay needs it, so styled names have one source of truth.
    pub custom_name: Option<Tag>,
    /// Full kinetic component with its typed rendering timing, decoded together.
    kinetic_weapon: Option<crate::kinetic_weapon::KineticComponent>,
    /// Saved consumable and its typed runtime value, replaced together.
    consumable: Option<Arc<ConsumableComponent>>,
    death_protection: Option<Arc<Vec<crate::items::ConsumeEffect>>>,
    death_protection_refused: bool,
    // The template stays in `other`. Only effects already read from Wire travel with it.
    use_remainder_effects: Option<Arc<[Admitted]>>,
    sulfur_cube_effects: Option<Arc<[Admitted]>>,
    /// Validated nutrition; the original component is preserved in `other`.
    villager_food: Option<i32>,
    /// The fuel patch, decoded alongside its preserved component in `other`.
    cooking_fuel: Option<cooking_fuel::Patch>,
    /// `minecraft:potion_contents.potion`.
    pub potion: Option<Arc<str>>,
    /// The rest of `minecraft:potion_contents` (`custom_color`, `custom_effects`,
    /// `custom_name`), kept as saved.
    pub potion_extra: Vec<(String, Tag)>,
    /// `minecraft:trim`: (material, pattern).
    pub trim: Option<Trim>,
    /// `minecraft:container` (shulker boxes): the non-empty slots.
    pub container: Vec<(u8, ItemStack)>,
    /// `minecraft:charged_projectiles` (a loaded crossbow).
    pub charged: Vec<ItemStack>,
    /// `minecraft:fireworks`: flight duration and explosions (None: the
    /// item's default).
    pub fireworks: Option<(u8, Vec<crate::firework::FireworkExplosion>)>,
    /// `minecraft:map_id` (filled maps).
    pub map_id: Option<i32>,
    /// `minecraft:writable_book_content` pages.
    pub writable_pages: Vec<String>,
    /// Each page's `Filterable.filtered`: empty when no page has one, else one per page.
    pub writable_filtered: Vec<Option<String>>,
    /// `minecraft:written_book_content`.
    pub written: Option<WrittenBook>,
    /// `minecraft:banner_patterns` (banners and shields).
    banner_patterns: Vec<BannerLayer>,
    /// `minecraft:base_color` (a shield's dye).
    pub base_color: Option<Arc<str>>,
    /// The raid captain's banner: `minecraft:banner_patterns` with the
    /// ominous layers, its item name, rarity and hidden tooltip.
    pub ominous_banner: bool,
    /// `minecraft:ominous_bottle_amplifier` (None: the item's default, 0).
    pub ominous_amplifier: Option<u8>,
    /// `minecraft:dyed_color` (leather armour, wolf armour, ... as RGB).
    pub dyed_color: Option<i32>,
    /// `minecraft:firework_explosion` (a firework star).
    pub firework_explosion: Option<crate::firework::FireworkExplosion>,
    /// `minecraft:pot_decorations`: back, left, right, front sherds.
    pub pot_decorations: Option<Box<[Option<ItemStack>; 4]>>,
    /// `minecraft:bundle_contents`.
    pub bundle_contents: Vec<ItemStack>,
    /// The resolved weight of `bundle_contents`, dropped whenever the data is handed out for
    /// editing.
    pub(crate) bundle_weight: crate::bundle::WeightCache,
    /// `BundleContents.selectedItem`: runtime only, never saved or sent.
    pub bundle_selected: BundleSelected,
    /// `minecraft:suspicious_stew_effects`: (effect id, duration).
    pub stew_effects: Vec<(Arc<str>, i32)>,
    /// `minecraft:map_post_processing`: 0 lock, 1 scale.
    pub map_post_processing: Option<u8>,
    /// Components Sunrise does not model, in key order. Custom data is decoded
    /// on insertion; malformed loaded values remain opaque for lossless fallback.
    other: Vec<(String, Tag)>,
    /// What the server sent for this stack, for sending it back unchanged.
    pub wire: Wire,
}
impl StackData {
    /// `PartialEq` with `overlay_enchantments` (when non-empty) and
    /// `overlay_trim` (when `Some`) substituted for `self`'s fields without
    /// cloning. Exhaustive: adding a component must also choose its equality here.
    fn eq_with_overlay(
        &self,
        other: &Self,
        overlay_enchantments: &[(Arc<str>, u32)],
        overlay_trim: Option<&Trim>,
    ) -> bool {
        // The two ItemEnchantments fields use map equality; all other fields
        // retain their existing equality, including ordered list components.
        let Self {
            damage_set,
            banner_patterns_set,
            enchantments,
            stored_enchantments,
            repair_cost,
            explicit_zero_damage,
            custom_name,
            kinetic_weapon,
            consumable,
            death_protection,
            death_protection_refused: _,
            use_remainder_effects: _,
            sulfur_cube_effects: _,
            villager_food,
            cooking_fuel: _, // derived from preserved components
            potion,
            potion_extra,
            trim,
            container,
            charged,
            fireworks,
            map_id,
            writable_pages,
            writable_filtered,
            written,
            banner_patterns,
            base_color,
            ominous_banner,
            ominous_amplifier,
            dyed_color,
            firework_explosion,
            pot_decorations,
            bundle_contents,
            bundle_selected,
            bundle_weight: _,
            stew_effects,
            map_post_processing,
            other: preserved,
            wire,
        } = self;
        let effective_enchantments = if overlay_enchantments.is_empty() {
            enchantments.as_slice()
        } else {
            overlay_enchantments
        };
        let effective_trim = overlay_trim.or(trim.as_ref());
        damage_set == &other.damage_set
            && banner_patterns_set == &other.banner_patterns_set
            && enchantments_equal(effective_enchantments, &other.enchantments)
            && enchantments_equal(stored_enchantments, &other.stored_enchantments)
            && repair_cost == &other.repair_cost
            && explicit_zero_damage == &other.explicit_zero_damage
            && custom_name == &other.custom_name
            && kinetic_weapon == &other.kinetic_weapon
            && consumable == &other.consumable
            && death_protection == &other.death_protection
            && villager_food == &other.villager_food
            && potion == &other.potion
            && potion_extra == &other.potion_extra
            && effective_trim == other.trim.as_ref()
            && container == &other.container
            && charged == &other.charged
            && fireworks == &other.fireworks
            && map_id == &other.map_id
            && writable_pages == &other.writable_pages
            && writable_filtered == &other.writable_filtered
            && written == &other.written
            && banner_patterns == &other.banner_patterns
            && base_color == &other.base_color
            && ominous_banner == &other.ominous_banner
            && ominous_amplifier == &other.ominous_amplifier
            && dyed_color == &other.dyed_color
            && firework_explosion == &other.firework_explosion
            && pot_decorations == &other.pot_decorations
            && bundle_contents == &other.bundle_contents
            && bundle_selected == &other.bundle_selected
            && stew_effects == &other.stew_effects
            && map_post_processing == &other.map_post_processing
            && preserved == &other.other
            && wire == &other.wire
    }
}
impl PartialEq for StackData {
    fn eq(&self, other: &Self) -> bool {
        self.eq_with_overlay(other, &[], None)
    }
}
impl std::hash::Hash for StackData {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Exhaustive like `eq`: whatever `eq` ignores or compares loosely is hashed to match.
        let Self {
            damage_set,
            banner_patterns_set,
            enchantments,
            stored_enchantments,
            repair_cost,
            explicit_zero_damage,
            custom_name,
            kinetic_weapon,
            consumable,
            death_protection,
            death_protection_refused: _,
            use_remainder_effects: _,
            sulfur_cube_effects: _,
            villager_food,
            cooking_fuel: _, // derived from preserved components
            potion,
            potion_extra,
            trim,
            container,
            charged,
            fireworks,
            map_id,
            writable_pages,
            writable_filtered,
            written,
            banner_patterns,
            base_color,
            ominous_banner,
            ominous_amplifier,
            dyed_color,
            firework_explosion,
            pot_decorations,
            bundle_contents,
            bundle_selected: _,
            bundle_weight: _,
            stew_effects,
            map_post_processing,
            other: preserved,
            wire: _,
        } = self;
        damage_set.hash(state);
        banner_patterns_set.hash(state);
        hash_enchantments(enchantments, state);
        hash_enchantments(stored_enchantments, state);
        repair_cost.hash(state);
        explicit_zero_damage.hash(state);
        custom_name.hash(state);
        kinetic_weapon.as_ref().map(|k| k.raw()).hash(state);
        consumable.as_ref().map(|c| &c.raw).hash(state);
        // ConsumeEffect holds floats, so only the effects' kinds are hashed.
        death_protection
            .as_ref()
            .map(|effects| effects.len())
            .hash(state);
        for effect in death_protection.iter().flat_map(|effects| effects.iter()) {
            std::mem::discriminant(effect).hash(state);
        }
        villager_food.hash(state);
        potion.hash(state);
        potion_extra.hash(state);
        trim.hash(state);
        container.hash(state);
        charged.hash(state);
        fireworks.hash(state);
        map_id.hash(state);
        writable_pages.hash(state);
        writable_filtered.hash(state);
        written.hash(state);
        banner_patterns.hash(state);
        base_color.hash(state);
        ominous_banner.hash(state);
        ominous_amplifier.hash(state);
        dyed_color.hash(state);
        firework_explosion.hash(state);
        pot_decorations.hash(state);
        bundle_contents.hash(state);
        stew_effects.hash(state);
        map_post_processing.hash(state);
        preserved.hash(state);
    }
}

/// Hashes enchantments as `enchantments_equal` compares them: as a multiset.
fn hash_enchantments<H: std::hash::Hasher>(levels: &Levels, state: &mut H) {
    use std::hash::Hash;
    if levels.len() < 2 {
        return levels.hash(state);
    }
    let mut sorted: Vec<_> = levels.iter().collect();
    sorted.sort();
    sorted.hash(state);
}

/// ItemEnchantments is a map, but keeps its written order for display. Compare
/// without allocating or reordering either component. The usual same-order
/// case uses slice equality; only reordered entries need the quadratic scan.
fn enchantments_equal(first: &[(Arc<str>, u32)], second: &[(Arc<str>, u32)]) -> bool {
    first == second
        || (first.len() == second.len()
            && first.iter().all(|entry| {
                first.iter().filter(|value| *value == entry).count()
                    == second.iter().filter(|value| *value == entry).count()
            }))
}

use sunrise_world::component_value::{consume_effects::Admitted, source::normalized_identifier};

/// A saved consumable decoded once for use animation, countdown and completion.
#[derive(Clone, Debug, PartialEq)]
struct ConsumableComponent {
    raw: Tag,
    value: Arc<crate::items::ItemConsumable>,
    refused: bool,
}
impl ConsumableComponent {
    fn new(mut raw: Tag, received: Option<&[Admitted]>) -> Option<Arc<Self>> {
        let mut refused = false;
        let effects = crate::items::stack_effects(
            &mut raw,
            "on_consume_effects",
            received,
            Some(&mut refused),
        )
        .ok()?;
        let sound = raw.get("sound");
        let value = crate::items::ItemConsumable {
            seconds: raw
                .get("consume_seconds")
                .and_then(crate::properties::component_float)
                .unwrap_or(1.6),
            animation: match raw.get("animation") {
                Some(Tag::String(name)) => {
                    crate::use_animation::animation_name(name.0.iter().copied())
                }
                _ => sunrise_core::use_animation::UseAnimation::Eat,
            },
            sound: sound
                .and_then(|sound| string(sound.get("sound_id").unwrap_or(sound)))
                .unwrap_or_else(|| "minecraft:entity.generic.eat".to_owned()),
            particles: raw
                .get("has_consume_particles")
                .and_then(component_bool)
                .unwrap_or(true),
            effects,
        };
        Some(Arc::new(Self {
            raw,
            value: Arc::new(value),
            refused,
        }))
    }
}
/// The component patch a server sent for a stack, as the creative slot packet
/// writes it, and the stack Sunrise decoded from it, to tell whether anything
/// changed since. Like vanilla's `BundleContents.selectedItem` it is left out
/// when stacks are compared.
#[derive(Clone, Debug, Default)]
pub struct Wire(Option<Arc<WireData>>);

#[derive(Debug)]
struct WireData {
    decoded: ItemStack,
    patch: Arc<[u8]>,
    effect_mapping: Option<Arc<()>>,
    admitted_effects: Arc<[Admitted]>,
    effect_refused: bool,
}

impl Wire {
    pub fn new(decoded: ItemStack, patch: Arc<[u8]>) -> Self {
        Self::from_network(decoded, patch, None, Arc::default(), false)
    }

    pub fn from_network(
        decoded: ItemStack,
        patch: Arc<[u8]>,
        effect_mapping: Option<Arc<()>>,
        admitted_effects: Arc<[Admitted]>,
        effect_refused: bool,
    ) -> Self {
        Self(Some(Arc::new(WireData {
            decoded,
            patch,
            effect_mapping,
            admitted_effects,
            effect_refused,
        })))
    }

    /// The server's patch, whether or not the stack changed since.
    pub fn patch(&self) -> Option<&[u8]> {
        self.0.as_ref().map(|wire| wire.patch.as_ref())
    }

    /// The effects projected from this exact patch, not from edits made since.
    pub fn admitted_effects(&self) -> &[Admitted] {
        self.0
            .as_ref()
            .map_or(&[], |wire| wire.admitted_effects.as_ref())
    }
}
impl PartialEq for Wire {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
/// A bundle's selected item index (-1: none). Like vanilla's
/// `BundleContents.equals` it is left out when stacks are compared.
#[derive(Clone, Copy, Debug)]
pub struct BundleSelected(pub i32);
impl Default for BundleSelected {
    fn default() -> Self {
        Self(-1)
    }
}
impl PartialEq for BundleSelected {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
/// The components `components_tag` writes from modelled fields.
const MODELLED: &[&str] = &[
    "minecraft:banner_patterns",
    "minecraft:base_color",
    "minecraft:bundle_contents",
    "minecraft:charged_projectiles",
    "minecraft:container",
    "minecraft:consumable",
    "minecraft:custom_name",
    "minecraft:damage",
    "minecraft:dyed_color",
    "minecraft:enchantments",
    "minecraft:firework_explosion",
    "minecraft:fireworks",
    "minecraft:kinetic_weapon",
    "minecraft:map_id",
    "minecraft:map_post_processing",
    "minecraft:ominous_bottle_amplifier",
    "minecraft:pot_decorations",
    "minecraft:potion_contents",
    "minecraft:repair_cost",
    "minecraft:stored_enchantments",
    "minecraft:suspicious_stew_effects",
    "minecraft:trim",
    "minecraft:writable_book_content",
    "minecraft:written_book_content",
];
impl StackData {
    pub fn custom_name_plain(&self) -> Option<String> {
        self.custom_name.as_ref().and_then(read_text)
    }

    /// The saved kinetic component and its decoded rendering timing.
    pub fn kinetic_weapon(&self) -> Option<&crate::kinetic_weapon::KineticComponent> {
        self.kinetic_weapon.as_ref()
    }
    /// Components whose stack templates or own values can carry consume effects.
    pub const CONSUME_EFFECT_COMPONENTS: [&str; 8] = [
        "minecraft:consumable",
        "minecraft:death_protection",
        "minecraft:bundle_contents",
        "minecraft:charged_projectiles",
        "minecraft:container",
        "minecraft:pot_decorations",
        "minecraft:use_remainder",
        "minecraft:sulfur_cube_content",
    ];

    /// Materialize an opaque stack template when it is used, not when its parent is read.
    pub fn template_stack(&self, id: &str) -> Option<ItemStack> {
        if self
            .other
            .iter()
            .any(|(key, _)| key.strip_prefix('!') == Some(id))
        {
            return None;
        }
        let effects = match id {
            "minecraft:use_remainder" => self.use_remainder_effects.as_ref(),
            "minecraft:sulfur_cube_content" => self.sulfur_cube_effects.as_ref(),
            _ => return None,
        };
        template_from_with(self.other(id)?, effects)
    }

    fn effect_templates(&self) -> impl Iterator<Item = (&'static str, &ItemStack)> {
        self.bundle_contents
            .iter()
            .map(|s| ("minecraft:bundle_contents", s))
            .chain(
                self.charged
                    .iter()
                    .map(|s| ("minecraft:charged_projectiles", s)),
            )
            .chain(
                self.container
                    .iter()
                    .map(|(_, s)| ("minecraft:container", s)),
            )
            .chain(
                self.pot_decorations
                    .iter()
                    .flat_map(|sides| sides.iter().flatten())
                    .map(|s| ("minecraft:pot_decorations", s)),
            )
    }

    /// Whether decoding this override (including already materialized children) refused an effect.
    pub fn refused_consume_effect_component(&self, id: &str) -> bool {
        match id {
            "minecraft:consumable" => self
                .consumable
                .as_ref()
                .map_or_else(|| self.other(id).is_some(), |value| value.refused),
            "minecraft:death_protection" => {
                self.death_protection_refused
                    || (self.death_protection.is_none() && self.other(id).is_some())
            }
            _ => self
                .effect_templates()
                .any(|(name, stack)| name == id && stack.has_refused_consume_effects()),
        }
    }

    /// Find an effect retained at admission without reinvoking its map codec.
    pub fn find_admitted_consume_effect(
        &self,
        matches: &impl Fn(&Admitted) -> bool,
    ) -> Option<Admitted> {
        self.consumable
            .iter()
            .flat_map(|c| &c.value.effects)
            .chain(
                self.death_protection
                    .iter()
                    .flat_map(|effects| effects.iter()),
            )
            .find_map(|effect| match effect {
                crate::items::ConsumeEffect::Native(effect) => effect
                    .registered
                    .as_ref()
                    .filter(|effect| matches(effect))
                    .cloned(),
                _ => None,
            })
            .or_else(|| {
                self.use_remainder_effects
                    .iter()
                    .chain(&self.sulfur_cube_effects)
                    .flat_map(|effects| effects.iter())
                    .find(|effect| matches(effect))
                    .cloned()
            })
            .or_else(|| {
                self.effect_templates()
                    .find_map(|(_, stack)| stack.data()?.find_admitted_consume_effect(matches))
            })
    }

    /// Borrow preserved components without bypassing their insertion codecs.
    pub fn preserved_components(&self) -> &[(String, Tag)] {
        &self.other
    }
    /// A preserved component's saved value.
    pub fn other(&self, id: &str) -> Option<&Tag> {
        if id == "minecraft:custom_name" {
            return self.custom_name.as_ref();
        }
        if id == "minecraft:kinetic_weapon" {
            return self.kinetic_weapon.as_ref().map(|k| k.raw());
        }
        if id == "minecraft:consumable"
            && let Some(consumable) = &self.consumable
        {
            return Some(&consumable.raw);
        }
        self.other.iter().find(|(k, _)| k == id).map(|(_, v)| v)
    }
    /// A preserved value as serialized, including components represented by
    /// the compact raid patch. Modeled fields such as layers have typed accessors.
    pub fn saved_other(&self, id: &str) -> Option<&Tag> {
        self.implicit_banner_values()
            .find_map(|(key, value)| (*key == id).then_some(value))
            .or_else(|| self.other(id))
    }

    fn implicit_banner_values(&self) -> impl Iterator<Item = &(&'static str, Tag)> {
        self.ominous_banner
            .then(ominous_banner_components)
            .into_iter()
            .flatten()
    }

    /// Whether the stack's own data sets component `id`, read from the live fields: the same
    /// components `components_tag` saves, plus the transient ones it leaves out
    /// (`map_post_processing`), without encoding anything.
    pub fn sets_component(&self, id: &str) -> bool {
        if self.other.iter().any(|(key, _)| key == id) {
            return true;
        }
        match id {
            "minecraft:damage" => self.damage_set,
            "minecraft:enchantments" => !self.enchantments.is_empty(),
            "minecraft:stored_enchantments" => !self.stored_enchantments.is_empty(),
            "minecraft:repair_cost" => self.repair_cost > 0,
            "minecraft:custom_name" => self.custom_name.is_some(),
            "minecraft:potion_contents" => self.potion.is_some() || !self.potion_extra.is_empty(),
            "minecraft:trim" => self.trim.is_some(),
            "minecraft:map_id" => self.map_id.is_some(),
            "minecraft:banner_patterns" => {
                self.ominous_banner || self.banner_patterns_set || !self.banner_patterns.is_empty()
            }
            "minecraft:base_color" => self.base_color.is_some(),
            "minecraft:writable_book_content" => !self.writable_pages.is_empty(),
            "minecraft:written_book_content" => self.written.is_some(),
            "minecraft:fireworks" => self.fireworks.is_some(),
            "minecraft:firework_explosion" => self.firework_explosion.is_some(),
            "minecraft:dyed_color" => self.dyed_color.is_some(),
            "minecraft:pot_decorations" => self.pot_decorations.is_some(),
            "minecraft:bundle_contents" => !self.bundle_contents.is_empty(),
            "minecraft:suspicious_stew_effects" => !self.stew_effects.is_empty(),
            "minecraft:charged_projectiles" => !self.charged.is_empty(),
            "minecraft:ominous_bottle_amplifier" => self.ominous_amplifier.is_some(),
            "minecraft:container" => !self.container.is_empty(),
            "minecraft:map_post_processing" => self.map_post_processing.is_some(),
            // An ominous banner's implied components (`components_tag`).
            "minecraft:item_name" | "minecraft:rarity" | "minecraft:tooltip_display"
                if self.ominous_banner =>
            {
                true
            }
            _ => self.other(id).is_some(),
        }
    }
    /// `SetNameFunction.run`: retain a loot name as its original component.
    pub fn set_loot_name(&mut self, id: &str, text: &serde_json::Value) {
        let tag = sunrise_game_data::scoreboard::json_to_tag(text);
        self.set_other(id, Some(tag));
    }
    /// Apply an ordered loot component patch without changing its NBT type.
    /// None removes the component. Trim retains its modelled representation.
    pub fn set_loot_component(&mut self, id: &str, value: &Option<Tag>) {
        let tag = value.clone();
        match id {
            // `set_banner_pattern` (and `set_components`) write the layers a
            // banner or shield wears; the renderer and the tooltip read them
            // from `banner_patterns`, not from a tag in `other`.
            "minecraft:banner_patterns" => {
                *self.banner_layers_mut() = crate::banner_layer::layers_from_nbt(tag.as_ref());
                self.banner_patterns_set = tag.is_some() && self.banner_patterns.is_empty();
            }
            // `set_book_cover` (and `set_components`) write the book's title,
            // author, generation and pages as one component: the fields the
            // tooltip and the reading screen use come back out of it.
            "minecraft:written_book_content" => {
                self.written = tag.as_ref().map(written_book_from_tag);
            }
            "minecraft:trim" => {
                self.trim = tag.as_ref().and_then(trim::from_nbt);
            }
            // The scalar lives on ItemStack; retain explicit zero presence here
            // so a non-durable prototype can carry its own durability.
            "minecraft:damage" => {
                if tag.is_none() {
                    // A removal: the stack's own damage is gone (its scalar is
                    // zeroed by the caller that owns it).
                    self.damage_set = false;
                }
                self.explicit_zero_damage = tag.as_ref().is_some_and(|value| {
                    sunrise_world::datafix::dynamic::as_int(Some(value), 0) == 0
                });
                self.set_other(id, tag);
            }
            "minecraft:custom_name" => self.set_other(id, tag),
            "!minecraft:villager_food" => {
                self.set_other("minecraft:villager_food", None);
                self.set_other(
                    id,
                    Some(tag.unwrap_or_else(|| Tag::Compound(Default::default()))),
                );
            }
            _ if MODELLED.contains(&id) => {
                if self.ominous_banner && is_ominous_component(id) {
                    self.materialize_ominous_banner();
                }
                let decoded =
                    tag.and_then(|tag| decode_components(Some(&Tag::compound([(id, tag)]))).1);
                self.apply_component_changes(decoded, std::iter::once(id));
            }
            _ => self.set_other(id, tag),
        }
    }
    /// Set (or with None remove) a preserved component.
    /// This setter deliberately remains infallible. Invalid custom data and opaque
    /// effect payloads refuse replacement without changing the previous value or cache.
    pub fn set_other(&mut self, id: &str, value: Option<Tag>) {
        if matches!(id, cooking_fuel::ID | cooking_fuel::REMOVED) {
            if value.is_some() {
                let opposite = if id == cooking_fuel::ID {
                    cooking_fuel::REMOVED
                } else {
                    cooking_fuel::ID
                };
                self.other.retain(|(key, _)| key != opposite);
            }
            if value.is_some() || self.other(id).is_some() {
                self.cooking_fuel = value.as_ref().map(|tag| {
                    if id == cooking_fuel::ID {
                        cooking_fuel::Patch::from_value(tag)
                    } else {
                        cooking_fuel::Patch::Unavailable
                    }
                });
            }
        }
        let mut value = if id == "minecraft:custom_data" {
            match value {
                Some(raw) => match custom_data::decode(raw) {
                    Ok(decoded) => Some(decoded),
                    Err(_) => return,
                },
                None => None,
            }
        } else {
            value
        };
        if id == "minecraft:villager_food" {
            self.villager_food = match &value {
                Some(raw) => {
                    let Some(nutrition) = raw
                        .get("nutrition")
                        .and_then(crate::properties::component_int)
                        .filter(|&nutrition| nutrition > 0)
                    else {
                        return;
                    };
                    self.other
                        .retain(|(key, _)| key != "!minecraft:villager_food");
                    Some(nutrition)
                }
                None => None,
            };
        }
        if id == "minecraft:death_protection" {
            self.death_protection = match value.as_mut() {
                Some(raw) => match crate::items::stack_effects(raw, "death_effects", None, None) {
                    Ok(effects) => Some(Arc::new(effects)),
                    Err(_) => return,
                },
                None => None,
            };
            self.death_protection_refused = false;
        }
        if id == "minecraft:consumable" {
            let replacement = match value {
                Some(raw) => match ConsumableComponent::new(raw, None) {
                    Some(component) if !component.refused => Some(component),
                    _ => return,
                },
                None => None,
            };
            self.other.retain(|(k, _)| k != id);
            self.consumable = replacement;
            return;
        }
        match id {
            "minecraft:use_remainder" => self.use_remainder_effects = None,
            "minecraft:sulfur_cube_content" => self.sulfur_cube_effects = None,
            _ => (),
        }
        let edits_raid_patch = (self.ominous_banner
            || self.banner_patterns.len() == OMINOUS_LAYERS.len())
            && is_ominous_component(id.strip_prefix('!').unwrap_or(id));
        if edits_raid_patch {
            self.materialize_ominous_banner();
        }
        self.other.retain(|(k, _)| k != id);
        if id == "minecraft:kinetic_weapon" {
            self.kinetic_weapon = value.map(crate::kinetic_weapon::KineticComponent::new);
            return;
        }
        if id == "minecraft:custom_name" {
            self.custom_name = value
                .filter(|tag| read_text(tag).is_some())
                .map(collapsed_text);
            return;
        }
        if let Some(v) = value {
            self.other.push((id.to_owned(), v));
            self.other.sort_by(|a, b| a.0.cmp(&b.0));
        }
        if edits_raid_patch {
            self.compact_ominous_banner();
        }
    }
    /// `DebugStickState.properties().get(block)`: the property selected for
    /// `block` in `minecraft:debug_stick_state`.
    pub fn debug_stick_property(&self, block: &str) -> Option<String> {
        let Tag::Compound(map) = self.other("minecraft:debug_stick_state")? else {
            return None;
        };
        match map.get(&JavaString::from(block))? {
            Tag::String(property) => Some(property.to_string_lossy()),
            _ => None,
        }
    }
    /// `DebugStickState.withProperty(block, property)`.
    pub fn set_debug_stick_property(&mut self, block: &str, property: &str) {
        let mut map = match self.other("minecraft:debug_stick_state") {
            Some(Tag::Compound(map)) => map.clone(),
            _ => Default::default(),
        };
        map.insert(
            JavaString::from(block),
            Tag::String(JavaString::from(property)),
        );
        self.set_other("minecraft:debug_stick_state", Some(Tag::Compound(map)));
    }
}
/// `Raid.getBannerComponentPatch`: the ominous banner's pattern layers.
pub const OMINOUS_LAYERS: [(&str, &str); 8] = [
    ("rhombus", "cyan"),
    ("stripe_bottom", "light_gray"),
    ("stripe_center", "gray"),
    ("border", "light_gray"),
    ("stripe_middle", "black"),
    ("half_horizontal", "light_gray"),
    ("circle", "light_gray"),
    ("border", "black"),
];

fn ominous_banner_layers() -> &'static [BannerLayer; OMINOUS_LAYERS.len()] {
    static LAYERS: std::sync::OnceLock<[BannerLayer; OMINOUS_LAYERS.len()]> =
        std::sync::OnceLock::new();
    LAYERS.get_or_init(|| {
        OMINOUS_LAYERS
            .map(|(pattern, color)| BannerLayer::new(&format!("minecraft:{pattern}"), color))
    })
}

/// The independent component values represented by the compact raid-banner flag.
fn ominous_banner_components() -> &'static [(&'static str, Tag); 4] {
    static COMPONENTS: std::sync::OnceLock<[(&str, Tag); 4]> = std::sync::OnceLock::new();
    COMPONENTS.get_or_init(|| {
        [
            (
                "minecraft:banner_patterns",
                crate::banner_layer::layers_to_nbt(ominous_banner_layers()),
            ),
            (
                "minecraft:tooltip_display",
                Tag::compound([(
                    "hidden_components",
                    Tag::List(vec![Tag::String("minecraft:banner_patterns".into())]),
                )]),
            ),
            (
                "minecraft:item_name",
                Tag::compound([(
                    "translate",
                    Tag::String("block.minecraft.ominous_banner".into()),
                )]),
            ),
            ("minecraft:rarity", Tag::String("uncommon".into())),
        ]
    })
}

fn is_ominous_component(id: &str) -> bool {
    ominous_banner_components()
        .iter()
        .any(|(component, _)| *component == id)
}

/// `WrittenBookContent` with the pages as plain text.
#[derive(Clone, Debug, Default, PartialEq, Hash)]
pub struct WrittenBook {
    pub title: String,
    pub author: String,
    pub generation: u8,
    pub pages: Vec<String>,
    /// Each page's component when it is not a plain string, so that its
    /// click events survive; empty or one per page.
    pub page_tags: Vec<Tag>,
    /// `WrittenBookContent.resolved`: a book whose selectors and scores have
    /// been resolved is never resolved again.
    pub resolved: bool,
}
impl StackData {
    /// Canonicalize only an exact match to the flag's independent values.
    fn compact_ominous_banner(&mut self) {
        if self.ominous_banner
            || self.banner_patterns.len() != OMINOUS_LAYERS.len()
            || self.banner_patterns.as_slice() != ominous_banner_layers()
        {
            return;
        }
        let components = ominous_banner_components();
        if !components
            .iter()
            .filter(|(id, _)| *id != "minecraft:banner_patterns")
            .all(|(id, value)| self.other(id) == Some(value))
            || self
                .other
                .iter()
                .any(|(id, _)| id.strip_prefix('!').is_some_and(is_ominous_component))
        {
            return;
        }
        self.banner_patterns = Vec::new();
        self.banner_patterns_set = false;
        self.other.retain(|(id, _)| !is_ominous_component(id));
        self.ominous_banner = true;
    }

    /// The effective layers, including those stored as the compact raid patch.
    pub fn banner_layers(&self) -> &[BannerLayer] {
        if self.ominous_banner {
            ominous_banner_layers()
        } else {
            &self.banner_patterns
        }
    }

    /// A direct layer edit first expands the entire raid patch.
    pub fn banner_layers_mut(&mut self) -> &mut Vec<BannerLayer> {
        self.materialize_ominous_banner();
        &mut self.banner_patterns
    }

    fn materialize_ominous_banner(&mut self) {
        if self.ominous_banner {
            let components = ominous_banner_components();
            let tag = Tag::compound(components.iter().cloned());
            let (_, extra) = decode_components_inner(Some(&tag), false, None);
            self.ominous_banner = false;
            self.apply_component_changes(extra, components.iter().map(|(key, _)| *key));
        }
    }

    fn apply_component_changes<'a>(
        &mut self,
        extra: Option<Box<StackData>>,
        changed: impl Iterator<Item = &'a str> + Clone,
    ) {
        let mut replacement = extra.map_or_else(StackData::default, |data| *data);
        let changes = |id: &str| changed.clone().any(|entry| entry == id);
        let data = self;
        macro_rules! replace_fields {
            ($($field:ident),+) => {{ $(data.$field = std::mem::take(&mut replacement.$field);)+ }};
        }
        for id in changed.clone() {
            match id {
                "minecraft:damage" => replace_fields!(damage_set, explicit_zero_damage),
                "minecraft:enchantments" => replace_fields!(enchantments),
                "minecraft:stored_enchantments" => replace_fields!(stored_enchantments),
                "minecraft:repair_cost" => replace_fields!(repair_cost),
                "minecraft:custom_name" => replace_fields!(custom_name),
                "minecraft:potion_contents" => replace_fields!(potion, potion_extra),
                "minecraft:death_protection" => {
                    replace_fields!(death_protection, death_protection_refused)
                }
                cooking_fuel::ID => replace_fields!(cooking_fuel),
                "minecraft:consumable" => replace_fields!(consumable),
                "minecraft:use_remainder" => replace_fields!(use_remainder_effects),
                "minecraft:sulfur_cube_content" => replace_fields!(sulfur_cube_effects),
                "minecraft:villager_food" => replace_fields!(villager_food),
                "minecraft:kinetic_weapon" => replace_fields!(kinetic_weapon),
                "minecraft:map_id" => replace_fields!(map_id),
                "minecraft:banner_patterns" => {
                    replace_fields!(banner_patterns, banner_patterns_set)
                }
                "minecraft:base_color" => replace_fields!(base_color),
                "minecraft:writable_book_content" => {
                    replace_fields!(writable_pages, writable_filtered)
                }
                "minecraft:written_book_content" => replace_fields!(written),
                "minecraft:fireworks" => replace_fields!(fireworks),
                "minecraft:firework_explosion" => replace_fields!(firework_explosion),
                "minecraft:dyed_color" => replace_fields!(dyed_color),
                "minecraft:pot_decorations" => replace_fields!(pot_decorations),
                "minecraft:bundle_contents" => {
                    replace_fields!(bundle_contents, bundle_selected);
                    data.bundle_weight = Default::default();
                }
                "minecraft:suspicious_stew_effects" => replace_fields!(stew_effects),
                "minecraft:map_post_processing" => replace_fields!(map_post_processing),
                "minecraft:charged_projectiles" => replace_fields!(charged),
                "minecraft:container" => replace_fields!(container),
                "minecraft:ominous_bottle_amplifier" => replace_fields!(ominous_amplifier),
                "minecraft:trim" => replace_fields!(trim),
                _ => {}
            };
        }
        let edited = |key: &str| changes(key.strip_prefix('!').unwrap_or(key));
        data.other.retain(|(key, _)| !edited(key));
        data.other
            .extend(replacement.other.into_iter().filter(|(key, _)| edited(key)));
        data.other.sort_by(|a, b| a.0.cmp(&b.0));
    }

    pub fn is_empty(&self) -> bool {
        self.wire.0.is_none() && *self == Self::default()
    }
}

/// `BlockItem.onDestroyed` (`ItemStack.onDestroyed` for a container item): the
/// stack's `container` is emptied and the stacks it held come back in slot
/// order — `ItemUtils.onContainerDestroyed` drops each where the entity lay.
pub fn destroyed_container(stack: &mut ItemStack) -> Vec<ItemStack> {
    let Some(data) = stack.extra.as_deref_mut() else {
        return Vec::new();
    };
    if data.container.is_empty() {
        return Vec::new();
    }
    // `set(CONTAINER, ItemContainerContents.EMPTY)`: no slot left filled.
    let mut held = std::mem::take(&mut data.container);
    stack.tidy();
    // `nonEmptyItemCopyStream` walks the 256 slots in order.
    held.sort_by_key(|(slot, _)| *slot);
    held.into_iter().map(|(_, stack)| stack).collect()
}

/// [`component_bool`] for a component held as JSON, as the item table reads its defaults: a
/// mod's declared patch reaches them from NBT, where a boolean is a byte, so a number is a
/// boolean too (`NbtOps.getBooleanValue`: true unless its double value is zero).
pub(crate) trait CodecBool {
    fn codec_bool(&self) -> Option<bool>;
}
impl CodecBool for serde_json::Value {
    fn codec_bool(&self) -> Option<bool> {
        self.as_bool().or_else(|| self.as_f64().map(|d| d != 0.0))
    }
}

/// `NbtOps.getBooleanValue`, used by boolean component codecs: every numeric tag
/// is true unless its double value is zero. In particular, fractions stay true.
pub fn component_bool(value: &Tag) -> Option<bool> {
    Some(match value {
        Tag::Byte(v) => *v != 0,
        Tag::Short(v) => *v != 0,
        Tag::Int(v) => *v != 0,
        Tag::Long(v) => *v != 0,
        Tag::Float(bits) => f32::from_bits(*bits) != 0.0,
        Tag::Double(bits) => f64::from_bits(*bits) != 0.0,
        _ => return None,
    })
}

/// `FoodProperties.DIRECT_CODEC`: required numeric nutrition/saturation and an
/// optional NbtOps boolean. Unknown fields are ignored; floats are unrestricted.
/// Reads borrowed NBT into the existing copyable value without allocating.
pub fn food_properties(
    value: &Tag,
) -> Result<sunrise_world::food_properties::FoodProperties, &'static str> {
    if !matches!(value, Tag::Compound(_)) {
        return Err("food must be a map");
    }
    let nutrition = value
        .get("nutrition")
        .and_then(crate::properties::component_int)
        .ok_or("nutrition must be a number")?;
    if nutrition < 0 {
        return Err("nutrition must be non-negative");
    }
    let saturation = value
        .get("saturation")
        .and_then(crate::properties::component_float)
        .ok_or("saturation must be a number")?;
    let can_always_eat = value
        .get("can_always_eat")
        .map(component_bool)
        .unwrap_or(Some(false))
        .ok_or("can_always_eat must be a number")?;
    Ok(sunrise_world::food_properties::FoodProperties {
        nutrition,
        saturation,
        can_always_eat,
    })
}

impl ItemStack {
    /// `ItemStack.isEmpty`: air and zero-count stacks are both empty.
    pub fn is_empty(&self) -> bool {
        self.count == 0 || &*self.item == "minecraft:air"
    }

    pub fn new(item: &str, count: u32) -> Self {
        Self {
            item: Arc::from(item),
            count,
            damage: 0,
            extra: None,
        }
    }
    /// The stack `ItemStack.CODEC` reads from `tag` (`id`, `count`,
    /// `components`), as `item_nbt::item_from_nbt` does; None for air or an
    /// id that is no valid identifier. The id is not looked up in a registry.
    pub fn from_nbt(tag: &sunrise_core::nbt::Compound) -> Option<Self> {
        crate::item_nbt::item_from_nbt(&Tag::Compound(tag.clone()))
    }
    pub fn data(&self) -> Option<&StackData> {
        self.extra.as_deref()
    }
    /// Whether the stack carries component `id`: its own data sets it, or its item has it by
    /// default and the stack's changes don't remove it (a `!id` entry). Reads the live
    /// components, so nothing is encoded.
    pub fn carries_component(&self, info: Option<&ItemInfo>, id: &str) -> bool {
        let data = self.data();
        let own = if id == "minecraft:damage" {
            self.damage > 0 || data.is_some_and(|d| d.sets_component(id))
        } else {
            data.is_some_and(|d| d.sets_component(id))
        };
        if own {
            return true;
        }
        if data.is_some_and(|d| d.other.iter().any(|(k, _)| k.strip_prefix('!') == Some(id))) {
            return false;
        }
        info.is_some_and(|info| info.components.iter().any(|c| c == id))
    }
    /// `ItemInstance.getMaxStackSize`: an override, the prototype component, or 1 if absent.
    /// Returns the raw value for range validation, or `None` for a malformed legacy patch.
    pub fn effective_max_stack_size(&self, info: Option<&ItemInfo>) -> Option<i64> {
        const MAX_STACK_SIZE: &str = "minecraft:max_stack_size";
        match self.data().and_then(|data| data.other(MAX_STACK_SIZE)) {
            Some(Tag::Int(size)) => Some(i64::from(*size)),
            Some(_) => None,
            None if self.carries_component(info, MAX_STACK_SIZE) => {
                info.map(|info| i64::from(info.max_stack))
            }
            None => Some(1),
        }
    }
    /// Effective villager food nutrition, including a stack's override or removal.
    pub fn villager_food(&self, info: Option<&ItemInfo>) -> Option<i32> {
        if let Some(data) = self.data() {
            if let Some(nutrition) = data.villager_food {
                return Some(nutrition);
            }
            if data.other("!minecraft:villager_food").is_some() {
                return None;
            }
        }
        info.and_then(|i| i.villager_food)
    }
    /// New persistent content must not admit a partial effect projection in a decoded stack.
    pub fn has_refused_consume_effects(&self) -> bool {
        self.data().is_some_and(|data| {
            data.refused_consume_effect_component("minecraft:consumable")
                || data.refused_consume_effect_component("minecraft:death_protection")
                || data
                    .effect_templates()
                    .any(|(_, stack)| stack.has_refused_consume_effects())
        })
    }

    /// Death protection effects retained at patch or prototype load.
    pub fn death_protection<'a>(
        &'a self,
        info: Option<&'a ItemInfo>,
    ) -> Option<&'a [crate::items::ConsumeEffect]> {
        if let Some(data) = self.data() {
            if let Some(value) = &data.death_protection {
                return Some(value);
            }
            if data.other("!minecraft:death_protection").is_some()
                || data.other("minecraft:death_protection").is_some()
            {
                return None;
            }
        }
        info.and_then(|i| i.death_protection.as_deref())
    }
    /// Effective consumable, decoded when the patch/prototype is loaded, with no tick allocation.
    pub fn consumable<'a>(
        &'a self,
        info: Option<&'a ItemInfo>,
    ) -> Option<&'a Arc<crate::items::ItemConsumable>> {
        if let Some(data) = self.data() {
            if data.other("!minecraft:consumable").is_some() {
                return None;
            }
            if let Some(consumable) = &data.consumable {
                return Some(&consumable.value);
            }
            if data.other("minecraft:consumable").is_some() {
                return None;
            }
        }
        info.and_then(|item| item.consumable.as_ref())
    }
    /// `ItemStack.getMaxDamage`: the stack override, removal, or item default.
    pub fn max_damage(&self, info: Option<&ItemInfo>) -> i32 {
        if !self.carries_component(info, "minecraft:max_damage") {
            return 0;
        }
        self.data()
            .and_then(|data| data.other("minecraft:max_damage"))
            .map(|value| crate::properties::component_int(value).unwrap_or(0))
            .unwrap_or_else(|| info.map_or(0, |item| item.max_damage as i32))
    }
    /// `ItemStack.getDamageValue`: a live patch, removal, or cached prototype damage.
    /// Preserve an explicit zero patch when the prototype starts damaged.
    pub fn effective_damage(&self, info: Option<&ItemInfo>) -> i32 {
        if self.damage > 0 || self.data().is_some_and(|data| data.damage_set) {
            return self.damage as i32;
        }
        if let Some(data) = self.data() {
            if let Some(value) = data.other("minecraft:damage") {
                return crate::properties::component_int(value).unwrap_or(0).max(0);
            }
            if data.other("!minecraft:damage").is_some() {
                return 0;
            }
        }
        info.map_or(0, |item| item.damage)
    }
    /// The patch the server sent for this stack, while the stack is still what
    /// it decoded to (its count aside); None once anything else changed.
    pub fn wire_patch(&self) -> Option<Arc<[u8]>> {
        let wire = self.data()?.wire.0.as_ref()?;
        let decoded = &wire.decoded;
        let none = StackData::default();
        (decoded.item == self.item
            && decoded.damage == self.damage
            && decoded.data().unwrap_or(&none) == self.data().unwrap_or(&none))
        .then(|| wire.patch.clone())
    }

    /// Reuse the server's bytes only under their original consume-effect mapping
    /// and if the live stack admitted every effect in them.
    pub fn wire_patch_for(&self, effect_mapping: Option<&Arc<()>>) -> Option<Arc<[u8]>> {
        let wire = self.data()?.wire.0.as_ref()?;
        let same_mapping = match (wire.effect_mapping.as_ref(), effect_mapping) {
            (None, None) => true,
            (Some(original), Some(current)) => Arc::ptr_eq(original, current),
            _ => false,
        };
        if wire.effect_refused || !same_mapping {
            return None;
        }
        self.wire_patch()
    }

    pub fn data_mut(&mut self) -> &mut StackData {
        let data = self.extra.get_or_insert_with(Box::default);
        data.bundle_weight = Default::default();
        data
    }
    /// Apply only the named component fields from a decoded snapshot of this same item.
    /// Unchanged fields keep their runtime data, including in nested stacks. The engine
    /// caller supplies unique names without the removal prefix and owns item/count validation.
    pub fn apply_component_fields_from<'a>(
        &mut self,
        mut snapshot: ItemStack,
        names: impl Iterator<Item = &'a str> + Clone,
    ) {
        if (self.data().is_some_and(|d| d.ominous_banner)
            || snapshot.data().is_some_and(|d| d.ominous_banner))
            && names.clone().any(is_ominous_component)
        {
            // A canonical incoming snapshot may itself be compact, but the
            // edit copies values. `tidy` compacts the result only if all agree.
            if let Some(data) = self.extra.as_deref_mut() {
                data.materialize_ominous_banner();
            }
            if let Some(data) = snapshot.extra.as_deref_mut() {
                data.materialize_ominous_banner();
            }
        }
        if names.clone().any(|id| id == "minecraft:damage") {
            self.damage = snapshot.damage;
        }
        if let Some(data) = self.extra.as_deref_mut() {
            data.apply_component_changes(snapshot.extra, names);
        } else if let Some(extra) = snapshot.extra {
            let mut data = StackData::default();
            data.apply_component_changes(Some(extra), names);
            self.extra = (!data.is_empty()).then(|| Box::new(data));
        }
        self.tidy();
    }
    /// Canonicalizes the component representation and drops an all-default box.
    pub fn tidy(&mut self) {
        if let Some(data) = self.extra.as_deref_mut() {
            data.compact_ominous_banner();
            if data.is_empty() {
                self.extra = None;
            }
        }
    }

    /// The effective banner pattern layers, including the compact raid patch.
    pub fn banner_layers(&self) -> &[BannerLayer] {
        self.data().map_or(&[], StackData::banner_layers)
    }
    pub fn enchantments(&self) -> &[(Arc<str>, u32)] {
        self.data().map_or(&[], |d| &d.enchantments)
    }
    pub fn stored_enchantments(&self) -> &[(Arc<str>, u32)] {
        self.data().map_or(&[], |d| &d.stored_enchantments)
    }
    /// `IItemStackExtension.getAllEnchantments`: the enchantments gameplay
    /// sees on the stack, its `minecraft:enchantments` as the mods'
    /// `get_enchantment_level` leaves them. Borrowed when no mod changes
    /// them. Tooltips, the anvil, the grindstone, saves and the network read
    /// the component itself ([`Self::enchantments`]).
    pub fn all_enchantments(&self) -> std::borrow::Cow<'_, [(Arc<str>, u32)]> {
        let own = self.enchantments();
        match crate::native_enchantment_hooks::get_enchantment_level(self, None, own) {
            Some(levels) => std::borrow::Cow::Owned(levels),
            None => std::borrow::Cow::Borrowed(own),
        }
    }
    /// `EnchantmentHelper.getItemEnchantmentLevel` as NeoForge has it
    /// (`IItemStackExtension.getEnchantmentLevel`): the level gameplay sees,
    /// after the mods' `get_enchantment_level` (0 when absent).
    pub fn enchantment_level(&self, id: &str) -> u32 {
        let own = self.enchantments();
        match crate::native_enchantment_hooks::get_enchantment_level(self, Some(id), own) {
            Some(levels) => level_in(&levels, id),
            None => level_in(own, id),
        }
    }
    /// `EnchantmentHelper.getTagEnchantmentLevel`: the level in the stack's
    /// own `minecraft:enchantments`, without the mods (0 when absent).
    pub fn tag_enchantment_level(&self, id: &str) -> u32 {
        level_in(self.enchantments(), id)
    }
    /// The component an anvil/grindstone works on: stored for books.
    pub fn crafting_enchantments(&self) -> &[(Arc<str>, u32)] {
        if &*self.item == "minecraft:enchanted_book" {
            self.stored_enchantments()
        } else {
            self.enchantments()
        }
    }
    pub fn set_crafting_enchantments(&mut self, levels: Levels) {
        if &*self.item == "minecraft:enchanted_book" {
            self.data_mut().stored_enchantments = levels;
        } else {
            self.data_mut().enchantments = levels;
        }
        self.tidy();
    }
    /// `ItemStack.enchant`: upgrade to the higher level. Vanilla's
    /// `EnchantmentHelper.updateEnchantments` returns EMPTY when the component
    /// is absent, so a removal patch makes this a no-op.
    pub fn enchant(&mut self, id: &str, level: u32) {
        let book = &*self.item == "minecraft:enchanted_book";
        let removal = if book {
            "!minecraft:stored_enchantments"
        } else {
            "!minecraft:enchantments"
        };
        if self
            .data()
            .is_some_and(|data| data.other(removal).is_some())
        {
            return;
        }
        let data = self.data_mut();
        let list = if book {
            &mut data.stored_enchantments
        } else {
            &mut data.enchantments
        };
        upgrade(list, id, level);
    }
    pub fn has_any_enchantments(&self) -> bool {
        !self.enchantments().is_empty() || !self.stored_enchantments().is_empty()
    }
    /// `ItemStack.hasFoil`: an explicit `minecraft:enchantment_glint_override`
    /// component wins, then the item's default override (`item_glint`), then a
    /// `Item.isFoil` (enchantments, or a compass carrying a lodestone tracker).
    /// Stored book enchantments do not count.
    pub fn has_foil(&self, item_glint: Option<bool>) -> bool {
        self.has_foil_with_enchantments(item_glint, !self.enchantments().is_empty())
    }

    /// Use the enchantments of a borrowed equipment stack, which can carry a gear overlay.
    pub fn has_foil_with_enchantments(&self, item_glint: Option<bool>, enchanted: bool) -> bool {
        let own = self
            .data()
            .and_then(|data| data.other("minecraft:enchantment_glint_override"))
            .and_then(component_bool);
        let removed = self.data().is_some_and(|data| {
            data.other("!minecraft:enchantment_glint_override")
                .is_some()
        });
        let override_glint = own.or(if removed { None } else { item_glint });
        let item_foil = crate::properties::item_foil(
            &self.item,
            self.data()
                .is_some_and(|data| data.other("minecraft:lodestone_tracker").is_some()),
            enchanted,
        );
        crate::properties::foil(override_glint, item_foil)
    }
    pub fn repair_cost(&self) -> u32 {
        self.data().map_or(0, |d| d.repair_cost)
    }
    pub fn custom_name(&self) -> Option<String> {
        self.data()?.custom_name_plain()
    }
    /// Non-empty slots of a `minecraft:container` component.
    pub fn container(&self) -> &[(u8, ItemStack)] {
        self.data().map_or(&[], |d| &d.container)
    }
    /// The `minecraft:custom_data` compound if present.
    pub fn custom_data(&self) -> Option<&sunrise_core::nbt::Compound> {
        match self.data()?.other("minecraft:custom_data")? {
            Tag::Compound(map) => Some(map),
            _ => None,
        }
    }
    /// `ResolvableProfile.name`: a bare profile string or its `name` field.
    pub fn profile_name(&self) -> Option<String> {
        let profile = self.data()?.other("minecraft:profile")?;
        string(profile).or_else(|| profile.get("name").and_then(string))
    }
    /// `minecraft:item_name` in the current language (`getItemName`'s
    /// component). It stays in the patch as loaded, so a translatable name
    /// saves and reaches clients unchanged; the ominous banner's is modelled
    /// by `ominous_banner` instead.
    pub fn item_name(&self) -> Option<String> {
        let name = self.data()?.other("minecraft:item_name")?;
        Some(sunrise_game_data::component::name_text(name).plain())
    }
    pub fn potion(&self) -> Option<&str> {
        self.data().and_then(|d| d.potion.as_deref())
    }
    /// `PotionContents.customEffects` as saved: (id, duration, amplifier). Both
    /// default to 0 like `MobEffectInstance.CODEC`.
    pub fn custom_potion_effects(&self) -> Vec<(String, i32, i32)> {
        let Some((_, Tag::List(list))) = self
            .data()
            .and_then(|d| d.potion_extra.iter().find(|(k, _)| k == "custom_effects"))
        else {
            return Vec::new();
        };
        list.iter()
            .filter_map(|e| {
                Some((
                    string(e.get("id")?)?,
                    e.get("duration").and_then(int).unwrap_or(0) as i32,
                    e.get("amplifier").and_then(int).unwrap_or(0).clamp(0, 255) as i32,
                ))
            })
            .collect()
    }
    /// `DataComponents.POTION_DURATION_SCALE` with the item's default: 0.125
    /// for a tipped arrow, 0.25 for a lingering potion, else 1.
    pub fn potion_duration_scale(&self) -> f32 {
        match self
            .data()
            .and_then(|d| d.other("minecraft:potion_duration_scale"))
        {
            Some(Tag::Float(bits)) => f32::from_bits(*bits),
            Some(Tag::Double(bits)) => f64::from_bits(*bits) as f32,
            Some(t) if int(t).is_some() => int(t).unwrap_or(1) as f32,
            _ => match &*self.item {
                "minecraft:tipped_arrow" => 0.125,
                "minecraft:lingering_potion" => 0.25,
                _ => 1.0,
            },
        }
    }
    pub fn with_potion(mut self, potion: &str) -> Self {
        self.data_mut().potion = Some(Arc::from(potion));
        self
    }
    /// `ItemStack.isSameItemSameComponents`.
    pub fn same_item_same_components(&self, other: &Self) -> bool {
        self.item == other.item && self.damage == other.damage && self.data() == other.data()
    }
    /// A hash equal for stacks that are [`Self::same_item_same_components`], whatever their counts.
    pub fn merge_kind(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (&*self.item, self.damage, self.data()).hash(&mut hasher);
        hasher.finish()
    }
    /// Compare live component identity as `PatchedDataComponentMap.fromPatch` does:
    /// ignore sets equal to the prototype and removals of absent defaults. This
    /// reads decoded components, including transient ones the save codec drops.
    pub fn same_item_same_components_with_prototype(
        &self,
        other: &Self,
        items: &crate::items::Items,
    ) -> bool {
        if self.item != other.item {
            return false;
        }
        if self.same_item_same_components(other) {
            return true;
        }
        let prototype = items
            .get(&self.item)
            .filter(|_| items.component_defaults.contains_key(self.item.as_ref()));
        let matches_patch = |stack: &Self, other: &Self| {
            let keep = |id: &str, removed| {
                prototype.is_none_or(|info| {
                    keep_component_patch_entry(
                        removed,
                        info.components.iter().any(|key| key == id),
                        || {
                            ingredient_components::component_equal(
                                id,
                                stack,
                                &info.component_prototype,
                            )
                        },
                    )
                })
            };
            let mut matches = true;
            ingredient_components::visit_components(stack, |id| {
                if matches && keep(id, false) {
                    matches = other.carries_component(None, id)
                        && ingredient_components::component_equal(id, stack, other);
                }
            });
            matches
                && stack
                    .data()
                    .into_iter()
                    .flat_map(StackData::preserved_components)
                    .filter_map(|(id, _)| id.strip_prefix('!'))
                    .all(|id| !keep(id, true) || ingredient_components::removed(other, id))
        };
        matches_patch(self, other) && matches_patch(other, self)
    }
    /// `ItemStack.matches`: the same item, count and components.
    pub fn matches(&self, other: &Self) -> bool {
        self.count == other.count && self.same_item_same_components(other)
    }
    /// `ItemStack.matches` with separate mob gear overlays (`enchantments` when
    /// non-empty, `trim` when `Some`) applied to `self` without cloning.
    pub fn matches_with_overlay(
        &self,
        other: &Self,
        enchantments: &[(Arc<str>, u32)],
        trim: Option<&Trim>,
    ) -> bool {
        if self.count != other.count || self.item != other.item || self.damage != other.damage {
            return false;
        }
        if enchantments.is_empty() && trim.is_none() {
            return self.data() == other.data();
        }
        let Some(other_data) = other.data() else {
            return false;
        };
        match self.data() {
            Some(self_data) => self_data.eq_with_overlay(other_data, enchantments, trim),
            None => StackData::default().eq_with_overlay(other_data, enchantments, trim),
        }
    }
    /// `getOrDefault(DataComponents.ATTRIBUTE_MODIFIERS)`: the stack's own
    /// `attribute_modifiers` component when it has one (none at all when it
    /// removed the component), else its item type's. Borrowed unless the stack
    /// has its own list.
    pub fn default_attribute_modifiers<'a>(
        &self,
        items: &'a crate::items::Items,
    ) -> std::borrow::Cow<'a, [ModifierRow]> {
        self.default_attribute_modifiers_or(items.get(&self.item).map_or(&[], |i| &i.modifiers))
    }
    /// `default_attribute_modifiers` with the item type's list given: `default`.
    pub fn default_attribute_modifiers_or<'a>(
        &self,
        default: &'a [ModifierRow],
    ) -> std::borrow::Cow<'a, [ModifierRow]> {
        use std::borrow::Cow;
        if let Some(data) = self.data() {
            if data.other("!minecraft:attribute_modifiers").is_some() {
                return Cow::Borrowed(&[]);
            }
            if let Some(tag) = data.other("minecraft:attribute_modifiers") {
                return Cow::Owned(modifier_rows(tag));
            }
        }
        Cow::Borrowed(default)
    }
    /// `ItemStack.transmuteCopy`: another item keeping every component.
    pub fn transmute(&self, item: &str, count: u32) -> Self {
        let mut stack = Self {
            item: Arc::from(item),
            count,
            ..self.clone()
        };
        stack.tidy();
        stack
    }
}
/// `ItemEnchantments.getLevel`: `id`'s level in `list` (0 when absent).
fn level_in(list: &[(Arc<str>, u32)], id: &str) -> u32 {
    list.iter().find(|(e, _)| &**e == id).map_or(0, |(_, l)| *l)
}
/// `ItemEnchantments.Mutable.upgrade`.
pub fn upgrade(list: &mut Levels, id: &str, level: u32) {
    if level == 0 {
        return;
    }
    match list.iter_mut().find(|(e, _)| &**e == id) {
        Some((_, l)) => *l = (*l).max(level),
        None => list.push((Arc::from(id), level)),
    }
}
/// `ItemEnchantments.Mutable.set` (level 0 removes).
pub fn set_level(list: &mut Levels, id: &str, level: u32) {
    if level == 0 {
        list.retain(|(e, _)| &**e != id);
    } else {
        match list.iter_mut().find(|(e, _)| &**e == id) {
            Some((_, l)) => *l = level,
            None => list.push((Arc::from(id), level)),
        }
    }
}

fn levels_tag(levels: &Levels) -> Tag {
    Tag::compound(levels.iter().map(|(id, l)| (&**id, Tag::Int(*l as i32))))
}
fn int(t: &Tag) -> Option<i64> {
    match t {
        Tag::Byte(v) => Some(i64::from(*v)),
        Tag::Short(v) => Some(i64::from(*v)),
        Tag::Int(v) => Some(i64::from(*v)),
        Tag::Long(v) => Some(*v),
        _ => None,
    }
}
fn string(t: &Tag) -> Option<String> {
    match t {
        Tag::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}
/// The persistent `ItemEnchantments` codec uses integer levels in 1..=255.
/// `TagValueInput.read` keeps the partial map when an entry fails decoding.
fn read_levels(t: Option<&Tag>) -> Levels {
    let Some(Tag::Compound(map)) = t else {
        return Vec::new();
    };
    // Pre-1.21.5 worlds wrap the map in {levels: {...}}.
    let map = match map.get(&JavaString::from("levels")) {
        Some(Tag::Compound(inner)) => inner,
        _ => map,
    };
    // The keys are enchantment Identifiers: `knockback` is `minecraft:knockback`.
    map.iter()
        .filter_map(|(k, v)| {
            let level = sunrise_world::datafix::dynamic::as_number(v)?.int_value();
            if !(1..=255).contains(&level) {
                return None;
            }
            let id = sunrise_world::datafix::dynamic::namespaced(&k.to_string_lossy());
            Some((Arc::from(id), level as u32))
        })
        .collect()
}
/// A text component as stored by vanilla: a plain string or `{"text": ...}`.
/// A `Filterable<String/Component>`: the value itself or `{raw: value}`.
/// `ComponentSerialization.CODEC`'s short form: a plain text component with
/// no style and no siblings is written as its string.
fn plain_component(t: &Tag) -> Tag {
    if let Tag::Compound(c) = t
        && let Some(text @ Tag::String(_)) = c.get(&"text".into())
        && c.iter().all(|(k, v)| {
            k.to_string_lossy() == "text"
                || (k.to_string_lossy() == "type"
                    && matches!(v, Tag::String(s) if s.to_string_lossy() == "text"))
        })
    {
        return text.clone();
    }
    t.clone()
}
/// `WrittenBookContent.CODEC`: the title and author as `Filterable` values,
/// the pages (each `{raw: component}`), the generation when it is not 0 and
/// the resolved flag when it is set.
fn written_book_tag(w: &WrittenBook) -> Tag {
    // Filterable values are saved as {raw: ..}.
    let raw = |t: Tag| Tag::compound([("raw", t)]);
    let pages = if w.page_tags.len() == w.pages.len() {
        w.page_tags.iter().cloned().map(raw).collect()
    } else {
        w.pages
            .iter()
            .map(|p| raw(Tag::String(p.as_str().into())))
            .collect()
    };
    let mut fields = vec![
        ("title", raw(Tag::String(w.title.as_str().into()))),
        ("author", Tag::String(w.author.as_str().into())),
    ];
    // `optionalFieldOf("pages", List.of())`: no pages, no field.
    if !w.pages.is_empty() {
        fields.push(("pages", Tag::List(pages)));
    }
    if w.generation != 0 {
        fields.push(("generation", Tag::Int(i32::from(w.generation))));
    }
    if w.resolved {
        fields.push(("resolved", Tag::Byte(1)));
    }
    Tag::compound(fields)
}

/// `WrittenBookContent` from its component: the title and author (a
/// `Filterable`'s raw text), the generation and the pages, whose component
/// tags are kept apart from their plain text for click events.
fn written_book_from_tag(b: &Tag) -> WrittenBook {
    WrittenBook {
        title: b.get("title").and_then(filterable_text).unwrap_or_default(),
        author: b.get("author").and_then(string).unwrap_or_default(),
        generation: b.get("generation").and_then(int).unwrap_or(0).clamp(0, 3) as u8,
        pages: match b.get("pages") {
            Some(Tag::List(pages)) => pages.iter().filter_map(filterable_text).collect(),
            _ => Vec::new(),
        },
        page_tags: match b.get("pages") {
            Some(Tag::List(pages)) => pages
                .iter()
                .map(|p| plain_component(p.get("raw").unwrap_or(p)))
                .collect(),
            _ => Vec::new(),
        },
        resolved: b.get("resolved").and_then(component_bool).unwrap_or(false),
    }
}

/// `WrittenBookContent.resolvePage`: the page's component with its selectors
/// and scores replaced by the text they name, recursing into `extra` and
/// `with`.
fn resolve_page_tag(tag: &Tag, text: &mut impl FnMut(&Tag) -> Option<String>) -> Tag {
    let Tag::Compound(c) = tag else {
        return tag.clone();
    };
    if let Some(resolved) = text(tag) {
        return Tag::compound([("text", Tag::String(resolved.as_str().into()))]);
    }
    let mut out = c.clone();
    for key in ["extra", "with"] {
        if let Some(Tag::List(list)) = out.get(&key.into()) {
            let list = list.iter().map(|t| resolve_page_tag(t, text)).collect();
            out.insert(key.into(), Tag::List(list));
        }
    }
    Tag::Compound(out)
}

impl WrittenBook {
    /// `WrittenBookContent.resolve` and `markResolved`: every page's
    /// selectors and scores become their text, and the book is marked
    /// resolved. A page whose resolved text passes `WrittenBookContent`'s
    /// 32767-character limit (`resolvePage`) cancels the resolution, and the
    /// book is only marked resolved. True when the pages were rewritten.
    pub fn resolve(&mut self, mut text: impl FnMut(&Tag) -> Option<String>) -> bool {
        if self.resolved {
            return false;
        }
        self.resolved = true;
        if self.page_tags.len() != self.pages.len() {
            return false;
        }
        let mut pages = Vec::with_capacity(self.pages.len());
        let mut tags = Vec::with_capacity(self.page_tags.len());
        for tag in &self.page_tags {
            let resolved = resolve_page_tag(tag, &mut text);
            let plain = sunrise_game_data::component::name_text(&resolved).plain();
            if plain.chars().count() > 32767 {
                return false;
            }
            pages.push(plain);
            tags.push(resolved);
        }
        self.pages = pages;
        self.page_tags = tags;
        true
    }
}

/// `WritableBookContent.CODEC`'s pages that decode, as their raw text.
fn writable_raw(content: Option<&Tag>) -> Vec<String> {
    writable_page_tags(content)
        .filter_map(filterable_text)
        .collect()
}
/// The filtered text of the pages [`writable_raw`] keeps: empty when none has one.
fn writable_filtered(content: Option<&Tag>) -> Vec<Option<String>> {
    let filtered: Vec<_> = writable_page_tags(content)
        .filter(|t| filterable_text(t).is_some())
        .map(|t| t.get("filtered").and_then(read_text))
        .collect();
    if filtered.iter().all(Option::is_none) {
        Vec::new()
    } else {
        filtered
    }
}
fn writable_page_tags(content: Option<&Tag>) -> impl Iterator<Item = &Tag> {
    match content.and_then(|b| b.get("pages")) {
        Some(Tag::List(pages)) => pages.as_slice(),
        _ => &[],
    }
    .iter()
}
fn filterable_text(t: &Tag) -> Option<String> {
    match t.get("raw") {
        Some(raw) => read_text(raw),
        None => read_text(t),
    }
}
fn read_text(t: &Tag) -> Option<String> {
    match t {
        Tag::String(s) => Some(s.to_string_lossy()),
        // A component's root `text` may be empty with the visible words in
        // `extra` (every legacy display name ViaVersion produces looks like
        // this), so flatten the whole component instead of reading one key.
        // ComponentSerialization.CODEC also takes a list (its first element
        // styling the rest), as servers behind ViaVersion send book pages.
        Tag::Compound(_) | Tag::List(_) => Some(sunrise_game_data::component::name_text(t).plain()),
        _ => None,
    }
}

/// `Component.tryCollapseToString`: `ComponentSerialization.CODEC` writes
/// plain text with no style and no siblings as a bare string.
fn collapsed_text(tag: Tag) -> Tag {
    if let Tag::Compound(map) = &tag
        && let Some(Tag::String(text)) = tag.get("text")
        && map
            .iter()
            .all(|(key, value)| match key.to_string_lossy().as_str() {
                "text" => true,
                "type" => matches!(value, Tag::String(s) if s.to_string_lossy() == "text"),
                "extra" => matches!(value, Tag::List(l) if l.is_empty()),
                _ => false,
            })
    {
        return Tag::String(text.clone());
    }
    tag
}

/// A `FireworkExplosion` as vanilla's codec writes it (defaults omitted).
fn explosion_tag(e: &crate::firework::FireworkExplosion) -> Tag {
    let colors = |c: &[u32]| Tag::List(c.iter().map(|c| Tag::Int(*c as i32)).collect());
    let mut fields = vec![("shape", Tag::String(e.shape.name().into()))];
    if !e.colors.is_empty() {
        fields.push(("colors", colors(&e.colors)));
    }
    if !e.fade.is_empty() {
        fields.push(("fade_colors", colors(&e.fade)));
    }
    if e.trail {
        fields.push(("has_trail", Tag::Byte(1)));
    }
    if e.twinkle {
        fields.push(("has_twinkle", Tag::Byte(1)));
    }
    Tag::compound(fields)
}
fn explosion_from(e: &Tag) -> crate::firework::FireworkExplosion {
    let ints = |t: Option<&Tag>| match t {
        Some(Tag::IntArray(v)) => v.iter().map(|c| *c as u32).collect(),
        Some(Tag::List(v)) => v.iter().filter_map(int).map(|c| c as u32).collect(),
        _ => Vec::new(),
    };
    crate::firework::FireworkExplosion {
        shape: crate::firework::FireworkShape::from_name(
            e.get("shape")
                .and_then(string)
                .as_deref()
                .map_or("small_ball", |s| s.strip_prefix("minecraft:").unwrap_or(s)),
        ),
        colors: ints(e.get("colors")),
        fade: ints(e.get("fade_colors")),
        trail: e.get("has_trail").and_then(int).unwrap_or(0) != 0,
        twinkle: e.get("has_twinkle").and_then(int).unwrap_or(0) != 0,
    }
}
/// An `ItemStackTemplate`: `{id, count?, components?}`.
fn template_tag(stack: &ItemStack, components: Option<Tag>) -> Tag {
    let mut entries = vec![("id", Tag::String((&*stack.item).into()))];
    if stack.count != 1 {
        entries.push(("count", Tag::Int(stack.count as i32)));
    }
    if let Some(components) = components {
        entries.push(("components", components));
    }
    Tag::compound(entries)
}
/// Save the complete template patch without applying the source item's defaults.
pub fn raw_template_tag(stack: &ItemStack) -> Tag {
    template_tag(stack, component_snapshot(stack))
}
/// The pot prototype's decorations: a brick on every side.
pub fn all_bricks(sides: &[Option<ItemStack>; 4]) -> bool {
    sides.iter().all(|s| {
        s.as_ref()
            .is_some_and(|s| &*s.item == "minecraft:brick" && s.count == 1 && s.extra.is_none())
    })
}
fn template_from(tag: &Tag) -> Option<ItemStack> {
    template_from_with(tag, None)
}
fn template_from_with(tag: &Tag, effects: Option<&Arc<[Admitted]>>) -> Option<ItemStack> {
    let id = match tag {
        Tag::String(id) => id.to_string_lossy(),
        _ => tag.get("id").and_then(string)?,
    };
    let id = normalized_identifier(&id).ok()?;
    if id == "minecraft:air" {
        return None;
    }
    // `ItemStackTemplate.CODEC`: `count` is `ExtraCodecs.intRange(1, 99)`, so a
    // template outside it does not decode.
    let count = tag.get("count").and_then(int).unwrap_or(1);
    if !(1..=99).contains(&count) {
        return None;
    }
    let (damage, extra) = decode_components_inner(tag.get("components"), true, effects);
    Some(ItemStack {
        item: Arc::from(id.as_ref()),
        count: count as u32,
        damage,
        extra,
    })
}
/// The stack a `show_item` hover event holds (`HoverEvent.ShowItem`): its
/// `{id, count?, components?}` object.
pub fn stack_from_hover(event: &serde_json::Value) -> Option<ItemStack> {
    template_from(&sunrise_game_data::scoreboard::json_to_tag(event))
}
/// Sorted-key, type-annotated text of a tag (the differential harnesses
/// print the same form from the vanilla side).
pub fn canon_tag(tag: &Tag) -> String {
    fn list(out: &mut String, prefix: &str, items: impl Iterator<Item = String>) {
        out.push_str(prefix);
        out.push_str(&items.collect::<Vec<_>>().join(","));
        out.push(']');
    }
    fn go(tag: &Tag, out: &mut String) {
        use std::fmt::Write as _;
        match tag {
            Tag::End => {}
            Tag::Byte(v) => write!(out, "{v}b").unwrap(),
            Tag::Short(v) => write!(out, "{v}s").unwrap(),
            Tag::Int(v) => write!(out, "{v}").unwrap(),
            Tag::Long(v) => write!(out, "{v}L").unwrap(),
            Tag::Float(v) => write!(out, "{}f", f32::from_bits(*v)).unwrap(),
            Tag::Double(v) => write!(out, "{}d", f64::from_bits(*v)).unwrap(),
            Tag::String(s) => write!(
                out,
                "\"{}\"",
                s.to_string_lossy()
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
            )
            .unwrap(),
            Tag::ByteArray(v) => list(out, "[B;", v.iter().map(|b| (*b as i8).to_string())),
            Tag::IntArray(v) => list(out, "[I;", v.iter().map(i32::to_string)),
            Tag::LongArray(v) => list(out, "[L;", v.iter().map(i64::to_string)),
            Tag::List(items) => {
                out.push('[');
                for (i, t) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    go(t, out);
                }
                out.push(']');
            }
            Tag::Compound(map) => {
                let mut keys: Vec<(String, &Tag)> =
                    map.iter().map(|(k, v)| (k.to_string_lossy(), v)).collect();
                keys.sort_by(|a, b| a.0.cmp(&b.0));
                out.push('{');
                for (i, (k, v)) in keys.into_iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&k);
                    out.push(':');
                    go(v, out);
                }
                out.push('}');
            }
        }
    }
    let mut out = String::new();
    go(tag, &mut out);
    out
}
/// Canonical saved stack using the world's loaded item prototypes.
pub fn canon_stack(stack: Option<&ItemStack>, items: &crate::items::Items) -> String {
    stack.map_or_else(
        || "-".to_owned(),
        |stack| canon_tag(&crate::item_nbt::item_to_nbt(stack, items)),
    )
}

/// The stack of an SNBT `ItemStack.CODEC` compound; None for "-".
pub fn parse_stack(snbt: &str) -> Result<Option<ItemStack>, String> {
    if snbt == "-" {
        return Ok(None);
    }
    let (tag, _) = sunrise_core::nbt::parse_snbt(snbt)?;
    Ok(crate::item_nbt::item_from_nbt(&tag))
}
/// `MapDecorations.Entry.CODEC`: a bare type ID uses the default resource namespace.
/// Keep preserved entry keys and numeric tags intact; this is not registry validation.
fn canonical_other_component(id: &str, tag: &Tag) -> Tag {
    let mut value = sunrise_core::nbt::clone_tag(tag);
    if id == "minecraft:map_decorations"
        && let Tag::Compound(entries) = &mut value
    {
        entries.for_each_mut(|_, entry| {
            if let Some(Tag::String(kind)) = entry.get_mut("type") {
                let text = kind.to_string_lossy();
                if !text.is_empty()
                    && !text.contains(':')
                    && let Ok(id) = sunrise_core::identifier::Identifier::parse(&text)
                {
                    *kind = id.to_string().as_str().into();
                }
            }
        });
    }
    value
}
/// The live component patch, including transient components and explicit defaults.
/// Omit the top-level damage field when the caller already carries it as a scalar.
pub fn live_component_patch(stack: &ItemStack, include_damage: bool) -> Option<Tag> {
    live_component_patch_matching(stack, include_damage, |_| true)
}

/// Select live component IDs without the `!` on removals. Nested stacks keep their full live patches.
pub fn live_component_patch_matching(
    stack: &ItemStack,
    include_damage: bool,
    selector: impl Fn(&str) -> bool,
) -> Option<Tag> {
    let mut components = components_tag_inner(
        stack.damage,
        stack.data(),
        include_damage,
        true,
        &selector,
        &|nested| template_tag(nested, live_component_patch(nested, true)),
    )?;
    canonicalize_component_patch(&mut components);
    Some(components)
}

/// An explicit snapshot of persistable components, including overrides equal to the item's default.
/// Persistent stacks use [`encode_components`] with the loaded catalog instead.
pub fn component_snapshot(stack: &ItemStack) -> Option<Tag> {
    encode_persistent_components(stack, true, &|_| true)
}

/// Encode the persistable extra patch for a snapshot whose typed scalar already carries damage.
pub fn encode_extra_components(stack: &ItemStack) -> Option<Tag> {
    encode_extra_components_matching(stack, |_| true)
}

/// Engine-only projection of the persistable extra patch, excluding the typed damage scalar.
/// The selector sees the component ID without `!` for removals. Selected nested
/// stacks keep their full persistable patches.
pub fn encode_extra_components_matching(
    stack: &ItemStack,
    selector: impl Fn(&str) -> bool,
) -> Option<Tag> {
    encode_persistent_components(stack, false, &selector)
}

fn keep_component_patch_entry(
    removed: bool,
    present_by_default: bool,
    equals_default: impl FnOnce() -> bool,
) -> bool {
    if removed {
        present_by_default
    } else {
        !present_by_default || !equals_default()
    }
}

/// `PatchedDataComponentMap`: retain sets that differ from the item's prototype and
/// removals of components the prototype actually has. Unknown prototypes leave the patch intact.
pub fn component_patch_filter<'a>(
    stack: &'a ItemStack,
    items: &'a crate::items::Items,
) -> impl Fn(&JavaString, bool) -> bool + 'a {
    let prototype = items
        .component_defaults
        .get(stack.item.as_ref())
        .zip(items.get(&stack.item));
    move |id, removed| {
        let Some((keys, info)) = prototype else {
            return true;
        };
        keep_component_patch_entry(removed, keys.contains_key(id), || {
            ingredient_components::component_equal(
                &id.to_string_lossy(),
                stack,
                &info.component_prototype,
            )
        })
    }
}

/// The saved `DataComponentPatch.CODEC`, compared against the loaded prototype.
/// Nested templates retain their raw patches, including redundant entries and removals.
pub fn encode_components(stack: &ItemStack, items: &crate::items::Items) -> Option<Tag> {
    let mut components = encode_persistent_components(stack, true, &|_| true)?;
    if let Tag::Compound(entries) = &mut components {
        let keep = component_patch_filter(stack, items);
        entries.retain(|id, _| {
            if let Some(removed) = id.0.strip_prefix(&[u16::from(b'!')]) {
                keep(&JavaString(removed.to_vec()), true)
            } else {
                keep(id, false)
            }
        });
        if entries.is_empty() {
            return None;
        }
    }
    Some(components)
}
/// Component fields from damage and extras, without item-prototype elision or the
/// typed map post-processing directive.
pub fn components_tag(damage: u32, data: Option<&StackData>) -> Option<Tag> {
    components_tag_inner(damage, data, true, false, &|_| true, &raw_template_tag)
}

fn components_tag_inner(
    damage: u32,
    data: Option<&StackData>,
    include_damage: bool,
    include_map_post_processing: bool,
    selector: &impl Fn(&str) -> bool,
    template_tag: &impl Fn(&ItemStack) -> Tag,
) -> Option<Tag> {
    let mut entries: Vec<(&str, Tag)> = Vec::new();
    // Explicit damage patches survive even at zero; a custom max damage can
    // also make its saved zero meaningful.
    if include_damage
        && selector("minecraft:damage")
        && (damage > 0
            || data.is_some_and(|data| {
                data.damage_set
                    || (data.explicit_zero_damage && data.other("minecraft:max_damage").is_some())
            }))
    {
        entries.push(("minecraft:damage", Tag::Int(damage as i32)));
    }
    if let Some(d) = data {
        if !d.enchantments.is_empty() && selector("minecraft:enchantments") {
            entries.push(("minecraft:enchantments", levels_tag(&d.enchantments)));
        }
        if !d.stored_enchantments.is_empty() && selector("minecraft:stored_enchantments") {
            entries.push((
                "minecraft:stored_enchantments",
                levels_tag(&d.stored_enchantments),
            ));
        }
        if d.repair_cost > 0 && selector("minecraft:repair_cost") {
            entries.push(("minecraft:repair_cost", Tag::Int(d.repair_cost as i32)));
        }
        if let Some(name) = &d.custom_name
            && selector("minecraft:custom_name")
        {
            entries.push(("minecraft:custom_name", name.clone()));
        }
        if let Some(consumable) = &d.consumable
            && selector("minecraft:consumable")
        {
            entries.push(("minecraft:consumable", consumable.raw.clone()));
        }
        if let Some(kinetic) = &d.kinetic_weapon
            && selector("minecraft:kinetic_weapon")
        {
            entries.push(("minecraft:kinetic_weapon", kinetic.raw().clone()));
        }
        if (d.potion.is_some() || !d.potion_extra.is_empty())
            && selector("minecraft:potion_contents")
        {
            let potion = d
                .potion
                .as_ref()
                .map(|p| ("potion", Tag::String((&**p).into())));
            let extra = d.potion_extra.iter().map(|(k, v)| (k.as_str(), v.clone()));
            entries.push((
                "minecraft:potion_contents",
                Tag::compound(potion.into_iter().chain(extra)),
            ));
        }
        if let Some(trim) = &d.trim
            && selector("minecraft:trim")
        {
            entries.push(("minecraft:trim", trim::to_nbt(trim)));
        }
        if let Some(id) = d.map_id
            && selector("minecraft:map_id")
        {
            entries.push(("minecraft:map_id", Tag::Int(id)));
        }
        entries.extend(
            d.implicit_banner_values()
                .filter(|(id, _)| selector(id))
                .cloned(),
        );
        if !d.ominous_banner
            && (!d.banner_layers().is_empty() || d.banner_patterns_set)
            && selector("minecraft:banner_patterns")
        {
            entries.push((
                "minecraft:banner_patterns",
                crate::banner_layer::layers_to_nbt(d.banner_layers()),
            ));
        }
        if let Some(color) = &d.base_color
            && selector("minecraft:base_color")
        {
            entries.push(("minecraft:base_color", Tag::String((&**color).into())));
        }
        // `Filterable.codec` encodes the full record: {raw: ..}, and `filtered` when a page has one.
        if !d.writable_pages.is_empty() && selector("minecraft:writable_book_content") {
            let pages = d
                .writable_pages
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let filtered = d.writable_filtered.get(i).and_then(Option::as_deref);
                    Tag::compound(
                        std::iter::once(("raw", Tag::String(p.as_str().into())))
                            .chain(filtered.map(|f| ("filtered", Tag::String(f.into())))),
                    )
                })
                .collect();
            entries.push((
                "minecraft:writable_book_content",
                Tag::compound([("pages", Tag::List(pages))]),
            ));
        }
        if let Some(w) = &d.written
            && selector("minecraft:written_book_content")
        {
            entries.push(("minecraft:written_book_content", written_book_tag(w)));
        }
        if let Some((flight, explosions)) = &d.fireworks
            && selector("minecraft:fireworks")
        {
            let mut fields = Vec::new();
            if *flight != 0 {
                fields.push(("flight_duration", Tag::Byte(*flight as i8)));
            }
            if !explosions.is_empty() {
                fields.push((
                    "explosions",
                    Tag::List(explosions.iter().map(explosion_tag).collect()),
                ));
            }
            entries.push(("minecraft:fireworks", Tag::compound(fields)));
        }
        if let Some(e) = &d.firework_explosion
            && selector("minecraft:firework_explosion")
        {
            entries.push(("minecraft:firework_explosion", explosion_tag(e)));
        }
        if let Some(color) = d.dyed_color
            && selector("minecraft:dyed_color")
        {
            entries.push(("minecraft:dyed_color", Tag::Int(color)));
        }
        if let Some(sides) = &d.pot_decorations
            && selector("minecraft:pot_decorations")
        {
            let mut sherds = Vec::new();
            for (name, side) in ["back", "left", "right", "front"].iter().zip(sides.iter()) {
                if let Some(stack) = side {
                    sherds.push((*name, template_tag(stack)));
                }
            }
            entries.push(("minecraft:pot_decorations", Tag::compound(sherds)));
        }
        if !d.bundle_contents.is_empty() && selector("minecraft:bundle_contents") {
            entries.push((
                "minecraft:bundle_contents",
                Tag::List(d.bundle_contents.iter().map(template_tag).collect()),
            ));
        }
        if !d.stew_effects.is_empty() && selector("minecraft:suspicious_stew_effects") {
            entries.push((
                "minecraft:suspicious_stew_effects",
                Tag::List(
                    d.stew_effects
                        .iter()
                        .map(|(id, duration)| {
                            // The codec leaves out the default duration (160).
                            let mut e = vec![("id", Tag::String((&**id).into()))];
                            if *duration != 160 {
                                e.push(("duration", Tag::Int(*duration)));
                            }
                            Tag::compound(e)
                        })
                        .collect(),
                ),
            ));
        }
        // MapPostProcessing.id: this is part of the live patch, not ItemStack.CODEC.
        if include_map_post_processing
            && let Some(directive) = d.map_post_processing
            && selector("minecraft:map_post_processing")
        {
            entries.push((
                "minecraft:map_post_processing",
                Tag::Int(i32::from(directive)),
            ));
        }
        if !d.charged.is_empty() && selector("minecraft:charged_projectiles") {
            entries.push((
                "minecraft:charged_projectiles",
                Tag::List(d.charged.iter().map(template_tag).collect()),
            ));
        }
        if let Some(a) = d.ominous_amplifier
            && selector("minecraft:ominous_bottle_amplifier")
        {
            entries.push(("minecraft:ominous_bottle_amplifier", Tag::Int(i32::from(a))));
        }
        if !d.container.is_empty() && selector("minecraft:container") {
            let slots = d
                .container
                .iter()
                .map(|(slot, stack)| {
                    Tag::compound([
                        ("slot", Tag::Int(i32::from(*slot))),
                        ("item", template_tag(stack)),
                    ])
                })
                .collect();
            entries.push(("minecraft:container", Tag::List(slots)));
        }
    }
    let mut out: sunrise_core::nbt::Compound = entries
        .into_iter()
        .map(|(k, v)| (JavaString::from(k), v))
        .collect();
    if let Some(d) = data {
        for (k, v) in &d.other {
            if !selector(k.strip_prefix('!').unwrap_or(k.as_str())) {
                continue;
            }
            out.entry(JavaString::from(k.as_str()))
                .or_insert_with(|| canonical_other_component(k, v));
        }
    }
    (!out.is_empty()).then_some(Tag::Compound(out))
}

fn canonicalize_component_patch(components: &mut Tag) {
    // MobEffectInstance.Details' codec always saves the resolved icon flag.
    // An omitted icon follows show_particles, whose default is true.
    if let Some(Tag::List(effects)) = components
        .get_mut("minecraft:potion_contents")
        .and_then(|potion| potion.get_mut("custom_effects"))
    {
        for effect in effects {
            let shown = effect.get("show_particles").and_then(int).unwrap_or(1) != 0;
            if let Tag::Compound(fields) = effect {
                fields
                    .entry(JavaString::from("show_icon"))
                    .or_insert(Tag::Byte(i8::from(shown)));
            }
        }
    }
    // LodestoneTracker.CODEC: optionalFieldOf("tracked", true).
    if let Some(Tag::Compound(tracker)) = components.get_mut("minecraft:lodestone_tracker")
        && tracker.get(&JavaString::from("tracked")) == Some(&Tag::Byte(1))
    {
        tracker.remove(&JavaString::from("tracked"));
    }
}

/// Borrow an ASCII component key in `buf` without allocating. Every modeled ID
/// is ASCII and fits; other spellings are preserved in the raw patch.
fn ascii_key<'a>(key: &JavaString, buf: &'a mut [u8; 64]) -> Option<&'a str> {
    let units = &key.0;
    if units.len() > buf.len() {
        return None;
    }
    for (slot, &unit) in buf.iter_mut().zip(units) {
        if unit > 0x7f {
            return None;
        }
        *slot = unit as u8;
    }
    std::str::from_utf8(&buf[..units.len()]).ok()
}

/// The saved tags of the component IDs `decode_components` models, found in one
/// pass over the compound. Reading them one at a time built a key and walked the
/// tree for every ID, present or not, and most items carry one or two.
#[derive(Default)]
struct Seen<'a> {
    damage: Option<&'a Tag>,
    death_protection: Option<&'a Tag>,
    consumable: Option<&'a Tag>,
    use_remainder: Option<&'a Tag>,
    sulfur_cube_content: Option<&'a Tag>,
    kinetic_weapon: Option<&'a Tag>,
    banner_patterns: Option<&'a Tag>,
    potion_contents: Option<&'a Tag>,
    enchantments: Option<&'a Tag>,
    stored_enchantments: Option<&'a Tag>,
    repair_cost: Option<&'a Tag>,
    custom_name: Option<&'a Tag>,
    map_id: Option<&'a Tag>,
    base_color: Option<&'a Tag>,
    writable_book_content: Option<&'a Tag>,
    written_book_content: Option<&'a Tag>,
    fireworks: Option<&'a Tag>,
    firework_explosion: Option<&'a Tag>,
    dyed_color: Option<&'a Tag>,
    pot_decorations: Option<&'a Tag>,
    bundle_contents: Option<&'a Tag>,
    suspicious_stew: Option<&'a Tag>,
    map_post_processing: Option<&'a Tag>,
    charged_projectiles: Option<&'a Tag>,
    container: Option<&'a Tag>,
    amplifier: Option<&'a Tag>,
    trim: Option<&'a Tag>,
}
impl<'a> Seen<'a> {
    fn read(map: &'a Compound) -> Self {
        let mut seen = Self::default();
        let mut buf = [0u8; 64];
        for (key, value) in map {
            let field = match ascii_key(key, &mut buf) {
                Some("minecraft:damage") => &mut seen.damage,
                Some("minecraft:death_protection") => &mut seen.death_protection,
                Some("minecraft:consumable") => &mut seen.consumable,
                Some("minecraft:use_remainder") => &mut seen.use_remainder,
                Some("minecraft:sulfur_cube_content") => &mut seen.sulfur_cube_content,
                Some("minecraft:kinetic_weapon") => &mut seen.kinetic_weapon,
                Some("minecraft:banner_patterns") => &mut seen.banner_patterns,
                Some("minecraft:potion_contents") => &mut seen.potion_contents,
                Some("minecraft:enchantments") => &mut seen.enchantments,
                Some("minecraft:stored_enchantments") => &mut seen.stored_enchantments,
                Some("minecraft:repair_cost") => &mut seen.repair_cost,
                Some("minecraft:custom_name") => &mut seen.custom_name,
                Some("minecraft:map_id") => &mut seen.map_id,
                Some("minecraft:base_color") => &mut seen.base_color,
                Some("minecraft:writable_book_content") => &mut seen.writable_book_content,
                Some("minecraft:written_book_content") => &mut seen.written_book_content,
                Some("minecraft:fireworks") => &mut seen.fireworks,
                Some("minecraft:firework_explosion") => &mut seen.firework_explosion,
                Some("minecraft:dyed_color") => &mut seen.dyed_color,
                Some("minecraft:pot_decorations") => &mut seen.pot_decorations,
                Some("minecraft:bundle_contents") => &mut seen.bundle_contents,
                Some("minecraft:suspicious_stew_effects") => &mut seen.suspicious_stew,
                Some("minecraft:map_post_processing") => &mut seen.map_post_processing,
                Some("minecraft:charged_projectiles") => &mut seen.charged_projectiles,
                Some("minecraft:container") => &mut seen.container,
                Some("minecraft:ominous_bottle_amplifier") => &mut seen.amplifier,
                Some("minecraft:trim") => &mut seen.trim,
                _ => continue,
            };
            *field = Some(value);
        }
        seen
    }
}
/// Damage and extra components from a stack's `components` compound.
pub fn decode_components(components: Option<&Tag>) -> (u32, Option<Box<StackData>>) {
    decode_components_inner(components, true, None)
}

/// Install the typed effects already projected from this packet's stack and its templates.
pub fn decode_network_components(
    components: Option<&Tag>,
    effects: &Arc<[Admitted]>,
) -> (u32, Option<Box<StackData>>) {
    decode_components_inner(components, true, Some(effects))
}

fn decode_components_inner(
    components: Option<&Tag>,
    compact_banner: bool,
    received_effects: Option<&Arc<[Admitted]>>,
) -> (u32, Option<Box<StackData>>) {
    let Some(Tag::Compound(map)) = components else {
        return (0, None);
    };
    let seen = Seen::read(map);
    // NbtOps.getNumberValue supplies a boxed number; Codec.INT truncates its value.
    let damage = sunrise_world::datafix::dynamic::as_int(seen.damage, 0).max(0) as u32;
    let potion = match seen.potion_contents {
        Some(Tag::String(s)) => Some(Arc::from(s.to_string_lossy().as_str())),
        Some(t) => t
            .get("potion")
            .and_then(string)
            .map(|s| Arc::from(s.as_str())),
        None => None,
    };
    let potion_extra = match seen.potion_contents {
        Some(Tag::Compound(map)) => map
            .iter()
            .filter(|(k, _)| k.to_string_lossy() != "potion")
            .map(|(k, v)| (k.to_string_lossy(), v.clone()))
            .collect(),
        _ => Vec::new(),
    };
    let mut death_tag = seen.death_protection.cloned();
    let mut death_protection_refused = false;
    let death_protection = death_tag
        .as_mut()
        .and_then(|raw| {
            crate::items::stack_effects(
                raw,
                "death_effects",
                received_effects.map(AsRef::as_ref),
                Some(&mut death_protection_refused),
            )
            .ok()
        })
        .map(Arc::new);
    let retain_effects = |template: Option<&Tag>| {
        received_effects
            .filter(|effects| template.is_some() && !effects.is_empty())
            .cloned()
    };
    let mut data = StackData {
        villager_food: None,
        cooking_fuel: None,
        damage_set: damage == 0 && seen.damage.is_some(),
        enchantments: read_levels(seen.enchantments),
        stored_enchantments: read_levels(seen.stored_enchantments),
        repair_cost: seen.repair_cost.and_then(int).unwrap_or(0).max(0) as u32,
        explicit_zero_damage: damage == 0 && seen.damage.is_some(),
        custom_name: seen
            .custom_name
            .filter(|tag| read_text(tag).is_some())
            .cloned()
            .map(collapsed_text),
        potion,
        potion_extra,
        death_protection,
        death_protection_refused,
        use_remainder_effects: retain_effects(seen.use_remainder),
        sulfur_cube_effects: retain_effects(seen.sulfur_cube_content),
        consumable: seen
            .consumable
            .cloned()
            .and_then(|raw| ConsumableComponent::new(raw, received_effects.map(AsRef::as_ref))),
        kinetic_weapon: seen
            .kinetic_weapon
            .cloned()
            .map(crate::kinetic_weapon::KineticComponent::new),
        map_id: seen.map_id.and_then(crate::properties::component_int),
        banner_patterns: crate::banner_layer::layers_from_nbt(seen.banner_patterns),
        banner_patterns_set: matches!(seen.banner_patterns, Some(Tag::List(l)) if l.is_empty()),
        base_color: seen
            .base_color
            .and_then(string)
            .map(|s| Arc::from(s.as_str())),
        writable_pages: writable_raw(seen.writable_book_content),
        writable_filtered: writable_filtered(seen.writable_book_content),
        written: seen.written_book_content.map(written_book_from_tag),
        // Fireworks.CODEC decodes an empty explosion list regardless of the item's prototype.
        fireworks: seen
            .fireworks
            .filter(|f| matches!(f, Tag::Compound(_)))
            .and_then(|f| {
                let flight = f
                    .get("flight_duration")
                    .and_then(int)
                    .unwrap_or(0)
                    .clamp(0, 255) as u8;
                let explosions = match f.get("explosions") {
                    Some(Tag::List(list)) => list.iter().map(explosion_from).collect(),
                    None => Vec::new(),
                    Some(_) => return None,
                };
                Some((flight, explosions))
            }),
        firework_explosion: seen.firework_explosion.map(explosion_from),
        dyed_color: seen
            .dyed_color
            .and_then(sunrise_core::color::rgb_color_from_tag),
        pot_decorations: seen
            .pot_decorations
            .map(|p| {
                let side = |name: &str| {
                    p.get(name)
                        .and_then(|tag| template_from_with(tag, received_effects))
                };
                Box::new([side("back"), side("left"), side("right"), side("front")])
            })
            .filter(|sides| !all_bricks(sides)),
        bundle_contents: match seen.bundle_contents {
            Some(Tag::List(list)) => list
                .iter()
                .filter_map(|tag| template_from_with(tag, received_effects))
                .collect(),
            _ => Vec::new(),
        },
        stew_effects: match seen.suspicious_stew {
            Some(Tag::List(list)) => list
                .iter()
                .filter_map(|e| {
                    Some((
                        Arc::from(e.get("id").and_then(string)?.as_str()),
                        e.get("duration").and_then(int).unwrap_or(160) as i32,
                    ))
                })
                .collect(),
            _ => Vec::new(),
        },
        map_post_processing: match seen.map_post_processing {
            Some(Tag::String(s)) => Some(u8::from(s.to_string_lossy() != "lock")),
            Some(t) => int(t).map(|v| v.clamp(0, 1) as u8),
            None => None,
        },
        charged: match seen.charged_projectiles {
            Some(Tag::List(list)) => list
                .iter()
                .filter_map(|tag| {
                    crate::item_nbt::item_from_nbt_with_effects(tag, received_effects)
                })
                .collect(),
            _ => Vec::new(),
        },
        container: match seen.container {
            Some(Tag::List(list)) => list
                .iter()
                .filter_map(|e| {
                    Some((
                        e.get("slot").and_then(int)?.clamp(0, 255) as u8,
                        template_from_with(e.get("item")?, received_effects)?,
                    ))
                })
                .collect(),
            _ => Vec::new(),
        },
        ominous_banner: false,
        ominous_amplifier: seen
            .amplifier
            .and_then(crate::properties::component_int)
            .filter(|v| (0..=4).contains(v))
            .map(|v| v as u8),
        trim: seen.trim.and_then(trim::from_nbt),
        bundle_selected: BundleSelected::default(),
        bundle_weight: Default::default(),
        other: Vec::new(),
        wire: Wire::default(),
    };
    // DataComponentPatch.CODEC visits NbtOps' compound entries in Java map
    // order; the last entry for the component type is the one it stores.
    let mut fuel = None;
    for (key, v) in map {
        let key = key.to_string_lossy();
        match key.as_str() {
            cooking_fuel::ID | "cooking_fuel" | ":cooking_fuel" => {
                fuel = Some((cooking_fuel::ID, v));
                continue;
            }
            cooking_fuel::REMOVED | "!cooking_fuel" | "!:cooking_fuel" => {
                fuel = Some((cooking_fuel::REMOVED, v));
                continue;
            }
            _ => {}
        }
        if key == "minecraft:villager_food" {
            data.set_other(&key, Some(v.clone()));
            // Persistence has no refusal channel. Retain a malformed patch
            // for round-trip, but do not admit it to the nutrition cache.
            if data.villager_food.is_none() {
                data.other.push((key, v.clone()));
            }
            continue;
        }
        if MODELLED.contains(&key.as_str())
            && key != "minecraft:damage"
            && !data.sets_component(&key)
        {
            data.other
                .push((key.clone(), canonical_other_component(&key, v)));
        }
        // An explicitly empty stored_enchantments still counts as present
        // (`ItemStack.has`); the list is modelled, so keep a marker.
        if key == "minecraft:stored_enchantments"
            && data.stored_enchantments.is_empty()
            && matches!(v, Tag::Compound(_))
        {
            data.other
                .push((key, Tag::compound(Vec::<(&str, Tag)>::new())));
        } else if !MODELLED.contains(&key.as_str()) {
            // Loading has no error channel: keep malformed source values opaque,
            // but never leave a valid flattened custom-data compound undecoded.
            let value = if key == "minecraft:death_protection" && data.death_protection.is_some() {
                death_tag.take().unwrap_or_else(|| v.clone())
            } else if key == "minecraft:custom_data" {
                custom_data::decode(v.clone()).unwrap_or_else(|_| v.clone())
            } else {
                canonical_other_component(&key, v)
            };
            data.other.push((key, value));
        }
    }
    if let Some((id, value)) = fuel {
        data.set_other(id, Some(value.clone()));
    } else {
        data.other.sort_by(|a, b| a.0.cmp(&b.0));
    }
    if compact_banner {
        data.compact_ominous_banner();
    }
    (damage, (!data.is_empty()).then(|| Box::new(data)))
}

/// The `[id=value,...]` component list of an item argument (`ItemParser`)
/// as a `components` compound, retaining `!id` removal patches.
pub fn command_components(text: &str) -> Result<Tag, String> {
    let inner = text
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .ok_or("expected '['")?;
    let mut entries = sunrise_core::nbt::Compound::new();
    let mut rest = inner.trim_start();
    while !rest.is_empty() {
        let end = rest.find(['=', ',']).unwrap_or(rest.len());
        let key = rest[..end].trim();
        sunrise_world::item_defaults::validate_component_name(key)?;
        rest = &rest[end..];
        if let Some(value) = rest.strip_prefix('=') {
            let (tag, used) = sunrise_core::nbt::parse_snbt(value)?;
            let id = if key.contains(':') {
                key.to_owned()
            } else {
                format!("minecraft:{key}")
            };
            sunrise_world::item_defaults::validate_component_tag(&id, &tag)?;
            entries.insert(JavaString::from(id.as_str()), tag);
            rest = &value[used..];
        } else if let Some(key) = key.strip_prefix('!') {
            let id = if key.contains(':') {
                format!("!{key}")
            } else {
                format!("!minecraft:{key}")
            };
            entries.insert(
                JavaString::from(id.as_str()),
                Tag::compound([] as [(&str, Tag); 0]),
            );
        } else {
            return Err("expected '='".to_string());
        }
        rest = rest.trim_start();
        rest = rest.strip_prefix(',').unwrap_or(rest).trim_start();
    }
    Ok(Tag::Compound(entries))
}
#[cfg(test)]
#[test]
fn villager_food_stack_override_and_removal() {
    let mut info = prototype_items()
        .get("minecraft:red_banner")
        .unwrap()
        .clone();
    info.villager_food = Some(4);
    let info = Some(&info);
    use sunrise_world::loot::{LootData, PackOverlay};
    let root = std::env::temp_dir().join(format!("sunrise-villager-food-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("items.json"), r#"{"test:meal":{}}"#).unwrap();
    let table = br#"{"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"test:meal","modifier":[
        {"type":"minecraft:set_components","components":{"minecraft:villager_food":{"nutrition":7}}},
        {"type":"minecraft:set_components","components":{"!villager_food":{}}},
        {"type":"minecraft:set_components","components":{"villager_food":{"nutrition":9}}}
    ]}]}]}"#;
    let files = [("loot_table/test:meal".into(), vec![table.to_vec()])];
    let defaults = sunrise_world::item_defaults::read(&root).unwrap();
    let loot = LootData::load(&root, &PackOverlay::new(&files), &defaults).unwrap();
    let rolled = loot.roll_split("test:meal", &mut sunrise_core::random::LegacyRandom::new(7));
    std::fs::remove_dir_all(root).unwrap();
    let [rolled] = rolled.as_slice() else {
        panic!("one meal should be rolled");
    };

    let mut stack = ItemStack::new(&rolled.item, rolled.count as u32);
    let id = "minecraft:villager_food";
    assert_eq!(stack.villager_food(info), Some(4));
    assert_eq!(rolled.components.len(), 3);
    for ((key, tag), expected) in rolled.components.iter().zip([Some(7), None, Some(9)]) {
        stack.data_mut().set_loot_component(key, tag);
        assert_eq!(stack.villager_food(info), expected, "after {key}");
        assert_eq!(stack.carries_component(None, id), expected.is_some());
    }
    let saved = components_tag(0, stack.data()).unwrap();
    assert!(saved.get("!minecraft:villager_food").is_none());
    stack.extra = decode_components(Some(&saved)).1;
    assert_eq!(stack.villager_food(info), Some(9));
}

#[cfg(test)]
#[test]
fn malformed_persistent_villager_food_is_lossless_but_not_effective() {
    let id = "minecraft:villager_food";
    for nutrition in [0, -1] {
        let food = Tag::compound([("nutrition", Tag::Int(nutrition))]);
        let mut stack = ItemStack::new("test:meal", 1);
        stack.data_mut().set_loot_component(id, &Some(food.clone()));
        assert!(!stack.carries_component(None, id));
        assert_eq!(stack.villager_food(None), None);
        let patch = Tag::compound([(id, food.clone())]);
        stack.extra = decode_components(Some(&patch)).1;
        assert_eq!(stack.villager_food(None), None);
        assert_eq!(stack.data().unwrap().other(id), Some(&food));
        assert_eq!(
            encode_components(&stack, &crate::items::Items::empty()),
            Some(patch)
        );
    }
}

#[cfg(test)]
fn prototype_items() -> &'static crate::items::Items {
    static ITEMS: std::sync::OnceLock<crate::items::Items> = std::sync::OnceLock::new();
    ITEMS.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("sunrise-prototypes-{}", std::process::id()));
        std::fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
        std::fs::write(root.join("items.json"), r#"{"minecraft:red_banner":{"minecraft:banner_patterns":[]},"minecraft:diamond_pickaxe":{"minecraft:damage":0},"minecraft:filled_map":{"minecraft:map_decorations":{}},"minecraft:enchanted_book":{"minecraft:stored_enchantments":{}},"minecraft:beehive":{"minecraft:block_state":{"honey_level":"0"}},"minecraft:bee_nest":{"minecraft:block_state":{"honey_level":"0"}}}"#).unwrap();
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let items = crate::items::Items::catalog(&root, &sunrise_world::loot::PackOverlay::NONE, &defaults).unwrap();
        std::fs::remove_dir_all(root).unwrap();
        items
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::banner_layer::BannerLayer;

    #[test]
    fn stack_data_that_compares_equal_hashes_equal() {
        use std::hash::{BuildHasher, RandomState};
        let rocket = |count: u32, patch: &[u8], selected: i32, order: [&str; 2]| {
            let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
            sword.data_mut().enchantments = order.iter().map(|id| ((*id).into(), 1)).collect();
            let mut stack = ItemStack::new("minecraft:firework_rocket", count);
            let data = stack.data_mut();
            data.fireworks = Some((2, Vec::new()));
            data.container = vec![(0, sword)];
            data.bundle_selected = BundleSelected(selected);
            let decoded = stack.clone();
            stack.data_mut().wire = Wire::new(decoded, patch.into());
            stack
        };
        let full = rocket(
            64,
            &[1, 64],
            0,
            ["minecraft:sharpness", "minecraft:mending"],
        );
        let half = rocket(
            32,
            &[1, 32],
            -1,
            ["minecraft:mending", "minecraft:sharpness"],
        );
        let (full, half) = (full.data().unwrap(), half.data().unwrap());
        assert_eq!(full, half);
        let hasher = RandomState::new();
        assert_eq!(hasher.hash_one(full), hasher.hash_one(half));
        let mut other = half.clone();
        other.fireworks = Some((3, Vec::new()));
        assert_ne!(hasher.hash_one(full), hasher.hash_one(&other));
    }

    #[test]
    fn prototype_match_uses_decoded_fireworks() {
        let paper = |count, fields| {
            let patch = Tag::compound([("minecraft:fireworks", Tag::compound(fields))]);
            let (damage, extra) = decode_components(Some(&patch));
            ItemStack {
                damage,
                extra,
                ..ItemStack::new("minecraft:paper", count)
            }
        };
        let implicit = paper(1, vec![("flight_duration", Tag::Byte(1))]);
        let explicit = paper(
            2,
            vec![
                ("flight_duration", Tag::Byte(1)),
                ("explosions", Tag::List(Vec::new())),
            ],
        );
        let different = paper(1, vec![("flight_duration", Tag::Byte(2))]);
        let absent = ItemStack::new("minecraft:paper", 1);
        let items = crate::items::Items::empty();
        assert!(implicit.same_item_same_components_with_prototype(&explicit, &items));
        assert!(explicit.same_item_same_components_with_prototype(&implicit, &items));
        assert!(!implicit.same_item_same_components_with_prototype(&different, &items));
        assert!(!implicit.same_item_same_components_with_prototype(&absent, &items));
    }

    #[test]
    fn enchantment_component_equality_ignores_entry_order_but_not_levels() {
        for item in ["minecraft:diamond_sword", "minecraft:enchanted_book"] {
            let mut first = ItemStack::new(item, 1);
            first.enchant("minecraft:unbreaking", 3);
            first.enchant("minecraft:sharpness", 4);
            let mut second = ItemStack::new(item, 1);
            second.enchant("minecraft:sharpness", 4);
            second.enchant("minecraft:unbreaking", 3);
            assert!(first.same_item_same_components(&second));
            assert_eq!(&*first.crafting_enchantments()[0].0, "minecraft:unbreaking");
            assert_eq!(&*second.crafting_enchantments()[0].0, "minecraft:sharpness");
            second.enchant("minecraft:sharpness", 5);
            assert!(!first.same_item_same_components(&second));
        }
        // List-shaped components keep their vanilla order-sensitive equality.
        let mut first = ItemStack::new("minecraft:paper", 1);
        first.data_mut().set_other(
            "minecraft:lore",
            Some(Tag::List(vec![
                Tag::String("first".into()),
                Tag::String("second".into()),
            ])),
        );
        let mut second = first.clone();
        second.data_mut().set_other(
            "minecraft:lore",
            Some(Tag::List(vec![
                Tag::String("second".into()),
                Tag::String("first".into()),
            ])),
        );
        assert!(!first.same_item_same_components(&second));
    }

    /// `set_banner_pattern` (and `set_components`) hand a banner's layers over
    /// as a component; the renderer and the tooltip read `banner_patterns`, so
    /// they have to come out of it.
    #[test]
    fn a_loot_banner_patterns_component_dresses_the_banner() {
        let mut stack = ItemStack::new("minecraft:white_banner", 1);
        stack.data_mut().set_loot_component(
            "minecraft:banner_patterns",
            &Some(
                sunrise_core::json_nbt::json_to_tag(&serde_json::json!([
                    {"pattern": "minecraft:stripe_bottom", "color": "red"},
                    {"pattern": "minecraft:border", "color": "blue"},
                ]))
                .unwrap(),
            ),
        );
        let layers = stack.banner_layers();
        assert_eq!(
            layers,
            &[
                BannerLayer::new("minecraft:stripe_bottom", "red"),
                BannerLayer::new("minecraft:border", "blue"),
            ]
        );
        // A null component takes them off again.
        stack
            .data_mut()
            .set_loot_component("minecraft:banner_patterns", &None);
        assert!(stack.banner_layers().is_empty());
    }

    /// `set_book_cover` (and `set_components`) hand a written book's content
    /// over as a component; the fields the tooltip and the reading screen read
    /// have to come out of it, not just sit in `other` unread.
    #[test]
    fn a_loot_written_book_component_fills_the_book() {
        let mut stack = ItemStack::new("minecraft:written_book", 1);
        stack.data_mut().set_loot_component(
            "minecraft:written_book_content",
            &Some(
                sunrise_core::json_nbt::json_to_tag(&serde_json::json!({
                    "title": {"raw": "A Guide"},
                    "author": "Cartographer",
                    "generation": 2,
                    "pages": [{"raw": "page one"}, {"raw": "page two"}],
                }))
                .unwrap(),
            ),
        );
        let book = stack
            .data()
            .and_then(|d| d.written.as_ref())
            .expect("content");
        assert_eq!(book.title, "A Guide");
        assert_eq!(book.author, "Cartographer");
        assert_eq!(book.generation, 2);
        assert_eq!(book.pages, ["page one", "page two"]);
        // A null component is no component.
        stack
            .data_mut()
            .set_loot_component("minecraft:written_book_content", &None);
        assert!(stack.data().is_none_or(|d| d.written.is_none()));
    }

    /// A stack's own `attribute_modifiers` replace its type's whole; a removed
    /// component leaves none (`getOrDefault`).
    #[test]
    fn a_stacks_own_attribute_modifiers_replace_the_types() {
        let entry = |attribute: &str, id: &str, amount: f64, slot: Option<&str>| {
            let mut e = vec![
                ("type", Tag::String(attribute.into())),
                ("id", Tag::String(id.into())),
                ("amount", Tag::Double(amount.to_bits())),
                ("operation", Tag::String("add_value".into())),
            ];
            if let Some(slot) = slot {
                e.push(("slot", Tag::String(slot.into())));
            }
            Tag::compound(e)
        };
        let types = vec![ModifierRow::new(
            "minecraft:attack_damage",
            "minecraft:base_attack_damage",
            6.0,
            "add_value",
            "mainhand",
        )];
        let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
        assert_eq!(&*stack.default_attribute_modifiers_or(&types), &types[..]);
        stack.data_mut().set_other(
            "minecraft:attribute_modifiers",
            Some(Tag::List(vec![
                entry("minecraft:armor", "test:plate", 2.0, Some("chest")),
                entry("minecraft:luck", "test:charm", 1.0, None),
                // No amount: not an entry.
                Tag::compound([("type", Tag::String("minecraft:luck".into()))]),
            ])),
        );
        let own = stack.default_attribute_modifiers_or(&types);
        assert_eq!(
            &*own,
            &[
                ModifierRow::new("minecraft:armor", "test:plate", 2.0, "add_value", "chest"),
                ModifierRow::new("minecraft:luck", "test:charm", 1.0, "add_value", "any"),
            ][..]
        );
        let mut removed = ItemStack::new("minecraft:diamond_sword", 1);
        removed.data_mut().set_other(
            "!minecraft:attribute_modifiers",
            Some(Tag::compound(Vec::<(&str, Tag)>::new())),
        );
        assert!(removed.default_attribute_modifiers_or(&types).is_empty());
        // `matches` also compares the count.
        let one = ItemStack::new("minecraft:diamond_sword", 1);
        assert!(one.matches(&ItemStack::new("minecraft:diamond_sword", 1)));
        assert!(!one.matches(&ItemStack::new("minecraft:diamond_sword", 2)));
        assert!(!one.matches(&stack));
    }

    #[test]
    fn an_empty_enchanted_book_patch_is_elided_only_when_saved() {
        let stack = parse_stack(r#"{id:"minecraft:enchanted_book",count:1,components:{"minecraft:stored_enchantments":{}}}"#)
            .unwrap().unwrap();
        let live = components_tag(stack.damage, stack.data());
        assert!(
            live.as_ref()
                .unwrap()
                .get("minecraft:stored_enchantments")
                .is_some()
        );
        assert_eq!(
            crate::item_nbt::item_to_nbt(&stack, prototype_items()),
            sunrise_core::nbt::parse_snbt(r#"{id:"minecraft:enchanted_book",count:1}"#)
                .unwrap()
                .0,
        );
        assert_eq!(
            components_tag(stack.damage, stack.data()),
            live,
            "saving must not remove live presence"
        );
    }

    #[test]
    fn nondefault_stored_enchantment_patches_are_preserved() {
        for (item, patch) in [
            ("minecraft:book", r#"{"minecraft:stored_enchantments":{}}"#),
            ("minecraft:stone", r#"{"minecraft:stored_enchantments":{}}"#),
            (
                "minecraft:enchanted_book",
                r#"{"minecraft:stored_enchantments":{"minecraft:sharpness":1}}"#,
            ),
            (
                "minecraft:enchanted_book",
                r#"{"!minecraft:stored_enchantments":{}}"#,
            ),
        ] {
            let expected = sunrise_core::nbt::parse_snbt(patch).unwrap().0;
            let stack = parse_stack(&format!("{{id:'{item}',count:1,components:{patch}}}"))
                .unwrap()
                .unwrap();
            assert_eq!(
                encode_components(&stack, prototype_items()),
                Some(expected),
                "{item}: {patch}"
            );
        }
    }

    #[test]
    fn an_empty_book_patch_keeps_unrelated_components() {
        let stack = parse_stack(r#"{id:"minecraft:enchanted_book",count:1,components:{"minecraft:stored_enchantments":{},"minecraft:custom_data":{keep:7}}}"#).unwrap().unwrap();
        let expected = sunrise_core::nbt::parse_snbt(r#"{"minecraft:custom_data":{keep:7}}"#)
            .unwrap()
            .0;
        assert_eq!(encode_components(&stack, prototype_items()), Some(expected));
    }

    #[test]
    fn inline_trim_is_modeled_and_saved_without_raw_fallback() {
        let source = sunrise_core::nbt::parse_snbt(r#"{id:"minecraft:diamond_chestplate",count:1,components:{"minecraft:custom_data":{keep:284},"minecraft:trim":{material:{palette_id:"minecraft:gold",description:"Inline material"},pattern:{asset_id:"minecraft:coast",description:"Inline pattern",decal:0b}}}}"#).unwrap().0;
        let decoded = crate::item_nbt::item_from_nbt(&source).unwrap();
        let data = decoded.data().unwrap();
        assert!(
            data.trim.is_some(),
            "inline trim must be adopted into the actual model"
        );
        assert!(data.other("minecraft:trim").is_none(), "not a raw fallback");
        let mut fresh = ItemStack::new("minecraft:diamond_chestplate", 1);
        fresh.data_mut().trim = data.trim.clone();
        fresh.data_mut().set_other(
            "minecraft:custom_data",
            Some(Tag::compound([("keep", Tag::Int(284))])),
        );
        for stack in [&decoded, &fresh] {
            assert_eq!(
                components_tag(0, stack.data()),
                source.get("components").cloned()
            );
            assert_eq!(
                crate::item_nbt::item_to_nbt(stack, prototype_items()),
                source
            );
        }
    }

    fn check_lodestone_saved_default(decoded: bool) {
        const TRACKER: &str = "minecraft:lodestone_tracker";
        const CUSTOM: &str = "minecraft:custom_data";
        let target = Tag::compound([
            ("dimension", Tag::String("minecraft:overworld".into())),
            ("pos", Tag::IntArray(vec![12, 200, 12])),
        ]);
        let custom = Tag::compound([("keep", Tag::Int(17))]);
        assert_eq!(
            encode_components(&ItemStack::new("minecraft:compass", 1), prototype_items()),
            None
        );
        let mut only_custom = ItemStack::new("minecraft:compass", 1);
        only_custom
            .data_mut()
            .set_other(CUSTOM, Some(custom.clone()));
        assert_eq!(
            encode_components(&only_custom, prototype_items()),
            Some(Tag::compound([(CUSTOM, custom.clone())]))
        );
        // The valid false and absent controls precede the old true-default failure.
        for (tracked, has_target) in [
            (Some(false), true),
            (None, true),
            (Some(true), true),
            (Some(true), false),
        ] {
            let mut fields = Vec::new();
            let mut expected_fields = Vec::new();
            if has_target {
                fields.push(("target", target.clone()));
                expected_fields.push(("target", target.clone()));
            }
            if let Some(value) = tracked {
                fields.push(("tracked", Tag::Byte(i8::from(value))));
                if !value {
                    expected_fields.push(("tracked", Tag::Byte(0)));
                }
            }
            let source = Tag::compound(fields);
            let input = Tag::compound([(TRACKER, source.clone()), (CUSTOM, custom.clone())]);
            let expected = Tag::compound([
                (TRACKER, Tag::compound(expected_fields)),
                (CUSTOM, custom.clone()),
            ]);
            let stack = if decoded {
                let (damage, extra) = decode_components(Some(&input));
                ItemStack {
                    damage,
                    extra,
                    ..ItemStack::new("minecraft:compass", 1)
                }
            } else {
                let mut fresh = ItemStack::new("minecraft:compass", 1);
                assert!(fresh.extra.is_none());
                fresh.data_mut().set_other(TRACKER, Some(source.clone()));
                fresh.data_mut().set_other(CUSTOM, Some(custom.clone()));
                fresh
            };
            {
                assert_eq!(stack.data().unwrap().other(TRACKER), Some(&source));
                assert_eq!(
                    encode_components(&stack, prototype_items()),
                    Some(expected.clone())
                );
                assert_eq!(components_tag(0, stack.data()), Some(input.clone()));
                let saved = crate::item_nbt::item_to_nbt(&stack, prototype_items());
                assert_eq!(saved.get("components"), Some(&expected));
                assert_eq!(
                    saved.get("id"),
                    Some(&Tag::String("minecraft:compass".into()))
                );
                assert_eq!(saved.get("count"), Some(&Tag::Int(1)));
                assert_eq!(stack.data().unwrap().other(TRACKER), Some(&source));
            }
        }
        let mut targetless = ItemStack::new("minecraft:compass", 1);
        targetless
            .data_mut()
            .set_other(TRACKER, Some(Tag::compound([("tracked", Tag::Byte(1))])));
        assert_eq!(
            encode_components(&targetless, prototype_items()),
            Some(Tag::compound([(
                TRACKER,
                Tag::Compound(Default::default())
            )]))
        );
    }

    #[test]
    fn lodestone_tracker_saved_default_decoded() {
        check_lodestone_saved_default(true);
    }

    #[test]
    fn lodestone_tracker_saved_default_fresh_model() {
        check_lodestone_saved_default(false);
    }

    #[test]
    fn numeric_damage_decodes_and_saves_as_a_modelled_int() {
        let keep = Tag::compound([("keep", Tag::Int(87))]);
        for (raw, expected) in [
            (None, 0),
            (Some(Tag::Byte(1)), 1),
            (Some(Tag::Short(1)), 1),
            (Some(Tag::Int(1)), 1),
            (Some(Tag::Long(4_294_967_297)), 1),
            (Some(Tag::Float(1.5_f32.to_bits())), 1),
            (Some(Tag::Double(2.75_f64.to_bits())), 2),
            (Some(Tag::Float((-0.5_f32).to_bits())), 0),
            (
                Some(Tag::Float(2_147_483_648_f32.to_bits())),
                i32::MAX as u32,
            ),
            (Some(Tag::Double(1.0e20_f64.to_bits())), i32::MAX as u32),
        ] {
            let mut entries = vec![("minecraft:custom_data", keep.clone())];
            if let Some(raw) = &raw {
                entries.push(("minecraft:damage", raw.clone()));
            }
            let (damage, extra) = decode_components(Some(&Tag::compound(entries)));
            let loaded = ItemStack {
                damage,
                extra,
                ..ItemStack::new("minecraft:diamond_pickaxe", 1)
            };
            assert_eq!(loaded.damage, expected, "{raw:?}");
            assert!(loaded.wire_patch().is_none());
            assert!(loaded.data().unwrap().other("minecraft:damage").is_none());
            let saved = crate::item_nbt::item_to_nbt(&loaded, prototype_items());
            // The loaded pickaxe prototype has damage 0, so an explicit decoded
            // zero is omitted just like an absent damage component.
            assert_eq!(
                saved
                    .get("components")
                    .and_then(|c| c.get("minecraft:damage")),
                (expected != 0).then_some(&Tag::Int(expected as i32)),
                "{raw:?}"
            );
            assert_eq!(
                saved
                    .get("components")
                    .unwrap()
                    .get("minecraft:custom_data"),
                Some(&keep)
            );
            // No retained source or wire patch: only modelled damage is saved.
            let mut fresh = ItemStack::new("minecraft:diamond_pickaxe", 1);
            fresh.damage = expected;
            fresh
                .data_mut()
                .set_other("minecraft:custom_data", Some(keep.clone()));
            fresh.data_mut().damage_set = expected == 0 && raw.is_some();
            assert_eq!(
                crate::item_nbt::item_to_nbt(&fresh, prototype_items()),
                saved
            );
        }
    }

    #[test]
    fn potion_custom_effect_icons_save_the_codec_default() {
        let parse = |text: &str| sunrise_core::nbt::parse_snbt(text).unwrap().0;
        for (flags, icon) in [
            ("", 1),
            (",show_icon:0b", 0),
            (",show_icon:1b", 1),
            (",show_particles:0b", 0),
            (",show_particles:0b,show_icon:0b", 0),
            (",show_particles:0b,show_icon:1b,ambient:1b", 1),
        ] {
            let contents = |suffix: &str| {
                format!(
                    r#"{{potion:"minecraft:water",custom_color:123456,custom_effects:[{{id:"minecraft:speed",duration:100,amplifier:2{suffix}}}]}}"#
                )
            };
            let input = parse(&format!(
                r#"{{"minecraft:potion_contents":{},"minecraft:custom_data":{{keep:7}}}}"#,
                contents(flags)
            ));
            let expected = parse(&format!(
                r#"{{"minecraft:potion_contents":{},"minecraft:custom_data":{{keep:7}}}}"#,
                contents(&format!("{flags},show_icon:{icon}b"))
            ));
            let (damage, extra) = decode_components(Some(&input));
            let decoded = ItemStack {
                damage,
                extra,
                ..ItemStack::new("minecraft:potion", 1)
            };
            assert_eq!(
                encode_components(&decoded, prototype_items()),
                Some(expected.clone()),
                "decoded {flags}"
            );
            // Build a model directly, without decoded input or a retained raw patch.
            let mut fresh = ItemStack::new("minecraft:potion", 1);
            fresh.data_mut().potion = Some(Arc::from("minecraft:water"));
            let potion = input.get("minecraft:potion_contents").unwrap();
            for name in ["custom_color", "custom_effects"] {
                fresh
                    .data_mut()
                    .potion_extra
                    .push((name.into(), potion.get(name).unwrap().clone()));
            }
            fresh.data_mut().set_other(
                "minecraft:custom_data",
                input.get("minecraft:custom_data").cloned(),
            );
            assert_eq!(
                encode_components(&fresh, prototype_items()),
                Some(expected),
                "modeled {flags}"
            );
        }
        assert!(
            encode_components(&ItemStack::new("minecraft:potion", 1), prototype_items()).is_none()
        );
        let mut base = ItemStack::new("minecraft:potion", 1);
        base.data_mut().potion = Some(Arc::from("minecraft:water"));
        assert_eq!(
            encode_components(&base, prototype_items()),
            Some(parse(
                r#"{"minecraft:potion_contents":{potion:"minecraft:water"}}"#
            ))
        );
    }

    #[test]
    fn map_decoration_types_round_trip_with_default_namespaces() {
        let parse = |text| sunrise_core::nbt::parse_snbt(text).unwrap().0;
        let source = parse(
            r#"{"minecraft:map_decorations":{a:{type:"minecraft:mansion",x:10.5d,z:-20.0d,rotation:90.0f},"a+b":{type:"red_x",x:-1.25d,z:2.75d,rotation:-45.5f}},"minecraft:custom_data":{keep:7}}"#,
        );
        let expected = parse(
            r#"{"minecraft:map_decorations":{a:{type:"minecraft:mansion",x:10.5d,z:-20.0d,rotation:90.0f},"a+b":{type:"minecraft:red_x",x:-1.25d,z:2.75d,rotation:-45.5f}},"minecraft:custom_data":{keep:7}}"#,
        );
        assert!(decode_components(None).1.is_none());
        for input in [&source, &expected] {
            let (damage, extra) = decode_components(Some(input));
            assert_eq!(
                extra.as_ref().unwrap().other("minecraft:map_decorations"),
                expected.get("minecraft:map_decorations")
            );
            let loaded = ItemStack {
                damage,
                extra,
                ..ItemStack::new("minecraft:filled_map", 1)
            };
            assert_eq!(
                encode_components(&loaded, prototype_items()),
                Some(expected.clone())
            );
            // A fresh model has no original NBT or retained component source.
            let mut fresh = ItemStack::new("minecraft:filled_map", 1);
            assert!(fresh.extra.is_none());
            for id in ["minecraft:map_decorations", "minecraft:custom_data"] {
                fresh.data_mut().set_other(id, input.get(id).cloned());
            }
            assert_eq!(
                encode_components(&fresh, prototype_items()),
                Some(expected.clone())
            );
            assert!(fresh.data().unwrap().map_id.is_none());
        }
    }

    #[test]
    fn saved_nested_templates_keep_redundant_values_and_removals() {
        for patch in [
            r#"{"minecraft:banner_patterns":[]}"#,
            r#"{"!minecraft:damage":{}}"#,
        ] {
            let template = format!(r#"{{id:"minecraft:red_banner",components:{patch}}}"#);
            for (component, payload) in [
                ("minecraft:bundle_contents", format!("[{template}]")),
                ("minecraft:charged_projectiles", format!("[{template}]")),
                (
                    "minecraft:container",
                    format!("[{{slot:0,item:{template}}}]"),
                ),
                ("minecraft:pot_decorations", format!("{{back:{template}}}")),
                ("minecraft:use_remainder", template.clone()),
            ] {
                let source = format!(
                    r#"{{id:"example:carrier",count:1,components:{{"{component}":{payload}}}}}"#
                );
                let stack = parse_stack(&source).unwrap().unwrap();
                let saved = crate::item_nbt::item_to_nbt(&stack, prototype_items());
                let expected = sunrise_core::nbt::parse_snbt(&source).unwrap().0;
                assert_eq!(saved, expected, "{component}: {patch}");
                let live =
                    sunrise_core::nbt::parse_snbt(&format!(r#"{{"{component}":{payload}}}"#))
                        .unwrap()
                        .0;
                assert_eq!(
                    component_snapshot(&stack),
                    Some(live),
                    "{component}: {patch}"
                );
            }
        }
    }

    #[test]
    fn saved_empty_banner_patterns_equal_the_loaded_prototype() {
        let stack = parse_stack(
            r#"{id:"minecraft:red_banner",count:1,components:{"minecraft:banner_patterns":[]}}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            crate::item_nbt::item_to_nbt(&stack, prototype_items()),
            sunrise_core::nbt::parse_snbt(r#"{id:"minecraft:red_banner",count:1}"#)
                .unwrap()
                .0
        );
    }

    #[test]
    fn saved_zero_damage_equals_the_loaded_prototype() {
        let stack = parse_stack(r#"{id:"minecraft:diamond_pickaxe",count:1,components:{"minecraft:damage":0,"minecraft:custom_data":{keep:87}}}"#).unwrap().unwrap();
        assert_eq!(crate::item_nbt::item_to_nbt(&stack, prototype_items()), sunrise_core::nbt::parse_snbt(r#"{id:"minecraft:diamond_pickaxe",count:1,components:{"minecraft:custom_data":{keep:87}}}"#).unwrap().0);
    }

    #[test]
    fn saved_mod_item_without_a_prototype_retains_its_components() {
        let source = r#"{id:"example:tool",count:1,components:{"minecraft:damage":0,"minecraft:banner_patterns":[],"example:counter":0}}"#;
        let stack = parse_stack(source).unwrap().unwrap();
        assert_eq!(
            crate::item_nbt::item_to_nbt(&stack, prototype_items()),
            sunrise_core::nbt::parse_snbt(source).unwrap().0
        );
    }

    #[test]
    fn empty_map_decoration_patch_uses_filled_map_prototype() {
        let parse = |text| sunrise_core::nbt::parse_snbt(text).unwrap().0;
        let empty = parse(r#"{"minecraft:map_decorations":{}}"#);
        assert!(decode_components(None).1.is_none());
        assert!(
            encode_components(
                &ItemStack::new("minecraft:filled_map", 1),
                prototype_items()
            )
            .is_none()
        );
        let mut fresh = ItemStack::new("minecraft:filled_map", 1);
        fresh
            .data_mut()
            .other
            .push(("minecraft:map_decorations".into(), parse("{}")));
        // The value codec has no item prototype; only the saved patch elides it.
        assert_eq!(components_tag(0, fresh.data()), Some(empty.clone()));
        assert!(encode_components(&fresh, prototype_items()).is_none());
        let (damage, extra) = decode_components(Some(&empty));
        let loaded = ItemStack {
            damage,
            extra,
            ..ItemStack::new("minecraft:filled_map", 1)
        };
        let saved = crate::item_nbt::item_to_nbt(&loaded, prototype_items());
        assert_eq!(saved, parse(r#"{id:"minecraft:filled_map",count:1}"#));
        let back = crate::item_nbt::item_from_nbt(&saved).unwrap();
        assert!(encode_components(&back, prototype_items()).is_none());
        let keep = parse(r#"{"minecraft:map_decorations":{},"minecraft:custom_data":{keep:7}}"#);
        let (damage, extra) = decode_components(Some(&keep));
        let stack = ItemStack {
            damage,
            extra,
            ..ItemStack::new("minecraft:filled_map", 1)
        };
        assert_eq!(
            encode_components(&stack, prototype_items()),
            Some(parse(r#"{"minecraft:custom_data":{keep:7}}"#))
        );
    }

    #[test]
    fn nondefault_map_decoration_patches_round_trip() {
        let parse = |text| sunrise_core::nbt::parse_snbt(text).unwrap().0;
        let nonempty = parse(
            r#"{"minecraft:map_decorations":{"a+b":{type:"minecraft:red_x",x:-1.25d,z:2.75d,rotation:-45.5f}},"minecraft:custom_data":{keep:7}}"#,
        );
        let removed =
            parse(r#"{"!minecraft:map_decorations":{},"minecraft:custom_data":{keep:7}}"#);
        let empty = parse(r#"{"minecraft:map_decorations":{}}"#);
        for (item, components) in [
            ("minecraft:filled_map", &nonempty),
            ("minecraft:stone", &nonempty),
            ("minecraft:filled_map", &removed),
            ("minecraft:stone", &empty),
            ("minecraft:map", &empty),
        ] {
            let (damage, extra) = decode_components(Some(components));
            let stack = ItemStack {
                damage,
                extra,
                ..ItemStack::new(item, 1)
            };
            assert_eq!(
                encode_components(&stack, prototype_items()),
                Some(components.clone()),
                "{item}"
            );
            let back = crate::item_nbt::item_from_nbt(&crate::item_nbt::item_to_nbt(
                &stack,
                prototype_items(),
            ))
            .unwrap();
            assert_eq!(
                encode_components(&back, prototype_items()),
                Some(components.clone()),
                "{item}"
            );
        }
    }

    fn check_hive_saved_patch(decoded: bool) {
        let parse = |text| sunrise_core::nbt::parse_snbt(text).unwrap().0;
        let zero = parse(r#"{"minecraft:block_state":{honey_level:"0"}}"#);
        let keep = parse(
            r#"{"minecraft:block_state":{honey_level:"0"},"minecraft:custom_data":{keep:34}}"#,
        );
        let custom = parse(r#"{"minecraft:custom_data":{keep:34}}"#);
        let one = parse(r#"{"minecraft:block_state":{honey_level:"1"}}"#);
        let empty = parse(r#"{"minecraft:block_state":{}}"#);
        let extra = parse(r#"{"minecraft:block_state":{honey_level:"0",facing:"south"}}"#);
        let removed = parse(r#"{"!minecraft:block_state":{},"minecraft:custom_data":{keep:34}}"#);
        for item in ["minecraft:beehive", "minecraft:bee_nest", "minecraft:stone"] {
            assert!(encode_components(&ItemStack::new(item, 1), prototype_items()).is_none());
            for source in [&zero, &keep, &one, &empty, &extra, &removed] {
                let mut stack = ItemStack::new(item, 1);
                if decoded {
                    (stack.damage, stack.extra) = decode_components(Some(source));
                } else {
                    // A fresh model, without retained source NBT.
                    assert!(stack.extra.is_none());
                    let Tag::Compound(entries) = source else {
                        unreachable!()
                    };
                    for (id, value) in entries {
                        stack
                            .data_mut()
                            .set_other(&id.to_string_lossy(), Some(value.clone()));
                    }
                }
                let before = stack.data().cloned();
                assert_eq!(
                    components_tag(stack.damage, stack.data()),
                    Some(source.clone())
                );
                let saved = encode_components(&stack, prototype_items());
                // Saved patch canonicalization must not change live/unsaved values.
                assert_eq!(stack.data(), before.as_ref());
                assert_eq!(
                    components_tag(stack.damage, stack.data()),
                    Some(source.clone())
                );
                let expected = if item != "minecraft:stone" && source == &zero {
                    None
                } else if item != "minecraft:stone" && source == &keep {
                    Some(custom.clone())
                } else {
                    Some(source.clone())
                };
                assert_eq!(saved, expected, "{item}, decoded={decoded}, {source:?}");
                let tag = crate::item_nbt::item_to_nbt(&stack, prototype_items());
                assert_eq!(tag.get("components"), expected.as_ref());
                let back = crate::item_nbt::item_from_nbt(&tag).unwrap();
                assert_eq!(encode_components(&back, prototype_items()), expected);
            }
        }
    }

    fn check_max_stack_saved_prototype(decoded: bool) {
        use crate::items::Items;
        use sunrise_world::loot::PackOverlay;
        let root =
            std::env::temp_dir().join(format!("h088-prototypes-{}-{decoded}", std::process::id()));
        std::fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
        std::fs::write(root.join("items.json"), r#"{"minecraft:iron_helmet":{"minecraft:max_stack_size":1},"minecraft:stone":{"minecraft:max_stack_size":64},"minecraft:ender_pearl":{"minecraft:max_stack_size":16},"audit:changed":{"minecraft:max_stack_size":7},"audit:missing":{}}"#).unwrap();
        let items = Items::catalog(
            &root,
            &PackOverlay::NONE,
            &sunrise_world::item_defaults::read(&root).unwrap(),
        )
        .unwrap();
        let parse = |text| sunrise_core::nbt::parse_snbt(text).unwrap().0;
        for (id, prototype, has_default) in [
            ("minecraft:iron_helmet", 1, true),
            ("minecraft:stone", 64, true),
            ("minecraft:ender_pearl", 16, true),
            ("audit:changed", 7, true),
            ("audit:missing", 64, false),
            ("audit:unknown", 7, false),
        ] {
            assert!(encode_components(&ItemStack::new(id, 1), &items).is_none());
            for (key, value, elided) in [
                ("minecraft:max_stack_size", Tag::Int(prototype), has_default),
                ("minecraft:max_stack_size", Tag::Int(3), false),
                // In this catalog, only audit:missing has a loaded prototype
                // without max_stack_size. audit:unknown has no prototype.
                (
                    "!minecraft:max_stack_size",
                    parse("{}"),
                    id == "audit:missing",
                ),
            ] {
                let input = Tag::compound([
                    (key, value.clone()),
                    ("minecraft:custom_data", parse("{keep:88}")),
                ]);
                let mut stack = ItemStack::new(id, 1);
                if decoded {
                    (stack.damage, stack.extra) = decode_components(Some(&input));
                } else {
                    assert!(stack.extra.is_none());
                    stack.data_mut().set_other(key, Some(value.clone()));
                    stack
                        .data_mut()
                        .set_other("minecraft:custom_data", Some(parse("{keep:88}")));
                }
                let before = stack.clone();
                assert_eq!(
                    components_tag(stack.damage, stack.data()),
                    Some(input.clone())
                );
                assert_eq!(
                    encode_components(&stack, prototype_items()),
                    Some(input.clone()),
                    "context-free API stays unchanged"
                );
                let expected = if elided {
                    Some(parse(r#"{"minecraft:custom_data":{keep:88}}"#))
                } else {
                    Some(input.clone())
                };
                assert_eq!(
                    encode_components(&stack, &items),
                    expected,
                    "{id}, decoded={decoded}"
                );
                assert_eq!(
                    crate::item_nbt::item_to_nbt(&stack, &items).get("components"),
                    expected.as_ref()
                );
                assert_eq!(stack, before, "no live stack mutation");
                assert_eq!(components_tag(stack.damage, stack.data()), Some(input));

                // When the elided entry was the only one, omit the field itself.
                stack.data_mut().set_other("minecraft:custom_data", None);
                let expected = (!elided).then(|| Tag::compound([(key, value)]));
                assert_eq!(
                    crate::item_nbt::item_to_nbt(&stack, &items).get("components"),
                    expected.as_ref(),
                    "{id}, {key} alone, decoded={decoded}"
                );
            }
        }
        // Reload the same known item's prototype through the real catalog loader.
        std::fs::write(
            root.join("items.json"),
            r#"{"minecraft:stone":{"minecraft:max_stack_size":7}}"#,
        )
        .unwrap();
        let changed = Items::catalog(
            &root,
            &PackOverlay::NONE,
            &sunrise_world::item_defaults::read(&root).unwrap(),
        )
        .unwrap();
        assert_eq!(items.get("minecraft:stone").unwrap().max_stack, 64);
        assert_eq!(changed.get("minecraft:stone").unwrap().max_stack, 7);
        for size in [7, 64] {
            let mut stack = ItemStack::new("minecraft:stone", 1);
            stack
                .data_mut()
                .set_other("minecraft:max_stack_size", Some(Tag::Int(size)));
            assert_eq!(encode_components(&stack, &items).is_none(), size == 64);
            assert_eq!(encode_components(&stack, &changed).is_none(), size == 7);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn decoded_max_stack_saved_patch_uses_loaded_prototype() {
        check_max_stack_saved_prototype(true);
    }
    #[test]
    fn modeled_max_stack_saved_patch_uses_loaded_prototype() {
        check_max_stack_saved_prototype(false);
    }
    #[test]
    fn decoded_hive_saved_patch_uses_whole_prototype() {
        check_hive_saved_patch(true);
    }

    #[test]
    fn fresh_hive_saved_patch_uses_whole_prototype() {
        check_hive_saved_patch(false);
    }

    #[test]
    fn an_absent_enchantments_component_is_not_enchantable() {
        let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
        stack.data_mut().set_other(
            "!minecraft:enchantments",
            Some(sunrise_core::nbt::Tag::compound(Vec::<(
                &str,
                sunrise_core::nbt::Tag,
            )>::new())),
        );
        stack.enchant("minecraft:sharpness", 1);
        assert!(stack.enchantments().is_empty());
        let mut book = ItemStack::new("minecraft:enchanted_book", 1);
        book.data_mut().set_other(
            "!minecraft:stored_enchantments",
            Some(sunrise_core::nbt::Tag::compound(Vec::<(
                &str,
                sunrise_core::nbt::Tag,
            )>::new())),
        );
        book.enchant("minecraft:sharpness", 1);
        assert!(book.stored_enchantments().is_empty());
    }

    #[test]
    fn components_round_trip() {
        let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
        stack.damage = 12;
        stack.enchant("minecraft:sharpness", 3);
        stack.enchant("minecraft:unbreaking", 2);
        stack.data_mut().repair_cost = 3;
        stack.data_mut().custom_name = Some(Tag::String("Blade".into()));
        let tag = encode_components(&stack, prototype_items()).expect("components");
        let (damage, extra) = decode_components(Some(&tag));
        assert_eq!(damage, 12);
        let back = ItemStack {
            damage,
            extra,
            ..ItemStack::new("minecraft:diamond_sword", 1)
        };
        assert_eq!(back.enchantment_level("minecraft:sharpness"), 3);
        assert_eq!(back.enchantment_level("minecraft:unbreaking"), 2);
        assert_eq!(back.repair_cost(), 3);
        assert_eq!(back.custom_name().as_deref(), Some("Blade"));
    }

    #[test]
    fn enchantment_ids_without_a_namespace_are_minecraft_ones() {
        let levels = Tag::compound([
            ("knockback", Tag::Int(2)),
            ("minecraft:sharpness", Tag::Int(1)),
        ]);
        let components = Tag::compound([("minecraft:enchantments", levels)]);
        let (_, extra) = decode_components(Some(&components));
        let stack = ItemStack {
            extra,
            ..ItemStack::new("minecraft:wooden_sword", 1)
        };
        assert_eq!(stack.enchantment_level("minecraft:knockback"), 2);
        assert_eq!(stack.enchantment_level("minecraft:sharpness"), 1);
    }

    #[test]
    fn invalid_enchantment_levels_leave_the_other_entries_intact() {
        for component in ["minecraft:enchantments", "minecraft:stored_enchantments"] {
            for level in [-1, 0, 256, i32::MAX] {
                let components = Tag::compound([
                    ("minecraft:damage", Tag::Int(12)),
                    (
                        component,
                        Tag::compound([("looting", Tag::Int(level)), ("sharpness", Tag::Int(3))]),
                    ),
                ]);
                let (damage, extra) = decode_components(Some(&components));
                let data = extra.unwrap();
                let levels = if component == "minecraft:enchantments" {
                    data.enchantments
                } else {
                    data.stored_enchantments
                };
                assert_eq!(damage, 12);
                assert_eq!(levels, vec![(Arc::from("minecraft:sharpness"), 3)]);
            }
        }
    }

    #[test]
    fn custom_name_reads_the_extra_spans() {
        let name = Tag::compound([
            (
                "extra",
                Tag::List(vec![
                    Tag::compound([("text", Tag::String("Games ".into()))]),
                    Tag::compound([("text", Tag::String("Right click".into()))]),
                ]),
            ),
            ("text", Tag::String("".into())),
        ]);
        let components = Tag::compound([("minecraft:custom_name", name)]);
        let (_, extra) = decode_components(Some(&components));
        assert_eq!(
            extra.and_then(|d| d.custom_name_plain()).as_deref(),
            Some("Games Right click")
        );
    }

    #[test]
    fn plain_custom_name_is_saved_as_a_string() {
        // Component.tryCollapseToString: `{text:"Cool"}` is written "Cool";
        // a style or a sibling keeps the compound.
        let decode = |name: Tag| {
            let components = Tag::compound([("minecraft:custom_name", name)]);
            decode_components(Some(&components)).1.unwrap().custom_name
        };
        let plain = Tag::compound([("text", Tag::String("Cool".into()))]);
        assert_eq!(decode(plain), Some(Tag::String("Cool".into())));
        let styled = Tag::compound([
            ("text", Tag::String("Cool".into())),
            ("color", Tag::String("red".into())),
        ]);
        assert_eq!(decode(styled.clone()), Some(styled));
        let mut data = StackData::default();
        data.set_other(
            "minecraft:custom_name",
            Some(Tag::compound([("text", Tag::String("Loot".into()))])),
        );
        assert_eq!(data.custom_name, Some(Tag::String("Loot".into())));
    }

    #[test]
    fn rich_custom_name_is_the_single_saved_value_after_rename() {
        let name = Tag::compound([
            ("text", Tag::String("".into())),
            (
                "extra",
                Tag::List(vec![
                    Tag::compound([
                        ("text", Tag::String("A".into())),
                        ("color", Tag::String("#ff0000".into())),
                    ]),
                    Tag::compound([
                        ("text", Tag::String("B".into())),
                        ("color", Tag::String("#0000ff".into())),
                    ]),
                ]),
            ),
        ]);
        let components = Tag::compound([("minecraft:custom_name", name.clone())]);
        let (_, extra) = decode_components(Some(&components));
        let mut stack = ItemStack {
            extra,
            ..ItemStack::new("minecraft:stick", 1)
        };
        assert_eq!(stack.custom_name().as_deref(), Some("AB"));
        assert_eq!(
            encode_components(&stack, prototype_items()),
            Some(components)
        );
        stack
            .data_mut()
            .set_other("minecraft:custom_name", Some(Tag::String("Changed".into())));
        assert_eq!(stack.custom_name().as_deref(), Some("Changed"));
        assert_eq!(
            encode_components(&stack, prototype_items())
                .unwrap()
                .get("minecraft:custom_name"),
            Some(&Tag::String("Changed".into()))
        );
    }

    #[test]
    fn loot_name_preserves_plain_and_styled_components() {
        let mut stack = ItemStack::new("minecraft:stone", 1);
        stack
            .data_mut()
            .set_loot_name("minecraft:custom_name", &serde_json::json!("Rock"));
        assert_eq!(
            encode_components(&stack, prototype_items())
                .unwrap()
                .get("minecraft:custom_name"),
            Some(&Tag::String("Rock".into()))
        );
        let styled = serde_json::json!({"text": "Rock", "color": "red"});
        stack
            .data_mut()
            .set_loot_name("minecraft:custom_name", &styled);
        assert_eq!(
            encode_components(&stack, prototype_items())
                .unwrap()
                .get("minecraft:custom_name"),
            Some(&sunrise_game_data::scoreboard::json_to_tag(&styled))
        );
    }

    #[test]
    fn loot_component_keeps_a_styled_custom_name() {
        let name = serde_json::json!({"text":"Prize","color":"#ff8800"});
        let mut stack = ItemStack::new("minecraft:stick", 1);
        stack.data_mut().set_loot_component(
            "minecraft:custom_name",
            &Some(sunrise_core::json_nbt::json_to_tag(&name).unwrap()),
        );
        assert_eq!(stack.custom_name().as_deref(), Some("Prize"));
        let expected = sunrise_game_data::scoreboard::json_to_tag(&name);
        assert_eq!(
            encode_components(&stack, prototype_items())
                .unwrap()
                .get("minecraft:custom_name"),
            Some(&expected)
        );
        stack
            .data_mut()
            .set_loot_component("minecraft:custom_name", &None);
        assert!(stack.custom_name().is_none());
    }

    /// A chest's `set_components` patch: a potion's contents land where
    /// drinking and splashing read them, not only in its saved form, and the
    /// components Sunrise keeps as saved stay so.
    #[test]
    fn a_loot_component_sunrise_models_lands_in_its_field() {
        use serde_json::json;
        let mut swift = ItemStack::new("minecraft:splash_potion", 1);
        swift.data_mut().set_loot_component(
            "minecraft:potion_contents",
            &Some(
                sunrise_core::json_nbt::json_to_tag(&json!({"potion": "minecraft:swiftness"}))
                    .unwrap(),
            ),
        );
        assert_eq!(swift.potion(), Some("minecraft:swiftness"));
        let mut custom = ItemStack::new("minecraft:potion", 1);
        let data = custom.data_mut();
        data.set_loot_component(
            "minecraft:custom_data",
            &Some(sunrise_core::json_nbt::json_to_tag(&json!({"page": 3})).unwrap()),
        );
        data.set_loot_component(
            "minecraft:damage",
            &Some(sunrise_core::json_nbt::json_to_tag(&json!(5)).unwrap()),
        );
        let effects = json!({"custom_effects": [{"id": "minecraft:resistance", "amplifier": 1, "duration": 1800}]});
        data.set_loot_component(
            "minecraft:potion_contents",
            &Some(sunrise_core::json_nbt::json_to_tag(&effects).unwrap()),
        );
        assert_eq!(
            custom.custom_potion_effects(),
            [("minecraft:resistance".to_owned(), 1800, 1)]
        );
        let data = custom.data().unwrap();
        assert!(data.other("minecraft:custom_data").is_some());
        assert!(
            data.other("minecraft:damage").is_some(),
            "a damage set before stays"
        );
        // Saved as before: the patch's contents, with the codec's icon flag.
        let saved = encode_components(&custom, prototype_items()).unwrap();
        assert_eq!(
            saved.get("minecraft:potion_contents").unwrap(),
            &sunrise_core::json_nbt::json_to_tag(&json!({"custom_effects": [{"id": "minecraft:resistance", "amplifier": 1, "duration": 1800, "show_icon": true}]})).unwrap()
        );
        // A null value takes the potion away again.
        swift
            .data_mut()
            .set_loot_component("minecraft:potion_contents", &None);
        assert_eq!(swift.potion(), None);
    }

    #[test]
    fn written_book_pages_may_be_component_lists() {
        // A page as Hypixel sends it through ViaVersion: a list, not a compound.
        let page = Tag::List(vec![
            Tag::compound([("text", Tag::String("".into()))]),
            Tag::compound([("text", Tag::String("Please read ".into()))]),
            Tag::compound([("text", Tag::String("the rules".into()))]),
        ]);
        let book = Tag::compound([
            (
                "title",
                Tag::compound([("raw", Tag::String("Rules".into()))]),
            ),
            ("author", Tag::String("Server".into())),
            (
                "pages",
                Tag::List(vec![
                    page,
                    Tag::compound([("raw", Tag::String("Two".into()))]),
                ]),
            ),
        ]);
        let components = Tag::compound([("minecraft:written_book_content", book)]);
        let (_, extra) = decode_components(Some(&components));
        let pages = extra.and_then(|d| d.written.clone()).map(|w| w.pages);
        assert_eq!(
            pages,
            Some(vec!["Please read the rules".to_owned(), "Two".to_owned()])
        );
    }

    #[test]
    fn banner_layers_and_shield_base_round_trip() {
        let mut stack = ItemStack::new("minecraft:shield", 1);
        stack.data_mut().base_color = Some(Arc::from("red"));
        *stack.data_mut().banner_layers_mut() = vec![
            crate::banner_layer::BannerLayer::new("minecraft:flower", "black"),
            crate::banner_layer::BannerLayer::new("minecraft:border", "white"),
        ];
        let tag = encode_components(&stack, prototype_items()).expect("components");
        let (_, extra) = decode_components(Some(&tag));
        let extra = extra.expect("data");
        assert_eq!(extra.base_color.as_deref(), Some("red"));
        assert_eq!(extra.banner_layers(), stack.banner_layers());
    }

    #[test]
    fn ominous_items_round_trip() {
        let mut banner = ItemStack::new("minecraft:white_banner", 1);
        banner.data_mut().ominous_banner = true;
        let tag = encode_components(&banner, prototype_items()).unwrap();
        assert!(tag.get("minecraft:banner_patterns").is_some());
        let (_, back) = decode_components(Some(&tag));
        let back = back.unwrap();
        assert!(back.ominous_banner);
        assert!(back.other("minecraft:tooltip_display").is_none());

        banner.data_mut().set_other(
            "minecraft:tooltip_display",
            Some(Tag::Compound(Default::default())),
        );
        let (_, custom_display) =
            decode_components(encode_components(&banner, prototype_items()).as_ref());
        assert_eq!(
            custom_display.unwrap().other("minecraft:tooltip_display"),
            Some(&Tag::Compound(Default::default()))
        );
        let mut bottle = ItemStack::new("minecraft:ominous_bottle", 1);
        bottle.data_mut().ominous_amplifier = Some(3);
        let (_, back) = decode_components(encode_components(&bottle, prototype_items()).as_ref());
        assert_eq!(back.unwrap().ominous_amplifier, Some(3));
    }
    #[test]
    fn plain_stacks_have_no_components() {
        let stack = ItemStack::new("minecraft:stone", 64);
        assert!(encode_components(&stack, prototype_items()).is_none());
        let mut named = stack.clone();
        named.data_mut();
        named.tidy();
        assert_eq!(named, stack);
    }

    #[test]
    fn books_store_enchantments() {
        let mut book = ItemStack::new("minecraft:enchanted_book", 1);
        book.enchant("minecraft:mending", 1);
        assert!(book.enchantments().is_empty());
        assert_eq!(book.stored_enchantments().len(), 1);
        upgrade(
            &mut book.data_mut().stored_enchantments,
            "minecraft:mending",
            1,
        );
        assert_eq!(book.crafting_enchantments()[0].1, 1);
        let book = ItemStack::new("minecraft:book", 1).transmute("minecraft:enchanted_book", 1);
        assert!(book.extra.is_none());
    }
    #[test]
    fn command_components_decode_fireworks() {
        let tag = command_components(
            "[fireworks={flight_duration:2,explosions:[{shape:large_ball,colors:[I;16711680],has_twinkle:true}]},damage=3]",
        )
        .unwrap();
        let (damage, data) = decode_components(Some(&tag));
        assert_eq!(damage, 3);
        let (flight, explosions) = data.unwrap().fireworks.unwrap();
        assert_eq!(flight, 2);
        assert_eq!(
            explosions[0].shape,
            crate::firework::FireworkShape::LargeBall
        );
        assert_eq!(explosions[0].colors, vec![0xFF0000]);
        assert!(explosions[0].twinkle && !explosions[0].trail);
    }

    #[test]
    fn command_components_rejects_bare_component_without_equals() {
        assert_eq!(
            command_components("[custom_data]"),
            Err("expected '='".to_string())
        );
        assert_eq!(
            command_components("[!tool,custom_data]"),
            Err("expected '='".to_string())
        );
    }

    #[test]
    fn debug_stick_state_round_trips() {
        let mut stack = ItemStack::new("minecraft:debug_stick", 1);
        stack
            .data_mut()
            .set_debug_stick_property("minecraft:oak_stairs", "facing");
        stack
            .data_mut()
            .set_debug_stick_property("minecraft:oak_stairs", "half");
        stack
            .data_mut()
            .set_debug_stick_property("minecraft:stone", "axis");

        let tag = encode_components(&stack, prototype_items()).expect("debug stick components");
        let (_, extra) = decode_components(Some(&tag));
        let data = extra.expect("debug stick state");
        assert_eq!(
            data.debug_stick_property("minecraft:oak_stairs").as_deref(),
            Some("half")
        );
        assert_eq!(
            data.debug_stick_property("minecraft:stone").as_deref(),
            Some("axis")
        );
        assert_eq!(data.debug_stick_property("minecraft:dirt"), None);
    }

    /// The modelled component IDs, as `decode_components` read them before the
    /// single pass: one `BTreeMap` lookup per ID, whether or not the item
    /// carries it.
    fn baseline_decode_components(components: Option<&Tag>) -> (u32, Option<Box<StackData>>) {
        let Some(c) = components else {
            return (0, None);
        };
        let damage =
            sunrise_world::datafix::dynamic::as_int(c.get("minecraft:damage"), 0).max(0) as u32;
        let potion = match c.get("minecraft:potion_contents") {
            Some(Tag::String(s)) => Some(Arc::from(s.to_string_lossy().as_str())),
            Some(t) => t
                .get("potion")
                .and_then(string)
                .map(|s| Arc::from(s.as_str())),
            None => None,
        };
        let potion_extra = match c.get("minecraft:potion_contents") {
            Some(Tag::Compound(map)) => map
                .iter()
                .filter(|(k, _)| k.to_string_lossy() != "potion")
                .map(|(k, v)| (k.to_string_lossy(), v.clone()))
                .collect(),
            _ => Vec::new(),
        };
        let mut data = StackData {
            villager_food: None,
            cooking_fuel: None,
            damage_set: damage == 0 && c.get("minecraft:damage").is_some(),
            death_protection_refused: false,
            use_remainder_effects: None,
            sulfur_cube_effects: None,
            death_protection: c
                .get("minecraft:death_protection")
                .cloned()
                .and_then(|mut raw| {
                    crate::items::stack_effects(&mut raw, "death_effects", None, None).ok()
                })
                .map(Arc::new),
            consumable: c
                .get("minecraft:consumable")
                .cloned()
                .and_then(|raw| ConsumableComponent::new(raw, None)),
            kinetic_weapon: c
                .get("minecraft:kinetic_weapon")
                .cloned()
                .map(crate::kinetic_weapon::KineticComponent::new),

            enchantments: read_levels(c.get("minecraft:enchantments")),
            stored_enchantments: read_levels(c.get("minecraft:stored_enchantments")),
            repair_cost: c
                .get("minecraft:repair_cost")
                .and_then(int)
                .unwrap_or(0)
                .max(0) as u32,
            explicit_zero_damage: damage == 0 && c.get("minecraft:damage").is_some(),
            custom_name: c
                .get("minecraft:custom_name")
                .filter(|tag| read_text(tag).is_some())
                .cloned()
                .map(collapsed_text),
            potion,
            potion_extra,
            map_id: c.get("minecraft:map_id").and_then(int).map(|v| v as i32),
            banner_patterns: crate::banner_layer::layers_from_nbt(
                c.get("minecraft:banner_patterns"),
            ),
            banner_patterns_set: matches!(
                c.get("minecraft:banner_patterns"),
                Some(Tag::List(l)) if l.is_empty()
            ),
            base_color: c
                .get("minecraft:base_color")
                .and_then(string)
                .map(|s| Arc::from(s.as_str())),
            writable_pages: writable_raw(c.get("minecraft:writable_book_content")),
            writable_filtered: writable_filtered(c.get("minecraft:writable_book_content")),
            written: c
                .get("minecraft:written_book_content")
                .map(written_book_from_tag),
            fireworks: c
                .get("minecraft:fireworks")
                .map(|f| {
                    let flight = f
                        .get("flight_duration")
                        .and_then(int)
                        .unwrap_or(0)
                        .clamp(0, 255) as u8;
                    let explosions = match f.get("explosions") {
                        Some(Tag::List(list)) => list.iter().map(explosion_from).collect(),
                        _ => Vec::new(),
                    };
                    (flight, explosions)
                })
                .filter(|(flight, explosions)| !(*flight == 1 && explosions.is_empty())),
            firework_explosion: c.get("minecraft:firework_explosion").map(explosion_from),
            dyed_color: c
                .get("minecraft:dyed_color")
                .and_then(sunrise_core::color::rgb_color_from_tag),
            pot_decorations: c
                .get("minecraft:pot_decorations")
                .map(|p| {
                    let side = |name: &str| p.get(name).and_then(template_from);
                    Box::new([side("back"), side("left"), side("right"), side("front")])
                })
                .filter(|sides| !all_bricks(sides)),
            bundle_contents: match c.get("minecraft:bundle_contents") {
                Some(Tag::List(list)) => list.iter().filter_map(template_from).collect(),
                _ => Vec::new(),
            },
            stew_effects: match c.get("minecraft:suspicious_stew_effects") {
                Some(Tag::List(list)) => list
                    .iter()
                    .filter_map(|e| {
                        Some((
                            Arc::from(e.get("id").and_then(string)?.as_str()),
                            e.get("duration").and_then(int).unwrap_or(160) as i32,
                        ))
                    })
                    .collect(),
                _ => Vec::new(),
            },
            map_post_processing: match c.get("minecraft:map_post_processing") {
                Some(Tag::String(s)) => Some(u8::from(s.to_string_lossy() != "lock")),
                Some(t) => int(t).map(|v| v.clamp(0, 1) as u8),
                None => None,
            },
            charged: match c.get("minecraft:charged_projectiles") {
                Some(Tag::List(list)) => list
                    .iter()
                    .filter_map(crate::item_nbt::item_from_nbt)
                    .collect(),
                _ => Vec::new(),
            },
            container: match c.get("minecraft:container") {
                Some(Tag::List(list)) => list
                    .iter()
                    .filter_map(|e| {
                        Some((
                            e.get("slot").and_then(int)?.clamp(0, 255) as u8,
                            template_from(e.get("item")?)?,
                        ))
                    })
                    .collect(),
                _ => Vec::new(),
            },
            ominous_banner: c
                .get("minecraft:item_name")
                .and_then(|n| n.get("translate"))
                .and_then(string)
                .as_deref()
                == Some("block.minecraft.ominous_banner")
                && matches!(
                    c.get("minecraft:banner_patterns"),
                    Some(Tag::List(layers)) if layers.len() == OMINOUS_LAYERS.len()),
            ominous_amplifier: c
                .get("minecraft:ominous_bottle_amplifier")
                .and_then(int)
                .map(|v| v.clamp(0, 4) as u8),
            trim: c.get("minecraft:trim").and_then(trim::from_nbt),
            bundle_selected: BundleSelected::default(),
            bundle_weight: Default::default(),
            other: Vec::new(),
            wire: Wire::default(),
        };
        if let Tag::Compound(map) = c {
            for (k, v) in map {
                let key = k.to_string_lossy();
                let ominous_part = data.ominous_banner
                    && (matches!(key.as_str(), "minecraft:item_name" | "minecraft:rarity")
                        || (key == "minecraft:tooltip_display"
                            && v == &ominous_banner_components()[1].1));
                if key == "minecraft:stored_enchantments"
                    && data.stored_enchantments.is_empty()
                    && matches!(v, Tag::Compound(_))
                {
                    data.other
                        .push((key, Tag::compound(Vec::<(&str, Tag)>::new())));
                } else if !MODELLED.contains(&key.as_str()) && !ominous_part {
                    let value = if key == "minecraft:custom_data" {
                        custom_data::decode(v.clone()).unwrap_or_else(|_| v.clone())
                    } else {
                        canonical_other_component(&key, v)
                    };
                    data.other.push((key, value));
                }
            }
            data.other.sort_by(|a, b| a.0.cmp(&b.0));
        }
        (damage, (!data.is_empty()).then(|| Box::new(data)))
    }
    /// Components as saved: none, one kept ID, every modelled ID, and IDs the
    /// model keeps in `other`, one of them with a non-ASCII unit.
    fn component_cases() -> Vec<(&'static str, Tag)> {
        let parse = |text: &str| sunrise_core::nbt::parse_snbt(text).unwrap().0;
        vec![
            ("not a compound", Tag::String("minecraft:damage".into())),
            ("empty", Tag::compound(Vec::<(&str, Tag)>::new())),
            (
                "one kept id",
                parse(r#"{"minecraft:custom_data":{fixture:"save batch"}}"#),
            ),
            (
                "every modelled id",
                parse(
                    r#"{"minecraft:damage": 7,
                        "minecraft:potion_contents": {potion:"minecraft:water",custom_color:255,custom_effects:[{id:"minecraft:speed",duration:100,amplifier:1}]},
                        "minecraft:enchantments": {levels:{"minecraft:sharpness":3}},
                        "minecraft:stored_enchantments": {levels:{"minecraft:mending":1}},
                        "minecraft:repair_cost": 4,
                        "minecraft:custom_name": {text:"Blade",color:"red"},
                        "minecraft:map_id": 12,
                        "minecraft:banner_patterns": [{pattern:"minecraft:border",color:"blue"}],
                        "minecraft:base_color": "red",
                        "minecraft:writable_book_content": {pages:[{raw:"one"},{raw:"two"}]},
                        "minecraft:written_book_content": {title:{raw:"T"},author:"A",generation:1,pages:[{raw:"p"}]},
                        "minecraft:fireworks": {flight_duration:2,explosions:[{shape:"large_ball",colors:[I;16711680],has_twinkle:true}]},
                        "minecraft:firework_explosion": {shape:"star",colors:[I;255]},
                        "minecraft:dyed_color": 16711680,
                        "minecraft:pot_decorations": {back:{id:"minecraft:brick"},front:{id:"minecraft:brick"}},
                        "minecraft:bundle_contents": [{id:"minecraft:stone",count:2}],
                        "minecraft:suspicious_stew_effects": [{id:"minecraft:poison",duration:100}],
                        "minecraft:map_post_processing": "lock",
                        "minecraft:charged_projectiles": [{id:"minecraft:arrow",count:1}],
                        "minecraft:container": [{slot:0,item:{id:"minecraft:stone",count:3}}],
                        "minecraft:ominous_bottle_amplifier": 2,
                        "minecraft:trim": {material:"minecraft:gold",pattern:"minecraft:coast"},
                        "minecraft:item_name": {translate:"block.minecraft.ominous_banner"},
                        "minecraft:rarity": "uncommon",
                        "minecraft:tooltip_display": {hide_tooltip:true}
                    }"#,
                ),
            ),
            (
                "unknown and non-ASCII ids",
                parse(
                    r#"{"minecraft:custom_data":{keep:1},"minecraft:future_component":"x","minecraft:damage\u00e9":3}"#,
                ),
            ),
        ]
    }
    /// One pass over the compound decodes every shape the per-ID lookups did,
    /// including IDs kept in `other` and keys that are not ASCII.
    #[test]
    fn one_pass_decode_matches_the_per_id_lookups() {
        for (name, tag) in component_cases() {
            assert_eq!(
                decode_components(Some(&tag)),
                baseline_decode_components(Some(&tag)),
                "{name}"
            );
        }
        assert_eq!(decode_components(None), baseline_decode_components(None));
    }
    /// `decode_components` against the per-ID lookups, over the cases above. 25
    /// measured rounds alternate order and report the median microseconds per
    /// call.
    #[test]
    #[ignore = "microbenchmark; run with --ignored --nocapture"]
    fn benchmark_decode_components() {
        use std::{hint::black_box, time::Instant};
        for (name, tag) in component_cases() {
            let reps = 10_000;
            let mut times = [Vec::new(), Vec::new()];
            for round in 0..25 {
                for optimized in [round % 2 == 0, round % 2 != 0] {
                    let started = Instant::now();
                    for _ in 0..reps {
                        if optimized {
                            black_box(decode_components(black_box(Some(&tag))));
                        } else {
                            black_box(baseline_decode_components(black_box(Some(&tag))));
                        }
                    }
                    if round > 0 {
                        times[usize::from(optimized)]
                            .push(started.elapsed().as_secs_f64() * 1e6 / f64::from(reps));
                    }
                }
            }
            for t in &mut times {
                t.sort_by(f64::total_cmp);
            }
            let (base, new) = (times[0][times[0].len() / 2], times[1][times[1].len() / 2]);
            println!(
                "decode {name}: baseline {base:.3} us, new {new:.3} us, {:.2}x",
                base / new
            );
        }
    }
}

#[cfg(test)]
mod carried_component_tests {
    use super::*;

    #[test]
    fn invalid_payload_replacements_leave_existing_component_unchanged() {
        for (id, field) in [
            ("minecraft:consumable", "on_consume_effects"),
            ("minecraft:death_protection", "death_effects"),
        ] {
            let good = Tag::compound([(
                field,
                Tag::List(vec![Tag::compound([
                    ("type", Tag::String("example:charge".into())),
                    ("amount", Tag::Int(42)),
                ])]),
            )]);
            let bad = Tag::compound([(
                field,
                Tag::List(vec![Tag::compound([
                    ("type", Tag::String("example:charge".into())),
                    ("amount", Tag::End),
                ])]),
            )]);
            let mut data = StackData::default();
            data.set_other(id, Some(good.clone()));
            let consumable = data.consumable.clone();
            let death = data.death_protection.clone();
            data.set_other(id, Some(bad));
            assert_eq!(data.other(id), Some(&good));
            if let Some(before) = consumable {
                assert!(Arc::ptr_eq(&before, data.consumable.as_ref().unwrap()));
            }
            if let Some(before) = death {
                assert!(Arc::ptr_eq(
                    &before,
                    data.death_protection.as_ref().unwrap()
                ));
            }
            data.set_other(id, None);
            assert!(data.other(id).is_none());
        }
    }

    #[test]
    fn death_payload_is_decoded_once_and_replaced_with_component() {
        use crate::items::ConsumeEffect;
        let component = Tag::compound([(
            "death_effects",
            Tag::List(vec![Tag::compound([
                ("type", Tag::String("example:charge".into())),
                ("amount", Tag::Int(42)),
            ])]),
        )]);
        let patch = Tag::compound([("minecraft:death_protection", component.clone())]);
        let (_, extra) = decode_components(Some(&patch));
        let mut stack = ItemStack::new("minecraft:stick", 1);
        stack.extra = extra;
        let copied = stack.clone();
        let (ConsumeEffect::Native(a), ConsumeEffect::Native(b)) = (
            &stack.death_protection(None).unwrap()[0],
            &copied.death_protection(None).unwrap()[0],
        ) else {
            panic!("native effect")
        };
        assert!(Arc::ptr_eq(&a.data, &b.data));
        assert_eq!(a.data.get("amount"), Some(&Tag::Int(42)));
        assert_eq!(components_tag(0, stack.data()), Some(patch));
        stack
            .data_mut()
            .set_other("minecraft:death_protection", Some(component));
        assert!(matches!(
            &stack.death_protection(None).unwrap()[0],
            ConsumeEffect::Native(_)
        ));
        stack
            .data_mut()
            .set_other("minecraft:death_protection", None);
        assert!(stack.death_protection(None).is_none());
    }

    #[test]
    fn damage_keeps_a_canonical_int_including_explicit_zero() {
        for damage in [0, 5] {
            let original = Tag::compound([("minecraft:damage", Tag::Int(damage))]);
            let (decoded, extra) = decode_components(Some(&original));
            assert_eq!(decoded, damage as u32);
            let encoded = components_tag(decoded, extra.as_deref());
            assert_eq!(encoded, Some(original));
        }
        let removed = Tag::compound([("!minecraft:damage", Tag::Compound(Default::default()))]);
        let (damage, extra) = decode_components(Some(&removed));
        assert_eq!(components_tag(damage, extra.as_deref()), Some(removed));
    }

    #[test]
    fn extra_snapshot_codec_omits_typed_damage_but_preserves_the_patch() {
        for damage in [0, 5] {
            let original = Tag::compound([("minecraft:damage", Tag::Int(damage))]);
            let (damage, extra) = decode_components(Some(&original));
            let stack = ItemStack {
                damage,
                extra,
                ..ItemStack::new("minecraft:iron_sword", 1)
            };
            assert_eq!(encode_extra_components(&stack), None);
            assert_eq!(encode_components(&stack, prototype_items()), Some(original));
        }
        let patch = Tag::compound([
            ("minecraft:damage", Tag::Int(5)),
            (
                "minecraft:custom_name",
                Tag::String(JavaString::from("Snapshot")),
            ),
            ("!minecraft:consumable", Tag::Compound(Default::default())),
        ]);
        let (damage, extra) = decode_components(Some(&patch));
        let stack = ItemStack {
            damage,
            extra,
            ..ItemStack::new("minecraft:iron_sword", 1)
        };
        let mut expected = patch;
        if let Tag::Compound(entries) = &mut expected {
            entries.remove(&JavaString::from("minecraft:damage"));
        }
        assert_eq!(encode_extra_components(&stack), Some(expected));
    }

    #[test]
    fn explicit_empty_and_preserved_values_are_carried() {
        let original = Tag::compound([(
            "minecraft:potion_contents",
            Tag::Compound(Default::default()),
        )]);
        let (_, extra) = decode_components(Some(&original));
        let mut stack = ItemStack::new("minecraft:stick", 1);
        stack.extra = extra;
        assert!(stack.carries_component(None, "minecraft:potion_contents"));
        assert_eq!(components_tag(0, stack.data()), Some(original));
        stack
            .data_mut()
            .set_other("minecraft:potion_contents", None);
        assert!(!stack.carries_component(None, "minecraft:potion_contents"));
        stack
            .data_mut()
            .set_other("minecraft:repair_cost", Some(Tag::Int(5)));
        assert!(stack.carries_component(None, "minecraft:repair_cost"));
        assert_eq!(
            components_tag(0, stack.data())
                .unwrap()
                .get("minecraft:repair_cost"),
            Some(&Tag::Int(5))
        );
        stack
            .data_mut()
            .set_loot_component("minecraft:repair_cost", &Some(Tag::Int(5)));
        assert!(stack.carries_component(None, "minecraft:repair_cost"));
    }

    #[test]
    #[ignore = "daily"]
    fn default_component_presence_and_removal() {
        let items = crate::items::prepared_items().expect("prepared item resources");
        let info = items.get("minecraft:stick").expect("stick");
        let component = info.components.first().expect("default component");
        let mut stack = ItemStack::new("minecraft:stick", 1);
        assert!(stack.carries_component(Some(info), component));
        stack.data_mut().set_other(
            &format!("!{component}"),
            Some(Tag::Compound(Default::default())),
        );
        assert!(!stack.carries_component(Some(info), component));
    }

    /// `map_post_processing` is transient: a stack carries it while `components_tag` saves
    /// nothing for it, so a presence check reads the live field.
    #[test]
    fn a_transient_component_is_carried_but_not_saved() {
        let mut map = ItemStack::new("minecraft:filled_map", 1);
        map.data_mut().map_post_processing = Some(1);
        assert!(map.carries_component(None, "minecraft:map_post_processing"));
        assert!(!map.carries_component(None, "minecraft:enchantments"));
        assert!(components_tag(map.damage, map.data()).is_none());
    }

    /// A removed default is not carried; a stack's own value is.
    #[test]
    fn a_removed_component_is_not_carried() {
        let mut stack = ItemStack::new("minecraft:stick", 1);
        stack
            .data_mut()
            .set_other("!minecraft:lore", Some(Tag::String("removed".into())));
        assert!(!stack.carries_component(None, "minecraft:lore"));
        stack.data_mut().set_other(
            "minecraft:custom_data",
            Some(Tag::compound([("kept", Tag::Byte(1))])),
        );
        assert!(stack.carries_component(None, "minecraft:custom_data"));
    }
}

#[cfg(test)]
mod charged_projectile_template_tests {
    use super::*;

    #[test]
    fn charged_templates_omit_default_count_and_keep_components() {
        for count in [1, 2] {
            let mut projectile = ItemStack::new("minecraft:arrow", count);
            let marker = Tag::compound([("audit", Tag::Int(7))]);
            projectile
                .data_mut()
                .set_other("minecraft:custom_data", Some(marker.clone()));
            let mut outer = ItemStack::new("minecraft:crossbow", 1);
            outer.data_mut().charged.push(projectile.clone());
            outer.data_mut().set_other(
                "minecraft:custom_data",
                Some(Tag::compound([("audit", Tag::Int(9))])),
            );
            let encoded = encode_components(&outer, prototype_items()).unwrap();
            let Tag::List(list) = encoded.get("minecraft:charged_projectiles").unwrap() else {
                panic!("projectile list")
            };
            assert_eq!(list.len(), 1);
            assert_eq!(
                list[0].get("count"),
                (count != 1).then_some(Tag::Int(count as i32)).as_ref()
            );
            assert_eq!(
                list[0]
                    .get("components")
                    .unwrap()
                    .get("minecraft:custom_data"),
                Some(&marker)
            );
            assert_eq!(
                encoded.get("minecraft:custom_data"),
                Some(&Tag::compound([("audit", Tag::Int(9))]))
            );
            // Generic ItemStack/inventory encoding retains its required count.
            assert_eq!(
                crate::item_nbt::item_to_nbt(&projectile, prototype_items()).get("count"),
                Some(&Tag::Int(count as i32))
            );
            let (_, decoded) = decode_components(Some(&encoded));
            let restored = decoded.unwrap();
            assert_eq!(restored.charged[0].count, count);
            assert_eq!(
                restored.charged[0]
                    .data()
                    .unwrap()
                    .other("minecraft:custom_data"),
                Some(&marker)
            );
        }
    }
}

#[cfg(test)]
mod written_book_tests {
    use super::*;

    /// `WrittenBookContent.resolve`/`markResolved`: each page's selectors and
    /// scores become their text, the book is marked resolved, and a resolved
    /// book is left alone.
    #[test]
    fn a_written_books_pages_resolve_selectors_and_scores() {
        let selector = Tag::compound([("selector", Tag::String("@p".into()))]);
        let score = Tag::compound([(
            "score",
            Tag::compound([
                ("name", Tag::String("*".into())),
                ("objective", Tag::String("kills".into())),
            ]),
        )]);
        let mut book = WrittenBook {
            title: "t".into(),
            author: "a".into(),
            pages: vec!["@p".into(), "score".into(), "plain".into()],
            page_tags: vec![selector, score, Tag::String("plain".into())],
            resolved: false,
            ..Default::default()
        };
        let mut asked = 0;
        let changed = book.resolve(|tag| {
            asked += 1;
            if tag.get("selector").is_some() {
                return Some("Steve".into());
            }
            tag.get("score").map(|_| "7".into())
        });
        assert!(changed, "the pages were rewritten");
        assert!(book.resolved, "the book is marked resolved");
        assert_eq!(asked, 2, "a plain page is not asked");
        assert_eq!(book.pages, ["Steve", "7", "plain"]);
        assert_eq!(
            book.page_tags[0],
            Tag::compound([("text", Tag::String("Steve".into()))])
        );
        // A resolved book is never resolved again.
        assert!(!book.resolve(|_| Some("other".into())));
        assert_eq!(book.pages, ["Steve", "7", "plain"]);
    }

    /// `WrittenBookContent.resolvePage`: a page whose resolved text passes
    /// 32767 characters cancels the resolution, and the raw pages stay.
    #[test]
    fn an_over_long_resolved_page_leaves_the_pages_raw() {
        let mut book = WrittenBook {
            pages: vec!["x".into()],
            page_tags: vec![Tag::compound([("selector", Tag::String("@p".into()))])],
            ..Default::default()
        };
        let changed = book.resolve(|_| Some("y".repeat(32768)));
        assert!(!changed);
        assert!(book.resolved, "the book is still marked resolved");
        assert_eq!(book.pages, ["x"], "the raw pages stay");
    }

    /// `WrittenBookContent.CODEC` keeps the resolved flag through a save and a
    /// load.
    #[test]
    fn a_resolved_book_round_trips_its_flag() {
        let book = WrittenBook {
            title: "T".into(),
            author: "A".into(),
            pages: vec!["hi".into()],
            page_tags: vec![Tag::String("hi".into())],
            resolved: true,
            ..Default::default()
        };
        let tag = written_book_tag(&book);
        assert_eq!(tag.get("resolved"), Some(&Tag::Byte(1)));
        assert_eq!(written_book_from_tag(&tag), book);
        let unresolved = WrittenBook {
            resolved: false,
            ..book
        };
        assert_eq!(written_book_tag(&unresolved).get("resolved"), None);
    }

    /// `ItemStackTemplate.CODEC` reads `count` with `intRange(1, 99)`: outside it
    /// the template is rejected, absent it is 1.
    #[test]
    fn a_template_count_outside_one_to_ninety_nine_is_rejected() {
        let template = |count: Option<i32>| {
            let mut fields = vec![("id", Tag::String("minecraft:stone".into()))];
            if let Some(count) = count {
                fields.push(("count", Tag::Int(count)));
            }
            template_from(&Tag::compound(fields)).map(|s| s.count)
        };
        assert_eq!(template(None), Some(1));
        assert_eq!(template(Some(1)), Some(1));
        assert_eq!(template(Some(99)), Some(99));
        assert_eq!(template(Some(0)), None);
        assert_eq!(template(Some(100)), None);
        assert_eq!(template(Some(-1)), None);
    }
}

#[cfg(test)]
mod custom_data_boundary_tests {
    use super::*;

    const CUSTOM: &str = "minecraft:custom_data";

    #[test]
    fn custom_data_preserves_literal_tag_types_and_utf16_on_round_trip() {
        let expected = Tag::compound([
            ("byte", Tag::Byte(-7)),
            ("short", Tag::Short(300)),
            ("int", Tag::Int(70000)),
            ("long", Tag::Long(5000000000)),
            ("float", Tag::Float(0x3fc00000)),
            ("double", Tag::Double(0xc002000000000000)),
            ("bytes", Tag::ByteArray(vec![255, 2])),
            ("ints", Tag::IntArray(vec![-3, 70000])),
            ("longs", Tag::LongArray(vec![5000000000, -9])),
            ("empty", Tag::compound([] as [(&str, Tag); 0])),
        ]);
        let mut data = StackData::default();
        data.set_other(CUSTOM, Some(Tag::String("{byte:-7b,short:300s,int:70000,long:5000000000L,float:1.5f,double:-2.25d,bytes:[B;-1b,2b],ints:[I;-3,70000],longs:[L;5000000000L,-9L],empty:{}}".into())));
        assert_eq!(data.other(CUSTOM), Some(&expected));
        assert_eq!(
            components_tag(0, Some(&data)),
            Some(Tag::compound([(CUSTOM, expected.clone())]))
        );
        let input = Tag::compound([(CUSTOM, expected.clone())]);
        let (_, loaded) = decode_components(Some(&input));
        assert_eq!(loaded.unwrap().other(CUSTOM), Some(&expected));

        // Compound payloads are opaque, even for mixed lists and lone Java surrogates.
        let opaque = Tag::Compound(
            [
                (
                    JavaString(vec![0xd800]),
                    Tag::String(JavaString(vec![0xdc00])),
                ),
                (
                    JavaString::from("mixed"),
                    Tag::List(vec![
                        Tag::Byte(1),
                        Tag::String("x".into()),
                        Tag::compound([] as [(&str, Tag); 0]),
                    ]),
                ),
            ]
            .into_iter()
            .collect(),
        );
        data.set_other(CUSTOM, Some(opaque));
        let saved = components_tag(0, Some(&data)).unwrap();
        let (_, loaded) = decode_components(Some(&saved));
        let literal = Tag::Compound(
            [
                (
                    JavaString(vec![0xd800]),
                    Tag::String(JavaString(vec![0xdc00])),
                ),
                (
                    JavaString::from("mixed"),
                    Tag::List(vec![
                        Tag::Byte(1),
                        Tag::String("x".into()),
                        Tag::compound([] as [(&str, Tag); 0]),
                    ]),
                ),
            ]
            .into_iter()
            .collect(),
        );
        assert_eq!(loaded.unwrap().other(CUSTOM), Some(&literal));
        assert_eq!(saved, Tag::compound([(CUSTOM, literal)]));
    }

    #[test]
    fn load_setter_and_loot_normalize_flattened_custom_data() {
        let flattened = Tag::String("{audit:7,nested:{enabled:1b}}".into());
        let expected = Tag::compound([
            ("audit", Tag::Int(7)),
            ("nested", Tag::compound([("enabled", Tag::Byte(1))])),
        ]);
        let input = Tag::compound([(CUSTOM, flattened.clone())]);
        let (_, loaded) = decode_components(Some(&input));
        assert_eq!(loaded.unwrap().other(CUSTOM), Some(&expected));

        let mut data = StackData::default();
        data.set_other(CUSTOM, Some(flattened.clone()));
        assert_eq!(data.other(CUSTOM), Some(&expected));
        data.set_other(CUSTOM, None);
        assert!(data.other(CUSTOM).is_none());
        data.set_loot_component(CUSTOM, &Some(flattened));
        assert_eq!(data.other(CUSTOM), Some(&expected));
        assert_eq!(
            components_tag(0, Some(&data)),
            Some(Tag::compound([(CUSTOM, expected)]))
        );
    }

    #[test]
    fn invalid_custom_data_replacement_is_atomic_and_load_stays_invalid() {
        let good = Tag::compound([("audit", Tag::Int(7))]);
        let mut data = StackData::default();
        data.set_other(CUSTOM, Some(good));
        data.set_other("test:opaque", Some(Tag::String("unchanged".into())));
        let before = data.clone();
        for bad in [
            Tag::String("{audit:7} trailing".into()),
            Tag::String("{audit:".into()),
            Tag::String("[]".into()),
            Tag::Int(7),
        ] {
            data.set_other(CUSTOM, Some(bad.clone()));
            assert_eq!(data, before);
            data.set_loot_component(CUSTOM, &Some(bad.clone()));
            assert_eq!(data, before);

            let input = Tag::compound([(CUSTOM, bad.clone())]);
            let (_, loaded) = decode_components(Some(&input));
            let loaded = loaded.unwrap();
            assert_eq!(loaded.other(CUSTOM), Some(&bad));
            assert_eq!(components_tag(0, Some(&loaded)), Some(input));
        }
    }

    #[test]
    fn matches_with_overlay_matches_projected_enchantments_and_trim_without_cloning() {
        let mut base = ItemStack::new("minecraft:iron_chestplate", 1);
        base.damage = 4;
        let enchants = vec![
            (Arc::from("minecraft:protection"), 2),
            (Arc::from("minecraft:unbreaking"), 3),
        ];
        let trim: Trim = ("minecraft:gold".into(), "minecraft:coast".into());

        let mut projected = base.clone();
        projected.data_mut().enchantments = vec![
            (Arc::from("minecraft:unbreaking"), 3),
            (Arc::from("minecraft:protection"), 2),
        ];
        projected.data_mut().trim = Some(trim.clone());

        assert!(base.matches_with_overlay(&projected, &enchants, Some(&trim)));
        assert!(!base.matches_with_overlay(&projected, &[], Some(&trim)));
        assert!(!base.matches_with_overlay(&projected, &enchants, None));
        assert!(!base.matches_with_overlay(&base, &enchants, Some(&trim)));
        assert!(base.matches_with_overlay(&base, &[], None));

        let mut different_damage = projected.clone();
        different_damage.damage = 5;
        assert!(!base.matches_with_overlay(&different_damage, &enchants, Some(&trim)));
    }
}
