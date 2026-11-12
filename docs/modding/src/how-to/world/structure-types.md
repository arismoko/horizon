# Structure types

A structure type is a mod's code where a data pack names one: a structure JSON whose `"type"` is
`"mymod:tower"` instead of a vanilla type generates by the mod's `StructureType` (vanilla's
`StructureType`, whose `find_generation_point` returns the start and whose pieces are
`StructurePiece`s, loaded by the `StructurePieceType` that `load_piece` is — NeoForge registers
these in the registries `STRUCTURE_TYPE` and `STRUCTURE_PIECE`). Register it once, at startup, in
`Hooks::register`; the id is in the mod's namespace, and a duplicate id, an id in another mod's
namespace or an empty name stops the game at startup with a message naming the mod:

```rust,ignore
{{#include ../../../examples/src/worldgen.rs:structure}}
```

There is no `Host` in world generation: starts build on worker threads and pieces place per chunk,
so the type and its pieces see only two small contexts, `Send + Sync` and deterministic. The same
seed gives the same pieces, and a type keeps no mutable state, because two workers can build
neighbouring chunks at once.

`StructureType::find_generation_point(&self, context: &mut dyn GenerationContext)` answers
`FoundStructure { position, pieces }` (each piece an `Arc<dyn StructurePiece>`) or `None` for "no start in this chunk". The
`GenerationContext` is vanilla's `GenerationContext` over one chunk: the seed, the chunk, the
random seeded as vanilla seeds it for the chunk (`setLargeFeatureSeed`), the height and
base-column queries, opaque noise-biome identities and the valid-biome test, borrowed immutable
`metadata()`, and the structure's config as a shared `Arc<Tag>` (`context.config()`). JSON converts
once when the definition loads or reloads, not once per start. Nested nulls refuse at load. Both contexts are traits and the random is a
`StructureRandom` (`next_int_bound`, `next_long`, `next_float`, `next_double`, `next_boolean`),
so a type built to `mod.wasm` runs the same code. Generated worker calls take the dependency
group's store lock and per-call fuel or epoch budget; a trap stops that group with a typed refusal. Everything around the type stays the game's: the structure set's
spacing, the structure's `biomes` and `terrain_adaptation` are placed by the engine, exactly as
for a fortress.

Each piece is a `StructurePiece` (`Debug + Send + Sync`): `bounding_box`, `post_process` and
`add_additional_save_data` are required. After all pieces of the start were placed, the type's own
`after_place` (vanilla's `Structure.afterPlace`) runs once per chunk with every piece's current
box; it defaults to doing nothing. The
`PieceContext` of `post_process` and `after_place` is the chunk being placed: `set_block` writes a
block and drops a position outside the writable box silently, as vanilla's
`StructurePiece.placeBlock` skips it; `block` and `height` read, `state`/`state_with` resolve a
a registered `BlockKey` into an API-owned `BlockState`, `random` is the piece's `RandomSource` as a
`StructureRandom`, and `metadata().name(state.block())` borrows its canonical block name.
`solid_render` asks whether
a state draws as a solid block, and `entity` queues an entity from its saved NBT — its `id`, `Pos`
and fields — which the generating chunk holds and the game finishes as a structure mob's
(`Mob.finalizeSpawn` with `STRUCTURE` when the NBT keeps the `"horizon:finalize"` byte a structure
template's entities carry). `block_entity` places a block entity from its NBT.

Saving goes through the engine's `StructurePiece.createTag` — the type id, `BB`, `O`, `GD` —
around the piece's `add_additional_save_data`. When the chunk loads back, the type's `load_piece`
(the `StructurePieceType` loader) rebuilds the piece exactly as it was saved; there is no
regeneration from the seed. `load_piece(metadata, tag)` returns a typed `StructureError` for invalid
saved block names, properties or piece data. Its metadata is the same retained snapshot the worker
contexts borrow. Resolve states once in piece construction or load, as the example does; keep
canonical names and properties in saved data, never process-local ordinals. `post_process` then
writes retained states without per-block name lookups or allocation.

**Open question — the saved piece's id is the structure type's, not a piece type's.** Every piece
of a type saves with that type's id, the `STRUCTURE_TYPE` id; Horizon has no separate
`STRUCTURE_PIECE` registry, so there is no per-piece-kind id in the tag. A mod whose type has
several kinds of pieces tells them apart in `load_piece` by its own saved data (a tag field its
`add_additional_save_data` writes). The `O` (orientation) and `GD` (genDepth) fields are written
as `-1` and `0` — the adapter has no orientation or depth to record — so a piece must not read
them back as meaningful.

The pieces ride the one piece mechanism the game has: they are legacy pieces behind an adapter, so
the start cache, the cross-chunk placement, the beardifier and `/locate structure` treat a tower
piece like a fortress piece's. `StructureRegistry` reads the registered types before it falls to
`Unsupported`, so a data pack structure whose `"type"` no installed mod registers places nothing,
exactly as before.
