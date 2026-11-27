//! Live candidate backing the items loot evaluator without whole-stack cloning or
//! detached `LootStack` snapshots.
use crate::items::{DYNAMIC, ItemStack, Items, prototypes_of};
use serde_json::Value;
use std::sync::{Arc, OnceLock};
use sunrise_core::nbt::Tag;
use sunrise_world::loot::{
    ApiExplorationMap, ExplorationMap, LootExplorationView, LootListAccess, LootStackAccess,
    LootStackEditAccess, LootToolAccess,
};

/// One live candidate in the items loot evaluator.
///
/// The resident [`ItemStack`] stays in place: untouched modeled fields (such as a
/// written book's contents, bundle contents, or banner layers) are never rebuilt
/// or stripped when a custom function or global loot modifier leaves them alone.
/// Intermediate signed count/damage, map/exploration flags, and ordered component
/// edits stay live across consecutive custom functions and global loot modifiers.
#[derive(Clone, Debug)]
pub(crate) struct LootCandidate {
    stack: ItemStack,
    count: i32,
    damage: i32,
    map: bool,
    exploration: Option<Box<ExplorationMap>>,
    ominous_amplifier: Option<i32>,
    enchantments: OnceLock<Vec<(String, i32)>>,
    stew_effects: OnceLock<Vec<(String, i32)>>,
    components: OnceLock<Vec<(String, Option<Tag>)>>,
    removals: Vec<String>,
    component_edited: bool,
    names: Vec<(String, Value)>,
}

impl LootCandidate {
    pub(crate) fn from_stack(stack: ItemStack) -> Self {
        let count = stack.count as i32;
        let damage = stack.damage as i32;
        let ominous_amplifier = stack
            .data()
            .and_then(|d| d.ominous_amplifier)
            .map(i32::from);
        Self {
            stack,
            count,
            damage,
            map: false,
            exploration: None,
            ominous_amplifier,
            enchantments: OnceLock::new(),
            stew_effects: OnceLock::new(),
            components: OnceLock::new(),
            removals: Vec::new(),
            component_edited: false,
            names: Vec::new(),
        }
    }

    pub(crate) fn from_item(item: impl Into<Arc<str>>, count: i32) -> Self {
        let mut candidate = Self::from_stack(ItemStack {
            item: item.into(),
            count: count.max(0) as u32,
            damage: 0,
            extra: None,
        });
        candidate.count = count;
        candidate
    }

    pub(crate) fn from_loot_stack(value: &sunrise_world::loot::LootStack) -> ItemStack {
        let mut candidate = Self::from_item(value.item.as_str(), value.count);
        if value.damage != 0 {
            candidate.set_damage(value.damage);
        }
        if !value.enchantments.is_empty() {
            candidate.set_enchantments(value.enchantments.clone());
        }
        if value.potion.is_some() {
            candidate.set_potion_id(value.potion.clone());
        }
        if value.ominous_amplifier.is_some() {
            candidate.set_ominous_amplifier(value.ominous_amplifier);
        }
        if !value.stew_effects.is_empty() {
            candidate.set_stew_effects(value.stew_effects.clone());
        }
        if !value.names.is_empty() {
            candidate.set_names_values(value.names.clone());
        }
        if !value.components.is_empty() {
            candidate.set_components(value.components.clone());
        }
        candidate.into_item_stack_with(&|_, _| None)
    }

    pub(crate) fn item(&self) -> &str {
        &self.stack.item
    }

    pub(crate) fn is_dynamic_marker(&self) -> bool {
        self.stack.item.starts_with(DYNAMIC)
    }

    pub(crate) fn count(&self) -> i32 {
        self.count
    }

    pub(crate) fn set_signed_count(&mut self, count: i32) {
        self.count = count;
        self.stack.count = count.max(0) as u32;
    }

    /// Run a vanilla edit on the resident stack once it has the correct removal
    /// markers, preserving signed count/damage when vanilla leaves them unchanged.
    pub(crate) fn with_resident_mut<T>(
        &mut self,
        items: &Items,
        edit: impl FnOnce(&mut ItemStack) -> T,
    ) -> T {
        self.resolve_component_removals(&prototypes_of(items));
        let flushed_count = self.stack.count;
        let flushed_damage = self.stack.damage;
        let prev_count = self.count;
        let prev_damage = self.damage;
        let result = edit(&mut self.stack);
        if self.stack.count != flushed_count {
            self.count = self.stack.count as i32;
        } else {
            self.count = prev_count;
        }
        if self.stack.damage != flushed_damage {
            self.damage = self.stack.damage as i32;
        } else {
            self.damage = prev_damage;
        }
        self.ominous_amplifier = self
            .stack
            .data()
            .and_then(|d| d.ominous_amplifier)
            .map(i32::from);
        self.enchantments.take();
        self.stew_effects.take();
        self.components.take();
        self.removals.clear();
        result
    }

    pub(crate) fn max_stack(&mut self, items: &Items) -> i32 {
        self.resolve_component_removals(&prototypes_of(items));
        crate::bundle::max_stack(items, &self.stack) as i32
    }

    pub(crate) fn split_into(mut self, items: &Items, out: &mut Vec<Self>) {
        if self.count <= 1 || self.is_dynamic_marker() {
            out.push(self);
            return;
        }
        let max = self.max_stack(items).max(1);
        if self.count <= max {
            out.push(self);
            return;
        }
        let mut remaining = self.count;
        while remaining > max {
            let mut part = self.clone();
            part.count = max;
            part.stack.count = max as u32;
            out.push(part);
            remaining -= max;
        }
        self.count = remaining;
        self.stack.count = remaining as u32;
        out.push(self);
    }

    pub(crate) fn into_item_stack(self, items: &Items) -> ItemStack {
        self.into_item_stack_with(&prototypes_of(items))
    }

    pub(crate) fn into_item_stack_with(
        mut self,
        prototype_has: &impl Fn(&str, &str) -> Option<bool>,
    ) -> ItemStack {
        self.resolve_component_removals(prototype_has);
        self.stack
    }

    fn resolve_component_removals(&mut self, prototype_has: &impl Fn(&str, &str) -> Option<bool>) {
        // A codec write takes effect immediately. Only removal markers depend on
        // the final item's prototype, which the splitter and consumer know.
        for id in &self.removals {
            if let Some(has) = prototype_has(&self.stack.item, id) {
                self.stack
                    .data_mut()
                    .set_other(&format!("!{id}"), has.then(removal_marker));
            }
        }
        if self.component_edited || !self.removals.is_empty() {
            self.stack.tidy();
            self.component_edited = false;
        }
    }

    fn write_component(&mut self, id: &str, value: &Option<Tag>) {
        let base = component_key(id);
        self.component_edited = true;
        if id != base && value.is_none() {
            // A !key/None removes the marker, not the component.
            self.stack.data_mut().set_other(id, None);
            self.removals.retain(|key| key != base);
            return;
        }
        let marker = format!("!{base}");
        if id != base || value.is_none() {
            let data = self.stack.data_mut();
            data.set_loot_component(base, &None);
            if id != base {
                data.set_other(&marker, Some(removal_marker()));
            }
            if base == "minecraft:damage" {
                self.stack.damage = 0;
            }
            if !self.removals.iter().any(|key| key == base) {
                self.removals.push(base.to_owned());
            }
        } else {
            let data = self.stack.data_mut();
            data.set_loot_component(base, value);
            if !data.sets_component(base) {
                return;
            }
            data.set_other(&marker, None);
            self.removals.retain(|key| key != base);
            if base == "minecraft:damage" {
                self.stack.damage =
                    sunrise_world::datafix::dynamic::as_int(value.as_ref(), 0).max(0) as u32;
            }
        }
        match base {
            "minecraft:damage" => self.damage = self.stack.damage as i32,
            "minecraft:enchantments" | "minecraft:stored_enchantments"
                if base == self.enchantments_id() =>
            {
                self.enchantments.take();
            }
            "minecraft:suspicious_stew_effects" => {
                self.stew_effects.take();
            }
            "minecraft:ominous_bottle_amplifier" => {
                self.ominous_amplifier = self
                    .stack
                    .data()
                    .and_then(|d| d.ominous_amplifier)
                    .map(i32::from);
            }
            "minecraft:custom_name" | "minecraft:item_name" => {
                self.names.retain(|(key, _)| key != base);
                if let Some(tag) = self.stack.data().and_then(|d| d.other(base)) {
                    self.names.push((
                        base.to_owned(),
                        sunrise_game_data::scoreboard::tag_to_json(tag),
                    ));
                }
            }
            _ => {}
        }
    }

    fn write_typed_component(&mut self, id: &str, value: Option<Tag>) {
        // An ordered raw edit may already exist for the same component. A
        // later typed write supersedes it; never replay the old raw value.
        let _ = self.components_slice();
        self.components
            .get_mut()
            .expect("components initialized")
            .retain(|(key, _)| component_key(key) != id);
        self.write_component(id, &value);
    }

    fn enchantments_id(&self) -> &'static str {
        if &*self.stack.item == "minecraft:enchanted_book" {
            "minecraft:stored_enchantments"
        } else {
            "minecraft:enchantments"
        }
    }

    fn enchantments_slice(&self) -> &[(String, i32)] {
        self.enchantments.get_or_init(|| {
            self.stack
                .crafting_enchantments()
                .iter()
                .map(|(id, level)| (id.to_string(), *level as i32))
                .collect()
        })
    }

    fn stew_effects_slice(&self) -> &[(String, i32)] {
        self.stew_effects.get_or_init(|| {
            self.stack
                .data()
                .map(|d| {
                    d.stew_effects
                        .iter()
                        .map(|(effect, ticks)| (effect.to_string(), *ticks))
                        .collect()
                })
                .unwrap_or_default()
        })
    }

    fn components_slice(&self) -> &[(String, Option<Tag>)] {
        self.components
            .get_or_init(|| capture_components(&self.stack))
    }
}

fn component_key(id: &str) -> &str {
    id.strip_prefix('!').unwrap_or(id)
}

fn removal_marker() -> Tag {
    Tag::Compound(Default::default())
}

fn capture_components(stack: &ItemStack) -> Vec<(String, Option<Tag>)> {
    let skip_dedicated_positive = |id: &str| {
        matches!(
            id,
            "minecraft:enchantments"
                | "minecraft:stored_enchantments"
                | "minecraft:ominous_bottle_amplifier"
                | "minecraft:suspicious_stew_effects"
        ) || (id == "minecraft:potion_contents"
            && stack.data().is_none_or(|data| data.potion_extra.is_empty()))
    };
    let mut components = Vec::new();
    if let Some(Tag::Compound(entries)) = crate::item_data::encode_extra_components(stack) {
        for (key, value) in entries {
            let id = key.to_string_lossy();
            if !id.starts_with('!') && skip_dedicated_positive(&id) {
                continue;
            }
            components.push((id, Some(value)));
        }
    }
    components
}

impl LootToolAccess for LootCandidate {
    fn item_id(&self) -> &str {
        &self.stack.item
    }

    fn enchantment_at(&self, index: usize) -> Option<(&str, i32)> {
        self.enchantments_slice()
            .get(index)
            .map(|(id, level)| (id.as_str(), *level))
    }
}

impl LootStackAccess for LootCandidate {
    fn count(&self) -> i32 {
        self.count
    }

    fn damage(&self) -> i32 {
        self.damage
    }

    fn map(&self) -> bool {
        self.map
    }

    fn components(&self) -> &[(String, Option<Tag>)] {
        self.components_slice()
    }

    fn enchantments(&self) -> &[(String, i32)] {
        self.enchantments_slice()
    }

    fn potion(&self) -> Option<&str> {
        self.stack.potion()
    }

    fn stew_effects(&self) -> &[(String, i32)] {
        self.stew_effects_slice()
    }

    fn ominous_amplifier(&self) -> Option<i32> {
        self.ominous_amplifier
    }

    fn exploration(&self) -> Option<LootExplorationView<'_>> {
        self.exploration.as_deref().map(|map| {
            LootExplorationView::new(
                &map.destination,
                &map.decoration,
                map.zoom,
                map.search_radius,
                map.skip_existing_chunks,
            )
        })
    }

    fn names(&self) -> &[(String, Value)] {
        &self.names
    }
}

/// Whether the modeled component `id` takes `value`: only components the stack models into
/// typed fields have a shape to check; every other patch is kept as written.
fn component_payload_ok(id: &str, value: &Tag) -> bool {
    id != "minecraft:trim" || crate::item_data::trim::payload_ok(value)
}

impl LootStackEditAccess for LootCandidate {
    fn accepts_component(&self, id: &str, value: &Tag) -> bool {
        component_payload_ok(id, value)
    }

    fn as_view(&self) -> &dyn LootStackAccess {
        self
    }

    fn set_item_id(&mut self, id: &str) {
        self.stack.item = Arc::from(id);
    }

    fn set_count(&mut self, count: i32) {
        self.set_signed_count(count);
    }

    fn set_damage(&mut self, damage: i32) {
        const DAMAGE: &str = "minecraft:damage";
        if damage != 0
            || self.damage != 0
            || self.stack.carries_component(None, DAMAGE)
            || self.removals.iter().any(|key| key == DAMAGE)
            || self
                .stack
                .data()
                .is_some_and(|data| data.other("!minecraft:damage").is_some())
        {
            self.write_typed_component(DAMAGE, Some(Tag::Int(damage.max(0))));
        }
        self.damage = damage;
    }

    fn set_map(&mut self, map: bool) {
        self.map = map;
    }

    fn set_ominous_amplifier(&mut self, amplifier: Option<i32>) {
        if amplifier.is_some() || self.ominous_amplifier.is_some() {
            self.write_typed_component(
                "minecraft:ominous_bottle_amplifier",
                amplifier.map(Tag::Int),
            );
        }
        self.ominous_amplifier = amplifier;
    }

    fn set_potion_id(&mut self, potion: Option<String>) {
        const POTION: &str = "minecraft:potion_contents";
        let extras = self
            .stack
            .data()
            .map_or(&[][..], |d| d.potion_extra.as_slice());
        let base = potion
            .as_deref()
            .map(|id| ("potion", Tag::String(id.into())));
        if base.is_none() && extras.is_empty() && self.stack.potion().is_none() {
            return;
        }
        let value = (base.is_some() || !extras.is_empty()).then(|| {
            Tag::compound(
                base.into_iter().chain(
                    extras
                        .iter()
                        .map(|(key, value)| (key.as_str(), value.clone())),
                ),
            )
        });
        // Potion extras also belong to the component view; retain them after
        // replacing the typed potion id so an explicit copy can keep them.
        let has_extras = !extras.is_empty();
        self.write_typed_component(POTION, value.clone());
        if has_extras {
            let _ = self.components_slice();
            let components = self.components.get_mut().expect("components initialized");
            components.retain(|(key, _)| component_key(key) != POTION);
            components.push((POTION.into(), value));
        }
    }

    fn set_exploration(&mut self, exploration: Option<ApiExplorationMap>) {
        self.exploration = exploration.map(|map| {
            Box::new(ExplorationMap {
                destination: map.destination,
                decoration: map.decoration,
                zoom: map.zoom,
                search_radius: map.search_radius,
                skip_existing_chunks: map.skip_existing_chunks,
            })
        });
    }

    fn push_enchantment(&mut self, id: String, level: i32) {
        let mut values = self.enchantments_slice().to_vec();
        values.push((id, level));
        self.set_enchantments(values);
    }

    fn set_enchantment(&mut self, id: String, level: i32) {
        let mut values = self.enchantments_slice().to_vec();
        match values.iter_mut().find(|(name, _)| *name == id) {
            Some(entry) => entry.1 = level,
            None => values.push((id, level)),
        }
        self.set_enchantments(values);
    }

    fn set_enchantments(&mut self, enchantments: Vec<(String, i32)>) {
        if !enchantments.is_empty() || !self.enchantments_slice().is_empty() {
            let levels = Tag::compound(
                enchantments
                    .iter()
                    .map(|(id, level)| (id.as_str(), Tag::Int(*level))),
            );
            self.write_typed_component(self.enchantments_id(), Some(levels));
        }
        // The intermediate API retains order and signed values even when
        // the item component codec cannot materialize an enchantment level.
        self.enchantments = OnceLock::from(enchantments);
    }

    fn clear_enchantments(&mut self) {
        self.set_enchantments(Vec::new());
    }

    fn push_stew_effect(&mut self, id: String, duration_ticks: i32) {
        let mut effects = self.stew_effects_slice().to_vec();
        effects.push((id, duration_ticks));
        self.set_stew_effects(effects);
    }

    fn set_stew_effects(&mut self, effects: Vec<(String, i32)>) {
        if !effects.is_empty() || !self.stew_effects_slice().is_empty() {
            let value = Tag::List(
                effects
                    .iter()
                    .map(|(id, duration)| {
                        Tag::compound([
                            ("id", Tag::String(id.as_str().into())),
                            ("duration", Tag::Int(*duration)),
                        ])
                    })
                    .collect(),
            );
            self.write_typed_component("minecraft:suspicious_stew_effects", Some(value));
        }
        self.stew_effects = OnceLock::from(effects);
    }

    fn clear_stew_effects(&mut self) {
        self.set_stew_effects(Vec::new());
    }

    fn push_name_value(&mut self, id: String, value: Value) {
        let tag = Some(sunrise_game_data::scoreboard::json_to_tag(&value));
        // Names preserve ordered entries; the codec stores the last name for a key.
        let previous = std::mem::take(&mut self.names);
        self.write_typed_component(&id, tag);
        self.names = previous;
        self.names.push((id, value));
    }

    fn set_name_value(&mut self, id: String, value: Value) {
        self.names.retain(|(existing, _)| *existing != id);
        self.push_name_value(id, value);
    }

    fn set_names_values(&mut self, names: Vec<(String, Value)>) {
        for (id, _) in std::mem::take(&mut self.names) {
            if !names.iter().any(|(next, _)| *next == id) {
                self.write_typed_component(&id, None);
            }
        }
        for (id, value) in &names {
            self.write_typed_component(id, Some(sunrise_game_data::scoreboard::json_to_tag(value)));
        }
        self.names = names;
    }

    fn clear_names(&mut self) {
        self.set_names_values(Vec::new());
    }

    fn push_component(&mut self, id: String, value: Option<Tag>) {
        let _ = self.components_slice();
        self.write_component(&id, &value);
        self.components
            .get_mut()
            .expect("components initialized")
            .push((id, value));
    }

    fn set_components(&mut self, components: Vec<(String, Option<Tag>)>) {
        let before = self
            .components
            .take()
            .unwrap_or_else(|| capture_components(&self.stack));
        for (id, _) in &before {
            if !components.iter().any(|(key, _)| key == id) {
                self.write_component(id, &None);
            }
        }
        for (id, value) in &components {
            let base = component_key(id);
            let same = |entry: &&(String, Option<Tag>)| component_key(&entry.0) == base;
            if !before
                .iter()
                .filter(same)
                .eq(components.iter().filter(same))
            {
                self.write_component(id, value);
            }
        }
        self.components = OnceLock::from(components);
    }

    fn clear_components(&mut self) {
        self.set_components(Vec::new());
    }

    fn retain_components(&mut self, keep: &mut dyn FnMut(&str, Option<&Tag>) -> bool) {
        let retained = self
            .components_slice()
            .iter()
            .filter(|(id, value)| keep(id, value.as_ref()))
            .cloned()
            .collect();
        self.set_components(retained);
    }
}

/// Items-owned backing for the evaluator's live output list.
pub(crate) struct LootCandidates(pub(crate) Vec<LootCandidate>);

impl LootListAccess for LootCandidates {
    fn accepts_component(&self, id: &str, value: &Tag) -> bool {
        component_payload_ok(id, value)
    }

    fn len(&self) -> usize {
        self.0.len()
    }

    fn get(&self, index: usize) -> Option<&dyn LootStackAccess> {
        self.0
            .as_slice()
            .get(index)
            .map(|stack| stack as &dyn LootStackAccess)
    }

    fn get_mut(&mut self, index: usize) -> Option<&mut dyn LootStackEditAccess> {
        self.0
            .as_mut_slice()
            .get_mut(index)
            .map(|stack| stack as &mut dyn LootStackEditAccess)
    }

    fn push_empty(&mut self, item_id: &str) -> &mut dyn LootStackEditAccess {
        self.0.push(LootCandidate::from_item(item_id, 1));
        self.0.last_mut().expect("just pushed")
    }

    fn insert_empty(
        &mut self,
        index: usize,
        item_id: &str,
    ) -> Option<&mut dyn LootStackEditAccess> {
        if index > self.len() {
            return None;
        }
        self.0.insert(index, LootCandidate::from_item(item_id, 1));
        Some(&mut self.0[index])
    }

    fn remove(&mut self, index: usize) -> bool {
        if index >= self.len() {
            return false;
        }
        self.0.remove(index);
        true
    }

    fn clear(&mut self) {
        self.0.clear();
    }

    fn truncate(&mut self, len: usize) {
        self.0.truncate(len);
    }

    fn swap(&mut self, a: usize, b: usize) -> bool {
        if a >= self.len() || b >= self.len() {
            return false;
        }
        self.0.as_mut_slice().swap(a, b);
        true
    }

    fn reverse(&mut self) {
        self.0.as_mut_slice().reverse();
    }

    fn reorder(&mut self, from: usize, to: usize) -> bool {
        if from >= self.len() || to >= self.len() {
            return false;
        }
        match from.cmp(&to) {
            std::cmp::Ordering::Less => self.0[from..=to].rotate_left(1),
            std::cmp::Ordering::Greater => self.0[to..=from].rotate_right(1),
            std::cmp::Ordering::Equal => {}
        }
        true
    }
}
