#![cfg(feature = "client")]
#![cfg(test)]
//! Full-context and canvas gradients through the game's generated dispatch: vanilla's colour
//! order with a clip or pose. Zero area, one-axis inversion and non-finite input draw nothing;
//! both axes inverted swaps the visual top and bottom colours.
use mod_api::client::hud_layout::PoseOp;
use mod_api::{ComponentHoverEvent, Hooks, ModEntry};
use mod_platform::testing::with_mods;
use sunrise_client::hud::HudBatch;
use sunrise_game_data::text::Text;

const TOP: u32 = 0x8000_FF00;
const BOTTOM: u32 = 0xFF00_00FF;

/// Draws gradients that draw nothing, one that does, and a painted one under a translation.
struct Gradients;
#[mod_api::__subscriptions]
impl Hooks for Gradients {
    fn component_hover(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut ComponentHoverEvent<'_>,
    ) -> mod_api::Flow {
        let draw = &mut *event.draw;
        draw.fill_gradient([10.0, 10.0, 0.0, 5.0], TOP, BOTTOM);
        draw.fill_gradient([10.0, 10.0, 5.0, -5.0], TOP, BOTTOM);
        draw.fill_gradient([f32::NAN, 10.0, 5.0, 5.0], TOP, BOTTOM);
        draw.fill_gradient([10.0, 10.0, f32::INFINITY, 5.0], TOP, BOTTOM);
        draw.with_scissor([0.0, 0.0, 100.0, 14.0], &mut |clipped| {
            clipped.fill_gradient([10.0, 10.0, 20.0, 8.0], TOP, BOTTOM);
        })
        .unwrap();
        draw.fill_gradient([130.0, 28.0, -20.0, -8.0], 0x8011_2233, 0xFF44_5566);
        draw.draw_gui(&mut |root| {
            root.scope(&[PoseOp::Translate(50.0, 60.0)], None, &mut |canvas| {
                canvas.fill_gradient([0.0, 0.0, 4.0, 2.0], TOP, BOTTOM)?;
                canvas.fill_gradient([4.0, 0.0, -4.0, 2.0], TOP, BOTTOM)
            })
        })
        .unwrap();
        // The enclosing clip must win over the inner canvas's own.
        draw.with_scissor([0.0, 104.0, 100.0, 10.0], &mut |clipped| {
            clipped
                .draw_gui(&mut |root| {
                    root.scope(&[], Some([0.0, 102.0, 100.0, 10.0]), &mut |canvas| {
                        canvas.fill_gradient([10.0, 100.0, 20.0, 10.0], 0xFF00_0000, 0xFF03_0000)
                    })
                })
                .unwrap();
        })
        .unwrap();
        mod_api::Flow::Cancel
    }
}

#[test]
#[ignore = "daily"]
fn a_mods_gradients_draw_top_to_bottom_in_the_clip_and_pose_and_empty_ones_draw_nothing() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::gui_gradient_tests::a_mods_gradients_draw_top_to_bottom_in_the_clip_and_pose_and_empty_ones_draw_nothing",
        || {
            let text = Text::from(serde_json::json!({
                "text": "x", "hover_event": { "action": "show_text", "value": "hi" }
            }));
            let mods: mod_api::Mods = vec![ModEntry::new("gradients", Box::new(Gradients))].into();
            let mut batch = HudBatch::scratch([320, 240]);
            with_mods(mods, || {
                crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
                    game.component_hover_effect(&mut batch, &text.spans[0].style, (40.0, 30.0));
                });
            });
            let drawn: Vec<_> = batch.vertices.iter().map(|v| (v.pos, v.color)).collect();
            // Each gradient stays whole with vanilla's corner colours; a clip gates fragments
            // through the quad's scissor, as `GuiRenderer.executeDraw` does.
            assert_eq!(
                drawn,
                [
                    ([10.0, 10.0], 0x8000_FF00),
                    ([30.0, 10.0], 0x8000_FF00),
                    ([30.0, 18.0], 0xFFFF_0000),
                    ([10.0, 10.0], 0x8000_FF00),
                    ([30.0, 18.0], 0xFFFF_0000),
                    ([10.0, 18.0], 0xFFFF_0000),
                    ([110.0, 20.0], 0xFF66_5544),
                    ([130.0, 20.0], 0xFF66_5544),
                    ([130.0, 28.0], 0x8033_2211),
                    ([110.0, 20.0], 0xFF66_5544),
                    ([130.0, 28.0], 0x8033_2211),
                    ([110.0, 28.0], 0x8033_2211),
                    ([50.0, 60.0], 0x8000_FF00),
                    ([54.0, 60.0], 0x8000_FF00),
                    ([54.0, 62.0], 0xFFFF_0000),
                    ([50.0, 60.0], 0x8000_FF00),
                    ([54.0, 62.0], 0xFFFF_0000),
                    ([50.0, 62.0], 0xFFFF_0000),
                    ([10.0, 100.0], 0xFF00_0000),
                    ([30.0, 100.0], 0xFF00_0000),
                    ([30.0, 110.0], 0xFF00_0003),
                    ([10.0, 100.0], 0xFF00_0000),
                    ([30.0, 110.0], 0xFF00_0003),
                    ([10.0, 110.0], 0xFF00_0003),
                ]
            );
            // The enclosing clip wins over the inner canvas's wider one.
            let visible: Vec<_> = batch.visible_quads().collect();
            assert_eq!(
                visible,
                [
                    Some([10.0, 10.0, 30.0, 14.0]),
                    Some([110.0, 20.0, 130.0, 28.0]),
                    Some([50.0, 60.0, 54.0, 62.0]),
                    Some([10.0, 104.0, 30.0, 110.0]),
                ]
            );
        },
    );
}
