<!-- chapter: Keeping a block capability -->

### Keeping a block capability

A pipe that asks a neighbour for `ITEMS` every tick doesn't have to look the capability up every
tick. `BlockCapabilityCache` keeps what one lookup found and gives it back while nothing the answer
depends on has changed. It is the caching half of NeoForge's `BlockCapabilityCache`, without the
listener: the cache is polled, never told.

Diverges from upstream: Horizon addition. horizon-rust/horizon has no `BlockCapabilityCache`.

#### Making one

```rust
let mut west = BlockCapabilityCache::new(&ITEMS, level, [x - 1, y, z], Some(Direction::East));
```

`new` looks nothing up. Keep the cache in your block entity's part or your machine's state, one per
neighbour and side you ask. Each cache holds at most one handler, the last one found, so what it
keeps is bounded by the number of caches you make.

#### Polling it

```rust
// Each tick, inside a transaction `tx`, with `own` your own handler.
if let Some(items) = west.get(host) {
    move_resources(host, items, own, |_| true, 8, tx)?;
}
```

`get` gives the kept handler while all of these still hold, compared in constant time with what
they were at the lookup:

- **The level is as in place as it was** (`LevelRef::in_place`). A parked level lends nothing, as a
  fresh lookup there finds nothing, and once its dimension is ticking again the cache asks again.
- **The block entity at the position is the same one.** Its handle carries a generation, so a block
  entity that was removed, replaced or unloaded with its chunk compares unequal.
- **The block state there is the same.** A chest that was turned is asked again.
- **No mod was switched on or off since.** The count comes from the game through the host, so a
  wasm mod's cache sees the same switches as a native one.

When any of these differs, `get` looks the capability up again with `LevelRef::capability`. A
successful poll allocates nothing and asks no provider.

#### When a level isn't there

A level that closed leaves its handles stale: `get` finds nothing for it, and a new world gives you
new `LevelRef`s to build new caches with. A parked level finds nothing until it is in place again.

#### Invalidating by hand

Some answers change with none of the above, such as a machine whose sides can be configured. The
cache can't see that, and the provider can't reach the caches other mods keep, so the cache's owner
calls `invalidate()` when it learns of the change, for example from the machine's own state or a
notice it sends. The next `get` then looks up again. An owner that can't tell when such an answer
changes shouldn't keep it: it looks the capability up each time instead. NeoForge's
`invalidateCapabilities()` is broadcast by the block entity to every cache; here each owner
invalidates its own.
