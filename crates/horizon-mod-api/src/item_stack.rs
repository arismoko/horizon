//! Owned item values. Registry-dependent validation belongs to the game's write boundary.
use crate::ItemKey;
use horizon_core::nbt::{Compound, JavaString, Tag};
use std::num::NonZeroU32;

/// A typed descriptor for one component in an item's component patch.
/// Implementations own the conversion between their value and Minecraft NBT.
pub trait Component {
    /// The value returned by this descriptor.
    type Value;
    /// Canonical component name, including its namespace.
    fn id(&self) -> &str;
    /// Decode a component value, refusing an incompatible NBT representation.
    fn decode(&self, value: &Tag) -> Option<Self::Value>;
    /// Encode a component value.
    fn encode(&self, value: Self::Value) -> Tag;
}

/// An owned component patch, independent of the engine's component representation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[horizon_mod_api_macros::wasm(via = Compound)]
pub struct Components {
    patch: Compound,
}

impl Components {
    /// Borrow the raw component patch at an NBT boundary.
    pub fn as_nbt(&self) -> &Compound {
        &self.patch
    }
}
impl From<Compound> for Components {
    fn from(patch: Compound) -> Self {
        Self { patch }
    }
}
impl From<&Components> for Compound {
    fn from(value: &Components) -> Self {
        value.patch.clone()
    }
}

/// A nonempty, owned item value. Maxima are checked when the game accepts a write,
/// not by this record: item definitions and effective component overrides belong to the game.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[horizon_mod_api_macros::wasm(via = StackRecord)]
pub struct ItemStack {
    item: ItemKey,
    count: NonZeroU32,
    damage: Option<u32>,
    components: Option<Box<Components>>,
}

/// Plain-data representation of an owned stack at serialization boundaries.
/// Decoding validates count through [`ItemStack::new`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct StackRecord {
    /// Process-local registry key; not a persistent Minecraft item identifier.
    pub item: ItemKey,
    /// A nonzero number of items.
    pub count: u32,
    /// Numeric override: None inherits the prototype unless the patch explicitly
    /// removes damage; Some(0) is a present zero-damage component.
    pub damage: Option<u32>,
    /// Component patch, absent for ordinary stacks.
    pub components: Option<Box<Components>>,
}

/// A stack operation could not represent the requested count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StackError {
    /// Empty stacks are represented by `None`, not by a zero-count item.
    Empty,
    /// Adding items would overflow the record's count.
    CountOverflow,
    /// Damage must be a nonnegative integer representable by u32.
    InvalidDamage,
    /// A patch and the damage field (or a patch value and removal) disagree.
    ConflictingDamage,
}
impl std::fmt::Display for StackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("an item stack must be nonempty"),
            Self::CountOverflow => f.write_str("item stack count overflow"),
            Self::InvalidDamage => f.write_str("invalid item damage"),
            Self::ConflictingDamage => f.write_str("conflicting item damage"),
        }
    }
}
impl std::error::Error for StackError {}

/// Why the game refused to accept an owned stack at a write boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StackWriteError {
    /// The host has no game backend for this operation.
    Unavailable,
    /// The process-local key is not present in this game's item registry.
    UnknownItem,
    /// The count is above the effective stack limit.
    CountExceeded {
        /// Effective maximum, including component overrides.
        maximum: u32,
    },
    /// The damage is above the effective durability limit.
    DamageExceeded {
        /// Effective maximum, including component overrides.
        maximum: u32,
    },
    /// A maximum component is not a valid nonnegative integer.
    InvalidMaximum,
    /// An engine result contained a zero-count stack instead of an empty slot.
    EmptyEngineStack,
    /// An engine snapshot's component patch was malformed or conflicted with its damage.
    InvalidComponents {
        /// The record-level normalization refusal.
        reason: StackError,
    },
}

impl crate::NoBackend for StackWriteError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// The two nonempty portions of a consuming split. Either portion can be absent.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct StackSplit {
    /// Up to the requested number of items.
    pub taken: Option<ItemStack>,
    /// Items left after taking that portion.
    pub remainder: Option<ItemStack>,
}

impl ItemStack {
    /// Construct a nonempty stack. Registry maxima are validated by the game on writes.
    pub fn new(item: ItemKey, count: u32) -> Option<Self> {
        Some(Self {
            item,
            count: NonZeroU32::new(count)?,
            damage: None,
            components: None,
        })
    }
    /// Construct a single item without a component patch.
    pub fn of(item: ItemKey) -> Self {
        Self {
            item,
            count: NonZeroU32::MIN,
            damage: None,
            components: None,
        }
    }
    /// The process-local identity of this item.
    pub fn item(&self) -> ItemKey {
        self.item
    }
    /// The nonzero item count.
    pub fn count(&self) -> u32 {
        self.count.get()
    }
    /// Change the count without consulting the game. A refusal leaves the value unchanged.
    pub fn set_count(&mut self, count: u32) -> Result<(), StackError> {
        self.count = NonZeroU32::new(count).ok_or(StackError::Empty)?;
        Ok(())
    }
    /// Stored damage, or zero when no numeric damage override is present.
    /// A plain stack inherits its item's default. Some(0) is an explicit present zero;
    /// [`Self::clear_damage`] records removal of the default component instead.
    pub fn damage(&self) -> u32 {
        self.damage.unwrap_or(0)
    }
    /// The numeric damage override: None inherits the prototype unless the patch
    /// contains an explicit removal; Some(0) is a present zero-damage component.
    pub fn damage_component(&self) -> Option<u32> {
        self.damage
    }
    /// Set damage. The game checks the effective maximum when accepting this value.
    pub fn set_damage(&mut self, damage: u32) {
        self.damage = Some(damage);
        if let Some(patch) = &mut self.components {
            patch.patch.remove(&JavaString::from("!minecraft:damage"));
            if patch.patch.is_empty() {
                self.components = None;
            }
        }
    }
    /// Remove the effective damage component, including an inherited default.
    /// The patch stores a real NBT removal, never a second numeric damage value.
    pub fn clear_damage(&mut self) {
        self.remove_component_value("minecraft:damage", true);
    }
    /// Whether a nonzero numeric damage override is stored on the stack.
    pub fn is_damaged(&self) -> bool {
        self.damage.is_some_and(|damage| damage != 0)
    }
    /// Add items. Overflow leaves the stack unchanged; registry maxima are checked on writes.
    pub fn grow(&mut self, count: u32) -> Result<(), StackError> {
        let count = self
            .count()
            .checked_add(count)
            .ok_or(StackError::CountOverflow)?;
        self.set_count(count)
    }
    /// Remove items, consuming the value so removing everything needs no empty sentinel.
    pub fn shrink(mut self, count: u32) -> Option<Self> {
        self.count = NonZeroU32::new(self.count().saturating_sub(count))?;
        Some(self)
    }
    /// Take up to `count` items. A full split moves the patch without cloning it.
    pub fn split(mut self, count: u32) -> StackSplit {
        if count == 0 {
            return StackSplit {
                taken: None,
                remainder: Some(self),
            };
        }
        if count >= self.count() {
            return StackSplit {
                taken: Some(self),
                remainder: None,
            };
        }
        let mut taken = self.clone();
        taken.count = NonZeroU32::new(count).expect("partial split is nonzero");
        self.count = NonZeroU32::new(self.count() - count).expect("partial split leaves items");
        StackSplit {
            taken: Some(taken),
            remainder: Some(self),
        }
    }
    /// Compare item identities, independent of count and components.
    pub fn same_item(&self, other: &Self) -> bool {
        self.item == other.item
    }
    /// Compare component patches and damage, independent of count and item identity.
    pub fn same_components(&self, other: &Self) -> bool {
        self.damage == other.damage && self.components == other.components
    }
    /// Whether these values have the same item and component patch.
    /// Capacity is a registry-dependent write-boundary check, not a promise of this predicate.
    pub fn is_stackable_with(&self, other: &Self) -> bool {
        self.same_item(other) && self.same_components(other)
    }
    /// Read a component with its typed descriptor.
    pub fn get<C: Component>(&self, component: &C) -> Option<C::Value> {
        if component.id() == "minecraft:damage" {
            return self
                .damage
                .and_then(|damage| component.decode(&unsigned_tag(damage)));
        }
        component.decode(
            self.components
                .as_ref()?
                .patch
                .get(&JavaString::from(component.id()))?,
        )
    }
    /// Replace a component value. Only stacks with a patch allocate component storage.
    pub fn set<C: Component>(&mut self, component: &C, value: C::Value) -> Result<(), StackError> {
        self.set_component_value(component.id(), component.encode(value))
    }
    pub(crate) fn set_component_value(&mut self, id: &str, value: Tag) -> Result<(), StackError> {
        if id == "minecraft:damage" {
            self.set_damage(damage_value(&value)?);
            return Ok(());
        }
        if id == "!minecraft:damage" {
            if !matches!(&value, Tag::Compound(value) if value.is_empty()) {
                return Err(StackError::InvalidDamage);
            }
            if self.damage.is_some() {
                return Err(StackError::ConflictingDamage);
            }
        }
        let patch = &mut self.components.get_or_insert_with(Default::default).patch;
        patch.remove(&JavaString::from(format!("!{id}").as_str()));
        patch.insert(JavaString::from(id), value);
        Ok(())
    }
    /// Remove an override, releasing the patch allocation when its last entry is removed.
    pub fn remove<C: Component>(&mut self, component: &C) {
        self.remove_component_value(component.id(), false);
    }
    pub(crate) fn remove_component_value(&mut self, id: &str, removal: bool) {
        if id == "minecraft:damage" {
            self.damage = None;
        }
        if removal {
            self.components.get_or_insert_with(Default::default);
        }
        if let Some(components) = &mut self.components {
            components.patch.remove(&JavaString::from(id));
            components
                .patch
                .remove(&JavaString::from(format!("!{id}").as_str()));
            if removal {
                components.patch.insert(
                    JavaString::from(format!("!{id}").as_str()),
                    Tag::Compound(Compound::new()),
                );
            }
            if components.patch.is_empty() {
                self.components = None;
            }
        }
    }
    /// The component patch, if this stack has overrides.
    pub fn components(&self) -> Option<&Components> {
        self.components.as_deref()
    }
    /// Replace the component patch at an NBT conversion boundary.
    /// Damage is normalized into the scalar field. Malformed or conflicting values
    /// refuse the entire replacement and leave the stack unchanged.
    pub fn set_components(&mut self, mut patch: Compound) -> Result<(), StackError> {
        let damage = patch.remove(&JavaString::from("minecraft:damage"));
        let removal = patch.get(&JavaString::from("!minecraft:damage"));
        if removal.is_some_and(|value| !matches!(value, Tag::Compound(value) if value.is_empty())) {
            return Err(StackError::InvalidDamage);
        }
        let removed = removal.is_some();
        let next = match damage {
            Some(value) => {
                let value = damage_value(&value)?;
                if removed || self.damage.is_some_and(|damage| damage != value) {
                    return Err(StackError::ConflictingDamage);
                }
                Some(value)
            }
            None => self.damage,
        };
        if removed && next.is_some() {
            return Err(StackError::ConflictingDamage);
        }
        self.damage = next;
        self.components = (!patch.is_empty()).then(|| Box::new(Components::from(patch)));
        Ok(())
    }
    /// Decode the item NBT escape using this host's load-resolved item ids.
    /// Unknown ids, zero counts and malformed/conflicting damage return None.
    /// Effective maxima remain the game's write-boundary responsibility.
    pub fn from_nbt(host: &dyn crate::Host, value: &Compound) -> Option<Self> {
        let Tag::String(id) = value.get(&JavaString::from("id"))? else {
            return None;
        };
        let id = crate::Identifier::parse(&id.to_string_lossy()).ok()?;
        let item = host.item_key(&id)?;
        let count = match value.get(&JavaString::from("count")) {
            None => 1,
            Some(value) => damage_value(value).ok()?,
        };
        let mut stack = Self::new(item, count)?;
        match value.get(&JavaString::from("components")) {
            Some(Tag::Compound(patch)) => stack.set_components(patch.clone()).ok()?,
            None => {}
            _ => return None,
        }
        Some(stack)
    }
    /// Encode the item NBT escape using this host's canonical id, never a process-local key.
    /// Returns None when id metadata is unavailable. Only components present on the
    /// stack are written; effective defaults and maxima belong to the items host area.
    pub fn to_nbt(&self, host: &dyn crate::Host) -> Option<Compound> {
        let id = self.item.id(host);
        if id.is_empty() {
            return None;
        }
        let mut value = Compound::new();
        value.insert(JavaString::from("id"), Tag::String(id.as_str().into()));
        value.insert(JavaString::from("count"), unsigned_tag(self.count()));
        let mut patch = self
            .components()
            .map(|patch| patch.as_nbt().clone())
            .unwrap_or_default();
        if let Some(damage) = self.damage {
            patch.insert(JavaString::from("minecraft:damage"), unsigned_tag(damage));
        }
        if !patch.is_empty() {
            value.insert(JavaString::from("components"), Tag::Compound(patch));
        }
        Some(value)
    }
}

pub(crate) fn unsigned_tag(value: u32) -> Tag {
    i32::try_from(value).map_or_else(|_| Tag::Long(i64::from(value)), Tag::Int)
}

fn damage_value(value: &Tag) -> Result<u32, StackError> {
    let value = match value {
        Tag::Byte(value) => i64::from(*value),
        Tag::Short(value) => i64::from(*value),
        Tag::Int(value) => i64::from(*value),
        Tag::Long(value) => *value,
        _ => return Err(StackError::InvalidDamage),
    };
    u32::try_from(value).map_err(|_| StackError::InvalidDamage)
}
impl From<&ItemStack> for StackRecord {
    fn from(value: &ItemStack) -> Self {
        Self {
            item: value.item,
            count: value.count(),
            damage: value.damage,
            components: value.components.clone(),
        }
    }
}
impl TryFrom<StackRecord> for ItemStack {
    type Error = StackError;
    fn try_from(value: StackRecord) -> Result<Self, Self::Error> {
        let mut stack = Self::new(value.item, value.count).ok_or(StackError::Empty)?;
        stack.damage = value.damage;
        if let Some(patch) = value.components {
            stack.set_components(patch.patch)?;
        }
        Ok(stack)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    include!("item_stack_damage_tests.rs");

    struct Number;
    impl Component for Number {
        type Value = i32;
        fn id(&self) -> &str {
            "test:number"
        }
        fn decode(&self, value: &Tag) -> Option<i32> {
            match value {
                Tag::Int(value) => Some(*value),
                _ => None,
            }
        }
        fn encode(&self, value: i32) -> Tag {
            Tag::Int(value)
        }
    }

    #[test]
    fn count_validation_is_atomic_and_registry_independent() {
        let key = ItemKey(0);
        assert!(ItemStack::new(key, 0).is_none());
        let mut stack = ItemStack::new(key, u32::MAX).unwrap();
        assert_eq!(stack.grow(1), Err(StackError::CountOverflow));
        assert_eq!(stack.set_count(0), Err(StackError::Empty));
        assert_eq!(stack.count(), u32::MAX);
        stack.set_damage(u32::MAX);
        assert_eq!(stack.damage(), u32::MAX);
    }

    #[test]
    fn splits_conserve_items_without_an_empty_sentinel() {
        for n in [0, 1, 4, 5, u32::MAX] {
            let mut stack = ItemStack::new(ItemKey(0), 4).unwrap();
            stack.set(&Number, 17).unwrap();
            let split = stack.split(n);
            assert_eq!(split.taken.as_ref().map_or(0, ItemStack::count), n.min(4));
            assert_eq!(
                split.remainder.as_ref().map_or(0, ItemStack::count),
                4 - n.min(4)
            );
            for portion in [split.taken, split.remainder].into_iter().flatten() {
                assert_eq!(portion.get(&Number), Some(17));
            }
        }
        assert!(ItemStack::of(ItemKey(0)).shrink(1).is_none());
    }

    #[test]
    fn patches_are_lazy_and_comparisons_ignore_counts() {
        let mut stack = ItemStack::of(ItemKey(0));
        assert!(stack.components().is_none());
        stack.set(&Number, 19).unwrap();
        let mut other = stack.clone();
        other.grow(3).unwrap();
        assert!(stack.is_stackable_with(&other));
        other.remove(&Number);
        assert!(!stack.same_components(&other));
        assert!(other.components().is_none());
        stack.remove(&Number);
        assert!(stack.is_stackable_with(&other));
    }

    #[test]
    fn record_decode_refuses_zero_and_normalizes_empty_patches() {
        let mut record = StackRecord::from(&ItemStack::of(ItemKey(0)));
        record.components = Some(Box::default());
        assert!(
            ItemStack::try_from(record.clone())
                .unwrap()
                .components()
                .is_none()
        );
        record.count = 0;
        assert_eq!(ItemStack::try_from(record), Err(StackError::Empty));
    }
}
