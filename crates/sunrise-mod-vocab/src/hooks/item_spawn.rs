//! The `item_spawn` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A dropped item entity was successfully added to the server level.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_item_spawn",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn item_spawn(entity: u32, item: &str, count: i32, pos: [f64; 3]);
}
