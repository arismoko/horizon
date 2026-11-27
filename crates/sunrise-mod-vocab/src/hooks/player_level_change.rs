//! The `player_level_change` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player's experience level actually changed. A signed point grant
    /// reports once after it finishes, not once per level; level grants/debits and
    /// `ServerPlayer.setExperienceLevels` report their applied changes too. A
    /// canceled grant or a write of the same value is not reported.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_level_change",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        version = 2,
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerLevelChangeEvent)",
        paper = "PlayerLevelChangeEvent"
    )]
    fn player_level_change(player: u128, name: &str, from: i32, to: i32);
}
