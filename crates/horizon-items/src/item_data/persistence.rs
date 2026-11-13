//! Persistent stack patches; live and network component views stay separate.
use super::*;
pub(super) fn encode_stack_components(stack: &ItemStack, include_damage: bool) -> Option<Tag> {
    let mut components = components_tag_inner(stack.damage, stack.data(), include_damage)?;
    // DataComponentPatch.CODEC filters transient SET/REMOVE entries, including
    // typed nested stack patches carried by persistent text and item codecs.
    horizon_world::item_defaults::strip_transient_component_map_tag(&mut components);
    if matches!(&components, Tag::Compound(entries) if entries.is_empty()) {
        return None;
    }
    // MobEffectInstance.Details' codec always saves the resolved icon flag.
    // An omitted icon follows show_particles, whose default is true.
    if let Some(Tag::List(effects)) = components
        .get_mut("minecraft:potion_contents")
        .and_then(|potion| potion.get_mut("custom_effects"))
    {
        for effect in effects {
            let shown = effect.get("show_particles").and_then(int).unwrap_or(1) != 0;
            if let Tag::Compound(fields) = effect {
                fields
                    .entry(JavaString::from("show_icon"))
                    .or_insert(Tag::Byte(i8::from(shown)));
            }
        }
    }
    // LodestoneTracker.CODEC: optionalFieldOf("tracked", true).
    if let Some(Tag::Compound(tracker)) = components.get_mut("minecraft:lodestone_tracker")
        && tracker.get(&JavaString::from("tracked")) == Some(&Tag::Byte(1))
    {
        tracker.remove(&JavaString::from("tracked"));
    }
    if &*stack.item == "minecraft:filled_map"
        && matches!(components.get("minecraft:map_decorations"), Some(Tag::Compound(entries)) if entries.is_empty())
        && let Tag::Compound(entries) = &mut components
    {
        entries.remove(&JavaString::from("minecraft:map_decorations"));
        if entries.is_empty() {
            return None;
        }
    }
    // Items.ENCHANTED_BOOK has empty stored enchantments in its prototype.
    // Keep the live presence marker; only the saved component patch elides it.
    if &*stack.item == "minecraft:enchanted_book"
        && matches!(components.get("minecraft:stored_enchantments"), Some(Tag::Compound(entries)) if entries.is_empty())
        && let Tag::Compound(entries) = &mut components
    {
        entries.remove(&JavaString::from("minecraft:stored_enchantments"));
        if entries.is_empty() {
            return None;
        }
    }
    // Items.BEEHIVE/BEE_NEST use this whole block-state map as their prototype.
    // Empty maps and maps with extra properties are distinct, nondefault patches.
    if matches!(&*stack.item, "minecraft:beehive" | "minecraft:bee_nest")
        && matches!(components.get("minecraft:block_state"), Some(Tag::Compound(state))
            if state.len() == 1
                && matches!(state.get(&JavaString::from("honey_level")),
                    Some(Tag::String(level)) if level.to_string_lossy() == "0"))
        && let Tag::Compound(entries) = &mut components
    {
        entries.remove(&JavaString::from("minecraft:block_state"));
        if entries.is_empty() {
            return None;
        }
    }
    Some(components)
}
