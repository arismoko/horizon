<!-- chapter: Block entity attachment -->

`block_entity_on_load` runs once for each newly attached mod block entity,
including fresh placement, saved chunk loads, and client chunk reception.
It follows `IBlockEntityExtension.onLoad`. Dispatch is keyed by the block
entity type id. The event supplies a `BlockEntityRef`; use its data methods
with the supplied `HostSide` to read the saved data already applied to it.

Attachment notices are drained before block entity ticking on each logical
side. Payload writes to an existing identity do not fire again. Unloading
and loading a chunk creates new identities, which receive new notices.
Vanilla block entities do not receive this hook.

Each drain handles only the notices pending when it began. A listener that
replaces a block queues the replacement for a later drain. If the pre-tick
drain replaces the entity about to tick, that tick waits; the replacement
receives its own notice first. Worlds without subscribers collect no notices.
