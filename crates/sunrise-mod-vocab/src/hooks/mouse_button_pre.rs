//! The `mouse_button_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Raw mouse button input on any screen, before dispatch; cancel skips dispatch and Post.
    /// Button/modifiers use the same GLFW encoding as screen hooks. Synthetic clicks skip it.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_mouse_button_pre",
        dispatch = "flat",
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "InputEvent.MouseButton.Pre",
        cancellable = true
    )]
    fn mouse_button_pre(button: i32, pressed: bool, modifiers: u32, canceled: &mut bool);
}
