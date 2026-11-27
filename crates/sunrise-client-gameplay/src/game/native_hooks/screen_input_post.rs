use mod_api::{ScreenView, TriState};

/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// A mouse press after its receiving screen ran (`ScreenEvent.MouseButtonPressed.Post`).
    pub(in crate::game) fn native_screen_mouse_button_pressed_post(
        &self,
        screen: &ScreenView<'_>,
        button: i32,
        modifiers: u32,
        double_click: bool,
        was_click_handled: bool,
        result: &mut TriState,
    ) {
        self.fire_screen_mouse_button_pressed_post(
            screen,
            button,
            modifiers,
            double_click,
            was_click_handled,
            result,
        );
    }
    /// A mouse release after its receiving screen ran (`ScreenEvent.MouseButtonReleased.Post`).
    pub(in crate::game) fn native_screen_mouse_button_released_post(
        &self,
        screen: &ScreenView<'_>,
        button: i32,
        modifiers: u32,
        was_release_handled: bool,
        result: &mut TriState,
    ) {
        self.fire_screen_mouse_button_released_post(
            screen,
            button,
            modifiers,
            was_release_handled,
            result,
        );
    }
    /// A drag the receiving screen did not handle (`ScreenEvent.MouseDragged.Post`).
    pub(in crate::game) fn native_screen_mouse_dragged_post(
        &self,
        screen: &ScreenView<'_>,
        button: i32,
        delta: [f32; 2],
        modifiers: u32,
    ) {
        self.fire_screen_mouse_dragged_post(screen, button, delta, modifiers);
    }
    /// A wheel turn the receiving screen did not handle (`ScreenEvent.MouseScrolled.Post`).
    pub(in crate::game) fn native_screen_mouse_scrolled_post(
        &self,
        screen: &ScreenView<'_>,
        scroll: [f64; 2],
        modifiers: u32,
    ) {
        self.fire_screen_mouse_scrolled_post(screen, scroll, modifiers);
    }
    /// A key press no screen took (`ScreenEvent.KeyPressed.Post`).
    pub(in crate::game) fn native_screen_key_pressed_post(
        &self,
        screen: &ScreenView<'_>,
        key: &str,
        repeat: bool,
        modifiers: u32,
        canceled: &mut bool,
    ) {
        self.fire_screen_key_pressed_post(screen, key, repeat, modifiers, canceled);
    }
    /// A key release no screen took (`ScreenEvent.KeyReleased.Post`).
    pub(in crate::game) fn native_screen_key_released_post(
        &self,
        screen: &ScreenView<'_>,
        key: &str,
        modifiers: u32,
        canceled: &mut bool,
    ) {
        self.fire_screen_key_released_post(screen, key, modifiers, canceled);
    }
    /// A character no text box took (`ScreenEvent.CharacterTyped.Post`).
    pub(in crate::game) fn native_screen_char_typed_post(
        &self,
        screen: &ScreenView<'_>,
        c: char,
        modifiers: u32,
    ) {
        self.fire_screen_char_typed_post(screen, c, modifiers);
    }
}
