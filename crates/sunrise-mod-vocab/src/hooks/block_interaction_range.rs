//! The `block_interaction_range` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether the player `player` (by UUID) may reach the block at `pos`: vanilla's
    /// `Player.isWithinBlockInteractionRange(pos, buffer)`, their eyes within their
    /// `block_interaction_range` plus `buffer` of the block. `within` arrives with vanilla's
    /// answer; set it to `true` to let the player act on a block further away, such as the
    /// blocks a remote-access block reaches, or to `false` to keep them from a block in reach.
    /// Asked on the server at: a use, a break or a pick-block request (`buffer` 1); the
    /// still-valid check of an open container, crafting table, station or lectern menu, and of
    /// a sign being edited or a lectern's book being read (`buffer` 4). A use's
    /// other checks (the hit point within a block of the block's centre, spawn protection,
    /// the world border) still apply. Asked often, for every player with a menu open: answer
    /// from what you already hold. NeoForge has no event for this.
    #[hook(
        side = "server",
        contract = "listen",
        context = "loop",
        target = "sunrise_sim::lan::guest::native_block_interaction_range",
        neoforge = "none (vanilla Player.isWithinBlockInteractionRange; NeoForge adds no event)"
    )]
    fn block_interaction_range(player: u128, pos: BlockPos, buffer: f64, within: &mut bool);
}
