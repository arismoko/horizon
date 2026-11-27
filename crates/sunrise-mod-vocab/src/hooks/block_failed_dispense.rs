//! The `block_failed_dispense` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A dispenser at `pos` failed to dispense `item`. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_block_failed_dispense",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn block_failed_dispense(dimension: &str, pos: [i32; 3], item: &str);
}
