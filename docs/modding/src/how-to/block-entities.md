# Declaring a machine's menu data

A mod block entity has two different kinds of storage: external item access uses
transactional Parts, while internal slots use the handle methods below; integer menu data is read with
`BlockEntityRef::data_slots` and changed on a server Host. Menu data is transient:
creation and loading both start with an empty data vector, even when the saved
payload still contains the machine's progress.

## State-aware creation

Declare the block entity type and its owning blocks during registration. To make
creation depend on block properties, answer `stateful_block_entity` with true for
that block. The existing `block_entity` hook then receives the actual placed
state, not just the block identity. Read its properties through the Host's state
registry and supply the initial saved compound. The registered declaration owns
the entity type; an unanswered state-aware creation makes no entity.

Creation runs inside the block write, after placement side effects, on servers
and clients. Changing properties within the same block preserves an existing
entity even when a fresh instance of the new state would have no entity.

## Size the data vector

Call `entity.set_data_slots(host, initial_values)` with the complete initial
vector when preparing a newly created or loaded machine. For a progress display,
use one element initialized from the progress value in its saved payload. This is
the live-handle replacement for assigning a mutable tick payload's `data` field.
Do not reset the vector to its initial values on every tick.

The method accepts at most the game's mod-menu data-slot limit (currently 256).
It returns `Ok(true)` when it changes the vector, `Ok(false)` for identical values,
or `BlockEntityDataError::TooManySlots` with the submitted count and limit. It
never silently clamps a vector. `BlockEntityDataError::Unavailable` means the
handle cannot reach a writable mod holder in the server level currently in place.

## Update individual values

After sizing, call `entity.set_data_slot(host, index, value)` for scalar updates.
This method does not resize: an absent index returns `Ok(false)`. Both methods use the
same write target, journal and `setChanged` path. Nested transactions retain their
ordinary behavior; an aborted outer transaction restores the vector that existed
before its first write, including writes committed by children. The normal menu
broadcast delivers committed changes to viewers.

The public handle resolves one entity's lifecycle generation as well as its level
and position. Removal, replacement and chunk reload make it stale; in-place data
edits preserve it. It is not a borrowed-payload API.

## Internal items versus automation

For a configured mod container, `item_slot(host, slot)` snapshots an internal slot;
`set_item_slot(host, tx, slot, stack)` replaces it within an existing server
transaction. Any held mod block-entity handle can be used; there is no caller
ownership check. This deliberately bypasses inputs, outputs and face policy, so a
recipe can consume its input and produce its output atomically, including output
slots that automation cannot insert into. Use the `ITEMS` capability for external
insertion; its policy is unchanged. Vanilla containers are unavailable here.

Indices use the current container's declared count. Lazy slots read empty; saved
excess slots are neither exposed nor truncated. None clears, identical values
return `Ok(false)`, and real changes send Changed at root commit. Nested abort
restores ordinary before-images, including committed-child writes. Invalid
indices, unknown/air/zero-count stacks and counts above the registered or
component-overridden maximum return `BlockEntityItemError`, never a partial write
or silent count clamp. A stale handle refuses a replacement rather than accessing
its container; acquire a new handle for the replacement.

## One-shot client effects

To tell clients about something that happened once, such as a burst of particles, queue
fields instead of writing them to the saved `nbt`. In `block_entity_tick`, or another
event with a mutable entity, native and wasm mods alike call
`event.entity.set_client_update_fields(fields)`, then `host.block_entity_sync(event.pos)`.
A later call replaces the batch; an empty compound clears it.

The next block change that reaches a viewer puts the fields over what
`block_entity_update_tag` gives, or sends them alone without one, in one packet for every
current viewer, and the batch is gone. A change nobody sees leaves it waiting, and neither a
chunk sent to a player nor a resend of the blocks under a single player takes it. Removing
or unloading the block entity drops it. It is never saved, cloned or compared, and it does
not change the entity's revision.

A client merges the fields into its copy's `nbt` as any update field, and they stay there
until sent again: put a counter in them to tell one effect from the next.

## Retaining a handle

A `BlockEntityRef` names one live entity, not every entity later placed at its position. Removal,
replacement and chunk reload invalidate it; reacquire with `host.block_entity(level, position)`.
In-place data edits and temporary tick storage preserve identity. Option reads return `None` for
stale handles; writes return their error's `Stale` variant, never writing to a replacement.
`set_lock`, `set_name`, `merge_data`, `replace_data`, `set_changed` and `set_data_slot`
return `Result<bool, BlockEntityError>`: handle `Err(BlockEntityError::Stale)` by dropping
or reacquiring the handle. `Ok(false)` means unchanged data or an unsupported operation,
not a stale identity. Existing item and data-vector methods retain their specialized errors.

## When its chunk unloads

A machine that keeps something outside its saved data (a running job, an open handle, its place in an
energy or pipe network) lets go of it in `on_chunk_unloaded`, NeoForge's `onChunkUnloaded`: a
`BlockEntityPart` on the type is told once for each of its block entities in a chunk that unloads, on each
side that holds a copy, before the copy is dropped.

```rust,ignore
struct Node;
#[mod_api::horizon_part]
impl mod_api::BlockEntityPart for Node {
    fn on_chunk_unloaded(
        &self,
        host: mod_api::HostSide<'_>,
        event: &mut mod_api::OnChunkUnloadedEvent<'_>,
    ) {
        if let mod_api::HostSide::Server(_) = host {
            // The handle equals the one `host.block_entity` gave while it was loaded.
            NETWORK.lock().unwrap().remove(&event.block_entity);
        }
    }
}
// In Hooks::register:
// registrar.block_entity_part(&node_type, Node)?;
```

`event.pos` is where it stands and `event.entity` its data at the call: on a server what the chunk was
just saved with, on a client its copy as the server last sent it. On a server the chunk is saved before
the call, so a change made in it is not kept, and the chunk may already have left the level, so its
handle's reads find nothing there. It is not called when the world closes, nor on a client when its whole
level goes, and never for a vanilla block entity. The chunk-wide notice is still `native_chunk_unload`, a
server hook told after the chunk's block entities; a client is told of the block entities alone
([hook notes](../reference/hook-notes.md#when-a-block-entitys-chunk-unloads)).

`player.open_menu(host, entity)` also returns `Result<bool, BlockEntityError>`:
`Ok(true)` means opened, `Ok(false)` means a non-provider, cancelled open or
unconnected/wrong-level player, and `Err(BlockEntityError::Stale)` means the entity
no longer resolves. It never opens a replacement through an old handle.

### Receiving synchronized data

Implement `block_entity_data_received` on the block entity's part to react on the
client after its synchronized tag has merged. It receives a `BlockEntityRef` and
the arriving compound; reads through the handle already see the new data. This
listen notice carries `BlockEntityDataSource::Chunk` or `UpdatePacket`. A chunk
with an empty compound sends no notice. An update packet with an empty compound
sends one notice; a packet with no tag sends none. Repeated identical packets
each notify, and the event contains only the arriving fields, while handle reads
see the merged copy.
It cannot change the arriving tag. Vanilla entities, missing tags, missing client
entities and mismatched types do not call it. Server tag production still uses
`block_entity_update_tag`.
