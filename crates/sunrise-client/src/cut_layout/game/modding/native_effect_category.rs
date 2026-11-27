#![cfg(test)]
#[cfg(test)]
mod tests {
    use mod_api::EffectCategory;
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;

    #[test]
    #[ignore = "daily"]
    fn registered_effect_categories_use_type_definitions() {
        with_game(|game| {
            for (id, expected) in [
                ("minecraft:speed", EffectCategory::Beneficial),
                ("minecraft:poison", EffectCategory::Harmful),
                ("minecraft:glowing", EffectCategory::Neutral),
            ] {
                let key = mod_platform::__private::part_tests::effect_key(id);
                assert_eq!(key.category(game), Some(expected), "{id}");
                let active = mod_api::ActiveEffect::new(key, 200, 1, false, true, true);
                assert_eq!(active.id.category(game), Some(expected));
            }
        });
    }
}
