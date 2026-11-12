# Item places and components

An item's own capability is reached where its stack lives, an `ItemPlace`: a player's slot, hand or cursor, a
block entity's slot, an item entity, a stack inside another item (`ItemPlace::Nested`) or a mod's own place
(`ItemPlace::Mod`, read and written by the `PlaceProvider` it registers with `r.place_provider`).
`r.item_capability(&ITEM_ITEMS, "mymod:backpack", |host, place| Some(Box::new(Backpack)))` answers
`ITEM_ITEMS` for an item with a `ResourceHandler<ItemResource>` made for that place, and
`r.item_capability_before_game` ahead of the game's own over `item_container`;
`place.capability(host, &ITEM_ITEMS)` (`ItemAccess.getCapability`) asks them in the same order. A handler keeps its contents in the
stack, and reads and writes it through the place, as NeoForge 26.3's `ItemAccess`: `place.stack(host)`,
`place.resource(host)` and `place.amount(host)`, and `place.insert`, `place.extract` and `place.exchange(host, tx,
&resource, amount)`, which answer how much moved. Each runs in a transaction of its own, nested in `tx`, and changes
the place where it is, journaled, so an abort puts it back at every level of an item inside an item. A handler
answering more than it was asked or offered is a mod bug: the move it was part of is undone, and the game warns
once, naming the mod. What doesn't stack with the place, or doesn't fit, goes where vanilla puts an item split off
it, so `exchange(host, tx, &filled, 1)` on a stack of empty buckets is vanilla's filled bucket: from a player's
place into their inventory, else dropped with no thrower (a creative player's hand takes anything and gives them
one if they have none, as `forInfiniteMaterials`); from an item entity dropped beside it; from a block entity's,
an entity's or an item's slot into that container's other slots (`forHandlerIndex`). A handler rewrites its item
with an `exchange` of one. An exchange inside one of two backpacks is done in one of them: that one is rewritten
holding the change and splits off, and the other stays as it was.

A stack's components change the same way, where it lives (NeoForge's `ItemResource.with` and `without`). A component
is named by a `ComponentId`, checked once as a namespaced id (`ComponentId::new("minecraft:damage")`, None for
`Damage` or `!minecraft:food`). `resource.component(&id)` reads the stack's own value in its patch as saved;
`resource.with_component(host, &id, value)` sets it as vanilla's `PatchedDataComponentMap.set` does, keeping no value
of the stack's own where `value` is the item's default (`host.item_default_component(item, id)`, compared as the component's codec reads it: a vanilla
component's numbers by exact value, `custom_data` and the other raw NBT components and a mod's own type for type), and `resource.without_component(host, &id)` removes the component as `remove` does: the stack's own value
goes, and where the item has the component by default a removal (`"!minecraft:max_damage": {}`) is recorded in the
patch.
An exchange of one puts the changed item in place: `place.exchange(host, tx, &place.resource(host)?.with_component(host, &damage,
Tag::Int(5)), 1)` damages one sword of a stack, splitting it off.
