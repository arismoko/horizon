//! The `explosion_prime` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server explosion is about to begin at `center`, with `radius` (Bukkit's
    /// `ExplosionPrimeEvent`). This is listen-only: the explosion proceeds as
    /// usual. Server-side, including the single-player server.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_explosion_prime",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's ExplosionPrimeEvent)"
    )]
    fn explosion_prime(center: [f64; 3], radius: f32);
}
