//! Component patches are validated and decoded once, before publication.
use super::*;
use std::collections::HashMap;
use sunrise_core::nbt::{Compound, Tag};
use sunrise_items::item_data::ingredient_components::{component_equal, removed, visit_components};

/// Decoded patch and item prototypes shared by concrete candidate checks.
#[derive(Clone, Debug)]
pub struct ComponentPredicate {
    expected: ItemStack,
    retained: Compound,
    patched: HashSet<String>,
    removed: Vec<String>,
    strict: bool,
    defaults: HashMap<String, (ItemStack, Vec<String>)>,
}
fn decode(patch: Compound) -> ItemStack {
    let (damage, extra) = crate::item_data::decode_components(Some(&Tag::Compound(patch)));
    ItemStack {
        item: "minecraft:air".into(),
        count: 1,
        damage,
        extra,
    }
}
fn component_id(id: &str) -> Result<String, String> {
    sunrise_core::identifier::Identifier::parse(id)
        .map(|id| id.to_string())
        .map_err(|error| error.to_string())
}
fn validate_key(id: &str, visited: &mut HashSet<String>, items: &Items) -> Result<String, String> {
    let known = |id: &str| {
        items
            .component_registry
            .ids("minecraft:data_component_type")
            .iter()
            .any(|name| name == id)
            || items
                .component_registry
                .persistent_components
                .iter()
                .any(|name| name == id)
            || sunrise_command::item::DECODED_COMPONENTS.contains(&id)
            || sunrise_protocol::item::custom::id(id).is_some()
    };
    let id = component_id(id)?;
    if sunrise_world::item_defaults::is_transient_component(&id) {
        return Err(format!("'{id}' is not a persistent component"));
    }
    if !visited.insert(id.clone()) {
        return Err(format!("repeated component {id}"));
    }
    if !known(&id) {
        return Err(format!("unregistered component {id}"));
    }
    Ok(id)
}
fn decode_rule_entries(
    rule: &IngredientRule,
    items: &Items,
) -> Result<(Compound, HashSet<String>, Vec<String>), String> {
    let IngredientRule::Components { patch, removed, .. } = rule;
    let mut visited = HashSet::new();
    let mut patched = HashSet::with_capacity(patch.len());
    let mut normalized_removed = Vec::with_capacity(removed.len());
    let mut decoded = Compound::new();
    for (raw_id, value) in patch {
        let name = validate_key(&raw_id.to_string_lossy(), &mut visited, items)?;
        let value = if name.starts_with("minecraft:") {
            sunrise_command::item::decoded_component_value(&name, value, &items.component_registry)
                .map_err(|error| format!("component {name}: {error}"))?
        } else {
            // component_types declares persistent, arbitrary NBT values.
            sunrise_core::nbt::validate_component_value(value)
                .map_err(|error| format!("component {name}: {error}"))?;
            value.clone()
        };
        patched.insert(name.clone());
        decoded.insert(name.as_str().into(), value);
    }
    for raw_id in removed {
        normalized_removed.push(validate_key(raw_id, &mut visited, items)?);
    }
    Ok((decoded, patched, normalized_removed))
}
#[cfg(test)]
pub(super) fn validate(rule: &IngredientRule, items: &Items) -> Result<(), String> {
    decode_rule_entries(rule, items).map(|_| ())
}
impl ComponentPredicate {
    pub(super) fn compile(
        rule: &IngredientRule,
        candidates: &HashSet<String>,
        items: &Items,
    ) -> Result<Self, String> {
        let (decoded, patched, removed) = decode_rule_entries(rule, items)?;
        let IngredientRule::Components { strict, .. } = rule;
        let expected = decode(decoded.clone());
        let defaults = candidates
            .iter()
            .map(|id| {
                let values = items
                    .component_defaults
                    .get(id)
                    .cloned()
                    .unwrap_or_default();
                let names = values.keys().map(|id| id.to_string_lossy()).collect();
                (id.clone(), (decode(values), names))
            })
            .collect();
        Ok(Self {
            expected,
            retained: decoded,
            patched,
            removed,
            strict: *strict,
            defaults,
        })
    }
    pub(super) fn matches(&self, stack: &ItemStack) -> bool {
        if stack.count == 0 || stack.item.as_ref() == "minecraft:air" {
            return false;
        }
        let default = self.defaults.get(stack.item.as_ref());
        let effective = |id: &str| {
            if stack.carries_component(None, id) {
                Some(stack)
            } else if removed(stack, id) {
                None
            } else {
                default
                    .filter(|(_, names)| names.iter().any(|name| name == id))
                    .map(|(stack, _)| stack)
            }
        };
        if !self.patched.iter().all(|id| {
            effective(id).is_some_and(|actual| component_equal(id, &self.expected, actual))
        }) || self.removed.iter().any(|id| effective(id).is_some())
        {
            return false;
        }
        if self.strict {
            let mut valid = true;
            visit_components(stack, |id| valid &= self.patched.contains(id));
            if !valid
                || default.is_some_and(|(_, names)| {
                    names
                        .iter()
                        .any(|id| effective(id).is_some() && !self.patched.contains(id))
                })
            {
                return false;
            }
        }
        true
    }
    pub(super) fn shown(&self, order: &[String]) -> Vec<ItemStack> {
        order
            .iter()
            .map(|id| {
                // ItemStack carries a patch: its renderer reads the prototype itself.
                let mut values = self.retained.clone();
                for id in &self.removed {
                    values.insert(
                        format!("!{id}").as_str().into(),
                        Tag::Compound(Default::default()),
                    );
                }
                let mut stack = decode(values);
                stack.item = id.as_str().into();
                stack
            })
            .collect()
    }
}

/// JSON optional record fields treat null as absent before the strict NBT boundary.
pub(super) fn json_patch(data: &serde_json::Map<String, Value>) -> Option<Compound> {
    const OPTIONAL_FIELDS: &[(&str, &[&str])] = &[
        (
            "weapon",
            &["item_damage_per_attack", "disable_blocking_for_seconds"],
        ),
        ("food", &["can_always_eat"]),
        (
            "custom_model_data",
            &["floats", "flags", "strings", "colors"],
        ),
        ("use_cooldown", &["cooldown_group"]),
        ("lodestone_tracker", &["target", "tracked"]),
        ("container_loot", &["seed"]),
        ("tooltip_display", &["hide_tooltip", "hidden_components"]),
        ("attack_animation", &["type", "duration"]),
        (
            "use_effects",
            &["can_sprint", "interact_vibrations", "speed_multiplier"],
        ),
        (
            "attack_range",
            &[
                "min_reach",
                "max_reach",
                "min_creative_reach",
                "max_creative_reach",
                "hitbox_margin",
                "mob_factor",
            ],
        ),
    ];
    let mut normalized = None;
    for (id, value) in data {
        let component = id.strip_prefix("minecraft:").unwrap_or(id);
        let optional = OPTIONAL_FIELDS
            .iter()
            .find(|(name, _)| *name == component)
            .map_or(&[][..], |(_, fields)| *fields);
        if let Some(record) = value.as_object() {
            // Integral JSON-to-NBT conversion would discard weapon's forbidden sign.
            if component == "weapon"
                && record
                    .get("disable_blocking_for_seconds")
                    .and_then(Value::as_f64)
                    .is_some_and(|seconds| seconds == 0.0 && seconds.is_sign_negative())
            {
                return None;
            }
            let mut normalized_record = None;
            for field in optional {
                if record.get(*field).is_some_and(Value::is_null) {
                    normalized_record
                        .get_or_insert_with(|| record.clone())
                        .remove(*field);
                }
            }
            if let Some(record) = normalized_record {
                normalized
                    .get_or_insert_with(|| data.clone())
                    .insert(id.clone(), Value::Object(record));
            }
        }
    }
    crate::json_nbt::json_to_compound(normalized.as_ref().unwrap_or(data)).ok()
}

/// Validate component keys and convert the JSON patch once during ingredient decoding.
pub(super) fn convert_component_patch(
    data: &serde_json::Map<String, Value>,
    items: &Items,
) -> Result<Compound, String> {
    let mut visited = HashSet::with_capacity(data.len());
    for raw_id in data.keys() {
        let id = raw_id.strip_prefix('!').unwrap_or(raw_id);
        validate_key(id, &mut visited, items)?;
    }
    let mut converted = Compound::new();
    for (id, value) in data {
        let single = serde_json::Map::from_iter([(id.clone(), value.clone())]);
        match json_patch(&single) {
            Some(field) => converted.extend(field),
            None => return Err(format!("component {id}: malformed JSON payload")),
        }
    }
    Ok(converted)
}

#[cfg(test)]
pub(super) fn validate_recipe_components(value: &Value, items: &Items) -> Result<(), String> {
    Ingredient::parse_with_error(value, items, &|_, _| None).map(|_| ())
}

/// Validate only recipe/ingredient codecs; a native ingredient's own JSON is opaque.
pub(super) fn persistent_recipe(value: &Value) -> Result<(), String> {
    if !value["type"]
        .as_str()
        .is_some_and(|kind| kind.starts_with("minecraft:"))
    {
        return Ok(());
    }
    sunrise_world::item_defaults::validate_stack(&value["result"])?;
    for key in crate::crafting::INGREDIENT_KEYS {
        persistent_ingredient(&value[key])?;
    }
    for key in crate::crafting::INGREDIENT_MAPS {
        for ingredient in value[key]
            .as_object()
            .into_iter()
            .flat_map(|map| map.values())
        {
            persistent_ingredient(ingredient)?;
        }
    }
    Ok(())
}
fn persistent_ingredient(value: &Value) -> Result<(), String> {
    if let Some(values) = value.as_array() {
        for value in values {
            persistent_ingredient(value)?;
        }
    } else if let Some(object) = value.as_object() {
        match ingredient_kind(object) {
            Some("neoforge:components") => {
                sunrise_world::item_defaults::validate_component_map(&value["components"])?
            }
            Some("neoforge:compound" | "neoforge:intersection") => {
                for key in ["children", "ingredients"] {
                    persistent_ingredient(&value[key])?;
                }
            }
            Some("neoforge:difference") => {
                persistent_ingredient(&value["base"])?;
                persistent_ingredient(&value["subtracted"])?;
            }
            Some("neoforge:custom_display") => {
                persistent_ingredient(&value["base"])?;
                persistent_display(&value["display"])?;
            }
            _ => {}
        }
    }
    Ok(())
}
fn persistent_display(value: &Value) -> Result<(), String> {
    match value["type"].as_str().unwrap_or("") {
        "minecraft:item_stack" => sunrise_world::item_defaults::validate_stack(&value["item"])?,
        "minecraft:composite" => {
            for display in value["contents"].as_array().into_iter().flatten() {
                persistent_display(display)?;
            }
        }
        "minecraft:with_any_potion" | "minecraft:only_with_component" => {
            persistent_display(&value["contents"])?
        }
        "minecraft:dyed" => persistent_display(&value["target"])?,
        "minecraft:smithing_trim" => {
            for key in ["base", "material", "pattern"] {
                persistent_display(&value[key])?;
            }
        }
        "minecraft:with_remainder" => {
            persistent_display(&value["input"])?;
            persistent_display(&value["remainder"])?;
        }
        _ => {}
    }
    Ok(())
}
