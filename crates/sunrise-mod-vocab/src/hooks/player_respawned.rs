//! The `player_respawned` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `PlayerRespawnPositionEvent`: where `player` respawns, once the server has
    /// found it: the bed or anchor they stand up at (facing it), or the world
    /// spawn's free column. `original_level` and `original_position` are that
    /// point; `missing_respawn_block` says the bed or anchor was gone, so the
    /// player came back at the world spawn. Change `level`, `position`, `yaw`
    /// and `pitch` to choose their final destination; no preliminary placement is made.
    /// `copy_original_spawn_position` (default: the block wasn't missing) says whether
    /// they keep their respawn point. Runs on the removed original player, before
    /// `player_clone` and placement. To change the point copied for future respawns,
    /// call `event.player.set_respawn_config(host, ...)` here, even on death, and set
    /// `copy_original_spawn_position` to true. This does not change the current destination.
    /// `from_end_fight` is true for the respawn after the End credits,
    /// false for death. An ordinary End portal teleport does not run this hook. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_respawn_position",
        dispatch = "api_carrier",
        side = "server",
        contract = "listen",
        version = 3,
        neoforge = "PlayerRespawnPositionEvent"
    )]
    fn player_respawn_position(
        player: PlayerRef,
        original_level: LevelDimension,
        original_position: [f64; 3],
        missing_respawn_block: bool,
        from_end_fight: bool,
        level: &mut LevelDimension,
        position: &mut [f64; 3],
        yaw: &mut f32,
        pitch: &mut f32,
        copy_original_spawn_position: &mut bool,
    );
    /// `player` (`name`) respawned (NeoForge's `PlayerRespawnEvent`): at the
    /// end of `PlayerList.respawn`, after placement and initial synchronization,
    /// with the replacement in scope. `end_conquered` is true after the End credits
    /// (the player keeps health, food and experience), false after death. An ordinary
    /// End portal teleport does not fire this hook. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_respawned",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        version = 2,
        neoforge = "PlayerEvent.PlayerRespawnEvent"
    )]
    fn player_respawned(player: u128, name: &str, end_conquered: bool);
}
