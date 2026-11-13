# Custom ingredient kinds

Implement `Hooks::ingredient(context, event)` to resolve recipe ingredient types outside the `minecraft:` namespace. Fill `event.items` with the item IDs your ingredient accepts. The loader removes duplicates in first-seen order.

This list supplies recipe-book placement candidates. Use the [match-time hook](ingredient-matching.md) to check a concrete stack. A nonempty candidate list also filters matching by item; an empty custom list retains the predicate without this filter.

## Load contract

- The hook receives the owning load's `DataLoadContext`.
- Each occurrence is dispatched once per load, including nested ingredients and ingredients in any recipe slot.
- Dispatch occurs before the load's tables parse recipes. Server tables, stations and copies sent to guests share that load.
- The client's recipe preload reports `ClientPreload` through `origin()`; the server's recipe load reports `ServerDataLoad`, including on a dedicated server.
- `/reload` dispatches the occurrences again. A client joining a remote server does not receive `ServerDataLoad`.
- Return a consistent answer for the same input. If no mod listens to the `ingredient` hook, a recipe file naming a custom kind fails whole at load with an error naming the file and the kind, and the recipe is absent everywhere. If a mod listens but leaves the list empty, the recipe and its match-time predicate are retained, with a warning naming the file and the kind that the recipe cannot be placed. It is excluded from recipe-book learning/displays, recipe property sets and stonecutter selections; the server also refuses recipe-book placement. The effective recipe map and manual matcher still retain it.
- An existing empty item tag likewise retains an unplaceable recipe, but never matches a stack. An undefined tag fails the recipe with an error. A direct empty list (`[]`) remains invalid and fails the recipe with an error.

## Ingredient data

Read `event.data` as an API `Compound`, using typed `Tag` variants such as `Int`, `Long`, `List` and `Compound`, not JSON accessors.

The loader converts recipe metadata once using vanilla JsonOps-to-NbtOps rules. Numeric JSON arrays remain lists. Nested null values refuse the ingredient and fail the recipe. The match-time hook receives the same retained compound without reconversion.
