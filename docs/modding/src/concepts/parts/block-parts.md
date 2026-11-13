# Block parts

`BlockPart::clone_item_stack` answers on the server when a player picks the block. It receives the
level, position, block state and `include_data`, true when a creative player picks with Ctrl. Return
`None` to keep the game's item.

## Light emission

`BlockPart::light_emission` supplies positional light for blocks that declare
`dynamic_light: true`. It runs on both sides with a read-only host. Return `Some(level)`
to replace the state's emission or `None` to keep it.

| Rule | Behavior |
| --- | --- |
| Eligibility | Every state of a block declaring `dynamic_light` is queried; other blocks are not. |
| Calls | Chunk arrival, block edits and `LevelRef::check_light(host, pos)` query emission. |
| Caching | Light workers read the stored answer; they do not call your Part. Call `check_light` when data your answer depends on changes. |
| Thread | The callback runs on the thread that owns the level. |
| Range | Values above 15 are clamped to 15 before entering the light queue. |
| Client updates | A server light update queries each affected section's cells at most once per cell per update. |
| Exclusions | The block's own rendered light, ambient occlusion and mob-spawn rules use the state's light instead. |

Keep the callback a pure function of the level, position and state, without side effects.

## Landing writes

`BlockPart::fall_on` runs at vanilla's landing moment on a read-only level random host.
Request a deferred write with `event.request(LandingWrite::SetBlock(state))`, where `state`
is an API `BlockState`, or `LandingWrite::DestroyBlock { drop }`. The game checks the
complete write batch before queuing it. An invalid block key or ordinal refuses that
callback's writes and other outputs, including damage; it never applies a valid prefix.
After the entity batch ticks, writes apply in request order if the block still matches
its original landed state. That guard runs once, so a first write can change the state
without suppressing the next write. SetBlock notifies neighbors and clients; DestroyBlock
preserves the requested resource-drop flag.

## Storage changes

`BlockPart::contents_changed(host, event)` observes a world block's storage at `setChanged`,
with a `ServerHost`, typed `event.level`, `event.pos: BlockPos` and `event.state`. Attach it to the specific
blocks you observe, including vanilla containers; it has no global `Hooks` counterpart. The backing
contents are already written, before dirty/comparator updates. Native transactions notify once per
backing block at root commit, never on abort or simulation. An explicit `BlockEntityRef::set_changed`
notifies even if contents are unchanged. A double container notifies each component, not once again
for the other half's comparator refresh. A listener's writes still apply, but a nested notice for that
same backing block does not re-enter the listener; writes to a different backing block notify normally.
Entity/menu placeholder coordinates are not block notices.
Read the committed items through the typed level:

```rust,ignore
{{#include ../../../../../crates/horizon-client/src/game/blocks/machines/contents_changed_tests.rs:contents_changed_listener}}
```

## Crop fertility and farmland hydration

Two `BlockPart` methods let a block change what vanilla's crops and farmland ask of the blocks around them.
Both run on an authoritative server with a read-only `LevelHost`, and a block with no part answers as vanilla does.

`BlockPart::is_fertile` is asked for the soil of a growing crop. On its random tick a crop, stem or pitcher
crop asks each of the 3 × 3 blocks under it, but only a block in `#minecraft:grows_crops`: a block outside the tag is
never asked. `fertile` starts as vanilla's answer, `moisture > 0`, and the contract is `collect`: set it to decide. A
fertile block adds 3 to the growth speed and any other tagged block 1 (a quarter of either off-centre).

`BlockPart::can_be_hydrated` belongs to a farmland block. On the farmland's random tick the game scans the 9 × 2 × 9 box
from its position (x and z from −4 to 4, y 0 and 1) in x, then y, then z order, and asks only at a position whose fluid
is not empty, passing the fluid (`FluidState`) and its position. The contract is `replace`:

| Answer | Meaning |
|---|---|
| `Some(true)` | This position waters the farmland; the first one ends the scan and moisture goes to 7. |
| `Some(false)` | This position does not water it, whatever the fluid. |
| `None` | Vanilla's rule for this position: a fluid in `#minecraft:water`. |

Rain on the block above, the watered-area ticket, moisture changes and drying out are unchanged. With no part on the
farmland the scan is vanilla's.
