//! The `player_changed_dimension` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `player` (`name`) changed dimension (NeoForge's
    /// `PlayerChangedDimensionEvent`): `from` and `to` are dimension ids
    /// (`minecraft:overworld`, `minecraft:the_nether`, `minecraft:the_end`).
    /// Fires once a cross-dimension teleport lands: a Nether or End portal
    /// (including travel on a vehicle and later End portal exits), or a
    /// command such as `/execute in ... run tp`. The destination level is in
    /// place and the player's state is swapped in, so `host.players().acting_player()`
    /// and `PlayerRef::tell` reach them. Runs on the logical server, not the client.
    /// Does not fire for a move within a level, a player who leaves before
    /// arrival, or a replacement created by respawn, even in another level.
    /// Use `player_respawned` for respawns after death or after the End credits.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_changed_dimension",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        version = 2,
        neoforge = "PlayerEvent.PlayerChangedDimensionEvent"
    )]
    fn player_changed_dimension(player: u128, name: &str, from: &str, to: &str);
}
