//! The `player_toggle_entity_age_lock` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player toggled an ageable mob's age lock with a golden
    /// dandelion, after the lock and item use changed.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_toggle_entity_age_lock",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_toggle_entity_age_lock(player: u128, name: &str, entity_id: u32, locked: bool);
}
