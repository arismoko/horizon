//! Encodes clientbound item stacks with the connection's registry ids and the shared scalar NBT reads.
use crate::items::ItemStack;
use sunrise_game_data::nbt_value::{float, number, text};
use sunrise_protocol::item::Stack;
use sunrise_world::component_value::consume_effects::Admitted;

/// A rocket's `minecraft:fireworks` and a star's `minecraft:firework_explosion`.
pub fn firework_components(
    d: &crate::item_data::StackData,
) -> Vec<(i32, sunrise_protocol::item::Component)> {
    use sunrise_protocol::item::{Component, component};
    let mut out = Vec::new();
    if let Some((flight, explosions)) = &d.fireworks {
        out.push((
            component::FIREWORKS,
            Component::Fireworks {
                flight: i32::from(*flight),
                explosions: explosions.iter().map(network_explosion).collect(),
            },
        ));
    }
    if let Some(e) = &d.firework_explosion {
        out.push((
            component::FIREWORK_EXPLOSION,
            Component::FireworkExplosion(network_explosion(e)),
        ));
    }
    out
}

/// `DyeColor.getId` of a dye's name.
fn dye_id(name: &str) -> Option<i32> {
    sunrise_level::dyes::DYES
        .iter()
        .position(|d| d.0 == name)
        .map(|i| i as i32)
}

/// `SignText.STREAM_CODEC` of a saved `SignText.CODEC` value: the four
/// messages, the four filtered ones when it has them, the dye's id (black when
/// absent) and the glow.
fn sign_text_bytes(text: &sunrise_core::nbt::Tag) -> Option<Vec<u8>> {
    use sunrise_core::nbt::Tag;
    let lines = |key: &str| match text.get(key) {
        Some(Tag::List(lines)) if lines.len() == 4 => Some(lines),
        _ => None,
    };
    let mut w = ::sunrise_core::vanilla_bytes::Writer::new();
    lines("messages")?.iter().for_each(|line| w.nbt(line));
    let filtered = lines("filtered_messages");
    w.bool(filtered.is_some());
    filtered.into_iter().flatten().for_each(|line| w.nbt(line));
    let color = match text.get("color") {
        Some(Tag::String(color)) => color.to_string_lossy(),
        _ => "black".into(),
    };
    w.varint(dye_id(&color)?);
    w.bool(matches!(text.get("has_glowing_text"), Some(Tag::Byte(b)) if *b != 0));
    Some(w.data)
}

/// `FireworkExplosion.STREAM_CODEC`'s fields (`Shape` by its BY_ID order).
fn network_explosion(
    e: &sunrise_items::firework::FireworkExplosion,
) -> sunrise_protocol::item::FireworkExplosion {
    sunrise_protocol::item::FireworkExplosion {
        shape: e.shape as i32,
        colors: e.colors.iter().map(|c| *c as i32).collect(),
        fade: e.fade.iter().map(|c| *c as i32).collect(),
        trail: e.trail,
        twinkle: e.twinkle,
    }
}

/// The off hand slot is conventionally 40 in saves and 45 in the menu.
/// `synced_id` names an entry of a synced registry (enchantments, trim
/// materials and patterns) by its id on this connection.
pub fn stack_to_network(
    item_id: impl Fn(&str) -> i32,
    synced_id: impl Fn(&str, &str) -> Option<i32>,
    s: &Option<ItemStack>,
) -> Option<Stack> {
    stack_to_network_projecting(item_id, synced_id, s, &|_| ())
}

/// Send the stack and expose effects materialized during this encoding to its hash input.
pub fn stack_to_network_projecting(
    item_id: impl Fn(&str) -> i32,
    synced_id: impl Fn(&str, &str) -> Option<i32>,
    s: &Option<ItemStack>,
    projected: &dyn Fn(&Admitted),
) -> Option<Stack> {
    Some(network_stack_projecting(
        &item_id,
        &synced_id,
        s.as_ref()?,
        projected,
    ))
}
/// `ProvidesTrimMaterial.STREAM_CODEC`, `ProvidesBannerPatterns.STREAM_CODEC`,
/// `AdventureModePredicate.STREAM_CODEC` and `ItemAttributeModifiers.STREAM_CODEC`:
/// encode supported typed saved forms.
/// A failed child discards this entire local buffer, including its prefix.
fn builtin_component(
    name: &str,
    value: &sunrise_core::nbt::Tag,
    ids: &dyn Fn(&str, &str) -> Option<i32>,
) -> Option<sunrise_protocol::item::Component> {
    use sunrise_core::nbt::Tag;
    use sunrise_core::vanilla_bytes::Writer;
    let mut w = Writer::new();
    match name {
        "minecraft:attribute_modifiers" => {
            return crate::item_sync::attribute_modifiers::encode(value, ids);
        }
        "minecraft:provides_trim_material" => match value {
            Tag::String(name) => {
                let name = name.to_string().ok()?;
                let id = ids(
                    "minecraft:trim_material",
                    &sunrise_world::datafix::dynamic::namespaced(&name),
                )?;
                if id < 0 {
                    return None;
                }
                w.varint(id.checked_add(1)?);
            }
            Tag::Compound(map) => {
                if map.keys().any(|k| {
                    !matches!(
                        k.to_string().ok().as_deref(),
                        Some("palette_id" | "description")
                    )
                }) {
                    return None;
                }
                let Tag::String(palette) = value.get("palette_id")? else {
                    return None;
                };
                let description @ Tag::String(_) = value.get("description")? else {
                    return None;
                };
                w.varint(0);
                w.string(&sunrise_world::datafix::dynamic::namespaced(
                    &palette.to_string().ok()?,
                ));
                w.bytes(&sunrise_core::nbt::write_network(description, Default::default()).ok()?);
            }
            _ => return None,
        },
        "minecraft:provides_banner_patterns" => {
            write_holder_set("minecraft:banner_pattern", value, ids, &mut w)?
        }
        "minecraft:can_place_on" | "minecraft:can_break" => {
            let predicates = match value {
                Tag::List(list) if !list.is_empty() => list.as_slice(),
                Tag::Compound(_) => std::slice::from_ref(value),
                _ => return None,
            };
            w.varint(i32::try_from(predicates.len()).ok()?);
            for predicate in predicates {
                write_block_predicate(predicate, ids, &mut w)?;
            }
        }
        "minecraft:tool" => write_tool(value, ids, &mut w)?,
        _ => return None,
    }
    Some(sunrise_protocol::item::Component::Raw(w.data))
}

/// `Tool.STREAM_CODEC` from `Tool.CODEC`'s saved form: the rules (blocks, an
/// optional speed and an optional `correct_for_drops`), then the default
/// mining speed, the wear per block and `can_destroy_blocks_in_creative`,
/// each at its codec default when the stack leaves it out.
fn write_tool(
    value: &sunrise_core::nbt::Tag,
    ids: &dyn Fn(&str, &str) -> Option<i32>,
    w: &mut ::sunrise_core::vanilla_bytes::Writer,
) -> Option<()> {
    use sunrise_core::nbt::Tag;
    use sunrise_world::datafix::dynamic::{as_bool_opt, as_number};
    let Tag::List(rules) = value.get("rules")? else {
        return None;
    };
    w.varint(i32::try_from(rules.len()).ok()?);
    for rule in rules.as_slice() {
        write_holder_set("minecraft:block", rule.get("blocks")?, ids, w)?;
        match rule.get("speed") {
            Some(speed) => {
                w.bool(true);
                w.f32(as_number(speed)?.float_value());
            }
            None => w.bool(false),
        }
        match rule.get("correct_for_drops") {
            Some(correct) => {
                w.bool(true);
                w.bool(as_bool_opt(Some(correct))?);
            }
            None => w.bool(false),
        }
    }
    let number = |key: &str| value.get(key).map(as_number);
    w.f32(match number("default_mining_speed") {
        Some(speed) => speed?.float_value(),
        None => 1.0,
    });
    // ExtraCodecs.NON_NEGATIVE_INT.
    let wear = match number("damage_per_block") {
        Some(wear) => wear?.int_value(),
        None => 1,
    };
    w.varint((wear >= 0).then_some(wear)?);
    w.bool(match value.get("can_destroy_blocks_in_creative") {
        Some(can) => as_bool_opt(Some(can))?,
        None => true,
    });
    Some(())
}

/// `ByteBufCodecs.holderSet`: tags or direct registry entries, without item-id remapping.
fn write_holder_set(
    registry: &str,
    value: &sunrise_core::nbt::Tag,
    ids: &dyn Fn(&str, &str) -> Option<i32>,
    w: &mut ::sunrise_core::vanilla_bytes::Writer,
) -> Option<()> {
    use sunrise_core::nbt::Tag;
    if let Tag::String(name) = value {
        let name = name.to_string().ok()?;
        if let Some(tag) = name.strip_prefix('#') {
            if tag.is_empty() {
                return None;
            }
            w.varint(0);
            w.string(&sunrise_world::datafix::dynamic::namespaced(tag));
            return Some(());
        }
    }
    let entries = match value {
        Tag::List(list) => list.as_slice(),
        Tag::String(_) => std::slice::from_ref(value),
        _ => return None,
    };
    w.varint(i32::try_from(entries.len()).ok()?.checked_add(1)?);
    for entry in entries {
        let Tag::String(name) = entry else {
            return None;
        };
        let name = name.to_string().ok()?;
        if name.starts_with('#') {
            return None;
        }
        let id = ids(
            registry,
            &sunrise_world::datafix::dynamic::namespaced(&name),
        )?;
        if id < 0 {
            return None;
        }
        w.varint(id);
    }
    Some(())
}

/// `BlockPredicate.STREAM_CODEC`: every admitted field is a conjunction.
fn write_block_predicate(
    value: &sunrise_core::nbt::Tag,
    ids: &dyn Fn(&str, &str) -> Option<i32>,
    w: &mut ::sunrise_core::vanilla_bytes::Writer,
) -> Option<()> {
    use sunrise_core::nbt::Tag;
    let Tag::Compound(map) = value else {
        return None;
    };
    if map.keys().any(|k| {
        !matches!(
            k.to_string().ok().as_deref(),
            Some("blocks" | "state" | "nbt" | "components" | "predicates")
        )
    }) {
        return None;
    }
    w.bool(value.get("blocks").is_some());
    if let Some(blocks) = value.get("blocks") {
        write_holder_set("minecraft:block", blocks, ids, w)?;
    }
    w.bool(value.get("state").is_some());
    if let Some(Tag::Compound(state)) = value.get("state") {
        // StatePropertiesPredicate's codec iterates a Java HashMap. Keep the
        // existing map implementation and a private UTF-16 String key adapter.
        #[derive(PartialEq, Eq)]
        struct Key<'a>(&'a sunrise_core::nbt::JavaString);
        impl sunrise_core::java_hash_map::JavaKey for Key<'_> {
            fn java_hash(&self) -> i32 {
                self.0
                    .0
                    .iter()
                    .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(*c as i32))
            }
            fn comparable_compare(&self, other: &Self) -> Option<std::cmp::Ordering> {
                Some(self.0.cmp(other.0))
            }
            fn tie_break(&self, other: &Self) -> std::cmp::Ordering {
                self.0.cmp(other.0)
            }
        }
        let mut ordered = sunrise_core::java_hash_map::JavaHashMap::default();
        for (key, matcher) in state {
            ordered.put(Key(key), matcher);
        }
        w.varint(i32::try_from(state.len()).ok()?);
        let mut cursor = ordered.cursor();
        while let Some((key, matcher)) = ordered.next_entry(&mut cursor).ok()? {
            w.string(&key.0.to_string().ok()?);
            match matcher {
                Tag::String(exact) => {
                    w.bool(true);
                    w.string(&exact.to_string().ok()?);
                }
                Tag::Compound(range) => {
                    if range
                        .keys()
                        .any(|k| !matches!(k.to_string().ok().as_deref(), Some("min" | "max")))
                    {
                        return None;
                    }
                    w.bool(false);
                    for bound in ["min", "max"] {
                        w.bool(matcher.get(bound).is_some());
                        if let Some(v) = matcher.get(bound) {
                            let Tag::String(v) = v else {
                                return None;
                            };
                            w.string(&v.to_string().ok()?);
                        }
                    }
                }
                _ => return None,
            }
        }
    } else if value.get("state").is_some() {
        return None;
    }
    w.bool(value.get("nbt").is_some());
    if let Some(nbt) = value.get("nbt") {
        let parsed;
        let nbt = if let Tag::String(text) = nbt {
            let text = text.to_string().ok()?;
            let (tag, end) = sunrise_core::nbt::parse_snbt(&text).ok()?;
            if !text[end..].trim().is_empty() {
                return None;
            }
            parsed = tag;
            &parsed
        } else {
            nbt
        };
        if !matches!(nbt, Tag::Compound(_)) {
            return None;
        }
        w.bytes(&sunrise_core::nbt::write_network(nbt, Default::default()).ok()?);
    }
    write_matchers(value, ids, w)
}

/// `DataComponentMatchers.STREAM_CODEC`: bounded exact-name and partial custom-data forms.
fn write_matchers(
    value: &sunrise_core::nbt::Tag,
    ids: &dyn Fn(&str, &str) -> Option<i32>,
    w: &mut ::sunrise_core::vanilla_bytes::Writer,
) -> Option<()> {
    use sunrise_core::nbt::Tag;
    for (field, expected, registry) in [
        (
            "components",
            "minecraft:custom_name",
            "minecraft:data_component_type",
        ),
        (
            "predicates",
            "minecraft:custom_data",
            "minecraft:data_component_predicate_type",
        ),
    ] {
        let Some(matchers) = value.get(field) else {
            w.varint(0);
            continue;
        };
        let Tag::Compound(map) = matchers else {
            return None;
        };
        w.varint(i32::try_from(map.len()).ok()?);
        for (name, matcher) in map {
            let name = sunrise_world::datafix::dynamic::namespaced(&name.to_string().ok()?);
            if name != expected {
                return None;
            }
            let id = ids(registry, &name)?;
            if id < 0 {
                return None;
            }
            if field == "components" {
                let Tag::String(_) = matcher else {
                    return None;
                };
                w.varint(id);
                w.bytes(&sunrise_core::nbt::write_network(matcher, Default::default()).ok()?);
            } else {
                let parsed;
                let compound = if let Tag::String(text) = matcher {
                    let text = text.to_string().ok()?;
                    let (tag, end) = sunrise_core::nbt::parse_snbt(&text).ok()?;
                    if !text[end..].trim().is_empty() {
                        return None;
                    }
                    parsed = tag;
                    &parsed
                } else {
                    matcher
                };
                if !matches!(compound, Tag::Compound(_)) {
                    return None;
                }
                // The official partial-matcher stream codec consumes a TAG String
                // here, unlike BlockPredicate's COMPOUND_TAG above.
                let text = crate::snbt::to_snbt(compound);
                w.bool(true);
                w.varint(id);
                w.bytes(
                    &sunrise_core::nbt::write_network(
                        &Tag::String(text.as_str().into()),
                        Default::default(),
                    )
                    .ok()?,
                );
            }
        }
    }
    Some(())
}

/// Project actual model holders; a direct definition never needs a registry id.
fn trim_component(
    trim: &crate::item_data::Trim,
    synced_id: &dyn Fn(&str, &str) -> Option<i32>,
) -> Option<sunrise_protocol::item::Component> {
    use crate::item_data::trim::Holder;
    use sunrise_protocol::item::{Component, TrimHolder, TrimMaterial, TrimPattern};
    let id = |registry: &str, name: &str| {
        if name.contains(':') {
            synced_id(registry, name)
        } else {
            synced_id(registry, &format!("minecraft:{name}"))
        }
    };
    let material = match &trim.0 {
        Holder::Named(name) => TrimHolder::Id(id("minecraft:trim_material", name)?),
        Holder::Direct(value) => TrimHolder::Direct(TrimMaterial {
            palette_id: value.palette_id.to_string(),
            description: value.description.clone(),
        }),
    };
    let pattern = match &trim.1 {
        Holder::Named(name) => TrimHolder::Id(id("minecraft:trim_pattern", name)?),
        Holder::Direct(value) => TrimHolder::Direct(TrimPattern {
            asset_id: value.asset_id.to_string(),
            description: value.description.clone(),
            decal: value.decal,
        }),
    };
    Some(match (material, pattern) {
        (TrimHolder::Id(material), TrimHolder::Id(pattern)) => {
            Component::Trim { material, pattern }
        }
        (material, pattern) => Component::TrimDirect { material, pattern },
    })
}
/// Drop prototype-equivalent entries from an encoded live stack. Nested templates
/// keep their own patches, even when an entry equals the item's prototype.
pub fn canonicalize_stack_patch(
    wire: &mut Stack,
    source: &ItemStack,
    items: &crate::items::Items,
    builtin: &sunrise_protocol::registry::Builtin,
) {
    if wire.added.is_empty() && wire.removed.is_empty() {
        return;
    }
    let keep = crate::item_data::component_patch_filter(source, items);
    let component_types = builtin.get("minecraft:data_component_type");
    let retain = |id, removed| {
        component_types
            .and_then(|types| types.name(id))
            .or_else(|| sunrise_protocol::item::custom::name(id))
            .is_none_or(|name| keep(&name.into(), removed))
    };
    wire.added.retain(|(id, _)| retain(*id, false));
    wire.removed.retain(|id| retain(*id, true));
}

pub(crate) fn network_stack_projecting(
    item_id: &dyn Fn(&str) -> i32,
    synced_id: &dyn Fn(&str, &str) -> Option<i32>,
    s: &ItemStack,
    projected: &dyn Fn(&Admitted),
) -> Stack {
    let enchantment_id = |name: &str| synced_id("minecraft:enchantment", name);
    let mut stack = Stack::new(item_id(&s.item), s.count as i32);
    if stack.item <= 0 {
        log::warn!("Cannot send item {}: no registered network id", s.item);
    }
    // Send the stack's own damage, including an explicit zero, not the prototype.
    if s.carries_component(None, "minecraft:damage") {
        stack.added.push((
            sunrise_protocol::item::component::DAMAGE,
            sunrise_protocol::item::Component::Int(s.effective_damage(None)),
        ));
    }
    let enchantments: Vec<(i32, i32)> = s
        .enchantments()
        .iter()
        .filter_map(|(e, level)| Some((enchantment_id(e)?, *level as i32)))
        .collect();
    if !enchantments.is_empty() {
        stack.added.push((
            sunrise_protocol::item::component::ENCHANTMENTS,
            sunrise_protocol::item::Component::Enchantments(enchantments),
        ));
    }
    let stored: Vec<(i32, i32)> = s
        .stored_enchantments()
        .iter()
        .filter_map(|(e, level)| Some((enchantment_id(e)?, *level as i32)))
        .collect();
    if !stored.is_empty() {
        stack.added.push((
            sunrise_protocol::item::component::STORED_ENCHANTMENTS,
            sunrise_protocol::item::Component::Enchantments(stored),
        ));
    }
    if let Some(d) = s.data() {
        let glint = "minecraft:enchantment_glint_override";
        if d.other("!minecraft:enchantment_glint_override").is_some() {
            stack
                .removed
                .push(sunrise_protocol::item::component::ENCHANTMENT_GLINT_OVERRIDE);
        } else if let Some(value) = d.other(glint).and_then(crate::item_data::component_bool) {
            stack.added.push((
                sunrise_protocol::item::component::ENCHANTMENT_GLINT_OVERRIDE,
                sunrise_protocol::item::Component::Bool(value),
            ));
        }
        // TridentItem.use runs on both peers: each must see the stack's own durability limit.
        if let Some(sunrise_core::nbt::Tag::Int(max)) = d.other("minecraft:max_damage") {
            stack.added.push((
                sunrise_protocol::item::component::MAX_DAMAGE,
                sunrise_protocol::item::Component::Int(*max),
            ));
        }
        // Unbreakable is a unit component; clients need its presence for the
        // durability bar, tooltip and item model, including in single player.
        if d.other("minecraft:unbreakable").is_some() {
            stack.added.push((
                sunrise_protocol::item::component::UNBREAKABLE,
                sunrise_protocol::item::Component::Unit,
            ));
        }
        if let Some(component) = d
            .other("minecraft:kinetic_weapon")
            .and_then(|value| crate::item_sync::kinetic::encode(value, synced_id))
        {
            stack
                .added
                .push((sunrise_protocol::item::component::KINETIC_WEAPON, component));
        }
        // These built-ins use their own stream codecs, not the custom NBT route.
        for (name, kind) in [
            (
                "minecraft:provides_trim_material",
                sunrise_protocol::item::component::PROVIDES_TRIM_MATERIAL,
            ),
            (
                "minecraft:provides_banner_patterns",
                sunrise_protocol::item::component::PROVIDES_BANNER_PATTERNS,
            ),
            (
                "minecraft:can_place_on",
                sunrise_protocol::item::component::CAN_PLACE_ON,
            ),
            (
                "minecraft:can_break",
                sunrise_protocol::item::component::CAN_BREAK,
            ),
            // The client predicts mining with the stack's own tool and shows
            // the stack's own attribute modifiers (#14641).
            ("minecraft:tool", sunrise_protocol::item::component::TOOL),
            (
                "minecraft:attribute_modifiers",
                sunrise_protocol::item::component::ATTRIBUTE_MODIFIERS,
            ),
        ] {
            if d.other(&format!("!{name}")).is_some() {
                stack.removed.push(kind);
            } else if let Some(value) = d.other(name)
                && let Some(component) = builtin_component(name, value, synced_id)
            {
                stack.added.push((kind, component));
            }
        }
        if let Some(rgb) = d.dyed_color {
            stack.added.push((
                sunrise_protocol::item::component::DYED_COLOR,
                sunrise_protocol::item::Component::Int(rgb),
            ));
        }
        if let Some(trim) = &d.trim
            && let Some(component) = trim_component(trim, synced_id)
        {
            stack
                .added
                .push((sunrise_protocol::item::component::TRIM, component));
        }
        // A pattern without a synced registry id leaves out the component.
        let layers: Option<Vec<(i32, i32)>> = d
            .banner_layers()
            .iter()
            .map(|layer| {
                let (pattern, color) = (&*layer.pattern, &*layer.color);
                let name = if pattern.contains(':') {
                    pattern.to_string()
                } else {
                    format!("minecraft:{pattern}")
                };
                Some((
                    synced_id("minecraft:banner_pattern", &name)?,
                    dye_id(color)?,
                ))
            })
            .collect();
        if let Some(layers) = layers.filter(|_| d.sets_component("minecraft:banner_patterns")) {
            stack.added.push((
                sunrise_protocol::item::component::BANNER_PATTERNS,
                sunrise_protocol::item::Component::BannerPatterns(layers),
            ));
        }
        if let Some(display @ sunrise_core::nbt::Tag::Compound(_)) =
            d.saved_other("minecraft:tooltip_display")
        {
            let mut seen = std::collections::HashSet::new();
            let hidden = match display.get("hidden_components") {
                Some(sunrise_core::nbt::Tag::List(entries)) => entries
                    .iter()
                    .filter_map(|tag| {
                        let name = text(Some(tag))?;
                        let name = sunrise_core::identifier::Identifier::parse(&name).ok()?;
                        synced_id("minecraft:data_component_type", &name.to_string())
                    })
                    .filter(|id| seen.insert(*id))
                    .collect(),
                _ => Vec::new(),
            };
            stack.added.push((
                sunrise_protocol::item::component::TOOLTIP_DISPLAY,
                sunrise_protocol::item::Component::TooltipDisplay {
                    hide: display
                        .get("hide_tooltip")
                        .and_then(crate::item_data::component_bool)
                        .unwrap_or(false),
                    hidden,
                },
            ));
        }
        // `painting/variant`: a reference into the synced registry.
        if let Some(id) = text(d.other("minecraft:painting/variant")).and_then(|v| {
            synced_id(
                "minecraft:painting_variant",
                &sunrise_world::datafix::dynamic::namespaced(&v),
            )
        }) {
            stack.added.push((
                sunrise_protocol::item::component::PAINTING_VARIANT,
                sunrise_protocol::item::Component::PaintingVariant(id),
            ));
        }
        // `sign_text_front` and `sign_text_back`: a copied sign's text, which the
        // client's own placement puts on the sign it predicts (#8102).
        for (name, kind) in [
            (
                "minecraft:sign_text_front",
                sunrise_protocol::item::component::SIGN_TEXT_FRONT,
            ),
            (
                "minecraft:sign_text_back",
                sunrise_protocol::item::component::SIGN_TEXT_BACK,
            ),
        ] {
            if let Some(bytes) = d.other(name).and_then(sign_text_bytes) {
                stack
                    .added
                    .push((kind, sunrise_protocol::item::Component::Raw(bytes)));
            }
        }
        // A shield's `base_color`.
        if let Some(id) = d.base_color.as_deref().and_then(dye_id) {
            stack.added.push((
                sunrise_protocol::item::component::BASE_COLOR,
                sunrise_protocol::item::Component::Int(id),
            ));
        }
        if let Some(id) = d.map_id {
            stack.added.push((
                sunrise_protocol::item::component::MAP_ID,
                sunrise_protocol::item::Component::Int(id),
            ));
        }
        if d.repair_cost > 0 {
            stack.added.push((
                sunrise_protocol::item::component::REPAIR_COST,
                sunrise_protocol::item::Component::Int(d.repair_cost as i32),
            ));
        }
        if let Some(mode) = d.map_post_processing {
            stack.added.push((
                sunrise_protocol::item::component::MAP_POST_PROCESSING,
                sunrise_protocol::item::Component::Int(i32::from(mode)),
            ));
        }
        if let Some(amplifier) = d.ominous_amplifier {
            stack.added.push((
                sunrise_protocol::item::component::OMINOUS_BOTTLE_AMPLIFIER,
                sunrise_protocol::item::Component::Int(i32::from(amplifier)),
            ));
        }
        stack.added.extend(firework_components(d));
        // The compass's lodestone target.
        if let Some(t) = d.other("minecraft:lodestone_tracker") {
            let target = t.get("target").and_then(|g| {
                let Some(sunrise_core::nbt::Tag::IntArray(pos)) = g.get("pos") else {
                    return None;
                };
                let dimension = text(g.get("dimension"))?;
                Some((dimension, [*pos.first()?, *pos.get(1)?, *pos.get(2)?]))
            });
            let tracked = number(t.get("tracked")).is_none_or(|v| v != 0);
            stack.added.push((
                sunrise_protocol::item::component::LODESTONE_TRACKER,
                sunrise_protocol::item::Component::LodestoneTracker { target, tracked },
            ));
        }
        // SuspiciousStewEffects: each effect by its `minecraft:mob_effect` id.
        if !d.stew_effects.is_empty() {
            let effects = d
                .stew_effects
                .iter()
                .filter_map(|(effect, duration)| {
                    let name = sunrise_world::datafix::dynamic::namespaced(effect);
                    Some((synced_id("minecraft:mob_effect", &name)?, *duration))
                })
                .collect();
            stack.added.push((
                sunrise_protocol::item::component::SUSPICIOUS_STEW_EFFECTS,
                sunrise_protocol::item::Component::StewEffects(effects),
            ));
        }
        // A loaded crossbow's ammunition.
        if !d.charged.is_empty() {
            let charged = d
                .charged
                .iter()
                .map(|c| network_stack_projecting(item_id, synced_id, c, projected))
                .collect();
            stack.added.push((
                sunrise_protocol::item::component::CHARGED_PROJECTILES,
                sunrise_protocol::item::Component::Stacks(charged),
            ));
        }
        // `ItemContainerContents`: a shulker box's items, for its tooltip
        // (ClientContainerTooltip). The wire form is the slot count, then one
        // optional template per slot, so the guest's stack lists the same items
        // the host's holds.
        if !d.container.is_empty() {
            let slots: Vec<(i32, Stack)> = d
                .container
                .iter()
                .map(|(slot, s)| {
                    (
                        i32::from(*slot),
                        network_stack_projecting(item_id, synced_id, s, projected),
                    )
                })
                .collect();
            // The saved list names only the filled slots; the codec counts them
            // all, from the first to the last that holds one.
            let size = slots.iter().map(|(slot, _)| slot + 1).max().unwrap_or(0);
            let mut w = ::sunrise_core::vanilla_bytes::Writer::new();
            w.varint(size);
            for slot in 0..size {
                let stack = slots.iter().find(|(at, _)| *at == slot).map(|(_, s)| s);
                w.bool(stack.is_some());
                if let Some(stack) = stack {
                    <sunrise_protocol::item::Template as sunrise_protocol::buf::Codec<Stack>>::write(
                        stack,
                        &mut w,
                    );
                }
            }
            stack.added.push((
                sunrise_protocol::item::component::CONTAINER,
                sunrise_protocol::item::Component::Raw(w.data),
            ));
        }
        // What a bundle holds, for its tooltip (ClientBundleTooltip).
        if !d.bundle_contents.is_empty() {
            let contents = d
                .bundle_contents
                .iter()
                .map(|c| network_stack_projecting(item_id, synced_id, c, projected))
                .collect();
            stack.added.push((
                sunrise_protocol::item::component::BUNDLE_CONTENTS,
                sunrise_protocol::item::Component::Stacks(contents),
            ));
        }
        // The books a lectern shows and a guest reads.
        if !d.writable_pages.is_empty() {
            stack.added.push((
                sunrise_protocol::item::component::WRITABLE_BOOK_CONTENT,
                sunrise_protocol::item::Component::WritableBook {
                    pages: d.writable_pages.clone(),
                    filtered: d.writable_filtered.clone(),
                },
            ));
        }
        if let Some(w) = &d.written {
            stack.added.push((
                sunrise_protocol::item::component::WRITTEN_BOOK_CONTENT,
                sunrise_protocol::item::Component::WrittenBook(
                    sunrise_protocol::item::WrittenBook {
                        title: w.title.clone(),
                        author: w.author.clone(),
                        generation: i32::from(w.generation),
                        pages: if w.page_tags.len() == w.pages.len() {
                            w.page_tags.clone()
                        } else {
                            w.pages
                                .iter()
                                .map(|p| sunrise_protocol::text::literal(p))
                                .collect()
                        },
                        resolved: true,
                        ..Default::default()
                    },
                ),
            ));
        }
    }
    // Keep the component's styles across integrated-server and LAN inventory sync.
    if let Some(name) = s.data().and_then(|d| d.other("minecraft:custom_name")) {
        stack.added.push((
            sunrise_protocol::item::component::CUSTOM_NAME,
            sunrise_protocol::item::Component::Text(name.clone()),
        ));
    }
    // Lore lines as loaded or as a server mod wrote them (a list of text
    // components), so a server menu's items can say more than their name.
    if let Some(lore_tag) = s.data().and_then(|d| d.other("minecraft:lore")) {
        let lines = match lore_tag {
            sunrise_core::nbt::Tag::List(lines) => Some(lines.clone()),
            // A string is one literal line, as ComponentSerialization decodes a string tag.
            sunrise_core::nbt::Tag::Compound(_) | sunrise_core::nbt::Tag::String(_) => {
                Some(vec![lore_tag.clone()])
            }
            _ => None,
        };
        if let Some(lines) = lines {
            stack.added.push((
                sunrise_protocol::item::component::LORE,
                sunrise_protocol::item::Component::Lore(lines),
            ));
        }
    }
    if s.data()
        .is_some_and(|d| d.other("!minecraft:lore").is_some())
    {
        stack.removed.push(sunrise_protocol::item::component::LORE);
    }
    // The stack's custom rarity ("common", "uncommon", "rare", "epic" or 0..=3).
    if let Some(r) = s.data().and_then(|d| d.saved_other("minecraft:rarity")) {
        let index = match r {
            sunrise_core::nbt::Tag::String(s) => match &*s.to_string_lossy() {
                "common" => Some(0),
                "uncommon" => Some(1),
                "rare" => Some(2),
                "epic" => Some(3),
                _ => None,
            },
            sunrise_core::nbt::Tag::Byte(b) if (0..=3).contains(b) => Some(*b as i32),
            sunrise_core::nbt::Tag::Short(s) if (0..=3).contains(s) => Some(*s as i32),
            sunrise_core::nbt::Tag::Int(i) if (0..=3).contains(i) => Some(*i),
            _ => None,
        };
        if let Some(index) = index {
            stack.added.push((
                sunrise_protocol::item::component::RARITY,
                sunrise_protocol::item::Component::Int(index),
            ));
        }
    }
    if s.data()
        .is_some_and(|d| d.other("!minecraft:rarity").is_some())
    {
        stack
            .removed
            .push(sunrise_protocol::item::component::RARITY);
    }
    // `DataComponents.MAX_STACK_SIZE`: an int in 1..=99 on this stack, not the
    // item's default. A removed component falls back to that default.
    if s.data()
        .is_some_and(|d| d.other("!minecraft:max_stack_size").is_some())
    {
        stack
            .removed
            .push(sunrise_protocol::item::component::MAX_STACK_SIZE);
    } else if let Some(sunrise_core::nbt::Tag::Int(n)) =
        s.data().and_then(|d| d.other("minecraft:max_stack_size"))
        && (1..=99).contains(n)
    {
        stack.added.push((
            sunrise_protocol::item::component::MAX_STACK_SIZE,
            sunrise_protocol::item::Component::Int(*n),
        ));
    }
    // The component as loaded, so a translatable name stays one.
    if let Some(name) = s.data().and_then(|d| d.saved_other("minecraft:item_name")) {
        stack.added.push((
            sunrise_protocol::item::component::ITEM_NAME,
            sunrise_protocol::item::Component::Text(name.clone()),
        ));
    }
    // The model a pack or mod picks for this stack, which the client's item model reads.
    if let Some(model) = s.data().and_then(|d| text(d.other("minecraft:item_model"))) {
        stack.added.push((
            sunrise_protocol::item::component::ITEM_MODEL,
            sunrise_protocol::item::Component::Ident(model),
        ));
    }
    // Arbitrary NBT other mods and item models read back client-side
    // (`item_model.rs`'s `custom_data_at`), as vanilla's DataComponentPatch does.
    if let Some(tag @ sunrise_core::nbt::Tag::Compound(_)) =
        s.data().and_then(|d| d.other("minecraft:custom_data"))
    {
        stack.added.push((
            sunrise_protocol::item::component::CUSTOM_DATA,
            sunrise_protocol::item::Component::Nbt(tag.clone()),
        ));
    }
    if let Some(t) = s
        .data()
        .and_then(|d| d.other("minecraft:custom_model_data"))
    {
        stack.added.push((
            sunrise_protocol::item::component::CUSTOM_MODEL_DATA,
            custom_model_data(t),
        ));
    }
    if let Some(d) = s.data() {
        use sunrise_protocol::item::persistent;

        let ids = |registry: &str, name: &str| {
            if registry == "minecraft:item" {
                let id = item_id(name);
                (id > 0).then_some(id)
            } else {
                synced_id(registry, name)
            }
        };
        let mut encode_saved = |name: &str, value: &sunrise_core::nbt::Tag| {
            let key = name.strip_prefix('!').unwrap_or(name);
            let kind = persistent::component_id(key);
            if key != name {
                // The peer registry can name a removal even when we have no value codec.
                let peer_kind = synced_id("minecraft:data_component_type", key)
                    .or_else(|| kind.filter(|&id| id < persistent::BUILTIN_COMPONENT_COUNT as i32));
                if let Some(id) = peer_kind
                    && !stack.removed.contains(&id)
                {
                    stack.removed.push(id);
                }
                return;
            }
            let Some(kind) = kind else { return };
            if kind >= persistent::BUILTIN_COMPONENT_COUNT as i32
                && synced_id("minecraft:data_component_type", key) != Some(kind)
            {
                // The peer did not negotiate this native component.
                return;
            }
            let encoded = if crate::item_data::StackData::CONSUME_EFFECT_COMPONENTS.contains(&key) {
                persistent::component_from_admitted_tag(
                    kind,
                    value,
                    &ids,
                    &|tag| d.find_admitted_consume_effect(&|effect| effect.matches_tag(tag)),
                    projected,
                )
            } else {
                persistent::component_from_tag(kind, value, &ids)
            };
            match encoded {
                Ok(component) => stack.added.push((kind, component)),
                Err(error) => {
                    log::warn!("Cannot send component {name} on {}: {error}", s.item);
                }
            }
        };
        // Consumable is modeled separately from the preserved fields, but its
        // saved value is available by reference alongside those overrides.
        if let Some(value) = d.other("minecraft:consumable") {
            encode_saved("minecraft:consumable", value);
        }
        for (name, value) in d.preserved_components() {
            // These fields have specialized encoders above; only the other
            // saved overrides go through the general protocol codec.
            if !matches!(
                name.as_str(),
                "minecraft:enchantment_glint_override"
                    | "minecraft:max_damage"
                    | "minecraft:unbreakable"
                    | "minecraft:kinetic_weapon"
                    | "minecraft:provides_trim_material"
                    | "minecraft:provides_banner_patterns"
                    | "minecraft:can_place_on"
                    | "minecraft:can_break"
                    | "minecraft:tool"
                    | "minecraft:attribute_modifiers"
                    | "minecraft:tooltip_display"
                    | "minecraft:painting/variant"
                    | "minecraft:sign_text_front"
                    | "minecraft:sign_text_back"
                    | "minecraft:lodestone_tracker"
                    | "minecraft:custom_name"
                    | "minecraft:lore"
                    | "minecraft:rarity"
                    | "minecraft:max_stack_size"
                    | "minecraft:item_name"
                    | "minecraft:item_model"
                    | "minecraft:custom_data"
                    | "minecraft:custom_model_data"
                    | "minecraft:consumable"
                    | "minecraft:potion_contents"
                    | "minecraft:pot_decorations"
            ) {
                encode_saved(name, value);
            }
        }
        if let Some(sunrise_core::nbt::Tag::Compound(entries)) =
            crate::item_data::encode_extra_components_matching(s, |name| {
                matches!(
                    name,
                    "minecraft:potion_contents" | "minecraft:pot_decorations"
                )
            })
        {
            for (name, value) in &entries {
                let name = name.to_string_lossy();
                if !name.starts_with('!') {
                    encode_saved(&name, value);
                }
            }
        }
    }
    stack
}

/// `CustomModelData.CODEC` read from its saved form, for its stream codec.
fn custom_model_data(t: &sunrise_core::nbt::Tag) -> sunrise_protocol::item::Component {
    use sunrise_core::nbt::Tag;
    // NbtOps writes a list of ints or bytes as an int or byte array.
    let entries = |key: &str| -> Vec<Tag> {
        match t.get(key) {
            Some(Tag::List(l)) => l.clone(),
            Some(Tag::IntArray(a)) => a.iter().map(|v| Tag::Int(*v)).collect(),
            Some(Tag::ByteArray(a)) => a.iter().map(|v| Tag::Byte(*v as i8)).collect(),
            _ => Vec::new(),
        }
    };
    // `ARGB.as8BitChannel`.
    let channel = |v: f64| ((v as f32) * 255.0).floor() as i32 & 255;
    sunrise_protocol::item::Component::CustomModelData {
        floats: entries("floats")
            .iter()
            .filter_map(|v| float(Some(v)))
            .map(|v| v as f32)
            .collect(),
        flags: entries("flags")
            .iter()
            .filter_map(|v| number(Some(v)))
            .map(|v| v != 0)
            .collect(),
        strings: entries("strings")
            .iter()
            .filter_map(|v| text(Some(v)))
            .collect(),
        // `ExtraCodecs.RGB_COLOR_CODEC`: an int, or an opaque colour from three floats.
        colors: entries("colors")
            .iter()
            .filter_map(|v| match v {
                Tag::List(rgb) if rgb.len() == 3 => {
                    let c = |i: usize| float(rgb.get(i)).map(channel);
                    Some(0xFF << 24 | c(0)? << 16 | c(1)? << 8 | c(2)?)
                }
                v => number(Some(v)).map(|v| v as i32),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_core::nbt::Tag;
    use sunrise_core::vanilla_bytes::Writer;
    use sunrise_protocol::buf::{Codec, Reader};
    use sunrise_protocol::item::{Component, component};

    #[test]
    fn builtin_predicate_streams_keep_typed_conjunctions_and_reject_whole_unknown_forms() {
        let ids = |registry: &str, name: &str| match (registry, name) {
            ("minecraft:block", "minecraft:chest") => Some(245),
            ("minecraft:data_component_type", "minecraft:custom_name") => Some(6),
            ("minecraft:data_component_predicate_type", "minecraft:custom_data") => Some(4),
            _ => None,
        };
        // Original value boundaries independently consumed by the pinned Java codecs.
        for (input, expected) in [
            (
                r#"{blocks:"minecraft:chest",nbt:"{Lock:{}}"}"#,
                "010102f50100010a0a00044c6f636b00000000",
            ),
            (
                r#"{blocks:"minecraft:chest",components:{"minecraft:custom_name":"Box"}}"#,
                "010102f50100000106080003426f7800",
            ),
            (
                r#"{blocks:"minecraft:chest",predicates:{"minecraft:custom_data":{a:1}}}"#,
                "010102f5010000000101040800057b613a317d",
            ),
        ] {
            let tag = sunrise_core::nbt::parse_snbt(input).unwrap().0;
            let encoded = builtin_component("minecraft:can_break", &tag, &ids).unwrap();
            let Component::Raw(bytes) = encoded else {
                panic!("fresh encoding")
            };
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(hex, expected, "{input}");
            let mut stack = sunrise_protocol::item::Stack::new(1, 1);
            stack
                .added
                .push((component::CAN_BREAK, Component::Raw(bytes)));
            let mut w = Writer::new();
            sunrise_protocol::item::Slot::write(&Some(stack), &mut w);
            let mut r = Reader::new(&w.data);
            let decoded = sunrise_protocol::item::Slot::read(&mut r).unwrap().unwrap();
            assert!(decoded.component(component::CAN_BREAK).is_some());
            assert_eq!(r.remaining(), 0);
        }
        for input in [
            r#"{blocks:"minecraft:chest",unknown:1}"#,
            r#"{blocks:"minecraft:chest",state:{age:{min:"3",unknown:"7"}}}"#,
            r#"{blocks:"minecraft:chest",components:{"minecraft:custom_name":"Box","minecraft:damage":1}}"#,
            r#"{blocks:"minecraft:chest",predicates:{"minecraft:custom_data":{a:1},"minecraft:damage":{}}}"#,
            r#"[{blocks:"minecraft:chest"},{blocks:"minecraft:missing"}]"#,
            r#"{blocks:"minecraft:chest",nbt:"{a:1} trailing"}"#,
        ] {
            let tag = sunrise_core::nbt::parse_snbt(input).unwrap().0;
            assert!(
                builtin_component("minecraft:can_break", &tag, &ids).is_none(),
                "{input}"
            );
        }
    }

    /// A `tool` or `attribute_modifiers` the writers cannot encode is left out
    /// of the packet, whole, as `can_break`'s is.
    #[test]
    fn a_tool_or_modifier_the_writers_cannot_encode_is_dropped_whole() {
        let ids = |registry: &str, name: &str| match (registry, name) {
            ("minecraft:block", "minecraft:stone") => Some(1),
            ("minecraft:attribute", "minecraft:attack_damage") => Some(3),
            _ => None,
        };
        let parse = |snbt: &str| sunrise_core::nbt::parse_snbt(snbt).unwrap().0;
        let entry = |extra: &str| {
            format!(r#"[{{type:"attack_damage",id:"x",amount:2.0d,operation:"add_value"{extra}}}]"#)
        };
        // The forms the cases below spoil all encode.
        for (name, snbt) in [
            (
                "minecraft:tool",
                r#"{rules:[{blocks:"minecraft:stone"}]}"#.to_owned(),
            ),
            ("minecraft:attribute_modifiers", entry("")),
            (
                "minecraft:attribute_modifiers",
                entry(r#",display:{type:"override",value:"x"}"#),
            ),
        ] {
            let encoded = builtin_component(name, &parse(&snbt), &ids);
            assert!(encoded.is_some(), "{name} {snbt}");
        }
        for (name, snbt) in [
            // No rules; a rule without blocks, or with a block the registry lacks.
            ("minecraft:tool", "{}".to_owned()),
            ("minecraft:tool", "{rules:[{speed:2.0f}]}".to_owned()),
            (
                "minecraft:tool",
                r#"{rules:[{blocks:"minecraft:missing"}]}"#.to_owned(),
            ),
            // A speed that is no number; a negative wear (`NON_NEGATIVE_INT`).
            (
                "minecraft:tool",
                r#"{rules:[{blocks:"minecraft:stone",speed:"fast"}]}"#.to_owned(),
            ),
            (
                "minecraft:tool",
                "{rules:[],damage_per_block:-1}".to_owned(),
            ),
            // Not a list; an unknown attribute, operation, slot group or display type.
            ("minecraft:attribute_modifiers", "{}".to_owned()),
            (
                "minecraft:attribute_modifiers",
                entry("").replace("attack_damage", "missing"),
            ),
            (
                "minecraft:attribute_modifiers",
                entry("").replace("add_value", "multiply"),
            ),
            ("minecraft:attribute_modifiers", entry(r#",slot:"tail""#)),
            (
                "minecraft:attribute_modifiers",
                entry(r#",display:{type:"loud"}"#),
            ),
            // An entry without its modifier id; an override without its text.
            (
                "minecraft:attribute_modifiers",
                entry("").replace(r#"id:"x","#, ""),
            ),
            (
                "minecraft:attribute_modifiers",
                entry(r#",display:{type:"override"}"#),
            ),
        ] {
            let encoded = builtin_component(name, &parse(&snbt), &ids);
            assert!(encoded.is_none(), "{name} {snbt}");
        }
        // Through the stack: the bad component is neither added nor removed.
        let mut stack = ItemStack::new("minecraft:iron_pickaxe", 1);
        stack
            .data_mut()
            .set_other("minecraft:tool", Some(parse("{}")));
        let sent = stack_to_network(|_| 1, ids, &Some(stack)).unwrap();
        assert!(sent.component(component::TOOL).is_none());
        assert!(sent.removed.is_empty());
    }

    #[test]
    fn builtin_removal_markers_survive_actual_saved_stack_adoption() {
        for (name, kind) in [
            (
                "minecraft:provides_trim_material",
                component::PROVIDES_TRIM_MATERIAL,
            ),
            (
                "minecraft:provides_banner_patterns",
                component::PROVIDES_BANNER_PATTERNS,
            ),
            ("minecraft:can_place_on", component::CAN_PLACE_ON),
            ("minecraft:can_break", component::CAN_BREAK),
        ] {
            let input = Tag::compound([
                ("id", Tag::String("minecraft:stone".into())),
                ("count", Tag::Int(1)),
                (
                    "components",
                    Tag::Compound(
                        [(
                            format!("!{name}").as_str().into(),
                            Tag::Compound(Default::default()),
                        )]
                        .into(),
                    ),
                ),
            ]);
            let actual = sunrise_items::item_nbt::item_from_nbt(&input).unwrap();
            assert!(actual.data().unwrap().other(&format!("!{name}")).is_some());
            let sent = stack_to_network(|_| 1, |_, _| None, &Some(actual)).unwrap();
            assert_eq!(sent.removed, [kind]);
            assert!(sent.added.is_empty());
        }
    }

    #[test]
    fn built_in_and_unmodelled_removals_round_trip_through_the_peer_registry() {
        let mut item = ItemStack::new("minecraft:stone", 1);
        for id in [
            "minecraft:unbreakable",
            "minecraft:item_model",
            "example:unmodelled",
        ] {
            item.data_mut().set_other(id, None);
            item.data_mut()
                .set_other(&format!("!{id}"), Some(Tag::Compound(Default::default())));
        }
        let sent = stack_to_network(
            |_| 1,
            |registry, name| {
                assert_eq!(registry, "minecraft:data_component_type");
                match name {
                    "minecraft:unbreakable" => Some(4),
                    "minecraft:item_model" => Some(10),
                    "example:unmodelled" => Some(300),
                    _ => None,
                }
            },
            &Some(item),
        );
        let mut writer = Writer::new();
        sunrise_protocol::item::Slot::write(&sent, &mut writer);
        let mut reader = Reader::new(&writer.data);
        let mut received = sunrise_protocol::item::Slot::read(&mut reader)
            .unwrap()
            .unwrap();
        received.removed.sort_unstable();
        assert_eq!(received.removed, [4, 10, 300]);
        assert!(received.added.is_empty());
    }

    #[test]
    fn lore_lines_reach_the_client() {
        let lines = vec![
            Tag::compound([("text", Tag::String("Buy 400".into()))]),
            Tag::compound([
                ("text", Tag::String("▲ 2.0%".into())),
                ("color", Tag::String("green".into())),
            ]),
        ];
        let mut item = ItemStack::new("minecraft:diamond", 1);
        item.data_mut()
            .set_other("minecraft:lore", Some(Tag::List(lines.clone())));
        let sent = stack_to_network(|_| 7, |_, _| None, &Some(item));
        let mut writer = Writer::new();
        sunrise_protocol::item::Slot::write(&sent, &mut writer);
        let mut reader = Reader::new(&writer.data);
        let received = sunrise_protocol::item::Slot::read(&mut reader)
            .unwrap()
            .unwrap();
        assert_eq!(
            received.component(component::LORE),
            Some(&Component::Lore(lines))
        );
    }

    #[test]
    fn rarity_reaches_the_client() {
        let mut item = ItemStack::new("minecraft:stick", 1);
        item.data_mut()
            .set_other("minecraft:rarity", Some(Tag::String("epic".into())));
        let sent = stack_to_network(|_| 7, |_, _| None, &Some(item));
        let mut writer = Writer::new();
        sunrise_protocol::item::Slot::write(&sent, &mut writer);
        let mut reader = Reader::new(&writer.data);
        let received = sunrise_protocol::item::Slot::read(&mut reader)
            .unwrap()
            .unwrap();
        assert_eq!(
            received.component(component::RARITY),
            Some(&Component::Int(3))
        );
    }

    #[test]
    fn styled_custom_name_survives_inventory_sync() {
        let name = Tag::compound([
            ("text", Tag::String("Red".into())),
            ("color", Tag::String("#ff0000".into())),
            ("italic", Tag::Byte(0)),
        ]);
        let mut item = ItemStack::new("minecraft:stick", 1);
        item.data_mut().custom_name = Some(name.clone());
        let sent = stack_to_network(|_| 7, |_, _| None, &Some(item));
        let mut writer = Writer::new();
        sunrise_protocol::item::Slot::write(&sent, &mut writer);
        let mut reader = Reader::new(&writer.data);
        let received = sunrise_protocol::item::Slot::read(&mut reader)
            .unwrap()
            .unwrap();
        assert_eq!(reader.remaining(), 0);
        assert_eq!(
            received.component(component::CUSTOM_NAME),
            Some(&Component::Text(name))
        );
    }

    #[test]
    fn potions_go_as_potion_registry_ids() {
        let synced_id = |registry: &str, name: &str| match (registry, name) {
            ("minecraft:potion", "minecraft:swiftness") => Some(14),
            _ => None,
        };
        let potion = ItemStack::new("minecraft:splash_potion", 1).with_potion("swiftness");
        let stack = stack_to_network(|_| 7, synced_id, &Some(potion)).unwrap();
        assert_eq!(
            stack.component(component::POTION_CONTENTS),
            Some(&Component::Potion {
                potion: Some(14),
                color: None,
                rest: Vec::new(),
            })
        );
    }

    #[test]
    fn trims_go_as_synced_registry_references() {
        let mut chestplate = ItemStack::new("minecraft:diamond_chestplate", 1);
        chestplate.data_mut().trim = Some(("minecraft:gold".into(), "coast".into()));
        let synced_id = |registry: &str, name: &str| match (registry, name) {
            ("minecraft:trim_material", "minecraft:gold") => Some(4),
            ("minecraft:trim_pattern", "minecraft:coast") => Some(1),
            _ => None,
        };
        let stack = stack_to_network(|_| 7, synced_id, &Some(chestplate.clone())).unwrap();
        assert_eq!(
            stack.component(component::TRIM),
            Some(&Component::Trim {
                material: 4,
                pattern: 1
            })
        );
        // A name the connection's registry lacks sends no trim.
        chestplate.data_mut().trim = Some(("minecraft:x".into(), "coast".into()));
        let stack = stack_to_network(|_| 7, synced_id, &Some(chestplate)).unwrap();
        assert_eq!(stack.component(component::TRIM), None);
    }

    /// A pack's pattern and material go to a client by the ids of the server's sync order.
    #[test]
    fn a_packs_trim_goes_by_its_synced_ids() {
        let mut chestplate = ItemStack::new("minecraft:iron_chestplate", 1);
        chestplate.data_mut().trim = Some(("test:jade".into(), "test:zigzag".into()));
        let synced_id = |registry: &str, name: &str| match (registry, name) {
            ("minecraft:trim_material", "test:jade") => Some(11),
            ("minecraft:trim_pattern", "test:zigzag") => Some(20),
            _ => None,
        };
        let stack = stack_to_network(|_| 7, synced_id, &Some(chestplate)).unwrap();
        assert_eq!(
            stack.component(component::TRIM),
            Some(&Component::Trim {
                material: 11,
                pattern: 20
            })
        );
    }

    #[test]
    fn goat_horns_send_their_instrument_by_registry_id() {
        let mut horn = ItemStack::new("minecraft:goat_horn", 1);
        horn.data_mut().set_other(
            "minecraft:instrument",
            Some(sunrise_core::nbt::Tag::String(
                "minecraft:sing_goat_horn".into(),
            )),
        );
        let synced_id = |registry: &str, name: &str| match (registry, name) {
            ("minecraft:instrument", "minecraft:sing_goat_horn") => Some(1),
            _ => None,
        };
        let stack = stack_to_network(|_| 7, synced_id, &Some(horn)).unwrap();
        assert_eq!(
            stack.component(component::INSTRUMENT),
            Some(&Component::Instrument(1))
        );
    }
}
