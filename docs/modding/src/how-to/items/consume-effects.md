# A mod's own consume effects

A consumable's `on_consume_effects` entry of a type in a namespace of its own reaches `native_consume_effect(kind, data)` when the item is consumed, on the server
thread, for the acting player. The kind and a borrowed `Tag` converted from the definition at load are there; the game's own effects,
sounds, teleports and drops are open to the hook. An unclaimed kind does nothing.
Consumable and death protection payloads are decoded once for item definitions and stack component patches; the event borrows the retained payload. Invalid payloads refuse definition loading rather than failing when consumed.
