#[cfg(any(test, feature = "test-support"))]
pub mod peaceful_tests {
    use sunrise_core::nbt::Tag;
    use sunrise_items::items::ItemStack;

    pub const TRADER: &str = "peaceful_test:trader";

    pub fn trader_egg() -> ItemStack {
        let mut stack = ItemStack::new("minecraft:cow_spawn_egg", 1);
        stack.data_mut().set_other(
            "minecraft:entity_data",
            Some(Tag::compound([("id", Tag::String(TRADER.into()))])),
        );
        stack
    }
}

pub mod terrain_cmds;
