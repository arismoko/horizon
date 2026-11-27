//! The `campfire_start` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An item was placed on a campfire at `pos` and began cooking.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_campfire_start",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn campfire_start(pos: [i32; 3], item: &str);
}
