//! The `screen_mouse_button_pressed_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A press about to go to `Screen.mouseClicked` (`ScreenEvent.MouseButtonPressed.Pre`).
    /// `screen` is the receiving screen and `screen.mouse` is the pointer position.
    /// `button` is the GLFW index (0 left, 1 right, 2 middle, 3 back, 4 forward; beyond those,
    /// the platform's own number). `modifiers` are GLFW bits (shift 1, control 2, alt 4, super 8).
    /// The receiver and press Post get the same `double_click`: true for a real press when
    /// the last handled real press was of this button on this screen instance less than 250 ms
    /// ago. Cancel to skip the screen handler, Post and gameplay mappings; a cancelled real
    /// press still counts as handled for double-click detection.
    ///
    /// Fires before the receiving screen acts, including the title's screens and
    /// a dialog, confirmation prompt, book, lectern or sign editor in front of
    /// another screen. It does not fire with no screen, on the connection or
    /// world-loading screen itself, or during a closing save. Only window mouse
    /// input (including a control-socket click sent through the window), not a
    /// gamepad's or keyboard focus's synthesized click.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_screen_mouse_button_pressed_pre",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.MouseButtonPressed.Pre",
        cancellable = true,
        version = 3
    )]
    fn screen_mouse_button_pressed_pre(
        screen: &ScreenView<'_>,
        button: i32,
        modifiers: u32,
        double_click: bool,
        canceled: &mut bool,
    );
}
