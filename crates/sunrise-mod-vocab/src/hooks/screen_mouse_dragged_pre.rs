//! The `screen_mouse_dragged_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before a real held-button movement reaches screen drag handlers.
    /// `screen.mouse` is the current GUI position; `delta` is its GUI-space movement.
    /// Cancel keeps the new cursor position but skips all screen drag handlers.
    /// Same receiving screens as `screen_mouse_button_pressed_pre`, including
    /// dialogs, confirmation prompts, books, lecterns and sign editors.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_screen_mouse_dragged_pre",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        version = 2,
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.MouseDragged.Pre",
        cancellable = true
    )]
    fn screen_mouse_dragged_pre(
        screen: &ScreenView<'_>,
        button: i32,
        delta: [f32; 2],
        modifiers: u32,
        canceled: &mut bool,
    );
}
