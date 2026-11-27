//! The `potion_splash` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A thrown splash or lingering potion applied its hit effects.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_potion_splash",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn potion_splash(item: &str, pos: [f64; 3]);
}
