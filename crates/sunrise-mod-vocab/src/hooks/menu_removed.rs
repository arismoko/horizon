//! The `menu_removed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A menu's owner teardown (`AbstractContainerMenu.removed`): the player closed it,
    /// walked out of reach, died, changed level, opened another menu, or left. Routed
    /// to the `MenuPart` owning `menu` (or `block` for a stock menu opened by a mod
    /// block). `player` is the live player handle when available, or `None` if the player
    /// is already gone (for example when a deferred teardown runs after disconnect).
    /// `uuid` always identifies the player whose menu closed, and `window` is the closed
    /// container id. `opening_hand` is the hand the opener supplied, if any. `owned_slots`
    /// contains the transient container's detached final values in slot order, or an empty
    /// slice when there was no such container; those stacks are not returned or dropped as
    /// entities by the game. The menu's transient storage is already detached before this
    /// callback runs, and `player_container_close` follows once teardown finishes.
    /// Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::menu_removed::ServerGame<'_>::native_menu_removed",
        dispatch = "api_carrier",
        side = "server",
        contract = "listen",
        scope = "menu",
        key = "menu",
        neoforge = "none (vanilla AbstractContainerMenu.removed, which each menu overrides)"
    )]
    fn menu_removed(
        menu: &str,
        block: Option<&str>,
        player: Option<PlayerRef>,
        uuid: u128,
        window: i32,
        opening_hand: Option<Hand>,
        owned_slots: &[Option<ItemStack>],
    );
}
