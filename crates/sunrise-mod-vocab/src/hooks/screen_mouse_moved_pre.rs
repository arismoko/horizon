//! The `screen_mouse_moved_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The pointer moved over the open screen (vanilla's `Screen.mouseMoved`, called by
    /// `MouseHandler.handleAccumulatedMovement` for every movement, whether or not a
    /// button is held, and before `Screen.mouseDragged`). NeoForge's `ScreenEvent` has no
    /// event for it, so this hook has no NeoForge name.
    /// `screen.mouse` is the new GUI position and `delta` its movement in GUI pixels, as
    /// `screen_mouse_dragged_pre` gives them. It fires first: with a button held,
    /// `screen_mouse_dragged_pre` follows for the same move.
    /// Cancel keeps the new cursor position and skips the rest of this move: the drag hook
    /// and the screen's drag handlers do not run (Sunrise's screens do no other work per
    /// move; hover is read from the cursor when the screen draws).
    /// Same receiving screens as `screen_mouse_dragged_pre`, including dialogs,
    /// confirmation prompts, books, lecterns and sign editors. A move with no change in
    /// position, a controller's or the keyboard focus's pointer, and a screen that holds
    /// the pointer grab (`ScreensClientHost::set_pointer_grab`: its motion is `screen_pointer_motion`)
    /// fire nothing. With no listener nothing is built.
    #[notes = r#"## Screen hover moves

`screen_mouse_moved_pre(screen, delta, modifiers, canceled)` is vanilla's `Screen.mouseMoved`: it fires for every real nonzero pointer movement over the open screen, whether or not a button is held, before `screen_mouse_dragged_pre` of the same move. screen.mouse is the new GUI position; delta is movement in GUI pixels. Cancel keeps the new cursor position and skips the rest of the move (the drag hook and drag handlers). A controller's pointer and a screen holding the pointer grab fire nothing; with no listener nothing is built."#]
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_screen_mouse_moved_pre",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        version = 1,
        contract = "cancel",
        out = "canceled",
        cancellable = true
    )]
    fn screen_mouse_moved_pre(
        screen: &ScreenView<'_>,
        delta: [f32; 2],
        modifiers: u32,
        canceled: &mut bool,
    );
}
