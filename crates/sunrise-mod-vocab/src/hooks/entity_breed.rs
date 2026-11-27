//! The `entity_breed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Two mobs completed a breeding action. `child` is absent when vanilla
    /// lays an egg instead of creating a baby.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_entity_breed",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_breed(parent: u32, partner: u32, child: Option<u32>);
}
