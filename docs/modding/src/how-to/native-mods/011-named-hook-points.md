<!-- chapter: Named hook points -->
<!-- separator: 1 -->

### Named hook points

Implement catalog hooks as methods of `mod_api::Hooks`. An injection does not select a hook by
`hook` or `hook_version`: the manifest schema accepts `[[inject]]` with a target in `at`,
a crate-root listener in `with`, and the pinned function hash. See [Injections](../../injections.md).

Native draw functions such as `horizon_mod_render::render::world_drawn`, `hand_drawn` and `world_views` use a HEAD mixin. The launcher compiles the patched function against the prebuilt game.
The renderer owns the frame geometry and dispatch slots; the mod host installs adapters before
startup. `mod_render::render::{Vertex, EntitySpan}` re-exports the engine's geometry without
copying buffers. `RenderNeeds` requests are copied back into the engine's frame requirements;
world and hand callbacks borrow the current GPU resources without allocating per frame.
World-view requests and draw notices use the same engine-owned slots: the provider copies
mod-facing `WorldView` values into engine-owned request values, and adapts the borrowed
`WorldViewDrawn` resources back to the mod-facing event. The renderer filters invalid,
duplicate and oversized requests before drawing or notifying listeners. No replacement means
no request collection. Slot dispatch adds no allocation; request collection and filtering
retain their existing `Vec` storage. This changes rendering only, not tick-time work.

#### Teleport hooks

`ender_pearl_teleport` version **2** covers player-owned and mob-owned pearls. Its `owner` is an
`EntityRef`; listeners can cancel or redirect `target`. Player owners retain the
acting-player context and mutable fall `damage`. Mob owners take no pearl damage
and make no endermite roll. Pearl placement does not emit the command-only
`entity_teleport` notification. The pearl is consumed even when teleportation is canceled.

`item_consumption_teleport` version **2** covers players and foxes with the random-teleport
consume effect. Its `consumer` is a `LivingRef`, and `consumed` is the stack captured
before use, including count and components. Cancellation ends further teleport
attempts, not consumption. Fox callbacks run before that fox's living AI/movement
and before the next mob ticks, with the full server host. Listener changes to an
actor's item, position or eligibility survive rather than being overwritten by a
stale teleport decision. Command and Ender-entity teleport hooks are unchanged.
