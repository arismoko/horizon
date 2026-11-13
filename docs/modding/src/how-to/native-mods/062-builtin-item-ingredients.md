<!-- chapter: Built-in item ingredients -->

# Built-in item ingredients

Item ingredients accept item ids, lists and `#item_tag` references. Ingredient objects
read `neoforge:ingredient_type` first, then `type`. The built-in NeoForge kinds are
`compound` (any child), `intersection` (every child), `difference` (base without
subtracted), `components`, `block_tag` and `custom_display`. These built-ins never
call a mod's `ingredient` hook. Holder-set objects are a separate format and are
not accepted here. These object forms are read from recipe resources. The
`Host::recipe_ingredient` convenience reader accepts flat item inputs.

```json
{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:damage":5},"strict":false}
```

`components.items` accepts an id, list or `#item_tag`. Every component in the patch
must equal the concrete stack's effective value, including item defaults. A `!`
prefix requires absence: `{"!minecraft:enchantments":{}}`. `strict` defaults to
false; when true, every effective component must also be named in the patch.
For example, a diamond sword's default `minecraft:max_damage` counts. Count does
not participate; empty stacks never match.

```json
{"type":"neoforge:block_tag","tag":"minecraft:logs"}
{"type":"neoforge:custom_display","base":"minecraft:apple","display":{"type":"minecraft:item","item":"minecraft:diamond"}}
```

`block_tag.tag` names a block tag without `#`, matching its blocks' items.
`custom_display` matches its base and uses the slot display only for presentation.
As with packet slot displays, potion, dye and trim demo displays show their source
stacks. Matching, placement and network ingredient membership retain the base's
item list.

Plain item lists, item tags and block tags are simple. Components and mod kinds
are not simple. Compound, intersection and difference are simple only when every
child is simple; custom display follows its base. Shapeless recipes use item lists
when all ingredients are simple and otherwise test each concrete stack. Shaped
recipes always test the concrete stack. Recipe-book craftability, placement,
recipe property sets and network ingredients continue to use item lists.

## Component decoder support

Every component id must be registered, including removal keys. A payload is
rejected when its available decoder rejects it, and the whole engine-owned
recipe is omitted. Mod-owned recipe types retain their own validation.

Registered components without a shared decoder **load**. Raw `other` values
compare by exact stored NBT equality. Modelled values, including potion, bundle,
container and trim contents, compare against their complete retained fields.
All preparation happens at load; candidate comparisons borrow values without
allocating or converting them to JSON. Removal keys require registration only.
Native registrations retain their persistent NBT validation contract.

The shared decoder currently covers `damage`, `repair_cost`, `max_damage`,
`max_stack_size`, `rarity`, `enchantments`, `stored_enchantments`, `custom_name`,
`enchantment_glint_override`, `item_name`, `item_model`, `dyed_color`, `block_entity_data`, `enchantable`, `map_id`,
`ominous_bottle_amplifier`, `potion_duration_scale`, and `minimum_attack_charge` in the `minecraft` namespace.
`enchantable` requires a positive integer `value`; extra record fields are ignored
and equivalent numeric NBT widths match, including item defaults. Map ids accept
signed numbers, including negative ids, using Java's integer conversion.
The amplifier accepts numeric forms converted to an integer in 0..=4;
out-of-range payloads are rejected, never clamped.
Potion duration scales decode numeric payloads to floats; zero and finite positive
values are accepted, but negative-zero floating tags, negatives, infinities, NaN,
and nonnumeric values are rejected. JSON's integral `-0.0` becomes zero before
decoding. Matching compares decoded float values, including item defaults,
without copying candidate payloads.
`minimum_attack_charge` converts numbers to 32-bit floats before checking the
inclusive range 0 to 1; decoded negative zero, NaN and infinities are rejected.
JSON-to-NBT conversion first canonicalizes literal `-0.0` to integer zero.
Equivalent numeric representations match live values and item defaults.
`item_model` accepts valid resource identifiers without requiring an existing model;
bare names and an empty namespace normalize to `minecraft`.
`enchantment_glint_override` accepts booleans and all numeric NBT widths:
zero (including negative zero) is false; nonzero values, fractions and NaN are
true. Displays retain canonical byte 0 or 1.
`dyed_color` accepts packed integer colors or three numeric RGB channels,
matching the decoded values without copying candidate payloads.
The other 104 registered built-ins use stored-value matching while their shared
decoders are added incrementally in **#3875**. Their absence is not a load refusal.

Component ingredient displays contain the patch, including removals. Difference
lists retain the base list and remove only items whose default stack passes the
subtracted predicate. Non-simple cooking ingredients are tested outside the
simple item index, in original recipe priority order.

A non-simple ingredient may have an empty default-stack display list and still
load. It retains its full candidate set for concrete matching and cooking.
For example, a diamond sword minus swords with damage 0 has no default display,
but a damage-5 sword satisfies it. Empty display lists do not erase candidates.
