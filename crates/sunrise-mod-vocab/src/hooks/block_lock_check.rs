//! The `block_lock_check` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A guest's block inventory lock check at `pos`. `locked` is the
    /// vanilla result and may be changed. The guest's state is not swapped in.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_block_lock_check",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "locked",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn block_lock_check(player: u128, name: &str, pos: [i32; 3], locked: &mut bool);
}
