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

## Setting off an explosion

`level.explode(host, source, center, radius, fire, interaction)` is
`Level.explode(source, x, y, z, radius, fire, interaction)`, on the server, in
the level in place. `source` is the exploding entity (`Option<EntityRef>`). The
entity that caused the blast is never passed: it comes from the source, as
vanilla's `Explosion.getIndirectSourceEntity` has it.

| `source` | Causing entity |
| --- | --- |
| A primed TNT | Its owner (the player or mob that lit it), or none |
| A living entity (a player, a mob) | The source itself |
| A projectile | Its owner when that is living, else none |
| Anything else, or `None` | None |

With a source and a causing entity the damage is `minecraft:player_explosion`,
otherwise `minecraft:explosion`, and the causing entity gets the kill: a mob
killed by a blast whose source is player-lit TNT dies to that player, and one
killed by unowned TNT to no one. The source itself is not hurt or pushed by its
own blast, and the blocks it breaks roll their loot with it as "this entity". A
TNT block the blast reaches primes, owned by the causing entity.

`interaction` says what happens to blocks:

| `ExplosionInteraction` | Blocks |
| --- | --- |
| `None` | Stay |
| `Block` | Break, drops decay by `block_explosion_drop_decay` |
| `Mob` | Break when the source may grief (`entity_mob_griefing` for it; without a source, the `mob_griefing` rule), drops decay by `mob_explosion_drop_decay`; otherwise stay |
| `Tnt` | Break, drops decay by `tnt_explosion_drop_decay` |
| `Trigger` | Stay; buttons, levers, doors, trapdoors, fence gates, bells and candles react as to a wind charge |

`fire` lights about a third of the air above solid blocks in reach, whatever the
interaction, `None` included. The steps run in vanilla's order:
`explosion_start` (which may cancel it), the blocks the rays reach,
`explosion_detonate`, entity damage and knockback, the blocks, the fire, then
what players hear and see. A blast set off inside another blast's hook keeps
its own source and cause and leaves the other blast's as they were.

`explode` answers `false` and explodes nothing for a parked level or a stale
handle, a `source` that is stale, names no entity or is a player in another
level, a chunk that isn't loaded, or a radius outside `0 < radius <= 32`.

## Reading explosion resistance

Three reads tell a mod what a blast would meet without setting one off, as a
mod that casts its own rays and breaks blocks over several ticks needs.

`host.block_states().explosion_resistance(state)` is the value the block of
`state` declares (`Block.getExplosionResistance`): stone 6, obsidian 1200,
bedrock 3,600,000, water 100, a content block's `explosion_resistance`. It is a
Horizon addition, readable on both sides: vanilla only reads it at a position.
Parts and fluids don't change it. `None` for an unknown state.

`level.explosion_resistance(host, pos, source, center, radius, fire,
interaction)` is what the blast `level.explode(host, source, center, radius,
fire, interaction)` would meet in the cell at `pos`, on the server, by the rule
its rays use. The answer is typed, because no resistance is not a resistance of
0: a ray loses only its fixed step in a cell with no resistance.

| `ExplosionResistance` | When |
| --- | --- |
| `Resists(r)` | The larger of the block's answer (its parts' `explosion_resistance` first, then its own value) and the fluid's (water and lava 100) |
| `NoResistance` | Air with no fluid, or void air above and below the level |
| `Unavailable` | A stale level or `source`, a chunk that isn't loaded, or a state the game doesn't know |

A read never loads a chunk. The blast's calculator follows its source, as
vanilla's sources choose theirs:

- A wind charge's names the blocks of `#blocks_wind_charge_explosions`. **It
  replaces the whole rule**: those blocks resist 3,600,000 and every other cell
  has no resistance, a fluid included, and no part is asked.
- A primed TNT minecart's rails and the blocks under them resist 0.
- A dangerous wither skull caps a block it may destroy at 0.8.

A respawn anchor's water cell belongs to the anchor's own blast, which has no
source and no description a mod can give, so no read sees it. Parts are asked
with the `ExplosionStart` the blast would have: its causing entity and damage
type come from `source` as for `explode`.

`level.explosion_resistances(host, &cells, source, center, radius, fire,
interaction)` reads many cells in one call, one answer a cell in the order
`cells` gives:

| `ResistanceCells` | Order |
| --- | --- |
| `Box { min, max }` | Every cell, both corners included: x fastest, then z, then y |
| `Columns { columns, min_y, max_y }` | Each column in the order given, from `min_y` up to `max_y` |

A read of more than `ResistanceCells::MAX` (262,144) cells, or of an inverted
area, is refused with `ResistanceCellsError` and reads nothing. It runs on the
server tick, so read a big area in pieces. A part's `explosion_resistance` is
asked once a cell, unless the part declares its answer pure:

```rust
#[mod_api::horizon_part]
impl BlockPart for Reinforced {
    fn explosion_resistance(&self, _: &dyn LevelHost, _: &mut ExplosionResistanceEvent<'_>) -> Option<f32> {
        Some(5000.0)
    }
    fn explosion_resistance_is_pure(&self) -> bool {
        true
    }
}
```

Then a batched read asks it once a state per call. Declare it only when the
answer depends on nothing but the state and the explosion; either way the
answer stays a pure function of its inputs, since a ray asks once as it enters
a block.

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
