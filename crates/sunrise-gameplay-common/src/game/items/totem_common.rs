use sunrise_items::items::ItemStack;

pub const TOTEM: &str = "minecraft:totem_of_undying";

/// `EntityEvent.TOTEM_OF_UNDYING`.
pub const TOTEM_EVENT: i8 = 35;

/// The `minecraft:death_protection` component of `item` (of `stack`, when
/// there is one): the stack's own patch when it sets or removes it, else the
/// item's default. It runs for both of the player's hands and for every
/// armed mob each tick, so the usual stack, which leaves the component alone,
/// costs one map lookup instead of a component map built from its patch.
pub fn death_protection_of<'m>(
    models: &'m sunrise_assets::item_model::ItemModels,
    item: &str,
    stack: Option<&ItemStack>,
) -> Option<std::borrow::Cow<'m, serde_json::Value>> {
    const KEY: &str = "minecraft:death_protection";
    if let Some(s) = stack
        && let Some(data) = s.data()
        && (data.other(KEY).is_some() || data.other("!minecraft:death_protection").is_some())
    {
        return models
            .components(&sunrise_sim::item_models::stack_view(s))
            .get(KEY)
            .cloned()
            .map(std::borrow::Cow::Owned);
    }
    models
        .prototypes
        .get(item)?
        .get(KEY)
        .map(std::borrow::Cow::Borrowed)
}
