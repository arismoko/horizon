//! `Mob.finalizeSpawn` and its overrides, for every spawn path (natural
//! spawning, chunk generation, spawners, trial spawners, spawn eggs,
//! commands, structures, patrols, raids, jockeys, conversions...).
//!
//! Every draw vanilla takes from `level.getRandom()` comes from the random
//! the caller passes (the level's random for a running level, the region's
//! for chunk generation); the few that come from the entity's own random
//! (`Zombie.handleAttributes`, `Piglin.createSpawnWeapon`, salmon sizes) come
//! from `Mob::random`. The order of the draws follows vanilla's class chain:
//! each species runs its own part before or after its superclass as vanilla
//! does. Jockeys a mob brings (`EntityType.create` inside finalizeSpawn) are
//! made through [`FinalizeLevel::create`], finalized with the same random and
//! returned in [`Ctx::created`].
//!
//! Variant state kept in [`super::Extra`]: `variant` is the registry or enum
//! index of the species' variant (cat, wolf, frog, farm animals by climate,
//! zombie nautilus, horse, llama, fox, rabbit, parrot, axolotl, salmon,
//! tropical fish pattern, panda main gene, villager type), `variant2` the
//! second part (horse markings, llama strength, panda hidden gene, tropical
//! fish base colour), `variant3` the tropical fish pattern colour, and
//! `sound_variant` the index in the species' sound variant registry.
use super::{
    Mob, MobWorld, Species, farm::Climate, gear, spawn_rules::Reason, spawning::LocalDifficulty,
};
use crate::{
    enchant::Enchantments,
    items::{ItemStack, Items},
    living_attributes::EquipmentDeltas,
};
use std::{borrow::Cow, sync::Arc};
use sunrise_core::{equipment_slot::EquipmentSlot, geometry::Aabb, random::RandomSource};
use sunrise_world::attributes::{AttributeError, Instance, Observer, Operation, entity::Catalog};

/// The two independent decisions of NeoForge's `FinalizeSpawnEvent`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SpawnEvent {
    pub skip_initialization: bool,
    pub veto_admission: bool,
}

/// The level as finalizeSpawn reads it.
pub trait FinalizeLevel {
    /// The owning world's immutable builtin generation (vanilla for bare fixtures).
    fn mob_maps(&self) -> &crate::neoforge_mob_maps::NeoForgeMobMaps {
        crate::neoforge_mob_maps::NeoForgeMobMaps::vanilla()
    }
    /// The biome id at a block.
    fn biome(&self, pos: [i32; 3]) -> String;
    /// The biome at a block is in a biome tag (`minecraft:...`).
    fn biome_tag(&self, tag: &str, pos: [i32; 3]) -> bool;
    /// The structures with a piece at a block.
    fn structures_with_piece_at(&self, pos: [i32; 3]) -> Vec<String>;
    /// Whether `structure` belongs to a `worldgen/structure` tag (`minecraft:...`).
    fn structure_tag(&self, _tag: &str, _structure: &str) -> bool {
        false
    }
    /// `EnvironmentAttributes.MOON_PHASE` at a block (0 full moon to 7).
    fn moon_phase(&self, pos: [i32; 3]) -> usize;
    /// `ServerLevel.isSpawningMonsters`.
    fn spawning_monsters(&self) -> bool;
    /// `Level.noCollision(AABB)`.
    fn no_collision(&self, bounds: Aabb) -> bool;
    /// The first chicken (in entity order) whose box meets `bounds` and
    /// that is not being ridden (`Zombie.finalizeSpawn`'s chicken jockeys).
    fn free_chicken(&self, bounds: Aabb) -> Option<u32>;
    /// A chicken found by `free_chicken` carries the zombie `rider`
    /// (`setChickenJockey(true)` and `startRiding`).
    fn ride_chicken(&mut self, chicken: u32, rider: u32);
    /// `getGameTime`.
    fn game_time(&self) -> i64;
    /// `SpecialDates.isHalloween`.
    fn halloween(&self) -> bool;
    /// `EntityType.create(level, reason)`: a new mob of a type (an id and an
    /// own random from the caller), or None where it cannot exist (monsters
    /// in peaceful).
    fn create(&mut self, kind: &str, reason: Reason) -> Option<Mob>;
    /// Fires the level's finalizeSpawn event for a mob another mob's
    /// finalizeSpawn creates (`ZombieHorse`'s zombie, a chicken jockey).
    /// The default finalizes: a level without the event keeps vanilla's flow.
    fn spawn_event(&mut self, _: &mut Mob, _: Reason) -> SpawnEvent {
        SpawnEvent::default()
    }
    /// What finalizeSpawn needs besides the level: the variant registries,
    /// enchantments and items the level owns.
    fn spawn_data(&self) -> SpawnData<'_>;
}

/// `AgeableMob.AgeableMobGroupData`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ageable {
    pub size: i32,
    pub should_spawn_baby: bool,
    pub baby_chance: f32,
}
impl Ageable {
    pub fn new(should_spawn_baby: bool, baby_chance: f32) -> Self {
        Self {
            size: 0,
            should_spawn_baby,
            baby_chance,
        }
    }
    /// `AgeableMobGroupData(boolean)`.
    pub fn babies(should_spawn_baby: bool) -> Self {
        Self::new(should_spawn_baby, 0.05)
    }
    /// `AgeableMobGroupData(float)`.
    pub fn chance(baby_chance: f32) -> Self {
        Self::new(true, baby_chance)
    }
}
/// `SpawnGroupData`: what the mobs of a pack share.
#[derive(Clone, Debug, PartialEq)]
pub enum Group {
    Ageable(Ageable),
    /// `Zombie.ZombieGroupData`, and `Husk.HuskGroupData` with `tried_camel`.
    Zombie {
        baby: bool,
        jockey: bool,
        husk: Option<bool>,
    },
    /// `Spider.SpiderEffectsGroupData`.
    Spider(Option<&'static str>),
    Horse(Ageable, u8),
    Llama(Ageable, u8),
    Fox(Ageable, u8),
    Rabbit(Ageable, u8),
    Axolotl(Ageable, [u8; 2]),
    Wolf(
        Ageable,
        crate::lan::data::RegistryRef<super::variants::WolfVariant>,
    ),
    /// `AbstractSchoolingFish.SchoolSpawnGroupData` (the leader's id), and
    /// `TropicalFishGroupData` with its variant.
    School(u32, Option<(u8, u8, u8)>),
}
impl Group {
    fn ageable(&mut self) -> Option<&mut Ageable> {
        match self {
            Self::Ageable(a)
            | Self::Horse(a, _)
            | Self::Llama(a, _)
            | Self::Fox(a, _)
            | Self::Rabbit(a, _)
            | Self::Axolotl(a, _)
            | Self::Wolf(a, _) => Some(a),
            _ => None,
        }
    }
}

/// What finalizeSpawn needs besides the level: variant tables, enchantments.
#[derive(Clone)]
pub struct SpawnData<'a> {
    pub variants: &'a super::variants::VariantTables,
    pub enchantments: &'a Enchantments,
    pub items: &'a Items,
    /// `EntityTypes.CAMEL_HUSK` dimensions (width, height).
    pub camel_husk_size: (f32, f32),
}

/// A finalizeSpawn call and the entities it made.
pub struct Ctx<'a, L: FinalizeLevel + ?Sized, R: RandomSource> {
    pub level: &'a mut L,
    pub random: &'a mut R,
    pub difficulty: LocalDifficulty,
    /// Jockeys and mounts made along the way, finalized, with their riding
    /// links set (`Extra::mount` / `Extra::rider`).
    pub created: Vec<Mob>,
    /// The rider as it was when its new chicken was added to the level, before
    /// its equipment and attributes: what a generated chunk keeps of it
    /// (`ProtoChunk.addEntity` saves the chicken with its passenger then).
    pub jockey_rider: Option<Mob>,
}

// ---- attributes -----------------------------------------------------------

/// The attributes a mob's own store holds: the ones finalizeSpawn changes,
/// and the ones a status effect's modifier changes on every mob
/// ([`Mob::set_effect_modifiers`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attr {
    MaxHealth,
    MovementSpeed,
    FollowRange,
    AttackDamage,
    KnockbackResistance,
    SpawnReinforcements,
    JumpStrength,
    Armor,
    BurningTime,
    /// `Attributes.FLYING_SPEED`: what `FlyingMoveControl` flies at.
    FlyingSpeed,
    /// `Attributes.SAFE_FALL_DISTANCE`: `calculateFallPower` subtracts it
    /// from a fall (Jump Boost's modifier).
    SafeFallDistance,
    /// `Attributes.MAX_ABSORPTION`: `getMaxAbsorption` (Absorption's modifier).
    MaxAbsorption,
    /// `Attributes.WAYPOINT_TRANSMIT_RANGE`: `isTransmittingWaypoint`
    /// (Invisibility's modifier).
    WaypointTransmitRange,
}
impl Attr {
    pub const ALL: [Self; 13] = [
        Self::MaxHealth,
        Self::MovementSpeed,
        Self::FollowRange,
        Self::AttackDamage,
        Self::KnockbackResistance,
        Self::SpawnReinforcements,
        Self::JumpStrength,
        Self::Armor,
        Self::BurningTime,
        Self::FlyingSpeed,
        Self::SafeFallDistance,
        Self::MaxAbsorption,
        Self::WaypointTransmitRange,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::MaxHealth => "max_health",
            Self::MovementSpeed => "movement_speed",
            Self::FollowRange => "follow_range",
            Self::AttackDamage => "attack_damage",
            Self::KnockbackResistance => "knockback_resistance",
            Self::SpawnReinforcements => "spawn_reinforcements",
            Self::JumpStrength => "jump_strength",
            Self::Armor => "armor",
            Self::BurningTime => "burning_time",
            Self::FlyingSpeed => "flying_speed",
            Self::SafeFallDistance => "safe_fall_distance",
            Self::MaxAbsorption => "max_absorption",
            Self::WaypointTransmitRange => "waypoint_transmit_range",
        }
    }
    /// The attribute's registry id (`minecraft:` and its name), without
    /// building it: mob ticks and entity tracking ask for every mob.
    pub fn id(self) -> &'static str {
        match self {
            Self::MaxHealth => "minecraft:max_health",
            Self::MovementSpeed => "minecraft:movement_speed",
            Self::FollowRange => "minecraft:follow_range",
            Self::AttackDamage => "minecraft:attack_damage",
            Self::KnockbackResistance => "minecraft:knockback_resistance",
            Self::SpawnReinforcements => "minecraft:spawn_reinforcements",
            Self::JumpStrength => "minecraft:jump_strength",
            Self::Armor => "minecraft:armor",
            Self::BurningTime => "minecraft:burning_time",
            Self::FlyingSpeed => "minecraft:flying_speed",
            Self::SafeFallDistance => "minecraft:safe_fall_distance",
            Self::MaxAbsorption => "minecraft:max_absorption",
            Self::WaypointTransmitRange => "minecraft:waypoint_transmit_range",
        }
    }
}
/// The definition of each [`Attr`] (`RangedAttribute`'s default and
/// bounds), read once from the attribute catalog when the mob types load:
/// what a mob's own attribute record sanitizes its values with, as
/// `AttributeInstance` does; and the catalog itself, from which a mob builds
/// the instances of its other attributes. A type built without a catalog (a
/// test fixture, the player's render type) has none, and its values are not
/// clamped.
#[derive(Clone, Debug, Default)]
pub struct AttrBounds {
    attrs: [Option<sunrise_world::attributes::Definition>; Attr::ALL.len()],
    catalog: Option<Arc<sunrise_world::attributes::entity::Catalog>>,
}
impl AttrBounds {
    /// Each attribute's definition in `catalog`.
    pub fn of(catalog: &Arc<sunrise_world::attributes::entity::Catalog>) -> Self {
        Self {
            attrs: Attr::ALL.map(|a| catalog.definition(a.id())),
            catalog: Some(catalog.clone()),
        }
    }
    /// The catalog the type took its bounds from.
    pub fn catalog(&self) -> Option<&Arc<sunrise_world::attributes::entity::Catalog>> {
        self.catalog.as_ref()
    }
    /// `RangedAttribute.sanitizeValue` of `a`: NaN to the minimum, then into
    /// the bounds.
    pub fn sanitize(&self, a: Attr, value: f64) -> f64 {
        sanitize(self.attrs[a as usize], value)
    }
    /// The saved base `base` of the attribute `id` as a load takes it
    /// ([`crate::living_attributes::loaded_base`]): clamped into an [`Attr`]'s
    /// bounds when a mod that widened them is gone.
    pub(crate) fn loaded_base(&self, id: &str, base: f64) -> f64 {
        Attr::ALL
            .into_iter()
            .find(|a| a.id() == id)
            .and_then(|a| self.attrs[a as usize])
            .map_or(base, |definition| {
                crate::living_attributes::loaded_base(id, definition, base)
            })
    }
}
fn sanitize(definition: Option<sunrise_world::attributes::Definition>, value: f64) -> f64 {
    definition.map_or(value, |definition| definition.sanitize(value))
}
/// `AttributeModifier.Operation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    AddValue,
    AddMultipliedBase,
    AddMultipliedTotal,
}
impl From<Op> for sunrise_world::attributes::Operation {
    fn from(op: Op) -> Self {
        match op {
            Op::AddValue => Self::AddValue,
            Op::AddMultipliedBase => Self::AddMultipliedBase,
            Op::AddMultipliedTotal => Self::AddMultipliedTotal,
        }
    }
}
impl From<sunrise_world::attributes::Operation> for Op {
    fn from(operation: sunrise_world::attributes::Operation) -> Self {
        use sunrise_world::attributes::Operation;
        match operation {
            Operation::AddValue => Self::AddValue,
            Operation::AddMultipliedBase => Self::AddMultipliedBase,
            Operation::AddMultipliedTotal => Self::AddMultipliedTotal,
        }
    }
}
/// A permanent attribute modifier (`AttributeModifier`).
#[derive(Clone, Debug, PartialEq)]
pub struct Modifier {
    pub attr: Attr,
    pub id: std::borrow::Cow<'static, str>,
    pub amount: f64,
    pub op: Op,
}
/// Attribute base values that differ from the type's and the permanent
/// modifiers finalizeSpawn, a load, `/attribute` and the mob's effects
/// ([`Mob::set_effect_modifiers`]) added, in the order vanilla adds them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attributes {
    pub bases: Vec<(Attr, f64)>,
    pub modifiers: Vec<Modifier>,
    /// The transient modifiers mods added: in the value and synced, never
    /// saved.
    pub transient: Vec<Modifier>,
    /// `NAME_TAG_DISTANCE` from a server attribute update: the client reads
    /// it for nameplates and a save never writes it.
    pub name_tag_distance: Option<f64>,
    /// `BELOW_NAME_DISTANCE` from a server attribute update.
    pub below_name_distance: Option<f64>,
    /// `CAMERA_DISTANCE` from a server attribute update; never saved.
    pub camera_distance: Option<f64>,
    /// The transient minecraft:baby speed modifier was taken away
    /// (`/attribute ... modifier remove`) until the next setBaby. Not
    /// saved: a load runs setBaby again.
    pub baby_speed_removed: bool,
}
/// The attribute `/attribute` names (`follow_range`, not
/// `minecraft:follow_range`) when gameplay reads it from this store. Armor
/// has its own map (`generic_attributes`).
pub fn command_attr(short: &str) -> Option<Attr> {
    Attr::ALL
        .into_iter()
        .find(|a| *a != Attr::Armor && a.name() == short)
}
/// The type's default value of an attribute, if the mob has it.
pub fn default_value(m: &Mob, a: Attr) -> Option<f64> {
    m.kind.attributes.get(a.id()).copied()
}
pub fn base_value(m: &Mob, a: Attr) -> f64 {
    base_value_with(m, a, || default_value(m, a))
}
/// [`base_value`] with the type's default given (`default_value`), for a
/// caller that already looked it up for the mob's type.
pub fn base_value_with(m: &Mob, a: Attr, default: impl FnOnce() -> Option<f64>) -> f64 {
    m.extra
        .attributes
        .as_ref()
        .and_then(|s| s.bases.iter().find(|(x, _)| *x == a).map(|(_, v)| *v))
        .or_else(default)
        .unwrap_or(match a {
            Attr::BurningTime => 1.0,
            // The registry's default, as the type's `flying_speed` falls back to.
            Attr::FlyingSpeed => 0.4,
            // The type's own field: its row's value, else the registry's 3.
            Attr::SafeFallDistance => m.kind.safe_fall_distance,
            _ => 0.0,
        })
}
/// The attribute instances the mob's own construction and load leave in its
/// `AttributeMap`, which `AttributeMap.pack` saves although the loaded tag held
/// none: `AttributeMap.getInstance` makes one for any `getAttribute`, a read or
/// a `setBaseValue`. What `set_base` recorded (finalizeSpawn, a size, a baby
/// growing up) is saved apart; these are the rest:
/// - `movement_speed`: Zombie and Piglin.setBaby (the load calls it even for an
///   adult), `Enderman.setTarget` (the load ends in `readPersistentAngerSaveData`,
///   which calls it), and the first tick of any mob (`LivingEntity.removeFrost`).
/// - `max_health`: `Wolf.applyTamingSideEffects`, through `setTame(false, true)`
///   when the load finds no owner (a wolf that is tamed in game makes it too).
/// - `attack_damage`: `Rabbit.setVariant`, and a baby Goat or Hoglin, whose load's
///   `setAge` crosses the baby boundary (`ageBoundaryReached`).
pub(crate) fn made_at_load(m: &Mob) -> Vec<Attr> {
    let s = m.species();
    let mut made = Vec::new();
    if super::doors::is_zombie(s) || matches!(s, Species::Piglin | Species::Enderman) || m.age > 0 {
        made.push(Attr::MovementSpeed);
    }
    if s == Species::Wolf {
        made.push(Attr::MaxHealth);
    }
    if s == Species::Rabbit || (matches!(s, Species::Goat | Species::Hoglin) && m.is_baby()) {
        made.push(Attr::AttackDamage);
    }
    made
}
/// The base of an instance `made_at_load` lists, as `AttributeMap.pack` writes it.
pub(crate) fn made_base(m: &Mob, a: Attr) -> f64 {
    match (m.species(), a) {
        // `Wolf.applyTamingSideEffects` sets 40 for a tame wolf and 8 for the others;
        // `attrs.max_health` is the value (modifiers included), never the base.
        (Species::Wolf, Attr::MaxHealth) => {
            base_value_with(m, a, || Some(super::tame::wolf_health_base(m.extra.tame)))
        }
        (s @ (Species::Goat | Species::Hoglin), Attr::AttackDamage) => {
            base_value_with(m, a, || age_attack_damage(s, m.is_baby()))
        }
        _ => base_value(m, a),
    }
}
/// `Goat` and `Hoglin.ageBoundaryReached`: the attack damage of a baby or an adult.
fn age_attack_damage(s: Species, baby: bool) -> Option<f64> {
    match s {
        Species::Goat => Some(if baby { 1.0 } else { 2.0 }),
        Species::Hoglin => Some(if baby { 0.5 } else { 6.0 }),
        _ => None,
    }
}
/// The transient modifiers of the mob's state (`SPEED_MODIFIER_BABY`).
pub fn transient(m: &Mob, a: Attr) -> Option<(f64, Op)> {
    if a != Attr::MovementSpeed
        || !m.is_baby()
        || m.extra
            .attributes
            .as_ref()
            .is_some_and(|s| s.baby_speed_removed)
    {
        return None;
    }
    match m.species() {
        Species::Zombie
        | Species::Husk
        | Species::Drowned
        | Species::ZombieVillager
        | Species::ZombifiedPiglin => Some((0.5, Op::AddMultipliedBase)),
        Species::Piglin => Some((f64::from(0.2f32), Op::AddMultipliedBase)),
        _ => None,
    }
}
/// The attribute modifier one status effect grants (`MobEffect.addAttributeModifier`),
/// scaled by the amplifier plus one (`MobEffect.AttributeTemplate.create`);
/// `None` when the effect does not touch `a`. Vanilla's amounts are `float`
/// literals widened to `double`, so `f64::from` the `f32` (Jump Boost's,
/// Absorption's and Invisibility's are `double` literals).
fn effect_modifier(effect: &str, amplifier: i32, a: Attr) -> Option<(f64, Op)> {
    let level = f64::from(amplifier + 1);
    match (effect.trim_start_matches("minecraft:"), a) {
        ("strength", Attr::AttackDamage) => Some((3.0 * level, Op::AddValue)),
        ("weakness", Attr::AttackDamage) => Some((-4.0 * level, Op::AddValue)),
        ("speed", Attr::MovementSpeed) => Some((f64::from(0.2f32) * level, Op::AddMultipliedTotal)),
        ("slowness", Attr::MovementSpeed) => {
            Some((f64::from(-0.15f32) * level, Op::AddMultipliedTotal))
        }
        ("health_boost", Attr::MaxHealth) => Some((4.0 * level, Op::AddValue)),
        ("jump_boost", Attr::SafeFallDistance) => Some((level, Op::AddValue)),
        ("absorption", Attr::MaxAbsorption) => Some((4.0 * level, Op::AddValue)),
        ("invisibility", Attr::WaypointTransmitRange) => Some((-level, Op::AddMultipliedTotal)),
        _ => None,
    }
}
/// The id of the modifier an effect grants: `minecraft:effect.<name>`, but
/// Invisibility's is `minecraft:effect.waypoint_transmit_range_hide`
/// (`MobEffects`).
fn effect_modifier_id(name: &str) -> String {
    match name {
        "invisibility" => "minecraft:effect.waypoint_transmit_range_hide".to_owned(),
        _ => format!("minecraft:effect.{name}"),
    }
}
/// Where a mob keeps the attribute `name`: `Some(Some(a))` for one of its
/// own store's that its type has, `Some(None)` for one outside the store,
/// which its attribute maps hold, and `None` for one of the store's that its
/// type does not have.
fn store_attr(m: &Mob, name: &str) -> Option<Option<Attr>> {
    match command_attr(name.strip_prefix("minecraft:").unwrap_or(name)) {
        Some(a) => default_value(m, a).map(|_| Some(a)),
        None => Some(None),
    }
}
/// Whether an effect's modifier on `name` goes to the mob's attribute maps,
/// where `/attribute` writes it: an attribute outside its own store, or one
/// of a sulfur cube's body archetype attributes, which its attribute map
/// holds beside the archetype's own modifiers and its save writes from there
/// (knockback resistance is in the store too, for the cube's bodiless
/// physics).
fn in_attribute_maps(m: &Mob, name: &str) -> bool {
    match store_attr(m, name) {
        Some(None) => true,
        Some(Some(_)) => {
            m.species() == Species::SulfurCube
                && super::sulfur_cube::ARCHETYPE_ATTRIBUTES.contains(&name)
        }
        None => false,
    }
}
/// The stored modifiers of the mob's own attributes that `effect` puts on it,
/// as (attribute, id): vanilla's, then those its declaration lists.
fn own_effect_modifier_keys(
    m: &Mob,
    effect: &str,
    def: Option<&sunrise_game_data::potions::EffectDef>,
) -> impl Iterator<Item = (Attr, Cow<'static, str>)> {
    let name = effect.trim_start_matches("minecraft:");
    let vanilla: Vec<_> = Attr::ALL
        .into_iter()
        .filter(|&a| effect_modifier(name, 0, a).is_some() && default_value(m, a).is_some())
        .map(|a| (a, Cow::Owned(effect_modifier_id(name))))
        .collect();
    let declared: Vec<_> = def
        .map_or(&[][..], |d| d.modifiers)
        .iter()
        .filter_map(|&(attribute, id, ..)| Some((store_attr(m, attribute)??, Cow::Borrowed(id))))
        .collect();
    vanilla.into_iter().chain(declared)
}
/// The mob's own record in the `attributes` list's form, the seed of its
/// live map.
fn live_seed(m: &Mob) -> Vec<sunrise_core::nbt::Tag> {
    crate::entity_nbt::attribute_entries(m)
        .into_iter()
        .map(|(_, entry)| entry)
        .collect()
}
type MapModifier = sunrise_world::attributes::Modifier;
/// A modifier as an attribute instance holds it: the row `getModifiers`
/// lists, and the row `pack` saves when it is permanent
/// (`addOrUpdateTransientModifier` over a permanent id leaves that one as
/// it was).
type InstanceRow = (MapModifier, Option<MapModifier>);
fn instance_rows(value: &Instance) -> Vec<InstanceRow> {
    value
        .modifiers()
        .iter()
        .map(|row| {
            let saved = value
                .permanent_modifiers()
                .iter()
                .find(|p| p.id() == row.id());
            (row.clone(), saved.cloned())
        })
        .collect()
}
fn same_modifier(a: &MapModifier, b: &MapModifier) -> bool {
    a.id() == b.id()
        && a.amount().to_bits() == b.amount().to_bits()
        && a.operation() == b.operation()
}
/// The rows of the attribute `name` in the mob's live map, which saves and
/// syncs it; none for an attribute it has no instance of.
fn live_rows(m: &Mob, name: &str) -> Vec<InstanceRow> {
    let Some(catalog) = m.kind.attribute_bounds.catalog() else {
        return Vec::new();
    };
    let Some(live) = m
        .extra
        .live
        .built_or_build(catalog, &m.kind.id, || live_seed(m))
    else {
        return Vec::new();
    };
    live.key(name)
        .and_then(|key| live.lookup(Some(&key)))
        .and_then(|handle| live.instance(&handle).ok())
        .map_or_else(Vec::new, instance_rows)
}
/// The rows of the attribute `name` in the mob's attribute map
/// (`generic_attributes`); before the map is made, the permanent ones of its
/// live map, which [`attribute_map`] makes it with.
fn map_rows(m: &Mob, name: &str) -> Vec<InstanceRow> {
    match m.extra.generic_attributes.as_deref() {
        Some(map) => map
            .catalog()
            .id(name)
            .and_then(|key| map.instance(key))
            .map_or_else(Vec::new, instance_rows),
        None => live_rows(m, name)
            .into_iter()
            .filter_map(|(_, saved)| saved.map(|saved| (saved.clone(), Some(saved))))
            .collect(),
    }
}
/// Puts the rows `owned` names back as `before` held them, permanent or
/// transient, each at its place among the others; the rows of later
/// independent writes stay as they are now. True when anything changed.
fn restore_instance_rows<O: Observer>(
    value: &mut Instance,
    observer: &mut O,
    before: Vec<InstanceRow>,
    owned: impl Fn(&str) -> bool,
) -> Result<bool, AttributeError<O::Error>> {
    let mut rows = instance_rows(value);
    let same_row = |(a, saved_a): &InstanceRow, (b, saved_b): &InstanceRow| {
        same_modifier(a, b)
            && match (saved_a, saved_b) {
                (Some(x), Some(y)) => same_modifier(x, y),
                (x, y) => x.is_none() && y.is_none(),
            }
    };
    if !crate::living_attributes::restore_modifier_order(
        &mut rows,
        before,
        |(row, _)| owned(row.id()),
        |(a, _), (b, _)| a.id() == b.id(),
        same_row,
    ) {
        return Ok(false);
    }
    value.remove_all(observer)?;
    for (row, saved) in rows {
        match saved {
            Some(saved) => {
                let updated = !same_modifier(&saved, &row);
                value.add_permanent(saved, observer)?;
                if updated {
                    value.add_or_update_transient(row, observer)?;
                }
            }
            None => value.add_transient(row, observer)?,
        }
    }
    Ok(true)
}
/// The (attribute, id) of each modifier a content mod's effect `def` puts on
/// the attribute maps of `m` ([`in_attribute_maps`]).
fn generic_effect_modifier_keys<'a>(
    m: &'a Mob,
    def: &'static sunrise_game_data::potions::EffectDef,
) -> impl Iterator<Item = (&'static str, &'static str)> + 'a {
    def.modifiers
        .iter()
        .filter(|&&(attribute, ..)| in_attribute_maps(m, attribute))
        .map(|&(attribute, id, ..)| (attribute, id))
}
/// `AttributeInstance.removeModifier(id)`, then with `modifier`
/// `addPermanentModifier`, on an attribute instance.
fn replace_modifier<O: Observer>(
    value: &mut Instance,
    observer: &mut O,
    id: &str,
    modifier: Option<&MapModifier>,
) -> Result<(), AttributeError<O::Error>> {
    value.remove(id, observer)?;
    if let Some(modifier) = modifier {
        value.add_permanent(modifier.clone(), observer)?;
    }
    Ok(())
}
/// The mob's attribute map (`generic_attributes`), which `/attribute` and
/// the effects' modifiers outside its own store write. Made on first use
/// from its type's defaults, then given what its live map holds of each
/// attribute, the base and permanent modifiers a load or an earlier write
/// kept there, so a reloaded mob's map reports and removes a saved
/// modifier. None for a type `catalog` has no row for.
pub fn attribute_map<'a>(
    m: &'a mut Mob,
    catalog: &Arc<sunrise_world::attributes::entity::Catalog>,
) -> Option<&'a mut sunrise_world::attributes::entity::EntityAttributes> {
    if m.extra.generic_attributes.is_none() {
        let mut map = catalog.create(&m.kind.id)?;
        if let Some(live) = m
            .extra
            .live
            .built_or_build(catalog, &m.kind.id, || live_seed(m))
        {
            for (_, key, instance) in live.instances() {
                let Some(id) = catalog.id(key.name()) else {
                    continue;
                };
                let base = instance.base_value();
                let _ = map.edit(id, |value, observer| {
                    value.set_base_value(base, observer)?;
                    for modifier in instance.permanent_modifiers() {
                        value.add_or_replace_permanent(modifier.clone(), observer)?;
                    }
                    Ok::<_, AttributeError<_>>(())
                });
            }
        }
        m.extra.generic_attributes = Some(Box::new(map));
    }
    m.extra.generic_attributes.as_deref_mut()
}
/// `LivingEntity.onAttributeUpdated(SCALE)`, `refreshDimensions`: the mob's
/// boxes and eye height take the scale its live map computes, the value the
/// attribute sync sends, which a remote client puts in the same field
/// (`extra.model_scale`). Nothing for a mob without a scale instance.
pub fn scale_changed(m: &mut Mob) {
    let Some(catalog) = m.kind.attribute_bounds.catalog() else {
        return;
    };
    let scale = m.extra.live.read(
        catalog,
        &m.kind.id,
        "minecraft:scale",
        sunrise_world::attribute_map::ValueKind::Computed,
        || live_seed(m),
    );
    if let Some(scale) = scale {
        m.extra.model_scale = scale as f32;
    }
}
/// One effect modifier on an attribute of the mob's attribute maps
/// ([`in_attribute_maps`]), where the mob's other attribute writes go: its
/// attribute map (`generic_attributes`, the one `/attribute` writes, armor
/// included) and its live map, which saves and syncs it. The id goes first,
/// then with `modifier` the modifier is added for good. Nothing for an
/// attribute the mob's type does not have, or a type built without a
/// catalog.
fn set_generic_effect_modifier(
    m: &mut Mob,
    name: &str,
    id: &str,
    modifier: Option<(f64, Operation)>,
) -> DirtyAttributes {
    let Some(catalog) = m.kind.attribute_bounds.catalog().cloned() else {
        return DirtyAttributes::default();
    };
    let Some(key) = catalog.id(name) else {
        return DirtyAttributes::default();
    };
    if catalog.default_base(&m.kind.id, key).is_none() {
        return DirtyAttributes::default();
    }
    let modifier =
        modifier.and_then(|(amount, operation)| MapModifier::new(id, amount, operation).ok());
    if let Some(map) = attribute_map(m, &catalog) {
        let _ = map.edit(key, |value, observer| {
            replace_modifier(value, observer, id, modifier.as_ref())
        });
    }
    let mut live = std::mem::take(&mut m.extra.live);
    live.keep_transient(name, id, None);
    let _ = live.edit(
        &catalog,
        &m.kind.id,
        name,
        true,
        || live_seed(m),
        |value, observer| replace_modifier(value, observer, id, modifier.as_ref()),
    );
    m.extra.live = live;
    DirtyAttributes {
        scale: name == "minecraft:scale",
        ..DirtyAttributes::default()
    }
}
/// `removeModifier(SPEED_MODIFIER_BABY_ID)` on `a` when `id` names the baby
/// speed bonus, which no store row holds: the store remembers it is gone.
/// True when it was on.
pub fn remove_baby_modifier(m: &mut Mob, a: Attr, id: &str) -> bool {
    if id != "minecraft:baby" || transient(m, a).is_none() {
        return false;
    }
    m.extra
        .attributes
        .get_or_insert_with(Box::default)
        .baby_speed_removed = true;
    true
}
/// The attributes effect modifiers dirtied whose
/// `LivingEntity.onAttributeUpdated` does more than read the value again:
/// the maximum health and absorption clamps, and the dimensions a scale
/// change refreshes. `refreshDirtyAttributes` runs them once after a whole
/// write ([`Mob::refresh_dirty_attributes`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct DirtyAttributes {
    max_health: bool,
    max_absorption: bool,
    scale: bool,
}
impl DirtyAttributes {
    pub fn merge(&mut self, other: Self) {
        self.max_health |= other.max_health;
        self.max_absorption |= other.max_absorption;
        self.scale |= other.scale;
    }
}
/// What a mob's attribute maps held of one attribute that declared effect
/// modifiers write ([`in_attribute_maps`]): the rows of its attribute map
/// and of its live map, of which the effects own `ids`.
struct MapUndo {
    attribute: &'static str,
    ids: Vec<&'static str>,
    map: Vec<InstanceRow>,
    live: Vec<InstanceRow>,
}
impl MapUndo {
    /// Puts the owned rows back in both maps, the live map's saved entry
    /// included, so a rebuilt map and the save have them as they were.
    fn restore(self, m: &mut Mob) {
        let Self {
            attribute,
            ids,
            map,
            live,
        } = self;
        let owned = |id: &str| ids.contains(&id);
        let Some(catalog) = m.kind.attribute_bounds.catalog().cloned() else {
            return;
        };
        let Some(key) = catalog.id(attribute) else {
            return;
        };
        if ids.is_empty() || catalog.default_base(&m.kind.id, key).is_none() {
            return;
        }
        let mut attributes = std::mem::take(&mut m.extra.live);
        let _ = attributes.edit(
            &catalog,
            &m.kind.id,
            attribute,
            true,
            || live_seed(m),
            |value, observer| restore_instance_rows(value, observer, live, owned),
        );
        m.extra.live = attributes;
        if let Some(attributes) = attribute_map(m, &catalog) {
            let _ = attributes.edit(key, |value, observer| {
                restore_instance_rows(value, observer, map, owned)
            });
        }
        if attribute == "minecraft:scale" {
            scale_changed(m);
        }
    }
}
/// One effect mutation's before-order in the legacy and live modifier seed stores.
/// A clear is one batch: restore all of its rows together, not at stale indices.
/// The declared modifiers in the mob's attribute maps keep their rows there
/// as they were, permanent or transient and in their order, and the baby
/// speed bonus whether it was taken away.
pub struct EffectModifierUndo {
    keys: Vec<(Attr, String)>,
    permanent: Vec<Modifier>,
    transient: Vec<Modifier>,
    live: crate::living_attributes::TransientModifiers,
    maps: Vec<MapUndo>,
    baby_speed_removed: Option<bool>,
}

impl EffectModifierUndo {
    pub fn keep_explicit_write(&mut self, attribute: &str, id: &str) {
        self.keys
            .retain(|(attr, key)| attr.id() != attribute || key != id);
        let unaffected = |row: &Modifier| row.attr.id() != attribute || row.id != id;
        self.permanent.retain(unaffected);
        self.transient.retain(unaffected);
        self.live.forget(attribute, id);
        for undo in self
            .maps
            .iter_mut()
            .filter(|undo| undo.attribute == attribute)
        {
            undo.ids.retain(|owned| *owned != id);
        }
    }

    pub fn restore(self, mob: &mut Mob) {
        let Self {
            keys,
            permanent,
            transient,
            live,
            maps,
            baby_speed_removed,
        } = self;
        // The mods' transient modifiers the effects replaced come back to the
        // live map's list first, so a map built for the restore has them.
        mob.extra
            .live
            .restore_transient_modifiers(live.clone(), |attribute, id| {
                maps.iter()
                    .any(|undo| undo.attribute == attribute && undo.ids.contains(&id))
            });
        for undo in maps {
            undo.restore(mob);
        }
        if keys.is_empty() {
            return;
        }
        let owns = |row: &Modifier| {
            keys.iter()
                .any(|(attr, id)| *attr == row.attr && *id == row.id)
        };
        let same_key = |a: &Modifier, b: &Modifier| a.attr == b.attr && a.id == b.id;
        let mut changed = false;
        if let Some(attrs) = mob.extra.attributes.as_mut() {
            use crate::living_attributes::restore_modifier_order;
            changed |= restore_modifier_order(
                &mut attrs.modifiers,
                permanent,
                owns,
                same_key,
                Modifier::eq,
            );
            changed |= restore_modifier_order(
                &mut attrs.transient,
                transient,
                owns,
                same_key,
                Modifier::eq,
            );
        }
        // An effect that named the baby speed bonus took it away.
        let baby = keys
            .iter()
            .any(|(attr, id)| *attr == Attr::MovementSpeed && id == "minecraft:baby");
        if let Some(removed) = baby_speed_removed.filter(|_| baby) {
            let now = mob
                .extra
                .attributes
                .as_ref()
                .is_some_and(|s| s.baby_speed_removed);
            if now != removed {
                mob.extra
                    .attributes
                    .get_or_insert_with(Box::default)
                    .baby_speed_removed = removed;
                changed = true;
            }
        }
        changed |= mob
            .extra
            .live
            .restore_transient_modifiers(live, |attribute, id| {
                keys.iter()
                    .any(|(attr, key)| attr.id() == attribute && key == id)
            });
        if changed {
            mob.extra.live.invalidate();
            for attr in Attr::ALL {
                if keys.iter().any(|(key, _)| *key == attr) {
                    refresh(mob, attr);
                }
            }
        }
    }
}

impl Mob {
    /// The modifier rows one effect write (or one bulk clear of the given effects) can replace.
    pub fn effect_modifier_undo(
        &self,
        effects: impl Iterator<Item = &'static str>,
    ) -> Option<EffectModifierUndo> {
        let effects: Vec<_> = effects
            .map(|effect| (effect, sunrise_game_data::potions::effect(effect)))
            .collect();
        let keys: Vec<_> = effects
            .iter()
            .flat_map(|&(effect, def)| own_effect_modifier_keys(self, effect, def))
            .map(|(attr, id)| (attr, id.into_owned()))
            .collect();
        let mut maps: Vec<MapUndo> = Vec::new();
        for (attribute, id) in effects
            .iter()
            .filter_map(|&(_, def)| def)
            .flat_map(|def| generic_effect_modifier_keys(self, def))
        {
            match maps.iter_mut().find(|undo| undo.attribute == attribute) {
                Some(undo) if undo.ids.contains(&id) => {}
                Some(undo) => undo.ids.push(id),
                None => maps.push(MapUndo {
                    attribute,
                    ids: vec![id],
                    map: map_rows(self, attribute),
                    live: live_rows(self, attribute),
                }),
            }
        }
        if keys.is_empty() && maps.is_empty() {
            return None;
        }
        let attrs = self.extra.attributes.as_deref();
        let baby = keys
            .iter()
            .any(|(attr, id)| *attr == Attr::MovementSpeed && id == "minecraft:baby");
        Some(EffectModifierUndo {
            keys,
            permanent: attrs
                .map(|attrs| attrs.modifiers.clone())
                .unwrap_or_default(),
            transient: attrs
                .map(|attrs| attrs.transient.clone())
                .unwrap_or_default(),
            live: self.extra.live.transient_modifiers(),
            maps,
            baby_speed_removed: baby.then(|| attrs.is_some_and(|s| s.baby_speed_removed)),
        })
    }

    /// The one place a mob's effect attribute modifiers change (#478). They
    /// are permanent modifiers in the attribute store, so they are saved and
    /// synced as any other, and a read derives nothing from the effect list.
    ///
    /// `Some(amplifier)` is vanilla's `LivingEntity.onEffectAdded` (a new
    /// effect) and `onEffectUpdated(instance, true, _)` (an `addEffect` whose
    /// `MobEffectInstance.update` changed the active instance, or a hidden
    /// instance taking over in `tickServer`): `MobEffect.addAttributeModifiers`
    /// removes each of the effect's modifiers (`minecraft:effect.<name>`, see
    /// `effect_modifier_id`, or the ids `def`, a content mod's effect,
    /// declares), then adds it again with `amount * (amplifier + 1)`. `None`
    /// is `onEffectsRemoved` (`removeEffect`, `removeAllEffects`, the effect
    /// running out): `removeAttributeModifiers` removes them. Both skip an
    /// attribute the mob has no instance of (`AttributeMap.getInstance` is
    /// null: a pig has no attack damage, so Strength adds nothing to it). A
    /// declared modifier on an attribute outside the mob's own store goes to
    /// its attribute maps (`set_generic_effect_modifier`).
    ///
    /// Loading `active_effects` calls none of these (`readAdditionalSaveData`
    /// puts the instances straight into the map), so a loaded effect has the
    /// modifier only if the saved `attributes` kept it.
    ///
    /// Then `refreshDirtyAttributes` ([`Self::refresh_dirty_attributes`]).
    pub fn set_effect_modifiers(
        &mut self,
        effect: &str,
        def: Option<&sunrise_game_data::potions::EffectDef>,
        amplifier: Option<i32>,
    ) {
        let dirty = self.write_effect_modifiers(effect, def, amplifier);
        self.refresh_dirty_attributes(dirty);
    }

    /// [`Self::set_effect_modifiers`] without `refreshDirtyAttributes`: what
    /// the writes dirtied, for the removal of several effects at once
    /// (`onEffectsRemoved` of `removeAllEffects`) to refresh once, after all
    /// of them.
    pub fn write_effect_modifiers(
        &mut self,
        effect: &str,
        def: Option<&sunrise_game_data::potions::EffectDef>,
        amplifier: Option<i32>,
    ) -> DirtyAttributes {
        let name = effect.trim_start_matches("minecraft:");
        let level = amplifier.map(|amplifier| f64::from(amplifier.wrapping_add(1)));
        let mut dirty = DirtyAttributes::default();
        for a in Attr::ALL {
            let Some((amount, op)) = effect_modifier(name, 0, a) else {
                continue;
            };
            if default_value(self, a).is_some() {
                let modifier = level.map(|level| (amount * level, op));
                dirty.merge(self.set_own_effect_modifier(a, &effect_modifier_id(name), modifier));
            }
        }
        for &(attribute, id, amount, operation) in def.map_or(&[][..], |d| d.modifiers) {
            let modifier = level.map(|level| (amount * level, operation));
            if in_attribute_maps(self, attribute) {
                dirty.merge(set_generic_effect_modifier(self, attribute, id, modifier));
            }
            if let Some(Some(a)) = store_attr(self, attribute) {
                let modifier = modifier.map(|(amount, operation)| (amount, operation.into()));
                dirty.merge(self.set_own_effect_modifier(a, id, modifier));
            }
        }
        dirty
    }

    /// `LivingEntity.refreshDirtyAttributes` after effect modifiers changed:
    /// a max health change runs `health_boost_changed`, which lowers a
    /// health above the new maximum as `onAttributeUpdated(MAX_HEALTH)` does
    /// and never heals; a max absorption change lowers an absorption above
    /// the new maximum (`onAttributeUpdated(MAX_ABSORPTION)`); a scale change
    /// refreshes the dimensions ([`scale_changed`]).
    pub fn refresh_dirty_attributes(&mut self, dirty: DirtyAttributes) {
        if dirty.max_health {
            health_boost_changed(self);
        }
        if dirty.max_absorption {
            self.absorption = self.absorption.min(value(self, Attr::MaxAbsorption) as f32);
        }
        if dirty.scale {
            scale_changed(self);
        }
    }

    /// One effect modifier on an attribute of the mob's own store:
    /// `removeModifier` takes the id off whatever holds it (the store's
    /// permanent and transient lists, the transient one the live map keeps
    /// for its next build, a mod's added before the effect, and the baby
    /// speed bonus), then with `modifier` it is stored again.
    fn set_own_effect_modifier(
        &mut self,
        a: Attr,
        id: &str,
        modifier: Option<(f64, Op)>,
    ) -> DirtyAttributes {
        let held = remove_baby_modifier(self, a, id)
            || has_modifier(self, a, id)
            || has_transient_modifier(self, a, id);
        if let Some(s) = self.extra.attributes.as_mut() {
            s.modifiers.retain(|x| x.attr != a || x.id != id);
            s.transient.retain(|x| x.attr != a || x.id != id);
        }
        self.extra.live.keep_transient(a.id(), id, None);
        if let Some((amount, op)) = modifier {
            let s = self.extra.attributes.get_or_insert_with(Box::default);
            s.modifiers.push(Modifier {
                attr: a,
                id: id.to_owned().into(),
                amount,
                op,
            });
        }
        self.extra.live.invalidate();
        match a {
            Attr::MaxHealth => DirtyAttributes {
                max_health: true,
                ..DirtyAttributes::default()
            },
            // `removeModifier` dirties the instance only when the id was on
            // it: a loaded Absorption, which added none, clamps nothing.
            Attr::MaxAbsorption => DirtyAttributes {
                max_absorption: held || modifier.is_some(),
                ..DirtyAttributes::default()
            },
            _ => {
                refresh(self, a);
                DirtyAttributes::default()
            }
        }
    }
}
/// `AttributeInstance.getValue`: the base, then the modifiers by operation.
pub fn value(m: &Mob, a: Attr) -> f64 {
    compute(m, a, true, std::iter::empty())
}
/// `value` with modifiers no store holds: the ones worn equipment grants, or
/// those of the mob's own state (a witch's drinking, a strider's suffocation).
pub(crate) fn value_with(m: &Mob, a: Attr, extra: impl Iterator<Item = (f64, Op)> + Clone) -> f64 {
    compute(m, a, true, extra)
}
/// `AttributeInstance.calculateValue`: the base plus every `ADD_VALUE`, then
/// each `ADD_MULTIPLIED_BASE` on that sum, then each `ADD_MULTIPLIED_TOTAL`
/// multiplying, then `sanitizeValue`. The modifiers are read in place, three
/// passes over the stored, the mods' transient, the baby's and `extra`, with
/// nothing allocated.
fn compute(
    m: &Mob,
    a: Attr,
    with_transient: bool,
    extra: impl Iterator<Item = (f64, Op)> + Clone,
) -> f64 {
    let held = m.extra.attributes.as_deref();
    let baby = if with_transient {
        transient(m, a)
    } else {
        None
    };
    let ops = || {
        held.into_iter()
            .flat_map(|s| s.modifiers.iter().chain(&s.transient))
            .filter(move |x| x.attr == a)
            .map(|x| (x.amount, x.op))
            .chain(baby)
            .chain(extra.clone())
    };
    let mut base = base_value(m, a);
    for (amount, op) in ops() {
        if op == Op::AddValue {
            base += amount;
        }
    }
    let mut result = base;
    for (amount, op) in ops() {
        if op == Op::AddMultipliedBase {
            result += base * amount;
        }
    }
    for (amount, op) in ops() {
        if op == Op::AddMultipliedTotal {
            result *= 1.0 + amount;
        }
    }
    m.kind.attribute_bounds.sanitize(a, result)
}
/// Copies the attribute values into the fields the game reads.
fn refresh(m: &mut Mob, a: Attr) {
    match a {
        Attr::MaxHealth => m.attrs.max_health = value(m, a) as f32,
        Attr::MovementSpeed => m.attrs.speed = base_with_modifiers(m, a),
        Attr::FollowRange => {
            m.attrs.follow_range = value(m, a);
            m.update_max_visited_nodes();
        }
        Attr::AttackDamage => m.attrs.attack_damage = value(m, a) as f32,
        Attr::KnockbackResistance => m.attrs.knockback_resistance = value(m, a),
        Attr::SpawnReinforcements => m.attrs.spawn_reinforcements = value(m, a),
        Attr::JumpStrength => m.attrs.jump_strength = value(m, a),
        Attr::Armor
        | Attr::BurningTime
        | Attr::FlyingSpeed
        | Attr::SafeFallDistance
        | Attr::MaxAbsorption
        | Attr::WaypointTransmitRange => {}
    }
}
/// The value without the transient baby modifier, which `Mob::movement_speed`
/// adds in its own `calculateValue` with the mob's other state modifiers.
fn base_with_modifiers(m: &Mob, a: Attr) -> f64 {
    compute(m, a, false, std::iter::empty())
}
/// `AttributeInstance.setBaseValue`.
pub fn set_base(m: &mut Mob, a: Attr, v: f64) {
    let s = m.extra.attributes.get_or_insert_with(Box::default);
    match s.bases.iter_mut().find(|(x, _)| *x == a) {
        Some(b) => b.1 = v,
        None => s.bases.push((a, v)),
    }
    m.extra.live.invalidate();
    refresh(m, a);
}
pub fn has_modifier(m: &Mob, a: Attr, id: &str) -> bool {
    m.extra
        .attributes
        .as_ref()
        .is_some_and(|s| s.modifiers.iter().any(|x| x.attr == a && x.id == id))
}
/// The amount of a permanent modifier, if the mob has it.
pub fn modifier_amount(m: &Mob, a: Attr, id: &str) -> Option<f64> {
    m.extra
        .attributes
        .as_ref()
        .and_then(|s| s.modifiers.iter().find(|x| x.attr == a && x.id == id))
        .map(|x| x.amount)
}
/// `addOrReplacePermanentModifier`.
pub fn add_modifier(
    m: &mut Mob,
    a: Attr,
    id: impl Into<std::borrow::Cow<'static, str>>,
    amount: f64,
    op: Op,
) {
    let id = id.into();
    let s = m.extra.attributes.get_or_insert_with(Box::default);
    let new = Modifier {
        attr: a,
        id: id.clone(),
        amount,
        op,
    };
    match s.modifiers.iter_mut().find(|x| x.attr == a && x.id == id) {
        Some(x) => *x = new,
        None => s.modifiers.push(new),
    }
    m.extra.live.invalidate();
    refresh(m, a);
}
/// Whether a mod's transient modifier with id `id` is on `a`.
pub fn has_transient_modifier(m: &Mob, a: Attr, id: &str) -> bool {
    m.extra
        .attributes
        .as_ref()
        .is_some_and(|s| s.transient.iter().any(|x| x.attr == a && x.id == id))
}
/// The amount of a mod's transient modifier, if the mob has it.
pub fn transient_modifier_amount(m: &Mob, a: Attr, id: &str) -> Option<f64> {
    m.extra
        .attributes
        .as_ref()
        .and_then(|s| s.transient.iter().find(|x| x.attr == a && x.id == id))
        .map(|x| x.amount)
}
/// `addOrUpdateTransientModifier` for a mod: the modifier counts in the value
/// and is synced, but is not saved.
pub fn add_transient_modifier(m: &mut Mob, a: Attr, id: &str, amount: f64, op: Op) {
    let s = m.extra.attributes.get_or_insert_with(Box::default);
    s.transient.retain(|x| x.attr != a || x.id != id);
    s.transient.push(Modifier {
        attr: a,
        id: id.to_owned().into(),
        amount,
        op,
    });
    // The live map is built from the saved record, so it keeps the modifier
    // itself for the next build.
    let modifier = sunrise_world::attributes::Modifier::new(id, amount, op.into()).ok();
    m.extra.live.keep_transient(a.id(), id, modifier);
    m.extra.live.invalidate();
    refresh(m, a);
}
/// `AttributeInstance.removeModifier`: the permanent modifier or a mod's
/// transient one with that id.
pub fn remove_modifier(m: &mut Mob, a: Attr, id: &str) {
    if let Some(s) = m.extra.attributes.as_mut() {
        s.modifiers.retain(|x| x.attr != a || x.id != id);
        s.transient.retain(|x| x.attr != a || x.id != id);
    }
    m.extra.live.keep_transient(a.id(), id, None);
    m.extra.live.invalidate();
    refresh(m, a);
}
/// `Mob.finalizeSpawn`, which every mob's own `finalizeSpawn` reaches
/// through `super`: the `random_spawn_bonus` follow range modifier (once)
/// and `setLeftHanded(nextFloat() < 0.05F)`.
pub(crate) fn mob_spawn(m: &mut Mob, random: &mut impl RandomSource) {
    if !has_modifier(m, Attr::FollowRange, "minecraft:random_spawn_bonus") {
        let bonus = random.triangle(0.0, 0.114_850_000_000_000_01);
        add_modifier(
            m,
            Attr::FollowRange,
            "minecraft:random_spawn_bonus",
            bonus,
            Op::AddMultipliedBase,
        );
    }
    m.extra.left_handed = random.next_float() < 0.05;
}
/// `Zombie.handleAttributes`, from the zombie's own random.
pub fn zombie_attributes(m: &mut Mob, special: f32, reason: Reason) {
    let r = &mut m.random;
    let reinforcements = if m.kind.species == Species::ZombifiedPiglin {
        0.0
    } else {
        r.next_double() * f64::from(0.1f32)
    };
    let knockback = r.next_double() * f64::from(0.05f32);
    let follow = r.next_double() * 1.5 * f64::from(special);
    let leader = r.next_float() < special * 0.05;
    let leader_rolls = leader.then(|| (r.next_double() * 0.25 + 0.5, r.next_double() * 3.0 + 1.0));
    set_base(m, Attr::SpawnReinforcements, reinforcements);
    add_modifier(
        m,
        Attr::KnockbackResistance,
        "minecraft:random_spawn_bonus",
        knockback,
        Op::AddValue,
    );
    if follow > 1.0 {
        add_modifier(
            m,
            Attr::FollowRange,
            "minecraft:zombie_random_spawn_bonus",
            follow,
            Op::AddMultipliedTotal,
        );
    }
    if let Some((reinforce, health)) = leader_rolls {
        add_modifier(
            m,
            Attr::SpawnReinforcements,
            "minecraft:leader_zombie_bonus",
            reinforce,
            Op::AddValue,
        );
        add_modifier(
            m,
            Attr::MaxHealth,
            "minecraft:leader_zombie_bonus",
            health,
            Op::AddMultipliedTotal,
        );
        if !matches!(
            reason,
            Reason::Conversion | Reason::Load | Reason::DimensionTravel
        ) {
            m.health = max_health(m);
        }
        super::doors::set_can_break_doors(m, true);
    }
}
pub fn max_health(m: &Mob) -> f32 {
    value(m, Attr::MaxHealth) as f32
}
/// Whether the mob stores a base for `a`: `set_base` or a load made one.
pub(crate) fn has_base(m: &Mob, a: Attr) -> bool {
    m.extra
        .attributes
        .as_ref()
        .is_some_and(|s| s.bases.iter().any(|(x, _)| *x == a))
}
/// `LivingEntity.onAttributeUpdated(MAX_HEALTH)` after a Health Boost
/// change: the cached maximum follows the attribute model again, and Health
/// above it is lowered (nothing is healed). A wolf with no stored max health
/// base keeps the cached maximum it was given outside the model; taming
/// stores the base (`tame::set_tame`), so a tame wolf follows the model.
pub(crate) fn health_boost_changed(m: &mut Mob) {
    if m.species() == Species::Wolf && !has_base(m, Attr::MaxHealth) {
        return;
    }
    refresh(m, Attr::MaxHealth);
    if m.health > m.attrs.max_health {
        m.health = m.attrs.max_health;
    }
}

// ---- species facts --------------------------------------------------------

/// `AgeableMob` subclasses.
pub fn ageable(s: Species) -> bool {
    use Species as S;
    matches!(
        s,
        S::Pig
            | S::Cow
            | S::Mooshroom
            | S::Sheep
            | S::Chicken
            | S::Rabbit
            | S::Horse
            | S::Donkey
            | S::Mule
            | S::Llama
            | S::TraderLlama
            | S::Goat
            | S::Panda
            | S::PolarBear
            | S::Fox
            | S::Wolf
            | S::Cat
            | S::Ocelot
            | S::Parrot
            | S::Bee
            | S::Turtle
            | S::Armadillo
            | S::Frog
            | S::Axolotl
            | S::Squid
            | S::GlowSquid
            | S::Dolphin
            | S::Villager
            | S::WanderingTrader
            | S::Slime
            | S::MagmaCube
            | S::SulfurCube
            | S::Hoglin
            | S::Strider
            | S::Camel
            | S::CamelHusk
            | S::Sniffer
            | S::HappyGhast
            | S::Nautilus
            | S::ZombieNautilus
            | S::ZombieHorse
            | S::SkeletonHorse
    )
}
fn zombie(s: Species) -> bool {
    matches!(
        s,
        Species::Zombie
            | Species::Husk
            | Species::Drowned
            | Species::ZombieVillager
            | Species::ZombifiedPiglin
    )
}
fn skeleton(s: Species) -> bool {
    matches!(
        s,
        Species::Skeleton
            | Species::Stray
            | Species::Bogged
            | Species::Parched
            | Species::WitherSkeleton
    )
}
fn raider(s: Species) -> bool {
    super::raider::is_raider(s)
}
/// `AgeableMob.getBabyStartAge` (a sniffer's is -48000).
pub fn baby_start_age(s: Species) -> i32 {
    if s == Species::Sniffer {
        super::sniffer::BABY_START_AGE
    } else {
        -24000
    }
}
/// The age a non-ageable baby (zombies, piglins, zoglins) keeps.
pub const NON_AGEABLE_BABY: i32 = i32::MIN / 2;

/// `AgeableMob.setAge`, with each class's `ageBoundaryReached`. `w` is the
/// ticking world where the age crosses; the spawn and load paths pass none,
/// and no happy ghast grows up there (a hatch is the baby setup, a spawn was
/// never a ghastling), so the ghastling's brain stop needs no deferral.
pub fn set_age(m: &mut Mob, age: i32, w: Option<&MobWorld<'_>>) {
    let old = m.extra.age;
    m.extra.age = age;
    if (old < 0) != (age < 0) {
        age_boundary_reached(m, w);
    }
}
fn age_boundary_reached(m: &mut Mob, w: Option<&MobWorld<'_>>) {
    let baby = m.is_baby();
    match m.species() {
        Species::Goat | Species::Hoglin => {
            if let Some(damage) = age_attack_damage(m.species(), baby) {
                set_base(m, Attr::AttackDamage, damage);
            }
        }
        Species::SulfurCube if !baby => sulfur_size(m, 2),
        // HappyGhast.ageBoundaryReached: adultGhastSetup or babyGhastSetup,
        // then Animal's no-op. A hatched ghastling needs it before it is in
        // the level (DriedGhastBlock.spawnGhastling sets the baby age before
        // addFreshEntity), or its first tick flies the adult goals.
        Species::HappyGhast => super::happy_ghast::ensure_setup(m, w),
        _ => {}
    }
}
/// `Goat.finalizeSpawn` on a natural spawn (the spawn path in spawning.rs): the ram
/// cooldowns (`GoatAi.initMemories`), screaming 2 percent of the time and a
/// horn lost 10 percent of the time, adults only.
#[cfg(test)]
pub(crate) fn goat_spawn(m: &mut Mob, r: &mut impl RandomSource) {
    let jump = r.next_int_between_inclusive(600, 1200).unwrap_or(600);
    set_memory(m, "long_jump_cooling_down", i64::from(jump), i64::MAX);
    let ram = r.next_int_between_inclusive(600, 6000).unwrap_or(600);
    set_memory(m, "ram_cooldown_ticks", i64::from(ram), i64::MAX);
    m.extra.screaming = r.next_double() < 0.02;
    age_boundary_reached(m, None);
    if !m.is_baby() && r.next_float() < 0.1 {
        let left = r.next_boolean();
        m.extra.lost_horns[usize::from(!left)] = true;
    }
}

/// `SpawnEggItem.spawnOffspringFromSpawnEgg`: the offspring of a spawn egg
/// used on a mob of the same kind is a baby of that kind: it starts with
/// the age of a baby and takes the variant and colour it is given (the
/// parent's, but a mooshroom's colour rolled by `MushroomCow.getOffspringVariant`).
pub fn spawn_egg_offspring(child: &mut Mob, species: Species, variant: u8, color: u8) {
    set_baby(child, true);
    child.extra.variant = variant;
    if species == Species::Sheep {
        child.extra.color = color;
    }
    // `SulfurCube.getBreedOffspring`: `setSize(1, true)`.
    if species == Species::SulfurCube {
        sulfur_size(child, 1);
    }
}
/// `setBaby`: AgeableMob through its age, zombies, piglins and zoglins.
pub fn supports_baby(species: Species) -> bool {
    ageable(species) || zombie(species) || matches!(species, Species::Piglin | Species::Zoglin)
}

/// `setBaby`: AgeableMob through its age, zombies, piglins and zoglins.
pub fn set_baby(m: &mut Mob, baby: bool) {
    let s = m.species();
    if ageable(s) {
        set_age(m, if baby { baby_start_age(s) } else { 0 }, None);
        return;
    }
    match s {
        _ if zombie(s) || s == Species::Piglin => {
            m.extra.age = if baby { NON_AGEABLE_BABY } else { 0 };
            // Zombie/Piglin.setBaby: removeModifier(minecraft:baby), then the
            // transient one again for a baby.
            if let Some(a) = m.extra.attributes.as_mut() {
                a.baby_speed_removed = false;
            }
            if has_modifier(m, Attr::MovementSpeed, "minecraft:baby") {
                remove_modifier(m, Attr::MovementSpeed, "minecraft:baby");
            }
        }
        Species::Zoglin => {
            m.extra.age = if baby { NON_AGEABLE_BABY } else { 0 };
            if baby {
                set_base(m, Attr::AttackDamage, 0.5);
            }
        }
        _ => {}
    }
}

// ---- equipment ------------------------------------------------------------

/// `EquipmentSlot` names (the `drops::SLOTS` order).
pub const SLOT_NAMES: [&str; 8] = super::drops::SLOTS;
pub use sunrise_items::equipment::{CHEST, FEET, HEAD, LEGS, MAINHAND, OFFHAND};
const SADDLE: usize = sunrise_items::equipment::SADDLE;
/// The `gear` slot of an `EquipmentSlot`.
pub fn gear_slot(slot: usize) -> usize {
    match slot {
        MAINHAND => gear::MAIN,
        OFFHAND => gear::OFF,
        FEET => gear::FEET,
        LEGS => gear::LEGS,
        CHEST => gear::CHEST,
        _ => gear::HEAD,
    }
}
pub fn item(m: &Mob, slot: usize) -> Option<Arc<str>> {
    match slot {
        MAINHAND..=HEAD | SADDLE => m.extra.slot_stack(slot).map(|s| s.item.clone()),
        _ => None,
    }
}
/// A single equipment stack, with any separate gear data needed to project it.
pub struct EquipmentRead<'a> {
    base: &'a ItemStack,
    overlays: Option<(&'a gear::Gear, usize)>,
}

impl<'a> EquipmentRead<'a> {
    /// Borrow a resident stack whose components are all on the stack itself.
    pub fn resident(base: &'a ItemStack) -> Self {
        Self {
            base,
            overlays: None,
        }
    }

    /// The stored stack. Gear overlays affect components, not item, count or damage.
    pub fn base(&self) -> &'a ItemStack {
        self.base
    }

    /// The active trim, if any, without projecting the stack's component patch.
    pub fn trim(&self) -> Option<&'a gear::Trim> {
        self.overlays
            .and_then(|(gear, index)| gear.trims[index].as_ref())
            .or_else(|| self.base.data()?.trim.as_ref())
    }

    /// Vanilla's foil test uses the same enchantments the caller can read from this stack.
    pub fn has_foil(&self, item_glint: Option<bool>) -> bool {
        let enchanted = !self.base.enchantments().is_empty()
            || self
                .overlays
                .is_some_and(|(gear, index)| !gear.slots[index].is_empty());
        self.base.has_foil_with_enchantments(item_glint, enchanted)
    }

    /// The complete stack: still borrowed unless its gear data needs overlaying.
    pub fn project(&self) -> Cow<'a, ItemStack> {
        if self.overlays.is_none() {
            return Cow::Borrowed(self.base);
        }
        let mut stack = self.base.clone();
        self.overlay_gear(&mut stack, |_| true);
        Cow::Owned(stack)
    }

    /// Whether this stack matches `other` (`ItemStack.matches`), including any
    /// separate gear overlay, without cloning the stack or its components.
    pub(crate) fn matches_stack(&self, other: &ItemStack) -> bool {
        match self.overlays {
            None => self.base.matches(other),
            Some((gear, index)) => self.base.matches_with_overlay(
                other,
                &gear.slots[index],
                gear.trims[index].as_ref(),
            ),
        }
    }

    /// `LivingEntity.equipmentHasChanged` negated, for a possibly empty slot.
    pub(crate) fn matches_previous(now: Option<&Self>, previous: Option<&ItemStack>) -> bool {
        match (now, previous) {
            (Some(now), Some(previous)) => now.matches_stack(previous),
            (None, None) => true,
            _ => false,
        }
    }

    /// Project selected live components and gear overlays without cloning the slot or encoding its damage scalar.
    pub fn extra_components_matching(
        &self,
        select: impl Fn(&str) -> bool,
    ) -> Option<sunrise_core::nbt::Tag> {
        use sunrise_core::nbt::Tag;
        use sunrise_items::item_data::live_component_patch_matching;

        let base = live_component_patch_matching(self.base, false, &select);
        if self.overlays.is_none()
            || !(select("minecraft:enchantments") || select("minecraft:trim"))
        {
            return base;
        }

        let mut shell = ItemStack {
            item: self.base.item.clone(),
            count: self.base.count,
            damage: 0,
            extra: None,
        };
        self.overlay_gear(&mut shell, &select);
        let Some(Tag::Compound(gear)) = live_component_patch_matching(&shell, false, select) else {
            return base;
        };
        let Some(Tag::Compound(mut patch)) = base else {
            return Some(Tag::Compound(gear));
        };
        patch.extend(gear);
        Some(Tag::Compound(patch))
    }

    fn overlay_gear(&self, stack: &mut ItemStack, select: impl Fn(&str) -> bool) {
        let Some((gear, index)) = self.overlays else {
            return;
        };
        if select("minecraft:enchantments") && !gear.slots[index].is_empty() {
            stack.data_mut().enchantments = gear.slots[index].clone();
        }
        if select("minecraft:trim")
            && let Some(trim) = &gear.trims[index]
        {
            stack.data_mut().trim = Some(trim.clone());
        }
    }
}

/// Borrow one slot as `Mob::equipment_stacks` reads it, without reading the other slots.
pub fn equipment_slot(m: &Mob, slot: EquipmentSlot) -> Option<EquipmentRead<'_>> {
    let slot = slot as usize;
    let base = m.extra.slot_stack(slot)?;
    if slot > HEAD {
        return Some(EquipmentRead::resident(base));
    }
    let index = gear_slot(slot);
    let overlays = gear::gear(m)
        .filter(|gear| !gear.slots[index].is_empty() || gear.trims[index].is_some())
        .map(|gear| (gear, index));
    Some(EquipmentRead { base, overlays })
}

/// A changed slot at `LivingEntity.collectEquipmentChanges`, still borrowed.
pub struct EquipmentChange<'a> {
    pub current: Option<EquipmentRead<'a>>,
    pub previous: Option<&'a ItemStack>,
}

pub fn changed_equipment_slot(m: &Mob, slot: EquipmentSlot) -> Option<EquipmentChange<'_>> {
    let previous = m.extra.live.last_equipment(slot);
    let current = equipment_slot(m, slot);
    (!EquipmentRead::matches_previous(current.as_ref(), previous))
        .then_some(EquipmentChange { current, previous })
}

/// Detect at the body tick. Return the first changed slot if its callback
/// needs the owning game; otherwise apply vanilla's changes without yielding.
pub fn detect_equipment_updates(
    m: &mut Mob,
    catalog: &Arc<Catalog>,
    items: &Items,
    notify: impl FnOnce() -> bool,
) -> Option<EquipmentSlot> {
    if m.extra.equipment.is_empty() && !m.extra.live.has_worn() {
        return None;
    }
    let mut deltas = EquipmentDeltas::default();
    let mut notify = Some(notify);
    let mut first = None;
    for slot in EquipmentSlot::ALL {
        let Some(change) = changed_equipment_slot(m, slot) else {
            continue;
        };
        if first.is_none() {
            first = Some(slot);
            if notify.take().is_some_and(|notify| notify()) {
                return first;
            }
        }
        deltas.set(slot, change.current.map(|read| read.project().into_owned()));
    }
    finish_equipment_tick(m, catalog, items, deltas);
    None
}

pub fn equipment_attribute_seed(m: &Mob) -> Vec<sunrise_core::nbt::Tag> {
    crate::entity_nbt::attribute_entries(m)
        .into_iter()
        .map(|(_, entry)| entry)
        .collect()
}

/// Settle only the observed deltas, plus slots whose live modifiers changed
/// early (such as through a listener's equipment write).
pub fn finish_equipment_tick(
    m: &mut Mob,
    catalog: &Arc<Catalog>,
    items: &Items,
    deltas: EquipmentDeltas,
) {
    super::armor::prepare(m, catalog);
    if deltas.is_empty() && !m.extra.live.has_pending_equipment() {
        return;
    }
    let mut live = std::mem::take(&mut m.extra.live);
    if let Err(error) = live.finish_equipment_tick(
        catalog,
        &m.kind.id,
        items,
        deltas,
        |slot| equipment_slot(m, slot),
        || equipment_attribute_seed(m),
    ) {
        log::warn!("equipment attributes of {} {}: {error}", m.kind.id, m.id);
    }
    m.extra.live = live;
}

/// The whole stack a mob keeps in `slot` (damage, name, components, count,
/// enchantments and trim), when it kept more than the item id.
pub(crate) fn stored_stack(m: &Mob, slot: usize) -> Option<ItemStack> {
    let read = equipment_slot(m, *EquipmentSlot::ALL.get(slot)?)?;
    let base = read.base();
    if slot > HEAD || (base.damage == 0 && base.count <= 1 && base.extra.is_none()) {
        return None;
    }
    Some(read.project().into_owned())
}
/// What a mob has in `slot` as a stack, including its count, enchantments and trim.
pub fn item_stack(m: &Mob, slot: usize) -> Option<crate::items::ItemStack> {
    if !matches!(slot, MAINHAND..=HEAD | SADDLE) {
        return None;
    }
    let read = equipment_slot(m, *EquipmentSlot::ALL.get(slot)?)?;
    let base = read.base();
    if slot <= HEAD && (base.damage != 0 || base.count > 1 || base.extra.is_some()) {
        return Some(read.project().into_owned());
    }
    let mut stack = ItemStack {
        item: base.item.clone(),
        count: base.count,
        damage: 0,
        extra: None,
    };
    stack.set_crafting_enchantments(slot_enchantments(m, slot));
    if let Some(trim) = gear::gear(m).and_then(|g| g.trims[gear_slot(slot)].clone()) {
        stack.data_mut().trim = Some(trim);
    }
    Some(stack)
}
/// `LivingEntity.getItemBySlot` from the stacks a mob keeps: the hands and
/// armour from `item_stack` (`EquipmentSlot` ordinals match these slots), the
/// body and saddle from their own fields.
pub fn equipment_stack(
    m: &Mob,
    slot: sunrise_core::equipment_slot::EquipmentSlot,
) -> Option<crate::items::ItemStack> {
    use sunrise_core::equipment_slot::EquipmentSlot as S;
    match slot {
        S::Body | S::Saddle => m.item_by_slot(slot),
        _ => item_stack(m, slot as usize),
    }
}
/// `setItemSlot` with the whole `stack`: its enchantments, trim, damage, name
/// and count stay with the mob.
pub fn set_item_stack(m: &mut Mob, slot: usize, stack: crate::items::ItemStack) {
    set_item(m, slot, &stack.item);
    set_slot_enchantments(m, slot, stack.enchantments().to_vec());
    if slot <= HEAD {
        gear::set_trim(
            m,
            gear_slot(slot),
            stack.data().and_then(|d| d.trim.clone()),
        );
    }
    if slot <= HEAD {
        m.extra.set_slot_stack(slot, Some(stack));
    }
}
/// `ItemStack.setDamageValue` on what `m` has in `slot` (one of the hands or
/// the armour): its enchantments and trim stay.
pub fn set_slot_damage(m: &mut Mob, slot: usize, damage: u32) {
    let Some(id) = item(m, slot).filter(|_| slot <= HEAD) else {
        return;
    };
    let kept = m.extra.slot_stack(slot).cloned();
    let mut stack = kept
        .filter(|s| s.item == id)
        .unwrap_or_else(|| crate::items::ItemStack::new(&id, 1));
    stack.damage = damage;
    m.extra.set_slot_stack(slot, Some(stack));
}
/// `setItemSlot` with a fresh stack (enchantments cleared).
pub fn set_item(m: &mut Mob, slot: usize, id: &str) {
    match slot {
        MAINHAND..=HEAD => m.extra.set_slot_id(slot, Some(Arc::from(id))),
        SADDLE => {
            m.extra.set_saddled(true);
            return;
        }
        _ => return,
    }
    gear::set(m, gear_slot(slot), Vec::new());
}
/// The enchantments on what `m` has in `slot`.
pub(crate) fn slot_enchantments(m: &Mob, slot: usize) -> gear::Levels {
    gear::gear(m).map_or_else(Vec::new, |g| g.slots[gear_slot(slot)].clone())
}
/// Replace the enchantments of what `m` has in `slot` (after `set_item`).
pub fn set_slot_enchantments(m: &mut Mob, slot: usize, levels: gear::Levels) {
    gear::set(m, gear_slot(slot), levels);
}
/// `EquipmentUser.equip` for one rolled stack: `setItemSlot` puts the item in
/// `slot` with its own enchantments and trim (set_components), then the
/// slot's drop chance is stored when the table names one.
pub fn equip_stack(
    m: &mut Mob,
    slot: usize,
    stack: &sunrise_world::loot::LootStack,
    drop_chance: Option<f32>,
) {
    set_item(m, slot, &stack.item);
    if slot <= HEAD {
        let levels = stack
            .enchantments
            .iter()
            .map(|(id, level)| (Arc::from(id.as_str()), (*level).max(0) as u32))
            .collect();
        gear::set(m, gear_slot(slot), levels);
        let trim = stack
            .components
            .iter()
            .rev()
            .find(|(id, _)| id == "minecraft:trim")
            .and_then(|(_, v)| v.as_ref().and_then(crate::item_data::trim::from_nbt));
        gear::set_trim(m, gear_slot(slot), trim);
    }
    if let Some(chance) = drop_chance {
        set_drop_chance(m, slot, chance);
    }
}
/// `Mob.setDropChance`.
pub fn set_drop_chance(m: &mut Mob, slot: usize, chance: f32) {
    let c = m
        .extra
        .drop_chances
        .get_or_insert_with(|| Box::new([super::drops::DEFAULT_DROP_CHANCE; 8]));
    c[slot] = chance;
}
/// The drop chance stored for a slot (`DropChances.byEquipment`).
pub fn stored_drop_chance(m: &Mob, slot: usize) -> f32 {
    m.extra
        .drop_chances
        .as_ref()
        .map_or(super::drops::DEFAULT_DROP_CHANCE, |c| c[slot])
}

/// `SpecialDates.isHalloween`: October 31.
pub(crate) fn halloween_date(month: u32, day: u32) -> bool {
    (month, day) == (10, 31)
}

/// `SpecialDates.isHalloween` from the local calendar.
pub fn is_halloween() -> bool {
    use chrono::Datelike;
    let today = chrono::Local::now();
    halloween_date(today.month(), today.day())
}

/// `Zombie` / `AbstractSkeleton` halloween pumpkin: 25% of empty heads, 10%
/// of those a jack o'lantern, drop chance 0.
pub fn halloween_head(m: &mut Mob, random: &mut impl RandomSource, halloween: bool) {
    if !(zombie(m.species()) || skeleton(m.species())) {
        return;
    }
    if m.extra.head_id().is_none() && halloween && random.next_float() < 0.25 {
        let head = if random.next_float() < 0.1 {
            "minecraft:jack_o_lantern"
        } else {
            "minecraft:carved_pumpkin"
        };
        set_item(m, HEAD, head);
        set_drop_chance(m, HEAD, 0.0);
    }
}

impl<L: FinalizeLevel + ?Sized, R: RandomSource> Ctx<'_, L, R> {
    fn float(&mut self) -> f32 {
        self.random.next_float()
    }
    fn int(&mut self, bound: i32) -> i32 {
        self.random.next_int_bound(bound).unwrap_or(0)
    }
    fn special(&self) -> f32 {
        self.difficulty.special()
    }
    /// `UniformInt.sample`.
    fn uniform(&mut self, min: i32, max: i32) -> i32 {
        self.random
            .next_int_between_inclusive(min, max)
            .unwrap_or(min)
    }

    // ---- Mob ----------------------------------------------------------

    /// `Mob.finalizeSpawn`: the follow range spawn bonus and left-handedness.
    fn mob(&mut self, m: &mut Mob, g: Option<Group>) -> Option<Group> {
        mob_spawn(m, self.random);
        g
    }
    /// `AgeableMob.finalizeSpawn`.
    fn ageable(&mut self, m: &mut Mob, g: Option<Group>) -> Option<Group> {
        let mut g = g.unwrap_or(Group::Ageable(Ageable::babies(true)));
        if let Some(a) = g.ageable() {
            let (should, size, chance) = (a.should_spawn_baby, a.size, a.baby_chance);
            if should && size > 0 && self.float() <= chance {
                let start = baby_start_age(m.species());
                set_age(m, start, None);
            }
            if let Some(a) = g.ageable() {
                // AgeableMobGroupData.increaseGroupSizeByOne increments a Java int.
                a.size = a.size.wrapping_add(1);
            }
        }
        self.mob(m, Some(g))
    }
    /// `Mob.populateDefaultEquipmentSlots`: a set of armour from the head
    /// down, of one material.
    fn armor(&mut self, m: &mut Mob) {
        if self.float() >= 0.15 * self.special() {
            return;
        }
        let mut kind = self.int(3);
        for _ in 1..=3 {
            if self.float() < 0.1087 {
                kind += 1;
            }
        }
        let partial = if self.difficulty.hard { 0.1 } else { 0.25 };
        const MATERIAL: [&str; 6] = [
            "leather",
            "copper",
            "golden",
            "chainmail",
            "iron",
            "diamond",
        ];
        let mut first = true;
        // EQUIPMENT_POPULATION_ORDER: head, chest, legs, feet.
        for (slot, piece) in [
            (HEAD, "helmet"),
            (CHEST, "chestplate"),
            (LEGS, "leggings"),
            (FEET, "boots"),
        ] {
            if !first && self.float() < partial {
                break;
            }
            first = false;
            if item(m, slot).is_none()
                && let Some(material) = MATERIAL.get(kind as usize)
            {
                set_item(m, slot, &format!("minecraft:{material}_{piece}"));
            }
        }
    }
    /// `populateDefaultEquipmentSlots` of each class.
    fn populate(&mut self, m: &mut Mob) {
        use Species as S;
        match m.species() {
            S::Zombie | S::Husk | S::ZombieVillager => {
                self.armor(m);
                let chance = if self.difficulty.hard { 0.05 } else { 0.01 };
                if self.float() < chance {
                    let weapon = match self.int(6) {
                        0 => "minecraft:iron_sword",
                        1 => "minecraft:iron_spear",
                        _ => "minecraft:iron_shovel",
                    };
                    set_item(m, MAINHAND, weapon);
                }
            }
            S::Drowned => {
                if f64::from(self.float()) > 0.9 {
                    let weapon = if self.int(16) < 10 {
                        "minecraft:trident"
                    } else {
                        "minecraft:fishing_rod"
                    };
                    set_item(m, MAINHAND, weapon);
                }
            }
            S::ZombifiedPiglin => {
                let weapon = if self.int(20) == 0 {
                    "minecraft:golden_spear"
                } else {
                    "minecraft:golden_sword"
                };
                set_item(m, MAINHAND, weapon);
            }
            S::WitherSkeleton => set_item(m, MAINHAND, "minecraft:stone_sword"),
            s if skeleton(s) => {
                self.armor(m);
                set_item(m, MAINHAND, "minecraft:bow");
            }
            S::Pillager => set_item(m, MAINHAND, "minecraft:crossbow"),
            S::Vindicator => {
                if super::raider::state(m).is_none_or(|s| s.raid_id.is_none()) {
                    set_item(m, MAINHAND, "minecraft:iron_axe");
                }
            }
            S::Piglin => {
                if !m.is_baby() {
                    for (slot, piece) in [
                        (HEAD, "helmet"),
                        (CHEST, "chestplate"),
                        (LEGS, "leggings"),
                        (FEET, "boots"),
                    ] {
                        if self.float() < 0.1 {
                            set_item(m, slot, &format!("minecraft:golden_{piece}"));
                        }
                    }
                }
            }
            S::PiglinBrute => set_item(m, MAINHAND, "minecraft:golden_axe"),
            S::Vex => {
                set_item(m, MAINHAND, "minecraft:iron_sword");
                set_drop_chance(m, MAINHAND, 0.0);
            }
            S::Fox => {
                if self.float() < 0.2 {
                    let odds = self.float();
                    let held = if odds < 0.05 {
                        "minecraft:emerald"
                    } else if odds < 0.2 {
                        "minecraft:egg"
                    } else if odds < 0.4 {
                        if self.random.next_boolean() {
                            "minecraft:rabbit_foot"
                        } else {
                            "minecraft:rabbit_hide"
                        }
                    } else if odds < 0.6 {
                        "minecraft:wheat"
                    } else if odds < 0.8 {
                        "minecraft:leather"
                    } else {
                        "minecraft:feather"
                    };
                    set_item(m, MAINHAND, held);
                }
            }
            _ => self.armor(m),
        }
    }
    /// `Mob.populateDefaultEquipmentEnchantments`: the weapon one time in
    /// four and each piece of armour one time in two, scaled by the local
    /// difficulty, through the `mob_spawn_equipment` provider.
    fn enchant(&mut self, m: &mut Mob) {
        if m.species() == Species::WitherSkeleton {
            return;
        }
        self.enchant_slot(m, MAINHAND, 0.25);
        // Pillager.enchantSpawnedWeapon: one crossbow in 300 pierces
        // (`pillager_spawn_crossbow`).
        if m.species() == Species::Pillager
            && self.int(300) == 0
            && item(m, MAINHAND).as_deref() == Some("minecraft:crossbow")
        {
            self.enchant_from_provider(
                m,
                MAINHAND,
                "minecraft:crossbow",
                "minecraft:pillager_spawn_crossbow",
            );
        }
        // EquipmentSlot.VALUES order: feet, legs, chest, head.
        for slot in [FEET, LEGS, CHEST, HEAD] {
            self.enchant_slot(m, slot, 0.5);
        }
    }
    fn enchant_slot(&mut self, m: &mut Mob, slot: usize, chance: f32) {
        let Some(id) = item(m, slot) else { return };
        if self.float() >= chance * self.special() {
            return;
        }
        self.enchant_from_provider(m, slot, &id, "minecraft:mob_spawn_equipment");
    }
    /// `EnchantmentHelper.enchantItemFromProvider`: apply `provider` to `slot`.
    fn enchant_from_provider(&mut self, m: &mut Mob, slot: usize, id: &str, provider: &str) {
        let special = self.special();
        let stack = item_stack(m, slot).unwrap_or_else(|| crate::items::ItemStack::new(id, 1));
        let d = self.level.spawn_data();
        let picked =
            d.enchantments
                .enchant_from_provider(provider, d.items, self.random, &stack, special);
        for (enchantment, level) in picked {
            gear::upgrade(m, gear_slot(slot), &enchantment, level);
        }
    }
    fn halloween(&mut self, m: &mut Mob) {
        halloween_head(m, self.random, self.level.halloween());
    }

    // ---- jockeys ------------------------------------------------------

    fn create(&mut self, kind: &str, reason: Reason, at: &Mob) -> Option<Mob> {
        let mut j = self.level.create(kind, reason)?;
        // snapTo(x, y, z, yRot, 0).
        j.position = at.position;
        j.previous = at.position;
        j.yaw = at.yaw;
        j.previous_yaw = at.yaw;
        j.body_yaw = at.yaw;
        j.head_yaw = at.yaw;
        j.head_pitch = 0.0;
        Some(j)
    }
    /// The finalizeSpawn event for a rider one of the classes above created:
    /// `false` drops it, otherwise it is finalized unless the level said the
    /// listener already did (`Mob.finalizeSpawn` fires for riders too).
    fn rider_kept(&mut self, j: &mut Mob, reason: Reason, group: Option<Group>) -> bool {
        let event = self.level.spawn_event(j, reason);
        if !event.skip_initialization {
            self.finalize(j, reason, group);
        }
        !event.veto_admission
    }
    /// `startRiding`: the rider leaves the vehicle it was on.
    fn ride(&mut self, rider: &mut Mob, vehicle: &mut Mob) {
        if let Some(old) = rider.extra.mount
            && let Some(v) = self.created.iter_mut().find(|c| c.id == old)
            && v.extra.rider == Some(rider.id)
        {
            v.extra.rider = None;
        }
        rider.extra.mount = Some(vehicle.id);
        rider.extra.passenger = true;
        if vehicle.extra.rider.is_none() {
            vehicle.extra.rider = Some(rider.id);
        }
    }

    // ---- the classes --------------------------------------------------

    fn zombie(&mut self, m: &mut Mob, g: Option<Group>, reason: Reason) -> Option<Group> {
        let mut g = self.mob(m, g);
        let special = self.special();
        if reason != Reason::Conversion {
            m.extra.pick_up_loot = self.float() < 0.55 * special;
        }
        if g.is_none() {
            g = Some(Group::Zombie {
                baby: self.float() < 0.05,
                jockey: true,
                husk: None,
            });
        }
        if let Some(Group::Zombie { baby, jockey, .. }) = g {
            if baby {
                set_baby(m, true);
                if jockey {
                    if f64::from(self.float()) < 0.05 {
                        let search = m
                            .bounds()
                            .inflate(sunrise_core::geometry::Vec3::new(5.0, 3.0, 5.0));
                        if let Some(chicken) = self.level.free_chicken(search) {
                            self.level.ride_chicken(chicken, m.id);
                            m.extra.mount = Some(chicken);
                            m.extra.passenger = true;
                        }
                    } else if f64::from(self.float()) < 0.05
                        && let Some(mut chicken) =
                            self.create("minecraft:chicken", Reason::Jockey, m)
                        && self.rider_kept(&mut chicken, Reason::Jockey, None)
                    {
                        chicken.extra.jockey = true;
                        self.ride(m, &mut chicken);
                        self.jockey_rider = Some(m.clone());
                        self.created.push(chicken);
                    }
                }
            }
            let can_break_doors = self.float() < special * 0.1;
            super::doors::set_can_break_doors(m, can_break_doors);
            if reason != Reason::Conversion {
                self.populate(m);
                self.enchant(m);
            }
        }
        self.halloween(m);
        zombie_attributes(m, special, reason);
        g
    }
    fn husk(&mut self, m: &mut Mob, g: Option<Group>, reason: Reason) -> Option<Group> {
        let mut g = self.zombie(m, g, reason);
        if reason != Reason::Conversion {
            m.extra.pick_up_loot = self.float() < 0.55 * self.special();
        }
        if let Some(Group::Zombie { baby, jockey, .. }) = g {
            g = Some(Group::Zombie {
                baby,
                jockey,
                husk: Some(reason != Reason::Natural),
            });
        }
        if let Some(Group::Zombie {
            husk: Some(tried @ false),
            ..
        }) = &mut g
        {
            let (bx, by, bz) = (
                m.position.x.floor(),
                m.position.y.floor(),
                m.position.z.floor(),
            );
            let (w, h) = self.level.spawn_data().camel_husk_size;
            let half = w / 2.0;
            let bounds = Aabb::new(
                sunrise_core::geometry::Vec3::new(
                    bx + 0.5 - f64::from(half),
                    by,
                    bz + 0.5 - f64::from(half),
                ),
                sunrise_core::geometry::Vec3::new(
                    bx + 0.5 + f64::from(half),
                    by + f64::from(h),
                    bz + 0.5 + f64::from(half),
                ),
            );
            if self.level.no_collision(bounds) {
                *tried = true;
                if self.float() < 0.1 {
                    set_item(m, MAINHAND, "minecraft:iron_spear");
                    if let Some(mut camel) =
                        self.level.create("minecraft:camel_husk", Reason::Natural)
                    {
                        // setPos: the husk's position, the camel's own rotation.
                        camel.position = m.position;
                        camel.previous = m.position;
                        if self.rider_kept(&mut camel, reason, None) {
                            self.ride(m, &mut camel);
                            if let Some(mut parched) =
                                self.create("minecraft:parched", Reason::Natural, m)
                                && self.rider_kept(&mut parched, reason, None)
                            {
                                self.ride(&mut parched, &mut camel);
                                self.created.push(camel);
                                self.created.push(parched);
                            } else {
                                self.created.push(camel);
                            }
                        }
                    }
                }
            }
        }
        g
    }
    fn drowned(&mut self, m: &mut Mob, g: Option<Group>, reason: Reason) -> Option<Group> {
        let g = self.zombie(m, g, reason);
        if m.extra.off_hand_id().is_none() && self.float() < 0.03 {
            set_item(m, OFFHAND, "minecraft:nautilus_shell");
            set_drop_chance(m, OFFHAND, 2.0);
        }
        let at = [
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        ];
        if matches!(reason, Reason::Natural | Reason::Structure)
            && m.extra.main_hand_id() == Some("minecraft:trident")
            && self.float() < 0.5
            && !m.is_baby()
            && !self
                .level
                .biome_tag("minecraft:more_frequent_drowned_spawns", at)
            && let Some(mut nautilus) = self.create("minecraft:zombie_nautilus", Reason::Jockey, m)
            && self.rider_kept(&mut nautilus, reason, None)
        {
            if reason == Reason::Structure {
                nautilus.persistent = true;
            }
            self.ride(m, &mut nautilus);
            self.created.push(nautilus);
        }
        g
    }
    fn skeleton(&mut self, m: &mut Mob, g: Option<Group>) -> Option<Group> {
        let g = self.mob(m, g);
        self.populate(m);
        self.enchant(m);
        m.extra.pick_up_loot = self.float() < 0.55 * self.special();
        self.halloween(m);
        if m.species() == Species::WitherSkeleton {
            set_base(m, Attr::AttackDamage, 4.0);
        }
        g
    }
    fn spider(&mut self, m: &mut Mob, g: Option<Group>, reason: Reason) -> Option<Group> {
        let mut g = self.mob(m, g);
        if self.int(100) == 0
            && let Some(mut skeleton) = self.create("minecraft:skeleton", Reason::Jockey, m)
            && self.rider_kept(&mut skeleton, reason, None)
        {
            self.ride(&mut skeleton, m);
            self.created.push(skeleton);
        }
        if g.is_none() {
            let mut effect = None;
            if self.difficulty.hard && self.float() < 0.1 * self.special() {
                effect = Some(super::spawning::spider_random_effect(self.random));
            }
            g = Some(Group::Spider(effect));
        }
        if let Some(Group::Spider(Some(effect))) = g {
            m.add_effect(effect, -1, 0);
        }
        g
    }
    /// `Raider` then `PatrollingMonster.finalizeSpawn`.
    fn raider(&mut self, m: &mut Mob, g: Option<Group>, reason: Reason) -> Option<Group> {
        let witch = m.species() == Species::Witch;
        super::raider::state_mut(m).can_join_raid = !witch || reason != Reason::Natural;
        if !matches!(reason, Reason::Patrol | Reason::Event | Reason::Structure)
            && self.float() < 0.06
            && super::raider::can_be_leader(m.species())
        {
            super::raider::state_mut(m).patrol_leader = true;
        }
        // PatrollingMonster.finalizeSpawn: the leader wears
        // `Raid.getOminousBannerInstance`, the patterned stack clients draw.
        if super::raider::state(m).is_some_and(|s| s.patrol_leader) {
            set_item(m, HEAD, super::raider::BANNER);
            super::raider::wear_banner(m);
        }
        if reason == Reason::Patrol {
            super::raider::state_mut(m).patrolling = true;
        }
        self.mob(m, g)
    }
    fn piglin(&mut self, m: &mut Mob, g: Option<Group>, reason: Reason) -> Option<Group> {
        if reason != Reason::Structure {
            if self.float() < 0.2 {
                set_baby(m, true);
            } else if !m.is_baby() {
                // createSpawnWeapon, from the piglin's own random.
                let r = &mut m.random;
                let weapon = if f64::from(r.next_float()) < 0.5 {
                    "minecraft:crossbow"
                } else if r.next_int_bound(10).unwrap_or(1) == 0 {
                    "minecraft:golden_spear"
                } else {
                    "minecraft:golden_sword"
                };
                set_item(m, MAINHAND, weapon);
            }
        }
        // PiglinAi.initMemories: HUNTED_RECENTLY for 30 to 120 seconds.
        let hunt = self.uniform(600, 2400);
        set_memory(m, "hunted_recently", 1, i64::from(hunt));
        self.populate(m);
        self.enchant(m);
        self.mob(m, g)
    }
    fn strider(&mut self, m: &mut Mob, g: Option<Group>) -> Option<Group> {
        if m.is_baby() {
            return self.ageable(m, g);
        }
        let mut g = g;
        if self.int(30) == 0 && self.level.spawning_monsters() {
            if let Some(mut jockey) = self.create("minecraft:zombified_piglin", Reason::Jockey, m) {
                let baby = self.float() < 0.05;
                if !self.rider_kept(
                    &mut jockey,
                    Reason::Jockey,
                    Some(Group::Zombie {
                        baby,
                        jockey: false,
                        husk: None,
                    }),
                ) {
                    return self.ageable(m, Some(Group::Ageable(Ageable::chance(0.0))));
                }
                g = Some(self.strider_jockey(m, &mut jockey));
                set_item(&mut jockey, MAINHAND, "minecraft:warped_fungus_on_a_stick");
                set_item(m, SADDLE, "minecraft:saddle");
                set_drop_chance(m, SADDLE, 2.0);
                self.created.push(jockey);
            }
        } else if self.int(10) == 0 {
            if let Some(mut jockey) = self.create("minecraft:strider", Reason::Jockey, m) {
                set_age(&mut jockey, -24000, None);
                if !self.rider_kept(&mut jockey, Reason::Jockey, None) {
                    return self.ageable(m, Some(Group::Ageable(Ageable::chance(0.0))));
                }
                g = Some(self.strider_jockey(m, &mut jockey));
                self.created.push(jockey);
            }
        } else {
            g = Some(Group::Ageable(Ageable::chance(0.5)));
        }
        self.ageable(m, g)
    }
    /// `Strider.spawnJockey`.
    fn strider_jockey(&mut self, m: &mut Mob, jockey: &mut Mob) -> Group {
        jockey.position = m.position;
        jockey.yaw = m.yaw;
        self.ride(jockey, m);
        Group::Ageable(Ageable::chance(0.0))
    }
    /// `AbstractHorse.finalizeSpawn` with each `randomizeAttributes`.
    fn horse(&mut self, m: &mut Mob, g: Option<Group>) -> Option<Group> {
        let g = g.or(Some(Group::Ageable(Ageable::chance(0.2))));
        use Species as S;
        match m.species() {
            S::Horse => {
                let health = self.horse_health();
                set_base(m, Attr::MaxHealth, f64::from(health));
                let speed = self.horse_speed();
                set_base(m, Attr::MovementSpeed, speed);
                let jump = self.horse_jump();
                set_base(m, Attr::JumpStrength, jump);
            }
            S::Donkey | S::Mule | S::Llama | S::TraderLlama => {
                let health = self.horse_health();
                set_base(m, Attr::MaxHealth, f64::from(health));
            }
            S::SkeletonHorse => {
                let jump = self.horse_jump();
                set_base(m, Attr::JumpStrength, jump);
            }
            S::ZombieHorse => {
                let d = 0.066_666_666_666_666_67;
                let jump = 0.5
                    + self.random.next_double() * d
                    + self.random.next_double() * d
                    + self.random.next_double() * d;
                set_base(m, Attr::JumpStrength, jump);
                let speed = (9.0
                    + self.random.next_double()
                    + self.random.next_double()
                    + self.random.next_double())
                    / f64::from(42.16f32);
                set_base(m, Attr::MovementSpeed, speed);
            }
            _ => {}
        }
        self.ageable(m, g)
    }
    fn horse_health(&mut self) -> f32 {
        15.0 + self.int(8) as f32 + self.int(9) as f32
    }
    fn horse_speed(&mut self) -> f64 {
        (f64::from(0.45f32)
            + self.random.next_double() * 0.3
            + self.random.next_double() * 0.3
            + self.random.next_double() * 0.3)
            * 0.25
    }
    fn horse_jump(&mut self) -> f64 {
        f64::from(0.4f32)
            + self.random.next_double() * 0.2
            + self.random.next_double() * 0.2
            + self.random.next_double() * 0.2
    }
    fn villager_type(&mut self, m: &mut Mob) {
        let at = block(m);
        m.extra.variant = self.level.mob_maps().villager_type(&self.level.biome(at));
    }

    /// `finalizeSpawn` of the mob's class.
    pub fn finalize(&mut self, m: &mut Mob, reason: Reason, g: Option<Group>) -> Option<Group> {
        use Species as S;
        let s = m.species();
        match s {
            S::Pig => {
                let at = block(m);
                if let Some(v) =
                    self.level
                        .spawn_data()
                        .variants
                        .pig
                        .select(&*self.level, at, self.random)
                {
                    m.extra.pig_variant = Some(v);
                }
                m.extra.pig_sound_variant = self
                    .level
                    .spawn_data()
                    .variants
                    .pig_sound
                    .random(self.random);
                self.ageable(m, g)
            }
            S::Cow => {
                let at = block(m);
                if let Some(v) =
                    self.level
                        .spawn_data()
                        .variants
                        .cow
                        .select(&*self.level, at, self.random)
                {
                    m.extra.cow_variant = Some(v);
                }
                m.extra.cow_sound_variant = self
                    .level
                    .spawn_data()
                    .variants
                    .cow_sound
                    .random(self.random);
                self.ageable(m, g)
            }
            S::Chicken => {
                let at = block(m);
                if let Some(v) =
                    self.level
                        .spawn_data()
                        .variants
                        .chicken
                        .select(&*self.level, at, self.random)
                {
                    m.extra.chicken_variant = Some(v);
                }
                m.extra.chicken_sound_variant = self
                    .level
                    .spawn_data()
                    .variants
                    .chicken_sound
                    .random(self.random);
                self.ageable(m, g)
            }
            S::Cat => {
                let g = self.ageable(m, g);
                let at = block(m);
                if let Some(v) =
                    self.level
                        .spawn_data()
                        .variants
                        .cat
                        .select(&*self.level, at, self.random)
                {
                    m.extra.cat_variant = Some(v);
                }
                m.extra.cat_sound_variant = self
                    .level
                    .spawn_data()
                    .variants
                    .cat_sound
                    .random(self.random);
                g
            }
            S::Wolf => {
                let g = match g {
                    Some(Group::Wolf(a, v)) => {
                        m.extra.wolf_variant = Some(v);
                        Some(Group::Wolf(a, v))
                    }
                    other => {
                        let r = &self.level.spawn_data().variants.wolf;
                        let at = block(m);
                        match r.select(&*self.level, at, self.random) {
                            Some(v) => {
                                m.extra.wolf_variant = Some(v);
                                Some(Group::Wolf(Ageable::babies(false), v))
                            }
                            None => other,
                        }
                    }
                };
                m.extra.wolf_sound_variant = self
                    .level
                    .spawn_data()
                    .variants
                    .wolf_sound
                    .random(self.random);
                self.ageable(m, g)
            }
            S::Frog => {
                if let Some(v) = self.level.spawn_data().variants.frog.select(
                    &*self.level,
                    block(m),
                    self.random,
                ) {
                    m.extra.frog_variant = Some(v);
                }
                let jump = self.uniform(100, 140);
                set_memory(m, "long_jump_cooling_down", i64::from(jump), i64::MAX);
                self.ageable(m, g)
            }
            S::ZombieNautilus => {
                let at = block(m);
                if let Some(v) = self.level.spawn_data().variants.zombie_nautilus.select(
                    &*self.level,
                    at,
                    self.random,
                ) {
                    m.extra.zombie_nautilus_variant = Some(v);
                }
                let cooldown = self.uniform(2400, 3600);
                set_memory(m, "attack_target_cooldown", i64::from(cooldown), i64::MAX);
                m.extra.counter = cooldown;
                self.ageable(m, g)
            }
            S::Nautilus => {
                let cooldown = self.uniform(2400, 3600);
                set_memory(m, "attack_target_cooldown", i64::from(cooldown), i64::MAX);
                m.extra.counter = cooldown;
                self.ageable(m, g)
            }
            S::Sheep => {
                let climate = if self
                    .level
                    .biome_tag("minecraft:spawns_warm_variant_farm_animals", block(m))
                {
                    Climate::Warm
                } else if self
                    .level
                    .biome_tag("minecraft:spawns_cold_variant_farm_animals", block(m))
                {
                    Climate::Cold
                } else {
                    Climate::Temperate
                };
                m.extra.color = super::farm::sheep_color(climate, self.random);
                self.ageable(m, g)
            }
            S::Rabbit => {
                let at = block(m);
                let mut variant = super::rabbit::random_variant(
                    self.level.biome_tag("minecraft:spawns_white_rabbits", at),
                    self.level.biome_tag("minecraft:spawns_gold_rabbits", at),
                    self.random,
                );
                let g = match g {
                    Some(Group::Rabbit(a, v)) => {
                        variant = v;
                        Group::Rabbit(a, v)
                    }
                    _ => Group::Rabbit(Ageable::chance(1.0), variant),
                };
                m.extra.variant = variant;
                self.ageable(m, Some(g))
            }
            S::Horse => {
                let (g, variant) = match g {
                    Some(Group::Horse(a, v)) => (Group::Horse(a, v), v),
                    _ => {
                        let v = self.int(7) as u8;
                        (Group::Horse(Ageable::babies(true), v), v)
                    }
                };
                let markings = self.int(5) as u8;
                m.extra.variant = variant;
                m.extra.variant2 = markings;
                self.horse(m, Some(g))
            }
            S::Llama | S::TraderLlama => {
                let mut g = g;
                if s == S::TraderLlama {
                    if reason == Reason::Event {
                        set_age(m, 0, None);
                    }
                    if g.is_none() {
                        g = Some(Group::Ageable(Ageable::babies(false)));
                    }
                }
                let max = if self.float() < 0.04 { 5 } else { 3 };
                m.extra.variant2 = (1 + self.int(max)) as u8;
                let g = match g {
                    Some(Group::Llama(a, v)) => {
                        m.extra.variant = v;
                        Group::Llama(a, v)
                    }
                    _ => {
                        let v = self.int(4) as u8;
                        m.extra.variant = v;
                        Group::Llama(Ageable::babies(true), v)
                    }
                };
                self.horse(m, Some(g))
            }
            S::Donkey | S::Mule | S::SkeletonHorse => self.horse(m, g),
            S::ZombieHorse => {
                if reason == Reason::Natural
                    && let Some(mut zombie) = self.create("minecraft:zombie", Reason::Jockey, m)
                    && self.rider_kept(&mut zombie, reason, None)
                {
                    set_item(&mut zombie, MAINHAND, "minecraft:iron_spear");
                    self.ride(&mut zombie, m);
                    self.created.push(zombie);
                }
                self.horse(m, g)
            }
            S::Camel | S::CamelHusk => {
                let now = self.level.game_time();
                m.extra.pose_change_tick = (now - 52 - 1).max(0);
                super::camel::stand_up_instantly(m);
                self.horse(m, g)
            }
            S::Goat => {
                goat_init_memories(m, self.random);
                m.extra.screaming = self.random.next_double() < 0.02;
                age_boundary_reached(m, None);
                if !m.is_baby() && self.float() < 0.1 {
                    let left = self.random.next_boolean();
                    m.extra.lost_horns[usize::from(!left)] = true;
                }
                self.ageable(m, g)
            }
            S::Panda => {
                let main = panda_gene(self.int(16));
                let hidden = panda_gene(self.int(16));
                m.extra.variant = main;
                m.extra.variant2 = hidden;
                super::panda::set_attributes(m);
                self.ageable(m, g.or(Some(Group::Ageable(Ageable::chance(0.2)))))
            }
            S::PolarBear | S::Ocelot => {
                self.ageable(m, g.or(Some(Group::Ageable(Ageable::chance(1.0)))))
            }
            S::Fox => {
                let at = block(m);
                let mut variant = u8::from(self.level.biome_tag("minecraft:spawns_snow_foxes", at));
                let mut baby = false;
                let g = match g {
                    Some(Group::Fox(a, v)) => {
                        variant = v;
                        baby = a.size >= 2;
                        Group::Fox(a, v)
                    }
                    _ => Group::Fox(Ageable::babies(false), variant),
                };
                m.extra.variant = variant;
                if baby {
                    set_age(m, -24000, None);
                }
                self.populate(m);
                self.ageable(m, Some(g))
            }
            S::Parrot => {
                m.extra.variant = self.int(5) as u8;
                self.ageable(m, g.or(Some(Group::Ageable(Ageable::babies(false)))))
            }
            S::Axolotl => {
                if reason == Reason::Bucket {
                    return g;
                }
                let mut baby = false;
                let g = match g {
                    Some(Group::Axolotl(a, types)) => {
                        baby = a.size >= 2;
                        Group::Axolotl(a, types)
                    }
                    _ => {
                        let a = self.int(4) as u8;
                        let b = self.int(4) as u8;
                        Group::Axolotl(Ageable::babies(false), [a, b])
                    }
                };
                if let Group::Axolotl(_, types) = g {
                    m.extra.variant = types[self.int(2) as usize];
                }
                if baby {
                    set_age(m, -24000, None);
                }
                self.ageable(m, Some(g))
            }
            S::Squid | S::GlowSquid => {
                self.ageable(m, g.or(Some(Group::Ageable(Ageable::chance(0.05)))))
            }
            S::Dolphin => {
                m.air = 4800;
                m.head_pitch = 0.0;
                self.ageable(m, g.or(Some(Group::Ageable(Ageable::chance(0.1)))))
            }
            S::Turtle => {
                m.extra.home = Some(block(m));
                self.ageable(m, g)
            }
            S::Villager | S::WanderingTrader => {
                if s == S::Villager {
                    if reason == Reason::Breeding {
                        m.extra.profession = Default::default();
                    }
                    self.villager_type(m);
                }
                self.ageable(m, g.or(Some(Group::Ageable(Ageable::babies(false)))))
            }
            S::Hoglin => {
                if self.float() < 0.2 {
                    set_baby(m, true);
                }
                self.ageable(m, g)
            }
            S::Zoglin => {
                if self.float() < 0.2 {
                    set_baby(m, true);
                }
                self.mob(m, g)
            }
            S::Strider => self.strider(m, g),
            S::Slime | S::MagmaCube | S::SulfurCube => {
                let g = if s == S::SulfurCube {
                    g
                } else {
                    g.or(Some(Group::Ageable(Ageable::babies(false))))
                };
                let g = self.ageable(m, g);
                if s == S::SulfurCube {
                    let size = if m.is_baby() { 1 } else { 2 };
                    sulfur_size(m, size);
                } else {
                    let mut scale = self.int(3);
                    if scale < 2 && self.float() < 0.5 * self.special() {
                        scale += 1;
                    }
                    cube_size(m, 1 << scale);
                }
                g
            }
            S::Cod | S::Salmon | S::TropicalFish => {
                if s == S::Salmon {
                    // A weighted list (small 30, medium 50, large 15) on the
                    // salmon's own random.
                    let r = m.random.next_int_bound(95).unwrap_or(0);
                    m.extra.variant = if r < 30 {
                        0
                    } else if r < 80 {
                        1
                    } else {
                        2
                    };
                }
                let given = g;
                self.mob(m, None);
                let g = match given {
                    None => Group::School(m.id, None),
                    Some(Group::School(leader, v)) => {
                        m.extra.leader = Some(leader);
                        Group::School(leader, v)
                    }
                    Some(other) => other,
                };
                if s != S::TropicalFish {
                    return Some(g);
                }
                let (g, (p, b, c)) = match g {
                    Group::School(leader, Some(v)) => (Group::School(leader, Some(v)), v),
                    other => {
                        if f64::from(self.float()) < 0.9 {
                            let v = super::aquatic::COMMON_TROPICAL[self.int(22) as usize];
                            (Group::School(m.id, Some(v)), v)
                        } else {
                            m.extra.lone = true;
                            let p = self.int(12) as u8;
                            let b = self.int(16) as u8;
                            let c = self.int(16) as u8;
                            (other, (p, b, c))
                        }
                    }
                };
                m.extra.variant = p;
                m.extra.variant2 = b;
                m.extra.variant3 = c;
                Some(g)
            }
            S::Zombie | S::ZombifiedPiglin => self.zombie(m, g, reason),
            S::ZombieVillager => {
                // `ZombieVillager.finalizeSpawn` chooses the villager type and
                // nothing else: the profession came from the constructor.
                self.villager_type(m);
                // The profession is drawn at construction, not finalizeSpawn.
                self.zombie(m, g, reason)
            }
            S::Husk => self.husk(m, g, reason),
            S::Drowned => self.drowned(m, g, reason),
            s if skeleton(s) => self.skeleton(m, g),
            S::Spider => self.spider(m, g, reason),
            // CaveSpider.finalizeSpawn returns at once.
            S::CaveSpider => g,
            S::Pillager => {
                self.populate(m);
                self.enchant(m);
                self.raider(m, g, reason)
            }
            S::Vindicator => {
                let g = self.raider(m, g, reason);
                self.populate(m);
                self.enchant(m);
                g
            }
            S::Illusioner => {
                set_item(m, MAINHAND, "minecraft:bow");
                self.raider(m, g, reason)
            }
            s if raider(s) => self.raider(m, g, reason),
            S::Piglin => self.piglin(m, g, reason),
            S::PiglinBrute => {
                let home = block(m);
                m.extra.home = Some(home);
                set_memory(m, "home", 0, i64::MAX);
                self.populate(m);
                self.mob(m, g)
            }
            S::Vex => {
                self.populate(m);
                self.enchant(m);
                self.mob(m, g)
            }
            S::Warden => {
                super::warden::finalize(m, reason == Reason::Triggered, &mut Vec::new());
                set_memory(m, "dig_cooldown", 0, 1200);
                if reason == Reason::Triggered {
                    set_memory(m, "is_emerging", 0, super::warden::EMERGE_DURATION);
                }
                self.mob(m, g)
            }
            S::Phantom => {
                let at = block(m);
                m.extra.home = Some([at[0], at[1] + 5, at[2]]);
                m.extra.size = 0;
                self.mob(m, g)
            }
            S::Shulker => {
                m.yaw = 0.0;
                m.head_yaw = 0.0;
                m.previous_yaw = 0.0;
                m.previous_head_yaw = 0.0;
                self.mob(m, g)
            }
            _ if ageable(s) => self.ageable(m, g),
            _ => self.mob(m, g),
        }
    }
}

/// `Mob.finalizeSpawn` of a fresh mob. Returns the group data for the next
/// mob of the pack.
pub fn finalize<L: FinalizeLevel + ?Sized, R: RandomSource>(
    cx: &mut Ctx<'_, L, R>,
    m: &mut Mob,
    reason: Reason,
    g: Option<Group>,
) -> Option<Group> {
    cx.finalize(m, reason, g)
}

fn block(m: &Mob) -> [i32; 3] {
    [
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ]
}
/// A brain memory finalizeSpawn sets: value and ticks until it expires.
fn set_memory(m: &mut Mob, name: &'static str, value: i64, ttl: i64) {
    match m.extra.memories.iter_mut().find(|x| x.0 == name) {
        Some(x) => *x = (name, value, ttl),
        None => m.extra.memories.push((name, value, ttl)),
    }
}
/// `GoatAi.initMemories`: a long jump cooldown (600 to 1200 ticks) and a ram
/// cooldown (600 to 6000), drawn from `random`.
pub fn goat_init_memories(m: &mut Mob, random: &mut impl RandomSource) {
    let jump = random.next_int_between_inclusive(600, 1200).unwrap_or(600);
    set_memory(m, "long_jump_cooling_down", i64::from(jump), i64::MAX);
    let ram = random.next_int_between_inclusive(600, 6000).unwrap_or(600);
    set_memory(m, "ram_cooldown_ticks", i64::from(ram), i64::MAX);
}
/// `Panda.Gene.getRandom` (enum order: normal, lazy, worried, playful,
/// brown, weak, aggressive).
pub(crate) fn panda_gene(roll: i32) -> u8 {
    match roll {
        0 => 1,
        1 => 2,
        2 => 3,
        4 => 6,
        r if r < 9 => 5,
        r if r < 11 => 4,
        _ => 0,
    }
}
/// `Panda.Gene.getVariantFromGenes`: a recessive gene shows when both match.
pub(crate) fn panda_variant(main: u8, hidden: u8) -> u8 {
    let recessive = matches!(main, 4 | 5);
    if recessive && main != hidden { 0 } else { main }
}
/// `AbstractCubeMob.setSize(size, true)` for slimes and magma cubes.
/// `SulfurCube.setSize(size, true)`.
fn sulfur_size(m: &mut Mob, size: i32) {
    super::sulfur_cube::set_size(m, size, true);
}
fn cube_size(m: &mut Mob, size: i32) {
    super::cube::set_size(m, size);
}
/// The index the game keeps for an enum variant species (`Mooshroom`, `Fox`,
/// `Horse`, `Llama`, `TraderLlama`).
pub fn variant_index(s: Species, name: &str) -> u8 {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    let i = match s {
        // MushroomCow.Variant: red is the codec's default, brown the other.
        Species::Mooshroom => ["red", "brown"].iter().position(|v| *v == name),
        // Fox.Variant, read from a `minecraft:fox/variant` component; the save
        // keeps the same two names under `Type`.
        Species::Fox => ["red", "snow"].iter().position(|v| *v == name),
        // Horse.Variant and Llama.Variant, the coat colours their
        // `.../variant` components carry; a trader llama inherits the llama's.
        Species::Horse => super::equine::VARIANT_NAMES.iter().position(|v| *v == name),
        Species::Llama | Species::TraderLlama => {
            super::llama::VARIANTS.iter().position(|v| *v == name)
        }
        _ => None,
    };
    i.unwrap_or(0) as u8
}
/// The variant data component a mob reports (`Entity.get`, e.g.
/// `Wolf.get(DataComponents.WOLF_VARIANT)`): component id and registry entry.
pub fn variant_component(m: &Mob) -> Option<(&'static str, String)> {
    species_variant_component(m.species(), m.extra.variant)
}
/// [`variant_component`] resolved against the world's data-driven variant tables
/// for species whose variants are registry holders.
pub fn variant_component_in(
    m: &Mob,
    variants: &super::variants::VariantTables,
) -> Option<(&'static str, String)> {
    match m.species() {
        Species::Wolf => {
            let r = m.wolf_variant(&variants.wolf)?;
            let id = variants.wolf.id(r)?;
            Some(("minecraft:wolf/variant", id.to_owned()))
        }
        Species::Cat => {
            let r = m.cat_variant(&variants.cat)?;
            let id = variants.cat.id(r)?;
            Some(("minecraft:cat/variant", id.to_owned()))
        }
        Species::Pig => {
            let r = m.pig_variant(&variants.pig)?;
            let id = variants.pig.id(r)?;
            Some(("minecraft:pig/variant", id.to_owned()))
        }
        Species::Cow => {
            let r = m.cow_variant(&variants.cow)?;
            let id = variants.cow.id(r)?;
            Some(("minecraft:cow/variant", id.to_owned()))
        }
        Species::Chicken => {
            let r = m.chicken_variant(&variants.chicken)?;
            let id = variants.chicken.id(r)?;
            Some(("minecraft:chicken/variant", id.to_owned()))
        }
        Species::Frog => {
            let r = m.frog_variant(&variants.frog)?;
            let id = variants.frog.id(r)?;
            Some(("minecraft:frog/variant", id.to_owned()))
        }
        Species::ZombieNautilus => {
            let r = m.zombie_nautilus_variant(&variants.zombie_nautilus)?;
            let id = variants.zombie_nautilus.id(r)?;
            Some(("minecraft:zombie_nautilus/variant", id.to_owned()))
        }
        _ => variant_component(m),
    }
}
/// [`variant_component`] of an enum-variant species with the game's variant index.
pub fn species_variant_component(s: Species, variant: u8) -> Option<(&'static str, String)> {
    let component = match s {
        Species::Mooshroom => "minecraft:mooshroom/variant",
        Species::Horse => "minecraft:horse/variant",
        Species::Llama | Species::TraderLlama => "minecraft:llama/variant",
        _ => return None,
    };
    Some((component, format!("minecraft:{}", variant_name(s, variant))))
}
/// The enum name of the game's variant index (`Mooshroom`, `Horse`, `Llama`,
/// `TraderLlama`).
pub fn variant_name(s: Species, index: u8) -> &'static str {
    let i = index as usize;
    match s {
        Species::Mooshroom => ["red", "brown"].get(i).copied().unwrap_or("red"),
        Species::Horse => super::equine::VARIANT_NAMES
            .get(i)
            .copied()
            .unwrap_or("white"),
        Species::Llama | Species::TraderLlama => {
            super::llama::VARIANTS.get(i).copied().unwrap_or("creamy")
        }
        _ => "",
    }
}
/// `VillagerType.BY_BIOME` (index into `render::VILLAGER_TYPES`: plains,
/// desert, jungle, savanna, snow, swamp, taiga); other biomes are plains.
pub const BIOME_VILLAGER_TYPES: &[(&str, u8)] = &[
    ("badlands", 1),
    ("desert", 1),
    ("eroded_badlands", 1),
    ("wooded_badlands", 1),
    ("bamboo_jungle", 2),
    ("jungle", 2),
    ("sparse_jungle", 2),
    ("savanna_plateau", 3),
    ("savanna", 3),
    ("windswept_savanna", 3),
    ("deep_frozen_ocean", 4),
    ("frozen_ocean", 4),
    ("frozen_river", 4),
    ("ice_spikes", 4),
    ("snowy_beach", 4),
    ("snowy_taiga", 4),
    ("snowy_plains", 4),
    ("grove", 4),
    ("snowy_slopes", 4),
    ("frozen_peaks", 4),
    ("jagged_peaks", 4),
    ("swamp", 5),
    ("mangrove_swamp", 5),
    ("old_growth_spruce_taiga", 6),
    ("old_growth_pine_taiga", 6),
    ("windswept_gravelly_hills", 6),
    ("windswept_hills", 6),
    ("taiga", 6),
    ("windswept_forest", 6),
];
/// `VillagerType.byBiome` over vanilla's table.
pub fn villager_type(biome: &str) -> u8 {
    let biome = biome.strip_prefix("minecraft:").unwrap_or(biome);
    BIOME_VILLAGER_TYPES
        .iter()
        .find(|(b, _)| *b == biome)
        .map_or(0, |(_, t)| *t)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mob's own attribute record sanitizes with the definitions of the
    /// catalog its type loaded with, not with copies of vanilla's bounds:
    /// 26.3's catalog keeps `max_health` at 1..1024 and `knockback_resistance`
    /// at -2..1, and a catalog with wider bounds keeps more.
    #[test]
    fn a_mobs_attributes_take_the_bounds_of_the_types_catalog() {
        use sunrise_world::attributes::entity::Catalog;
        let bits = |v: f64| format!("{:016x}", v.to_bits());
        let catalog = |max_health: f64| {
            Catalog::from_json(&serde_json::json!({
                "data_version": sunrise_world::DATA_VERSION,
                "attributes": [
                    {"id": "minecraft:max_health", "default": bits(20.0), "min": bits(1.0), "max": bits(max_health), "syncable": true},
                    {"id": "minecraft:knockback_resistance", "default": bits(0.0), "min": bits(-2.0), "max": bits(1.0), "syncable": true},
                ],
                "entities": {},
            }))
            .unwrap()
        };
        let mob = |catalog: &Catalog| {
            let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
            kind.attribute_bounds = AttrBounds::of(&Arc::new(catalog.clone()));
            let mut m = Mob::new(1, Arc::new(kind), sunrise_core::geometry::Vec3::ZERO, 0.0);
            set_base(&mut m, Attr::MaxHealth, 5000.0);
            set_base(&mut m, Attr::KnockbackResistance, -1.5);
            m
        };
        let vanilla = mob(&catalog(1024.0));
        assert_eq!(value(&vanilla, Attr::MaxHealth), 1024.0);
        assert_eq!(vanilla.attrs.max_health, 1024.0);
        assert_eq!(value(&vanilla, Attr::KnockbackResistance), -1.5);
        let mut nan = mob(&catalog(1024.0));
        set_base(&mut nan, Attr::MaxHealth, f64::NAN);
        assert_eq!(value(&nan, Attr::MaxHealth), 1.0, "NaN is the minimum");
        let wide = mob(&catalog(1e6));
        assert_eq!(value(&wide, Attr::MaxHealth), 5000.0);
        assert_eq!(wide.attrs.max_health, 5000.0);
    }

    #[test]
    fn panda_genes_follow_the_vanilla_table() {
        assert_eq!(panda_gene(0), 1);
        assert_eq!(panda_gene(3), 5);
        assert_eq!(panda_gene(4), 6);
        assert_eq!(panda_gene(9), 4);
        assert_eq!(panda_gene(15), 0);
        assert_eq!(panda_variant(4, 0), 0);
        assert_eq!(panda_variant(4, 4), 4);
        assert_eq!(panda_variant(6, 0), 6);
    }
    /// `MushroomCow.get(DataComponentGetter)`: a mooshroom reports its colour
    /// as the variant component, and the name maps back to the index.
    #[test]
    fn a_mooshroom_reports_its_variant_component() {
        for (index, name) in [(0, "minecraft:red"), (1, "minecraft:brown")] {
            assert_eq!(
                species_variant_component(Species::Mooshroom, index),
                Some(("minecraft:mooshroom/variant", name.to_owned()))
            );
            assert_eq!(variant_index(Species::Mooshroom, name), index);
            assert_eq!(
                format!("minecraft:{}", variant_name(Species::Mooshroom, index)),
                name
            );
        }
        // Variant.DEFAULT is red: an unknown name, or an index past brown.
        assert_eq!(variant_index(Species::Mooshroom, "minecraft:temperate"), 0);
        assert_eq!(variant_name(Species::Mooshroom, 2), "red");
    }
    /// `Horse.get(DataComponents.HORSE_VARIANT)` and
    /// `Llama.get(DataComponents.LLAMA_VARIANT)`: the coat colour a horse or a
    /// llama reports, and the names a `.../variant` component carries.
    #[test]
    fn a_horse_and_a_llama_report_their_variant_component() {
        for (species, component, names) in [
            (
                Species::Horse,
                "minecraft:horse/variant",
                &crate::mobs::equine::VARIANT_NAMES[..],
            ),
            (
                Species::Llama,
                "minecraft:llama/variant",
                &crate::mobs::llama::VARIANTS[..],
            ),
            (
                Species::TraderLlama,
                "minecraft:llama/variant",
                &crate::mobs::llama::VARIANTS[..],
            ),
        ] {
            for (index, name) in names.iter().enumerate() {
                let index = index as u8;
                let name = format!("minecraft:{name}");
                assert_eq!(
                    species_variant_component(species, index),
                    Some((component, name.clone())),
                    "{species:?} {index}"
                );
                assert_eq!(variant_index(species, &name), index, "{name}");
                assert_eq!(format!("minecraft:{}", variant_name(species, index)), name);
            }
        }
        // Horse.Variant.WHITE and Llama.Variant.CREAMY are the defaults.
        assert_eq!(variant_index(Species::Horse, "minecraft:temperate"), 0);
        assert_eq!(variant_index(Species::Llama, "minecraft:temperate"), 0);
        assert_eq!(variant_name(Species::Horse, 7), "white");
        assert_eq!(variant_name(Species::TraderLlama, 4), "creamy");
    }
    /// #2269: the offspring of a spawn egg used on a mob is a baby of that
    /// kind (`SpawnEggItem.spawnOffspringFromSpawnEgg`), and only the mob's
    /// own kind takes the egg.
    #[test]
    fn a_spawn_egg_offspring_is_a_baby() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            return;
        }
        let data = crate::mobs::MobTypes::load(&root).unwrap();
        let kind = std::sync::Arc::new(data.types.get("minecraft:cow").unwrap().clone());
        let mut child = crate::mobs::Mob::new(
            2,
            kind,
            sunrise_core::geometry::Vec3::new(0.0, 4.0, 0.0),
            0.0,
        );
        assert!(!child.is_baby());
        spawn_egg_offspring(&mut child, Species::Cow, 3, 0);
        // The offspring is a baby of the mob's kind.
        assert!(child.is_baby());
        // It keeps the parent's variant.
        assert_eq!(child.extra.variant, 3);
    }
    /// #7877: `/attribute <mob> <attr> modifier add` hands `add_modifier` an
    /// id typed at run time; it lands in the value, reads back, and goes.
    #[test]
    fn a_command_modifier_with_a_runtime_id_changes_the_value() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            return;
        }
        let data = crate::mobs::MobTypes::load(&root).unwrap();
        let kind = std::sync::Arc::new(data.types.get("minecraft:zombie").unwrap().clone());
        let mut m = crate::mobs::Mob::new(
            2,
            kind,
            sunrise_core::geometry::Vec3::new(0.0, 4.0, 0.0),
            0.0,
        );
        set_base(&mut m, Attr::AttackDamage, 4.0);
        let id = String::from("test:typed");
        add_modifier(&mut m, Attr::AttackDamage, id.clone(), 3.0, Op::AddValue);
        assert!(has_modifier(&m, Attr::AttackDamage, &id));
        assert_eq!(modifier_amount(&m, Attr::AttackDamage, &id), Some(3.0));
        assert_eq!(m.attrs.attack_damage, 7.0);
        assert_eq!(value(&m, Attr::AttackDamage), 7.0);
        remove_modifier(&mut m, Attr::AttackDamage, &id);
        assert!(!has_modifier(&m, Attr::AttackDamage, &id));
        assert_eq!(m.attrs.attack_damage, 4.0);
    }
    /// #7877: every attribute `/attribute` can name on a mob is edited in the
    /// store the mob reads: base, modifier and removal reach the cached field
    /// gameplay uses, for each attribute the mob types hold.
    #[test]
    fn every_command_attribute_reaches_the_store_the_mob_reads() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            return;
        }
        let data = crate::mobs::MobTypes::load(&root).unwrap();
        let mut covered = 0;
        for a in Attr::ALL {
            let short = a.name();
            assert_eq!(command_attr(short), (a != Attr::Armor).then_some(a));
            if a == Attr::Armor {
                continue;
            }
            // Any type holding the attribute, in a stable order.
            let mut names: Vec<_> = data.types.keys().collect();
            names.sort();
            let Some(kind) = names
                .into_iter()
                .map(|n| &data.types[n])
                .find(|k| k.attributes.contains_key(a.id()))
            else {
                continue;
            };
            let mut m = crate::mobs::Mob::new(
                2,
                std::sync::Arc::new(kind.clone()),
                sunrise_core::geometry::Vec3::new(0.0, 4.0, 0.0),
                0.0,
            );
            let cached = |m: &crate::mobs::Mob| match a {
                Attr::MaxHealth => Some(f64::from(m.attrs.max_health)),
                Attr::MovementSpeed => Some(m.attrs.speed),
                Attr::FollowRange => Some(m.attrs.follow_range),
                Attr::AttackDamage => Some(f64::from(m.attrs.attack_damage)),
                Attr::KnockbackResistance => Some(m.attrs.knockback_resistance),
                Attr::SpawnReinforcements => Some(m.attrs.spawn_reinforcements),
                Attr::JumpStrength => Some(m.attrs.jump_strength),
                _ => None,
            };
            // Max health clamps below 1.
            let base = if a == Attr::MaxHealth { 2.0 } else { 0.5 };
            set_base(&mut m, a, base);
            assert_eq!(base_value(&m, a), base, "{short} base");
            add_modifier(&mut m, a, format!("test:{short}"), 0.25, Op::AddValue);
            assert_eq!(modifier_amount(&m, a, &format!("test:{short}")), Some(0.25));
            assert_eq!(value(&m, a), base + 0.25, "{short} value");
            if let Some(c) = cached(&m) {
                assert_eq!(c, base + 0.25, "{short} cached field");
            }
            remove_modifier(&mut m, a, &format!("test:{short}"));
            assert!(!has_modifier(&m, a, &format!("test:{short}")));
            if let Some(c) = cached(&m) {
                assert_eq!(c, base, "{short} cached field after removal");
            }
            covered += 1;
        }
        assert!(covered >= 7, "only {covered} attributes had a mob type");
    }
    /// #7877: `/attribute <mob> movement_speed|attack_damage base set` goes
    /// through `AttributeInstance.setBaseValue`, so the value survives the
    /// next refresh (an effect's modifier) instead of snapping back.
    #[test]
    fn a_set_base_survives_a_later_modifier_refresh() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            return;
        }
        let data = crate::mobs::MobTypes::load(&root).unwrap();
        let kind = std::sync::Arc::new(data.types.get("minecraft:zombie").unwrap().clone());
        let mut m = crate::mobs::Mob::new(
            2,
            kind,
            sunrise_core::geometry::Vec3::new(0.0, 4.0, 0.0),
            0.0,
        );
        set_base(&mut m, Attr::MovementSpeed, 0.5);
        set_base(&mut m, Attr::AttackDamage, 7.0);
        // Refresh through another attribute's change and a modifier.
        set_base(&mut m, Attr::MaxHealth, 30.0);
        add_modifier(
            &mut m,
            Attr::MovementSpeed,
            "test:x",
            0.5,
            Op::AddMultipliedTotal,
        );
        assert_eq!(base_value(&m, Attr::MovementSpeed), 0.5);
        assert_eq!(m.attrs.speed, 0.75);
        remove_modifier(&mut m, Attr::MovementSpeed, "test:x");
        assert_eq!(m.attrs.speed, 0.5);
        assert_eq!(base_value(&m, Attr::AttackDamage), 7.0);
        assert_eq!(m.attrs.attack_damage, 7.0);
    }
    /// #3887: Weakness and Slowness (and Health Boost) carry attribute
    /// modifiers in vanilla (`MobEffects`), and every effect's amount is
    /// scaled by the amplifier plus one (`MobEffect.addAttributeModifier`);
    /// before this only strength and speed were mapped.
    #[test]
    fn weakness_slowness_and_health_boost_carry_attribute_modifiers() {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        let modifier = |effect: &str, amplifier: i32, attr: Attr| {
            effect_modifier(effect, amplifier, attr).expect("the effect should carry a modifier")
        };
        // MobEffects.WEAKNESS: attack damage -4 per level, ADD_VALUE.
        let (amount, op) = modifier("minecraft:weakness", 0, Attr::AttackDamage);
        assert!(close(amount, -4.0));
        assert_eq!(op, Op::AddValue);
        let (amount, _) = modifier("weakness", 1, Attr::AttackDamage);
        assert!(close(amount, -8.0));
        // MobEffects.SLOWNESS: movement speed -0.15F per level, ADD_MULTIPLIED_TOTAL.
        let (amount, op) = modifier("minecraft:slowness", 0, Attr::MovementSpeed);
        assert!(close(amount, f64::from(-0.15f32)));
        assert_eq!(op, Op::AddMultipliedTotal);
        let (amount, _) = modifier("slowness", 1, Attr::MovementSpeed);
        assert!(close(amount, f64::from(-0.15f32) * 2.0));
        // MobEffects.HEALTH_BOOST: max health +4 per level, ADD_VALUE.
        let (amount, op) = modifier("minecraft:health_boost", 0, Attr::MaxHealth);
        assert!(close(amount, 4.0));
        assert_eq!(op, Op::AddValue);
        // The effects that already worked keep their vanilla amounts.
        let (amount, _) = modifier("strength", 0, Attr::AttackDamage);
        assert!(close(amount, 3.0));
        let (amount, _) = modifier("speed", 0, Attr::MovementSpeed);
        assert!(close(amount, f64::from(0.2f32)));
        // An effect does not touch attributes it does not modify.
        assert!(effect_modifier("weakness", 0, Attr::MovementSpeed).is_none());
        assert!(effect_modifier("poison", 0, Attr::AttackDamage).is_none());
    }
    #[test]
    fn villager_types_by_biome() {
        assert_eq!(villager_type("minecraft:badlands"), 1);
        assert_eq!(villager_type("minecraft:grove"), 4);
        assert_eq!(villager_type("minecraft:plains"), 0);
    }
    /// A worn piece that wears (Thorns' `ChangeItemDamage`) keeps its
    /// enchantments and trim, and the next wear adds to the last.
    #[test]
    fn a_worn_piece_keeps_its_enchantments_and_trim_as_it_wears() {
        let kind = crate::mobs::sensor_scan_tests::kind();
        let mut m = crate::mobs::Mob::new(1, kind, sunrise_core::geometry::Vec3::ZERO, 0.0);
        set_item(&mut m, CHEST, "minecraft:iron_chestplate");
        set_slot_enchantments(&mut m, CHEST, vec![(Arc::from("minecraft:thorns"), 3)]);
        let trim: gear::Trim = ("minecraft:gold".into(), "minecraft:coast".into());
        gear::set_trim(&mut m, gear::CHEST, Some(trim.clone()));
        set_slot_damage(&mut m, CHEST, 2);
        set_slot_damage(&mut m, CHEST, 5);
        let worn = item_stack(&m, CHEST).unwrap();
        assert_eq!(worn.damage, 5);
        assert_eq!(worn.enchantment_level("minecraft:thorns"), 3);
        assert_eq!(worn.data().and_then(|d| d.trim.clone()), Some(trim));
    }
    #[test]
    fn october_31_is_halloween() {
        assert!(halloween_date(10, 31));
        assert!(!halloween_date(10, 30));
        assert!(!halloween_date(11, 1));
        assert!(!halloween_date(1, 1));
    }

    #[test]
    fn equipment_read_matches_stack_with_and_without_gear_overlays() {
        let kind = crate::mobs::sensor_scan_tests::kind();
        let mut m = crate::mobs::Mob::new(1, kind, sunrise_core::geometry::Vec3::ZERO, 0.0);
        set_item(&mut m, CHEST, "minecraft:iron_chestplate");
        set_slot_enchantments(&mut m, CHEST, vec![(Arc::from("minecraft:thorns"), 3)]);
        let trim: gear::Trim = ("minecraft:gold".into(), "minecraft:coast".into());
        gear::set_trim(&mut m, gear::CHEST, Some(trim));
        set_slot_damage(&mut m, CHEST, 2);

        let read = equipment_slot(&m, EquipmentSlot::Chest).unwrap();
        let projected = read.project().into_owned();
        assert!(read.matches_stack(&projected));

        let mut altered = projected.clone();
        altered.damage = 3;
        assert!(!read.matches_stack(&altered));

        let mut unenchanted = ItemStack::new("minecraft:iron_chestplate", 1);
        unenchanted.damage = 2;
        assert!(!read.matches_stack(&unenchanted));
    }

    struct ProviderTestLevel {
        variants: crate::mobs::variants::VariantTables,
        items: Items,
    }
    impl FinalizeLevel for ProviderTestLevel {
        fn biome(&self, _: [i32; 3]) -> String {
            "minecraft:plains".into()
        }
        fn biome_tag(&self, _: &str, _: [i32; 3]) -> bool {
            false
        }
        fn structures_with_piece_at(&self, _: [i32; 3]) -> Vec<String> {
            Vec::new()
        }
        fn moon_phase(&self, _: [i32; 3]) -> usize {
            0
        }
        fn spawning_monsters(&self) -> bool {
            true
        }
        fn no_collision(&self, _: Aabb) -> bool {
            true
        }
        fn free_chicken(&self, _: Aabb) -> Option<u32> {
            None
        }
        fn ride_chicken(&mut self, _: u32, _: u32) {}
        fn game_time(&self) -> i64 {
            0
        }
        fn halloween(&self) -> bool {
            false
        }
        fn create(&mut self, _: &str, _: Reason) -> Option<Mob> {
            None
        }
        fn spawn_data(&self) -> SpawnData<'_> {
            SpawnData {
                variants: &self.variants,
                enchantments: &self.items.enchantments,
                items: &self.items,
                camel_husk_size: (1.7, 2.375),
            }
        }
    }

    fn provider_test_level(files: &[(String, Vec<Vec<u8>>)]) -> ProviderTestLevel {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("items.json"),
            serde_json::json!({
                "minecraft:iron_sword": {},
                "minecraft:iron_shovel": {},
                "minecraft:crossbow": {},
            })
            .to_string(),
        )
        .unwrap();
        let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
        let overlay = sunrise_world::loot::PackOverlay::new(files);
        let (packed, admitted) = crate::lan::data::SyncData::stage_registry_definitions(
            root.path(),
            &defaults,
            &overlay,
            &Default::default(),
            &Default::default(),
            |_| true,
        );
        admitted.unwrap();
        let mut items = Items::tags_only(root.path(), &overlay, &defaults).unwrap();
        packed
            .install_items(&mut items, root.path(), &overlay, &defaults)
            .unwrap();
        ProviderTestLevel {
            variants: Default::default(),
            items,
        }
    }

    #[test]
    fn mob_spawn_equipment_provider_override_enchants_zombie_sword_with_pack_sharpness() {
        let files = [
            (
                "enchantment/minecraft:sharpness".into(),
                vec![
                    serde_json::json!({
                        "description": "Sharpness",
                        "weight": 10,
                        "max_level": 5,
                        "anvil_cost": 1,
                        "min_cost": {"base": 1, "per_level_above_first": 11},
                        "max_cost": {"base": 21, "per_level_above_first": 11},
                        "supported_items": "minecraft:iron_sword",
                        "slots": ["mainhand"]
                    })
                    .to_string()
                    .into_bytes(),
                ],
            ),
            (
                "enchantment_provider/minecraft:mob_spawn_equipment".into(),
                vec![
                    serde_json::json!({
                        "type": "minecraft:single",
                        "enchantment": "minecraft:sharpness",
                        "level": 5
                    })
                    .to_string()
                    .into_bytes(),
                ],
            ),
        ];
        let mut level = provider_test_level(&files);
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.species = Species::Zombie;
        let kind = Arc::new(kind);
        let difficulty = LocalDifficulty {
            base: 3,
            effective: 6.75,
            normal: false,
            hard: true,
            id: 3,
        };
        let mut found = false;
        for seed in 0..5000_i64 {
            let mut m = Mob::new(
                1,
                Arc::clone(&kind),
                sunrise_core::geometry::Vec3::ZERO,
                0.0,
            );
            let mut rng = sunrise_core::random::LegacyRandom::new(seed);
            let mut cx = Ctx {
                level: &mut level,
                random: &mut rng,
                difficulty,
                created: Vec::new(),
                jockey_rider: None,
            };
            finalize(&mut cx, &mut m, Reason::Natural, None);
            if item(&m, MAINHAND).as_deref() == Some("minecraft:iron_sword")
                && gear::enchanted(&m, gear::MAIN)
            {
                assert_eq!(gear::level(&m, gear::MAIN, "minecraft:sharpness"), 5);
                found = true;
                break;
            }
        }
        assert!(
            found,
            "expected a seeded zombie with an enchanted iron sword"
        );
    }

    #[test]
    fn pillager_spawn_crossbow_provider_override_changes_spawned_pillager_crossbow() {
        let files = [
            (
                "enchantment/minecraft:multishot".into(),
                vec![
                    serde_json::json!({
                        "description": "Multishot",
                        "weight": 2,
                        "max_level": 1,
                        "anvil_cost": 4,
                        "min_cost": {"base": 20, "per_level_above_first": 0},
                        "max_cost": {"base": 50, "per_level_above_first": 0},
                        "supported_items": "minecraft:crossbow",
                        "slots": ["mainhand"]
                    })
                    .to_string()
                    .into_bytes(),
                ],
            ),
            (
                "enchantment_provider/minecraft:pillager_spawn_crossbow".into(),
                vec![
                    serde_json::json!({
                        "type": "minecraft:single",
                        "enchantment": "minecraft:multishot",
                        "level": 1
                    })
                    .to_string()
                    .into_bytes(),
                ],
            ),
        ];
        let mut level = provider_test_level(&files);
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.species = Species::Pillager;
        let kind = Arc::new(kind);
        // special == 0.0 so only the 1-in-300 `pillager_spawn_crossbow` roll fires.
        let difficulty = LocalDifficulty {
            base: 2,
            effective: 1.0,
            normal: true,
            hard: false,
            id: 2,
        };
        let mut found = false;
        for seed in 0..5000_i64 {
            let mut m = Mob::new(
                1,
                Arc::clone(&kind),
                sunrise_core::geometry::Vec3::ZERO,
                0.0,
            );
            let mut rng = sunrise_core::random::LegacyRandom::new(seed);
            let mut cx = Ctx {
                level: &mut level,
                random: &mut rng,
                difficulty,
                created: Vec::new(),
                jockey_rider: None,
            };
            finalize(&mut cx, &mut m, Reason::Natural, None);
            if gear::enchanted(&m, gear::MAIN) {
                assert_eq!(gear::level(&m, gear::MAIN, "minecraft:multishot"), 1);
                assert_eq!(gear::level(&m, gear::MAIN, "minecraft:piercing"), 0);
                found = true;
                break;
            }
        }
        assert!(
            found,
            "expected a seeded pillager to hit the 1-in-300 spawn crossbow roll"
        );
    }
}

#[cfg(test)]
#[path = "finalize_load_tests.rs"]
mod load_tests;
