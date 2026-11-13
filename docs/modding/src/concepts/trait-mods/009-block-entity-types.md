<!-- chapter: Block entity types -->
<!-- separator: 1 -->

Declare a block entity type and its valid blocks during `Hooks::register`, then attach its part:

```rust,ignore
let press = r.block_entity_type("press")
    .blocks(&["press", "mymod:big_press"])
    .build();
r.block_entity_part(&press, Press).expect("native block entity type");
```

The builder qualifies the type ID and each bare block name with the registering mod's namespace, so
`"press"` above is `mymod:press`; the blocks may be declared later in `blocks`. It owns the save ID, wire type
and valid blocks; `block_entity` returns only initial `Compound` data (or none for an empty compound). It is fired
inside the write that creates the entity (`LevelChunk.setBlockState`) and receives the read-only level, a `LevelSide`,
on a server and on a client of one.
Unknown blocks, duplicate types, overlapping membership and attempts to own a `minecraft:` block are
refused at load. Refused native block, item or block entity declarations stop startup, as does unreadable
known-components input; the game does not fall back to vanilla content. An explicitly declared mod
template block uses its declared type; undeclared template blocks retain their vanilla behavior. Adding mod blocks to vanilla types is not supported by this builder.
The hooks about the entity (`block_entity_tick`, `block_entity_update_tag`, `pre_remove_side_effects`,
`on_chunk_unloaded`, `menu_button`, `menu_quick_move`) find it by that declared type; the others (`block_entity`, `screen`, the
block hooks) by block. The factory is a `BlockEntityPart` method, not a global `Hooks` method.

`r.block_entity_type_key("other:press")` looks up another mod's type, and
`r.block_entity_type_key("minecraft:chest")` looks up a vanilla type. Both return the same opaque
`BlockEntityTypeKey` as the declaration builder: equality and hashing use the shared load-owned
registration slot, not the type's blocks. Bare names use the `minecraft:` namespace. The final
registry resolves requests after all declarations and refuses unknown IDs, naming the requesting mod
and type. `key.id(host)` reads the canonical name for diagnostics; a key retains its identity until
restart. Use a type ID, not a block ID (`minecraft:shulker_box`, not `minecraft:red_shulker_box`). `BlockEntityRef::kind(host)` returns `Option<BlockEntityTypeKey>`.
`living.hurt_and_break(host, slot, amount)` changes equipped durability on the server; negative amounts repair. It returns the pre-shrink stack only on break, and `None` when nothing breaks or the entity or slot is unavailable. Creative players' equipment is exempt. The operation does not fire equip effects.

`block_entity.replace_data(host, data)` replaces its saved payload, clearing omitted fields while retaining
identity and position. Like `merge_data`, it loads transactionally and synchronizes the change; it returns
`Err(BlockEntityError::Stale)` for an invalidated handle, or `Ok(false)` for data the entity cannot
load or an unchanged result.
Lookup keys may answer capabilities for vanilla and native types. `block_entity_part` accepts only
native types and returns `BlockEntityPartError::VanillaType` immediately for a vanilla key. Native
Parts are expanded after every mod has registered, so a Part can target a type declared later.

## Transform saved data with a structure

Add an optional `.transform(...)` step to the declaration when the saved payload contains a facing
or relative positions that must turn with a structure. The same declaration works for native and
wasm mods:

```rust,ignore
use mod_api::{Compound, StructureMirror, StructureRotation, StructureTransform, Tag, TransformRefusal};

let press = r.block_entity_type("press")
    .blocks(&["mymod:press"])
    .transform(|data: &Compound, transform: StructureTransform| {
        let facing = match data.get(&"facing".into()) {
            Some(Tag::String(value)) => value.to_string()
                .map_err(|_| TransformRefusal::new("invalid facing text"))?,
            _ => return Err(TransformRefusal::new("missing facing")),
        };
        let mirrored = match (transform.mirror, facing.as_str()) {
            (StructureMirror::LeftRight, "north") => "south",
            (StructureMirror::LeftRight, "south") => "north",
            (StructureMirror::FrontBack, "east") => "west",
            (StructureMirror::FrontBack, "west") => "east",
            (_, facing) => facing,
        };
        let directions = ["north", "east", "south", "west"];
        let index = directions.iter().position(|facing| *facing == mirrored)
            .ok_or_else(|| TransformRefusal::new("unknown facing"))?;
        let turns = match transform.rotation {
            StructureRotation::None => 0,
            StructureRotation::Clockwise90 => 1,
            StructureRotation::Clockwise180 => 2,
            StructureRotation::Counterclockwise90 => 3,
            _ => return Err(TransformRefusal::new("unknown rotation")),
        };
        let mut result = data.clone();
        result.insert("facing".into(), Tag::String(directions[(index + turns) % 4].into()));
        Ok(result)
    })
    .build();
```

The step runs on the authoritative server during structure-template placement and worldgen,
possibly on worldgen worker threads. The block state has already been mirrored and rotated; the
step returns the saved data to store. Mirror applies before rotation. Its only inputs are the data,
rotation and mirror: it gets no `Host` or world access. Determinism is the mod's obligation, not
something the engine enforces; any matching `Send + Sync + 'static` closure is accepted. Avoid
randomness, clocks and mutable external state when calculating the result.

The engine restores the original `id` and any original `x`, `y`, `z` fields over the result, so a
payload transform cannot change the saved entity's identity or original coordinates. With no step,
data is unchanged. A `TransformRefusal` logs the type and position and keeps the original data;
placement still succeeds. Invalid output exceeding standard NBT limits (64 MiB, one million nodes,
depth 512), native unwinding panics and wasm traps also keep the original data.

Callbacks are registered at startup and frozen with the block registry snapshot. A data-pack
`/reload` keeps that registry and its existing callbacks; it does not register replacement code.
A newly prepared registry receives its own callback snapshot.
