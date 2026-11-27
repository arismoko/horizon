//! The `client_level_unload` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The client is dropping its level for `dimension` (`LevelEvent.Unload`
    /// from `Minecraft.setLevel`, `clearClientLevel`, and `disconnect`). On
    /// replacement, the new level's load is heard first, but
    /// `host.level(event.dimension)` still resolves the retiring level, and
    /// previously acquired handles can still read its chunks and available
    /// synchronized datapack registries in this callback. Reconfiguration,
    /// leaving a server, and quitting all unload the current level once.
    /// When quitting single player, this fires before halting its server.
    /// Client-side.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_client_level_unload",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "LevelEvent.Unload",
        cancellable = false,
        version = 3
    )]
    fn client_level_unload(dimension: &str);
}
