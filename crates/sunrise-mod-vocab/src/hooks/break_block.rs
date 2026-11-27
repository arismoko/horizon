//! The `break_block` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player attempts to break the block `state` at `pos`, air included: where
    /// `ServerPlayerGameMode.destroyBlock` posts NeoForge `CommonHooks.fireBlockBreak`.
    /// `event.cancelled` starts `true` when the held item, game mode, or game-master
    /// check refuses the break; listeners registered with `#[listener(receive_cancelled)]`
    /// can observe restricted attempts, and clearing `event.cancelled` bypasses the held
    /// item check while game-mode and game-master restrictions still prevent removal.
    /// Set `canceled` (return `Flow::Cancel`) to keep the block: nothing is destroyed or
    /// dropped, and a guest sees it back. Set `exp_to_drop` to `Some(amount)` to replace
    /// the experience sample later, after item drops are prepared;
    /// `None` keeps sampling. A subscribed block part's `exp_drop` answer is
    /// supplied as this event's initial amount. `block_drops` sees and may change that
    /// amount afterwards; cancellation or disabled block drops suppresses it. If a listener
    /// already removed the block or completed a nested player break at that position in
    /// that level (even if a non-air block or fluid remains), the outer break does not
    /// apply removal effects again. A break of the same block made from inside this callback does
    /// not call it again; it removes the block as a nested break.
    ///
    /// Version 3: fires for a block read from the level at the position even when it is air or its
    /// column was not resident (loaded to read it), and a nested break of the same block from the
    /// outstanding callback does not dispatch again.
    /// Server-side, for every player of the world (the
    /// helpers read the acting one). Diverges from upstream: renamed from
    /// `destroy_block`, to NeoForge's `BreakBlockEvent`.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_break_block",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "BreakBlockEvent",
        version = 3
    )]
    fn break_block(
        pos: [i32; 3],
        state: BlockState,
        canceled: &mut bool,
        exp_to_drop: &mut Option<u32>,
    );
}
