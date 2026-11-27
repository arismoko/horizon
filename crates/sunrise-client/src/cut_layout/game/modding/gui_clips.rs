#![cfg(test)]
/// The generated dispatch lends the full context: a nested scope cannot leave a clip active
/// after it returns or unwinds, either in its caller or in another listener.
#[cfg(test)]
mod dispatch_tests {
    use mod_api::{ComponentHoverEvent, Flow, GuiClipRefusal, Hooks, ModEntry};
    use mod_platform::testing::with_mods;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use sunrise_client::hud::HudBatch;
    use sunrise_game_data::text::Text;

    struct Clipper;
    #[mod_api::__subscriptions]
    impl Hooks for Clipper {
        fn component_hover(
            &self,
            _: &dyn mod_api::ClientHost,
            event: &mut ComponentHoverEvent<'_>,
        ) -> Flow {
            let square = [0.0, 0.0, 100.0, 100.0];
            event
                .draw
                .with_scissor([10.0, 10.0, 10.0, 10.0], &mut |outer| {
                    assert_eq!(
                        outer.with_scissor([0.0, f32::NAN, 5.0, 5.0], &mut |_| {
                            panic!("a refused rectangle must not invoke the callback")
                        }),
                        Err(GuiClipRefusal::NotFinite)
                    );
                    outer
                        .with_scissor([12.0, 12.0, 5.0, 5.0], &mut |inner| {
                            inner.fill(square, 0xFF00_FF00);
                        })
                        .unwrap();
                    let unwind = catch_unwind(AssertUnwindSafe(|| {
                        outer.with_scissor([15.0, 15.0, 10.0, 10.0], &mut |inner| {
                            inner.fill(square, 0xFF00_00FF);
                            panic!("leave the nested scope by unwinding")
                        })
                    }));
                    assert!(unwind.is_err());
                    outer.fill(square, 0xFFFF_0000);
                })
                .unwrap();
            event.draw.fill([8.0, 8.0, 1.0, 1.0], 0xFFFF_FF00);
            Flow::Continue
        }
    }

    struct Unclipped;
    #[mod_api::__subscriptions]
    impl Hooks for Unclipped {
        fn component_hover(
            &self,
            _: &dyn mod_api::ClientHost,
            event: &mut ComponentHoverEvent<'_>,
        ) -> Flow {
            event.draw.fill([24.0, 24.0, 4.0, 4.0], u32::MAX);
            Flow::Cancel
        }
    }

    #[test]
    #[ignore = "daily"]
    fn a_hook_call_clips_scoped_draws_and_restores_on_unwind() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::client_side::gui_clips::dispatch_tests::a_hook_call_clips_scoped_draws_and_restores_on_unwind",
            || {
                let text = Text::from(serde_json::json!({
                    "text": "x", "hover_event": { "action": "show_text", "value": "hi" }
                }));
                let mods: mod_api::Mods = vec![
                    ModEntry::new("clipper", Box::new(Clipper)),
                    ModEntry::new("unclipped", Box::new(Unclipped)),
                ]
                .into();
                let mut batch = HudBatch::scratch([320, 240]);
                with_mods(mods, || {
                    crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
                        game.component_hover_effect(&mut batch, &text.spans[0].style, (40.0, 30.0));
                    });
                });
                assert_eq!(batch.vertices.len(), 30);
                for (quad, color) in batch.vertices.as_chunks::<6>().0.iter().zip([
                    0xFF00_FF00,
                    0xFFFF_0000,
                    0xFF00_00FF,
                    0xFF00_FFFF,
                    u32::MAX,
                ]) {
                    assert_eq!(quad.each_ref().map(|v| v.color), [color; 6]);
                }
                assert_eq!(
                    batch.scissors_in(0..30).collect::<Vec<_>>(),
                    [
                        (0..6, Some([12.0, 12.0, 17.0, 17.0])),
                        (6..12, Some([15.0, 15.0, 20.0, 20.0])),
                        (12..18, Some([10.0, 10.0, 20.0, 20.0])),
                        (18..30, None),
                    ]
                );
            },
        );
    }
}
