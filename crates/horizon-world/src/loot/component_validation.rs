//! Codec-boundary validation before publishing persistent loot and predicates.
use super::*;
use crate::item_defaults::validate_component_map;

fn keyed<'a>(value: &'a Value, name: &str) -> &'a Value {
    value
        .as_object()
        .and_then(|map| {
            map.iter().find_map(|(key, value)| {
                (key.strip_prefix("minecraft:").unwrap_or(key) == name).then_some(value)
            })
        })
        .unwrap_or(&Value::Null)
}
fn kind(value: &Value) -> &str {
    let name = value
        .get("type")
        .or_else(|| value.get("condition"))
        .or_else(|| value.get("function"))
        .and_then(Value::as_str)
        .unwrap_or("");
    name.strip_prefix("minecraft:").unwrap_or(name)
}

/// `DataComponentMatchers.CODEC`, used by item and block predicates. Partial
/// any-value predicates name a type, not a persistent value, and remain valid.
pub fn validate_item_predicate(value: &Value) -> Result<(), String> {
    validate_component_map(&value["components"])?;
    validate_partial_predicates(&value["predicates"])
}
fn validate_partial_predicates(value: &Value) -> Result<(), String> {
    for name in ["container", "bundle_contents", "charged_projectiles"] {
        let collection = &keyed(value, name)["items"];
        for item in collection["contains"].as_array().into_iter().flatten() {
            validate_item_predicate(item)?;
        }
        for count in collection["count"].as_array().into_iter().flatten() {
            validate_item_predicate(&count["test"])?;
        }
    }
    Ok(())
}

/// `EntityPredicate.CODEC`'s registered component-bearing sub-predicates.
/// Unknown/native sub-predicate payloads and NBT are intentionally opaque.
pub fn validate_entity_predicate(value: &Value) -> Result<(), String> {
    validate_component_map(keyed(value, "components"))?;
    validate_partial_predicates(keyed(value, "predicates"))?;
    for name in ["equipment", "slots"] {
        for item in keyed(value, name)
            .as_object()
            .into_iter()
            .flat_map(|map| map.values())
        {
            validate_item_predicate(item)?;
        }
    }
    for name in ["vehicle", "passenger", "targeted_entity"] {
        validate_entity_predicate_inner(keyed(value, name))?;
    }
    validate_entity_predicate_inner(&keyed(value, "type_specific/player")["looking_at"])?;
    validate_entity_predicate_inner(&keyed(value, "type_specific/lightning")["entity_struck"])?;
    for name in ["location", "stepping_on", "movement_affected_by"] {
        validate_item_predicate(&keyed(value, name)["block"])?;
    }
    Ok(())
}
fn validate_entity_predicate_inner(value: &Value) -> Result<(), String> {
    if !value.is_null() {
        validate_entity_predicate(value)?;
    }
    Ok(())
}

/// Loot condition dispatch, including nested condition and entity codecs.
pub fn validate_loot_condition(value: &Value) -> Result<(), String> {
    if let Some(values) = value.as_array() {
        for value in values {
            validate_loot_condition(value)?;
        }
        return Ok(());
    }
    match kind(value) {
        "match_tool" => validate_item_predicate(&value["predicate"]),
        "entity_properties" => validate_entity_predicate(&value["predicate"]),
        "all_of" | "any_of" => validate_loot_condition(&value["terms"]),
        "inverted" => validate_loot_condition(&value["term"]),
        "damage_source_properties" => {
            validate_entity_predicate(&value["predicate"]["direct_entity"])?;
            validate_entity_predicate(&value["predicate"]["source_entity"])
        }
        "location_check" => validate_item_predicate(&value["predicate"]["block"]),
        "random_chance" => super::number_validation::number(&value["chance"]),
        "int_value_check" | "float_value_check" => {
            super::number_validation::number(&value["value"])?;
            super::number_validation::range(&value["test"])
        }
        "time_check" => super::number_validation::range(&value["value"]),
        "entity_scores" => {
            for score in value["scores"]
                .as_object()
                .into_iter()
                .flat_map(|map| map.values())
            {
                super::number_validation::range(score)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Item-modifier dispatch; custom function data and `set_custom_data.tag` do
/// not contain component maps just because they have a key named components.
pub fn validate_item_modifier(value: &Value) -> Result<(), String> {
    if let Some(values) = value.as_array() {
        for value in values {
            validate_item_modifier(value)?;
        }
        return Ok(());
    }
    validate_loot_condition(&value["condition"])?;
    super::number_validation::modifier(value)?;
    match kind(value) {
        "set_components" => validate_component_map(&value["components"]),
        "set_name" => crate::item_defaults::validate_text(&value["name"]),
        "set_lore" => crate::item_defaults::validate_text(&value["lore"]),
        "set_written_book_pages" => {
            crate::item_defaults::validate_component_text("written_book_content", value)
        }
        "set_attributes" => crate::item_defaults::validate_component_text(
            "attribute_modifiers",
            &value["modifiers"],
        ),
        "sequence" => validate_item_modifier(&value["functions"]),
        "modify_contents" => validate_item_modifier(&value["modifier"]),
        "filtered" => {
            validate_item_predicate(&value["item_filter"])?;
            validate_item_modifier(&value["on_pass"])?;
            validate_item_modifier(&value["on_fail"])
        }
        "set_contents" => {
            for entry in value["entries"].as_array().into_iter().flatten() {
                validate_entry(entry)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
fn validate_modifiers_and_conditions(value: &Value) -> Result<(), String> {
    validate_item_modifier(&value["modifier"])?;
    validate_loot_condition(&value["condition"])
}
fn validate_entry(value: &Value) -> Result<(), String> {
    validate_modifiers_and_conditions(value)?;
    match kind(value) {
        "alternatives" | "group" | "sequence" => {
            for child in value["children"].as_array().into_iter().flatten() {
                validate_entry(child)?;
            }
        }
        "loot_table" if value["value"].is_object() => validate_loot_table(&value["value"])?,
        _ => {}
    }
    Ok(())
}
pub fn validate_loot_table(value: &Value) -> Result<(), String> {
    validate_item_modifier(&value["modifier"])?;
    for pool in value["pools"].as_array().into_iter().flatten() {
        super::number_validation::number(&pool["rolls"])?;
        super::number_validation::number(&pool["bonus_rolls"])?;
        validate_modifiers_and_conditions(pool)?;
        for entry in pool["entries"].as_array().into_iter().flatten() {
            validate_entry(entry)?;
        }
    }
    Ok(())
}
pub(super) fn retain_valid(kind: &str, id: &str, result: Result<(), String>) -> bool {
    match result {
        Ok(()) => true,
        Err(error) => {
            log::warn!("{kind} {id} rejected: {error}");
            false
        }
    }
}

/// `SlotSources.CODEC`: only the stock filtered source owns an item predicate.
pub fn validate_slot_source(value: &Value) -> Result<(), String> {
    if let Some(values) = value.as_array() {
        for value in values {
            validate_slot_source(value)?;
        }
        return Ok(());
    }
    match kind(value) {
        "group" => validate_slot_source(&value["terms"]),
        "contents" | "limit_slots" => validate_slot_source(&value["slot_source"]),
        "filtered" => {
            validate_item_predicate(&value["item_filter"])?;
            validate_slot_source(&value["slot_source"])
        }
        _ => Ok(()),
    }
}
