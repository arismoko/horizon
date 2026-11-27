//! The `player_stop_tracking` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A client stopped tracking an entity (NeoForge's
    /// `PlayerEvent.StopTracking`, posted at the end of
    /// `ServerEntity.removePairing`): `entity` went out of range, left the
    /// level or died, or `player` (`name`) left, changed level or respawned,
    /// which stops everything they tracked. `entity`, `uuid` and `kind` are as
    /// `player_start_tracking` told, once for each start; the player no longer counts as tracking
    /// it. A stop of the tracking pass is heard right after the removal
    /// packet, and what a listener sends the player reaches it after that
    /// removal. A leaving player's stops come after `player_logged_out`; those
    /// of a respawn or a level change, which send their own removals, at the
    /// next tracking pass. The entity is not necessarily still there, and a
    /// player who left is gone. As in `player_start_tracking`, the engine is
    /// walking its tracker: don't spawn, remove or teleport entities from
    /// here. No player's state is swapped in. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_stop_tracking",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        version = 2,
        neoforge = "PlayerEvent.StopTracking",
        cancellable = false
    )]
    fn player_stop_tracking(player: u128, name: &str, entity: u32, uuid: u128, kind: &str);
}
