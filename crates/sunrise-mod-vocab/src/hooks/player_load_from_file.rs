//! The `player_load_from_file` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player was loaded from the world's save (NeoForge's
    /// `PlayerEvent.LoadFromFile`, `PlayerList.loadPlayerData`): their `.dat`
    /// has been read and applied, and they are not placed in a level yet.
    /// `directory` is the world's player directory (`players/data`): a mod
    /// reads its own file there, named `<uuid>.<suffix>` as
    /// `getPlayerFile(suffix)` names it. Heard once for each join, for the
    /// owner of an integrated server as for anyone else, on a server, with no
    /// player's state swapped in. Not cancellable.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_load_from_file",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "PlayerEvent.LoadFromFile (not cancellable)",
        cancellable = false
    )]
    fn player_load_from_file(player: u128, name: &str, directory: &std::path::Path);
}
