//! The `movement_input_update` hook declaration (see `hooks/mod.rs`).

pub trait Hooks {
    /// Lends the local player's sampled [`MovementInput`] after sampling, before auto-jump
    /// and sprint decisions. Every listener sees the preceding listeners' edits.
    ///
    /// Change `event.input.move_vector` for local movement or `event.input.key_presses` for
    /// key handling and the flags sent to the server. Neither edit rewrites the other.
    /// Auto-jump can set the final jump flag. The sprint flag is a key press, not a switch
    /// for sprinting: vanilla eligibility, the sprint latch and double-tap still apply.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::movement_input::ClientGame<'_>::native_movement_input_update",
        receiver = "read",
        dispatch = "api_carrier",
        side = "client",
        contract = "listen",
        neoforge = "MovementInputUpdateEvent",
        cancellable = false,
        event_per_listener = true
    )]
    fn movement_input_update(input: &mut MovementInput);
}
