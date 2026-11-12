# Cache reload-stable recipe definitions

A machine should not fetch and parse its recipe files on every block-entity tick.
Cache the **definitions**, not the answer to whether a recipe matches. This pattern
uses the same source and API for native and wasm mods. It needs no wasm-specific
memoizer or polling import.

## Publish after the final server data

Listen to `tags_updated` on the server side. It runs after the recipes and tags have
been published, both on first open and after a successful reload. Build the whole next
table before taking the write lock, then replace the old table in one operation.
An empty table must replace the old one too: pack overrides and removals matter.
Tag-only reloads also rebuild the ingredients. A failed engine reload does not fire
the publication hook, so the previous definitions remain in force.

A reload listener's prepare/apply phase is useful for reading your own resources,
but is too early to cache the final recipe table. Do not move this work into the
first tick as a lazy initialization path.

This example is a custom recipe type, whose fields live in `Recipe::json`. Use
the host's existing ingredient and result parsers rather than implementing tag,
component or custom-ingredient semantics yourself. Invalid definitions are skipped
in file order; the first matching valid definition wins.

```rust,ignore
{{#include ../../examples/src/reload_recipes.rs:cache}}
```

Clear the table at server startup and after teardown. Share the listener's cache
with the machine Part (for example using `Arc<Processing>`); do not create a new
cache in the tick. Until publication, the empty table means no recipe can run.

```rust,ignore
{{#include ../../examples/src/reload_recipes.rs:publication}}
```

This ownership assumes **one authoritative server per process and mod group**, as
in the current integrated, LAN and dedicated entry points. The connected client
does not own this server table. If the engine ever supports concurrent servers in
one group, key the cache by a server identity once the API offers one; neither a
recipe-type key nor a dimension ID distinguishes servers. Startup/teardown clearing
prevents reuse across sequential world opens, not concurrent ones.

## Keep live answers live

Every tick, call `result` with the actual input. `Ingredient::test` still consults
the host and custom matchers each time. Do not remember a match by item ID: the
stack's components or the matcher's world-dependent answer can change without a
reload. This example takes one read guard per selection, not one per recipe, and
clones only the selected result stack, not the recipe array or its JSON.

After selection, check the current output's compatibility, count and stack limit.
Keep capability, pipe, inventory and world reads live too. Caching definitions
does not authorize a write or reserve output capacity. A blocked machine resets
progress according to its existing policy.

The [machine sample](https://github.com/arismoko/horizon-machine-sample) demonstrates
the full processing and transfer path. Test reload overrides, empty/removal and
tag-only reloads, sequential worlds, replaced input (components included), and a
custom matcher whose live answer changes. A test should fail when publication is
mutated to keep the old table. Measure end-to-end tick time separately from reload
cost; fewer imports alone do not prove a particular millisecond saving.
