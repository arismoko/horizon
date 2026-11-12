# Custom ingredient kinds

Implement `Hooks::ingredient(context, event)` to resolve recipe ingredient types outside the `minecraft:` namespace. Fill `event.items` with the item IDs your ingredient accepts. The loader removes duplicates in first-seen order.

This list filters by item only. Use the [match-time hook](ingredient-matching.md) to check components on a concrete stack.

## Load contract

- The hook receives the owning load's `DataLoadContext`.
- Each occurrence is dispatched once per load, including nested ingredients and ingredients in any recipe slot.
- Dispatch occurs before the load's tables parse recipes. Server tables, stations and copies sent to guests share that load.
- The client's recipe preload reports `ClientPreload` through `origin()`; the server's recipe load reports `ServerDataLoad`, including on a dedicated server.
- `/reload` dispatches the occurrences again. A client joining a remote server does not receive `ServerDataLoad`.
- Return a consistent answer for the same input. An unhandled kind leaves an empty list and fails the recipe.

## Ingredient data

Read `event.data` as an API `Compound`, using typed `Tag` variants such as `Int`, `Long`, `List` and `Compound`, not JSON accessors.

The loader converts recipe metadata once using vanilla JsonOps-to-NbtOps rules. Numeric JSON arrays remain lists. Nested null values refuse the ingredient and fail the recipe. The match-time hook receives the same retained compound without reconversion.
