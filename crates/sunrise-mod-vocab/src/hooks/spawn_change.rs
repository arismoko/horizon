//! The `spawn_change` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The world's spawn point changed to `pos` in level `dimension`, facing `angle`.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_spawn_change",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)",
        version = 2
    )]
    fn spawn_change(dimension: &str, pos: [i32; 3], angle: [f32; 2]);
}
