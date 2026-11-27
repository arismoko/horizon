//! The `brewing_start` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A brewing stand at `pos` began brewing `ingredient`, after fuel and the timer were set. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_brewing_start",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn brewing_start(pos: [i32; 3], ingredient: &str);
}
