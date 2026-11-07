<!-- chapter: Registry keys -->
<!-- separator: 1 -->

On `r: &mut dyn Registrar`, `r.block_key(id)` and `r.effect_key(id)` request registry entries,
not new content. `Registrar` is the generated union of the kind-local registration traits
(such as `BlocksRegistrar` and `EffectsRegistrar`); mod registration hooks and the
[horizon-machine-sample](https://github.com/arismoko/horizon-machine-sample) continue to
use `dyn Registrar`. Each kind owns its declarations and implementation in `parts/<kind>.rs`;
the union and WebAssembly bindings discover those traits without a shared method list.
The capability-provider lookup union is assembled the same way. Declare them in `register`, retain the returned `BlockKey` or `EffectKey` in the
mod or Part, and reuse them for registry operations. Requests are canonicalized (a bare id
uses `minecraft:`), deduplicated across mods and checked after every mod's content is known.
An unknown entry refuses the load with the requesting mod and canonical id, even if the
key was never attached to a Part. A later mod may supply the requested entry.

Keys are small, private `u32` registration slots on both native and wasm. `key.id(host)`
returns an owned canonical `String` through the generated read backend for diagnostics; it
requires a game-backed host and an invalid slot is a boundary error, not an empty name.
Normal operations use the slot's resolved numeric mapping instead of looking up its name.
Block reload rebinds that mapping without changing the key's identity or canonical name;
a refused rebind clears the old mapping. Effect registration is process-owned and fixed
until restart, matching the engine's effect registry.

Block tag membership is bound from the same merged item/block tag snapshot at world load and
reload, including worlds without pack overrides. A read maps the slot to a numeric block id,
looks up the tag name and tests a `BlockSet` bit; it never resolves the block's name. A fully
namespaced tag needs no allocation (a bare tag adds `minecraft:`). Binding visits each merged
block-tag member once, resolves its default state and inserts its numeric block id; it replaces
the previous snapshot rather than accumulating membership. No membership work is added to ticks
that do not call this read. Tags and block mappings belong to their resolving thread; missing
authoritative context or an unavailable mapping returns `None`, not a stale membership result.

`host.block_state_default(stone)`, `host.block_states().states_of(stone)`,
`host.block_item(stone)`, `host.block_in_tag(stone, tag)` and restricted-level
`schedule_block_tick(pos, stone, delay)` use `BlockKey`. Property expressions accepted by
`block_state_id`, tag names and existing fluid operation arguments remain strings for now.

Declare a tank-only fluid in your own namespace with `r.declare_fluid("chocolate")`; the returned `FluidKey` identifies it for the mod load. Request vanilla or another mod's declared fluid with `r.fluid_key("minecraft:water")` or `r.fluid_key("other:chocolate")`. Requests resolve after all registrations, so declaration order does not matter; an unknown request stops the load and names the requesting mod. Tank-only fluids have no world simulation or world state. Fluid stacks and recipe ingredients use these keys; remaining fluid operations move to them in subsequent changes.
A saved tank whose fluid's mod is absent carries an opaque unresolved identity. Its stack's
`key()` is `None`; `UnresolvedFluid::id(host)` reads its saved name. It can move between
name-backed mod tanks, but not into vanilla buckets or cauldrons.

`NativeActiveEffect::id` and effect hook payloads carry `EffectKey`; living effect queries,
application and removal use the same resolved key. `NativeActiveEffect::new(speed, …)`
uses a key requested through `r.effect_key("minecraft:speed")`. Beacon-selection hooks
carry `Option<EffectKey>` for each power; `None` is no selection. Enchantment effect-component
names (for example `minecraft:damage` in `has_enchantment_effect`) are a different registry
and remain strings, as do labels and tags.

### API-owned values: migration table

| Previous engine value | API-owned value | Mod changes |
| --- | --- | --- |
| Package manifest `Side` | `mod_api::Side` | Match `Client` and `Both` on the package metadata snapshot. This is not `HostSide` or the executing logical side. |
| Command `Suggestion` | `mod_api::Suggestion` | Construct with `Suggestion::new((start, end), text)`; optional `tooltip` and `int_value` remain writable. Offsets are byte offsets. |

The game owns package and completion conversion. Mods do not import package-loader or
client-core types for these values. Completion edits are copied back after subscribed
listeners run; no listener means no snapshot allocation. Package metadata is copied once
at startup, retaining declared namespaces and dependencies.

### Item values and tag names

Every item has an `ItemKey`, including vanilla items no mod requested during registration.
`r.item_key(id)` retains a forward reference in the same load-owned table; after the item
registry is complete, `host.item_key(&identifier)` resolves any registered item. A key is
still a private `u32` index, not a copy of an item definition. Resolve once and retain it.
`host.max_stack_size(key)` and `host.max_damage(key)` read item defaults without exposing
the engine's item table.

`ItemStack` is owned API data with private fields. `ItemStack::new(key, count)` refuses
zero, while `ItemStack::of(key)` makes one item. Count mutation refuses zero and arithmetic
overflow. Registry-dependent maximum count and damage, including component overrides,
are checked by the game when accepting a write; an oversized value is refused with a typed
error, never silently clamped. This is the same boundary on native and wasm. Consuming
`split(n)` returns `StackSplit { taken, remainder }`, both optional; consuming `shrink(n)`
returns the remainder or `None`. There is no empty-stack sentinel. Component descriptors
read and write an optional owned NBT patch; a plain stack has no patch allocation.

`host.tags()` and a restricted level host's `tags()` expose the same `Tags` service. Resolve
a validated identifier once with `tags.key(&identifier)` and retain its `TagKey`. The runtime
owns tag-name slots for the process; reload replaces membership, not names or key identity.
`tags.is_in_tag(item_key, tag_key)` tests merged item membership. `tags.items(tag_key)`
and `tags.fluids(tag_key)` return registered keys in vanilla's member order. Unknown tags
return false or empty lists. Membership reads return `None` on clients, before authoritative
tags load, or if the relevant registry mapping is unavailable. Key resolution itself can be
refused if the runtime cannot allocate a name slot. No lookup stringifies a retained key.
Block member enumeration moves with the registry-wide `BlockKey` table in the next type slice.
The engine `TagRegistry`, `StackData`, `StackView` and `DisplayContext` are not API types.

A hook whose catalog entry has a `scope` (`block`, `block_entity`, `item`, `menu`) and a `key`, the argument
naming the thing it is about, is a method on that scope's trait only: `BlockPart`, `BlockEntityPart` (a
`BlockPart` too), `ItemPart` and `MenuPart`, as vanilla's and NeoForge's `Block` and `Item` methods are. It has
no `Hooks` method: a thing without parts is never asked, and the game asks the parts attached to it in dispatch
order, through the hook's contract, so a `Cancel` or a replacement skips the parts after it. Where NeoForge fires
a bus event at that moment, the event is a hook of its own on `Hooks`, asked first for every thing, and its cancel
skips the parts: a mod reacting to its own blocks uses a part, one reacting to every block the event. A part's kind is its type: the same kind twice on one
thing, an invalid key declaration, or an unnamespaced id in the other Part registrations stops
the game at startup with a message naming the mods. A panic in a part
adds `, part <type>` to the mod and hook it names. Parts take `&self` and are `Send + Sync`, like `Hooks`.
