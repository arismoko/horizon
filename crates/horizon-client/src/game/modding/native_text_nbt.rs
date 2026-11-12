//! `text_to_nbt`: a text as item components and block entities store it.
use super::Game;
use horizon_client::{lan::data::component_nbt, text::Text};
use horizon_core::nbt::Tag;
use mod_api::TextFormError;

impl Game<'_> {
    /// The text as item components store it.
    pub(super) fn text_to_nbt(&self, text: &Text) -> Result<Tag, TextFormError> {
        Ok(component_nbt(&text.component()))
    }
}

#[cfg(test)]
mod tests {
    use super::{Text, component_nbt};

    /// The NBT reads back as the same component, as an item's name does.
    #[test]
    fn text_to_nbt_is_the_components_nbt_form() {
        let text = Text::legacy("\u{a7}6Golden \u{a7}lsword");
        let tag = component_nbt(&text.component());
        let back = horizon_client::component::name_text(&tag);
        assert_eq!(back.plain(), "Golden sword");
        let styles: Vec<_> = back.chars().map(|(_, style)| style.clone()).collect();
        assert_eq!(styles[0].color, Some(0xFFAA00), "gold");
        assert!(!styles[0].is_bold());
        assert!(styles[7].is_bold(), "the second word is bold");
    }
}
