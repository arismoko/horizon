<!-- chapter: Transactions -->
<!-- separator: 1 -->

On a `ServerHost`, import `HostTransaction` to call `host.transaction(|host, tx| …)` (NeoForge 26.3's `Transaction`). It runs a closure whose `Ok` commits and whose
`Err` aborts, nested in the open transaction if any. `tx`, a `&Transaction` (NeoForge's `TransactionContext`), is
what every handler and place write takes, so none happens outside one. Inside, a capability write applies at once, so later steps see it,
and the game keeps each storage as it was before the transaction first wrote it: a block entity's item slots, saved
compound and attachments, a hopper's cooldown, a cauldron's block state, a player's inventory (slots, armour, off
hand and cursor, whether the player is the host or a guest), an entity's inventory (a chest or hopper minecart,
a chest boat, a chested donkey, mule or llama) and an item entity's stack (one emptied stays, empty, until the
outermost commit removes it). A nested commit hands its changes to
the transaction around it, and only the outermost commit makes them final. Then the world is told of each change
once, as `SnapshotJournal.onRootCommit`: `setChanged` and comparators, a jukebox starting or stopping, a cauldron's
block update, and an item dropped at a block, thrown by a player (what a filled bucket's split no longer fits) or
by an entity or split off an item entity. An abort puts every storage back quietly, so nothing outside saw the
change. A panic unwinding out of the closure aborts that transaction as it passes, and the one around it stays open. Open one with `host.transaction`. The game, not the values it hands out, holds the authority, and it keeps writing apart from ending. The closure, like any handler, is lent the `Transaction`: it names one open transaction by a token the game drew (unguessable in practice), and a write given one that names no open transaction changes nothing. Ending a transaction takes its `TransactionSeal`, which only `host.transaction`'s guard (or whoever called the raw `transaction_open`) holds; the raw `transaction_close` refuses a seal that names no open transaction. So a handler can neither commit the transaction it was lent, nor end one around it, and a value decoded from bytes or kept past its closure does nothing. A transaction left open inside another is aborted when the one around it closes, so a caught panic or an `Err` still finds the rollback done. A handler keeps no journal of its own:
its state is the game's, so the game's journal covers it. Direct writes are not covered: an item entity's stack,
`LivingRef::set_equipment`, and the player and armor-stand `slot_set` paths take effect and are final at once, even
inside a transaction. Any equip/unequip event and equip sound from `set_equipment` also happen immediately; an
abort does not undo them. This differs from writes through journaled capability handlers.

Code that may run inside or outside a transaction asks the host where it is (NeoForge's `Transaction.getLifecycle`,
`getCurrentOpenedTransaction` and `TransactionContext.depth`): `host.transaction_lifecycle()` is
`TransactionLifecycle::None`, `Open`, `Closing` (a nested transaction committing into the one around it or
aborting) or `RootClosing` (the outermost closing, while the game tells the world of its commit: a
`contents_changed` listener runs then, and what it writes is outside the transaction that just ended).
`host.transaction_current()` is the innermost open `Transaction`, to write in, and none outside one; it names the
transaction only, so ending it still takes its opener's seal. `host.transaction_depth(tx)` is 0 for the outermost,
1 for one nested in it, and none for a transaction that is not open. Like transactions, these are server only.

`ItemPlace::EntitySlot { entity, slot }` is slot `slot` of such an entity's inventory (NeoForge's
`Capabilities.Item.ENTITY`), read and written like any other place.
Reading or writing a minecart's slots unpacks its loot table with no player, as `getChestVehicleItem` does.
