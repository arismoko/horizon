//! Sulfur cubes (`SulfurCube` on `AbstractCubeMob`): hopping like a slime
//! but harmless, size 1 babies growing into size 2 adults, swallowing a
//! block (by hand, from the ground or a dispenser) that stops it in place
//! and gives it the block's archetype (`SulfurCubeArchetype`: buoyancy,
//! knockback, sounds, contact damage, a TNT fuse), shearing the block out,
//! buckets, and the outer/inner models and their render scale.
use super::{
    Damage, DamageKind, Mob, MobEvent, MobWorld, NamedDamage,
    control::MoveKind,
    goals::{Cx, Goal, JUMP, LOOK, MOVE, goal_clone},
};
use crate::health::DamageLevel;
use std::cell::Cell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use sunrise_core::{geometry::Vec3, random::RandomSource};
use sunrise_world::attributes::{Modifier, Operation};
use sunrise_world::worldgen::value_provider::FloatProvider;

// Sulfur cube state (on top of `cube`'s `size`, `counter` jump delay,
// `flag` aggressive, `flag2` was on ground, `vector[0]` direction):
// `head` the swallowed block, `sulfur_fuse` the fuse and its synced `MAX_FUSE`,
// `ints[3]` the pickup timer, `scratch[1]` the sounds pending (1 a fuse just
// lit),
// `scratch[2]` the push sound cooldown, `scratch[3]` from a bucket,
// `point` the item it is after.
pub const PICKUP_TIMER: i32 = 100;

/// One entry of `SulfurCubeArchetype.attributeModifiers`.
#[derive(Clone, Debug)]
pub struct AttributeEntry {
    pub attribute: String,
    pub id: String,
    pub amount: f64,
    pub operation: Operation,
}
/// `SulfurCubeArchetype.ContactDamage`: the damage type, its amount (a float provider sampled
/// per hit) and whether a hit on a non-player is attributed to the cube.
#[derive(Clone, Debug)]
pub struct ContactDamage {
    pub amount: FloatProvider,
    pub attribute_to_source: bool,
    pub damage_type: &'static str,
}
/// One `SulfurCubeArchetype` of the world's registry (the parts Sunrise's physics can use).
#[derive(Clone, Debug)]
pub struct Archetype {
    /// The registry id (`minecraft:explosive`).
    pub id: String,
    /// `items`, resolved against the world's item tags.
    pub items: HashSet<String>,
    pub modifiers: Vec<AttributeEntry>,
    pub buoyant: bool,
    /// `explosion`: fuse ticks, power, causes fire.
    pub explosion: Option<(i32, f32, bool)>,
    /// `contact_damage`.
    pub contact_damage: Option<ContactDamage>,
    /// The knockback resistance attribute with the modifiers applied to a base of 0 (the
    /// attribute range is -2..1; negative values add knockback).
    pub knockback_resistance: f64,
    /// The explosion knockback resistance modifier (its own entry in the
    /// archetype's `attribute_modifiers`).
    pub explosion_knockback_resistance: f32,
    /// `Attributes.BOUNCINESS` (the modifiers applied to a base of 0).
    pub bounciness: f32,
    /// `Attributes.FRICTION_MODIFIER` and `AIR_DRAG_MODIFIER` (the modifiers applied to a base of 1).
    pub friction_modifier: f32,
    pub air_drag_modifier: f32,
    pub horizontal_power: f32,
    pub vertical_power: f32,
    pub hit_sound: &'static str,
    pub push_sound: &'static str,
    pub push_cooldown: f32,
    pub push_threshold: f32,
}

/// The `items` holder set of an archetype file: a tag, or the listed items.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemSet {
    Tag(String),
    Items(Vec<String>),
}

/// An archetype file after `SulfurCubeArchetype.DIRECT_CODEC`, before its items are resolved.
#[derive(Clone, Debug)]
pub struct Decoded {
    pub items: ItemSet,
    archetype: Archetype,
}

/// A sound or damage type name kept for the life of the process, because the mob events carry
/// `&'static str`. Equal names share one allocation, so a reload leaks nothing new.
fn intern(name: &str) -> &'static str {
    static POOL: Mutex<BTreeSet<&'static str>> = Mutex::new(BTreeSet::new());
    let mut pool = POOL.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(&known) = pool.get(name) {
        return known;
    }
    let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
    pool.insert(leaked);
    leaked
}

impl Decoded {
    /// `SulfurCubeArchetype.DIRECT_CODEC` over the file: any missing or mistyped field, or a
    /// sound `sound_exists` refuses, is an error.
    pub fn decode(
        id: &str,
        value: &serde_json::Value,
        sound_exists: &dyn Fn(&str) -> bool,
    ) -> Result<Self, String> {
        use serde_json::Value;
        let fields = value.as_object().ok_or("not a map")?;
        let field = |name: &str| fields.get(name).ok_or(format!("{name} is required"));
        let object = |value: &'_ Value, what: &str| {
            value
                .as_object()
                .cloned()
                .ok_or(format!("{what}: not a map"))
        };
        let number = |value: &Value, what: &str| {
            value
                .as_f64()
                .ok_or(format!("{what}: expected a number, got {value}"))
        };
        let boolean = |value: &Value, what: &str| {
            value
                .as_bool()
                .ok_or(format!("{what}: expected a boolean, got {value}"))
        };
        let identifier = |value: &Value, what: &str| -> Result<String, String> {
            let text = value.as_str().ok_or(format!("{what}: not an identifier"))?;
            sunrise_core::identifier::Identifier::parse(text)
                .map(|id| id.to_string())
                .map_err(|error| format!("{what}: {error}"))
        };
        // `RegistryCodecs.holderSet`: `#tag`, one id, or a list of ids.
        let items = match field("items")? {
            Value::String(text) => match text.strip_prefix('#') {
                Some(tag) => ItemSet::Tag(identifier(&Value::String(tag.into()), "items")?),
                None => ItemSet::Items(vec![identifier(field("items")?, "items")?]),
            },
            Value::Array(list) => ItemSet::Items(
                list.iter()
                    .map(|item| identifier(item, "items"))
                    .collect::<Result<_, _>>()?,
            ),
            other => return Err(format!("items: not a holder set: {other}")),
        };
        let mut modifiers = Vec::new();
        for (index, entry) in field("attribute_modifiers")?
            .as_array()
            .ok_or("attribute_modifiers: not a list")?
            .iter()
            .enumerate()
        {
            let what = format!("attribute_modifiers[{index}]");
            let entry = object(entry, &what)?;
            let get = |name: &str| entry.get(name).ok_or(format!("{what}.{name} is required"));
            let operation = match get("operation")?.as_str() {
                Some("add_value") => Operation::AddValue,
                Some("add_multiplied_base") => Operation::AddMultipliedBase,
                Some("add_multiplied_total") => Operation::AddMultipliedTotal,
                _ => return Err(format!("{what}.operation: unknown operation")),
            };
            modifiers.push(AttributeEntry {
                attribute: identifier(get("attribute")?, &format!("{what}.attribute"))?,
                id: identifier(get("id")?, &format!("{what}.id"))?,
                amount: number(get("amount")?, &format!("{what}.amount"))?,
                operation,
            });
        }
        let buoyant = fields
            .get("buoyant")
            .map(|value| boolean(value, "buoyant"))
            .transpose()?
            .unwrap_or(false);
        let explosion = fields
            .get("explosion")
            .map(|value| -> Result<_, String> {
                let data = object(value, "explosion")?;
                let get = |name: &str| {
                    data.get(name)
                        .ok_or(format!("explosion.{name} is required"))
                };
                let power = get("power")?
                    .as_i64()
                    .filter(|p| (0..=i64::from(i32::MAX)).contains(p));
                let fuse = get("fuse")?
                    .as_i64()
                    .filter(|f| (1..=i64::from(i32::MAX)).contains(f));
                Ok((
                    fuse.ok_or("explosion.fuse: expected a positive int")? as i32,
                    power.ok_or("explosion.power: expected a non-negative int")? as f32,
                    boolean(get("causes_fire")?, "explosion.causes_fire")?,
                ))
            })
            .transpose()?;
        let contact_damage = fields
            .get("contact_damage")
            .map(|value| -> Result<_, String> {
                let data = object(value, "contact_damage")?;
                let get = |name: &str| {
                    data.get(name)
                        .ok_or(format!("contact_damage.{name} is required"))
                };
                // `FloatProviders.codec(0.0F)`: any provider whose minimum is at least 0.
                let amount = FloatProvider::from_json(&get("amount")?.to_string())
                    .map_err(|error| format!("contact_damage.amount: {error}"))?;
                if amount.bounds()[0] < 0.0 {
                    return Err("contact_damage.amount: must be at least 0".into());
                }
                let attribute_to_source = boolean(
                    get("attribute_to_source")?,
                    "contact_damage.attribute_to_source",
                )?;
                Ok(ContactDamage {
                    amount,
                    attribute_to_source,
                    damage_type: intern(&identifier(
                        get("damage_type")?,
                        "contact_damage.damage_type",
                    )?),
                })
            })
            .transpose()?;
        let knockback = object(field("knockback_modifiers")?, "knockback_modifiers")?;
        let power = |name: &str| {
            number(
                knockback
                    .get(name)
                    .ok_or(format!("knockback_modifiers.{name} is required"))?,
                &format!("knockback_modifiers.{name}"),
            )
        };
        let (horizontal_power, vertical_power) = (
            power("horizontal_power")? as f32,
            power("vertical_power")? as f32,
        );
        let sounds = object(field("sound_settings")?, "sound_settings")?;
        let sound = |name: &str| -> Result<&'static str, String> {
            let what = format!("sound_settings.{name}");
            let value = sounds.get(name).ok_or(format!("{what} is required"))?;
            // `SoundEvent.CODEC`: an id of the registry, or an inline event.
            let id = match value {
                Value::Object(inline) => inline.get("sound_id").and_then(Value::as_str),
                other => other.as_str(),
            };
            let id = identifier(
                &Value::String(id.ok_or(format!("{what}: not a sound event"))?.into()),
                &what,
            )?;
            let bare = id.strip_prefix("minecraft:").unwrap_or(&id);
            if !matches!(value, Value::Object(_)) && !sound_exists(&id) {
                return Err(format!("{what}: unknown holder {id}"));
            }
            Ok(intern(bare))
        };
        let (hit_sound, push_sound) = (sound("hit_sound")?, sound("push_sound")?);
        let sound_number = |name: &str| {
            number(
                sounds
                    .get(name)
                    .ok_or(format!("sound_settings.{name} is required"))?,
                &format!("sound_settings.{name}"),
            )
        };
        let (push_threshold, push_cooldown) = (
            sound_number("push_sound_impulse_threshold")? as f32,
            sound_number("push_sound_cooldown")? as f32,
        );
        let applied = |attribute: &str, base: f64| {
            let of = |operation| {
                modifiers
                    .iter()
                    .filter(move |m| m.attribute == attribute && m.operation == operation)
            };
            let mut total = base + of(Operation::AddValue).map(|m| m.amount).sum::<f64>();
            let after_adds = total;
            total += of(Operation::AddMultipliedBase)
                .map(|m| after_adds * m.amount)
                .sum::<f64>();
            for modifier in of(Operation::AddMultipliedTotal) {
                total *= 1.0 + modifier.amount;
            }
            total
        };
        let archetype = Archetype {
            id: id.to_owned(),
            items: HashSet::new(),
            buoyant,
            explosion,
            contact_damage,
            knockback_resistance: applied("minecraft:knockback_resistance", 0.0),
            explosion_knockback_resistance: applied("minecraft:explosion_knockback_resistance", 0.0)
                as f32,
            bounciness: applied("minecraft:bounciness", 0.0) as f32,
            friction_modifier: applied("minecraft:friction_modifier", 1.0) as f32,
            air_drag_modifier: applied("minecraft:air_drag_modifier", 1.0) as f32,
            horizontal_power,
            vertical_power,
            hit_sound,
            push_sound,
            push_cooldown,
            push_threshold,
            modifiers,
        };
        Ok(Self { items, archetype })
    }

    /// The archetype with its `items` holder set resolved against the world's item tags
    /// (`HolderSet.Named` of a tag the world does not define holds nothing).
    pub fn resolve(self, item_tags: &HashMap<String, HashSet<String>>) -> Archetype {
        let items = match self.items {
            ItemSet::Tag(tag) => item_tags.get(&tag).cloned().unwrap_or_default(),
            ItemSet::Items(items) => items.into_iter().collect(),
        };
        Archetype {
            items,
            ..self.archetype
        }
    }
}

/// `SulfurCubeArchetype.DEFAULT_KNOCKBACK_MODIFIERS`.
const DEFAULT_KNOCKBACK: (f32, f32) = (0.33, 0.06);
/// Hit sound from `SulfurCubeArchetype.DEFAULT_SOUND_SETTINGS`.
const DEFAULT_HIT_SOUND: &str = "entity.sulfur_cube.regular.hit";
/// `SulfurCubeArchetype.DEFAULT_SOUND_SETTINGS`: push settings without a matching archetype.
const DEFAULT_PUSH: (&str, f32, f32) = ("entity.sulfur_cube.regular.push", 0.5, 0.2);
/// The world's archetypes in registry order, as [`install`] last published them.
static TABLE: RwLock<Option<Arc<Vec<Arc<Archetype>>>>> = RwLock::new(None);

/// Decode every `sulfur_cube_archetype` entry of the world (registry order) and resolve its
/// `items` against `item_tags`. The entries were validated when the world's registries were
/// staged, so one that no longer decodes is skipped.
pub fn build(
    entries: &[(String, &serde_json::Value)],
    item_tags: &HashMap<String, HashSet<String>>,
) -> Vec<Arc<Archetype>> {
    entries
        .iter()
        .filter_map(|(id, value)| Decoded::decode(id, value, &|_| true).ok())
        .map(|decoded| Arc::new(decoded.resolve(item_tags)))
        .collect()
}

/// Publish the open world's archetypes, replacing the last world's.
pub fn install(archetypes: Vec<Arc<Archetype>>) {
    *TABLE.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(archetypes));
}

#[cfg(test)]
thread_local! {
    /// The table a unit fixture published for its own test thread, read before [`TABLE`].
    static TEST_TABLE: std::cell::RefCell<Option<Arc<Vec<Arc<Archetype>>>>> =
        const { std::cell::RefCell::new(None) };
}

fn installed() -> Option<Arc<Vec<Arc<Archetype>>>> {
    #[cfg(test)]
    if let Some(table) = TEST_TABLE.with(|t| t.borrow().clone()) {
        return Some(table);
    }
    TABLE.read().unwrap_or_else(PoisonError::into_inner).clone()
}

/// The archetypes the unit fixtures run on, published for the calling test's thread only (the
/// process-wide [`TABLE`] is left alone): the explosive and hot bodies, and two of the
/// physics-only ones.
#[cfg(test)]
fn test_archetype_items() {
    use serde_json::json;
    let archetype = |name: &str,
                     extra: serde_json::Value,
                     physics: [f64; 6],
                     power: [f64; 2],
                     sound: [f64; 2]| {
        // The multiplier amounts are the files' doubles: a float constant widened, minus one.
        let [kb, ek, bounce, friction, drag, _] = physics;
        let mut value = json!({
            "items": format!("#minecraft:sulfur_cube_archetype/{name}"),
            "attribute_modifiers": [
                {"attribute": "minecraft:knockback_resistance", "id": format!("minecraft:{name}_add_knockback_resistance"), "amount": kb, "operation": "add_value"},
                {"attribute": "minecraft:explosion_knockback_resistance", "id": format!("minecraft:{name}_add_explosion_knockback_resistance"), "amount": ek, "operation": "add_value"},
                {"attribute": "minecraft:bounciness", "id": format!("minecraft:{name}_add_bounciness"), "amount": bounce, "operation": "add_value"},
                {"attribute": "minecraft:friction_modifier", "id": format!("minecraft:{name}_mul_friction_modifier"), "amount": f64::from(friction as f32) - 1.0, "operation": "add_multiplied_total"},
                {"attribute": "minecraft:air_drag_modifier", "id": format!("minecraft:{name}_mul_air_drag_modifier"), "amount": f64::from(drag as f32) - 1.0, "operation": "add_multiplied_total"},
            ],
            "buoyant": physics[5] != 0.0,
            "knockback_modifiers": {"horizontal_power": power[0], "vertical_power": power[1]},
            "sound_settings": {
                "hit_sound": format!("minecraft:entity.sulfur_cube.{name}.hit"),
                "push_sound": format!("minecraft:entity.sulfur_cube.{name}.push"),
                "push_sound_impulse_threshold": sound[0],
                "push_sound_cooldown": sound[1],
            },
        });
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        (format!("minecraft:{name}"), value)
    };
    let table = [
        archetype(
            "explosive",
            json!({"explosion": {"power": 3, "causes_fire": false, "fuse": 120}}),
            [-1.0, -1.0, 0.5, 0.3, 0.3, 1.0],
            [0.4125, 0.09],
            [0.1, 0.7],
        ),
        archetype(
            "high_resistance",
            json!({}),
            [0.7, 0.7, 0.2, 1.0, 0.01, 0.0],
            [0.4125, 0.09],
            [0.03, 0.7],
        ),
        archetype(
            "hot",
            json!({"contact_damage": {"amount": 1.0, "attribute_to_source": false, "damage_type": "minecraft:sulfur_cube_hot"}}),
            [-1.0, -1.0, 0.5, 0.3, 0.1, 1.0],
            [0.4125, 0.09],
            [0.2, 0.7],
        ),
        archetype(
            "slow_bouncy",
            json!({}),
            [0.4, 0.4, 0.6, 0.3, 0.05, 0.0],
            [0.4125, 0.24],
            [0.05, 0.5],
        ),
    ];
    let entries: Vec<_> = table
        .iter()
        .map(|(id, value)| (id.clone(), value))
        .collect();
    let tags = HashMap::from([
        (
            "minecraft:sulfur_cube_archetype/explosive".to_owned(),
            HashSet::from(["minecraft:tnt".to_owned()]),
        ),
        (
            "minecraft:sulfur_cube_archetype/hot".to_owned(),
            HashSet::from(["minecraft:magma_block".to_owned()]),
        ),
    ]);
    let built = Arc::new(build(&entries, &tags));
    TEST_TABLE.with(|t| *t.borrow_mut() = Some(built));
}
/// `matchingArchetypes`: the last one wins (as `collectEquipmentChanges`
/// leaves it), buoyancy if any.
pub fn archetype_of(item: &str) -> Option<Arc<Archetype>> {
    installed()?
        .iter()
        .rev()
        .find(|a| a.items.contains(item))
        .cloned()
}
/// `matchingArchetypes`: every archetype whose items hold the item, in
/// registry order.
fn matching(item: &str) -> Vec<Arc<Archetype>> {
    installed().map_or_else(Vec::new, |table| {
        table
            .iter()
            .filter(|a| a.items.contains(item))
            .cloned()
            .collect()
    })
}
/// `SulfurCubeArchetype.attributeModifiers`: the entries of the archetype's file.
pub fn attribute_modifiers(a: &Archetype) -> &[AttributeEntry] {
    &a.modifiers
}
/// The attributes `attribute_modifiers` touch, routed to the cube's own map.
pub const ARCHETYPE_ATTRIBUTES: [&str; 5] = [
    "minecraft:knockback_resistance",
    "minecraft:explosion_knockback_resistance",
    "minecraft:bounciness",
    "minecraft:friction_modifier",
    "minecraft:air_drag_modifier",
];
/// `SulfurCube.collectEquipmentChanges` (from `LivingEntity.tick`'s
/// `detectEquipmentUpdates`), its attribute part: when the body changed,
/// removes the previous body's archetype modifiers and adds the new body's
/// as transient ones (`addOrUpdateTransientModifier`).
pub fn adopt_body(m: &mut Mob) {
    if m.species() != super::Species::SulfurCube
        || m.extra.adopted_body.as_deref() == m.extra.body_id()
    {
        return;
    }
    let new = m.extra.body_id().map(std::sync::Arc::<str>::from);
    let Some(map) = m.extra.generic_attributes.as_deref_mut() else {
        return;
    };
    let old = std::mem::replace(&mut m.extra.adopted_body, new.clone());
    for a in old.as_deref().into_iter().flat_map(matching) {
        for entry in attribute_modifiers(&a) {
            if let Some(key) = map.catalog().id(&entry.attribute) {
                let _ = map.edit(key, |inst, observer| inst.remove(&entry.id, observer));
            }
        }
    }
    for a in new.as_deref().into_iter().flat_map(matching) {
        for entry in attribute_modifiers(&a) {
            let (Some(key), Ok(modifier)) = (
                map.catalog().id(&entry.attribute),
                Modifier::new(&entry.id, entry.amount, entry.operation),
            ) else {
                continue;
            };
            let _ = map.edit(key, |inst, observer| {
                inst.add_or_update_transient(modifier, observer)
            });
        }
    }
}
/// `isSwallowableItem`: the `sulfur_cube_swallowable` tag.
pub fn swallowable(m: &Mob, item: &str) -> bool {
    m.kind.tempt.iter().any(|t| t == item)
}
pub fn has_body(m: &Mob) -> bool {
    m.extra.body_id().is_some()
}

#[cfg(test)]
mod knockback_sync_tests {
    use super::*;
    use crate::mobs::{Mob, Species};
    use std::sync::Arc;

    /// `SulfurCube.knockback` sets `needsSync`, as `LivingEntity.knockback`
    /// does, so a guest sees a cube with a block knocked about at once.
    #[test]
    fn a_knocked_cube_with_a_block_syncs_at_once() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        test_archetype_items();
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.species = Species::SulfurCube;
        let mut m = Mob::new(1, Arc::new(kind), Vec3::ZERO, 0.0);
        m.extra.set_body(Some("minecraft:tnt".into()));
        assert!(!m.needs_sync);
        m.damage(level, 5.0, Damage::player([2.0, 0.0, 0.0]));
        assert!(m.velocity.x < 0.0, "velocity {:?}", m.velocity);
        assert!(m.needs_sync);
    }
    /// `TemptGoal` scales its range by the player's visibility: an invisible
    /// player five blocks away does not tempt a sulfur cube.
    #[test]
    #[ignore = "daily"]
    fn an_invisible_player_tempts_a_sulfur_cube_only_within_two_blocks() {
        use crate::mobs::goals::Cx;
        use crate::mobs::world::{test_player, with_test_world};
        let mut outcomes = Vec::new();
        if !with_test_world(&[], |w, _| {
            for invisible in [false, true] {
                let mut world = w.clone();
                let mut p = test_player(Vec3::new(5.5, 64.0, 0.5));
                p.main_hand = Some("minecraft:iron_ingot".into());
                p.invisible = invisible;
                world.player = Some(p);
                let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
                kind.species = Species::SulfurCube;
                kind.tempt = Arc::from(vec!["minecraft:iron_ingot".to_string()]);
                let mut m = Mob::new(1, Arc::new(kind), Vec3::new(0.5, 64.0, 0.5), 0.0);
                m.ai.goals.add(1, SulfurTempt { player: None });
                let mut lr = crate::health::TestLevel::new(0);
                let mut events = Vec::new();
                let mut cx = Cx {
                    w: &world,
                    level: &mut lr.level(),
                    events: &mut events,
                };
                let mut goals = std::mem::take(&mut m.ai.goals);
                goals.tick(&mut m, &mut cx);
                outcomes.push(goals.is_running("SulfurTempt"));
                m.ai.goals = goals;
            }
        }) {
            return;
        }
        assert_eq!(
            outcomes,
            vec![true, false],
            "a visible player tempts at five; an invisible one does not"
        );
    }
}
/// Added controls (H192): the attribute part of `collectEquipmentChanges` on
/// the cube's own map, with the amounts and ids of the pinned 26.3
/// `sulfur_cube_archetype/{explosive,hot}.json`.
#[cfg(test)]
pub(crate) mod adopt_body_tests {
    use super::*;
    use crate::mobs::{Mob, Species};
    use std::sync::Arc;
    use sunrise_world::attributes::entity::{Catalog, EntityAttributes};

    const KR: &str = "minecraft:knockback_resistance";
    const FRICTION: &str = "minecraft:friction_modifier";
    const AIR: &str = "minecraft:air_drag_modifier";

    pub(crate) fn cube(body: Option<&str>) -> Mob {
        test_archetype_items();
        let bits = |v: f64| format!("{:016x}", v.to_bits());
        let attribute = |id: &str, default: f64, min: f64, max: f64| serde_json::json!({"id": id, "default": bits(default), "min": bits(min), "max": bits(max), "syncable": id != KR});
        let catalog = Arc::new(
            Catalog::from_json(&serde_json::json!({
                "data_version": sunrise_world::DATA_VERSION,
                "attributes": [
                    attribute(KR, 0.0, -2.0, 1.0),
                    attribute("minecraft:explosion_knockback_resistance", 0.0, 0.0, 1.0),
                    attribute("minecraft:bounciness", 0.0, 0.0, 1.0),
                    attribute(FRICTION, 1.0, 0.0, 2048.0),
                    attribute(AIR, 1.0, 0.0, 2048.0),
                ],
                "entities": {"minecraft:sulfur_cube": {
                    KR: bits(0.0),
                    "minecraft:explosion_knockback_resistance": bits(0.0),
                    "minecraft:bounciness": bits(0.0),
                    FRICTION: bits(1.0),
                    AIR: bits(1.0),
                }}
            }))
            .unwrap(),
        );
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.species = Species::SulfurCube;
        kind.id = "minecraft:sulfur_cube".into();
        let mut m = Mob::new(1, Arc::new(kind), Vec3::ZERO, 0.0);
        m.extra.set_body(body.map(Into::into));
        m.extra.generic_attributes =
            Some(Box::new(catalog.create("minecraft:sulfur_cube").unwrap()));
        m
    }
    fn map(m: &mut Mob) -> &mut EntityAttributes {
        m.extra.generic_attributes.as_deref_mut().unwrap()
    }
    fn value(m: &mut Mob, attribute: &str) -> f64 {
        let map = map(m);
        let key = map.catalog().id(attribute).unwrap();
        map.value(key).unwrap()
    }
    fn permanent(m: &mut Mob, attribute: &str) -> Vec<(String, f64)> {
        let map = map(m);
        let key = map.catalog().id(attribute).unwrap();
        map.instance(key)
            .map(|i| {
                i.permanent_modifiers()
                    .iter()
                    .map(|x| (x.id().to_string(), x.amount()))
                    .collect()
            })
            .unwrap_or_default()
    }
    fn modifier(m: &mut Mob, attribute: &str, id: &str) -> Option<f64> {
        let map = map(m);
        let key = map.catalog().id(attribute).unwrap();
        map.modifier_value(key, id).ok()
    }

    #[test]
    fn a_body_change_swaps_the_archetype_modifiers_and_its_ids_shadow_then_drop_a_permanent_one() {
        let mut m = cube(Some("minecraft:tnt"));
        // A permanent modifier with the explosive archetype's own id, before adoption.
        let key = map(&mut m).catalog().id(KR).unwrap();
        let permanent_one = Modifier::new(
            "minecraft:explosive_add_knockback_resistance",
            0.25,
            Operation::AddValue,
        )
        .unwrap();
        map(&mut m).edit(key, |i, o| i.add_permanent(permanent_one, o).unwrap());
        assert_eq!(value(&mut m, KR), 0.25);

        adopt_body(&mut m);
        // addOrUpdateTransientModifier: the archetype's -1 shadows the permanent
        // 0.25, which stays saved.
        assert_eq!(value(&mut m, KR), -1.0);
        assert_eq!(
            permanent(&mut m, KR),
            [(
                "minecraft:explosive_add_knockback_resistance".to_string(),
                0.25
            )]
        );
        assert_eq!(value(&mut m, FRICTION), 1.0 + -0.699999988079071);
        // Nothing changes without a new body.
        adopt_body(&mut m);
        assert_eq!(value(&mut m, KR), -1.0);

        m.extra.set_body(Some("minecraft:magma_block".into()));
        adopt_body(&mut m);
        // removeModifier drops the transient and the permanent entry of that id.
        assert_eq!(permanent(&mut m, KR), []);
        assert_eq!(
            modifier(
                &mut m,
                FRICTION,
                "minecraft:explosive_mul_friction_modifier"
            ),
            None
        );
        assert_eq!(
            modifier(&mut m, AIR, "minecraft:hot_mul_air_drag_modifier"),
            Some(-0.8999999985098839)
        );
        assert_eq!(value(&mut m, KR), -1.0);

        m.extra.set_body(None);
        adopt_body(&mut m);
        assert_eq!(
            modifier(&mut m, AIR, "minecraft:hot_mul_air_drag_modifier"),
            None
        );
        for (attribute, default) in [
            (KR, 0.0),
            (FRICTION, 1.0),
            (AIR, 1.0),
            ("minecraft:bounciness", 0.0),
        ] {
            assert_eq!(value(&mut m, attribute), default, "{attribute}");
        }
    }

    #[test]
    fn the_fixture_table_is_visible_to_its_own_thread_only() {
        test_archetype_items();
        assert!(archetype_of("minecraft:tnt").is_some());
        let elsewhere = std::thread::spawn(|| archetype_of("minecraft:tnt").is_some());
        assert!(!elsewhere.join().unwrap());
    }

    #[test]
    fn the_modifier_amounts_are_the_archetype_files_doubles() {
        test_archetype_items();
        let table = installed().unwrap();
        let by_name = |n: &str| {
            table
                .iter()
                .find(|a| a.id == format!("minecraft:{n}"))
                .unwrap()
        };
        let slow_bouncy = attribute_modifiers(by_name("slow_bouncy"));
        assert_eq!(
            slow_bouncy[0].id,
            "minecraft:slow_bouncy_add_knockback_resistance"
        );
        assert_eq!(slow_bouncy[0].amount, 0.4);
        let high = attribute_modifiers(by_name("high_resistance"));
        assert_eq!(high[0].amount, 0.7);
        assert_eq!(high[1].amount, 0.7);
    }

    #[test]
    fn a_cube_without_a_map_or_of_another_species_adopts_nothing() {
        let mut m = cube(Some("minecraft:tnt"));
        m.extra.generic_attributes = None;
        adopt_body(&mut m);
        assert_eq!(m.extra.adopted_body, None);
        let mut other = cube(Some("minecraft:tnt"));
        Arc::make_mut(&mut other.kind).species = Species::Slime;
        adopt_body(&mut other);
        assert_eq!(value(&mut other, KR), 0.0);
    }
}
pub fn archetype(m: &Mob) -> Option<Arc<Archetype>> {
    m.extra.body_id().and_then(archetype_of)
}
/// The explosion data `SulfurCube.explosionData` caches: the adopted body's,
/// as `collectEquipmentChanges` last took it.
pub fn adopted_explosion(m: &Mob) -> Option<(i32, f32, bool)> {
    m.extra
        .adopted_body
        .as_deref()
        .and_then(archetype_of)
        .and_then(|a| a.explosion)
}
/// The archetype whose bounciness, friction and air drag attributes a
/// sulfur cube's physics use (no other mob has those modifiers).
pub fn physics(m: &Mob) -> Option<Arc<Archetype>> {
    if m.species() == super::Species::SulfurCube {
        archetype(m)
    } else {
        None
    }
}
/// `SulfurCube.fuse` (the countdown) and `MAX_FUSE` (the synced value it was last set from), each
/// the full signed `int` vanilla stores; -1, unprimed, by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fuse {
    pub fuse: i32,
    pub max: i32,
}
impl Default for Fuse {
    fn default() -> Self {
        Self { fuse: -1, max: -1 }
    }
}
impl Fuse {
    /// `setFuse` and `entityData.set(MAX_FUSE, ...)` together, as the load and `primeTime` do.
    pub fn set(value: i32) -> Self {
        Self {
            fuse: value,
            max: value,
        }
    }
    /// `tickFuse`'s countdown: a positive fuse goes down by one, any other stays.
    fn count_down(&mut self) {
        if self.fuse > 0 {
            self.fuse -= 1;
        }
    }
}
pub fn fuse(m: &Mob) -> i32 {
    m.extra.sulfur_fuse.fuse
}
/// `SulfurCube.MAX_FUSE`, the synced data value.
pub fn max_fuse(m: &Mob) -> i32 {
    m.extra.sulfur_fuse.max
}
pub fn primed(m: &Mob) -> bool {
    fuse(m) >= 0
}
/// `SulfurCube.tick`: tickFuse removes the cube before super.tick can handle
/// its portal. This happens even when the TNT explosion gamerule is off.
pub fn removes_before_portal_tick(m: &Mob) -> bool {
    if m.species() != super::Species::SulfurCube {
        return false;
    }
    let fuse = fuse(m);
    let next_fuse = if fuse > 0 { fuse - 1 } else { fuse };
    next_fuse == 0 && archetype(m).is_some_and(|a| a.explosion.is_some())
}
/// `hurtServer`'s priming by a hit whose direct entity is a burning
/// `AbstractArrow` (an arrow or a trident): a cube with a block that can
/// explode lights its fuse, the long one.
pub fn burning_arrow_hit(m: &mut Mob) -> bool {
    has_body(m) && can_explode(m) && prime(m, false)
}

#[cfg(test)]
mod burning_arrow_tests {
    use super::*;
    use crate::mobs::{Mob, Species};
    use std::sync::Arc;

    fn cube(head: Option<&str>) -> Mob {
        test_archetype_items();
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.species = Species::SulfurCube;
        let mut m = Mob::new(1, Arc::new(kind), Vec3::ZERO, 0.0);
        m.extra.set_body(head.map(Into::into));
        m
    }

    #[test]
    fn a_burning_arrow_lights_the_long_fuse() {
        let mut m = cube(Some("minecraft:tnt"));
        assert!(burning_arrow_hit(&mut m));
        let (fuse_ticks, _, _) = archetype(&m).and_then(|a| a.explosion).unwrap();
        assert_eq!(fuse(&m), fuse_ticks);
        assert!(m.invulnerable_flag);
        // lit once: a second arrow leaves the fuse alone.
        m.extra.sulfur_fuse.fuse -= 5;
        assert!(!burning_arrow_hit(&mut m));
        assert_eq!(fuse(&m), fuse_ticks - 5);
    }

    #[test]
    fn a_burning_arrow_lights_only_an_explosive_block() {
        for head in [None, Some("minecraft:dirt")] {
            let mut m = cube(head);
            assert!(!burning_arrow_hit(&mut m), "{head:?}");
            assert!(!primed(&m) && !m.invulnerable_flag, "{head:?}");
        }
    }

    /// the arrow's knock, then the fuse lit: the next tick plays
    /// `entity.tnt.primed`, then the archetype's hit sound.
    #[test]
    #[ignore = "daily"]
    fn a_knock_and_a_lit_fuse_both_sound() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut events = Vec::new();
        let ran = crate::mobs::world::with_test_world(&[], |w, _| {
            let mut m = cube(Some("minecraft:tnt"));
            m.damage(level, 5.0, Damage::player([2.0, 0.0, 0.0]));
            assert!(burning_arrow_hit(&mut m));
            ai_step(&mut m, w, &mut events);
        });
        if !ran {
            return; // needs the prepared 26.3 data
        }
        let sounds: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                MobEvent::Sound(id, ..) => Some(*id),
                _ => None,
            })
            .collect();
        let hit = archetype_of("minecraft:tnt").unwrap().hit_sound;
        assert_eq!(sounds, ["entity.tnt.primed", hit]);
    }

    /// `SulfurCube.ageBoundaryReached` runs only where `AgeableMob.setAge`
    /// crosses the baby boundary: a loaded adult of size 1 and a big baby
    /// keep their size, and only a baby growing up becomes size 2 (#14916).
    #[test]
    #[ignore = "daily"]
    fn only_growing_up_makes_a_cube_size_2() {
        let mut events = Vec::new();
        let ran = crate::mobs::world::with_test_world(&[], |w, _| {
            // `/summon sulfur_cube` without Size or Age: an adult of size 1.
            let mut adult = cube(None);
            from_nbt(
                &mut adult,
                &sunrise_core::nbt::Tag::Compound(Default::default()),
            );
            // A split of a size-4 cube: a baby of size 2.
            let mut baby = cube(None);
            baby.extra.age = -24000;
            set_size(&mut baby, 2, true);
            for tick in 1..=40 {
                adult.age = tick;
                baby.age = tick;
                ai_step(&mut adult, w, &mut events);
                ai_step(&mut baby, w, &mut events);
            }
            assert_eq!((adult.extra.size, adult.attrs.max_health), (1, 4.0));
            assert!(!adult.is_baby());
            assert_eq!((baby.extra.size, baby.attrs.max_health), (2, 8.0));
            // The baby boundary: a cub of size 1 that grows up is size 2.
            let mut cub = cube(None);
            cub.extra.age = -1;
            set_size(&mut cub, 1, true);
            crate::mobs::finalize::set_age(&mut cub, 0, Some(w));
            assert_eq!((cub.extra.size, cub.health), (2, 8.0));
        });
        if !ran {
            eprintln!("skipped: needs the prepared 26.3 data");
        }
    }
}
/// Client copy of `tickFuse`: count the fuse down for the renderer. The
/// server alone explodes (`tick_fuse`); a guest never fires `Explode` here.
pub fn client_tick(m: &mut Mob) {
    m.extra.sulfur_fuse.count_down();
}
fn can_explode(m: &Mob) -> bool {
    archetype(m).is_some_and(|a| a.explosion.is_some()) && m.alive() && !primed(m)
}

#[cfg(test)]
mod immune_tests {
    use super::*;
    use crate::mobs::{Mob, Species};
    use std::sync::Arc;

    fn cube(head: Option<&str>) -> Mob {
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.species = Species::SulfurCube;
        let mut m = Mob::new(1, Arc::new(kind), Vec3::ZERO, 0.0);
        m.extra.set_body(head.map(Into::into));
        m
    }

    /// `#sulfur_cube_with_block_immune_to` has these too: a cube with a block
    /// loses no health to any of them (and `hurtServer` still answers true), a cube without one does.
    #[test]
    fn a_cube_with_a_block_shrugs_off_the_whole_tag() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        for kind in [
            DamageKind::FallingAnvil,
            DamageKind::FallingBlock,
            DamageKind::FallingStalactite,
            DamageKind::DryOut,
            DamageKind::Spit,
            DamageKind::Stalagmite,
            DamageKind::SulfurCubeHot,
            DamageKind::WindCharge,
        ] {
            let mut m = cube(Some("minecraft:dirt"));
            let health = m.health;
            // `SulfurCube.hurtServer` returns true for these, though no health is lost.
            assert!(m.damage(level, 1.0, Damage::of(kind)), "{kind:?}");
            assert_eq!(m.health, health, "{kind:?}");
            let mut m = cube(None);
            assert!(m.damage(level, 1.0, Damage::of(kind)), "{kind:?}");
            assert_eq!(m.health, health - 1.0, "{kind:?}");
        }
    }

    /// `SulfurCube.knockback` with `comesFromEffect`: a sprint hit's extra push (power 0.5) is
    /// `0.5 * 0.25` of the plain push, a Knockback II hit's (power 1.0) a quarter of it.
    #[test]
    fn an_effect_push_is_a_fraction_of_the_blows_own() {
        let push = |scale: f32| {
            launch(
                (1.0, 0.5),
                0.0,
                4.0,
                (1.0, 0.0),
                [-2.0, 0.0, 0.0],
                [0.0, 0.0, 0.0],
                [0.0, 0.5, 0.0],
                1.0,
                None,
                scale,
            )
        };
        let plain = push(1.0);
        assert!(plain[0].abs() > 0.01, "the plain push moves it: {plain:?}");
        for (power, scale) in [(0.5_f32, 0.125_f64), (1.0, 0.25)] {
            let extra = push(power * 0.25);
            for axis in 0..3 {
                assert!(
                    (extra[axis] - plain[axis] * scale).abs() < 1.0e-6,
                    "{power}: {extra:?} against {plain:?}"
                );
            }
        }
    }

    /// outside the tag (`magic`) a cube with a block is hurt.
    #[test]
    fn a_cube_with_a_block_takes_magic() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut m = cube(Some("minecraft:dirt"));
        let health = m.health;
        assert!(m.damage(level, 1.0, Damage::of(DamageKind::Magic)));
        assert_eq!(m.health, health - 1.0);
    }
}
pub fn register(m: &mut Mob) {
    use super::cube;
    m.ai.goals.add(1, cube::CubeFloat);
    m.ai.goals.add(2, SulfurTempt { player: None });
    m.ai.goals.add(3, SearchForItems);
    m.ai.goals.add(4, cube::CubeRandomDirection::default());
    m.ai.goals.add(5, cube::CubeKeepJumping);
    m.ai.mov.kind = MoveKind::Custom;
    m.ai.nav.can_float = true;
    m.extra.vector[0] = f64::from(180.0 * m.yaw / std::f32::consts::PI);
    // Created at size 1 (finalizeSpawn sizes it); not made a baby here.
    set_size(m, 1, false);
    m.health = m.attrs.max_health;
}
/// `setSize`: the max health (`setCubeMobHealth`, 4 per size) and movement
/// speed (0.2 + 0.1 * size, in floats) bases, which the save then holds;
/// size 1 makes an adult a baby.
pub fn set_size(m: &mut Mob, size: i32, update_health: bool) {
    use super::finalize::{Attr, set_base};
    let size = size.clamp(*super::cube::SIZES.start(), *super::cube::SIZES.end());
    m.extra.size = size;
    set_base(m, Attr::MaxHealth, f64::from(4 * size));
    set_base(
        m,
        Attr::MovementSpeed,
        f64::from(0.2f32 + 0.1f32 * size as f32),
    );
    if update_health {
        m.health = m.attrs.max_health;
        if size == 1 && !m.is_baby() {
            m.extra.age = -24000;
        }
    }
}
/// `setSpawnSize`: babies are size 1, adults size 2.
pub fn finalize(m: &mut Mob) {
    let size = if m.is_baby() { 1 } else { 2 };
    set_size(m, size, true);
}
/// `getSplitCount`: two babies, none from a primed cube.
pub fn split_count(m: &Mob) -> i32 {
    if primed(m) { 0 } else { 2 }
}
pub fn tiny(m: &Mob) -> bool {
    m.extra.size <= 1
}
pub fn jump_sound(m: &Mob) -> &'static str {
    if tiny(m) {
        "entity.small_sulfur_cube.jump"
    } else {
        "entity.sulfur_cube.jump"
    }
}
pub fn squish_sound(m: &Mob) -> &'static str {
    if tiny(m) {
        "entity.small_sulfur_cube.squish"
    } else if has_body(m) {
        "entity.sulfur_cube.bounce"
    } else {
        "entity.sulfur_cube.squish"
    }
}
/// The hurt and death sounds by size.
pub fn sounds(m: &Mob) -> [&'static str; 3] {
    if tiny(m) {
        [
            "",
            "entity.small_sulfur_cube.hurt",
            "entity.small_sulfur_cube.death",
        ]
    } else {
        ["", "entity.sulfur_cube.hurt", "entity.sulfur_cube.death"]
    }
}
thread_local! {
    /// The `tnt_explodes` game rule of the level this thread ticks, which
    /// `primeTime` and `tickFuse` read (shared by the game each tick and on
    /// a rule change).
    static TNT_EXPLODES: Cell<bool> = const { Cell::new(true) };
}
/// The `tnt_explodes` rule this thread's sulfur cubes read.
pub fn tnt_explodes() -> bool {
    TNT_EXPLODES.get()
}
/// Share the `tnt_explodes` rule with this thread's sulfur cubes.
pub fn set_tnt_explodes(on: bool) {
    TNT_EXPLODES.set(on);
}
/// `primeTime`: light the fuse (short when an explosion did it), only
/// while `tnt_explodes` is on.
pub fn prime(m: &mut Mob, imminent: bool) -> bool {
    let Some((fuse, _, _)) = archetype(m).and_then(|a| a.explosion) else {
        return false;
    };
    prime_with_fuse(m, imminent, fuse)
}
/// `primeTime` with the fuse of the cached `explosionData` (the body
/// `collectEquipmentChanges` last adopted), which is not the live body's
/// when the body changed since.
pub fn prime_with_fuse(m: &mut Mob, imminent: bool, fuse: i32) -> bool {
    if !m.alive() || !TNT_EXPLODES.get() || primed(m) {
        return false;
    }
    let t = if imminent {
        m.random.next_int_bound((fuse / 4).max(1)).unwrap_or(0) + fuse / 8
    } else {
        fuse
    };
    m.invulnerable_flag = true;
    m.extra.sulfur_fuse = Fuse::set(t);
    m.extra.scratch[1] |= 1;
    true
}
/// `#sulfur_cube_with_block_immune_to`: the damage a cube with a block shrugs off and is only
/// knocked by.
pub fn immune_to(source: &Damage) -> bool {
    if let DamageKind::Named(named) = source.kind {
        return named.has(NamedDamage::SULFUR_CUBE_WITH_BLOCK_IMMUNE_TO);
    }
    matches!(
        source.kind,
        DamageKind::PlayerAttack
            | DamageKind::MaceSmash
            | DamageKind::Spear
            | DamageKind::MobAttack
            | DamageKind::MobAttackNoAggro
            | DamageKind::Arrow
            | DamageKind::Thrown
            | DamageKind::Explosion
            | DamageKind::Fireworks
            | DamageKind::Fall
            | DamageKind::Sting
            | DamageKind::Cactus
            | DamageKind::Freeze
            | DamageKind::SweetBerryBush
            | DamageKind::HotFloor
            | DamageKind::FallingAnvil
            | DamageKind::FallingBlock
            | DamageKind::FallingStalactite
            | DamageKind::DryOut
            | DamageKind::Spit
            | DamageKind::Stalagmite
            | DamageKind::SulfurCubeHot
            | DamageKind::WindCharge
    )
}
/// `hurtServer`: fire or `#is_explosion` (a firework rocket's burst too)
/// light an explosive cube; with a block it shrugs off blows, falls and
/// explosions and is only knocked about.
pub fn accepts_damage(
    m: &mut Mob,
    level: &mut DamageLevel<'_>,
    source: &Damage,
    amount: f32,
) -> bool {
    if !has_body(m) {
        return true;
    }
    if can_explode(m) && !primed(m) {
        if source.is_fire() {
            prime(m, false);
        } else if source.kind.is_explosion() {
            prime(m, true);
        }
    }
    if immune_to(source) {
        // `dealDefaultKnockback`, unless the damage type is tagged no_knockback.
        if !source.no_knockback() {
            deal_default_knockback(m, level, source, amount);
        }
        return false;
    }
    true
}
/// `Vec2.rotate`: `Mth.sin`/`Mth.cos` of the angle, in floats.
fn rotate(x: f32, y: f32, angle: f64) -> (f32, f32) {
    let (cosine, sine) = (
        sunrise_core::geometry::trig::cos(angle),
        sunrise_core::geometry::trig::sin(angle),
    );
    (x * cosine - y * sine, y * cosine + x * sine)
}
fn normalize(v: [f64; 3]) -> [f64; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length < f64::from(1.0e-5_f32) {
        [0.0; 3]
    } else {
        [v[0] / length, v[1] / length, v[2] / length]
    }
}
/// `SulfurCube.applyHorizontalHitAngleScale`: the knock's direction turned by how far the
/// attacker's aim is from the cube, in the horizontal plane, times the scale.
fn horizontal_hit_angle_scale(
    scale: f32,
    angle: (f32, f32),
    eye: [f64; 3],
    aim: [f64; 3],
    center: [f64; 3],
) -> (f32, f32) {
    let to_target = normalize([center[0] - eye[0], center[1] - eye[1], center[2] - eye[2]]);
    let diff = sunrise_core::jmath::atan2(
        aim[0] * to_target[2] - aim[2] * to_target[0],
        aim[0] * to_target[0] + aim[2] * to_target[2],
    ) as f32;
    rotate(angle.0, angle.1, f64::from(diff * scale))
}
/// `SulfurCube.applyVerticalHitAnglePowerTransfer`: power moved between the horizontal and the
/// vertical launch by where the attacker aims between the cube's top and bottom.
fn vertical_hit_angle_power_transfer(
    scale: f32,
    powers: (f32, f32),
    eye: [f64; 3],
    aim: [f64; 3],
    center: [f64; 3],
    height: f32,
) -> (f32, f32) {
    let half = f64::from(0.5 * height);
    let top = normalize([
        center[0] - eye[0],
        center[1] + half - eye[1],
        center[2] - eye[2],
    ]);
    let bottom = normalize([
        center[0] - eye[0],
        center[1] - half - eye[1],
        center[2] - eye[2],
    ]);
    // Mth.clampedMap(aim.y, top.y, bottom.y, -1, 1): clampedLerp(inverseLerp(..), -1, 1).
    let t = (aim[1] - top[1]) / (bottom[1] - top[1]);
    let factor = if t < 0.0 {
        -1.0
    } else if t > 1.0 {
        1.0
    } else {
        -1.0 + t * 2.0
    } as f32;
    let mut ratio = (factor * scale).abs();
    if factor < 0.0 {
        ratio = -ratio;
    }
    (powers.0 * (1.0 - ratio), powers.1 * (1.0 + ratio))
}
/// `Attributes.KNOCKBACK_RESISTANCE` ranges from -2 to 1; a negative resistance (an
/// explosive or bouncy block) adds knockback.
fn resistance(attribute: f64) -> f64 {
    attribute.clamp(-2.0, 1.0)
}
/// `playerPush`'s scale: `Math.max(0, 1 - knockback_resistance)`, in double like vanilla.
fn push_factor(attribute: f64) -> f64 {
    (1.0 - attribute.clamp(-2.0, 1.0)).max(0.0)
}
/// What `SulfurCube.knockback` adds to the cube's movement for a blow of `damage`: the
/// archetype's `powers` (horizontal, vertical) turned by where the blow came from (`from`, the
/// direct entity: the direction of the knock) and, with the causing entity's `aim` (eye, view
/// direction), by where it points; the rotation by height uses its feet (`attacker_feet`).
#[allow(clippy::too_many_arguments)]
fn launch(
    powers: (f32, f32),
    resistance: f64,
    damage: f32,
    (xd, zd): (f64, f64),
    attacker_feet: [f64; 3],
    position: [f64; 3],
    center: [f64; 3],
    height: f32,
    aim: Option<([f64; 3], [f64; 3])>,
    effect_scale: f32,
) -> [f64; 3] {
    let (h0, v0) = powers;
    let (mut xd, mut zd) = (xd as f32, zd as f32);
    let (mut h, mut v) = (h0, v0);
    if let Some((eye, look)) = aim {
        let look = normalize(look);
        (xd, zd) = horizontal_hit_angle_scale(1.6, (xd, zd), eye, look, center);
        (h, v) = vertical_hit_angle_power_transfer(0.5, (h, v), eye, look, center, height);
    }
    // applyVerticalPositionAnglePowerRotation from feet to feet.
    let d = [
        position[0] - attacker_feet[0],
        position[1] - attacker_feet[1],
        position[2] - attacker_feet[2],
    ];
    let angle = sunrise_core::jmath::atan2(-d[1], (d[0] * d[0] + d[2] * d[2]).sqrt()) as f32;
    let (mut h, mut v) = rotate(h, v, f64::from(-angle * 0.8));
    let hr = if h0 > 0.0 { h.abs() / h0 } else { 0.0 };
    let vr = if v0 > 0.0 { v.abs() / v0 } else { 0.0 };
    let max = hr.max(vr);
    if max > 1.0 {
        h *= 1.0 / max;
        v *= 1.0 / max;
    }
    // `Mth.sqrt(damage) * (comesFromEffect ? power * 0.25 : 1)`.
    let k = damage.sqrt() * effect_scale;
    h *= k;
    v *= k;
    let resistance_scale = (1.0 - resistance) as f32;
    h *= resistance_scale;
    v *= resistance_scale;
    h *= 0.4;
    let (h, v) = (h.clamp(-128.0, 128.0), v.clamp(-128.0, 128.0));
    let (xd, zd) = (f64::from(xd), f64::from(zd));
    let len = (xd * xd + zd * zd).sqrt();
    let (kx, kz) = if len < f64::from(1.0e-5_f32) {
        (0.0, 0.0)
    } else {
        (xd / len * f64::from(h), zd / len * f64::from(h))
    };
    [-kx, f64::from(v) * 1.2, -kz]
}
/// `LivingEntity.dealDefaultKnockback` for a cube with a block: from where the blow came
/// (`source.getSourcePosition()`) with the short `knockback(0.4, ...)` entry.
fn deal_default_knockback(m: &mut Mob, level: &mut DamageLevel<'_>, source: &Damage, damage: f32) {
    // A projectile knocks the way it travelled (`calculateHorizontalHurtKnockbackDirection`),
    // else from the source's position.
    let (xd, zd) = match m.projectile_motion.take() {
        Some([x, z]) => (-x, -z),
        None => source
            .from
            .map_or((0.0, 0.0), |f| (f[0] - m.position.x, f[2] - m.position.z)),
    };
    m.knockback(level, f64::from(0.4_f32), xd, zd, source, damage);
}
/// `SulfurCube.knockback` with a body item and a causing entity: the launch powers, turned by
/// where the blow came from and by the entity's aim (`Mob::attacker_aim`, from the game).
pub(super) fn knockback(
    m: &mut Mob,
    direction: (f64, f64),
    from: Option<[f64; 3]>,
    damage: f32,
    effect_power: Option<f64>,
) {
    let (powers, knockback_resistance, hit_sound) = match archetype(m) {
        Some(a) => (
            (a.horizontal_power, a.vertical_power),
            a.knockback_resistance,
            a.hit_sound,
        ),
        None => (
            DEFAULT_KNOCKBACK,
            m.knockback_resistance(),
            DEFAULT_HIT_SOUND,
        ),
    };
    let aim = m.attacker_aim.take();
    let from = from.unwrap_or([m.position.x, m.position.y, m.position.z]);
    let delta = launch(
        powers,
        resistance(knockback_resistance),
        damage,
        direction,
        aim.map_or(from, |a| a.feet),
        [m.position.x, m.position.y, m.position.z],
        m.center(),
        m.height(),
        aim.map(|a| (a.eye, a.look)),
        effect_power.map_or(1.0, |p| p as f32 * 0.25),
    );
    m.velocity = Vec3::new(
        m.velocity.x + delta[0],
        m.velocity.y + delta[1],
        m.velocity.z + delta[2],
    );
    // needsSync: the push reaches the other players at once.
    m.needs_sync = true;
    // `Entity.playSound(hitSound.value())`: at the cube's feet, nothing while silent.
    let mut events = std::mem::take(&mut m.outbox);
    m.sound(hit_sound, &mut events);
    m.outbox = events;
}
/// `SulfurCube.tickFuse`: count down and discard at zero before the living
/// tick, including NoAI cubes. Pickup and push cooldowns belong to AI instead.
pub fn tick_fuse(m: &mut Mob, events: &mut Vec<MobEvent>) {
    m.extra.sulfur_fuse.count_down();
    if fuse(m) == 0
        && let Some((_, power, _)) = archetype(m).and_then(|a| a.explosion)
    {
        // With `tnt_explodes` off the cube still goes, without a blast.
        if TNT_EXPLODES.get() {
            // `level.explode(this, ..., TNT)`: the cube is the blast's source and its
            // causing entity, so its entity query leaves it out.
            let name = match &m.custom_name {
                Some(name) => crate::text::Text::literal(name.clone()),
                None => crate::text::Text::translate("entity.minecraft.sulfur_cube", &[]),
            };
            events.push(MobEvent::ExplosionSource {
                mob: m.id,
                charged: false,
                name,
                in_water: m.fluids.in_water,
                kind: m.kind.id.clone(),
            });
            events.push(MobEvent::Explode(
                [m.position.x, m.position.y + 0.0625, m.position.z],
                power,
                m.species(),
                m.silent,
            ));
        }
        m.removed = true;
    }
}

/// `SulfurCube.customServerAiStep` and the remaining cube additions:
/// the timers, a swallowed block's stillness, buoyancy and
/// facing, growing up, and players pushing it.
pub fn ai_step(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    // primeTime's sound.
    let pending = m.extra.scratch[1];
    if pending & 1 != 0 {
        events.push(MobEvent::Sound("entity.tnt.primed", m.center(), 1.0, 1.0));
    }
    m.extra.scratch[1] = 0;
    if m.extra.ints[3] > 0 {
        m.extra.ints[3] -= 1;
    }
    if m.extra.scratch[2] > 0 {
        m.extra.scratch[2] -= 1;
    }
    // Growing to size 2 is `ageBoundaryReached`, run only where `setAge` crosses
    // the baby boundary (`finalize::set_age`), never by the tick (#14916).
    let body = has_body(m);
    // collectEquipmentChanges: a swallowed block takes every goal away.
    if body {
        m.ai.goals.disabled |= MOVE | LOOK | JUMP;
        m.ai.speed = 0.0;
        // SulfurCubeLookControl: square to the nearest quarter turn.
        let close = (m.yaw + 45.0).rem_euclid(90.0) - 45.0;
        m.yaw -= close;
        m.head_yaw = m.yaw;
    } else {
        m.ai.goals.disabled &= !(MOVE | LOOK | JUMP);
    }
    // travelInFluid: a buoyant block bobs up.
    if body && m.is_in_liquid() && archetype(m).is_some_and(|a| a.buoyant) {
        let vibe = 0.2 * (m.age as f32 * 0.4).sin();
        let height = if m.fluids.in_water {
            m.fluids.water
        } else {
            m.fluids.lava
        };
        let immersion = height - f64::from(m.height()) * 0.2 + f64::from(vibe);
        if immersion > 0.0 {
            m.velocity.y += immersion.min(1.0) * f64::from(0.04f32);
        }
    }
    if body && let Some(p) = w.selected_player() {
        player_push(m, p, w, events);
    }
}
/// `SulfurCube.doPush` / `applyContactDamage`, for ordinary mob targets.
/// Use the same overlap, pushability and team predicate as pushEntities; the
/// contact still happens when Entity.push produces no impulse at equal centres.
/// Each hit samples the archetype's amount from the cube's own random and
/// carries its damage type; the cube is the source only with `attribute_to_source`.
pub fn mob_contacts(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    let Some(contact) = archetype(m).and_then(|a| a.contact_damage.clone()) else {
        return;
    };
    let bounds = m.bounds();
    let own_team = w.teams.of(super::Who::Mob(m.id));
    let attacker = contact
        .attribute_to_source
        .then_some((m.id, [m.position.x, m.position.y, m.position.z]));
    for other in w.others_near(bounds) {
        if other.id != m.id
            && other.alive
            && other.pushable
            && other.bounds.intersects(bounds)
            && crate::scoreboard::pushable_by(own_team, w.teams.of(super::Who::Mob(other.id)), true)
        {
            events.push(MobEvent::ContactDamageMob {
                target: other.id,
                amount: contact.amount.sample(&mut m.random),
                kind: contact.damage_type,
                attacker,
            });
        }
    }
}

/// `playerTouch` / `playerPush`: a cube holding a block is shoved by a
/// player walking into it (and burns them when hot).
fn player_push(m: &mut Mob, p: &super::PlayerView, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if !p.alive || p.spectator {
        return;
    }
    let touch = p.bounds.inflate(Vec3::new(1.0, 0.5, 1.0));
    if !m.bounds().intersects(touch) {
        return;
    }
    let (dx, dz) = (m.position.x - p.position.x, m.position.z - p.position.z);
    let horizontal = (dx * dx + dz * dz).sqrt();
    let (bottom, top) = (m.position.y, m.position.y + f64::from(m.height()));
    let player_top = p.bounds.max_position().y;
    if !(horizontal < f64::from(1.3f32) && p.position.y <= top && player_top > bottom) {
        return;
    }
    let a = archetype(m);
    let a = a.as_deref();
    let kb = push_factor(a.map_or(0.0, |a| a.knockback_resistance));
    let len = horizontal.max(1.0e-4);
    let speed = (p.velocity.length() * 2.0 * f64::from(0.3f32)).clamp(0.0, 0.5);
    let push = Vec3::new(
        dx / len * kb * speed,
        if m.ground.on_ground {
            kb * f64::from(0.3f32) * speed
        } else {
            0.0
        },
        dz / len * kb * speed,
    );
    let (sound, cooldown, threshold) = a.map_or(DEFAULT_PUSH, |a| {
        (a.push_sound, a.push_cooldown, a.push_threshold)
    });
    if push.length_squared() > f64::from(threshold * threshold) && m.extra.scratch[2] <= 0 {
        m.extra.scratch[2] = (cooldown * 20.0) as i32;
        events.push(MobEvent::Sound(sound, m.center(), 1.0, 1.0));
    }
    m.velocity = m.velocity.add_vector(push);
    if let Some(contact) = a.and_then(|a| a.contact_damage.clone()) {
        events.push(w.route_player_event(MobEvent::DamagePlayer {
            amount: contact.amount.sample(&mut m.random),
            from: [m.position.x, m.position.y, m.position.z],
            kind: contact.damage_type,
            wielder: None,
        }));
    }
}
/// `SulfurCubeRenderer.scale` (after the cube's size and squish, flat when
/// holding a block): the fuse swell, half size for an adult and the lift.
pub fn scale(m: &Mob, alpha: f32) -> glam::Mat4 {
    let size = m.extra.size.max(1) as f32;
    let squish = if has_body(m) {
        0.0
    } else {
        m.anim.lerp(0, alpha) / (size * 0.5 + 1.0)
    };
    let w = 1.0 / (squish + 1.0);
    let mut t = glam::Mat4::from_scale(glam::Vec3::splat(0.999))
        * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.001, 0.0))
        * glam::Mat4::from_scale(glam::Vec3::new(w * size, 1.0 / w * size, w * size));
    let fuse = if primed(m) {
        fuse(m) as f32 - alpha + 1.0
    } else {
        0.0
    };
    if fuse < 10.0 && fuse > 0.0 {
        let g = (1.0 - fuse / 10.0).clamp(0.0, 1.0);
        let s = 1.0 + g * g * g * g * 0.3;
        t *= glam::Mat4::from_scale(glam::Vec3::splat(s));
    }
    let baby = m.is_baby();
    let (offset, down) = if baby { (1.24, 1.0) } else { (0.98, 0.5) };
    t * glam::Mat4::from_scale(glam::Vec3::splat(down))
        * glam::Mat4::from_translation(glam::Vec3::new(0.0, offset - 1.0 / 16.0, 0.0))
}
/// The outer body model and texture.
pub fn body(m: &Mob) -> (String, String) {
    if m.is_baby() {
        (
            "sulfur_cube_small#main".into(),
            "entity/sulfur_cube/sulfur_cube_outer_small".into(),
        )
    } else {
        (
            "sulfur_cube#main".into(),
            "entity/sulfur_cube/sulfur_cube_outer".into(),
        )
    }
}
/// `SulfurCubeInnerLayer`: the inner cube when no block is held.
pub fn inner(m: &Mob) -> Option<(&'static str, &'static str)> {
    if has_body(m) {
        return None;
    }
    Some(if m.is_baby() {
        (
            "sulfur_cube_small#inner",
            "entity/sulfur_cube/sulfur_cube_inner_small",
        )
    } else {
        ("sulfur_cube#inner", "entity/sulfur_cube/sulfur_cube_inner")
    })
}
/// `SulfurCubeTemptGoal` (`TemptGoal.ForNonPathfinders`): face a player
/// holding slime balls (babies) or something to swallow (adults) and hop.
#[derive(Clone, Debug)]
struct SulfurTempt {
    player: Option<[f64; 3]>,
}
impl Goal for SulfurTempt {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        LOOK
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(p) = cx.w.selected_player() else {
            return false;
        };
        let wants = |i: &Option<std::sync::Arc<str>>| {
            i.as_deref().is_some_and(|i| {
                if m.is_baby() {
                    m.kind.food.iter().any(|f| f == i)
                } else {
                    swallowable(m, i)
                }
            })
        };
        // Attributes.TEMPT_RANGE 8, scaled by visibility (floor 2).
        let range = (8.0 * p.visibility_percent(m.species())).max(2.0);
        let ok = p.alive
            && !p.spectator
            && (wants(&p.main_hand) || wants(&p.off_hand))
            && m.distance_sqr(p.position) <= range * range;
        self.player = ok.then_some(p.eye);
        ok
    }
    #[inline]
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(p) = cx.w.selected_player() else {
            return;
        };
        if m.distance_sqr(p.position) < 1.0 {
            super::cube::set_wanted(m, 0.0);
            return;
        }
        m.turn_to(p.eye, 10.0, 10.0);
        let yaw = m.yaw;
        super::cube::set_direction(m, yaw, true);
    }
    #[inline]
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        super::cube::set_wanted(m, 0.0);
    }
}
/// `SulfurCubeSearchForItemsGoal`: hop toward the nearest thing it could
/// swallow (the game marks it in `point`).
#[derive(Clone, Debug)]
struct SearchForItems;
impl Goal for SearchForItems {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        LOOK
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        !m.is_baby() && m.extra.ints[3] <= 0 && m.extra.point.is_some()
    }
    #[inline]
    fn tick(&mut self, m: &mut Mob, _: &mut Cx) {
        let Some(p) = m.extra.point else { return };
        m.turn_to(p, 10.0, 10.0);
        let yaw = m.yaw;
        super::cube::set_direction(m, yaw, true);
    }
}
/// `addAdditionalSaveData`: the cube's size, the timers and the fuse.
pub fn to_nbt(m: &Mob, map: &mut crate::entity_nbt::EntityTagWriter) {
    use sunrise_core::nbt::Tag;
    map.insert("Size".into(), Tag::Int(m.extra.size.max(1) - 1));
    map.insert("wasOnGround".into(), Tag::Byte(i8::from(m.extra.flag2)));
    map.insert("pickup_timer".into(), Tag::Int(m.extra.ints[3]));
    map.insert(
        "from_bucket".into(),
        Tag::Byte(i8::from(m.extra.scratch[3] != 0)),
    );
    map.insert("fuse".into(), Tag::Int(fuse(m)));
}
/// `SulfurCube.readAdditionalSaveData`: size precedes the living health default.
pub fn from_nbt(m: &mut Mob, tag: &sunrise_core::nbt::Tag) {
    use sunrise_core::nbt::Tag;
    let int = |k: &str| match tag.get(k) {
        Some(Tag::Int(v)) => Some(*v),
        Some(Tag::Byte(v)) => Some(i32::from(*v)),
        _ => None,
    };
    m.extra.age = int("Age").unwrap_or(0);
    m.extra.age_locked = int("AgeLocked").unwrap_or(0) != 0;
    set_size(m, int("Size").unwrap_or(0) + 1, false);
    // LivingEntity's load applies the saved bases after the size, so they win.
    if let Some(Tag::List(saved)) = tag.get("attributes") {
        use super::finalize::{Attr, set_base};
        for a in saved {
            let attr = match crate::save::text(a.get("id")).as_deref() {
                Some("minecraft:max_health") => Attr::MaxHealth,
                Some("minecraft:movement_speed") => Attr::MovementSpeed,
                _ => continue,
            };
            if let Some(base) = crate::save::float(a.get("base")) {
                set_base(m, attr, base);
            }
        }
    }
    // The saved Health, clamped to the sized maximum: vanilla sizes the cube before `setHealth`.
    m.health = crate::entity_nbt::loaded_health(m, crate::save::float(tag.get("Health")));
    m.extra.flag2 = int("wasOnGround").unwrap_or(0) != 0;
    m.extra.ints[3] = int("pickup_timer").unwrap_or(0);
    m.extra.scratch[3] = int("from_bucket").unwrap_or(0);
    // `setFuse(input.getIntOr("fuse", -1))` stores any numeric value (narrowed, floats floored),
    // `MAX_FUSE` takes it too, and the load leaves `Invulnerable` as `Entity.load` read it.
    let fuse = tag.get("fuse").and_then(crate::nbt_path::int_value);
    m.extra.sulfur_fuse = Fuse::set(fuse.unwrap_or(-1));
}

#[cfg(test)]
mod tnt_rule_tests {
    use super::*;
    use crate::mobs::{Damage, DamageKind, Mob, Species};
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;

    fn tnt_cube() -> Mob {
        test_archetype_items();
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.species = Species::SulfurCube;
        let mut m = Mob::new(1, Arc::new(kind), Vec3::ZERO, 0.0);
        m.extra.set_body(Some("minecraft:tnt".into()));
        m
    }

    /// Vanilla 26.3 server witness (`VanillaSulfurCubeFuseOracle`, `tools/test_sulfur_cube_fuse_parity.py`).
    #[test]
    #[ignore = "run by tools/test_sulfur_cube_fuse_parity.py"]
    fn a_loaded_fuse_is_kept_as_it_is_and_leaves_invulnerable_alone() {
        use sunrise_core::nbt::{JavaString, Tag, parse_snbt};
        let dir = std::env::var_os("SUNRISE_SULFUR_CUBE_FUSE_WITNESS_DIR").unwrap_or_else(|| {
            panic!(
                "SUNRISE_SULFUR_CUBE_FUSE_WITNESS_DIR is unset; run tools/test_sulfur_cube_fuse_parity.py"
            )
        });
        let path = std::path::PathBuf::from(dir).join("sulfur_cube_fuse_server.txt");
        let witness = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| {
                panic!(
                    "failed to read witness {} (run tools/test_sulfur_cube_fuse_parity.py): {e}",
                    path.display()
                )
            })
            .replace("\r\n", "\n");
        let mut checked = 0usize;
        for line in witness.lines().filter(|line| !line.starts_with('#')) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            assert_eq!(fields[2], "loaded", "{line}");
            assert_eq!(fields[7], "saved", "{line}");
            let (fuse_snbt, inv_snbt) = (fields[0], fields[1]);
            let expected_fuse: i32 = fields[3].parse().unwrap();
            let expected_max: i32 = fields[4].parse().unwrap();
            let expected_inv: bool = fields[5].parse().unwrap();
            let expected_primed: bool = fields[6].parse().unwrap();
            let expected_saved_fuse: i32 = fields[8].parse().unwrap();
            let mut m = tnt_cube();
            // `Entity.load` parses `Invulnerable` before `SulfurCube.readAdditionalSaveData`.
            m.invulnerable_flag = inv_snbt == "1b";
            let snbt = match fuse_snbt {
                "absent" => "{}".to_owned(),
                value => format!("{{fuse:{value}}}"),
            };
            let tag = parse_snbt(&snbt).unwrap().0;
            from_nbt(&mut m, &tag);
            assert_eq!(fuse(&m), expected_fuse, "{line}");
            assert_eq!(max_fuse(&m), expected_max, "{line}");
            assert_eq!(m.invulnerable_flag, expected_inv, "{line}");
            assert_eq!(primed(&m), expected_primed, "{line}");
            let mut saved = crate::entity_nbt::EntityTagWriter::new(None);
            to_nbt(&m, &mut saved);
            assert_eq!(
                saved.get(&JavaString::from("fuse")),
                Some(&Tag::Int(expected_saved_fuse)),
                "{line}"
            );
            checked += 1;
        }
        assert!(checked > 0, "empty witness");
    }

    #[test]
    #[ignore = "daily"]
    fn mob_tick_counts_the_fuse_but_keeps_noai_cooldowns_still() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        crate::mobs::world::with_test_world(&[], |w, _| {
            for no_ai in [true, false] {
                let mut m = tnt_cube();
                m.no_ai = no_ai;
                m.no_gravity = true;
                m.persistent = true;
                m.extra.sulfur_fuse = Fuse::set(3);
                m.extra.ints[3] = 100;
                m.extra.scratch[2] = 10;
                let mut events = Vec::new();
                for remaining in [2, 1] {
                    assert!(m.tick(w, level, &mut events));
                    assert_eq!(fuse(&m), remaining);
                    assert_eq!(m.extra.ints[3], if no_ai { 100 } else { 97 + remaining });
                    assert_eq!(m.extra.scratch[2], if no_ai { 10 } else { 7 + remaining });
                }
                assert!(!m.tick(w, level, &mut events));
                assert!(m.removed);
                assert_eq!(
                    events
                        .iter()
                        .filter(|e| matches!(e, MobEvent::Explode(..)))
                        .count(),
                    1
                );
                assert!(!m.tick(w, level, &mut events));
                assert_eq!(
                    events
                        .iter()
                        .filter(|e| matches!(e, MobEvent::Explode(..)))
                        .count(),
                    1
                );
            }
        });
    }

    #[test]
    fn named_damage_tags_drive_block_immunity_and_explosive_fuse() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let source = |tags| Damage::of(DamageKind::Named(NamedDamage::new("example:hit", tags)));
        let mut untagged = tnt_cube();
        assert!(accepts_damage(&mut untagged, level, &source(0), 5.0));
        assert!(!primed(&untagged));
        let mut tagged = tnt_cube();
        assert!(!accepts_damage(
            &mut tagged,
            level,
            &source(
                NamedDamage::IS_EXPLOSION
                    | NamedDamage::SULFUR_CUBE_WITH_BLOCK_IMMUNE_TO
                    | NamedDamage::NO_KNOCKBACK,
            ),
            5.0,
        ));
        assert!(primed(&tagged));
    }

    #[test]
    fn no_fuse_is_lit_while_tnt_does_not_explode() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        // primeTime checks the tnt_explodes rule: no fuse, no invulnerability,
        // and no draw for the short fuse.
        let mut m = tnt_cube();
        let state = m.random.state();
        set_tnt_explodes(false);
        m.damage(level, 5.0, Damage::of(DamageKind::Explosion));
        let lit = prime(&mut m, true);
        let (unprimed, vulnerable) = (!primed(&m), !m.invulnerable_flag);
        set_tnt_explodes(true);
        assert!(!lit && unprimed && vulnerable);
        assert_eq!(m.random.state(), state);
        assert!(prime(&mut m, false));
        assert!(m.invulnerable_flag);
    }

    #[test]
    #[ignore = "daily"]
    fn a_fuse_running_out_while_tnt_does_not_explode_makes_no_blast() {
        // tickFuse: the blast (and its source) only with tnt_explodes, the
        // discard either way.
        let mut events = Vec::new();
        let ran = crate::mobs::world::with_test_world(&[], |_, _| {
            let mut m = tnt_cube();
            assert!(prime(&mut m, false));
            m.extra.sulfur_fuse = Fuse::set(1); // 0 after this tick
            set_tnt_explodes(false);
            tick_fuse(&mut m, &mut events);
            set_tnt_explodes(true);
            assert!(m.removed);
        });
        if !ran {
            return; // needs the prepared 26.3 data
        }
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, MobEvent::Explode(..) | MobEvent::ExplosionSource { .. })),
            "{events:?}"
        );
    }
}

#[cfg(test)]
mod fuse_anim_tests {
    use super::*;
    use crate::mobs::{Mob, Species};
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;

    fn cube() -> Mob {
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.species = Species::SulfurCube;
        Mob::new(1, Arc::new(kind), Vec3::ZERO, 0.0)
    }

    #[test]
    fn a_primed_cube_is_still_knocked_about() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        // SulfurCube.hurtServer runs the immune knockback before
        // LivingEntity's invulnerability check, and priming makes it invulnerable.
        test_archetype_items();
        let mut m = cube();
        m.extra.set_body(Some("minecraft:tnt".into()));
        assert!(prime(&mut m, false));
        assert!(m.invulnerable_flag);
        let (health, fuse_left) = (m.health, fuse(&m));
        m.damage(level, 5.0, crate::mobs::Damage::player([2.0, 0.0, 0.0]));
        assert!(m.velocity.x < 0.0, "velocity {:?}", m.velocity);
        assert_eq!(m.health, health);
        assert_eq!(fuse(&m), fuse_left);
    }

    #[test]
    fn client_fuse_counts_down_without_going_negative() {
        let mut m = cube();
        m.extra.sulfur_fuse = Fuse::set(4);
        assert!(primed(&m));
        client_tick(&mut m);
        assert_eq!(fuse(&m), 3);
        for _ in 0..10 {
            client_tick(&mut m);
        }
        // Stops at fuse 0 (primed until the server removes it).
        assert_eq!(fuse(&m), 0);
    }

    #[test]
    fn a_rockets_burst_primes_a_tnt_cube_that_shrugs_it_off() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        // `fireworks` is in #is_explosion, so it lights the short fuse
        // (`PrimedTnt.getRandomShortFuse`: 15 to 44 of 120 ticks), and so in
        // #sulfur_cube_with_block_immune_to; it is not in #no_knockback.
        test_archetype_items();
        let source = Damage::fireworks(Some(crate::mobs::Who::Player), [2.0, 0.0, 0.0]);
        assert_eq!(source.kind, DamageKind::Fireworks);
        assert!(!source.no_knockback());
        let mut m = cube();
        m.extra.set_body(Some("minecraft:tnt".into()));
        let health = m.health;
        m.damage(level, 7.0, source);
        assert!((15..45).contains(&fuse(&m)), "fuse {}", fuse(&m));
        assert_eq!(m.health, health);
        assert!(m.velocity.x < 0.0);
        // Without a block it takes the hit, records it and is knocked away
        // from the rocket.
        let mut m = cube();
        m.damage(level, 1.0, source);
        assert_eq!(m.health, health - 1.0);
        assert_eq!(
            m.last_damage.map(|(d, _)| d.kind),
            Some(DamageKind::Fireworks)
        );
        assert!(m.velocity.x < 0.0);
    }
    #[test]
    fn knockback_plays_one_hit_sound_per_launch_at_feet_unless_silent() {
        let sounds = |m: &Mob| {
            m.outbox
                .iter()
                .filter_map(|e| match e {
                    MobEvent::Sound(name, at, volume, pitch) => Some((*name, *at, *volume, *pitch)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };

        // Exercise the default launch without installing an archetype table. The
        // resource-backed living-knockback test also covers the explosive archetype.
        let mut m = cube();
        m.position = Vec3::new(1.0, 2.0, 3.0);
        m.extra.set_body(Some("minecraft:iron_sword".into()));
        m.attrs.knockback_resistance = 0.9;
        knockback(&mut m, (1.0, 0.0), Some([2.0, 2.0, 3.0]), 4.0, None);
        let expected_h = 0.33_f32 * 2.0 * ((1.0_f64 - 0.9_f64) as f32) * 0.4;
        assert_eq!(m.velocity.x, -f64::from(expected_h));
        assert_ne!(
            m.velocity.x,
            -f64::from(0.33_f32 * 2.0 * (1.0_f32 - 0.9_f64 as f32) * 0.4)
        );
        knockback(&mut m, (1.0, 0.0), Some([2.0, 2.0, 3.0]), 4.0, Some(1.0));
        assert_eq!(m.extra.scratch[1], 0);
        assert_eq!(
            sounds(&m),
            [
                ("entity.sulfur_cube.regular.hit", [1.0, 2.0, 3.0], 1.0, 1.0),
                ("entity.sulfur_cube.regular.hit", [1.0, 2.0, 3.0], 1.0, 1.0),
            ]
        );

        m.outbox.clear();
        m.silent = true;
        knockback(&mut m, (1.0, 0.0), Some([2.0, 2.0, 3.0]), 4.0, None);
        assert!(m.outbox.is_empty());
    }
}

#[cfg(test)]
mod launch_tests {
    use super::launch;

    /// What vanilla 26.3's `SulfurCube.knockback` adds to the movement, computed by running its
    /// three private helpers' bodies on the real `Vec2`, `Vec3` and `Mth` classes: four blows
    /// with the attacker at the blow's position (a melee hit), one with no aim (an attacker that
    /// gives none) and two from an arrow, whose position is where the knock comes from while
    /// the owner's feet give the rotation by height.
    #[test]
    fn the_launch_follows_the_vanilla_helpers() {
        struct Case {
            powers: (f32, f32),
            resistance: f64,
            damage: f32,
            /// The direct entity's position.
            from: [f64; 3],
            /// Its horizontal motion, if it is a projectile.
            motion: Option<[f64; 2]>,
            attacker_feet: [f64; 3],
            position: [f64; 3],
            aim: Option<([f64; 3], [f64; 3])>,
            expected: [f64; 3],
        }
        let height = 0.5;
        let cases = [
            Case {
                powers: (1.0, 0.6),
                resistance: 0.0,
                damage: 5.0,
                from: [0.5, 64.0, 0.5],
                motion: None,
                attacker_feet: [0.5, 64.0, 0.5],
                position: [3.5, 64.0, 0.5],
                aim: Some(([0.5, 65.62, 0.5], [1.0, -0.1, 0.0])),
                expected: [0.8944272398948669, 0.536656379699707, -0.0],
            },
            Case {
                powers: (1.0, 0.6),
                resistance: 0.25,
                damage: 8.0,
                from: [0.5, 64.0, 0.5],
                motion: None,
                attacker_feet: [0.5, 64.0, 0.5],
                position: [3.5, 64.0, 1.5],
                aim: Some(([0.5, 65.62, 0.5], [0.95, -0.3, 0.2])),
                expected: [0.7428210177564502, 0.5091168880462646, 0.41014247554144134],
            },
            Case {
                powers: (2.0, 1.0),
                resistance: 0.0,
                damage: 6.0,
                from: [10.5, 70.0, -3.5],
                motion: None,
                attacker_feet: [10.5, 70.0, -3.5],
                position: [7.5, 64.0, -2.0],
                aim: Some(([10.5, 71.62, -3.5], [-0.5, -0.7, 0.4])),
                expected: [-1.1930282398964365, -2.939387798309326, 0.15109800367878837],
            },
            Case {
                powers: (1.5, 0.8),
                resistance: 0.5,
                damage: 4.0,
                from: [0.5, 64.0, 0.5],
                motion: None,
                attacker_feet: [0.5, 64.0, 0.5],
                position: [2.5, 64.0, 0.5],
                aim: None,
                expected: [0.6000000238418579, 0.9600000143051147, -0.0],
            },
            Case {
                powers: (1.0, 0.6),
                resistance: 0.0,
                damage: 6.0,
                from: [1.5, 64.9, 0.5],
                motion: Some([0.9, 0.1]),
                attacker_feet: [-6.5, 64.0, 0.5],
                position: [3.5, 64.0, 0.5],
                aim: Some(([-6.5, 65.62, 0.5], [1.0, -0.05, 0.1])),
                expected: [0.9786302987781631, 0.5878775954246521, -0.04777874198383045],
            },
            Case {
                powers: (2.0, 1.0),
                resistance: 0.25,
                damage: 5.0,
                from: [-2.5, 66.0, 3.5],
                motion: Some([-0.7, 0.4]),
                attacker_feet: [-9.5, 64.0, 6.5],
                position: [1.5, 64.0, -0.5],
                aim: Some(([-9.5, 65.62, 6.5], [0.8, -0.2, -0.5])),
                expected: [-0.38538593044563885, 2.012461280822754, 0.22688699055044273],
            },
        ];
        for (i, c) in cases.into_iter().enumerate() {
            // The box centre: the feet position plus half the height.
            let center = [
                c.position[0],
                c.position[1] + f64::from(height) / 2.0,
                c.position[2],
            ];
            // dealDefaultKnockback: a projectile knocks the way it travelled, else away from the
            // source's position.
            let direction = match c.motion {
                Some([x, z]) => (-x, -z),
                None => (c.from[0] - c.position[0], c.from[2] - c.position[2]),
            };
            let got = launch(
                c.powers,
                c.resistance,
                c.damage,
                direction,
                c.attacker_feet,
                c.position,
                center,
                height,
                c.aim,
                1.0,
            );
            for axis in 0..3 {
                assert!(
                    (got[axis] - c.expected[axis]).abs() < 1.0e-9,
                    "case {i} axis {axis}: {got:?} against {:?}",
                    c.expected
                );
            }
        }
    }
}

#[cfg(test)]
mod resistance_tests {
    use super::{launch, push_factor, resistance};

    #[test]
    fn negative_resistance_adds_knockback() {
        assert_eq!(resistance(-1.0), -1.0);
        assert_eq!(resistance(-2.0), -2.0);
        assert_eq!(resistance(-5.0), -2.0);
        assert_eq!(resistance(3.0), 1.0);
        assert_eq!(push_factor(-1.0), 2.0);
        assert_eq!(push_factor(-2.0), 3.0);
        assert_eq!(push_factor(1.0), 0.0);
        assert_eq!(push_factor(0.3), 1.0 - 0.3);
        let push = |r: f64, damage: f32| {
            launch(
                (1.0, 0.6),
                resistance(r),
                damage,
                (1.0, 0.0),
                [0.0, 64.0, 0.0],
                [1.0, 64.0, 0.0],
                [1.0, 64.5, 0.0],
                1.0,
                None,
                1.0,
            )
        };
        let base = push(0.0, 4.0);
        let explosive = push(-1.0, 4.0);
        let bouncy = push(-2.0, 4.0);
        assert!((explosive[0] / base[0] - 2.0).abs() < 1.0e-6);
        assert!((bouncy[0] / base[0] - 3.0).abs() < 1.0e-6);
        let fractional = push(0.9, 4.0);
        assert_eq!(
            fractional[0],
            -f64::from(1.0_f32 * 2.0 * ((1.0_f64 - 0.9_f64) as f32) * 0.4)
        );
        assert_ne!(
            fractional[0],
            -f64::from(1.0_f32 * 2.0 * (1.0_f32 - 0.9_f64 as f32) * 0.4)
        );
    }
}

#[cfg(test)]
mod mob_contact_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    #[ignore = "daily"]
    fn mob_tick_contacts_only_overlapping_pushable_mobs_with_a_hot_body() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        test_archetype_items();
        let victim = |id: u32, at: Vec3, alive: bool| {
            let mut m = Mob::new(id, crate::mobs::sensor_scan_tests::kind(), at, 0.0);
            if !alive {
                m.health = 0.0;
            }
            m.seen(None, &crate::items::Items::empty())
        };
        let others = [
            victim(2, Vec3::ZERO, true),
            victim(3, Vec3::new(8.0, 0.0, 0.0), true),
            victim(4, Vec3::ZERO, false),
        ];
        crate::mobs::world::with_test_world(&others, |w, types| {
            for no_ai in [true, false] {
                for body in [
                    None,
                    Some("minecraft:white_concrete_powder"),
                    Some("minecraft:magma_block"),
                ] {
                    let mut m = Mob::new(
                        1,
                        Arc::new(types.types["minecraft:sulfur_cube"].clone()),
                        Vec3::ZERO,
                        0.0,
                    );
                    m.no_ai = no_ai;
                    m.no_gravity = true;
                    m.persistent = true;
                    m.extra.set_body(body.map(Into::into));
                    let mut events = Vec::new();
                    assert!(m.tick(w, level, &mut events));
                    let hits: Vec<_> = events
                        .iter()
                        .filter_map(|event| match event {
                            MobEvent::ContactDamageMob {
                                target,
                                amount,
                                kind,
                                attacker,
                            } => Some((*target, *amount, *kind, *attacker)),
                            _ => None,
                        })
                        .collect();
                    if body == Some("minecraft:magma_block") {
                        assert_eq!(
                            hits,
                            [(2, 1.0, "minecraft:sulfur_cube_hot", None)],
                            "NoAI={no_ai}"
                        );
                    } else {
                        assert!(hits.is_empty(), "{body:?}, NoAI={no_ai}: {hits:?}");
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod contact_decode_tests {
    use super::*;
    use serde_json::json;

    fn decode(amount: serde_json::Value, attribute: bool) -> Result<Decoded, String> {
        Decoded::decode(
            "test:acid",
            &json!({
                "items": ["minecraft:dirt"],
                "attribute_modifiers": [],
                "contact_damage": {
                    "amount": amount,
                    "attribute_to_source": attribute,
                    "damage_type": "test:acid",
                },
                "knockback_modifiers": {"horizontal_power": 0.33, "vertical_power": 0.06},
                "sound_settings": {
                    "hit_sound": {"sound_id": "test:hit"},
                    "push_sound": {"sound_id": "test:push"},
                    "push_sound_impulse_threshold": 0.2,
                    "push_sound_cooldown": 0.5,
                },
            }),
            &|_| true,
        )
    }

    #[test]
    fn a_pack_contact_damage_keeps_its_provider_type_and_attribution() {
        let uniform =
            json!({"type": "minecraft:uniform", "min_inclusive": 2.0, "max_exclusive": 4.0});
        let archetype = decode(uniform, true).unwrap().archetype;
        let contact = archetype.contact_damage.unwrap();
        assert!(contact.attribute_to_source);
        assert_eq!(contact.damage_type, "test:acid");
        let mut random = sunrise_core::random::LegacyRandom::new(1);
        for _ in 0..16 {
            let amount = contact.amount.sample(&mut random);
            assert!((2.0..4.0).contains(&amount), "{amount}");
        }
        let plain = decode(json!(1.5), false).unwrap().archetype;
        assert!(!plain.contact_damage.unwrap().attribute_to_source);
    }

    #[test]
    fn a_contact_amount_with_a_negative_minimum_is_refused() {
        assert!(decode(json!(-1.0), false).is_err());
        let uniform =
            json!({"type": "minecraft:uniform", "min_inclusive": -1.0, "max_exclusive": 4.0});
        assert!(decode(uniform, false).is_err());
    }
}
