//! The `menu_still_valid` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether a player's mod menu stays open (`AbstractContainerMenu.stillValid`).
    /// Asked on the server each tick the menu is open and before processing a
    /// matching non-spectator container click. If `menu_clicked` listeners run and leave the
    /// menu open and the default unhandled, this check runs again before the default action.
    /// `pos` is the backing block's position, or `None` for a blockless menu
    /// even if its opener supplied a position for the client's screen. `opening_item` is
    /// present when the opener supplied a hand; its `stack_matches` says whether that hand
    /// holds the same item with the same components as at open (count ignored). Neither
    /// affects the initial validity answer.
    /// `valid` starts true without a backing block; for a block menu it is
    /// whether `eye` is within the player's block interaction range plus 4 of
    /// each backing block. Set it to keep the menu open or close it. Vanilla
    /// menu types are never asked, and a menu whose block entity is gone
    /// closes without asking. Answer from the event and loop context alone.
    #[notes = r##"### Keeping a mod menu open

`menu_still_valid` is vanilla's `AbstractContainerMenu.stillValid(Player)`, the method a Java menu overrides to
say whether a player may keep it open. The server asks the scoped `MenuPart` of the open mod menu's type each
tick the menu is open and before processing a matching non-spectator container click. A surviving menu is
checked again after `menu_clicked` listeners if the default click will run. A click for a different window is
dropped; a matching spectator click resends the menu without asking the part. Vanilla menu types keep
their own check and never ask a part. A false answer closes the menu as walking away does: what the
cursor held goes back to the inventory and the client's window closes.

`event.player` is the player's UUID and `event.menu` is the menu type. `event.pos` is the backing block's
position, or `None` for a blockless menu even if its opener supplied a position for the client's screen.
An item opener can supply a hand independently of how the menu stores its slots. The part sees it as
`event.opening_item`; its `stack_matches` compares the hand's current item and components with its
contents at open, ignoring count. This compares values, not stack instance identity. A part can use the result
when setting `event.valid`; the game does not close the menu just because the answer is false.
`event.valid` starts true for blockless menus. For block menus it starts true only if the player's eyes
(`event.eye`) are within their block interaction range plus 4 of each backing block. A part may keep a larger
machine's menu open past that distance, or close a menu for any reason. A part that leaves `valid` alone, or
whose mod is switched off, keeps the game's answer. A menu whose block entity is gone closes without asking;
respawning, changing dimension or leaving also closes the menu.

The hook runs in the loop context (no level or game Host) on every open menu of the type each tick, so answer
from the event and the context alone, and fast. Whether the type has a part that answers is looked up once,
when the menu opens: a menu with none does no work for this hook."##]
    #[hook(
        side = "server",
        contract = "listen",
        scope = "menu",
        key = "menu",
        context = "loop",
        version = 4,
        target = "sunrise_sim::lan::menu::native_menu_still_valid",
        neoforge = "none (vanilla AbstractContainerMenu.stillValid, which each menu overrides)"
    )]
    fn menu_still_valid(
        player: u128,
        menu: &str,
        pos: Option<[i32; 3]>,
        eye: [f64; 3],
        opening_item: Option<MenuOpeningItem>,
        valid: &mut bool,
    );
}
