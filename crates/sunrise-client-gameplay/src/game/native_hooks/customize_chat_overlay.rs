/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// The in-game chat is about to be drawn (`CustomizeGuiOverlayEvent.Chat`): `x`, `y` are its
    /// bottom-left corner in GUI pixels, which a mod may move.
    pub(in crate::game) fn native_customize_chat_overlay(
        &mut self,
        gui_width: i32,
        gui_height: i32,
        focused: bool,
        x: &mut i32,
        y: &mut i32,
    ) {
        self.fire_customize_chat_overlay(gui_width, gui_height, focused, x, y);
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod customize_chat_overlay_tests {
    use mod_api::{ClientHost, CustomizeChatOverlayEvent, Hooks};
    use std::sync::{Arc, Mutex};
    use sunrise_client::hud::HudBatch;

    pub type Seen = Arc<Mutex<Vec<(i32, i32, bool, i32, i32)>>>;

    /// Lifts the chat 100 GUI pixels, logging what it was told.
    pub struct Lift(pub Seen);

    #[mod_api::__subscriptions]
    impl Hooks for Lift {
        fn customize_chat_overlay(
            &self,
            _: &mut dyn ClientHost,
            event: &mut CustomizeChatOverlayEvent,
        ) {
            let e = &*event;
            self.0
                .lock()
                .unwrap()
                .push((e.gui_width, e.gui_height, e.focused, e.x, e.y));
            event.y -= 100;
        }
    }

    /// The top of what the chat draws, in GUI pixels, and the batch's GUI scale.
    pub fn chat_top(game: &mut crate::game::ClientGame<'_>) -> (Option<f32>, f32) {
        let mut batch = HudBatch::scratch([1280, 720]);
        game.draw_chat(&mut batch);
        let top = batch.vertices.iter().map(|v| v.pos[1]).reduce(f32::min);
        (top, batch.scale)
    }
}
