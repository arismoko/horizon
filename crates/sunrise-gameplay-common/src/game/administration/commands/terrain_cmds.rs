#[cfg(any(test, feature = "test-support"))]
pub mod loot_mine_component_tests {

    use sunrise_items::items::ItemStack;

    use sunrise_core::nbt::Tag;

    use sunrise_sim::save;

    pub fn decorations(stack: &ItemStack) -> Option<Tag> {
        save::item_to_nbt(stack, &Default::default())
            .get("components")?
            .get("minecraft:pot_decorations")
            .cloned()
    }
}
