//! The live attribute map of a living entity, shared by the player, mobs,
//! armor stands and the dragon: vanilla's `AttributeMap` over the type's
//! defaults, the `attributes` NBT list (`AttributeMap.pack` and `apply`) and
//! the item modifiers of `LivingEntity.collectEquipmentChanges`.
//!
//! A player owns its map from the start. The others build theirs on first use
//! (an item with attribute modifiers, a read through a `LivingRef`, the save),
//! because most mobs never need one and a map costs a few KiB.
use crate::{Result, mobs::finalize::EquipmentRead};
use std::{cell::OnceCell, collections::BTreeMap, convert::Infallible, fmt, io, sync::Arc};
use sunrise_game_data::nbt_value::{float, string, text};
use sunrise_items::{
    equipment::Equipment,
    items::{ItemStack, Items},
};
use {
    sunrise_core::{equipment_slot::EquipmentSlot, nbt::Tag},
    sunrise_world::{
        attribute_map::ValueKind,
        attributes::{
            AttributeError, Definition, Instance, Modifier, Operation, Packed,
            entity::Catalog,
            live::{Dirty, Identities, LiveAttributes},
        },
    },
};

/// An entity's attribute instances, as a player has them.
pub type Live = LiveAttributes<Box<dyn Identities>>;

pub fn attribute_error(error: impl fmt::Debug) -> io::Error {
    io::Error::other(format!("movement attribute: {error:?}"))
}

/// A map over `entity`'s type defaults, with no instance yet
/// (`new AttributeMap(DefaultAttributes.getSupplier(type))`). Holder hashes are
/// the registry order, instance identities count up from 1.
pub fn new_live(catalog: &Arc<Catalog>, entity: &str) -> Result<Live> {
    let hashes: Vec<i32> = (0..catalog.attributes().count() as i32).collect();
    let mut identity = 0i32;
    let next: Box<dyn Identities> = Box::new(move || {
        identity = identity.wrapping_add(1);
        identity
    });
    Ok(LiveAttributes::new(catalog.clone(), entity, &hashes, next).map_err(attribute_error)?)
}

/// `EquipmentSlotGroup.test`: whether an item modifier of slot `group` applies
/// in the worn or held `slot`.
pub fn modifier_applies(group: &str, slot: &str) -> bool {
    group == slot
        || group == "any"
        || (group == "hand" && matches!(slot, "mainhand" | "offhand"))
        || (group == "armor" && matches!(slot, "feet" | "legs" | "chest" | "head"))
}

fn operation_named(name: &str) -> Operation {
    match name {
        "add_multiplied_base" => Operation::AddMultipliedBase,
        "add_multiplied_total" => Operation::AddMultipliedTotal,
        _ => Operation::AddValue,
    }
}

/// Adds (or, with `present` false, removes) a transient modifier: the
/// `removeModifier(id)` then `addTransientModifier` of
/// `LivingEntity.collectEquipmentChanges`. An attribute the entity type does not
/// have is left alone.
pub fn set_modifier(
    attributes: &mut Live,
    attribute: &str,
    id: &str,
    amount: f64,
    operation: &str,
    present: bool,
) -> Result<()> {
    let Some(key) = attributes.key(attribute) else {
        return Ok(());
    };
    let Some(handle) = attributes
        .get_instance(Some(key))
        .map_err(attribute_error)?
    else {
        return Ok(());
    };
    let modifier = Modifier::new(id, amount, operation_named(operation))?;
    attributes
        .edit(&handle, |value, observer| {
            value.remove(modifier.id(), observer)?;
            if present {
                value.add_transient(modifier, observer)?;
            }
            Ok(())
        })
        .map_err(attribute_error)?;
    Ok(())
}

/// `AttributeMap.pack` in `AttributeInstance.Packed.LIST_CODEC`'s form: each
/// instance's id, base and, with `modifiers`, its permanent modifiers.
pub fn pack(attributes: &Live, modifiers: bool) -> Vec<Tag> {
    attributes
        .instances()
        .map(|(_, key, instance)| packed_entry(key.name(), instance, modifiers))
        .collect()
}

/// One instance's entry of `pack`.
fn packed_entry(name: &str, instance: &Instance, modifiers: bool) -> Tag {
    let packed = instance.pack();
    let mut entry = vec![
        ("id", string(name)),
        ("base", Tag::Double(packed.base.to_bits())),
    ];
    // `modifiers` is an optional field whose default is the empty list.
    if modifiers && !packed.modifiers.is_empty() {
        let modifiers = packed
            .modifiers
            .iter()
            .map(|m| {
                let operation = match m.operation() {
                    Operation::AddValue => "add_value",
                    Operation::AddMultipliedBase => "add_multiplied_base",
                    Operation::AddMultipliedTotal => "add_multiplied_total",
                };
                Tag::compound([
                    ("id", string(m.id())),
                    ("amount", Tag::Double(m.amount().to_bits())),
                    ("operation", string(operation)),
                ])
            })
            .collect();
        entry.push(("modifiers", Tag::List(modifiers)));
    }
    Tag::compound(entry)
}

/// `AttributeMap.apply` of a saved `attributes` list: each known attribute
/// takes its saved base and permanent modifiers; unknown ones are skipped.
pub fn apply_saved(attributes: &mut Live, saved: Option<&Tag>) -> Result<()> {
    let Some(Tag::List(list)) = saved else {
        return Ok(());
    };
    apply_list(attributes, list, |_| true)
}

/// Whether `entity`'s own code sets `attribute`'s base past the attribute's range, so a load
/// keeps the saved base as vanilla does: `AbstractCubeMob.setCubeMobHealth` (size², up to
/// 16129) and `MagmaCube.setSize`'s armor (size * 3). Any other base takes the #3728 clamp.
pub fn own_unbounded_base(entity: &str, attribute: &str) -> bool {
    matches!(
        (entity, attribute),
        (
            "minecraft:slime" | "minecraft:magma_cube",
            "minecraft:max_health"
        ) | ("minecraft:magma_cube", "minecraft:armor")
    )
}

/// [`apply_saved`] over a list, clamping a base (`loaded_base`) only where `clamp` says.
fn apply_list(attributes: &mut Live, list: &[Tag], clamp: impl Fn(&str) -> bool) -> Result<()> {
    for entry in list {
        let Some(key) = text(entry.get("id")).and_then(|id| attributes.key(&id)) else {
            continue;
        };
        let modifiers = match entry.get("modifiers") {
            Some(Tag::List(l)) => l
                .iter()
                .filter_map(|m| {
                    let operation = match text(m.get("operation"))?.as_str() {
                        "add_value" => Operation::AddValue,
                        "add_multiplied_base" => Operation::AddMultipliedBase,
                        "add_multiplied_total" => Operation::AddMultipliedTotal,
                        _ => return None,
                    };
                    Modifier::new(&text(m.get("id"))?, float(m.get("amount"))?, operation).ok()
                })
                .collect(),
            _ => Vec::new(),
        };
        let definition = attributes.catalog().attribute(key.attribute()).definition;
        let base = float(entry.get("base")).unwrap_or(0.0);
        let packed = Packed {
            base: if clamp(key.name()) {
                loaded_base(key.name(), definition, base)
            } else {
                base
            },
            modifiers,
        };
        let Some(handle) = attributes
            .get_instance(Some(key))
            .map_err(attribute_error)?
        else {
            continue;
        };
        attributes
            .edit(&handle, |value, observer| value.apply(&packed, observer))
            .map_err(attribute_error)?;
    }
    Ok(())
}

/// A saved base as a load takes it (#3728): one outside the attribute's
/// bounds, saved while a mod had widened them, is clamped into them, and the
/// first such base of each attribute in a world load is logged
/// ([`LOAD_CLAMPS`]). Any other base, NaN included, stays as saved, as
/// vanilla's `AttributeInstance.load` keeps it.
pub fn loaded_base(name: &str, definition: Definition, base: f64) -> f64 {
    loaded_base_in(&LOAD_CLAMPS, name, definition, base).0
}

/// [`loaded_base`] against the clamps `log` holds for its load, with whether
/// this clamp was the one logged.
fn loaded_base_in(log: &ClampLog, name: &str, definition: Definition, base: f64) -> (f64, bool) {
    let Some((min, max)) = definition.bounds() else {
        return (base, false);
    };
    if (min..=max).contains(&base) || base.is_nan() {
        return (base, false);
    }
    let logged = log.first(name);
    if logged {
        log::warn!(
            "Sunrise: a saved {name} base of {base} is outside its bounds [{min}, {max}] (a mod that widened them is not loaded): clamped"
        );
    }
    (base.clamp(min, max), logged)
}

/// The clamps of the world being loaded: [`loaded_base`] warns once per
/// attribute in each world load, and each world open starts a new load.
pub static LOAD_CLAMPS: ClampLog = ClampLog::new();

/// The attributes whose saved base one load has clamped and logged (#3728).
pub struct ClampLog(std::sync::Mutex<std::collections::BTreeSet<String>>);
impl ClampLog {
    pub const fn new() -> Self {
        Self(std::sync::Mutex::new(std::collections::BTreeSet::new()))
    }
    /// A new load: each attribute's first clamp is logged again.
    pub fn begin_load(&self) {
        self.names().clear();
    }
    /// Whether this is the first clamp of `name` in the load.
    fn first(&self, name: &str) -> bool {
        self.names().insert(name.to_owned())
    }
    fn names(&self) -> std::sync::MutexGuard<'_, std::collections::BTreeSet<String>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
impl Default for ClampLog {
    fn default() -> Self {
        Self::new()
    }
}

/// The attribute map of a mob, an armor stand or the dragon.
///
/// Nothing is allocated for an entity that wears nothing with modifiers and is
/// not asked: `map` and `worn` stay empty, and `saved` holds only the loaded
/// `attributes` entries nothing else models. Once built, by a read too, the
/// map stays until `invalidate` drops it.
#[derive(Clone, Default)]
pub struct LivingAttributes {
    map: OnceCell<Box<Live>>,
    worn: Option<Box<WornEquipment>>,
    /// The loaded `attributes` entries, applied when the map is built.
    saved: Option<Box<[Tag]>>,
    /// The transient modifiers mods added, by attribute: not saved, and put
    /// back when the map is built again.
    transient: Option<Box<Vec<(Arc<str>, Modifier)>>>,
}
impl fmt::Debug for LivingAttributes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LivingAttributes")
            .field("live", &self.map.get().is_some())
            .field("saved", &self.saved.as_ref().map_or(0, |s| s.len()))
            .field("transient", &self.transient.as_ref().map_or(0, |t| t.len()))
            .finish()
    }
}
/// One slot delta. Its previous value is still in the history until application.
pub type EquipmentDelta = (EquipmentSlot, Option<ItemStack>);

/// The deltas one body tick found, held by slot: the slot set is fixed, so collecting them
/// allocates nothing, and reading them back is in `EquipmentSlot` order.
#[derive(Default)]
pub struct EquipmentDeltas([Option<Option<ItemStack>>; EquipmentSlot::ALL.len()]);

impl EquipmentDeltas {
    pub fn set(&mut self, slot: EquipmentSlot, now: Option<ItemStack>) {
        self.0[slot as usize] = Some(now);
    }
    fn take(&mut self, slot: EquipmentSlot) -> Option<Option<ItemStack>> {
        self.0[slot as usize].take()
    }
    pub fn is_empty(&self) -> bool {
        self.0.iter().all(Option::is_none)
    }
    fn slots(&self) -> impl Iterator<Item = EquipmentSlot> + '_ {
        EquipmentSlot::ALL
            .into_iter()
            .filter(|slot| self.0[*slot as usize].is_some())
    }
    fn into_deltas(self) -> impl Iterator<Item = EquipmentDelta> {
        EquipmentSlot::ALL
            .into_iter()
            .zip(self.0)
            .filter_map(|(slot, now)| Some((slot, now?)))
    }
}

#[derive(Clone, Default, PartialEq)]
struct WornEquipment {
    /// The items whose modifiers the map holds. Normally also `lastEquipmentItems`.
    applied: Equipment,
    /// A synchronous write or map invalidation can advance the live modifiers
    /// before the next body tick. Keep only the displaced slots, so detection
    /// still compares against what the last body tick observed.
    before_tick: Vec<EquipmentDelta>,
}

impl WornEquipment {
    fn remember(&mut self, slot: EquipmentSlot, previous: Option<ItemStack>) {
        if !self.before_tick.iter().any(|(saved, _)| *saved == slot) {
            self.before_tick.push((slot, previous));
        }
    }
}

/// A snapshot of the modifier rows and equipment detection cache, independent
/// of base values. An invalidated map still has its current seed values.
pub struct ModifierSnapshot {
    worn: Option<Box<WornEquipment>>,
    rows: BTreeMap<(String, String), ModifierState>,
}

#[derive(Clone)]
struct ModifierState {
    modifier: Modifier,
    permanent: bool,
}

impl PartialEq for ModifierState {
    fn eq(&self, other: &Self) -> bool {
        self.permanent == other.permanent
            && self.modifier.operation() == other.modifier.operation()
            && self.modifier.amount().to_bits() == other.modifier.amount().to_bits()
    }
}

/// The live modifier edits that a transaction owns, in refresh order.
#[derive(Default)]
pub struct ModifierChanges {
    worn_before: Option<Option<Box<WornEquipment>>>,
    modifiers: Vec<ModifierChange>,
}

struct ModifierChange {
    attribute: String,
    id: String,
    before: Option<ModifierState>,
    after: Option<ModifierState>,
}

impl ModifierChanges {
    pub fn merge(&mut self, child: Self) {
        if self.worn_before.is_none() {
            self.worn_before = child.worn_before;
        }
        self.modifiers.extend(child.modifiers);
    }

    /// Explicit writes are not part of a gear transaction, even when they
    /// remove a modifier or replace it with an equal value.
    pub fn keep_explicit_write(&mut self, attribute: &str, id: &str) {
        self.modifiers
            .retain(|m| m.attribute != attribute || m.id != id);
    }
}

impl ModifierSnapshot {
    fn from_map(map: &Live, worn: Option<Box<WornEquipment>>) -> Self {
        let rows = map
            .instances()
            .flat_map(|(_, key, value)| {
                value.modifiers().iter().map(move |modifier| {
                    let id = modifier.id();
                    (
                        (key.name().to_owned(), id.to_owned()),
                        ModifierState {
                            modifier: modifier.clone(),
                            permanent: value.permanent_modifiers().iter().any(|m| m.id() == id),
                        },
                    )
                })
            })
            .collect();
        Self { worn, rows }
    }

    /// Keep only the rows and detection cache a single step actually changed.
    pub fn diff(&self, after: &Self) -> Option<ModifierChanges> {
        let mut changes = ModifierChanges {
            worn_before: (self.worn != after.worn).then(|| self.worn.clone()),
            ..Default::default()
        };
        for (key, before) in &self.rows {
            let after = after.rows.get(key);
            if Some(before) != after {
                let (attribute, id) = key.clone();
                changes.modifiers.push(ModifierChange {
                    attribute,
                    id,
                    before: Some(before.clone()),
                    after: after.cloned(),
                });
            }
        }
        for (key, after) in &after.rows {
            if !self.rows.contains_key(key) {
                let (attribute, id) = key.clone();
                changes.modifiers.push(ModifierChange {
                    attribute,
                    id,
                    before: None,
                    after: Some(after.clone()),
                });
            }
        }
        (changes.worn_before.is_some() || !changes.modifiers.is_empty()).then_some(changes)
    }
}

/// Rewind only the rows a write owned. Walking the whole before-order anchors
/// restored rows to surviving neighbours, even if a later explicit write moved
/// or removed a row. Among those later writes the current order wins.
pub fn restore_modifier_order<Row: Clone>(
    current: &mut Vec<Row>,
    before: Vec<Row>,
    owned: impl Fn(&Row) -> bool,
    same_key: impl Fn(&Row, &Row) -> bool,
    same_row: impl Fn(&Row, &Row) -> bool,
) -> bool {
    let mut restored: Vec<_> = current.iter().filter(|row| !owned(row)).cloned().collect();
    let mut at = 0;
    for old in before {
        if owned(&old) {
            restored.insert(at, old);
            at += 1;
        } else if let Some(index) = restored.iter().position(|row| same_key(row, &old)) {
            at = at.max(index + 1);
        }
    }
    if current.len() == restored.len() && current.iter().zip(&restored).all(|(a, b)| same_row(a, b))
    {
        return false;
    }
    *current = restored;
    true
}

/// The transient seed in its insertion order, independent of the built equipment map.
#[derive(Clone, Default)]
pub struct TransientModifiers(Vec<(Arc<str>, Modifier)>);

impl TransientModifiers {
    pub fn forget(&mut self, attribute: &str, id: &str) {
        self.0
            .retain(|(name, m)| &**name != attribute || m.id() != id);
    }
}

impl LivingAttributes {
    /// Keeps the entries of a loaded `attributes` list that `keep` accepts
    /// (`LivingEntity.readAdditionalSaveData`), for the map to take.
    pub fn load(&mut self, saved: Option<&Tag>, keep: impl Fn(&str) -> bool) {
        let Some(Tag::List(list)) = saved else {
            return;
        };
        let kept: Vec<Tag> = list
            .iter()
            .filter(|e| text(e.get("id")).is_some_and(|id| keep(&id)))
            .cloned()
            .collect();
        self.saved = (!kept.is_empty()).then(|| kept.into_boxed_slice());
        self.invalidate();
    }

    /// Forgets the loaded attributes the entity type has no instance of, as
    /// `AttributeMap.apply` skips them and `pack` never writes them.
    pub fn retain_supplier(&mut self, catalog: &Catalog, entity: &str) {
        let Some(saved) = self.saved.as_deref() else {
            return;
        };
        let kept: Vec<Tag> = saved
            .iter()
            .filter(|e| {
                text(e.get("id"))
                    .and_then(|id| catalog.id(&id))
                    .is_some_and(|id| catalog.default_base(entity, id).is_some())
            })
            .cloned()
            .collect();
        self.saved = (!kept.is_empty()).then(|| kept.into_boxed_slice());
    }

    /// Whether a previously equipped slot still needs comparison.
    pub fn has_worn(&self) -> bool {
        self.worn.is_some()
    }

    /// Drops the map because its seed changed, retaining the body tick's
    /// comparison baseline. The next refresh puts the item modifiers back.
    pub fn invalidate(&mut self) {
        self.map.take();
        if let Some(worn) = self.worn.as_mut() {
            for slot in EquipmentSlot::ALL {
                if let Some(previous) = worn.applied[slot as usize].take() {
                    worn.remember(slot, Some(previous));
                }
            }
        }
    }

    /// Neither the modifier map nor any applied item modifiers are cached.
    pub fn is_invalidated(&self) -> bool {
        self.map.get().is_none() && self.worn.as_ref().is_none_or(|w| w.applied.is_empty())
    }

    /// The map, if it is built.
    pub fn built(&self) -> Option<&Live> {
        self.map.get().map(|map| &**map)
    }

    fn build(&self, catalog: &Arc<Catalog>, entity: &str, seed: Vec<Tag>) -> Result<Live> {
        let mut live = new_live(catalog, entity)?;
        // The seed is the entity's own record, which no load clamp touches.
        apply_list(&mut live, &seed, |_| false)?;
        if let Some(saved) = &self.saved {
            apply_list(&mut live, saved, |name| !own_unbounded_base(entity, name))?;
        }
        for (name, modifier) in self.transient.iter().flat_map(|t| t.iter()) {
            let Some(key) = live.key(name) else {
                continue;
            };
            let Some(handle) = live.get_instance(Some(key)).map_err(attribute_error)? else {
                continue;
            };
            live.edit(&handle, |value, observer| {
                value.add_or_update_transient(modifier.clone(), observer)
            })
            .map_err(attribute_error)?;
        }
        Ok(live)
    }

    /// Edits `name`'s instance (`AttributeMap.getInstance`, then the edit),
    /// building the map first: a mod's write. None for an attribute the
    /// entity type does not have. With `persist`, the instance's base and
    /// permanent modifiers then replace its saved entry, so a rebuilt map and
    /// the save keep them; without, another record (`finalize`'s) holds them.
    pub fn edit<R>(
        &mut self,
        catalog: &Arc<Catalog>,
        entity: &str,
        name: &str,
        persist: bool,
        seed: impl FnOnce() -> Vec<Tag>,
        edit: impl FnOnce(
            &mut Instance,
            &mut Dirty<'_>,
        ) -> std::result::Result<R, AttributeError<Infallible>>,
    ) -> Option<R> {
        if self.map.get().is_none() {
            let built = self.build(catalog, entity, seed()).ok()?;
            self.map.get_or_init(|| Box::new(built));
        }
        let map = self.map.get_mut()?;
        let key = map.key(name)?;
        let handle = map.get_instance(Some(key)).ok()??;
        let out = map.edit(&handle, edit).ok()?;
        if persist {
            let entry = packed_entry(name, map.instance(&handle).ok()?, true);
            let mut saved = self.saved.take().map(Vec::from).unwrap_or_default();
            match saved
                .iter_mut()
                .find(|t| text(t.get("id")).as_deref() == Some(name))
            {
                Some(t) => *t = entry,
                None => saved.push(entry),
            }
            self.saved = Some(saved.into_boxed_slice());
        }
        Some(out)
    }

    /// Keeps a mod's transient modifier on `name` for the next build, or
    /// with `None` forgets the one with id `id`.
    pub fn keep_transient(&mut self, name: &str, id: &str, modifier: Option<Modifier>) {
        if modifier.is_none() && self.transient.is_none() {
            return;
        }
        let list = self.transient.get_or_insert_with(Box::default);
        list.retain(|(n, m)| !(&**n == name && m.id() == id));
        if let Some(modifier) = modifier {
            list.push((Arc::from(name), modifier));
        }
        if list.is_empty() {
            self.transient = None;
        }
    }

    pub fn transient_modifiers(&self) -> TransientModifiers {
        TransientModifiers(self.transient.as_deref().cloned().unwrap_or_default())
    }

    pub fn restore_transient_modifiers(
        &mut self,
        before: TransientModifiers,
        owned: impl Fn(&str, &str) -> bool,
    ) -> bool {
        let mut current = self.transient.take().unwrap_or_default();
        let changed = restore_modifier_order(
            &mut current,
            before.0,
            |(name, m)| owned(name, m.id()),
            |(name, m), (other, n)| name == other && m.id() == n.id(),
            |(name, m), (other, n)| name == other && m.same_identity(n),
        );
        self.transient = (!current.is_empty()).then_some(current);
        changed
    }

    /// `AttributeMap.getValue` or `getBaseValue` of `name`: None for an
    /// attribute the entity type does not have. `seed` is what the entity keeps
    /// of its attributes elsewhere, in the `attributes` list's form; it is only
    /// asked for when the map is not built yet. The map built for the read is
    /// kept, so the next read allocates nothing.
    pub fn read(
        &self,
        catalog: &Arc<Catalog>,
        entity: &str,
        name: &str,
        kind: ValueKind,
        seed: impl FnOnce() -> Vec<Tag>,
    ) -> Option<f64> {
        let map = self.built_or_build(catalog, entity, seed)?;
        map.value(Some(map.key(name)?), kind).ok()
    }

    /// `AttributeInstance.getModifiers` of `name`, in the order they were
    /// added: None for an attribute the entity type does not have, an empty
    /// list for one no modifier is on (an instance is created only by a write,
    /// a worn item or the saved list). Read as `read` reads the value, over
    /// the same built map.
    pub fn modifiers(
        &self,
        catalog: &Arc<Catalog>,
        entity: &str,
        name: &str,
        seed: impl FnOnce() -> Vec<Tag>,
    ) -> Option<&[Modifier]> {
        self.built_or_build(catalog, entity, seed)?.modifiers(name)
    }

    /// The map, built from `seed` if this is its first use, and kept.
    pub fn built_or_build(
        &self,
        catalog: &Arc<Catalog>,
        entity: &str,
        seed: impl FnOnce() -> Vec<Tag>,
    ) -> Option<&Live> {
        match self.map.get() {
            Some(map) => Some(map),
            None => {
                let built = self.build(catalog, entity, seed()).ok()?;
                Some(self.map.get_or_init(|| Box::new(built)))
            }
        }
    }

    /// Whether early refreshes or invalidation left slots to settle on the tick.
    pub fn has_pending_equipment(&self) -> bool {
        self.worn
            .as_ref()
            .is_some_and(|w| !w.before_tick.is_empty())
    }

    /// `LivingEntity.lastEquipmentItems`, independent of early modifier refreshes.
    pub fn last_equipment(&self, slot: EquipmentSlot) -> Option<&ItemStack> {
        let worn = self.worn.as_ref()?;
        match worn.before_tick.iter().find(|(saved, _)| *saved == slot) {
            Some((_, previous)) => previous.as_ref(),
            None => worn.applied.get(slot as usize),
        }
    }

    /// `LivingEntity.collectEquipmentChanges` for living entities whose slot
    /// reads need no gear projection or call to a listener here.
    pub fn collect_equipment_changes(
        &mut self,
        catalog: &Arc<Catalog>,
        entity: &str,
        equipment: &Equipment,
        items: &Items,
        seed: impl FnOnce() -> Vec<Tag>,
    ) -> Result<()> {
        let read = |slot: EquipmentSlot| equipment.get(slot as usize).map(EquipmentRead::resident);
        let changes = equipment_deltas(read, |slot| self.last_equipment(slot));
        self.finish_equipment_tick(catalog, entity, items, changes, read, seed)
    }

    /// An immediate refresh compares against what the map already holds but
    /// does not consume the next body tick's observation.
    pub fn refresh_equipment<'a>(
        &mut self,
        catalog: &Arc<Catalog>,
        entity: &str,
        items: &Items,
        read: impl Fn(EquipmentSlot) -> Option<EquipmentRead<'a>>,
        seed: impl FnOnce() -> Vec<Tag>,
    ) -> Result<()> {
        let changes = equipment_deltas(read, |slot| self.worn.as_ref()?.applied.get(slot as usize));
        self.apply_equipment_deltas(catalog, entity, items, changes, true, seed)
    }

    /// On a body tick the detected deltas normally go straight to the map.
    /// Only slots already touched by an early refresh need reconciliation. A
    /// listener may have edited such a slot after it was observed; keep the
    /// observed value for the next tick without overwriting live modifiers.
    pub fn finish_equipment_tick<'a>(
        &mut self,
        catalog: &Arc<Catalog>,
        entity: &str,
        items: &Items,
        mut changes: EquipmentDeltas,
        read: impl Fn(EquipmentSlot) -> Option<EquipmentRead<'a>>,
        seed: impl FnOnce() -> Vec<Tag>,
    ) -> Result<()> {
        if let Some(worn) = self.worn.as_mut()
            && !worn.before_tick.is_empty()
        {
            for (slot, previous) in std::mem::take(&mut worn.before_tick) {
                let mut observed = changes.take(slot).unwrap_or(previous);
                let now = read(slot);
                let matches_observed =
                    EquipmentRead::matches_previous(now.as_ref(), observed.as_ref());
                if !EquipmentRead::matches_previous(now.as_ref(), worn.applied.get(slot as usize)) {
                    let current = if matches_observed {
                        observed.take()
                    } else {
                        now.map(|read| read.project().into_owned())
                    };
                    changes.set(slot, current);
                }
                if !matches_observed {
                    worn.before_tick.push((slot, observed));
                }
            }
        }
        self.apply_equipment_deltas(catalog, entity, items, changes, false, seed)
    }

    /// `LivingEntity.collectEquipmentChanges`: remove every changed slot's old
    /// modifiers before applying new ones, in `EquipmentSlot` order. An early
    /// refresh changes the live map now but never consumes a tick observation.
    fn apply_equipment_deltas(
        &mut self,
        catalog: &Arc<Catalog>,
        entity: &str,
        items: &Items,
        changes: EquipmentDeltas,
        early: bool,
        seed: impl FnOnce() -> Vec<Tag>,
    ) -> Result<()> {
        let mut seed = Some(seed);
        if let (Some(worn), Some(map)) = (&self.worn, self.map.get_mut()) {
            for slot in changes.slots() {
                if let Some(old) = worn.applied.get(slot as usize) {
                    apply_item(map, old, slot as usize, items, false)?;
                }
            }
        }
        for (slot, new) in changes.into_deltas() {
            if let Some(stack) = new.as_ref()
                && !crate::item_attributes::attribute_modifiers(stack, items).is_empty()
            {
                if self.map.get().is_none() {
                    let seed = seed.take().map_or_else(Vec::new, |f| f());
                    let built = self.build(catalog, entity, seed)?;
                    self.map.get_or_init(|| Box::new(built));
                }
                if let Some(map) = self.map.get_mut() {
                    apply_item(map, stack, slot as usize, items, true)?;
                }
            }
            let worn = self.worn.get_or_insert_with(Box::default);
            let old = std::mem::replace(&mut worn.applied[slot as usize], new);
            if early {
                worn.remember(slot, old);
            }
        }
        if self
            .worn
            .as_ref()
            .is_some_and(|w| w.applied.is_empty() && w.before_tick.is_empty())
        {
            self.worn = None;
        }
        Ok(())
    }

    /// Read the real live rows, not what a new item query would return.
    /// A lazy map contributes its seed rows without making the cache live.
    pub fn modifier_snapshot(
        &self,
        catalog: &Arc<Catalog>,
        entity: &str,
        seed: impl FnOnce() -> Vec<Tag>,
    ) -> Result<ModifierSnapshot> {
        let seeded;
        let map = if let Some(map) = self.built() {
            map
        } else {
            seeded = self.build(catalog, entity, seed())?;
            &seeded
        };
        Ok(ModifierSnapshot::from_map(map, self.worn.clone()))
    }

    /// What invalidation alone will discard: the live overlay over the current
    /// seed and the equipment detection state, never changes to the seed itself.
    pub fn modifier_invalidation_undo(
        &self,
        catalog: &Arc<Catalog>,
        entity: &str,
        seed: impl FnOnce() -> Vec<Tag>,
    ) -> Result<Option<ModifierChanges>> {
        let mut changes = if let Some(map) = self.built() {
            let seeded = self.build(catalog, entity, seed())?;
            ModifierSnapshot::from_map(map, None).diff(&ModifierSnapshot::from_map(&seeded, None))
        } else {
            None
        };
        if let Some(worn) = self.worn.as_ref().filter(|w| !w.applied.is_empty()) {
            changes.get_or_insert_with(Default::default).worn_before = Some(Some(worn.clone()));
        }
        Ok(changes)
    }

    /// Restores the recorded live rows, not the current answers to item queries.
    /// If an independent write discarded the map, its current seed still wins.
    pub fn undo_modifier_changes(
        &mut self,
        catalog: &Arc<Catalog>,
        entity: &str,
        changes: ModifierChanges,
        seed: impl FnOnce() -> Vec<Tag>,
    ) -> Result<()> {
        let ModifierChanges {
            worn_before,
            modifiers,
        } = changes;
        if let Some(worn) = worn_before {
            self.worn = worn;
        }
        if modifiers.is_empty() {
            return Ok(());
        }
        if self.map.get().is_none() {
            let built = self.build(catalog, entity, seed())?;
            self.map.get_or_init(|| Box::new(built));
        }
        let map = self
            .map
            .get_mut()
            .expect("equipment rollback has an attribute map");
        for ModifierChange {
            attribute,
            id,
            before,
            after,
        } in modifiers.into_iter().rev()
        {
            let Some(key) = map.key(&attribute) else {
                continue;
            };
            let Some(handle) = map.get_instance(Some(key)).map_err(attribute_error)? else {
                continue;
            };
            map.edit(&handle, |value, observer| {
                let current = value.modifier(&id).map(|modifier| ModifierState {
                    modifier: modifier.clone(),
                    permanent: value.permanent_modifiers().iter().any(|m| m.id() == id),
                });
                // Leave a later, independent modifier intact. The semantic
                // row comparison also works when an explicit write has rebuilt
                // the cache with new modifier identities.
                if current != after {
                    return Ok(());
                }
                value.remove(&id, observer)?;
                if let Some(before) = before {
                    if before.permanent {
                        value.add_permanent(before.modifier, observer)?;
                    } else {
                        value.add_transient(before.modifier, observer)?;
                    }
                }
                Ok(())
            })
            .map_err(attribute_error)?;
        }
        Ok(())
    }

    /// Writes what the map and the loaded entries say into a saved
    /// `attributes` list, by attribute id: `AttributeMap.pack`, permanent
    /// modifiers only (the item modifiers are transient).
    pub fn save_into(&self, list: &mut Vec<Tag>) {
        let mut put = |entry: Tag| {
            let id = text(entry.get("id"));
            match list.iter_mut().find(|t| text(t.get("id")) == id) {
                Some(t) => *t = entry,
                None => list.push(entry),
            }
        };
        for entry in self.saved.iter().flat_map(|s| s.iter()) {
            put(entry.clone());
        }
        if let Some(map) = self.map.get() {
            for entry in pack(map, true) {
                put(entry);
            }
        }
    }
}

fn equipment_deltas<'a, 'b>(
    read: impl Fn(EquipmentSlot) -> Option<EquipmentRead<'a>>,
    previous: impl Fn(EquipmentSlot) -> Option<&'b ItemStack>,
) -> EquipmentDeltas {
    let mut deltas = EquipmentDeltas::default();
    for slot in EquipmentSlot::ALL {
        let now = read(slot);
        if !EquipmentRead::matches_previous(now.as_ref(), previous(slot)) {
            deltas.set(slot, now.map(|read| read.project().into_owned()));
        }
    }
    deltas
}

fn apply_item(
    map: &mut Live,
    stack: &ItemStack,
    slot: usize,
    items: &Items,
    present: bool,
) -> Result<()> {
    let slot_name = sunrise_items::equipment::SLOT_NAMES[slot];
    for sunrise_items::item_data::ModifierRow {
        attribute,
        id,
        amount,
        operation,
        slot: group,
    } in crate::item_attributes::attribute_modifiers(stack, items).iter()
    {
        if modifier_applies(group, slot_name) {
            set_modifier(map, attribute, id, *amount, operation, present)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A catalog with one player attribute, bounded at `max`, and its map.
    fn player(id: &str, max: f64) -> Live {
        let bits = |v: f64| format!("{:016x}", v.to_bits());
        let catalog = Arc::new(
            Catalog::from_json(&serde_json::json!({
                "data_version": sunrise_world::DATA_VERSION,
                "attributes": [{"id": id, "default": bits(20.0), "min": bits(1.0), "max": bits(max), "syncable": true}],
                "entities": {"minecraft:player": {id: bits(20.0)}},
            }))
            .unwrap(),
        );
        new_live(&catalog, "minecraft:player").unwrap()
    }

    /// #3728: with a mod's widened bounds, a base past vanilla's survives a
    /// save and a load; without them, the load clamps the base itself.
    #[test]
    fn a_saved_base_past_the_bounds_survives_only_with_them() {
        let id = "test:widened_health";
        let mut widened = player(id, 1e6);
        let key = widened.key(id).unwrap();
        let handle = widened.get_instance(Some(key)).unwrap().unwrap();
        widened
            .edit(&handle, |value, observer| {
                value.set_base_value(5000.0, observer)
            })
            .unwrap();
        let saved = Tag::List(pack(&widened, true));
        let mut reloaded = player(id, 1e6);
        apply_saved(&mut reloaded, Some(&saved)).unwrap();
        assert_eq!(reloaded.computed_value_of(id).unwrap(), 5000.0);
        let mut removed = player(id, 1024.0);
        apply_saved(&mut removed, Some(&saved)).unwrap();
        let key = removed.key(id).unwrap();
        assert_eq!(removed.value(Some(key), ValueKind::Base).unwrap(), 1024.0);
        let definition = removed.catalog().definition(id).unwrap();
        assert!(loaded_base(id, definition, f64::NAN).is_nan());
    }

    /// #3728: a clamped saved base warns once per attribute in each load,
    /// and a second load warns again.
    #[test]
    fn a_clamped_base_warns_once_per_attribute_in_each_load() {
        let health = Definition::ranged(20.0, 1.0, 1024.0).unwrap();
        let armor = Definition::ranged(0.0, 0.0, 30.0).unwrap();
        let log = ClampLog::new();
        for load in 0..2 {
            log.begin_load();
            let first = loaded_base_in(&log, "minecraft:max_health", health, 5000.0);
            assert_eq!(first, (1024.0, true), "load {load}: the first clamp warns");
            let again = loaded_base_in(&log, "minecraft:max_health", health, 9000.0);
            assert_eq!(
                again,
                (1024.0, false),
                "load {load}: a duplicate is suppressed"
            );
            let other = loaded_base_in(&log, "minecraft:armor", armor, 100.0);
            assert_eq!(other, (30.0, true), "load {load}: another attribute warns");
            let kept = loaded_base_in(&log, "minecraft:max_health", health, 500.0);
            assert_eq!(kept, (500.0, false), "load {load}: a base in bounds stays");
        }
    }

    /// Held items apply their main-hand and off-hand modifiers (#4118, #4777).
    #[test]
    fn equipment_modifiers_follow_their_slot_group() {
        assert!(modifier_applies("mainhand", "mainhand"));
        assert!(modifier_applies("hand", "mainhand"));
        assert!(modifier_applies("any", "mainhand"));
        assert!(!modifier_applies("armor", "mainhand"));
        assert!(!modifier_applies("mainhand", "head"));
        assert!(modifier_applies("armor", "chest"));
        assert!(!modifier_applies("hand", "feet"));
        assert!(modifier_applies("offhand", "offhand"));
        assert!(modifier_applies("hand", "offhand"));
        assert!(modifier_applies("any", "offhand"));
        assert!(!modifier_applies("armor", "offhand"));
        assert!(!modifier_applies("mainhand", "offhand"));
        assert!(!modifier_applies("offhand", "mainhand"));
        // The body and saddle slots are not armor (`EquipmentSlotGroup.ARMOR`).
        assert!(modifier_applies("body", "body"));
        assert!(modifier_applies("saddle", "saddle"));
        assert!(!modifier_applies("armor", "body"));
        assert!(modifier_applies("any", "saddle"));
    }
}
