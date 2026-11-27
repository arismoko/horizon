//! The `entity_unleash` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A leashed mob's leash was removed, before any lead drop is handled
    /// (Bukkit's `EntityUnleashEvent`).
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_entity_unleash",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's EntityUnleashEvent)"
    )]
    fn entity_unleash(entity: u32, kind: &str);
}
