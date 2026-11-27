//! The `entities_unload` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Entities in `chunk` left the active server world and were queued for
    /// saving. `count` is the number of entity records. An idle column sweep
    /// waits for its writes: a refused record leaves the column and its entities
    /// resident instead of sending their unload notices.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_entities_unload",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entities_unload(dimension: &str, chunk: [i32; 2], count: usize);
}
