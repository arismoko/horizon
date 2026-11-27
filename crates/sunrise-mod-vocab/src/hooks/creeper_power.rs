//! The `creeper_power` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Lightning charged a creeper.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_creeper_power",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn creeper_power(entity: u32, lightning: [f64; 3]);
}
