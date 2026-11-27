//! The `crafter_craft` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A crafter at `pos` completed recipe `recipe`.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_crafter_craft",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn crafter_craft(pos: [i32; 3], recipe: &str);
}
