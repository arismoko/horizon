# Entity automation

`entity.capability(host, &ENTITY_AUTOMATION, side)` (NeoForge 26.3's `Capabilities.Item.ENTITY_AUTOMATION`, "automation-accessible
inventory of an entity (boats, minecarts, etc)") is the items of an entity as a hopper reaches them, with the same
`Option<Direction>` context as the energy and fluid keys. The game answers it for the container entities
`EntitySelector.CONTAINER_ENTITY_SELECTOR` picks (chest and hopper minecarts, chest boats) with the slots their
[`ENTITY_ITEMS`](entity-items.md) handler has; a player and a chested donkey, mule or llama answer none. A hopper
pushing into the entity in front of it asks for the side it faces the entity from (`Up` for a hopper over it), a hopper
pulling from the entity above it for `Down`, and a dropper for the side it drops onto. Beside the game's container
entities, those entities at the target whose mod provider answers it join the candidates, and the hopper picks one at
random with the draw `getEntityContainer` makes; a mod handler takes the container's place and moves one item at a time
in a transaction, as a mod block's `ITEMS` does. `r.entity_capability(&ENTITY_AUTOMATION, "mymod:robot", |host, robot, side| …)`
registers one, and `r.entity_capability_before_game` answers ahead of the game's own for a minecart or a boat.
