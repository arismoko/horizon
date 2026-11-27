//! The `anvil_damaged` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An anvil at `pos` took damage, changing its block from `from` to `to`.
    /// `to` is air when the anvil broke. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_anvil_damaged",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn anvil_damaged(dimension: &str, pos: [i32; 3], from: &str, to: &str);
}
