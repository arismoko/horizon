fn borrowed_text(
    tag: &sunrise_core::nbt::Tag,
    original: impl FnOnce(&sunrise_core::nbt::Tag) -> serde_json::Value,
) -> serde_json::Value {
    original(tag)
}
