//! The `mouse_button_post` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An accepted raw button event finished dispatch, including a screen handler's early return.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_mouse_button_post",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "InputEvent.MouseButton.Post"
    )]
    fn mouse_button_post(button: i32, pressed: bool, modifiers: u32);
}
