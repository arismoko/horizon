#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {
    #[cfg(feature = "client")]
    use mod_api::ModEntry;
    #[cfg(feature = "client")]
    use mod_api::{
        ClientHost, Hooks, RenderFramePostEvent, RenderFramePreEvent, RenderLevelStageEvent,
    };
    #[cfg(feature = "client")]
    use std::sync::{Arc, Mutex};
    #[cfg(feature = "client")]
    use std::time::Duration;
    #[cfg(feature = "client")]
    use sunrise_client::title::Title;
    #[cfg(feature = "client")]
    use sunrise_client_gameplay::game::native_hooks::particle_count_tests::frame_scene;
    #[cfg(feature = "client")]
    use sunrise_sim::clock::FixedStep;
    #[cfg(feature = "client")]
    use {
        crate::game::server_side::native_entity_refs::tests::with_client_game,
        sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    };

    #[cfg(feature = "client")]
    #[derive(Default)]
    struct Heard {
        pub pre: Vec<(f32, f32)>,
        post: Vec<(f32, f32)>,
        stages: Vec<f32>,
    }
    #[cfg(feature = "client")]
    struct Frames(Arc<Mutex<Heard>>);
    #[cfg(feature = "client")]
    #[mod_api::__subscriptions]
    impl Hooks for Frames {
        fn render_frame_pre(&self, _: &mut dyn ClientHost, event: &mut RenderFramePreEvent) {
            self.0
                .lock()
                .unwrap()
                .pre
                .push((event.world_partial_tick, event.partial_tick));
            // Listener edits must not become the renderer's or post event's time.
            event.world_partial_tick = -1.0;
            event.partial_tick = -1.0;
        }
        fn render_frame_post(&self, _: &mut dyn ClientHost, event: &mut RenderFramePostEvent) {
            self.0
                .lock()
                .unwrap()
                .post
                .push((event.world_partial_tick, event.partial_tick));
        }
        fn render_level_stage(&self, _: &dyn ClientHost, event: &mut RenderLevelStageEvent<'_>) {
            self.0.lock().unwrap().stages.push(event.view.partial_tick);
        }
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn frame_interpolation_matches_draw_and_holds_while_paused_or_frozen() {
        let name = "cut_layout::game::native_hooks::render_frame_pre::tests::frame_interpolation_matches_draw_and_holds_while_paused_or_frozen";
        if run_fresh_cases(name, "SUNRISE_FRAME_PARTIAL_CHILD", &["frames"]) {
            return;
        }
        let heard = Arc::new(Mutex::new(Heard::default()));
        let mods = vec![ModEntry::new("frames", Box::new(Frames(heard.clone())))];
        mod_platform::testing::with_mods(mods.into(), || {
            with_client_game(|game| {
                let _event_loop = frame_scene(game);
                game.clock = FixedStep::client();
                game.clock.set_frozen(false);
                game.clock.end_frame(false);
                let elapsed = Duration::from_secs_f64(1.0 / 60.0);
                let frame = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
                             world: bool| {
                    game.clock.advance(elapsed, 50.0);
                    game.extras_frame(elapsed);
                    // A clock change after pre cannot change the captured draw/post value.
                    game.clock.advance(Duration::from_millis(1), 50.0);
                    game.render_and_flip(elapsed).unwrap();
                    let mut h = heard.lock().unwrap();
                    let pre = *h.pre.last().unwrap();
                    assert_eq!(h.post.last(), Some(&pre));
                    if world {
                        assert!(!h.stages.is_empty());
                        assert!(h.stages.iter().all(|&partial| partial == pre.0));
                    } else {
                        assert!(h.stages.is_empty());
                        assert_eq!(pre.0, pre.1);
                    }
                    h.stages.clear();
                    pre
                };
                let running = frame(game, true);
                assert_eq!(running, (0.32, 0.32));
                let held = game.clock.fraction() as f32;
                game.clock.end_frame(true);
                let paused = frame(game, true);
                let paused_next = frame(game, true);
                assert_eq!(paused.0, held);
                assert_eq!(paused_next.0, held);
                assert_ne!(paused.1, paused_next.1);
                game.clock.set_frozen(true);
                assert!(!game.clock.world_runs());
                game.clock.end_frame(false);
                let frozen = frame(game, true);
                let frozen_next = frame(game, true);
                assert_eq!(frozen.0, 1.0);
                assert_eq!(frozen_next.0, 1.0);
                assert_ne!(frozen.1, frozen_next.1);
                let saves = tempfile::tempdir().unwrap();
                let title = Title::new(saves.path().to_path_buf(), &game.options.resources, "");
                *game.test_title_mut() = Some(title);
                frame(game, false);
            });
        });
    }
}
