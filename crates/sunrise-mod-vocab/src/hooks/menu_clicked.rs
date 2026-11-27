//! The `menu_clicked` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A click in `menu`, before its default action (`ServerGamePacketListenerImpl.handleContainerClick`,
    /// `AbstractContainerMenu.clicked`). Only parts registered for that menu type are asked.
    /// The player's own `InventoryMenu` (window 0, while no other menu is open) is
    /// `minecraft:inventory`: it has no owner, so every mod's Parts attached to it are asked, in
    /// mod order, and the default runs only if none handles the click; a handled click resends
    /// the inventory. The creative picker is never asked.
    /// `slot` is in menu order (container slots, then any player inventory; -999 outside),
    /// `button` and `mode` are the click input, and `uuid` identifies the clicking player.
    /// Menu-button clicks, recipe placement and the creative set-slot packet do not call it.
    /// Return [`Flow::Cancel`] after applying the click yourself to skip the default: for
    /// example, copy an item into a filter slot without consuming the cursor.
    /// A display-only menu has no default moves. Other menus keep their ordinary slot rules
    /// when unhandled. The client predicts the ordinary click and the server corrects it.
    /// No player's state is swapped in. Use their current `PlayerRef::container_menu` to reach
    /// its slots and cursor, or [`MenuRef::set_items`] to replace a display menu's contents.
    /// Runs directly on the receiving menu without a synthetic decided-action wrapper or
    /// whole-inventory rollback: open a transaction explicitly with [`ServerHost::transaction`]
    /// when a callback wants atomic multi-step edits, and prior unrelated writes stand even if
    /// the default click is cancelled or refused. On [`Flow::Continue`], the server rechecks
    /// receiver preconditions (live player, same menu generation and window, non-spectator,
    /// the menu's `stillValid` answer and slot index) before running default click execution on
    /// the live menu. Closing or replacing it prevents the default; inside a transaction, the
    /// close or replacement takes effect only when the outermost transaction commits.
    /// A panic aborts any transaction still open in that callback and disables only that mod's
    /// `menu_clicked` callback for the session, with an error log and toast naming the mod.
    /// Remaining parts continue, but the failed click's default never runs. The server resyncs
    /// the still-live menu and keeps committed edits.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_menu_clicked",
        dispatch = "api_carrier",
        side = "server",
        contract = "cancel",
        scope = "menu",
        key = "menu",
        out = "handled",
        cancellable = true,
        neoforge = "none (a Java menu overrides AbstractContainerMenu.clicked; no event)",
        version = 5
    )]
    fn menu_clicked(
        menu: &str,
        uuid: u128,
        window: i32,
        slot: i32,
        button: i32,
        mode: i32,
        handled: &mut bool,
    );
}
