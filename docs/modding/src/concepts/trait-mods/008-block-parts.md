<!-- chapter: Block Parts -->
<!-- separator: 1 -->

`BlockPart::clone_item_stack` answers on the server when a player picks the block. It receives the
level, position, block state and `include_data`, true when a creative player picks with Ctrl. Return
`None` to keep the game's item.

`BlockPart::light_emission` answers the light a block declaring `dynamic_light: true` emits at a position
(`IBlockExtension.getLightEmission`), on both sides, with a read-only host: `Some(level)` replaces the state's
light there, `None` keeps it. It is asked as a chunk holding the block comes in, as an edit sets it and at
`LevelRef::check_light(host, pos)`, which a part whose answer reads more than the state calls when that changes; the
light engine keeps the answer until the cell's state changes and never asks on its own.

- What a part may rely on: the level it is given, read-only, the position and the state; and that it is asked
  only on the thread that owns the level. The light workers never ask: they read the stored answer.
- What is not asked, where the state's own `light` is used instead: the block's own rendered light (the mesher
  lights its faces with the state's light and the light around it), the ambient-occlusion choice, and the
  mob-spawn rule. NeoForge asks for all three, and from its light worker; don't count on any of them.
- The declaration is per block: every state of a declared block is asked, and parts on a block that does not
  declare `dynamic_light` are never asked, at any moment. NeoForge declares per state, and only its first
  lighting of a chunk skips an undeclared state; a block change or a queued check asks it either way.
- An answer above 15 is clamped to 15 at the engine, so an out-of-range value never reaches the light queue.
  NeoForge clamps only what its light manager stores.
- A client asks its own parts again for the cells of the sections a server's light update changed, at most once
  per cell per update. NeoForge reads emission again on no light packet: this is a Horizon addition, for an
  answer that reads data the client receives later.
- The game may ask more often than NeoForge does (a chunk that comes in with saved light, a client's light
  update), so keep the answer a pure function of the level, the position and the state, with no side effects.

`BlockPart::fall_on` runs at vanilla's landing moment on a read-only level random host.
Request a deferred write with `event.request(LandingWrite::SetBlock(state))`, where `state`
is an API `BlockState`, or `LandingWrite::DestroyBlock { drop }`. The game checks the
complete write batch before queuing it. An invalid block key or ordinal refuses that
callback's writes and other outputs, including damage; it never applies a valid prefix.
After the entity batch ticks, writes apply in request order if the block still matches
its original landed state. That guard runs once, so a first write can change the state
without suppressing the next write. SetBlock notifies neighbors and clients; DestroyBlock
preserves the requested resource-drop flag.

`BlockPart::contents_changed(host, event)` observes a world block's storage at `setChanged`,
with a `ServerHost`, typed `event.level`, `event.pos: BlockPos` and `event.state`. Attach it to the specific
blocks you observe, including vanilla containers; it has no global `Hooks` counterpart. The backing
contents are already written, before dirty/comparator updates. Native transactions notify once per
backing block at root commit, never on abort or simulation. An explicit `BlockEntityRef::set_changed`
notifies even if contents are unchanged. A double container notifies each component, not once again
for the other half's comparator refresh. A listener's writes still apply, but a nested notice for that
same backing block does not re-enter the listener; writes to a different backing block notify normally.
Entity/menu placeholder coordinates are not block notices.
This replaces Create's positional `contents_changed` mixin for depot/package/schematic storage; there is no
NeoForge bus event. A compiled listener that reads the committed items through the typed level (the game's
own test, so it cannot drift):

```rust,ignore
{{#include ../../../../../crates/horizon-client/src/game/blocks/machines/contents_changed_tests.rs:contents_changed_listener}}
```
