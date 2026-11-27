//! The `block_damage_abort` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player stopped digging `pos` before the block broke, after
    /// the server cleared the dig and sent the block back to the client.
    /// The player's state is not swapped in.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_block_damage_abort",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's BlockDamageAbortEvent)"
    )]
    fn block_damage_abort(player: u128, name: &str, pos: [i32; 3]);
}
