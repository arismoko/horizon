#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    use sunrise_items::items::ItemStack;

    use mod_api::{AttributeModifier, AttributeOperation, Identifier};

    pub fn stack(item: &str) -> Option<ItemStack> {
        Some(ItemStack::new(item, 1))
    }
    pub fn modifier(id: &str, amount: f64, operation: AttributeOperation) -> AttributeModifier {
        AttributeModifier::new(Identifier::parse(id).unwrap(), amount, operation)
    }
    /// The ids of the modifiers a saved `attributes` list holds.
    pub fn saved_ids<'a>(
        list: impl IntoIterator<Item = &'a sunrise_core::nbt::Tag>,
    ) -> Vec<String> {
        use sunrise_core::nbt::Tag;
        use sunrise_game_data::nbt_value::text;
        list.into_iter()
            .flat_map(|entry| match entry.get("modifiers") {
                Some(Tag::List(l)) => l.iter().filter_map(|m| text(m.get("id"))).collect(),
                _ => Vec::new(),
            })
            .collect()
    }
}
