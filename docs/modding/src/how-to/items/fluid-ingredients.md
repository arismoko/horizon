# Fluid ingredients

A mod's own recipe type that drinks from a tank reads its fluid inputs as a `FluidIngredient`, the
fluid analogue of an item ingredient: `FluidIngredient::from_json(json, context)` parses a fluid
(`"minecraft:water"`), a `#tag` (the fluid tags the load's `DataLoadContext` holds), a list of
them, or NeoForge's `compound`, `intersection` and `difference` shapes, with the same walk the item
ingredient parses its own JSON with — call it where a data load hook hands you the context, as the
`ingredient` hook does while the game resolves recipes. `test(&FluidStack)` asks whether a tank's
fluid answers, and `fluids()` lists the fluids it accepts, in tag order, for a recipe book. A kind
outside the standard shapes fails the parse: fluids have no custom ingredient hook, and components
are not checked — any stack of a named fluid answers.

`SizedFluidIngredient` adds the amount, as NeoForge's `SizedFluidIngredient`:
`{"ingredient": <fluid ingredient>, "amount": 500}` in millibuckets, `"amount"` defaulting to a
bucket (1000) and always positive. Its `test(&FluidStack)` asks that the stack matches the
ingredient *and* carries at least that much.
