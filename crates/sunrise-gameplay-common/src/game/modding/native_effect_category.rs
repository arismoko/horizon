//! Registered effect metadata reads.

impl crate::game::CommonGame<'_> {
    pub fn effect_category(&self, key: mod_api::EffectKey) -> Option<mod_api::EffectCategory> {
        let index = mod_api::__private::effect_index(key)?;
        let definition = sunrise_game_data::potions::effect_by_index(index)?;
        Some(match definition.category {
            sunrise_game_data::potions::Category::Beneficial => mod_api::EffectCategory::Beneficial,
            sunrise_game_data::potions::Category::Harmful => mod_api::EffectCategory::Harmful,
            sunrise_game_data::potions::Category::Neutral => mod_api::EffectCategory::Neutral,
        })
    }
}
