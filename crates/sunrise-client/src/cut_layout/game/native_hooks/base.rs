#![cfg(test)]
use sunrise_client::hud::HudBatch;
#[cfg(test)]
#[path = "base_tests/screen_open_tests.rs"]
pub(crate) mod screen_open_tests;

#[cfg(test)]
#[path = "base_tests/screen_widgets_tests.rs"]
pub(crate) mod screen_widgets_tests;

#[cfg(test)]
pub(crate) mod text_tests {
    use crate::game::native_hooks::base::*;

    /// `DrawContext::text` takes ARGB as vanilla's `GuiGraphics` does: the alpha reaches the
    /// vertices, and fully transparent text draws nothing.
    #[test]
    #[ignore = "daily"]
    fn text_takes_argb_and_skips_transparent_text() {
        use mod_api::DrawContext;
        sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
            let drawn = |argb: u32| {
                let mut batch = HudBatch::scratch([640, 480]);
                sunrise_client_gameplay::game::host::draw::GameDrawContext::new(game, &mut batch)
                    .text("Hi", [4.0, 4.0], argb, false);
                batch.vertices
            };
            let opaque = drawn(0xFFFF_FFFF);
            let half = drawn(0x80FF_FFFF);
            assert!(!opaque.is_empty());
            assert_eq!(opaque.len(), half.len());
            assert!(opaque.iter().all(|v| v.color >> 24 == 0xFF));
            assert!(half.iter().all(|v| v.color >> 24 == 0x80));
            // RGB alone, as the API took before, is fully transparent now.
            assert!(drawn(0x00FF_FFFF).is_empty());
        });
    }
}
