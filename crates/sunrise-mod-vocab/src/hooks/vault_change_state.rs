//! The `vault_change_state` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A vault at `pos` changed to `state`. Its block state is updated.
    /// Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_vault_change_state",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn vault_change_state(dimension: &str, pos: [i32; 3], state: &str);
}
