# Piston push reactions

`BlockPart::piston_push_reaction(event)` returns `Option<PushReaction>` for
`event.state`, without a Host. `None` preserves the block properties. Values
are `PushPull`, `Push`, `Popped`, `Immoveable`, and `IgnoreEntity`.
Answers must depend only on state and agree on both sides. They are resolved
when parts are installed at load and reload; simulation and client replay
read one cached answer. Removing the last subscriber restores the properties.

The order is the reaction first, then the shared block-entity rule for any
reaction that would move the block: push-pull, ignore-entity, and push only.
Unbreakable hardness, immovable reactions, and push-only in the wrong direction
refuse before `relocability` is asked. `relocability` is asked
only for an otherwise-pushable block. For one with a block entity, only
`Some(true)` permits movement; `None` and `Some(false)` refuse it.
`BlockPart::relocability(level, event)` is NeoForge's
`IBlockExtension.getRelocability`, read as a yes or no for pistons: `Some(false)`
also keeps a block without a block entity in place. It is asked on both sides,
as a client replays a move, so answer the same on both.

The carry machinery takes the entity and attachments out before a
source is overwritten, holds them in the moving piston (including mid-move
saves), and installs them at the destination without inventory drops.
For example, a chest with 27 slots and seven diamonds in slot 4 remains intact
at its source when push only has no `relocability` answer. With `Some(true)`, all seven
diamonds remain in slot 4 at the destination, with exactly one block entity.
Push-pull with `Some(true)` follows the same rule; push-pull alone is refused.

**Diverges from vanilla: push only never discards a block entity**.
`Popped` destroys the block through the normal drop and removal
paths without asking `relocability`. The chest drops its seven diamonds once,
and no block entity travels. The destruction callback and entity push-reaction
query remain separate APIs.

Position-owned names, lock/loot/source fields, container limits and hopper cooldowns
travel in the carried payload, including saves during movement. Open menus close;
opener counts, animation state and render membership are invalidated rather than
retargeted. The destination becomes a fresh ticker holder. A vanilla entity whose
state exists only in raw saved data or a separate circuit store (for example a
comparator), and any payload borrowed by its ticker, is refused:
there is no live owned payload to transfer safely.
