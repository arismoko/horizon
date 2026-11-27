//! The `screen_mouse_scrolled_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The wheel turned over the open screen (`ScreenEvent.MouseScrolled.Pre`,
    /// vanilla's `Screen.mouseScrolled`): `screen` is the screen it is,
    /// `screen.mouse` where the pointer is (so there is no position of its
    /// own), `scroll` the wheel's movement in x and y as the game's own scroll
    /// handler scales it (a line times the wheel sensitivity: with Discrete
    /// Scrolling on, a notch is ±1, its sign only; a touchpad's pixels are
    /// divided by 30), and
    /// `modifiers` GLFW's bits (shift 1, control 2, alt 4, super 8). Set
    /// `canceled` and nothing the screen would scroll moves: not the bundle
    /// under the pointer, nor a list, slider, scrollbar or map. Nothing else
    /// reads the wheel while a screen of the game's is open, so no hotbar slot
    /// changes either.
    ///
    /// Fires before the receiving screen scrolls, including the title's screens
    /// and a dialog, confirmation prompt, book, lectern or sign editor in front
    /// of another screen. With no screen, on the connection or world-loading
    /// screen itself, or during a closing save, it does not fire. A window wheel
    /// event (including the control socket's `scroll-at`) fires it; a gamepad or
    /// keyboard focus move does not.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_screen_mouse_scrolled_pre",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        version = 2,
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.MouseScrolled.Pre",
        cancellable = true
    )]
    fn screen_mouse_scrolled_pre(
        screen: &ScreenView<'_>,
        scroll: [f64; 2],
        modifiers: u32,
        canceled: &mut bool,
    );
}
