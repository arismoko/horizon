# Item places and components

Use an `ItemPlace` to read or change a stack where it lives. A place can identify a player's
slot, hand or cursor, a block entity's slot, an item entity, an entity's inventory or equipment,
a menu slot, or a stack inside another item (`ItemPlace::Nested`). For storage you manage,
register a `PlaceProvider` with `r.place_provider` and use `ItemPlace::Mod`.

## Item capabilities

Register an item's capability with
`r.item_capability(&ITEM_ITEMS, "mymod:backpack", |host, place| Some(Box::new(Backpack)))`.
Your provider returns a `ResourceHandler<ItemResource>` for that place. Use
`r.item_capability_before_game` to answer ahead of the game's provider, which handles
slots declared with `r.item_container(&item, provider)`. `place.capability(host, &ITEM_ITEMS)` queries these providers in order.
The registered `ItemContainerProvider::slots(&dyn Host)` callback returns `Option<u32>` dynamically;
it is not called by registration-time coverage queries. `ITEM_ITEMS` covers the declared item type
even when that callback currently returns `None`. A closure `|host: &dyn Host| …` can be the provider.
Declarations may target vanilla or mod items. Enabled callbacks run in registration order;
`None` falls through to the next provider, and the first `Some` supplies the slot count.

Keep the contents in the stack and use these methods to access it:

| Method | Result |
|---|---|
| `place.stack(host)` | The stack, or `None` |
| `place.resource(host)` | Its resource without a count, or `None` |
| `place.amount(host)` | Its count, or 0 |
| `place.insert(host, tx, &resource, amount)` | How much was inserted |
| `place.extract(host, tx, &resource, amount)` | How much was extracted |
| `place.exchange(host, tx, &resource, amount)` | How much was exchanged, or 0 if the exchange fails |

Each write runs in its own transaction nested in `tx`. Aborting restores changes at every
level of nested items. If a handler answers more than requested, the move is undone and the
game warns once, naming the mod.

## Exchanging one item

Rewrite one item with `place.exchange(host, tx, &changed, 1)`. For example, exchanging one empty
bucket for a filled bucket splits the filled bucket from the stack. Overflow follows the
place's rules:

- A player's place sends it to their inventory, then drops anything that does not fit with no
  thrower. A creative player's hand accepts items and gives them one if they have none
  (`forInfiniteMaterials`).
- An item entity drops it beside itself.
- A block entity, entity or nested item slot tries the container's other slots (`forHandlerIndex`).
- A mod-owned place uses its `PlaceProvider`'s rules.
- A menu-slot insert fills only that slot; excess stays out.

An exchange inside a stack of two backpacks changes one backpack and splits it off. The other
backpack keeps its contents.

## Editing the whole stack

To change components on the **whole stack**, use `place.edit(host, tx, |stack| { ... })`
with a server host. It reads the stack, applies the closure and exchanges the entire
count in the same transaction. Missing stacks and edits that change the item or count
return `false` without changing the place; successful writes return `true` and a parent
abort restores them. `stack.set` and `stack.remove` edit typed overrides; `get` then
`set` covers `update`. `stack.apply_components(&patch)` merges values and `!id`
removals without losing unrelated components, refusing malformed damage atomically.
`stack.copy_component_from(&other, id)` copies an own value or removal; when `other`
inherits that component, it clears the target's override rather than recording removal.

## Component patches

To change **one item** of a stack instead (NeoForge's `ItemResource.with` and `without`), use a resource and exchange it as below. Name a component with `ComponentId::new("minecraft:damage")`. This returns `None` for an invalid
namespaced ID, such as `Damage` or `!minecraft:food`.

| Method | Behaviour |
|---|---|
| `resource.component(&id)` | Reads the stack's own value from its saved patch, not the item's default |
| `resource.with_component(host, &id, value)` | Returns `Result<ItemResource, StackError>` with the value set |
| `resource.without_component(host, &id)` | Returns an `ItemResource` with the component removed |

`with_component` removes a patch value when it equals the item's default from
`host.item_default_component(item, id)`. Equality follows the component codec: vanilla numeric
components compare by value, while raw NBT components such as `custom_data` and mod components
compare by NBT type and value. Invalid component values return `StackError`.

`without_component` removes the stack's own value. If the item has a default value, it also
records a removal such as `"!minecraft:max_damage": {}` in the patch.

These methods return detached resources. Store the result with an exchange of one item:

```rust,ignore
let resource = place.resource(host).ok_or("empty place")?;
let changed = resource.with_component(host, &damage, Tag::Int(5))?;
place.exchange(host, tx, &changed, 1);
```

The exchange damages one sword from a stack, splitting it off when necessary.
