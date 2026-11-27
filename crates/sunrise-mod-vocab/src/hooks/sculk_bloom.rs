//! The `sculk_bloom` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A sculk catalyst at `pos` has begun blooming after taking experience.
    /// The catalyst block state has changed. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_sculk_bloom",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn sculk_bloom(dimension: &str, pos: [i32; 3]);
}
