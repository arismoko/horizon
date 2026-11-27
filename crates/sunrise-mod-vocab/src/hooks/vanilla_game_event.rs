//! The `vanilla_game_event` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A vanilla game event is about to reach its listeners (NeoForge's
    /// `VanillaGameEvent`, vanilla's `Level.gameEvent`): every event, in
    /// `dimension`. `event` is its id (`minecraft:step`), `pos` where it
    /// happened and `cause` the network id of the entity that caused it
    /// (None for none). `affected_state` is the block state the event touched
    /// (`GameEvent.Context.affectedState`, with `cause` its `sourceEntity`): the
    /// block a player broke or placed, the block a mob stepped on; None where
    /// vanilla passes none (`Context.of(entity)`).
    /// Canceling keeps the sculk sensors, wardens and allays from hearing it.
    /// A stock dispenser publishes fluid pickup before routing the remainder.
    /// Inside a transaction, including `fluid_util::dispense_container`, publication waits for the outermost commit. Server-side.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "VanillaGameEvent",
        version = 3,
        cancellable = true
    )]
    fn vanilla_game_event(
        dimension: &str,
        event: &str,
        pos: [f64; 3],
        cause: Option<u32>,
        affected_state: Option<BlockState>,
        cancel: &mut bool,
    );
}
