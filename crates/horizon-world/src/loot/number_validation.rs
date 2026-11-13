//! Number-provider codecs can contain loot conditions, even in unchosen branches.
use horizon_core::nbt::Tag;
use serde_json::Value;

fn kind(value: &Value) -> &str {
    let kind = value["type"].as_str().unwrap_or("");
    kind.strip_prefix("minecraft:").unwrap_or(kind)
}
pub fn number(value: &Value) -> Result<(), String> {
    if let Some(values) = value.as_array() {
        for value in values {
            number(value)?;
        }
        return Ok(());
    }
    let keys: &[&str] = match kind(value) {
        "uniform" => &["min", "max"],
        "constant" => &["value"],
        "binomial" => &["n", "p"],
        "add" | "mul" | "avg" | "max" | "min" | "length" => &["inputs"],
        "sub" | "div" | "mod" | "floor_div" | "floor_mod" => &["left", "right"],
        "pow" => &["base", "exponent"],
        "abs" | "negate" | "from_float" | "from_int" | "floor" | "ceil" | "round" | "sqrt"
        | "sin" | "cos" | "truncate" => &["input"],
        "score" | "storage" => &["fallback"],
        "conditional" => {
            super::validate_loot_condition(&value["condition"])?;
            &["on_true", "on_false"]
        }
        "number_dispatcher" => {
            for case in value["cases"].as_array().into_iter().flatten() {
                super::validate_loot_condition(&case["condition"])?;
                number(&case["value"])?;
            }
            &["default"]
        }
        "weighted_list" => {
            for entry in value["distribution"].as_array().into_iter().flatten() {
                number(&entry["data"])?;
            }
            &[]
        }
        _ => &[],
    };
    for key in keys {
        number(&value[*key])?;
    }
    Ok(())
}
pub(super) fn range(value: &Value) -> Result<(), String> {
    number(value)?;
    number(&value["min"])?;
    number(&value["max"])
}
pub(super) fn modifier(value: &Value) -> Result<(), String> {
    match kind(value) {
        "set_count" | "enchanted_count_increase" => number(&value["count"]),
        "limit_count" => range(&value["limit"]),
        "set_damage" => number(&value["damage"]),
        "enchant_with_levels" => number(&value["levels"]),
        "set_ominous_bottle_amplifier" => number(&value["amplifier"]),
        "set_random_dyes" => number(&value["number_of_dyes"]),
        "set_enchantments" => {
            for amount in value["enchantments"]
                .as_object()
                .into_iter()
                .flat_map(|map| map.values())
            {
                number(amount)?;
            }
            Ok(())
        }
        "set_stew_effect" => {
            for effect in value["effects"].as_array().into_iter().flatten() {
                number(&effect["duration"])?;
            }
            Ok(())
        }
        "set_attributes" => {
            for modifier in value["modifiers"].as_array().into_iter().flatten() {
                number(&modifier["amount"])?;
            }
            Ok(())
        }
        "set_custom_model_data" => {
            for amount in value["floats"]["values"].as_array().into_iter().flatten() {
                number(amount)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn field<'a>(value: &'a Tag, key: &str) -> &'a Tag {
    value.get(key).unwrap_or(&Tag::End)
}
fn list(value: &Tag) -> &[Tag] {
    if let Tag::List(values) = value {
        values
    } else {
        &[]
    }
}
fn tag_kind<'a>(value: &Tag, scratch: &'a mut [u8; 128]) -> &'a str {
    let Tag::String(name) = field(value, "type") else {
        return "";
    };
    let Some(name) = crate::item_defaults::ascii_name(name, scratch) else {
        return "";
    };
    name.strip_prefix("minecraft:").unwrap_or(name)
}
pub fn number_tag(value: &Tag) -> Result<(), String> {
    if let Tag::List(values) = value {
        for value in values {
            number_tag(value)?;
        }
        return Ok(());
    }
    let mut scratch = [0u8; 128];
    let keys: &[&str] = match tag_kind(value, &mut scratch) {
        "uniform" => &["min", "max"],
        "constant" => &["value"],
        "binomial" => &["n", "p"],
        "add" | "mul" | "avg" | "max" | "min" | "length" => &["inputs"],
        "sub" | "div" | "mod" | "floor_div" | "floor_mod" => &["left", "right"],
        "pow" => &["base", "exponent"],
        "abs" | "negate" | "from_float" | "from_int" | "floor" | "ceil" | "round" | "sqrt"
        | "sin" | "cos" | "truncate" => &["input"],
        "score" | "storage" => &["fallback"],
        "conditional" => {
            super::validate_loot_condition_tag(field(value, "condition"))?;
            &["on_true", "on_false"]
        }
        "number_dispatcher" => {
            for case in list(field(value, "cases")) {
                super::validate_loot_condition_tag(field(case, "condition"))?;
                number_tag(field(case, "value"))?;
            }
            &["default"]
        }
        "weighted_list" => {
            for entry in list(field(value, "distribution")) {
                number_tag(field(entry, "data"))?;
            }
            &[]
        }
        _ => &[],
    };
    for key in keys {
        number_tag(field(value, key))?;
    }
    Ok(())
}
pub(super) fn range_tag(value: &Tag) -> Result<(), String> {
    number_tag(value)?;
    number_tag(field(value, "min"))?;
    number_tag(field(value, "max"))
}
pub(super) fn modifier_tag(value: &Tag) -> Result<(), String> {
    let mut scratch = [0u8; 128];
    match tag_kind(value, &mut scratch) {
        "set_count" | "enchanted_count_increase" => number_tag(field(value, "count")),
        "limit_count" => range_tag(field(value, "limit")),
        "set_damage" => number_tag(field(value, "damage")),
        "enchant_with_levels" => number_tag(field(value, "levels")),
        "set_ominous_bottle_amplifier" => number_tag(field(value, "amplifier")),
        "set_random_dyes" => number_tag(field(value, "number_of_dyes")),
        "set_enchantments" => {
            if let Tag::Compound(amounts) = field(value, "enchantments") {
                for amount in amounts.values() {
                    number_tag(amount)?;
                }
            }
            Ok(())
        }
        "set_stew_effect" => {
            for effect in list(field(value, "effects")) {
                number_tag(field(effect, "duration"))?;
            }
            Ok(())
        }
        "set_attributes" => {
            for modifier in list(field(value, "modifiers")) {
                number_tag(field(modifier, "amount"))?;
            }
            Ok(())
        }
        "set_custom_model_data" => {
            for amount in list(field(field(value, "floats"), "values")) {
                number_tag(amount)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
