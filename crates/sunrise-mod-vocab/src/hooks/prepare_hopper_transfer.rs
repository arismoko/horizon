//! The `prepare_hopper_transfer` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A hopper at `pos` began an item transfer attempt. This runs after its
    /// cooldown and enabled checks, before pushing or pulling any items.
    /// Server-side, including the single-player server.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_prepare_hopper_transfer",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no hopper event)"
    )]
    fn prepare_hopper_transfer(pos: [i32; 3]);
}
