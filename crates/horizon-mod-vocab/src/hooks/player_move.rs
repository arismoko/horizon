//! The `player_move` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player moved or turned: `from` and `to` are their positions,
    /// `from_rotation` and `to_rotation` their `[yaw, pitch]`. Heard once a
    /// tick, after the server accepted their movement, when they moved more
    /// than 1/16 of a block or turned more than 10 degrees since they were
    /// last heard (Paper's thresholds). Set `cancel` and they are teleported
    /// back to `from`, facing as they did there. Their state is swapped in
    /// (`host.players().acting_player()`, `PlayerRef::tell`). Not heard: a
    /// passenger (`vehicle_moved` hears its vehicle), a move into another
    /// level, a teleport. Server-side, in the level the player is in.
    #[notes = r#"## Player move

- **`player_move(host, event)`**: Bukkit's `PlayerMoveEvent` for a guest's own movement. Paper fires it from `ServerGamePacketListenerImpl.handleMovePlayer` for each packet that moved the player more than 1/256 of a block squared or turned them more than 10 degrees since the last call, and a cancelled event teleports the player back to `from`. Here the hook is heard once a tick, after the hub accepted the tick's moves, with the same thresholds against the place the player was last heard at, so a cancel undoes the whole tick's movement. A player's first tick in a level only records where they are."#]
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
