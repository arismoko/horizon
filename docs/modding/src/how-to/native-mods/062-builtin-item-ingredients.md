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
`max_stack_size`, `rarity`, `base_color`, `enchantments`, `stored_enchantments`, `custom_name`,
`enchantment_glint_override`, `item_name`, `item_model`, `dyed_color`, `unbreakable`, `glider`, `intangible_projectile`, `tooltip_style`, `note_block_sound`, `dye`, `sheep/color`, `shulker/color`, `cat/collar`, `wolf/collar`, `waxed`, `block_entity_data`, `enchantable`, `map_id`, `villager_food`, `food`, `weapon`, `fox/variant`,
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
`unbreakable`, `glider`, `intangible_projectile` and `waxed` require an object, ignore its fields and normalizes to `{}`;
scalars and lists are rejected. Matching requires effective presence, including
item defaults, while an explicit removal suppresses that default.
`dye` accepts only the exact serialized dye color names, not numeric ordinals.
`tooltip_style` and `note_block_sound`, like `item_model`, accept identifiers
without a registry lookup; empty paths are accepted and missing or empty namespaces
normalize to `minecraft`. Matching borrows candidate strings without allocation.
`base_color` uses the same strict dye palette as `dye`, retains its dedicated
shield value, and remains absent on banners without that component.
`villager_food` requires a compound with positive integer `nutrition`; unknown fields are ignored.
The other 86 registered built-ins use stored-value matching while their shared
decoders are added incrementally in **#3875**. Their absence is not a load refusal.

Component ingredient displays contain the patch, including removals. Difference
lists retain the base list and remove only items whose default stack passes the
subtracted predicate. Non-simple cooking ingredients are tested outside the
simple item index, in original recipe priority order.

A non-simple ingredient may have an empty default-stack display list and still
load. It retains its full candidate set for concrete matching and cooking.
For example, a diamond sword minus swords with damage 0 has no default display,
but a damage-5 sword satisfies it. Empty display lists do not erase candidates.

`sheep/color` shares the strict dye palette decoder and has no implicit item default.
Explicit prototype values participate in matching and removal checks.

`shulker/color` also uses the strict dye palette and borrows explicit prototype values.

`cat/collar` uses the same strict dye palette and explicit prototype matching.

`wolf/collar` shares the strict dye palette and explicit prototype matching.

`food` requires non-negative `nutrition` after Java integer conversion and numeric
`saturation` (32-bit float, including negative and nonfinite values).
`can_always_eat` defaults to false and uses numeric boolean rules. Unknown fields
are ignored. Float equality treats all NaNs as equal but distinguishes signed zeros.

`weapon` decodes its optional damage and blocking-disable fields with defaults 1 and +0.0.
Damage uses Java integer conversion before its nonnegative check; seconds must be finite
and between +0.0 and the largest finite float, excluding negative zero.

`fox/variant` accepts the exact persistent names `red` and `snow`.

`compostable` requires `layers`: any integer or a syntactically valid reference; unknown fields are ignored.

`cushion/color` uses the shared dye palette.
