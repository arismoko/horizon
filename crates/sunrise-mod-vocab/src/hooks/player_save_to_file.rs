//! The `player_save_to_file` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player was saved to the world's store (NeoForge's
    /// `PlayerEvent.SaveToFile`, `PlayerList.save`): when they leave or the
    /// world closes, when a dedicated or integrated server saves its guests,
    /// and for an integrated server's owner when the world saves them.
    /// Fires only after successful replacement of the player file, independently
    /// of world metadata. `directory` is the world's player directory
    /// (`players/data`): write your own file
    /// there, named `<uuid>.<suffix>`, never the player's `.dat`. The player
    /// may be leaving, so no player's state is swapped in. Not cancellable.
    /// On a full world save, including an autosave, successful player writes and
    /// these notifications finish before the first `level_save`; placed players
    /// stay online. A world save before any player joins has no player to save.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_save_to_file",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "PlayerEvent.SaveToFile (not cancellable)",
        cancellable = false,
        version = 2
    )]
    fn player_save_to_file(player: u128, name: &str, directory: &std::path::Path);
}
