//! The `mouse_scrolling` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Gameplay scrolling after accumulation, before hotbar/spectator handling.
    /// `scroll` is scaled x/y, `accumulated` whole x/y steps, `buttons` left/middle/right,
    /// and `cursor` the GUI position. Cancel skips gameplay scrolling.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_mouse_scrolling",
        dispatch = "flat",
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "InputEvent.MouseScrollingEvent",
        cancellable = true
    )]
    fn mouse_scrolling(
        scroll: [f64; 2],
        accumulated: [i32; 2],
        buttons: [bool; 3],
        cursor: [f32; 2],
        modifiers: u32,
        canceled: &mut bool,
    );
}
