//! The `furnace_burn` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A furnace at `pos` consumed fuel and began or resumed burning it.
    /// `fuel` is the item consumed from its fuel slot. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_furnace_burn",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn furnace_burn(pos: [i32; 3], fuel: &str);
}
