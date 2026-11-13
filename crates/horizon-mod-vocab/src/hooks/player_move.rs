//! The `player_move` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player's movement report is about to be applied: they would move from
    /// `from` to `to`, or turn from `from_rotation` to `to_rotation` (`[yaw,
    /// pitch]`). Heard once for each validated report whose position or
    /// rotation differs from the last validated connection report, after the server's own
    /// validation and before the report is committed. Set `cancel` for a real
    /// veto and not a rollback: the report is not applied and its bookkeeping
    /// is skipped, and the client is corrected to the accepted position and
    /// rotation through the existing teleport correction. The veto adds no
    /// anticheat violation of its own, but the report was already seen by the
    /// server's own checks and by the anticheat's record of the client's ticks
    /// before the hook, and what a violation of those would have done still
    /// happens. The values are the exact ones: there
    /// is no once-per-tick accumulation and no threshold in the engine, so the
    /// mod filters (`if (to - from).length() < 1.0 / 16.0 { return; }` skips
    /// anything under 1/16 of a block). Their state is swapped in
    /// (`host.players().acting_player()`, `PlayerRef::tell`).
    ///
    /// A held report is ruled on in the same tick, and the player's later
    /// packets (further reports, the end of the client tick, pongs) wait in
    /// order behind it and are read from the next tick on. A report is out of
    /// time at its 21st tick, counting the tick it was held at as the first: it
    /// is dropped with the correction and never committed, even when a ruling
    /// arrives after that. A player's disconnect, teleport or level change
    /// drops a pending report cleanly. Not heard: the first tick in a level (the
    /// tick after a join or a level change; the second is heard), a player riding
    /// something (`vehicle_moved` hears the vehicle), and any teleport: its
    /// confirmation is validated and committed without the hook, and so is
    /// the veto's own correction, and passage crossings. With no listener the report is committed in
    /// place with no extra work: nothing is held, copied or allocated. With one,
    /// the cost is one held report for each moving player a tick. Server-side.
    /// A wasm fault or epoch timeout answers nothing, not a veto: the report
    /// commits if no other listener vetoes. The pending deadline applies across
    /// ticks only. A non-returning native listener is out of scope and blocks
    /// the game thread.
    #[notes = r#"## Player move

- **`player_move(host, event)`**: Bukkit's `PlayerMoveEvent` for a guest's own movement, fired from `ServerGamePacketListenerImpl.handlePlayerPositionChange`'s place: after the server validated the report, before it is committed. Cancelling vetoes it; the client is sent back to the accepted position and rotation. The engine applies no threshold: a mod that wants Paper's 1/256 square block or ten degrees compares `from` and `to` itself."#]
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no player move event; PlayerTickEvent covers the tick)",
        paper = "PlayerMoveEvent",
        cancellable = true
    )]
    fn player_move();
}
