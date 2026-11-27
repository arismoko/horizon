# Block parts

`BlockPart::clone_item_stack` answers on the server when a player picks the block. It receives the
level, position, block state, and `include_data`, true when a creative player picks with Ctrl.
`event.player` is the player who sent the pick request. Use it with the read-only host for
player reads such as `event.player.food(host).map(|food| food.level)`. The request does not include
a client hit face. Return `None` to keep the game's item.

A server mod reads the same answer, with that player acting, through `PlayerRef::clone_item_stack`
(see its documentation in the [API reference](../../reference/api.md)).

## Breaking-crack display

`BlockPart::break_progress_positions(host, event)` supplies extra positions for a
client crack, with a mutable `ClientHost`. The event identifies the client `level`,
original `pos`, cracked `state` and `progress` (0 through 9). Append positions to
`event.positions`, which starts empty. The original position is drawn separately.
For example, a multi-block can add its other three cells so all four show the same
crack stage. NeoForge has no counterpart to this vanilla
`LevelRenderer.destroyBlockProgress` moment.

The part runs only on the cracked state's subscribed block, when a crack is set
or changes stage/position, for both local prediction and other players' cracks.
Repeated updates at the same stage refresh the remote crack's expiry without
asking again. Rendering reads the stored answer, never your callback. Clearing
the crack, closing/resetting the client level or expiring a remote crack after
400 ticks removes its extras too. Local extras disappear with local mining.

These are **display bounds**, not interaction limits: at most 64 distinct extras,
each within Euclidean distance 8 blocks of the original position. `BlockPos`
coordinates refer to the supplied level. The original position, duplicates,
out-of-range values and extras past the limit are dropped with a debug log,
never a panic. Existing render-distance and block-overlay eligibility filters
still apply. Overlapping cracks use the highest stage for each cell. The callback
changes neither break speed nor which blocks break, and never runs on a server.

## Random display ticks

`BlockPart::animate_tick(host, event)` is vanilla's `Block.animateTick`, with a mutable
`ClientHost`. It runs when the client's ambient sampling (`ClientLevel.animateTick`, 667 random
positions within 16 blocks and 667 within 32 each tick) picks a position whose block has the part.
It runs inside the pick: after the block's own vanilla effects, before its fluid's. The event has the client `level`, `pos`, `state`
and the level's `random`. Add particles with `host.add_particle` and play sounds with
`host.play_local_sound`. A block without the method costs the sampler one table lookup, and the
hook never runs on a server. See [Random display ticks for blocks](../../how-to/client/block-animate-ticks.md).

## Light emission

`BlockPart::light_emission` supplies positional light for blocks that declare
`dynamic_light: true`. It runs on both sides with a read-only host. Return `Some(level)`
to replace the state's emission or `None` to keep it. Use `event.state` as the emitter;
a [contextual query](../../how-to/world/block-state-facts.md#contextual-light-emission) can name
an unplaced state at that position.

| Rule | Behavior |
| --- | --- |
| Eligibility | Every state of a block declaring `dynamic_light` is queried; other blocks are not. |
| Calls | Explicit `LevelRef::get_light_emission` reads, chunk arrival, block edits and `LevelRef::check_light` query emission. |
| Caching | Light workers read the stored answer; they do not call your Part. Call `check_light` when data your answer depends on changes. |
| Thread | The callback runs on the thread that owns the level. |
| Range | Explicit reads return the Part's raw answer. Values above 15 are clamped only before entering the light queue. |
| Client updates | A server light update queries each affected section's cells at most once per cell per update. |
| Exclusions | The block's own rendered light, ambient occlusion and mob-spawn rules use the state's light instead. |

Keep the callback a pure function of the level, position and state, without side effects.

## Enchanting power

`BlockPart::enchant_power_bonus` receives the queried `state`, provider `pos`, `table_pos`,
and a read-only host. `event.power` starts at 1.0 for `#minecraft:enchantment_power_provider`
and 0.0 otherwise. Explicit reads receive the answer unchanged; offers and particles keep the
default if the Part answers with NaN or infinity.

For live offers and `LevelRef::get_enchant_power_bonus`, `event.level` names the level to use
for contextual reads, even if it is not the level in play. An explicit query may supply a state
that isn't at `pos` or positions outside a table's bookshelf scan. For the client particle
snapshot, `event.level` is `None` and both positions are the origin; answer from the state alone.

## Path classification

`BlockPart::block_path_type` answers for `event.state` at `event.pos`, with an owning
`&dyn LevelHost`. Return `Some(PathType)` to replace
the whole static classification, even for air, or `None` to use vanilla. The callback runs on
the server only, and no mob is supplied. The same state may answer differently by position.
Queries on the supplied level (such as `level.block_state(pos)` or looking up a block entity via
`level.block_entity(pos)` and `level.block_entity_data(entity)`) are strictly read-only: they never
perform writes, random draws, chunk loads or lazy loot generation. `level.block_entity(pos)`
returns `None` when no block entity is present, the chunk is unloaded, or `pos` is outside the build
height.
`level.block_entity_data(entity)` returns validated saved data without identity metadata, or
`None` for stale generations and handles from another level or world.

This is separate from `is_pathfindable`, which remains state-only. The server caches the final
static classification in 4096 direct-mapped positional slots shared by its AI workers, including
`Open` for missing navigation-region chunks. A collision replaces a slot. A matching
`sendBlockUpdated` notification clears it: ordinary writes require an actual state change,
`UPDATE_CLIENTS` and full chunk status at least `BLOCK_TICKING`; direct block-entity notifications
clear even unchanged state. Time, `/reload`, chunk load and chunk unload do not clear it, and
there is no neighbour invalidation. Keep the answer deterministic for that lifetime. Neighbour
and mob-specific adjustments are not stored in this cache; existing per-search evaluator caches remain.

`BlockPart::adjacent_block_path_type` answers for a neighbour at its own `event.pos`;
`event.path_type` is that neighbour's classified type, not the node's type. Return `Some`
to end the scan immediately, even if the answer equals the node's current type. Return
`None` to retain that neighbour's fluid and vanilla contribution. Neighbours are visited
in x, y, z order, each from -1 to 1, excluding the node's own column. No mob is supplied
to either query. A panicking callback is contained and falls back to the default.
Vanilla water contributes through its classified path type, so a solid waterlogged
block remains `Blocked` rather than producing `WaterBorder`. Custom fluids keep
their default `WaterBorder` contribution. This deliberately differs from NeoForge's
raw-water default to preserve exact vanilla behavior for vanilla content.

## Connections

`BlockPart::can_be_connected_to` lets the block beside a fence, a wall, or iron bars or a pane
answer whether that connector connects to it. The event names your block's `pos` and `state`, the
`connector` state, its vanilla rule (`kind`), the `direction` from your block toward the connector
and `face_solid`, whether your block's face toward it is sturdy. Return `Some(true)` to connect,
`Some(false)` to refuse, or `None` to keep vanilla's rule.

| Rule | Behavior |
| --- | --- |
| Order | Vanilla's rule, then your block's answer, then the connector's own `connects_to`, which starts from that answer and has the last word. |
| Calls | When the connector is placed and each time it settles its shape toward your block, on authoritative servers. |
| Walls | A connected wall side is still low or tall from the block above it. |
| Panics | A panicking part is contained with a warning, and vanilla's answer is used. |

Answer from the event without touching the world: the level host is read-only.

## State changes

`BlockPart::on_block_state_change` hears a block change from `event.old` to `event.new` at
`event.pos`, at the end of a `Level.setBlock` that held. The parts of the old state's block hear
it, then, when the block changed, the parts of the new state's block; `event.state` names the
state whose parts are asked. The host is a read-only `LevelSide`.

| Rule | Behavior |
| --- | --- |
| Moment | After the client update, the neighbour updates and the neighbours' shape updates: a fence beside new stone already shows its connection. A write made inside those updates is heard after the updates it queued itself, so its neighbours are settled too; several such writes are heard in the order they happened. |
| Replaced writes | A write whose `on_place` replaced the block is not heard; the write that replaced it is. |
| Sides | Servers hear every changed write with `event.predicted: false`. Clients hear server updates, applied local predictions (including breaks and placements), settled corrections and piston replays. |
| Predictions | Applied local writes are heard immediately with `event.predicted: true`. Server updates at predicted positions stay held and unheard until acknowledgement. A confirmation of the same state has no second event; a denial is heard once from the predicted state to the server's with `predicted: false`. |
| Not heard | Equal-state writes, unapplied guesses made while deciding an action, and world generation's writes. |
| Cost | Without a listener, one flag check; no event or level adapter is built. |
| `block_changed` | The global server hook hears a change earlier, right after the state is written, and also for replaced writes. |
| Panics | A panicking part is contained with a warning. |

## Placement writes

`BlockPart::on_place` is vanilla's `BlockBehaviour.onPlace`, heard on the server for every write of a
new state: a player's placement, `/setblock`, `/fill`, a structure, a piston. Only the parts on the new
state hear it, and `event.old_state` and `event.moved_by_piston` say what it replaced and whether a
piston wrote it. It gets the level writer `neighbor_changed` gets, so a part can read its signals and set
blocks in the call, as a lamp switches to its lit block the moment it is placed beside power; the write's
neighbour updates run as vanilla's do. It runs before the block's own `onPlace` handling.

## Rotation

`BlockPart::rotate` answers how a structure template turns your block as it places it: world
generation's structures, `/place template`, the structure block, jigsaw blocks and the host's
template snapshots. `event.state` is already mirrored (see [Mirroring](#mirroring)), `event.pos`
is where it goes and `event.rotation` the template's turn. `event.rotated` starts at the game's own turned state; set
it to another state of the same block, for example
`context.state_with_property(event.state, "facing", "north")`. Read the value to turn with
`context.state_property(event.state, "facing")` (vanilla's `state.getValue(FACING)`), an owned
`String`, or `None` for a key the state does not have.

Jigsaw connector discovery is different from block placement: matching and expansion read
the connector's plain rotated state, without calling `BlockPart::rotate`, as
`StructureTemplate.getJigsaws` does in vanilla. The template palette still depends on the read
position: the origin for candidate matching, the piece's real origin for expansion. Named
start anchors likewise use the palette at the position where they are sampled. The actual
template block writes still call the rotation Part at their placed world positions.

| Rule | Behavior |
| --- | --- |
| Context | A worldgen Part method: a `WorldgenHost` on the worldgen workers or the server thread, no `Host`, no writes. Keep it a pure function of the event. |
| Refusals | A state of another block, or one the game does not have, is refused with a warning; the game's state is placed. |
| Mirroring | `BlockPart::mirror`, asked before this one: see [Mirroring](#mirroring). |
| Panics | A panicking part is contained with a warning, and the game's state is placed. |

## Mirroring

`BlockPart::mirror` answers how a structure template mirrors your block as it places it, as a block
overriding vanilla's `BlockBehaviour.mirror(BlockState, Mirror)` does: world generation's structures,
`/place template`, the structure block and the host's template snapshots. `event.state` is the
template's state, `event.pos` is where it goes and `event.mirror` the template's mirror
(`LeftRight` flips north and south, `FrontBack` east and west). `event.mirrored` starts at the
game's own mirrored state; set it to another state of the same block, for example
`context.state_with_property(event.state, "facing", "south")`. `BlockPart::rotate` then turns the
state you answer. A template asks it for every block, `StructureMirror::None` included, as
`StructureTemplate.placeInWorld` calls `mirror(NONE)`; the legacy structure pieces ask it only when
they mirror. Jigsaw connectors are never mirrored. Context, refusals and panics follow the
[rotation](#rotation) rules.

## Step effects

`BlockPart::step_on` is the block's `Block.stepOn` callback. It runs on the `getOnPosLegacy`
state of a grounded entity affected by blocks, after the move's restitution and block speed factor
but before the entity's inside-block effects. An idle grounded item still steps even when it does
not move. Each effects pass has its own callback; this is not a landing notification.

Register the Part on the block. `StepOnEvent` carries the contacted `state` and `pos`, the entity id
and kind, whether it is `stepping_carefully`, its world-space `bounds` as `[min, max]` XYZ corners,
and its current `movement` (velocity). Edit `movement` to change what the rest of this tick sees.
Non-finite edits are ignored; there is no veto. Keep movement answers consistent on both sides and
on replay.

`LevelRandomSide::Client` lends a read-only level. Only the server arm accepts writes, and it
lends the same level random host as `fall_on`. `event.write_entity(EntityWrite::Hurt { damage_type, amount })` applies damage to the stepping body through its own damage path, before the inside scan.
For a magma-like Part, check `level.entity_type_is_living(event.kind) == Some(true)` and
`!event.stepping_carefully` before requesting damage. The entity-type read is available on both
sides and includes living mod types. Damage itself is not limited to living bodies: other Parts
can still hurt dropped items.
Use `bounds` to test overlap with a surface that is smaller than the block. `event.request(LandingWrite::…)`
queues a write at `pos`. The `fall_on` drain applies the ordered block writes after the entity batch,
only if the block still holds the contacted state. An invalid state or non-finite movement or damage
refuses that Part's outputs. Other Parts' accepted writes and movement edits remain.

Boats, minecarts, armor stands and projectiles ignore `EntityWrite::Hurt` from both `step_on`
and `fall_on`. Experience orbs accept it only from `step_on`.

## Landing writes

`BlockPart::bounce_restitution` answers the block's coefficient for a qualifying downward collision,
including those of dropped items and other nonliving bodies. `event.pos` and `event.state` are the paired
`Entity.move` effect contact from `getOnPosLegacy`: its column follows support, but its cell can differ
from the one supporting the body. Set `event.factor` to edit the block's coefficient. For a nonliving
body the engine multiplies it by `0.8` in `f32`, then takes the greater of that and the body's own
bounciness. A zero answer is not a veto. Slow falls and contacts that suppress bounce do not ask.

This runs on whichever side simulates the move. Keep it a pure read of `LevelSide` and the event so
prediction and replay produce the same answer. It does not run for packet interpolation, projectile
impacts or a piston's direct slime launch.

When the collision actually bounces, the server runs `BlockPart::bounce_on` before it posts
`BOUNCE` and commits the returned movement. This is a separate callback, with the captured effect
state and vanilla's movement-affecting position (which can differ from the legacy position).
`event.fall_distance` is the distance stored after fall handling, so a body that has just landed can report zero.

`BlockPart::fall_on` runs at vanilla's landing moment on a read-only level random host.
The event exposes writable `event.distance` and `event.damage_multiplier`, not calculated
damage. Vanilla blocks supply their own pair: hay uses 0.2, beds 0.5, slime 0,
and pointed dripstone adds 2.5 to distance and uses 2.0. Other blocks start at 1.0.
The engine applies `floor((distance + 1e-6 - safe_fall_distance) * damage_multiplier * fall_damage_multiplier_attribute)`.
The safe fall distance attribute defaults to 3: halving distance is not halving damage.
A zero multiplier or distance within the safe fall means no damage or generic fall sounds.
Creative and fall-immune entities and items keep their immunity regardless of edits.

Request a deferred write with `event.request(LandingWrite::SetBlock(state))`, where `state`
is an API `BlockState`, or `LandingWrite::DestroyBlock { drop }`. The game checks the
complete write batch before queuing it. An invalid block key or ordinal refuses that
callback's writes and other outputs, including distance and multiplier; it never applies a valid prefix.
After the entity batch ticks, writes apply in request order if the block still matches
its original landed state. That guard runs once, so a first write can change the state
without suppressing the next write. SetBlock notifies neighbors and clients; DestroyBlock
preserves the requested resource-drop flag.

## Inside-block effects

`BlockPart::entity_inside` (version 4) runs on the server as the entity's sweep reaches the block.
Use `event.run_before` or `event.run_after` to queue an owned closure around an `InsideEffect` slot.
The closure receives a short-lived `&mut InsideEntityView` when the sweep flushes, even if no
`event.apply` was requested for that slot. Built-in effects use the same ordered slots, and
later callbacks stop when the entity dies. A dropped item sweeps once, after `step_on`.

```rust,ignore
event.run_after(InsideEffect::FireIgnite, |entity| {
    entity.hurt("minecraft:hot_floor", 1.0);
});
```

When migrating a v2 `EntityWrite::Hurt`, use the same registered damage-type ID you used for
`r.damage_type_key`. `hurt_with_source` can additionally take typed handles for the direct
entity and the causing entity (for example, a projectile and its shooter). An absent damage
type or stale source handle makes that hurt return `false`. Players, mobs and dropped items
use their live hurt path; a projectile's hurt path can mark it hurt for sync and still return
`false`.

`BlockPart::step_on` still queues `EntityWrite`, not closures. Its `EntityWrite::Hurt` still
takes a `DamageTypeKey` requested in `register`, and has no direct or causing entity.
Experience orbs accept hurt from `step_on` only. Neither callback adds a hurt path for armor
stands or vehicles.

## Storage changes

`BlockPart::contents_changed(host, event)` observes a world block's storage at `setChanged`,
with a `ServerHost`, typed `event.level`, `event.pos: BlockPos` and `event.state`. Attach it to the specific
blocks you observe, including vanilla containers; it has no global `Hooks` counterpart.
An automation block entity can watch another position without polling or attaching Parts to
every possible source block type. Its Part calls
`observer.watch_contents(host, source_pos, true)` on its live `BlockEntityRef`; false unregisters.
The source is in the observer's level and need not be loaded. Registration is idempotent and
joins the current transaction: abort restores the previous watches. An observer without a
subscribing BlockPart returns `ContentsWatchError::NotSubscribed`; a stale or unwritable
observer returns `Stale`. A block entity temporarily taken for its own tick can still register.

`event.observer` is `None` for the source's own Parts and `Some(observer)` for a watcher.
The event's level, position and state **always describe the changed source**, not the observer.
Source Parts run first, then a snapshot of watching observers in registration order, each one's
subscribing Parts in their usual order. Registration changes made in a callback affect the next
notice; an observer invalidated before its turn is skipped. A self-watch never duplicates the
source callback. Removal, replacement and chunk unload discard the observer's watches. Reload
alone does not restore them: the Part must call `watch_contents` again. Watches are runtime
state, never saved. Unloading the source does not cancel watches of its position.

The backing contents are already written, before dirty/comparator updates. The default `ITEMS`
and item `STORAGE` handlers coalesce their implicit notice per backing block at root commit,
for vanilla containers and engine-managed mod slots. During `menu_quick_move`, implicit notices
publish only after a committed move settles its source slot (including when the source is
emptied), and never after an unhandled or refused move. If the original backing container was
removed or replaced, neither its replacement nor watchers receive that notice. Explicit
`BlockEntityRef::set_changed` notifies immediately, even inside a transaction and for
unchanged contents; abort does not retract it. Each Part runs in its own transaction. Keep using `host.transaction` for additional writes:
a successful nested transaction merges into the Part's frame, but a panic in that Part discards
its writes without undoing the backing change that raised the notice. The next Part can still
run, and only the panicking callback is disabled for the session.
Direct writes such as `BlockEntityRef::set_item_slot` and writes to storage a mod provides itself
do not synthesize notifications; neither do rollback restores. A double container notifies
each component, not again for the other half's comparator refresh. A listener's same-block writes
still apply and mark storage dirty and refresh comparators, but do not re-enter or queue a second
notice, even during a quick move. Another block notifies normally.
Entity/menu placeholder coordinates are not block notices. Explicit notices may read tentative items:

```rust,ignore
{{#include ../../../../../crates/sunrise-client/src/cut_layout/game/blocks/machines/contents_changed_tests.rs:contents_changed_listener}}
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
| `None` | Vanilla's rule for this position if the owner's rule is not held: a fluid in `#minecraft:water`. |

Normally rain on the block above, session water tickets and dry-out work as usual. With no Part on the farmland the
scan is vanilla's. When the content owner's subscribed rule is [switched off](../disabled-content.md), only earlier
enabled Parts or an active session water ticket (tickets attributed to a disabled mod are cleared on disable) can wet
the block. Rain and dry-out are held, not trampling.
Bookshelf slot-occupancy flags and the last-interacted slot are transactional storage bookkeeping,
not notification effects. Aborting restores them with the items. Changed observers, comparator
ticks and data-send requests already issued still survive that abort.

## Climbing and scaffolding

`BlockPart::is_ladder` is asked for the block a living entity stands in, and `BlockPart::is_scaffolding` for the block a
player on a climbable stands in. Both fire on both sides, are `listen`, and must be pure functions of the event: set the
field to decide, and a block with no part keeps vanilla's answer.

| Part | Starts as | Effect of the answer |
|---|---|---|
| `is_ladder` | The block is in `#minecraft:climbable` | `true` climbs like a ladder, `false` refuses a tagged block |
| `is_scaffolding` | The state is `minecraft:scaffolding` | While sneaking, `true` lets the player keep sliding down the climbable, `false` stops the slide |

`is_scaffolding` is asked for players only; mobs never ask it. The player's id, the block its position is in and that
block's state are passed.

## Map colour

`BlockPart::map_color` sets the map colour a block shows at a position, as NeoForge's `IBlockExtension.getMapColor`
does. It is `collect`, server only, and read-only: set `color` to a `MapColor` id from 0 to 61, starting from the state's
own colour. Map updates ask it for the dirt and stone noise under a ceiling, for every step of the scan down from the
surface (0 keeps the scan going) and for the colour a scanned block, or its fluid's block, finally draws as. An answer
above 61 is refused: the block keeps its own colour, and the refusal is logged once per block.

## Fluid faces beside a block

Client `BlockPart` methods change how a fluid draws its faces against your block. They are `collect`: set the field to
decide. They depend on the block state and the fluid alone, so the client asks them on its own thread for every state
and fluid when it joins a world and on `ClientHost::refresh_render_visibility`, as it asks `block_render_visibility`,
and the meshing workers read the answers from a table. They never run per frame, and the level and position are not
arguments. `event.fluid` is the fluid as it is asked: a source at amount 8, or a flowing fluid at amount 7, not falling.
Water's and lava's flowing forms are fluids of their own, so a part can answer each.

`BlockPart::should_hide_adjacent_fluid_face` answers whether your block hides the face a fluid beside it draws toward
it. `event.direction` points from the fluid to your block, so `Direction::Up` is the top of a fluid under your block.
`hidden` starts false. Vanilla's tests still apply first: a fluid beside the same fluid, or behind a face that occludes
it, draws no face there whatever the answer.

`BlockPart::should_display_fluid_overlay` answers whether a fluid's side beside your block uses the fluid's overlay
sprite (water's `water_overlay`) instead of its flowing sprite. `overlay` starts with vanilla's test: true beside a block
of vanilla's `HalfTransparentBlock` class (glass among them) or `LeavesBlock` class, and for a content block, its
template's class. A fluid with no overlay sprite, such as lava, draws its flowing sprite whatever the answer.

With no part answering either method, nothing is asked, the tables are empty and fluids draw exactly as vanilla's.

## Hit and destroy particles

`add_hit_effects` and `add_destroy_effects` are client `BlockPart` methods, like NeoForge's
`IClientBlockExtensions` methods of the same names. Return `Flow::Cancel` if your part handled
these particles (including choosing to add none). `Flow::Continue` permits the usual effect.
With no part answering, the block state's normal terrain-particle flag decides.

A hit callback sees a non-air block with a visible render shape, after the optional mining sound.
It gets the hit face, position and state. A destroy callback receives the non-air state that was
destroyed, even when the position is already air or contains another block. An invisible block
can still handle destruction. Both callbacks receive the client level's random; neither controls
sounds.

Use `event.level.add_breaking_particles(host, pos, state, face)` for a normal hit crumb from an
explicit state and position, or `event.level.add_terrain_particle(host, state, source_pos, at,
velocity)` for an individual piece of debris. The debris uses the supplied state's material and
samples its tint at `source_pos`, which can differ from the spawn point. These helpers don't
redispatch the hit or destroy Part or play a sound, but can ask block tint sources. Read the
supplied state's outline with `event.level.visit_block_shape` if you need to sample it. The hit
helper uses the same random stream as `event.random`.
