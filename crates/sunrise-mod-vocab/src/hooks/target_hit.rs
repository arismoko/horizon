//! The `target_hit` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A projectile hit the target block at `pos`, which now has `strength`
    /// output power. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_target_hit",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn target_hit(dimension: &str, pos: [i32; 3], strength: i32);
}
