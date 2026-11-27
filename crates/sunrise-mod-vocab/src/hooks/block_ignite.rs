//! The `block_ignite` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// On an authoritative server, fire is about to be placed at `pos` by
    /// flint and steel, a fire charge, lava, lightning or fire spread. Set
    /// `player` is the acting server player when there is one. Set `cancel` to
    /// leave the position unchanged.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's BlockIgniteEvent)",
        cancellable = true,
        target = "sunrise_gameplay_common::game::native_hooks::base::GameLevel<'_>::native_block_ignite"
    )]
    fn block_ignite(
        dimension: LevelDimension,
        player: Option<u128>,
        pos: BlockPos,
        cause: &str,
        cancel: &mut bool,
    );
}
