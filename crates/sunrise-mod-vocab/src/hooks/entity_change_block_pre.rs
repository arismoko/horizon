//! Declarations carried from upstream `entity_change_block_v2`; the generator owns the typed bindings.

pub trait Hooks {
    /// An entity changes the block at `pos` from `from` to `to`, before its
    /// neighbour updates. `player` is the player whose landing changed it;
    /// `entity` and `kind` identify a mob for a mob's block-changing action.
    /// Cancel to keep `from`. Farmland landings use `farmland_trample` instead;
    /// `entity_change_block` observes a completed mob block change.
    #[hook(
        side = "server",
        contract = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's EntityChangeBlockEvent is cancellable)",
        cancellable = true,
        version = 2,
        out = "cancel",
        target = "sunrise_gameplay::game::native_hooks::entity_change_block_pre::ServerGame<'_>::native_entity_change_block_pre"
    )]
    fn entity_change_block_pre(
        player: Option<u128>,
        entity: Option<u32>,
        kind: &str,
        pos: [i32; 3],
        from: BlockState,
        to: BlockState,
        cancel: &mut bool,
    );
}
