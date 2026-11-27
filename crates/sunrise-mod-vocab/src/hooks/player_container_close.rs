//! The `player_container_close` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `player` closed the `menu_type` that `player_container_open` told
    /// of (NeoForge's `PlayerContainerEvent.Close`, posted in
    /// `ServerPlayer.doCloseContainer` after the menu's `removed` and state
    /// transfer, before `containerMenu` resets to the player's inventory menu):
    /// they closed it, walked out of reach, died, changed level, or another menu
    /// opened. On disconnect, `PlayerList.remove` posts `player_logged_out`
    /// before `Player.remove` closes the menu. Once for each open; also told
    /// when the server stops, as each remaining player's menu closes before the player is saved,
    /// and a menu a listener opens during that close or save is not published. `menu`
    /// is the closing menu handle, still valid during this call with the same
    /// support boundary as `player_container_open` (`None` for crafting tables,
    /// merchants, workstations, lecterns and mount inventories), and stale once
    /// the hook returns. The menu's `removed` cleanup has already run: the
    /// cursor stack was returned to inventory or dropped, block/vehicle
    /// containers have closed, and any menu-owned transient slots have been
    /// detached. State swapped in, when the player has a body. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_container_close",
        dispatch = "flat",
        version = 2,
        side = "server",
        contract = "listen",
        neoforge = "PlayerContainerEvent.Close",
        cancellable = false
    )]
    fn player_container_close(
        player: PlayerRef,
        menu_type: &str,
        pos: Option<[i32; 3]>,
        menu: Option<mod_api::MenuRef>,
    );
}
