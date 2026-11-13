# Block experience and explosions

Attach a `BlockPart` to the blocks that own the behavior.

## Experience

`exp_drop(level, event)` answers `Some(points)` to replace the vanilla experience
sample, before enchantments. `None` leaves the answer to another attached part
and then vanilla. The query receives the dimension, state, position, optional
block entity, breaker entity handle and tool. The level host reads without writing;
`level.block_entity_data(entity)` reads the supplied entity's saved data without
identity metadata and refuses stale handles. Answers are clamped to 0 through
32,767. Player breaks supply the part's answer to the global `break_block`
event's `exp_to_drop`, after Silk Touch suppresses the starting amount (except
for spawners). An explicit global override is applied as given. `block_drops` sees the resulting amount afterwards.
Level and mob breaks use the shared resource-drop path. A canceled player break
or a break that does not harvest produces no award.

## Explosions

`on_block_exploded(host, event)` runs before the engine removes each destroyed
block. The server host permits writes. `event.block_entity` is a live handle,
and `entity.data(host)` can read it while the block remains. The event borrows
the same `ExplosionStart` description as the global start event: dimension,
center, radius, fire, interaction, damage type, source and owner.

The engine writes air after the callback, including when the callback
replaced the block. Vanilla's separate `wasExploded` behavior follows removal.
Air, spared blocks, canceled explosions and explosions that keep blocks produce
no callback. The initial explosion experience amount is zero.

Both methods route through prepared per-block subscriptions. Unsubscribed
blocks build no callback context. A native `exp_drop` panic unwinds before the
engine applies its answer. A native
`on_block_exploded` panic is contained and reported as
`on_block_exploded callback failed: {error}`; block removal continues.
