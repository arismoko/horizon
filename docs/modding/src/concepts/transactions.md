# Transactions

Use `HostTransaction` on a `ServerHost` to group capability writes. The closure's
`Ok` commits; `Err` aborts. Pass the borrowed `&Transaction` to each handler or
place write.

```rust,ignore
use mod_api::{HostTransaction, ServerHost};

fn update(host: &mut dyn ServerHost) -> Result<(), ()> {
    host.transaction(|host, tx| {
        // Pass host and tx to your capability writes.
        Ok(())
    })
}
```

## Commit and rollback

- Writes apply immediately, so subsequent steps in the transaction see them.
- The game snapshots each storage before its first write. Handlers use the game's
  journal rather than keeping their own.
- A nested commit merges into its parent. Only the outermost commit makes storage
  changes final and publishes deferred game-held vanilla container effects.
- An abort restores recorded storage without publishing those commit effects.
- Raw mod-owned storage writes and restores issue no synthetic notifications.
- Explicit changed/comparator/neighbour/data-send notifications run when issued,
  including inside a transaction, and survive abort.
- A panic unwinding out of the closure aborts its transaction. The parent remains
  open if you catch the panic inside the parent closure.
- Closing a transaction aborts any transactions left open inside it before closing
  the parent.

The journal covers writes through capability handlers to these storages:

| Storage | Recorded state |
|---|---|
| Block entity | Item slots, saved compound, custom name and attachments |
| Hopper | Cooldown |
| Cauldron | Block state |
| Player | Inventory slots, armour, offhand and cursor, for host and guest players |
| Entity inventory | Chest or hopper minecart, chest boat, chested donkey, mule or llama inventory |
| Item entity | Stack; an emptied entity remains until the outermost commit removes it |

At the outermost commit, game-held vanilla containers publish their implicit change
notifications. Other deferred effects include jukebox playback, cauldron block updates and
item drops, including items placed at a block, thrown by a player or entity, or split off an
item entity. Explicit mod-issued world notifications are not deferred with these effects.

Direct writes are not journaled: direct item-entity stack writes,
`LivingRef::set_equipment`, and the player and armor-stand `slot_set` paths take
effect immediately, even inside a transaction. Equipment sounds and equip/unequip
events from `LivingRef::set_equipment` also happen immediately. An abort does not
undo these writes or effects. Use journaled capability handlers when you need rollback.

## Transaction authority

`Transaction` identifies an open transaction with a game-issued write token. It
does not grant permission to close it. A write with a token that does not name an
open transaction changes nothing, including a token retained past its closure.

Closing requires a separate `TransactionSeal`. The `host.transaction` guard holds
that seal; its closure receives only the write token. If you use the raw
`transaction_open` method, you receive the seal and must close it with
`transaction_close`. The game refuses a seal that does not match an open
transaction and logs `Horizon: refused to close a transaction the seal does not end`.

## Inspecting a transaction

These methods are server-only:

| Method | Result |
|---|---|
| `host.transaction_current()` | Innermost open `Transaction`, or `None`; not a closing seal |
| `host.transaction_depth(tx)` | `Some(0)` for the outermost transaction, increasing by one per nesting level; `None` if it is not open |
| `host.transaction_lifecycle()` | `TransactionLifecycle` describing the open or closing phase |

| `TransactionLifecycle` variant | Meaning |
|---|---|
| `None` | No transaction is open |
| `Open` | A transaction is open and none is closing |
| `Closing` | A nested transaction is committing into its parent or aborting |
| `RootClosing` | The outermost transaction is closing |

Implicit vanilla container notices run during `RootClosing`, outside the transaction whose
commit triggered them. Explicit mod notices issued during a transaction run during `Open`.

## Entity inventory slots

`ItemPlace::EntitySlot { entity, slot }` addresses an entity inventory slot, like
NeoForge's `Capabilities.Item.ENTITY`. Read and write it like any other item place.
Reading or writing a minecart's slots unpacks its loot table without a player,
as vanilla's `getChestVehicleItem` does.
