# Typed bulk block edits

Use complete `BlockState` values in `BlockEdit.palette`. Construct them with
`BlockState::of(&host.block_states(), block, constraints)` (or change an existing state
with `with`). `filters` contains owned `StatePredicate` values. `StatePredicate::block`
validates property names and values strictly; `StatePredicate::tag` retains arbitrary
property requirements, including for an empty tag, and rejects duplicate properties.

A predicate requires block identity or tag membership and every supplied property.
Omitted properties are wildcards, never default-state requirements. An unknown or
missing property on a tag member never matches. Sparse, non-Cartesian state tables
are supported: a partial predicate does not need to form a complete state with defaults.

All palette entries, predicates and run indices are prepared before any mutation.
NBT is not representable. At most 65,536 filters are allowed. Unloaded chunks and
out-of-height portions of runs are skipped. Edits preserve strict block-entity behavior,
notify no neighbors and send whole changed chunks.

## Read column palettes

`ColumnBlocks.palette` also contains complete `BlockState` values. States of the
same block with different properties have distinct entries. Only distinct numeric
states are converted to API values, not each block in the returned columns.

#### Upgrading saved block entities and entities

Data a mod reads from an older source (a schematic, a structure file, an old world's region) carries the
`DataVersion` it was saved at. `host.upgrade_block_entity_nbt(data_version, nbt)` and
`host.upgrade_entity_nbt(data_version, nbt)` run the game's own datafixes on one such value, the ones the chunk
loader runs on a block entity of a chunk of that version and the entity chunk loader on an entity (its passengers
included), up to the game's `DataVersion`. Both are server-side and pure: they read nothing from the level and change
nothing in it.

- Data at exactly the game's version comes back unchanged, and so does a block entity whose `id` its version
  doesn't know (as a chunk keeps one) and data the fixes can't read. Version 0 is a valid source.
- The fixes that need a whole chunk don't run: those that look at the blocks around a block entity, and the one that
  removes bed block entities.
- `Err` names a refusal, made before any fix runs and leaving nothing changed: a negative version; a version newer
  than the game's (the fixes only go forward); a list or compound nested 512 levels deep or more, the depth cap of
  every NBT read; more than 1 GiB as NBT, the chunk loader's cap on saved data. There is no node cap, as saved
  chunks have none, so nothing a saved chunk could carry is refused.
