# Capabilities

Use capability providers to expose a block's storage to pipes, machines and hoppers. Register a
provider for a block entity type, including a vanilla type, or for a block with or without a block entity.
For example, during registration, obtain `pipe` from your declaration builder or
`r.block_entity_type_key("mymod:pipe")`, then supply your handler:

```rust,ignore
r.block_entity_capability(&ITEMS, &pipe, |_, at| {
    Some(Box::new(PipeItems(at)))
});
```

Here, `PipeItems` is your implementation of `ResourceHandler<ItemResource>`. Its `BlockSide` context
contains the level, position and requested side. Return `None` to let another provider answer.
This registration corresponds to NeoForge's `RegisterCapabilitiesEvent`.

## Register and look up providers

| Operation | Behaviour |
|---|---|
| `r.block_entity_capability(&ITEMS, &kind, provider)` | The provider receives `&mut dyn ServerHost` and `BlockSide`. |
| `r.block_capability(&ITEMS, &block, provider)` | The provider also receives the `BlockState` and `Option<BlockEntityRef>` at the position. Obtain `block` with `r.block_key("mymod:void_bin")`. |
| `r.block_entity_capability_before_game(...)` / `r.block_capability_before_game(...)` | Register a provider ahead of the game's provider. |
| `level.capability(host, &ITEMS, pos, side)` | Look up a handler on a `LevelRef`; `side` is `Option<Direction>`, with `None` meaning no particular side. The result is `Option<Capability<dyn ResourceHandler<ItemResource>>>`. |

A block provider can expose a trash can that voids items or forward a multiblock port to its controller.
A block with a block entity can have both kinds of provider.

Lookup uses this order:

1. Providers registered with `_before_game`.
2. The game's provider: for `ITEMS`, container slots exposed by side using the rules hoppers follow,
   including a mod block entity's `block_entity_container`.
3. Other registered providers.

Within each mod-provider group, block and block entity type providers form one list in registration
order. The first `Some` wins; handlers are not merged. Use `_before_game` when you need priority over
the game's handler, but do not assume precedence over another mod at the same priority.
Disabled mods' providers are skipped. A stale level handle or a level that is not in place returns
`None` without asking providers.

One capability supports at most 65,535 providers per block or block entity type. The next registration
is refused at load with a message naming the mod, capability and target and stating that the target
already has the most providers it may have.

Declare your own capability with `CapabilityKey::new("mymod:chemicals")`. Only registered providers
answer it; it has no game-provided handler. A `Capability` wraps the handler and identifies its
provider through `owner()`.

## Read and move items

`ITEMS` handlers implement `ResourceHandler<ItemResource>`, corresponding to NeoForge 26.3's
`ResourceHandler<ItemResource>`. An `ItemResource` identifies an item and its components without a count.
Each method receives the host; the handler holds no borrow of the game.

| Method | Meaning |
|---|---|
| `size` | Number of indices, numbered from 0. |
| `resource` | Resource at an index, or `None`. |
| `amount` | Amount held at an index. |
| `capacity` | `u32` limit for a specified resource, independent of the index's current contents. This is neither free space nor permission to insert. |
| `is_valid` | Whether the resource is allowed at an index. The default checks for nonzero capacity; override it for separate acceptance rules and enforce those rules in `insert`. |
| `insert` / `extract` | Insert or extract up to the requested amount at an index, using a `&Transaction`; return the amount moved. Insertion can refuse even with nonzero capacity. |

Keep handler state in transaction-managed storage such as block entity slots, saved compounds or
attachments. Your handler can look up and call a neighbour's capability through the host.

Use `move_resources(host, &from, &to, filter, max, tx)` to transfer matching resources, up to `max`,
from source indices in order into any destination index that accepts them. Each transfer uses a nested
transaction: rejected items stay in the source, and partial acceptance is retried with the accepted
amount. The result is `Result<u32, Overanswered>`. If a handler reports more than requested, its transfer
is undone and `Overanswered` identifies the provider in `owner`. Propagate the error from your
`host.transaction` closure to abort the whole operation.

There is no `simulate` parameter. To try an operation without keeping its changes, run it in a
transaction and return `Err` to abort. See [Transactions](transactions.md).

### Hopper and dropper attempts

Vanilla containers, composter-provided containers and container entities keep their direct
automation paths even if a mod also exposes storage. Only blocks without a vanilla container
use the storage attempt path, identically for native and portable handlers.

- A pull skips only a fully stacked hopper. It visits every nonempty index of the source's
  bottom-face storage in order, extracts one in a fresh transaction, then chooses the first
  receptive hopper slot (empty or matching item and components with room). An empty lower
  index wins over a matching later stack. It commits success; otherwise it aborts and continues.
- A push skips only storage whose slots are all at capacity. One root transaction covers the
  hopper slots in order: snapshot and remove one, ask the destination's indexless insertion,
  restore a refused source slot by direct write, and continue. Commit the first success or
  abort all refusals. The destination face is opposite the hopper's facing.
- A dropper inserts one in one transaction and always commits the attempt, including refusal.
  An accepted item is removed from the source before root-commit callbacks run, so a writable
  destination callback cannot move the same source item a second time.
- Hopper ticks push before pulling; a push can make room for a pull in the same tick. A push
  makes no decrement/commit changed mark. A pull marks after writing its hopper slot, before
  commit. The single final mark and eight-tick cooldown happen only after an actual move.
- A failed block-hopper pull does not try item entities. A hopper minecart still tries item
  entities after failure and has no block-hopper transfer cooldown.

Notifications your extract/insert issues happen immediately and survive abort. Silent mod
storage writes/restores do not manufacture notifications; game-held vanilla containers alone
retain implicit root-commit coalescing. Keep storage in the transaction journal: a handler that
ignores it can lose an extracted item on abort. The hopper does not guard or repair that loss.

The ignored `horizon-client` library tests under
`game::machines::native_items::storage_attempt_tests::` exercise real ticks and Parts storage,
including literal attempt/commit/abort/restore/mark traces, cooldowns, comparator scheduling,
direct vanilla/saved-pot paths, writable notification callbacks and the minecart fallback. They require prepared `HORIZON_TEST_RESOURCES`
and scratch homes/cwd.
The comparator regression uses a supported comparator in a forced, lit chunk with its 3x3
neighbours loaded. It drives the real scheduler before each hopper tick, retaining literal
comparator executions on game ticks 3 and 5 rather than manually firing those ticks.

`game::machines::native_items::storage_attempt_wasm_tests::compiled_guest_storage_notifies_reentrantly_before_actual_hopper_abort`
loads the repository's compiled `wasm-fixture` package and matching shared standard library
(`HORIZON_WASM_FIXTURE`, `HORIZON_WASM_STD`). Its pull and push cases invoke real guest storage,
reenter that same active guest through `contents_changed`, and retain the callback observations
across actual Game transaction aborts.

Look up a neighbour once per tick and reuse the `Capability` for that tick's moves. Look it up again
on the next tick because the block entity may have been removed. Reusing a lookup also avoids repeated
sandbox crossings for wasm mods.
