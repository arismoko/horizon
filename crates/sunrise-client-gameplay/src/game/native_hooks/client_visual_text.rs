use crate::game::ClientGame;

use crate::game::host::draw::text_layout::LabelLayout;
use mod_api::{
    Text, TextLayoutError, TextLayoutSize, TextLayoutSpec, TextLine, VisualTextCharacterRef,
};
use sunrise_client_lib::api_text::to_local_engine;

impl ClientGame<'_> {
    pub(in crate::game) fn visit_text_visual_order(
        &self,
        text: &Text,
        visit: &mut dyn FnMut(VisualTextCharacterRef<'_>) -> bool,
    ) -> bool {
        let local = to_local_engine(text);
        let resolved = local.engine().resolved();
        let visual = sunrise_client::bidi::visual_characters(&resolved, |a, b| {
            local.visual_styles_match(a, b)
        });
        // Keep deep style payloads in Text so they drop iteratively.
        let mut component = Text::literal("");
        let mut previous = None;
        for (scalar, effective, source_index) in visual {
            // Distinct effective styles may carry different opaque metadata.
            if !previous.is_some_and(|old| std::ptr::eq(old, effective)) {
                component = Text::literal("").with_style(local.style_to_api(effective));
                previous = Some(effective);
            }
            if !visit(VisualTextCharacterRef::new(
                scalar as u32,
                &component.style,
                source_index,
            )) {
                return false;
            }
        }
        true
    }

    pub(in crate::game) fn visit_text_lines(
        &self,
        text: &Text,
        spec: TextLayoutSpec,
        visit: &mut dyn FnMut(TextLine<'_>),
    ) -> Result<TextLayoutSize, TextLayoutError> {
        LabelLayout::try_from(spec)?.visit_components(&self.client.gui.atlas.font, text, visit)
    }
}
