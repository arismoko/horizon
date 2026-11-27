//! The `menu_button` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player pressed a mod menu's button (`clickMenuButton`), on the
    /// server. `subject` lends sequential access to the live resident block
    /// entity (when present) and the server `Host`: `subject.with_view(...)`
    /// returns `None` on a blockless menu or after the block entity is removed
    /// or replaced, while `subject.host()` and `subject.transaction(...)` stay
    /// usable on both. Mods may change the live block entity in place (each
    /// write is validated at the boundary) and the clicking player's carried
    /// stack. A panicking listener is contained and disabled for `menu_button`
    /// for the rest of the session, discarding its carried-stack edit and
    /// rolling back any open transaction. A panic in a different hook does not
    /// discard the cursor edit of a button callback that returns normally.
    #[notes = r#"### Menu buttons

`menu_button` handles a mod menu's button (`AbstractContainerMenu.clickMenuButton(Player, int)`) on the server, for
single player and LAN guests. It is routed to the active menu type's owner and exposes the player identity, button,
optional block position, a live `BlockEntityScope` (`subject`) and the mutable carried stack, so a widget outside the
slots can take or give what the player carries. Call `subject.with_view(...)` to read or edit the resident block entity
in place (each write is validated at the boundary; `with_view` returns `None` on a blockless menu or after the block
entity is removed or replaced), `subject.host()` to reach the server `Host`, and `subject.transaction(...)` when an
edit must span the subject and other world storage atomically. A panicking listener is contained (its carried-stack
edit is discarded and any open transaction rolls back) and disabled for `menu_button` for the rest of the session.
A panic in a different hook does not discard the cursor edit of a button callback that returns normally."#]
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::base::native_menu_button",
        receiver = "associated",
        dispatch = "flat",
        version = 5,
        side = "server",
        contract = "listen",
        scope = "menu",
        key = "menu",
        neoforge = "none (NeoForge has no such event; vanilla AbstractContainerMenu.clickMenuButton)"
    )]
    fn menu_button(
        menu: &str,
        block: Option<&str>,
        context: &mod_api::item::menu::MenuButtonContext,
        subject: mod_api::BlockEntityScope<'_>,
        carried: &mut Option<mod_api::ItemStack>,
    );
}
