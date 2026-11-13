//! Borrowed NBT codec boundaries for command admission, without JSON conversion.
use crate::item_defaults::{
    ascii_name, validate_component_map_tag, validate_item_predicate_tag,
    validate_partial_predicate_tag,
};
use horizon_core::nbt::Tag;

fn field<'a>(value: &'a Tag, name: &str) -> &'a Tag {
    value.get(name).unwrap_or(&Tag::End)
}
fn keyed<'a>(value: &'a Tag, name: &str) -> &'a Tag {
    value
        .get(name)
        .or_else(|| value.get(name.strip_prefix("minecraft:").unwrap_or(name)))
        .unwrap_or(&Tag::End)
}
fn list(value: &Tag) -> &[Tag] {
    if let Tag::List(values) = value {
        values
    } else {
        &[]
    }
}
fn kind<'a>(value: &Tag, scratch: &'a mut [u8; 128]) -> &'a str {
    let name = value
        .get("type")
        .or_else(|| value.get("condition"))
        .or_else(|| value.get("function"));
    let Some(Tag::String(name)) = name else {
        return "";
    };
    let Some(name) = ascii_name(name, scratch) else {
        return "";
    };
    name.strip_prefix("minecraft:").unwrap_or(name)
}
fn entity(value: &Tag) -> Result<(), String> {
    if !matches!(value, Tag::Compound(_)) {
        return Ok(());
    }
    validate_component_map_tag(keyed(value, "minecraft:components"))?;
    // Only borrowed partial collections need traversing; any-value types are opaque.
    if let Tag::Compound(predicates) = keyed(value, "minecraft:predicates") {
        let mut scratch = [0u8; 128];
        for (name, predicate) in predicates {
            let Some(name) = ascii_name(name, &mut scratch) else {
                continue;
            };
            validate_partial_predicate_tag(name, predicate)?;
        }
    }
    for name in ["minecraft:equipment", "minecraft:slots"] {
        if let Tag::Compound(items) = keyed(value, name) {
            for item in items.values() {
                validate_item_predicate_tag(item)?;
            }
        }
    }
    for name in [
        "minecraft:vehicle",
        "minecraft:passenger",
        "minecraft:targeted_entity",
    ] {
        entity(keyed(value, name))?;
    }
    entity(field(
        keyed(value, "minecraft:type_specific/player"),
        "looking_at",
    ))?;
    entity(field(
        keyed(value, "minecraft:type_specific/lightning"),
        "entity_struck",
    ))?;
    for name in [
        "minecraft:location",
        "minecraft:stepping_on",
        "minecraft:movement_affected_by",
    ] {
        validate_item_predicate_tag(field(keyed(value, name), "block"))?;
    }
    Ok(())
}

/// The persistent payloads of stock loot-condition codecs; names/native data stay opaque.
pub fn validate_loot_condition_tag(value: &Tag) -> Result<(), String> {
    if let Tag::List(values) = value {
        for value in values {
            validate_loot_condition_tag(value)?;
        }
        return Ok(());
    }
    let mut scratch = [0u8; 128];
    match kind(value, &mut scratch) {
        "match_tool" => validate_item_predicate_tag(field(value, "predicate")),
        "entity_properties" => entity(field(value, "predicate")),
        "all_of" | "any_of" => validate_loot_condition_tag(field(value, "terms")),
        "inverted" => validate_loot_condition_tag(field(value, "term")),
        "damage_source_properties" => {
            let predicate = field(value, "predicate");
            entity(field(predicate, "direct_entity"))?;
            entity(field(predicate, "source_entity"))
        }
        "location_check" => validate_item_predicate_tag(field(field(value, "predicate"), "block")),
        "random_chance" => super::number_validation::number_tag(field(value, "chance")),
        "int_value_check" | "float_value_check" => {
            super::number_validation::number_tag(field(value, "value"))?;
            super::number_validation::range_tag(field(value, "test"))
        }
        "time_check" => super::number_validation::range_tag(field(value, "value")),
        "entity_scores" => {
            if let Tag::Compound(scores) = field(value, "scores") {
                for score in scores.values() {
                    super::number_validation::range_tag(score)?;
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
/// Borrowed counterpart of `validate_item_modifier`, at inline command admission.
pub fn validate_item_modifier_tag(value: &Tag) -> Result<(), String> {
    if let Tag::List(values) = value {
        for value in values {
            validate_item_modifier_tag(value)?;
        }
        return Ok(());
    }
    validate_loot_condition_tag(field(value, "condition"))?;
    super::number_validation::modifier_tag(value)?;
    let mut scratch = [0u8; 128];
    match kind(value, &mut scratch) {
        "set_components" => validate_component_map_tag(field(value, "components")),
        "set_name" => crate::item_defaults::validate_text_tag(field(value, "name")),
        "set_lore" => crate::item_defaults::validate_text_tag(field(value, "lore")),
        "set_written_book_pages" => {
            crate::item_defaults::validate_component_text_tag("written_book_content", value)
        }
        "set_attributes" => crate::item_defaults::validate_component_text_tag(
            "attribute_modifiers",
            field(value, "modifiers"),
        ),
        "sequence" => validate_item_modifier_tag(field(value, "functions")),
        "modify_contents" => validate_item_modifier_tag(field(value, "modifier")),
        "filtered" => {
            validate_item_predicate_tag(field(value, "item_filter"))?;
            validate_item_modifier_tag(field(value, "on_pass"))?;
            validate_item_modifier_tag(field(value, "on_fail"))
        }
        "set_contents" => {
            for value in list(field(value, "entries")) {
                entry(value)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
fn fields(value: &Tag) -> Result<(), String> {
    validate_item_modifier_tag(field(value, "modifier"))?;
    validate_loot_condition_tag(field(value, "condition"))
}
fn entry(value: &Tag) -> Result<(), String> {
    fields(value)?;
    let mut scratch = [0u8; 128];
    match kind(value, &mut scratch) {
        "alternatives" | "group" | "sequence" => {
            for value in list(field(value, "children")) {
                entry(value)?;
            }
        }
        "loot_table" if matches!(field(value, "value"), Tag::Compound(_)) => {
            validate_loot_table_tag(field(value, "value"))?
        }
        _ => {}
    }
    Ok(())
}
/// Borrowed table codec: table/pool/entry modifiers and condition predicates.
pub fn validate_loot_table_tag(value: &Tag) -> Result<(), String> {
    validate_item_modifier_tag(field(value, "modifier"))?;
    for pool in list(field(value, "pools")) {
        super::number_validation::number_tag(field(pool, "rolls"))?;
        super::number_validation::number_tag(field(pool, "bonus_rolls"))?;
        fields(pool)?;
        for value in list(field(pool, "entries")) {
            entry(value)?;
        }
    }
    Ok(())
}
