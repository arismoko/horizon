<!-- chapter: Capability providers -->
<!-- separator: 1 -->

A capability is answered by providers, registered by block entity type as NeoForge's
`RegisterCapabilitiesEvent` registers them: obtain `pipe` with the declaration builder or
`r.block_entity_type_key("mymod:pipe")`, then `r.block_entity_capability(&ITEMS, &pipe, |host, at|
Some(Box::new(PipeItems)))` gives a handler for the block entity and side `at` (a `BlockSide`), or none to
leave it to the next provider. A provider may answer for a vanilla block entity type too, using its lookup key.

A block with no block entity answers by block instead (`RegisterCapabilitiesEvent.registerBlock`): a trash can
that voids items, a multiblock's port block that hands a pipe on to its controller. `r.block_capability(&ITEMS,
&bin, |host, at, state, entity| Some(Box::new(Voids)))`, with `bin` from `r.block_key("mymod:void_bin")`, is given
the position and side, the block state there and the block entity, if one stands there (an `Option<BlockEntityRef>`);
`r.block_capability_before_game` goes ahead of the game. A block that has a block entity may be answered both ways.

Providers add up: `level.capability(host, &ITEMS, pos, side)` (`Level.getCapability`, on a `LevelRef`) asks those
registered `_before_game` first, then the game's own (a block entity's slots by side, by the rules hoppers follow,
and a mod block entity's `block_entity_container`), then the others; within each group the block's providers and
the block entity type's are one list in registration order, neither kind first, and the first to answer wins. A
provider that answers none passes the lookup on; handlers are never merged. One capability may have at most 65,535 providers for one block or one block entity type: the next
registration is refused at load, naming the mod, the capability and the target. Against the game's own and against
another mod's provider at the same priority, don't count on a place: a provider that must be asked first registers
`_before_game`. It returns a `Capability`, the handler and who answered. A handler holds
no borrow: each call is given the host, so it may call a
neighbour's capability, and it keeps its state in the block entity's slots, compound or attachments. A provider
is given the block entity and side, so the handler it makes knows where its contents live. Item handlers are
NeoForge 26.3's `ResourceHandler<ItemResource>`: indices from 0 (`size`), what each holds (`resource`, an
`ItemResource`: an item and its components, without a count) and how much (`amount`), how much of a resource an
index holds when it holds that (`capacity`, none where it may not go), and `insert` and `extract`, which take a
`&Transaction` and answer how much moved. There is no `simulate`: a write happens in a transaction
(`host.transaction(|host, tx| …)`, below), and to try one, abort it. `move_resources(host, &from, &to, filter, max,
tx)` (NeoForge's `ResourceHandlerUtil.move`) moves what `filter` accepts, up to `max`, index by index out of `from`
into wherever `to` takes it, each move all or nothing: what `to` won't take stays put. It answers how much moved, or
`Overanswered` (below). Hoppers use a mod's handler
where one is registered, by block entity type or by block, each move in a transaction of its own. A mod's own capability is a
`CapabilityKey::new("mymod:chemicals")`, answered only by parts.

A `Capability` may be held for the tick: look a neighbour's up once per tick and reuse it for
every move that tick, as pipes and machines do. Ask again on the next tick, since the block
entity behind it may be gone by then. For a wasm mod each lookup crosses the sandbox (about
2.5 µs, see [Building a mod to `mod.wasm`](../../how-to/wasm-mods.md)), so holding it matters more there.
