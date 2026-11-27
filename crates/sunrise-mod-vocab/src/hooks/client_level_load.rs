//! The `client_level_load` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The client made a new level for `dimension` (NeoForge's
    /// `LevelEvent.Load` from the `ClientLevel` constructor): on joining a
    /// server, single player's own included, and on changing dimension.
    /// `host.level(event.dimension)` resolves the new level before it is installed.
    /// The new level can read datapack registries already synchronized by the
    /// server; its chunks arrive later. On replacement, `client_level_unload` follows
    /// while the retiring level and its own handles still resolve. Client-side,
    /// on the window; the server's levels have `level_load`.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_client_level_load",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "LevelEvent.Load",
        cancellable = false,
        version = 2
    )]
    fn client_level_load(dimension: &str);
}
