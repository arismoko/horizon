# Entity energy and fluids

`entity.capability(host, &ENTITY_ENERGY, side)` and `entity.capability(host, &ENTITY_FLUIDS, side)` reach a mod's
entity energy and fluid handlers (NeoForge 26.3's `Capabilities.Energy.ENTITY` and `Capabilities.Fluid.ENTITY`), where
`side` is the `Option<Direction>` of the entity the lookup reaches it from, `None` for no particular side. Vanilla
answers neither; register a provider with `r.entity_capability(&ENTITY_ENERGY, "mymod:robot", |host, robot, side| …)`,
which is given that side and may answer differently for each. An entity capability's context is its key's second type:
`()` for `ENTITY_ITEMS`, `Option<Direction>` for these two and for `ENTITY_AUTOMATION`. The existing `move_energy` and `move_resources` transfer
between these and block or item handlers inside a transaction.
