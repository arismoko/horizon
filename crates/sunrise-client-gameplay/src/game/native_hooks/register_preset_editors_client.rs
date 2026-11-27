use super::*;

impl crate::game::ClientGame<'_> {
    pub fn preset_editor_dimensions(&self) -> Option<mod_api::Compound> {
        let sunrise_core::nbt::Tag::Compound(dimensions) =
            self.preset_editor_title()?.preset_editor_dimensions()?
        else {
            return None;
        };
        Some(dimensions)
    }
    pub fn set_preset_editor_dimensions(&self, dimensions: mod_api::Compound) -> bool {
        self.preset_editor_title().is_some_and(|title| {
            title.set_preset_editor_dimensions(sunrise_core::nbt::Tag::Compound(dimensions))
        })
    }
    pub fn preset_editor_title(&self) -> Option<&sunrise_client::title::Title> {
        if self.client.screen != Screen::Playing
            || self.screen_over_playing()
            || self.remote_loading()
        {
            return None;
        }
        self.client
            .title
            .0
            .as_ref()
            .filter(|title| title.preset_editor_session().is_some())
    }
}
