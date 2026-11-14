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

`explosion_resistance(level, event)` answers `Some(resistance)` to replace the
block's own explosion resistance against `event.explosion`, as a ray of the
blast enters the block. The ray's next steps in the same block reuse the
answer, so keep it a pure function of the event. A fluid in the cell still
raises the value to 100, and the overrides vanilla applies afterwards keep
winning: a respawn anchor's water cell, a TNT minecart's rails and the blocks
under them (never asked) and a dangerous wither skull's 0.8 cap. A negative or
non-finite answer is refused with a warning and the block's own value is used.

`can_drop_from_explosion(level, event)` answers whether a destroyed block drops.
`Some(false)` skips its loot and its `spawnAfterBreak` (an infested block's
silverfish, an ore's experience); `Some(true)` drops even for TNT, the one
block vanilla keeps from dropping. The block is removed either way, and a TNT
still primes. Both queries get a read-only level host and the same
`ExplosionStart` as `on_block_exploded`; `None` leaves the answer to another
attached part and then vanilla.

Every method on this page routes through prepared per-block subscriptions.
Unsubscribed blocks build no callback context. A native `exp_drop` panic
unwinds before the engine applies its answer. A native
`on_block_exploded` panic is contained and reported as
`on_block_exploded callback failed: {error}`; block removal continues. A panic
in `explosion_resistance` or `can_drop_from_explosion` is contained the same
way, and vanilla's answer is used.

## Entities that destroy blocks

`can_entity_destroy(host, event)` answers whether `event.entity` may destroy the
block, where vanilla asks its per-block rule: the wither's block break (any
block but air and `#wither_immune`), a dangerous wither skull's blast (the
entity is the skull, still readable; only a block it may destroy has its
resistance capped at 0.8), the dragon's wall check, a zombie breaking a door
and a zombie choosing a turtle egg to trample. `Some(true)` makes a block
vanilla spares destroyable for that entity, such as bedrock for the wither;
`Some(false)` spares one it would break, and the dragon is then in a wall. The
griefing gate runs first, so with `mob_griefing` off nothing breaks whatever
the part says, and `living_destroy_block` can still keep a block the part
allowed (the turtle-egg choice fires none). For the dragon,
`#dragon_transparent` blocks are skipped before the part is asked, and a
`#dragon_immune` block stays even when the part says yes. A ravager asks only
the griefing event. The host is read-only. The dragon asks every
tick for each block in its head, neck and body, so answer fast. A panic is
contained and vanilla's rule is used.

## Pistons

`on_destroyed_by_push_reaction(level, event)` hears a piston's move destroy the
block (a `Destroy` push reaction, such as a torch's), after the block dropped
its resources and while it is still in its cell. `event.direction` is the way
the blocks move and `event.fluid` is the fluid state the cell holds. The removal still happens: the cell becomes air afterwards.
The level host writes and schedules ticks, but a write to any cell of the move
is lost. The destroyed cells turn to air, and the pushed blocks were read
before, so each moves as it was and the cells they leave and enter end as air
or a moving block, as does the arm. Writes outside the move stay. A move that
`piston_pre` cancels and a client's replay of a move are not heard. A panic is
contained and the move goes on.
