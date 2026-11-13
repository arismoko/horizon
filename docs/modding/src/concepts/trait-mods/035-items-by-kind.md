<!-- chapter: Items by kind: STORAGE -->

**Diverges from upstream: Horizon addition.** `STORAGE` (`horizon:storage`) has no counterpart in NeoForge 26.3's
transfer API. `ITEMS` (`Capabilities.Item.BLOCK`) stays the slot-indexed capability every mod reaches; `STORAGE`
sits beside it for stores, pipes and networks that move items by kind and in bulk.

A block's `ItemStorage` answers by kind, with `u64` amounts:

- `amount(host, resource)` and `space_for(host, resource)`: how much of one kind it holds, and how much an insert
  would take now (a figure to plan by, not a promise).
- `insert(host, tx, resource, amount)` and `extract(...)`: writes in the caller's transaction, never more than asked.
- `page(host, cursor, page)`: one page of what it holds. The cursor is a plain value the caller keeps (0 for the
  start); the answer is the cursor to ask for next, or none at the end.

**Who answers it.** The game answers `STORAGE` for every block entity `ITEMS` reaches. Its own containers are asked in
one pass over the slots the side reaches, by the rules hoppers follow, so a chest, a hopper and a furnace give the same
answers through either capability. A container whose `ITEMS` a mod answers is wrapped slot by slot. A mod's bulk store
(a drive, drawers, a vault) answers `STORAGE` with a provider of its own, one entry per kind however many items it
holds: register it `before_game` if the block also has slots.

**Moving.** `transfer(host, from, to, resource, max, tx)` moves one kind as one step of `move_resources`: all or
nothing, a partial take retried, and `StorageError::Overanswered` when a handler answers more than it was asked. It is
four handler calls whatever it moves. `transfer_matching` moves every kind a filter accepts, a page at a time.

**Limits the host keeps.** A `StoragePage` holds at most `MAX_STORAGE_PAGE` (256) entries whatever limit a caller asks,
and a limit of 0 is refused with `StorageError::ZeroPage`. A handler cannot push past its page, and a page crossing the
wasm boundary is checked again when it is read. Totals and cursors that do not fit in 64 bits are refused with
`StorageError::Overflow`, never wrapped.

**Cost.** A bulk store keyed by kind answers in constant time. The game's adapter answers in one pass over the slots
(O(slots)), with no work per item moved, so a question about a chest costs the same whether it moves one item or a
stack. A wrapped mod `ITEMS` handler costs one handler call per slot.

**When to use `ITEMS` instead.** A pipe that moves a handful of items a tick between small containers is cheaper on
`ITEMS` and `move_resources`, slot by slot. In the measurements behind this API, 256 pipes moving 8 items a tick each
cost 2.8 ms a tick through `ITEMS` against 3.7 ms through `STORAGE`. `STORAGE` pays off for bulk stores, moves of many
items at once, and networks large enough that sleeping while idle matters: at 4,096 pipes, idle, 17.0 ms a tick against
1.25 ms.

**Measuring it.** The engine reproduces the comparison over a grid of barrels: today's per-tick pipe on `ITEMS`
against `STORAGE` with sleeping sources, busy and then idle, with a share of moves aborted after they wrote and every
item counted before and after:

```sh
HORIZON_TEST_RESOURCES=<prepared 26.3> HORIZON_STORAGE_TIMING=1 cargo test -p horizon-client --lib --release -- \
  --ignored --exact --nocapture game::native_block_entity_item_tests::timing_storage_against_items_across_barrel_cells
```
