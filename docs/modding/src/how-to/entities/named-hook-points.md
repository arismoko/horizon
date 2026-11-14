# Named hook points

Implement catalog hooks as methods of `mod_api::Hooks`.

For function-slot listeners, declare `[[inject]]` with `id`, a target in `at`,
`kind` (`"head"`, `"return"` or `"replace"`), and a crate-root listener in `with`.
The optional `hash` pins the function fingerprint; when you omit it, the build
uses the pinned game's catalog. The schema rejects `hook` and `hook_version`.
See [Injections](../../concepts/injections.md).

Native rendering callbacks such as `horizon_mod_render::render::world_drawn`,
`hand_drawn` and `world_views` use a HEAD mixin. The launcher compiles the patched
function against the prebuilt game. Callbacks borrow the frame's GPU resources;
`world_views` supplies additional camera views. The renderer refuses invalid,
duplicate and oversized view requests by name, as `push` returns them.

Use the geometry types re-exported as `horizon_mod_render::render::{Vertex, EntitySpan}`.
Implement the `render_needs` callback to set `RenderNeeds` requests for world depth
and entity geometry; these requests feed the engine's frame requirements so it
retains the resources your drawing callbacks borrow.

## Teleport hooks

`ender_pearl_teleport` covers player-owned and mob-owned pearls. Its `owner` is an
`EntityRef`; listeners can cancel or redirect `target`, and read what the pearl hit
(`hit_entity`, or `hit_block` and `hit_face`, and the point `hit_at`). Player owners retain the
acting-player context and mutable fall `damage`. Mob owners take no pearl damage
and make no endermite roll. Pearl placement does not emit the command-only
`entity_teleport` notification. The pearl is consumed even when teleportation is canceled.

`item_consumption_teleport` covers players and foxes with the random-teleport
consume effect. Its `consumer` is a `LivingRef`, and `consumed` is the stack captured
before use, including count and components. Cancellation ends further teleport
attempts, not consumption. Fox callbacks run before that fox's living AI/movement
and before the next mob ticks, with the full server host. Listener changes to an
actor's item, position or eligibility survive rather than being overwritten by a
stale teleport decision.
