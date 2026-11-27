//! Regressions for the typed bindings carried by the upstream sync.
use super::native_entity_refs::tests::with_client_game;
use mod_api::{ClientHost, Flow, Hooks, ModEntry};

mod background {
    use super::*;
    #[derive(Default)]
    struct Background;
    #[mod_api::sunrise_mod]
    impl Hooks for Background {
        fn screen_transparent_background(
            &self,
            host: &dyn ClientHost,
            event: &mut mod_api::ScreenTransparentBackgroundEvent<'_>,
        ) -> Flow {
            if event.screen != "inventory" {
                return Flow::Continue;
            }
            assert!(host.apply_screen_blur());
            assert!(!host.apply_screen_blur());
            event.batch.fill([3.0, 4.0, 5.0, 6.0], 0xff123456);
            Flow::Cancel
        }
    }
}
#[test]
#[ignore = "daily"]
fn cancelled_background_keeps_custom_geometry_and_one_blur_request() {
    mod_platform::testing::with_mods(
        vec![ModEntry::new("background", background::hooks())].into(),
        || {
            with_client_game(|game| {
                let mut batch = sunrise_client::hud::HudBatch::scratch([800, 600]);
                game.render_transparent_background("inventory", &mut batch);
                assert_eq!(
                    batch.vertices.len(),
                    6,
                    "custom quad only, no vanilla gradient"
                );
                assert!(game.client.screen_blur_requested.get());
                batch.vertices.clear();
                game.render_transparent_background("chest", &mut batch);
                assert_eq!(
                    batch.vertices.len(),
                    6,
                    "uncancelled screen keeps its gradient"
                );
                game.client.screen_blur_requested.set(false);
                game.render_transparent_background("inventory", &mut batch);
                assert_eq!(
                    batch.vertices.len(),
                    12,
                    "the next frame may request blur again"
                );
            })
        },
    );
}
