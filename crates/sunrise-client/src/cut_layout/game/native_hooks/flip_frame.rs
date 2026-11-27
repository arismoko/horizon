#![cfg(test)]
#[cfg(test)]
mod tests {
    #[cfg(feature = "client")]
    use mod_api::ModEntry;
    #[cfg(feature = "client")]
    use mod_api::{ClientHost, FlipFrameEvent, Hooks, RenderFramePostEvent, RenderFramePreEvent};
    #[cfg(feature = "client")]
    use std::sync::{Arc, Mutex};
    #[cfg(feature = "client")]
    use std::time::Duration;
    #[cfg(feature = "client")]
    use sunrise_client_gameplay::game::native_hooks::particle_count_tests::frame_scene;
    #[cfg(feature = "client")]
    use {
        crate::game::server_side::native_entity_refs::tests::with_client_game,
        sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    };

    #[cfg(feature = "client")]
    struct Moments(Arc<Mutex<Vec<&'static str>>>);
    #[cfg(feature = "client")]
    #[mod_api::__subscriptions]
    impl Hooks for Moments {
        fn render_frame_pre(&self, _: &mut dyn ClientHost, _: &mut RenderFramePreEvent) {
            self.0.lock().unwrap().push("pre");
        }
        fn render_frame_post(&self, _: &mut dyn ClientHost, _: &mut RenderFramePostEvent) {
            self.0.lock().unwrap().push("post");
        }
        fn flip_frame(&self, _: &mut dyn ClientHost, _: &mut FlipFrameEvent) {
            self.0.lock().unwrap().push("flip");
        }
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn flip_frame_follows_render_frame_post_on_presented_frames_only() {
        let name = "cut_layout::game::native_hooks::flip_frame::tests::flip_frame_follows_render_frame_post_on_presented_frames_only";
        if run_fresh_cases(name, "SUNRISE_FLIP_FRAME_CHILD", &["frames"]) {
            return;
        }
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new("moments", Box::new(Moments(heard.clone())))];
        mod_platform::testing::with_mods(mods.into(), || {
            with_client_game(|game| {
                let _event_loop = frame_scene(game);
                let elapsed = Duration::from_millis(16);
                let frame = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
                             shown,
                             moments: &[&str]| {
                    game.extras_frame(elapsed);
                    assert_eq!(game.render_and_flip(elapsed).unwrap(), shown);
                    assert_eq!(*heard.lock().unwrap(), moments);
                    heard.lock().unwrap().clear();
                };

                frame(game, true, &["pre", "post", "flip"]);

                let saves = tempfile::tempdir().unwrap();
                let title = game.new_title(saves.path().to_path_buf());
                *game.test_title_mut() = Some(title);
                game.render_with(|_, frame| {
                    assert!(frame.title);
                    Ok(false)
                })
                .unwrap();
                frame(game, true, &["pre", "post", "flip"]);

                // A minimized window: the frame is prepared, never drawn.
                game.test_renderer_mut().as_mut().unwrap().suspended = true;
                frame(game, false, &["pre", "post"]);
                *game.test_title_mut() = None;
                frame(game, false, &["pre", "post"]);
            });
        });
    }
}
