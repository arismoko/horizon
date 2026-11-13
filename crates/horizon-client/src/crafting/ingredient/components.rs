//! Component patches are validated and decoded once, before publication.
use super::*;
use horizon_core::nbt::{Compound, Tag};
use horizon_items::item_data::ingredient_components::{component_equal, removed, visit_components};
use std::collections::HashMap;

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
    horizon_core::identifier::Identifier::parse(id)
        .map(|id| id.to_string())
        .map_err(|error| error.to_string())
}
pub(super) fn validate(rule: &IngredientRule, items: &Items) -> Result<(), String> {
    let IngredientRule::Components { patch, removed, .. } = rule;
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
            || horizon_command::item::DECODED_COMPONENTS.contains(&id)
            || horizon_protocol::item::custom::id(id).is_some()
    };
    let mut visited = HashSet::new();
    for id in patch
        .keys()
        .map(|key| key.to_string_lossy())
        .chain(removed.iter().cloned())
    {
        let id = component_id(&id)?;
        if horizon_world::item_defaults::is_transient_component(&id) {
            return Err(format!("'{id}' is not a persistent component"));
        }
        if !visited.insert(id.clone()) {
            return Err(format!("repeated component {id}"));
        }
        if !known(&id) {
            return Err(format!("unregistered component {id}"));
        }
    }
    for (id, value) in patch {
        let id = component_id(&id.to_string_lossy())?;
        if id.starts_with("minecraft:") {
            horizon_command::item::decoded_component_value(&id, value, &items.component_registry)
                .map_err(|error| format!("component {id}: {error}"))?;
        } else {
            // component_types declares persistent, arbitrary NBT values.
            horizon_protocol::item::custom::validate(value)
                .map_err(|error| format!("component {id}: {error}"))?;
        }
    }
    Ok(())
}
impl ComponentPredicate {
    pub(super) fn compile(
        rule: &IngredientRule,
        candidates: &HashSet<String>,
        items: &Items,
    ) -> Result<Self, String> {
        validate(rule, items)?;
        let IngredientRule::Components {
            patch,
            removed,
            strict,
        } = rule;
        let decoded = patch
            .iter()
            .map(|(id, value)| {
                let name = component_id(&id.to_string_lossy())?;
                let value = if name.starts_with("minecraft:") {
                    horizon_command::item::decoded_component_value(
                        &name,
                        value,
                        &items.component_registry,
                    )?
                } else {
                    value.clone()
                };
                Ok((name.as_str().into(), value))
            })
            .collect::<Result<Compound, String>>()?;
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
            patched: patch
                .keys()
                .map(|key| component_id(&key.to_string_lossy()))
                .collect::<Result<_, _>>()?,
            removed: removed
                .iter()
                .map(|id| component_id(id))
                .collect::<Result<_, _>>()?,
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

pub(super) fn validate_recipe(value: &Value, items: &Items, recipe: &str) -> bool {
    match value {
        Value::Object(object) => match ingredient_kind(object) {
            Some("neoforge:components") => {
                let keys = object
                    .get("components")
                    .and_then(Value::as_object)
                    .map(|patch| {
                        patch
                            .keys()
                            .map(|id| id.strip_prefix('!').unwrap_or(id).to_owned())
                            .collect()
                    })
                    .unwrap_or_default();
                let ids = IngredientRule::Components {
                    patch: Default::default(),
                    removed: keys,
                    strict: false,
                };
                if let Err(error) = validate(&ids, items) {
                    log::warn!("recipe {recipe} rejected: {error}");
                    return false;
                }
                let parsed = WalkedIngredient::walk(
                    value,
                    &items.item_tag_order,
                    &|_| None,
                    &resolved_items,
                    &|data| crate::json_nbt::json_to_compound(data).ok(),
                );
                let result = parsed
                    .and_then(|parsed| parsed.rule)
                    .ok_or_else(|| "malformed components ingredient".to_owned())
                    .and_then(|rule| validate(&rule, items));
                if let Err(error) = result {
                    log::warn!("recipe {recipe} rejected: {error}");
                    return false;
                }
                true
            }
            Some("neoforge:compound" | "neoforge:intersection") => ["children", "ingredients"]
                .iter()
                .filter_map(|key| object.get(*key))
                .all(|value| validate_recipe(value, items, recipe)),
            Some("neoforge:difference") => ["base", "subtracted"]
                .iter()
                .filter_map(|key| object.get(*key))
                .all(|value| validate_recipe(value, items, recipe)),
            Some("neoforge:custom_display") => object
                .get("base")
                .is_none_or(|value| validate_recipe(value, items, recipe)),
            _ => {
                crate::crafting::INGREDIENT_KEYS
                    .iter()
                    .filter_map(|key| object.get(*key))
                    .all(|value| validate_recipe(value, items, recipe))
                    && crate::crafting::INGREDIENT_MAPS
                        .iter()
                        .filter_map(|key| object.get(*key).and_then(Value::as_object))
                        .flat_map(|map| map.values())
                        .all(|value| validate_recipe(value, items, recipe))
            }
        },
        Value::Array(values) => values
            .iter()
            .all(|value| validate_recipe(value, items, recipe)),
        _ => true,
    }
}

/// Validate only recipe/ingredient codecs; a native ingredient's own JSON is opaque.
pub(super) fn persistent_recipe(value: &Value) -> Result<(), String> {
    if !value["type"]
        .as_str()
        .is_some_and(|kind| kind.starts_with("minecraft:"))
    {
        return Ok(());
    }
    horizon_world::item_defaults::validate_stack(&value["result"])?;
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
                horizon_world::item_defaults::validate_component_map(&value["components"])?
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
        "minecraft:item_stack" => horizon_world::item_defaults::validate_stack(&value["item"])?,
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
