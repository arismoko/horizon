//! The `menu_quick_move` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player shift-clicked slot `slot` of the menu of the mod block
    /// entity (`AbstractContainerMenu.quickMoveStack`, the method a Java
    /// menu overrides), on the server, before the game's own routing:
    /// move the stack where your block wants it. `subject` lends sequential
    /// access to the live resident block entity (`subject.with_view(...)`),
    /// the server `Host` (`subject.host()`), and atomic transactions
    /// (`subject.transaction(...)`); `with_view` returns `None` if the block
    /// entity was removed or replaced during the call. `slot` is the menu's
    /// index: below `container_slots` it is a container slot (the block's own,
    /// a linked slot, or a crafting result); from there on the player's inventory
    /// (27 main slots, then the hotbar). After the live slot allows pickup,
    /// `stack` copies what it holds. Take what you move out of `stack` and return
    /// `Flow::Cancel`. The same source keeps the remainder (`None` empties it)
    /// and the client gets the menu again. A result handler consumes its own inputs;
    /// the remainder stays available for the inputs it leaves until the grid changes.
    /// Left unhandled, the game undoes the listener's journaled writes before routing the
    /// same menu click without a second pickup check. A later Part sees accepted proposals.
    ///
    /// Writes through `subject.with_view(...)` validate at the boundary and apply to
    /// the live resident block entity in place. Journaled writes and the source remainder
    /// commit together; a refused move (invalid remainder, changed source or stale menu)
    /// rolls them back. Implicit container notices (`BlockEntity.setChanged`) are delivered
    /// after settlement, only to still-resident holders, and before `menu_slot_take`.
    /// Explicit `BlockEntityRef::set_changed` calls are immediate; a same-block write made
    /// by that notice's callback does not queue another notice.
    /// A panicking callback loses its journaled writes and outputs and is disabled for this hook.
    /// A later Part can still handle the move; if none does, a quick-move Part panic
    /// causes a resync instead of falling back. A panic contained in a nested hook does
    /// not by itself reject the move.
    ///
    /// For a mod container on a vanilla menu type and on a mod menu type,
    /// the left or right button, a slot that holds something and that the
    /// mod's screen shows; not for a spectator or a dead player, a menu no
    /// longer valid, nor during a drag.
    /// Asked once per click, where vanilla asks `quickMoveStack` again while
    /// it keeps moving the same item. The player's state is swapped in, so
    /// `subject.host().players().acting_player()` names them.
    #[notes = r#"## Shift-click in a mod container

`menu_quick_move(kind, subject, container_slots, slot, stack, handled)` (contract version 6) is a mod container's
`AbstractContainerMenu.quickMoveStack`, the method a Java menu overrides to say where a shift-clicked stack goes. It is
asked on the server before the game's own routing, for the mod block entity whose menu a player shift-clicked
with the left or right button, on a vanilla menu type (`minecraft:generic_9x1`...) or on the mod's own menu type.
`subject` (`BlockEntityScope<'_>`) lends sequential access to the live resident block entity (`subject.with_view(...)`,
which exposes `position()`, `item_slot(...)`, `set_item_slot(...)`, `nbt()`, `data_slots()` and returns `None` if the
block entity was removed or replaced during the call), the server `Host` (`subject.host()`), and atomic transactions
(`subject.transaction(...)`). `slot` is the menu's index: below `container_slots` a container slot (its own, linked,
or the crafting result), from there the player's inventory, its 27 main slots, then the hotbar. The live menu asks
`Slot.mayPickup` first, then passes a copy of what the slot holds as `stack`. Take what you move out of `stack` and
return `Flow::Cancel`: the actual clicked source keeps the remainder (`None` empties it), the game's routing is
skipped, and the player's client gets the whole menu again. For a linked slot or result this does not replace an
own storage slot. A result handler must consume its inputs: the remainder stays available for the inputs it leaves
until the grid changes. Left unhandled, the game's routing runs on the original clicked stack without asking pickup
again: the listener's journaled edits are rolled back first. A later Part sees the earlier healthy Parts' proposals.
Writes through `subject.with_view(...)` validate at the boundary and apply to the live resident block entity in place.
The journaled edits and clicked-slot settlement commit together, before publishing implicit `BlockEntity.setChanged`
notices and calling `menu_slot_take` where applicable. Deferred notices are delivered only to still-resident holders.
An explicit `BlockEntityRef::set_changed` call is immediate; a same-block write in that callback does not queue another
notice. A panicking Part loses its journaled edits and outputs and is disabled for the rest of the session. A later
Part can still handle the move; if none does after a quick-move Part panics, the menu is resynced instead of falling back.
A panic contained in a nested hook does not by itself reject the move. A refused move, including an invalid remainder,
a changed clicked stack, or a stale menu, rolls back its journaled writes and does not fall back.

Not asked for an empty slot, a slot the mod's screen hides, another click type, a spectator or a dead player, a menu
no longer valid, or a shift-click during a drag (it only ends the drag, as in vanilla). It is asked once per click, where vanilla asks `quickMoveStack` again while the same item keeps moving: move
all you want in one call. The clicking player's state is swapped in (`subject.host().players().acting_player()` names them), and the block entity stays resident in the level during the call. Only a server's players (single player's server
thread, LAN guests, a dedicated server) shift-click this way; a native mod's own menus (`native_open_menu`) get their
clicks already."#]
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::base::native_menu_quick_move",
        receiver = "associated",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "handled",
        scope = "block_entity",
        key = "kind",
        neoforge = "none (a Java menu overrides AbstractContainerMenu.quickMoveStack; no event)",
        cancellable = true,
        version = 6
    )]
    fn menu_quick_move(
        kind: mod_api::BlockEntityTypeKey,
        subject: mod_api::BlockEntityScope<'_>,
        container_slots: usize,
        slot: usize,
        stack: &mut Option<mod_api::ItemStack>,
        handled: &mut bool,
    );
}
