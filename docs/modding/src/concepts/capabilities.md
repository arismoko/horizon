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
Hoppers use registered block and block entity type handlers, with each move in its own transaction.

Look up a neighbour once per tick and reuse the `Capability` for that tick's moves. Look it up again
on the next tick because the block entity may have been removed. Reusing a lookup also avoids repeated
sandbox crossings for wasm mods.
