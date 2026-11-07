# Item-stack hot-path audit

Part of #1608 and PR #1674. These are conversion costs, not whole-game tick
measurements. The native benchmark uses the actual generated `fire_use_item`,
one subscribed ItemPart reading item/count, and the checked-in game converter.
Its command and methodology are in [README.md](README.md). The PR body records
the main/head timing table and exact source heads.

## Allocation contract

Measured per read-only stack conversion and subscribed fire, using a separate
allocation-counting binary:

| Input | Allocations | Allocated bytes |
| --- | ---: | ---: |
| Plain | 0 | 0 |
| Numeric damage only, including explicit zero | 0 | 0 |
| Custom-name component | 14 | 1,220 |
| Nested NBT/list/byte-array component | 18 | 2,786 |
| Unsubscribed named-stack fire | 0 | 0 |

Plain construction and cloning allocate nothing. Component-bearing conversion
is **eager**, not lazy: the existing engine encoder builds an owned NBT patch
(entries, key/value and compound storage), then the API owns the normalized
patch. Nested NBT and byte/list arrays are cloned. Dispatch borrows that one
API snapshot; the benchmark measures no additional allocation in dispatch.
Cloning an API stack with a boxed component patch deep-clones that patch;
there is no copy-on-write cache. The read-only benchmark does not include
caller-initiated clones or mutable-output write-back.
Damage is copied as a typed scalar and omitted by the same engine encoder,
not by a second component codec. Explicit damage removal remains an opaque
patch entry and is not the numeric-only case.

An executable regression compiles the actual generated item-fire crossing:
1,000 unsubscribed calls perform zero conversions and zero dispatches; a
subscribed call performs one conversion and one dispatch. The guard is a
coarse subscription-kind guard, not a per-item or enabled-listener preflight.

## Tick, entity and slot boundaries

Every stack conversion below is guarded **before API snapshot construction**.
A callback with no stack payload does not implicitly take an inventory snapshot.
Explicit handle reads can still request an eager owned stack.

| Boundary | Guard and conversion | Component-lazy? |
| --- | --- | --- |
| `inventory_tick` (carried, armour and offhand slots) | Global subscriber guard; payload is only slot and selected flag. No stack conversion. The caller borrows the engine slot to check presence. | No implicit component read; an explicit stack read is eager. |
| `player_tick_pre/post`, `entity_tick_pre/post` (including item entities) | Subscriber dispatch; identity/scalar payload only. No stack conversion. | No implicit component read; an explicit stack read is eager. |
| `living_entity_use_item_tick`, `on_use_tick` | Global/Part guard respectively in generated fire, before converting the held stack. | **No.** Subscribed component-bearing stacks are eager. |
| `item_after_vertical_collision` | Block Part guard before converting the item entity's stack; runs on a vertical collision, potentially each grounded tick. `None` consumes the item; invalid replacement preserves the original. | **No.** |
| `block_entity_tick` | BlockEntity Part guard in the game adapter before snapshotting entity fields and item slots. | **No.** Slots are eager; the entity snapshot also clones its NBT/data and slot vector. |
| `menu_slot_rules`, `item_stacked_on_other` | Part guards in restricted-context adapters before converting candidate stacks. Rule evaluation may probe many slots; stacked-on-other is a click operation. | **No.** |
| `menu_button`, `menu_quick_move` | Part guard before entity/held-stack snapshots; action-only, not every tick. | **No.** |
| `on_equip`, `living_equipment_change` | Generated subscriber guard before converting new/from/to stacks; only an equipment change, not an unconditional per-slot snapshot each tick. | **No.** |
| `ingredient_test`, `get_enchantment_level`, `item_attribute_modifier` | Guards in installed restricted callbacks before stack conversion. These queries may be reached from tick, entity and slot processing. | **No.** |
| `use_duration`, `use_animation`, `can_eat`, `living_get_projectile` | Generated guards before conversion; query frequency depends on the calling use/AI path. | **No.** |
| `item_bar`, `item_decorations`, `get_arm_pose`, `render_item_in_frame`, four item-model property/tint hooks | Subscriber/Part guards precede snapshotting; these may run per visible slot/entity per frame. | **No.** |
| `render_hand`, tooltip/component/draw/gather/texture hooks, block-entity submit/bounds | Guarded before stack/entity snapshotting; per-frame rendering/hover paths. | **No.** |
| Container open/close, `menu_clicked`, `menu_closed`, `mod_container_viewers`, hotbar switches, `creative_slot`, `item_container` | Identity/scalar metadata only; no implicit stack conversion. | Not applicable. |

There is no separate `slot_changed` catalog hook. Other stack-bearing calls
(crafting, smithing, brewing completion, drops, attacks, expiry, interactions)
are action/change boundaries and use the same guarded converter. A container
snapshot's slot-vector allocation is additional to the per-stack counts above.
Pre-existing engine-local copies used for change detection are not included in
this API conversion benchmark and are not claimed removed.

## Named follow-up: lazy or cached component patches on hot boundaries

This PR does **not** implement first-component-read conversion. The measured
14 allocations / 1,220 bytes for a name and 18 / 2,786 for nested NBT remain even
when a subscribed callback reads only item/count. The review must explicitly
consider this cost for the hot rows marked **No** above.

The follow-up must cover use ticks, grounded item collisions, block-entity
slots, slot rules, attribute/enchantment/ingredient queries and per-frame item
render/model queries. It must keep item/count reads allocation-free, defer
component materialization until needed, and preflight item-specific subscribers
before conversion. A cache needs audited invalidation for engine component
mutation; a resource-based solution needs generated native/wasm bindings and
call-lifetime/write-back semantics. Neither may put an engine reference inside
the API's plain-data ItemStack or add a separate wasm implementation. This is a
separate boundary/storage concern, not a safe local codec substitution.

No wasm runtime speedup or whole-game tick improvement is claimed. Existing
stale entity/container handle contracts remain with the world-visibility work;
3a changes item values and retained load-owned keys, not entity residency.
