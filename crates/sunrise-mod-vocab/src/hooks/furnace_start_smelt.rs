//! The `furnace_start_smelt` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A furnace at `pos` began cooking `item` (its cooking timer advanced
    /// from zero). Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_furnace_start_smelt",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn furnace_start_smelt(pos: [i32; 3], item: &str);
}
