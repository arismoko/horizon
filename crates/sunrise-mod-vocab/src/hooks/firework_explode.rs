//! The `firework_explode` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A firework rocket completed its explosion effects.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_firework_explode",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn firework_explode(entity: u32, pos: [f64; 3], bursts: usize);
}
