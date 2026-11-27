//! Item stacks in NBT (`ItemStack.CODEC`): id, count and components, alone or in
//! slot lists.
use std::sync::Arc;
use sunrise_core::nbt::{Tag, parse_snbt, snbt_whitespace};
use sunrise_game_data::nbt_value::{number, string, text};

/// The two measured saved forms of `BlockPredicate.CODEC`, and the `Identifier`s
/// of `attribute_modifiers` written whole. Operate only on
/// the fresh ItemStack save, never its model or the shared live component view.
fn canonical_saved_adventure_predicates(components: &mut Tag) {
    fn predicate(value: &mut Tag) {
        let named = match value.get("blocks") {
            Some(Tag::List(names)) => match names.as_slice() {
                [Tag::String(name)]
                    if sunrise_core::identifier::Identifier::parse(&name.to_string_lossy())
                        .is_ok_and(|id| !id.path().is_empty()) =>
                {
                    Some(name.clone())
                }
                _ => None,
            },
            _ => None,
        };
        if let Some(name) = named {
            *value.get_mut("blocks").unwrap() = Tag::String(name);
        }
        // CustomDataPredicate -> NbtPredicate -> TagParser.LENIENT_CODEC.
        // Exact `components` and the item's own custom_data are distinct.
        if let Some(custom) = value
            .get_mut("predicates")
            .and_then(|p| p.get_mut("minecraft:custom_data"))
            && matches!(custom, Tag::Compound(_))
        {
            *custom = Tag::String(sunrise_world::snbt_text::to_snbt(custom).as_str().into());
        }
    }
    for key in ["minecraft:can_break", "minecraft:can_place_on"] {
        match components.get_mut(key) {
            Some(Tag::List(predicates)) => predicates.iter_mut().for_each(predicate),
            Some(value) => predicate(value),
            None => {}
        }
    }
    // `Attribute.CODEC` and `Identifier.CODEC` read a bare name as `minecraft:`'s
    // and write it back whole, so a stack given `attack_damage` saves as
    // `minecraft:attack_damage` (#14641).
    if let Some(Tag::List(entries)) = components.get_mut("minecraft:attribute_modifiers") {
        for entry in entries.iter_mut() {
            for key in ["type", "id"] {
                if let Some(Tag::String(name)) = entry.get_mut(key) {
                    let named =
                        sunrise_world::datafix::dynamic::namespaced(&name.to_string_lossy());
                    *name = named.as_str().into();
                }
            }
        }
    }
}

fn stack_fields(
    stack: &crate::items::ItemStack,
    components: Option<Tag>,
) -> Vec<(&'static str, Tag)> {
    let mut entries = vec![
        ("id", string(&stack.item)),
        ("count", Tag::Int(stack.count as i32)),
    ];
    if let Some(components) = components {
        entries.push(("components", components));
    }
    entries
}

pub fn encode_item(
    stack: &crate::items::ItemStack,
    items: &crate::items::Items,
) -> Vec<(&'static str, Tag)> {
    let components = crate::item_data::encode_components(stack, items).map(|mut components| {
        canonical_saved_adventure_predicates(&mut components);
        components
    });
    stack_fields(stack, components)
}
/// ItemStack.CODEC saved against the actual loaded item prototype.
pub fn item_to_nbt(stack: &crate::items::ItemStack, items: &crate::items::Items) -> Tag {
    Tag::compound(encode_item(stack, items))
}
/// Encode an optional item stack as `ItemStack.OPTIONAL_CODEC` over `JsonOps`:
/// `None` encodes as `{}`, and `Some(stack)` encodes as `{"id", "count", "components": ...}`
/// against the loaded item prototypes.
pub fn item_to_json(
    stack: Option<&crate::items::ItemStack>,
    items: &crate::items::Items,
) -> Result<serde_json::Value, sunrise_world::component_value::json::JsonError> {
    item_to_json_with(stack, items, |_| false, |_| true)
}
/// Encode an optional item stack as `ItemStack.OPTIONAL_CODEC` over `JsonOps`,
/// resolving registered native component types via `is_native` and items via `has_item`.
pub fn item_to_json_with(
    stack: Option<&crate::items::ItemStack>,
    items: &crate::items::Items,
    is_native: impl Fn(&str) -> bool,
    has_item: impl Fn(&str) -> bool,
) -> Result<serde_json::Value, sunrise_world::component_value::json::JsonError> {
    use sunrise_world::component_value::json::JsonError;

    let Some(stack) = stack else {
        return Ok(serde_json::Value::Object(Default::default()));
    };
    if stack.item.as_ref() == "minecraft:air" {
        return Err(JsonError::field("id", "air is not a valid item"));
    }
    if !has_item(&stack.item) {
        return Err(JsonError::field(
            "id",
            format!("unknown item {}", stack.item),
        ));
    }
    if !(1..=99).contains(&stack.count) {
        return Err(JsonError::field("count", "count outside 1..99"));
    }
    let nbt = item_to_nbt(stack, items);
    let template = sunrise_world::component_value::source::item_stack_from_tag_with(
        &nbt, is_native, &has_item,
    )
    .map_err(JsonError::from_codec_error);
    sunrise_core::nbt::discard_tag(nbt);
    template?.to_item_stack_json()
}
/// Decode one complete JSON value as `ItemStack.OPTIONAL_CODEC` over `JsonOps`:
/// `{}` decodes as `Ok(None)`, and a stack object decodes via `ItemStack.CODEC`.
pub fn item_from_json(
    json: &str,
) -> Result<Option<crate::items::ItemStack>, sunrise_world::component_value::json::JsonError> {
    item_from_json_with(json, |_| false, |_| true)
}
/// Decode one complete JSON value as `ItemStack.OPTIONAL_CODEC` over `JsonOps`,
/// resolving registered native component types via `is_native` and every item, nested ones
/// included, via `has_item`.
pub fn item_from_json_with(
    json: &str,
    is_native: impl Fn(&str) -> bool,
    has_item: impl Fn(&str) -> bool,
) -> Result<Option<crate::items::ItemStack>, sunrise_world::component_value::json::JsonError> {
    use sunrise_world::component_value::json::{JsonError, optional_item_stack_from_json_with};

    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|error| JsonError::Syntax(error.to_string()))?;
    let Some(template) = optional_item_stack_from_json_with(&value, is_native, &has_item)? else {
        return Ok(None);
    };
    let components = template
        .patch_to_tag()
        .map_err(JsonError::from_codec_error)?;
    let stack = decode_item_fields(
        &template.item,
        template.count as u32,
        components.as_ref(),
        None,
    );
    if stack.has_refused_consume_effects() {
        return Err(JsonError::field(
            "components",
            "invalid registered consume effect payload",
        ));
    }
    Ok(Some(stack))
}
/// Read one `TagParser.parseCompoundFully` compound as an `ItemStack.CODEC` item.
/// The SNBT grammar and saved-item decoder report their own errors; binary NBT quotas
/// are not part of this text boundary.
pub fn item_from_snbt(snbt: &str) -> Result<crate::items::ItemStack, String> {
    item_from_snbt_with(snbt, |_| false, |_| true)
}

/// Read one `TagParser.parseCompoundFully` compound as an `ItemStack.CODEC` item,
/// resolving registered native component types via `is_native` and every item, nested ones
/// included, via `has_item`.
pub fn item_from_snbt_with(
    snbt: &str,
    is_native: impl Fn(&str) -> bool,
    has_item: impl Fn(&str) -> bool,
) -> Result<crate::items::ItemStack, String> {
    let (tag, end) = parse_snbt(snbt)?;
    if let Some((pos, _)) = snbt[end..]
        .char_indices()
        .find(|&(_, ch)| !snbt_whitespace(ch.into()))
    {
        return Err(format!("argument.nbt.trailing at {}", end + pos));
    }
    item_from_persistent_nbt_with(&tag, is_native, has_item)
}

/// Decode an `ItemStack.CODEC` compound at a strict input boundary. Unlike the save
/// reader, refuse an invalid count or component patch and preserve the diagnostic.
pub fn item_from_persistent_nbt(tag: &Tag) -> Result<crate::items::ItemStack, String> {
    item_from_persistent_nbt_with(tag, |_| false, |_| true)
}

/// Decode an `ItemStack.CODEC` compound at a strict input boundary, resolving registered
/// native component types via `is_native` and items via `has_item`.
pub fn item_from_persistent_nbt_with(
    tag: &Tag,
    is_native: impl Fn(&str) -> bool,
    has_item: impl Fn(&str) -> bool,
) -> Result<crate::items::ItemStack, String> {
    let template = sunrise_world::component_value::source::item_stack_from_tag_with(
        tag, is_native, &has_item,
    )?;
    let components = template.patch_to_tag()?;
    let stack = decode_item_fields(
        &template.item,
        template.count as u32,
        components.as_ref(),
        None,
    );
    if stack.has_refused_consume_effects() {
        // The registered codec reports its own refusal. Do not admit a partial stack.
        return Err("invalid registered consume effect payload".into());
    }
    Ok(stack)
}

fn saved_item_id(tag: &Tag) -> Option<String> {
    let (namespace, path) = sunrise_world::datafix::identifier(&text(tag.get("id"))?)?;
    let id = format!("{namespace}:{path}");
    (id != "minecraft:air").then_some(id)
}
/// A stack read from a save. Earlier Sunrise saves can hold transient components
/// (`creative_slot_lock`, `additional_trade_cost`, `map_post_processing`), so only
/// those entries are dropped and the stack stays; commands and data files use
/// [`item_from_persistent_nbt`]. Cost: one pass over the component keys; the
/// patch is cloned only when it holds a transient entry. A refused registered
/// effect remains in the saved patch but isn't installed in the live stack.
///
/// ItemStack.CODEC's count range is independent of the item's stack limit;
/// an out-of-range count uses the field's default of one. The id is read as an
/// `Identifier`: `stone` is `minecraft:stone` (a summoned display's or a
/// command's NBT often leaves the namespace out), and an id that is no valid
/// identifier makes no stack.
pub fn item_from_nbt(tag: &Tag) -> Option<crate::items::ItemStack> {
    item_from_nbt_with_effects(tag, None)
}

pub(crate) fn item_from_nbt_with_effects(
    tag: &Tag,
    effects: Option<&Arc<[sunrise_world::component_value::consume_effects::Admitted]>>,
) -> Option<crate::items::ItemStack> {
    let id = saved_item_id(tag)?;
    let count = number(tag.get("count"))
        .or_else(|| number(tag.get("Count")))
        .filter(|count| (1..=99).contains(count))
        .unwrap_or(1) as u32;
    let mut persistent = None;
    let components = match tag.get("components") {
        Some(components)
            if sunrise_world::item_defaults::validate_component_map_tag(components).is_err() =>
        {
            let mut stripped = components.clone();
            sunrise_world::item_defaults::strip_transient_component_map_tag(&mut stripped);
            Some(&*persistent.insert(stripped))
        }
        components => components,
    };
    Some(decode_item_fields(&id, count, components, effects))
}

fn decode_item_fields(
    id: &str,
    count: u32,
    components: Option<&Tag>,
    effects: Option<&Arc<[sunrise_world::component_value::consume_effects::Admitted]>>,
) -> crate::items::ItemStack {
    let (damage, extra) = match effects {
        Some(effects) => crate::item_data::decode_network_components(components, effects),
        None => crate::item_data::decode_components(components),
    };
    crate::items::ItemStack {
        item: Arc::from(id),
        count,
        damage,
        extra,
    }
}
pub fn encode_items(slots: &[Option<crate::items::ItemStack>], items: &crate::items::Items) -> Tag {
    Tag::List(encode_items_using(slots, |stack| item_to_nbt(stack, items)))
}

/// Encode the occupied slot rows with the stack encoder owned by their persistence boundary.
pub fn encode_items_using(
    slots: &[Option<crate::items::ItemStack>],
    encode: impl Fn(&crate::items::ItemStack) -> Tag,
) -> Vec<Tag> {
    slots
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let mut tag = encode(s.as_ref()?);
            if let Tag::Compound(entries) = &mut tag {
                entries.insert("Slot".into(), Tag::Byte(i as i8));
            }
            Some(tag)
        })
        .collect()
}
pub fn decode_items(tag: Option<&Tag>, size: usize) -> Vec<Option<crate::items::ItemStack>> {
    let mut slots = vec![None; size];
    if let Some(Tag::List(items)) = tag {
        for it in items {
            let slot = number(it.get("Slot")).unwrap_or(0) as u8 as usize;
            if let Some(cell) = slots.get_mut(slot) {
                *cell = item_from_nbt(it);
            }
        }
    }
    slots
}

/// An explicit stack snapshot, including default-valued component presence.
/// For persistent ItemStack.CODEC output use [`item_to_nbt`] with the catalog.
impl From<&crate::items::ItemStack> for Tag {
    fn from(stack: &crate::items::ItemStack) -> Self {
        let components = crate::item_data::component_snapshot(stack).map(|mut components| {
            canonical_saved_adventure_predicates(&mut components);
            components
        });
        Tag::compound(stack_fields(stack, components))
    }
}

/// A stack read from its saved NBT (`item_from_nbt`); a tag that is no stack is refused as
/// itself.
impl TryFrom<Tag> for crate::items::ItemStack {
    type Error = Tag;
    fn try_from(tag: Tag) -> Result<Self, Tag> {
        item_from_nbt(&tag).ok_or(tag)
    }
}

/// Components as a stack's `components` compound (`components_tag`), undamaged.
impl From<&crate::item_data::StackData> for Tag {
    fn from(data: &crate::item_data::StackData) -> Self {
        crate::item_data::components_tag(0, Some(data))
            .unwrap_or_else(|| Tag::Compound(Default::default()))
    }
}

/// Components read from a stack's `components` compound (`decode_components`); a damage in it
/// is not a component of these.
impl TryFrom<Tag> for crate::item_data::StackData {
    type Error = Tag;
    fn try_from(tag: Tag) -> Result<Self, Tag> {
        let (_, extra) = crate::item_data::decode_components(Some(&tag));
        Ok(extra.map(|data| *data).unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_snbt_reports_the_trailing_data_cursor() {
        assert_eq!(
            item_from_snbt("{id:stone} x").unwrap_err(),
            "argument.nbt.trailing at 11"
        );
    }

    #[test]
    fn item_snbt_validates_component_codecs_registry_membership_and_narrowed_count() {
        let narrowed = item_from_snbt(r#"{id:"minecraft:stone",count:4294967297L}"#).unwrap();
        assert_eq!(narrowed.count, 1);
        let narrowed_max = item_from_snbt(r#"{id:"minecraft:stone",count:4294967395L}"#).unwrap();
        assert_eq!(narrowed_max.count, 99);
        assert!(item_from_snbt(r#"{id:"minecraft:stone",count:4294967296L}"#).is_err());
        assert!(item_from_snbt(r#"{id:"minecraft:stone",count:4294967396L}"#).is_err());

        let negative_damage =
            item_from_snbt(r#"{id:"minecraft:diamond_sword",components:{"minecraft:damage":-1}}"#)
                .unwrap_err();
        assert!(
            negative_damage.contains("minecraft:damage"),
            "{negative_damage}"
        );

        let unknown =
            item_from_snbt(r#"{id:"minecraft:stone",components:{"nomod:unknown":1}}"#).unwrap_err();
        assert!(unknown.contains("nomod:unknown"), "{unknown}");

        let unknown_removal =
            item_from_snbt(r#"{id:"minecraft:stone",components:{"!nomod:unknown":{}}}"#)
                .unwrap_err();
        assert!(
            unknown_removal.contains("nomod:unknown"),
            "{unknown_removal}"
        );

        let registered = item_from_snbt_with(
            r#"{id:"diamond_sword",components:{"damage":5,"!repair_cost":{ignored:1},"sample:charge":{value:5}}}"#,
            |id| id == "sample:charge",
            |_| true,
        )
        .unwrap();
        assert_eq!(&*registered.item, "minecraft:diamond_sword");
        assert_eq!(registered.damage, 5);
        let data = registered.data().unwrap();
        assert_eq!(
            data.other("!minecraft:repair_cost"),
            Some(&Tag::Compound(Default::default()))
        );
        assert_eq!(
            data.other("sample:charge"),
            Some(&Tag::compound([("value", Tag::Int(5))]))
        );
    }

    #[test]
    fn persistent_count_checks_the_java_int_not_the_long() {
        let (tag, _) = sunrise_core::nbt::parse_snbt(r#"{id:"stone",count:4294967301L}"#).unwrap();
        assert_eq!(item_from_persistent_nbt(&tag).unwrap().count, 5);
        assert_eq!(item_from_nbt(&tag).unwrap().count, 1);
    }

    /// `ItemStackTemplate.CODEC`'s `components` is `DataComponentPatch.CODEC.optionalFieldOf`:
    /// absent is the empty patch, a present non-map value fails the stack.
    #[test]
    fn persistent_components_must_be_a_compound() {
        let with = |components: Option<Tag>| {
            let mut tag = Tag::compound([("id", string("minecraft:stone"))]);
            if let (Tag::Compound(map), Some(components)) = (&mut tag, components) {
                map.insert("components".into(), components);
            }
            item_from_persistent_nbt(&tag)
        };
        assert!(with(None).is_ok());
        assert!(
            with(Some(Tag::compound([(
                "minecraft:repair_cost",
                Tag::Int(2)
            )])))
            .is_ok()
        );
        assert!(with(Some(string("not-a-compound"))).is_err());
        assert!(with(Some(Tag::Int(1))).is_err());
    }

    #[test]
    fn nbt_count_defaults_to_one_outside_codec_range() {
        for item in ["minecraft:stone", "minecraft:diamond_sword"] {
            for (raw, expected) in [
                (None, 1),
                (Some(Tag::Int(-1)), 1),
                (Some(Tag::Int(0)), 1),
                (Some(Tag::Int(1)), 1),
                (Some(Tag::Int(2)), 2),
                (Some(Tag::Int(64)), 64),
                (Some(Tag::Int(99)), 99),
                (Some(Tag::Int(100)), 1),
                (Some(Tag::Int(255)), 1),
                (Some(Tag::Int(256)), 1),
                (Some(Tag::Int(i32::MAX)), 1),
                (Some(Tag::Byte(1)), 1),
                (Some(Tag::Float(1.5_f32.to_bits())), 1),
                (Some(string("1")), 1),
            ] {
                let mut tag = Tag::compound([("id", string(item))]);
                if let (Tag::Compound(map), Some(raw)) = (&mut tag, &raw) {
                    map.insert("count".into(), raw.clone());
                }
                let stack = item_from_nbt(&tag).unwrap();
                assert_eq!(stack.count, expected, "{item}: {raw:?}");
                assert_eq!(
                    number(item_to_nbt(&stack, &Default::default()).get("count")),
                    Some(i64::from(expected))
                );
            }
        }
    }

    /// `/summon item_display ~ ~ ~ {item:{id:"stone"}}` shows stone in vanilla:
    /// the id reads as an Identifier, with the default namespace (#8570).
    #[test]
    fn an_id_without_a_namespace_is_a_minecraft_item() {
        let read = |id: &str| item_from_nbt(&Tag::compound([("id", string(id))]));
        assert_eq!(
            read("stone").map(|s| s.item),
            Some("minecraft:stone".into())
        );
        assert_eq!(
            read(":diamond").map(|s| s.item),
            Some("minecraft:diamond".into())
        );
        assert_eq!(read("nexo:ruby").map(|s| s.item), Some("nexo:ruby".into()));
        assert!(read("air").is_none());
        assert!(read("Stone").is_none(), "not a valid identifier");
    }

    #[test]
    fn runtime_counts_remain_independent_of_the_nbt_decode_range() {
        let stack = crate::items::ItemStack::new("minecraft:diamond_sword", 1000);
        assert_eq!(stack.count, 1000);
        assert_eq!(
            number(item_to_nbt(&stack, &Default::default()).get("count")),
            Some(1000)
        );
    }

    /// Items as Sunrise saved them before transient components were filtered out of saves.
    #[test]
    fn a_save_with_a_transient_component_keeps_its_item_and_drops_only_that_entry() {
        for (snbt, transient) in [
            (
                r#"{id:"minecraft:emerald",count:5,components:{"minecraft:additional_trade_cost":3,"minecraft:repair_cost":2}}"#,
                "minecraft:additional_trade_cost",
            ),
            (
                r#"{id:"minecraft:diamond_pickaxe",count:1,components:{"minecraft:creative_slot_lock":{},"minecraft:repair_cost":2}}"#,
                "minecraft:creative_slot_lock",
            ),
        ] {
            let (saved, _) = sunrise_core::nbt::parse_snbt(snbt).unwrap();
            let stack = item_from_nbt(&saved).expect("the stack survives");
            assert_eq!(stack.count, if transient.ends_with("cost") { 5 } else { 1 });
            let live = crate::item_data::components_tag(stack.damage, stack.data()).unwrap();
            assert_eq!(live.get(transient), None);
            assert_eq!(live.get("minecraft:repair_cost"), Some(&Tag::Int(2)));
            // Strict input refuses the component, naming it, instead of stripping it.
            assert!(
                item_from_persistent_nbt(&saved)
                    .unwrap_err()
                    .contains(transient)
            );
        }
    }
}

#[cfg(test)]
mod h285_saved_adventure_tests {
    use super::*;
    use crate::{item_data::components_tag, items::ItemStack};
    fn parse(s: &str) -> Tag {
        sunrise_core::nbt::parse_snbt(s).unwrap().0
    }
    fn item(key: &str, value: Tag) -> Tag {
        Tag::compound([
            ("id", string("minecraft:stone")),
            ("count", Tag::Int(3)),
            (
                "components",
                Tag::compound([
                    (key, value),
                    ("minecraft:custom_data", parse("{keep:17}")),
                    ("minecraft:repair_cost", Tag::Int(2)),
                ]),
            ),
        ])
    }
    #[derive(Clone, Copy)]
    enum Mode {
        Decoded,
        NoSource,
        Mutated,
    }
    fn check(mode: Mode, custom: bool) {
        for key in ["minecraft:can_break", "minecraft:can_place_on"] {
            let wanted = if custom {
                parse(r#"{predicates:{"minecraft:custom_data":{mark:7}}}"#)
            } else {
                parse(r#"{blocks:["minecraft:stone"]}"#)
            };
            let stale = if custom {
                parse(r#"{predicates:{"minecraft:custom_data":{mark:1}}}"#)
            } else {
                parse(r#"{blocks:["minecraft:dirt"]}"#)
            };
            let source = item(
                key,
                if matches!(mode, Mode::Mutated) {
                    stale.clone()
                } else {
                    wanted.clone()
                },
            );
            let mut stack = if matches!(mode, Mode::NoSource) {
                let mut s = ItemStack::new("minecraft:stone", 3);
                s.data_mut().repair_cost = 2;
                s.data_mut()
                    .set_other("minecraft:custom_data", Some(parse("{keep:17}")));
                s.data_mut().set_other(key, Some(wanted.clone()));
                s
            } else {
                item_from_nbt(&source).unwrap()
            };
            if matches!(mode, Mode::Mutated) {
                assert_eq!(stack.data().unwrap().other(key), Some(&stale));
                stack.data_mut().set_other(key, Some(wanted.clone()));
                assert_eq!(source.get("components").unwrap().get(key), Some(&stale));
            }
            assert_eq!((&*stack.item, stack.count), ("minecraft:stone", 3));
            assert_eq!(stack.data().unwrap().other(key), Some(&wanted));
            assert!(stack.data().unwrap().wire.patch().is_none());
            let live = components_tag(stack.damage, stack.data()).unwrap();
            let saved = item_to_nbt(&stack, &Default::default());
            let components = saved.get("components").unwrap();
            assert_eq!(saved.get("id"), Some(&string("minecraft:stone")));
            assert_eq!(saved.get("count"), Some(&Tag::Int(3)));
            assert_eq!(
                components.get("minecraft:custom_data"),
                Some(&parse("{keep:17}"))
            );
            assert_eq!(components.get("minecraft:repair_cost"), Some(&Tag::Int(2)));
            assert_eq!(components_tag(stack.damage, stack.data()), Some(live));
            assert_eq!(stack.data().unwrap().other(key), Some(&wanted));
            let canonical = if custom {
                parse(r#"{predicates:{"minecraft:custom_data":"{mark:7}"}}"#)
            } else {
                parse(r#"{blocks:"minecraft:stone"}"#)
            };
            assert_eq!(
                components.get(key),
                Some(&canonical),
                "actual saved codec must canonicalize the two admitted forms"
            );
        }
    }
    #[test]
    fn decoded_singleton() {
        check(Mode::Decoded, false)
    }
    #[test]
    fn no_source_singleton() {
        check(Mode::NoSource, false)
    }
    #[test]
    fn mutated_singleton_ignores_stale_source() {
        check(Mode::Mutated, false)
    }
    #[test]
    fn decoded_custom_data() {
        check(Mode::Decoded, true)
    }
    #[test]
    fn no_source_custom_data() {
        check(Mode::NoSource, true)
    }
    #[test]
    fn mutated_custom_data_ignores_stale_source() {
        check(Mode::Mutated, true)
    }
    #[test]
    fn canonical_and_unrelated_forms_keep_their_types_and_order() {
        for key in ["minecraft:can_break", "minecraft:can_place_on"] {
            for value in [
                r#"{blocks:"minecraft:stone"}"#,
                r#"{blocks:["minecraft:stone","minecraft:dirt"]}"#,
                r#"[{blocks:"minecraft:stone"},{blocks:"minecraft:dirt"}]"#,
                r#"{predicates:{"minecraft:custom_data":"{mark:7}"}}"#,
                r#"{components:{"minecraft:custom_data":{mark:7}}}"#,
                r#"{state:{axis:"y"}}"#,
                "{}",
            ] {
                let wanted = parse(value);
                let stack = item_from_nbt(&item(key, wanted.clone())).unwrap();
                assert_eq!(
                    item_to_nbt(&stack, &Default::default())
                        .get("components")
                        .unwrap()
                        .get(key),
                    Some(&wanted)
                );
            }
        }
        assert!(
            item_to_nbt(&ItemStack::new("minecraft:stone", 3), &Default::default())
                .get("components")
                .is_none()
        );
    }
    // Added after the frozen old proof: both saved wrappers and repeat safety.
    #[test]
    fn both_saved_wrappers_are_idempotent_without_mutating_the_model() {
        let items = crate::items::Items::for_tests(Default::default());
        for key in ["minecraft:can_break", "minecraft:can_place_on"] {
            for (wanted, stale, expected) in [
                (
                    r#"{blocks:["minecraft:stone"]}"#,
                    r#"{blocks:["minecraft:dirt"]}"#,
                    r#"{blocks:"minecraft:stone"}"#,
                ),
                (
                    r#"{predicates:{"minecraft:custom_data":{mark:7}}}"#,
                    r#"{predicates:{"minecraft:custom_data":{mark:1}}}"#,
                    r#"{predicates:{"minecraft:custom_data":"{mark:7}"}}"#,
                ),
            ] {
                for mode in [Mode::Decoded, Mode::NoSource, Mode::Mutated] {
                    let source = item(
                        key,
                        parse(if matches!(mode, Mode::Mutated) {
                            stale
                        } else {
                            wanted
                        }),
                    );
                    let mut stack = if matches!(mode, Mode::NoSource) {
                        let mut s = ItemStack::new("minecraft:stone", 3);
                        s.data_mut().repair_cost = 2;
                        s.data_mut()
                            .set_other("minecraft:custom_data", Some(parse("{keep:17}")));
                        s.data_mut().set_other(key, Some(parse(wanted)));
                        s
                    } else {
                        item_from_nbt(&source).unwrap()
                    };
                    if matches!(mode, Mode::Mutated) {
                        stack.data_mut().set_other(key, Some(parse(wanted)));
                        assert_eq!(
                            source.get("components").unwrap().get(key),
                            Some(&parse(stale))
                        );
                    }
                    let before = stack.clone();
                    let live = components_tag(stack.damage, stack.data());
                    let a = item_to_nbt(&stack, &Default::default());
                    let b = item_to_nbt(&stack, &items);
                    assert_eq!(a, b);
                    assert_eq!(
                        a.get("components").unwrap().get(key),
                        Some(&parse(expected))
                    );
                    assert_eq!(item_to_nbt(&stack, &Default::default()), a);
                    assert_eq!(item_to_nbt(&stack, &items), b);
                    let back = item_from_nbt(&a).unwrap();
                    assert_eq!(item_to_nbt(&back, &Default::default()), a);
                    assert_eq!(item_to_nbt(&back, &items), a);
                    assert_eq!(stack, before);
                    assert_eq!(components_tag(stack.damage, stack.data()), live);
                    assert_eq!(stack.data().unwrap().other(key), Some(&parse(wanted)));
                    assert_eq!(a.get("id"), Some(&string("minecraft:stone")));
                    assert_eq!(a.get("count"), Some(&Tag::Int(3)));
                    assert_eq!(
                        a.get("components").unwrap().get("minecraft:custom_data"),
                        Some(&parse("{keep:17}"))
                    );
                }
            }
        }
    }
    #[test]
    fn tag_reference_unmeasured_and_exact_forms_keep_the_prior_path() {
        for key in ["minecraft:can_break", "minecraft:can_place_on"] {
            for value in [
                r##"{blocks:"#minecraft:logs"}"##,
                r##"{blocks:["#minecraft:logs"]}"##,
                r#"{blocks:[]}"#,
                r#"{blocks:[""]}"#,
                r#"{blocks:["minecraft:"]}"#,
                r#"{blocks:7}"#,
                r#"{blocks:[7]}"#,
                r#"{nbt:"{mark:7}"}"#,
                r#"{components:{"minecraft:custom_data":{mark:7}}}"#,
            ] {
                let value = parse(value);
                let stack = item_from_nbt(&item(key, value.clone())).unwrap();
                assert_eq!(
                    item_to_nbt(&stack, &Default::default())
                        .get("components")
                        .unwrap()
                        .get(key),
                    Some(&value)
                );
            }
        }
        let value =
            parse(r#"{blocks:["minecraft:stone"],predicates:{"minecraft:custom_data":{mark:7}}}"#);
        let mut stack = ItemStack::new("minecraft:stone", 3);
        stack
            .data_mut()
            .set_other("minecraft:custom_data", Some(value.clone()));
        stack
            .data_mut()
            .set_other("!minecraft:can_break", Some(parse("{}")));
        let saved = item_to_nbt(&stack, &Default::default());
        assert_eq!(
            saved
                .get("components")
                .unwrap()
                .get("minecraft:custom_data"),
            Some(&value)
        );
        assert_eq!(
            saved.get("components").unwrap().get("!minecraft:can_break"),
            Some(&parse("{}"))
        );
    }
    #[test]
    fn direct_encoder_and_nested_templates_are_outside_the_projection() {
        let raw =
            parse(r#"{blocks:["minecraft:stone"],predicates:{"minecraft:custom_data":{mark:7}}}"#);
        let mut inner = ItemStack::new("minecraft:stone", 3);
        inner
            .data_mut()
            .set_other("minecraft:can_break", Some(raw.clone()));
        assert_eq!(
            crate::item_data::encode_components(&inner, &Default::default())
                .unwrap()
                .get("minecraft:can_break"),
            Some(&raw)
        );
        assert_eq!(
            components_tag(inner.damage, inner.data())
                .unwrap()
                .get("minecraft:can_break"),
            Some(&raw)
        );
        let mut outer = ItemStack::new("minecraft:shulker_box", 1);
        outer.data_mut().container.push((0, inner));
        let saved = item_to_nbt(&outer, &Default::default());
        let Some(Tag::List(slots)) = saved.get("components").unwrap().get("minecraft:container")
        else {
            panic!("container")
        };
        assert_eq!(
            slots[0]
                .get("item")
                .unwrap()
                .get("components")
                .unwrap()
                .get("minecraft:can_break"),
            Some(&raw),
            "retained source gap, not a Java parity assertion"
        );
    }
    /// `Attribute.CODEC` and `Identifier.CODEC` save a bare name as
    /// `minecraft:`'s: vanilla's `/item replace` of `type:"attack_damage"`
    /// saves `type:"minecraft:attack_damage"` (checked against the 26.3 server
    /// by `test_builtin_component_delivery_parity`, #14641).
    #[test]
    fn a_bare_attribute_and_modifier_name_save_namespaced() {
        let entry = |attribute: &str, id: &str| {
            parse(&format!(
                r#"{{type:"{attribute}",id:"{id}",amount:20.0d,operation:"add_value",slot:"mainhand"}}"#
            ))
        };
        let mut stack = ItemStack::new("minecraft:iron_pickaxe", 1);
        stack.data_mut().set_other(
            "minecraft:attribute_modifiers",
            Some(Tag::List(vec![
                entry("attack_damage", "base_attack_damage"),
                entry("minecraft:armor", "mod:x"),
            ])),
        );
        let saved = item_to_nbt(&stack, &Default::default());
        assert_eq!(
            saved
                .get("components")
                .unwrap()
                .get("minecraft:attribute_modifiers"),
            Some(&Tag::List(vec![
                entry("minecraft:attack_damage", "minecraft:base_attack_damage"),
                entry("minecraft:armor", "mod:x"),
            ]))
        );
        // The live stack keeps what it was given: only the save is canonical.
        assert_eq!(
            stack.data().unwrap().other("minecraft:attribute_modifiers"),
            Some(&Tag::List(vec![
                entry("attack_damage", "base_attack_damage"),
                entry("minecraft:armor", "mod:x"),
            ]))
        );
    }
}

#[cfg(test)]
mod item_json_tests {
    use super::*;
    use crate::items::ItemStack;
    use serde_json::json;
    use sunrise_world::component_value::json::JsonError;

    #[test]
    fn item_json_roundtrips_plain_empty_multi_component_and_removal_against_prototypes() {
        let mut items = crate::items::Items::for_tests(Default::default());
        let mut sword_defaults = sunrise_core::nbt::Compound::new();
        sword_defaults.insert("minecraft:damage".into(), Tag::Int(0));
        sword_defaults.insert("minecraft:max_damage".into(), Tag::Int(1561));
        let mut defaults = std::collections::HashMap::new();
        defaults.insert("minecraft:diamond_sword".to_owned(), sword_defaults);
        items.component_defaults = Arc::new(defaults);

        // Empty stack <-> {}
        assert_eq!(item_to_json(None, &items).unwrap(), json!({}));
        assert_eq!(item_from_json("{}").unwrap(), None);

        // Plain stack <-> {"id", "count"}
        let stone = ItemStack::new("minecraft:stone", 64);
        let stone_json = json!({"id": "minecraft:stone", "count": 64});
        assert_eq!(item_to_json(Some(&stone), &items).unwrap(), stone_json);
        assert_eq!(
            item_from_json(&stone_json.to_string()).unwrap(),
            Some(stone)
        );

        // Multi-component stack: styled text component (real JSON bool), enchantments,
        // int damage, and float attack charge.
        let sword_json = json!({
            "id": "minecraft:diamond_sword",
            "count": 1,
            "components": {
                "minecraft:custom_name": {"text": "Blade", "color": "gold", "bold": true},
                "minecraft:enchantments": {"minecraft:sharpness": 3},
                "minecraft:damage": 5,
                "minecraft:minimum_attack_charge": 0.5
            }
        });
        let sword = item_from_json(&sword_json.to_string()).unwrap().unwrap();
        assert_eq!(sword.damage, 5);
        assert_eq!(item_to_json(Some(&sword), &items).unwrap(), sword_json);

        // Explicit removal against a prototype that defines minecraft:damage.
        let removed_json = json!({
            "id": "minecraft:diamond_sword",
            "count": 1,
            "components": {
                "!minecraft:damage": {}
            }
        });
        let removed = item_from_json(&removed_json.to_string()).unwrap().unwrap();
        assert_eq!(item_to_json(Some(&removed), &items).unwrap(), removed_json);

        // Prototype-equal damage (0) is elided on encode.
        let fresh_sword = ItemStack::new("minecraft:diamond_sword", 1);
        assert_eq!(
            item_to_json(Some(&fresh_sword), &items).unwrap(),
            json!({"id": "minecraft:diamond_sword", "count": 1})
        );

        // Decode errors name the failing field.
        assert!(matches!(
            item_from_json(r#"{"id":"minecraft:diamond_sword","components":{"minecraft:damage":-1}}"#),
            Err(JsonError::Field { field, .. }) if field == "minecraft:damage"
        ));
        assert!(matches!(
            item_from_json(r#"{"id":"minecraft:stone","count":100}"#),
            Err(JsonError::Field { field, .. }) if field == "count"
        ));
        assert!(matches!(
            item_from_json("{not valid json"),
            Err(JsonError::Syntax(_))
        ));
    }
}
