//! Portable item-stack view, NBT-to-JSON component conversion, server item models, and item registry IDs.
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    sync::Arc,
};
use sunrise_assets::item_model::{ItemModels, StackView};
use sunrise_core::nbt::Tag;

/// An NBT value as the JSON its component codec reads.
pub fn tag_json(tag: &Tag) -> Value {
    match tag {
        Tag::End => Value::Null,
        Tag::Byte(v) => Value::from(*v),
        Tag::Short(v) => Value::from(*v),
        Tag::Int(v) => Value::from(*v),
        Tag::Long(v) => Value::from(*v),
        Tag::Float(bits) => serde_json::Number::from_f64(f64::from(f32::from_bits(*bits)))
            .map_or(Value::Null, Value::Number),
        Tag::Double(bits) => {
            serde_json::Number::from_f64(f64::from_bits(*bits)).map_or(Value::Null, Value::Number)
        }
        Tag::ByteArray(a) => Value::Array(a.iter().map(|b| Value::from(*b as i8)).collect()),
        Tag::String(s) => Value::String(s.to_string_lossy()),
        Tag::List(l) => Value::Array(l.iter().map(tag_json).collect()),
        Tag::Compound(c) => Value::Object(
            c.iter()
                .map(|(k, v)| (k.to_string_lossy(), tag_json(v)))
                .collect(),
        ),
        Tag::IntArray(a) => Value::Array(a.iter().map(|v| Value::from(*v)).collect()),
        Tag::LongArray(a) => Value::Array(a.iter().map(|v| Value::from(*v)).collect()),
    }
}
/// Custom model flags use Codec.BOOL even though NBT stores them as numbers.
/// Keep the other custom-model fields, and generic NBT JSON, numeric.
fn custom_model_data_json(tag: &Tag) -> Value {
    let mut value = tag_json(tag);
    let flags: Option<Vec<Value>> = match tag.get("flags") {
        Some(Tag::List(flags)) => flags
            .iter()
            .map(|flag| crate::item_data::component_bool(flag).map(Value::Bool))
            .collect(),
        Some(Tag::ByteArray(flags)) => flags
            .iter()
            .map(|flag| crate::item_data::component_bool(&Tag::Byte(*flag as i8)).map(Value::Bool))
            .collect(),
        Some(Tag::IntArray(flags)) => flags
            .iter()
            .map(|flag| crate::item_data::component_bool(&Tag::Int(*flag)).map(Value::Bool))
            .collect(),
        Some(Tag::LongArray(flags)) => flags
            .iter()
            .map(|flag| crate::item_data::component_bool(&Tag::Long(*flag)).map(Value::Bool))
            .collect(),
        _ => None,
    };
    if let (Some(flags), Some(object)) = (flags, value.as_object_mut()) {
        object.insert("flags".into(), Value::Array(flags));
    }
    value
}
/// The resolver's view of a client stack: its item, count and component patch.
pub fn stack_view(stack: &crate::items::ItemStack) -> StackView {
    let mut view = StackView::new(&stack.item, stack.count as i32);
    view.bundle_selected = stack.data().map_or(-1, |d| d.bundle_selected.0);
    if let Some(Tag::Compound(c)) = crate::item_data::components_tag(stack.damage, stack.data()) {
        for (k, v) in &c {
            let key = k.to_string_lossy();
            match key.strip_prefix('!') {
                Some(removed) => view.patch.insert(removed.to_owned(), None),
                None => {
                    // The component codec turns NBT numeric booleans into JSON booleans.
                    // Generic NBT conversion must keep unrelated bytes numeric.
                    let value = if key == "minecraft:enchantment_glint_override" {
                        crate::item_data::component_bool(v)
                            .map(Value::Bool)
                            .unwrap_or_else(|| tag_json(v))
                    } else if key == "minecraft:custom_model_data" {
                        custom_model_data_json(v)
                    } else {
                        tag_json(v)
                    };
                    view.patch.insert(key, Some(value))
                }
            };
        }
    }
    view
}

/// Server-only portable `Arc<ItemModels>`: prototypes from `ItemData` with empty client definitions.
pub fn load_for_server(
    item_data: &sunrise_world::item_defaults::ItemData,
) -> std::io::Result<Arc<ItemModels>> {
    let prototypes = Arc::clone(item_data.prototypes()?);
    Ok(Arc::new(ItemModels {
        definitions: BTreeMap::new(),
        prototypes,
    }))
}

/// Load `minecraft:item` registry ids from prepared `registries.json` (`Item.getId`).
pub fn load_item_ids(prepared: &Path) -> HashMap<String, i32> {
    sunrise_protocol::registry::Builtin::find(prepared)
        .and_then(|p| sunrise_protocol::registry::Builtin::load(&p).ok())
        .and_then(|b| {
            let ids = b.get("minecraft:item")?;
            let mut out = HashMap::new();
            for i in 0.. {
                let Some(name) = ids.name(i) else { break };
                out.insert(name.to_owned(), i);
            }
            Some(out)
        })
        .unwrap_or_default()
}

/// `Item.getId`, unavailable when the registry report or item is missing.
pub fn item_registry_id(item_ids: &HashMap<String, i32>, item: &str) -> Option<u32> {
    item_ids
        .get(item)
        .copied()
        .and_then(|id| id.try_into().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_assets::item_model::{ClientItem, Context, DisplayContext, FoilType, LayerContent};

    #[test]
    fn custom_model_flags_select_their_model_after_stack_nbt_decoding() {
        let mut actual = Vec::new();
        for flags in [
            Tag::List(vec![Tag::Byte(1), Tag::Byte(0)]),
            Tag::ByteArray(vec![1, 0]),
        ] {
            let mut stack = crate::items::ItemStack::new("minecraft:diamond", 1);
            let component = Tag::compound([
                ("flags", flags),
                ("floats", Tag::List(vec![Tag::Float(1.25f32.to_bits())])),
                ("colors", Tag::List(vec![Tag::Int(0x12_34_56)])),
                ("strings", Tag::List(vec![Tag::String("keep".into())])),
            ]);
            (stack.damage, stack.extra) = crate::item_data::decode_components(Some(
                &Tag::compound([("minecraft:custom_model_data", component)]),
            ));
            let view = stack_view(&stack);
            for index in 0..3 {
                let mut models = ItemModels::default();
                Arc::make_mut(&mut models.prototypes).insert(
                    "minecraft:diamond".into(),
                    BTreeMap::from([(
                        "minecraft:item_model".into(),
                        serde_json::json!("minecraft:diamond"),
                    )]),
                );
                models.definitions.insert("minecraft:diamond".into(), ClientItem::decode(&serde_json::json!({
                    "model": {"type": "minecraft:condition", "property": "minecraft:custom_model_data", "index": index,
                        "on_true": {"type": "minecraft:model", "model": "sunrise:item/yes"},
                        "on_false": {"type": "minecraft:model", "model": "sunrise:item/no"}}
                })).unwrap());
                let resolved = models.resolve(
                    &view,
                    &Context {
                        display: DisplayContext::Gui,
                        owner: None,
                        level: None,
                        seed: 0,
                    },
                    &sunrise_assets::item_model::NoEnv,
                );
                assert_eq!(resolved.layers.len(), 1);
                actual.push(
                    resolved.layers[0].content
                        == LayerContent::Model {
                            model: "sunrise:item/yes".into(),
                        },
                );
            }
            // Non-boolean fields must retain their codec's numeric/string values.
            let component = view.patch["minecraft:custom_model_data"].as_ref().unwrap();
            assert_eq!(component["floats"], serde_json::json!([1.25]));
            assert_eq!(component["colors"], serde_json::json!([0x12_34_56]));
            assert_eq!(component["strings"], serde_json::json!(["keep"]));
        }
        assert_eq!(actual, [true, false, false, true, false, false]);
    }
    #[test]
    fn nbt_components_read_as_json() {
        let tag = Tag::compound([
            ("a", Tag::Int(3)),
            ("b", Tag::List(vec![Tag::String("x".into())])),
            ("c", Tag::Float(1.5f32.to_bits())),
            ("d", Tag::Byte(1)),
        ]);
        assert_eq!(
            tag_json(&tag),
            serde_json::json!({"a": 3, "b": ["x"], "c": 1.5, "d": 1})
        );
    }
    #[test]
    fn stack_glint_override_reaches_the_resolved_item_layers() {
        use sunrise_assets::item_model::NoEnv;
        let mut models = ItemModels::default();
        models.definitions.insert(
            "sunrise:fixture".into(),
            ClientItem::from_json(
                r#"{"model":{"type":"minecraft:model","model":"sunrise:item/fixture"}}"#,
            )
            .unwrap(),
        );
        for default in [false, true] {
            Arc::make_mut(&mut models.prototypes).insert(
                "minecraft:diamond".into(),
                BTreeMap::from([
                    (
                        "minecraft:item_model".into(),
                        serde_json::json!("sunrise:fixture"),
                    ),
                    (
                        "minecraft:enchantment_glint_override".into(),
                        Value::Bool(default),
                    ),
                ]),
            );
            for (tag, flag) in [
                (Tag::Byte(1), true),
                (Tag::Byte(0), false),
                (Tag::Byte(-1), true),
                (Tag::Short(256), true),
                (Tag::Int(1), true),
                (Tag::Long(1 << 40), true),
                (Tag::Float(0.5_f32.to_bits()), true),
                (Tag::Float((-0.0_f32).to_bits()), false),
                (Tag::Double(0.5_f64.to_bits()), true),
                (Tag::Double((-0.0_f64).to_bits()), false),
                (Tag::Float(f32::NAN.to_bits()), true),
                (Tag::Double(f64::INFINITY.to_bits()), true),
            ] {
                let mut stack = crate::items::ItemStack::new("minecraft:diamond", 1);
                (stack.damage, stack.extra) = crate::item_data::decode_components(Some(
                    &Tag::compound([("minecraft:enchantment_glint_override", tag)]),
                ));
                assert_eq!(stack.has_foil(Some(default)), flag);
                let view = stack_view(&stack);
                assert_eq!(
                    view.patch["minecraft:enchantment_glint_override"],
                    Some(Value::Bool(flag))
                );
                let resolved = models.resolve(
                    &view,
                    &Context {
                        display: DisplayContext::Gui,
                        owner: None,
                        level: None,
                        seed: 0,
                    },
                    &NoEnv,
                );
                assert_eq!(resolved.layers.len(), 1);
                assert_eq!(
                    resolved.layers[0].foil,
                    if flag {
                        FoilType::Standard
                    } else {
                        FoilType::None
                    }
                );
            }
        }
    }
}
