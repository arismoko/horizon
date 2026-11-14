//! The `mouse_turn_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The grabbed mouse moved `delta` (x/y window pixels) since the last frame and is about to
    /// turn the player (`MouseHandler.handleAccumulatedMovement`, before `turnPlayer`): raw,
    /// before the sensitivity, the cinematic camera's smoothing and the inverted axes. Cancel
    /// drops this frame's turn: `calculate_player_turn`, the tutorial and the player see none of
    /// it, so a mod steers something of its own with the mouse. Told once a frame while playing
    /// with the pointer grabbed in an active window, only when the mouse moved; an agent's
    /// `mouse-move` counts. A controller's look does not come through it. Client-side.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "none (vanilla MouseHandler.handleAccumulatedMovement; NeoForge has no event before turnPlayer)"
    )]
    fn mouse_turn_pre();
}
