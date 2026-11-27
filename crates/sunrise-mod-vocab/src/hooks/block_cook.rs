//! The `block_cook` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A block finished cooking `item` into `result` at `pos`.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_block_cook",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn block_cook(pos: [i32; 3], item: &str, result: &str);
}
