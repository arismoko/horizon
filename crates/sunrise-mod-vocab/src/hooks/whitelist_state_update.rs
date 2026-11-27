//! The `whitelist_state_update` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The dedicated server's whitelist changed to `enabled`.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_whitelist_state_update",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's WhitelistToggleEvent)",
        paper = "WhitelistToggleEvent"
    )]
    fn whitelist_state_update(enabled: bool);
}
