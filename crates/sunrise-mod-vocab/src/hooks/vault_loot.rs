//! The `vault_loot` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A vault at `pos` is about to eject a loot item.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_vault_loot",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's BlockDispenseLoot)"
    )]
    fn vault_loot(pos: [i32; 3], item: &str);
}
