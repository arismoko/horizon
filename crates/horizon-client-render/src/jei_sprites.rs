//! The official Recipes viewer's artwork from Just Enough Items, under MIT (`jei_recipes/LICENSE.txt`,
//! THIRD_PARTY_NOTICES.md): `HudAtlas` loads each as the sprite `horizon/jei/<name>`.

/// Each sprite's name and its PNG.
pub(crate) const SPRITES: &[(&str, &[u8])] = &[
    (
        "button_enabled_v2",
        include_bytes!("jei_recipes/button_enabled_v2.png"),
    ),
    (
        "button_disabled_v2",
        include_bytes!("jei_recipes/button_disabled_v2.png"),
    ),
    (
        "button_highlight_v2",
        include_bytes!("jei_recipes/button_highlight_v2.png"),
    ),
    (
        "icons/arrow_previous_v2",
        include_bytes!("jei_recipes/icons/arrow_previous_v2.png"),
    ),
    (
        "icons/arrow_next_v2",
        include_bytes!("jei_recipes/icons/arrow_next_v2.png"),
    ),
    (
        "search_background_v2",
        include_bytes!("jei_recipes/search_background_v2.png"),
    ),
    ("slot", include_bytes!("jei_recipes/slot.png")),
    ("icons/flame", include_bytes!("jei_recipes/icons/flame.png")),
    ("output_slot", include_bytes!("jei_recipes/output_slot.png")),
    (
        "recipe_arrow",
        include_bytes!("jei_recipes/recipe_arrow.png"),
    ),
    (
        "recipe_arrow_filled",
        include_bytes!("jei_recipes/recipe_arrow_filled.png"),
    ),
    (
        "single_recipe_background_v2",
        include_bytes!("jei_recipes/single_recipe_background_v2.png"),
    ),
    (
        "icons/list_badge",
        include_bytes!("jei_recipes/icons/list_badge.png"),
    ),
    (
        "interactive_ingredient_tooltip_background",
        include_bytes!("jei_recipes/interactive_ingredient_tooltip_background.png"),
    ),
    (
        "recipe_preview_background_v2",
        include_bytes!("jei_recipes/recipe_preview_background_v2.png"),
    ),
    (
        "ingredient_list_slot_background_v2",
        include_bytes!("jei_recipes/ingredient_list_slot_background_v2.png"),
    ),
    (
        "scrollbar_background_v2",
        include_bytes!("jei_recipes/scrollbar_background_v2.png"),
    ),
    (
        "scrollbar_marker_v2",
        include_bytes!("jei_recipes/scrollbar_marker_v2.png"),
    ),
];
