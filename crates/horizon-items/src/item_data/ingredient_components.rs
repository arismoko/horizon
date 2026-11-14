//! Borrowed live component queries for decoded ingredient predicates.
use super::*;

/// Visit present component names without serializing or allocating.
pub fn visit_components(stack: &ItemStack, mut visit: impl FnMut(&str)) {
    for id in MODELLED {
        if stack.carries_component(None, id) {
            visit(id);
        }
    }
    for id in [
        "minecraft:item_name",
        "minecraft:rarity",
        "minecraft:tooltip_display",
    ] {
        if stack.carries_component(None, id) {
            visit(id);
        }
    }
    if let Some(data) = stack.data() {
        for (id, _) in data.preserved_components() {
            if !id.starts_with('!') && !MODELLED.contains(&id.as_str()) {
                visit(id);
            }
        }
    }
}

/// Whether a patch explicitly removed this component, without building a prefixed key.
pub fn removed(stack: &ItemStack, id: &str) -> bool {
    !stack.carries_component(None, id)
        && stack.data().is_some_and(|data| {
            data.preserved_components()
                .iter()
                .any(|(key, _)| key.strip_prefix('!') == Some(id))
        })
}

/// Equality in the decoded representation used by the shared component codecs.
/// Presence is checked by the caller, so empty decoded enchantment maps compare equal.
pub fn component_equal(id: &str, a: &ItemStack, b: &ItemStack) -> bool {
    match id {
        "minecraft:damage" => a.damage == b.damage,
        "minecraft:repair_cost" => {
            a.data().map_or(0, |d| d.repair_cost) == b.data().map_or(0, |d| d.repair_cost)
        }
        "minecraft:enchantments" => levels_equal(
            a.data().map_or(&[], |d| d.enchantments.as_slice()),
            b.data().map_or(&[], |d| d.enchantments.as_slice()),
        ),
        "minecraft:stored_enchantments" => levels_equal(
            a.data().map_or(&[], |d| d.stored_enchantments.as_slice()),
            b.data().map_or(&[], |d| d.stored_enchantments.as_slice()),
        ),
        "minecraft:custom_name" => {
            a.data().and_then(|d| d.custom_name.as_ref())
                == b.data().and_then(|d| d.custom_name.as_ref())
        }
        "minecraft:max_damage" | "minecraft:max_stack_size" => {
            a.data()
                .and_then(|d| d.other(id))
                .and_then(crate::properties::component_int)
                == b.data()
                    .and_then(|d| d.other(id))
                    .and_then(crate::properties::component_int)
        }
        "minecraft:block_entity_data" => block_entity_equal(
            a.data().and_then(|d| d.other(id)),
            b.data().and_then(|d| d.other(id)),
        ),
        "minecraft:custom_data" => match (
            a.data().and_then(|d| d.other(id)),
            b.data().and_then(|d| d.other(id)),
        ) {
            (Some(a @ Tag::Compound(_)), Some(b @ Tag::Compound(_))) => custom_data_equal(a, b),
            _ => false,
        },
        "minecraft:item_name" => item_name_equal(a, b),
        "minecraft:item_model" => identifier_equal(
            a.data().and_then(|d| d.other(id)),
            b.data().and_then(|d| d.other(id)),
        ),
        "minecraft:rarity" => rarity(a) == rarity(b),
        _ => stored_component_equal(id, a, b),
    }
}

/// Exact stored-value comparison, including the fields removed from `other` by decoding.
pub fn stored_component_equal(id: &str, a: &ItemStack, b: &ItemStack) -> bool {
    if id == "minecraft:damage" {
        return a.damage == b.damage;
    }
    if !MODELLED.contains(&id) {
        return a.data().and_then(|d| d.other(id)) == b.data().and_then(|d| d.other(id));
    }
    let raw_a = a.data().and_then(|data| {
        data.preserved_components()
            .iter()
            .find(|(key, _)| key == id)
            .map(|(_, value)| value)
    });
    let raw_b = b.data().and_then(|data| {
        data.preserved_components()
            .iter()
            .find(|(key, _)| key == id)
            .map(|(_, value)| value)
    });
    // Empty/malformed retained values are not interchangeable with a populated modelled field.
    if raw_a.is_some() || raw_b.is_some() {
        return raw_a == raw_b;
    }
    let empty = StackData::default();
    let a = a.data().unwrap_or(&empty);
    let b = b.data().unwrap_or(&empty);
    match id {
        "minecraft:potion_contents" => {
            a.potion == b.potion
                && a.potion_extra.len() == b.potion_extra.len()
                && a.potion_extra
                    .iter()
                    .all(|entry| b.potion_extra.contains(entry))
        }
        "minecraft:bundle_contents" => stacks_equal(&a.bundle_contents, &b.bundle_contents),
        "minecraft:charged_projectiles" => stacks_equal(&a.charged, &b.charged),
        "minecraft:container" => {
            a.container.len() == b.container.len()
                && a.container
                    .iter()
                    .zip(&b.container)
                    .all(|((slot, stack), (other, value))| slot == other && stack.matches(value))
        }
        "minecraft:trim" => a.trim == b.trim,
        "minecraft:fireworks" => a.fireworks == b.fireworks,
        "minecraft:firework_explosion" => a.firework_explosion == b.firework_explosion,
        "minecraft:map_id" => a.map_id == b.map_id,
        "minecraft:base_color" => a.base_color == b.base_color,
        "minecraft:dyed_color" => a.dyed_color == b.dyed_color,
        "minecraft:ominous_bottle_amplifier" => a.ominous_amplifier == b.ominous_amplifier,
        "minecraft:map_post_processing" => a.map_post_processing == b.map_post_processing,
        "minecraft:writable_book_content" => a.writable_pages == b.writable_pages,
        "minecraft:written_book_content" => a.written == b.written,
        "minecraft:suspicious_stew_effects" => a.stew_effects == b.stew_effects,
        "minecraft:pot_decorations" => match (&a.pot_decorations, &b.pot_decorations) {
            (Some(a), Some(b)) => a.iter().zip(b.iter()).all(|(a, b)| match (a, b) {
                (Some(a), Some(b)) => a.matches(b),
                (None, None) => true,
                _ => false,
            }),
            (None, None) => true,
            _ => false,
        },
        "minecraft:banner_patterns" => banner_equal(a, b),
        "minecraft:consumable" => {
            a.consumable.as_ref().map(|value| &value.raw)
                == b.consumable.as_ref().map(|value| &value.raw)
        }
        "minecraft:kinetic_weapon" => {
            a.kinetic_weapon.as_ref().map(|value| value.raw())
                == b.kinetic_weapon.as_ref().map(|value| value.raw())
        }
        // These have shared decoders; keep this helper complete for direct users too.
        "minecraft:repair_cost" => a.repair_cost == b.repair_cost,
        "minecraft:enchantments" => levels_equal(&a.enchantments, &b.enchantments),
        "minecraft:stored_enchantments" => {
            levels_equal(&a.stored_enchantments, &b.stored_enchantments)
        }
        "minecraft:custom_name" => a.custom_name == b.custom_name,
        _ => unreachable!("every modelled component has a borrowed comparison"),
    }
}
fn stacks_equal(a: &[ItemStack], b: &[ItemStack]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.matches(b))
}
fn banner_equal(a: &StackData, b: &StackData) -> bool {
    match (a.ominous_banner, b.ominous_banner) {
        (true, true) => true,
        (false, false) => a.banner_patterns == b.banner_patterns,
        (true, false) => ominous_layers(&b.banner_patterns),
        (false, true) => ominous_layers(&a.banner_patterns),
    }
}
fn ominous_layers(layers: &[crate::banner_layer::BannerLayer]) -> bool {
    layers.len() == OMINOUS_LAYERS.len()
        && layers
            .iter()
            .zip(OMINOUS_LAYERS)
            .all(|(layer, (pattern, color))| {
                layer
                    .pattern
                    .strip_prefix("minecraft:")
                    .unwrap_or(&layer.pattern)
                    == pattern
                    && layer.color.as_ref() == color
            })
}

fn java_is(value: &JavaString, text: &str) -> bool {
    value.0.iter().copied().eq(text.encode_utf16())
}
fn implicit_ominous_name(stack: &ItemStack) -> bool {
    stack
        .data()
        .is_some_and(|data| data.ominous_banner && data.other("minecraft:item_name").is_none())
}
fn is_ominous_name(tag: Option<&Tag>) -> bool {
    let Some(Tag::Compound(map)) = tag else {
        return false;
    };
    let mut translate = false;
    map.iter().all(|(key, value)| {
        if java_is(key, "translate") {
            translate = matches!(value, Tag::String(value) if java_is(value, "block.minecraft.ominous_banner"));
            translate
        } else if java_is(key, "type") {
            matches!(value, Tag::String(value) if java_is(value, "translatable"))
        } else if java_is(key, "extra") || java_is(key, "with") {
            matches!(value, Tag::List(values) if values.is_empty())
        } else { false }
    }) && translate
}
fn item_name_equal(a: &ItemStack, b: &ItemStack) -> bool {
    let a_name = a.data().and_then(|data| data.other("minecraft:item_name"));
    let b_name = b.data().and_then(|data| data.other("minecraft:item_name"));
    match (implicit_ominous_name(a), implicit_ominous_name(b)) {
        (true, true) => true,
        (true, false) => is_ominous_name(b_name),
        (false, true) => is_ominous_name(a_name),
        (false, false) => a_name.map(text_value) == b_name.map(text_value),
    }
}
fn rarity(stack: &ItemStack) -> Option<u8> {
    let data = stack.data()?;
    match data.other("minecraft:rarity") {
        Some(Tag::String(value)) => ["common", "uncommon", "rare", "epic"]
            .iter()
            .position(|text| java_is(value, text))
            .map(|index| index as u8),
        Some(_) => None,
        None => data.ominous_banner.then_some(1),
    }
}

/// `Identifier.bySeparator`/`equals`: default namespaces, borrowing Java UTF-16.
fn identifier_equal(a: Option<&Tag>, b: Option<&Tag>) -> bool {
    let (Some(Tag::String(a)), Some(Tag::String(b))) = (a, b) else {
        return false;
    };
    fn units(value: &JavaString) -> impl Iterator<Item = u16> + '_ {
        let (prefix, path) = match value.0.iter().position(|unit| *unit == u16::from(b':')) {
            Some(0) => ("minecraft:", &value.0[1..]),
            None => ("minecraft:", value.0.as_slice()),
            Some(_) => ("", value.0.as_slice()),
        };
        prefix.encode_utf16().chain(path.iter().copied())
    }
    units(a).eq(units(b))
}

/// `CustomData.equals`, not `matchedBy`: compounds are exact, lists are ordered,
/// and FloatTag/DoubleTag records use Java's NaN-canonicalizing floating equality.
fn custom_data_equal(a: &Tag, b: &Tag) -> bool {
    match (a, b) {
        (Tag::Float(a), Tag::Float(b)) => {
            a == b || (f32::from_bits(*a).is_nan() && f32::from_bits(*b).is_nan())
        }
        (Tag::Double(a), Tag::Double(b)) => {
            a == b || (f64::from_bits(*a).is_nan() && f64::from_bits(*b).is_nan())
        }
        (Tag::Compound(a), Tag::Compound(b)) => {
            a.len() == b.len()
                && a.iter().zip(b).all(|((key, value), (other_key, other))| {
                    key == other_key && custom_data_equal(value, other)
                })
        }
        (Tag::List(a), Tag::List(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| custom_data_equal(a, b))
        }
        _ => a == b,
    }
}

fn block_entity_equal(a: Option<&Tag>, b: Option<&Tag>) -> bool {
    let (Some(Tag::Compound(a)), Some(Tag::Compound(b))) = (a, b) else {
        return a == b;
    };
    if a.len() != b.len() {
        return false;
    }
    a.iter().all(|(key, value)| {
        let Some(other) = b.get(key) else {
            return false;
        };
        if key.0.iter().copied().eq("id".encode_utf16()) {
            let (Tag::String(a), Tag::String(b)) = (value, other) else {
                return false;
            };
            let a_prefix = if a.0.contains(&u16::from(b':')) {
                ""
            } else {
                "minecraft:"
            };
            let b_prefix = if b.0.contains(&u16::from(b':')) {
                ""
            } else {
                "minecraft:"
            };
            a_prefix
                .encode_utf16()
                .chain(a.0.iter().copied())
                .eq(b_prefix.encode_utf16().chain(b.0.iter().copied()))
        } else {
            value == other
        }
    })
}

fn levels_equal(a: &[(Arc<str>, u32)], b: &[(Arc<str>, u32)]) -> bool {
    a.len() == b.len() && a.iter().all(|entry| b.contains(entry))
}
// Borrow the same plain-literal normalization as collapsed_text, without cloning a tag.
fn text_value(tag: &Tag) -> &Tag {
    let Tag::Compound(map) = tag else { return tag };
    let key_is = |key: &JavaString, value: &str| key.0.iter().copied().eq(value.encode_utf16());
    let Some((_, text @ Tag::String(_))) = map.iter().find(|(key, _)| key_is(key, "text")) else {
        return tag;
    };
    if map.iter().all(|(key, value)| {
        key_is(key, "text")
            || key_is(key, "type") && matches!(value, Tag::String(value) if key_is(value, "text"))
            || key_is(key, "extra") && matches!(value, Tag::List(values) if values.is_empty())
    }) {
        text
    } else {
        tag
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decoded_component_equality_borrows_maps_and_plain_text() {
        let mut a = ItemStack::new("minecraft:stone", 1);
        let mut b = ItemStack::new("minecraft:stone", 1);
        a.data_mut().enchantments = vec![
            ("minecraft:sharpness".into(), 1),
            ("minecraft:unbreaking".into(), 2),
        ];
        b.data_mut().enchantments = vec![
            ("minecraft:unbreaking".into(), 2),
            ("minecraft:sharpness".into(), 1),
        ];
        assert!(component_equal("minecraft:enchantments", &a, &b));
        a.data_mut()
            .set_other("minecraft:item_name", Some(Tag::String("Named".into())));
        b.data_mut().set_other(
            "minecraft:item_name",
            Some(Tag::compound([("text", Tag::String("Named".into()))])),
        );
        assert!(component_equal("minecraft:item_name", &a, &b));
        b.data_mut().enchantments[0].1 = 3;
        assert!(!component_equal("minecraft:enchantments", &a, &b));
    }
}
