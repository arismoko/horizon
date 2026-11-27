//! The `screen_key_pressed_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A key went down over the open screen (`ScreenEvent.KeyPressed.Pre`,
    /// vanilla's `Screen.keyPressed`): `screen` is the screen it is, `key`
    /// vanilla's name for the key (`key.keyboard.e`) — a key with no name
    /// fires nothing — `repeat` whether it is
    /// the keyboard's auto-repeat of a held press (Sunrise's extra; NeoForge
    /// folds repeats into the event), and `modifiers` GLFW's bits (shift 1,
    /// control 2, alt 4, super 8). Set `canceled` and the press goes no
    /// further: no focus move, no Escape-close, no inventory or hotbar key, no
    /// typing by the key itself.
    ///
    /// Fires for named window keys (including repeats) and the control socket's
    /// `key ... down`, `key ... repeat`, and the press phase of `key ... tap`,
    /// after window shortcuts such as fullscreen, screenshot and debug keys.
    /// The receiving screen can be a title screen or a dialog, confirmation
    /// prompt, book, lectern or sign editor in front of another screen. It does
    /// not fire with no screen, on the connection or world-loading screen
    /// itself, or during a closing save. A canceled press does not cancel its
    /// release or its separate characters: see `screen_key_released_pre` and
    /// `screen_char_typed_pre`.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_screen_key_pressed_pre",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        version = 2,
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.KeyPressed.Pre",
        cancellable = true
    )]
    fn screen_key_pressed_pre(
        screen: &ScreenView<'_>,
        key: &str,
        repeat: bool,
        modifiers: u32,
        canceled: &mut bool,
    );
}
