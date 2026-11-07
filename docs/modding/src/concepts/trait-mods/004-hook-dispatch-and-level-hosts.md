<!-- chapter: Hook dispatch and level hosts -->
<!-- separator: 1 -->

A linked trait mod implements `mod_api::Hooks`. Helper injections run first.
An injection that returns early skips trait mods. A claimed cancellation also skips trait mods.
Otherwise, trait mods run in their supplied order. The first `Cancel` stops dispatch.
For replacement hooks, the first `Some` wins. Collection hooks call every mod. Receiverless hooks have no `Host`.

Each hook method takes its host, where it has one, and one event:

```rust
fn living_incoming_damage(
    &self,
    _level: &mut dyn LevelRandomHost,
    event: &mut LivingIncomingDamageEvent<'_>,
) -> Flow {
    if event.kind == "minecraft:pig" {
        let halved = event.container.new_damage() * 0.5;
        event.container.set_new_damage(halved);
    }
    Flow::Continue
}
```

The event is a generated struct named after the catalog hook, in CamelCase plus `Event`, with
the hook point's arguments as public fields: values as values and borrowed ones as references (`event.kind`). A
number or `bool` a mod may change is a plain field, written back once every mod has run, even after a `Cancel`;
anything else a mod may change, a game struct or a list, is a `&mut` field. A cancellation or replacement is still
the return value; a collection hook's list is a field (`event.lines`). One event goes through every mod, so each
sees the changes before it. Because no mod can build or match the whole
struct, a later version can add a field without breaking any mod, as NeoForge's event objects do; the cost is that
a mod's own tests drive a hook through its dispatch function ([Testing a trait mod](026-testing-a-trait-mod.md#testing-a-trait-mod)).

Hook methods take `&self` and require `Send + Sync`. The process-wide mod list is installed once, like mixin module statics. One mod instance serves both
the window's game and the server-thread game. A hook can also fire again inside
the same mod: for example, `LevelRef::set_block` fires `block_changed`. Mods keep mutable
state with interior mutability so these calls can share the instance safely. Data-load
hooks can run on any loading thread; item default changes must be identical on every side.

The trait has every hook declared under `crates/horizon-mod-api/src/hooks/` ([Adding a hook](027-adding-a-hook.md#adding-a-hook)).
Level hooks use access-specific hosts rather than `Host`. `LevelHost` provides
`block_state` through `LevelAccess::block_state`, client-side detection and
`state_with_property`. `state_string(state)` serializes the full name and properties, or returns `None`
for an unknown state; `min_y()` and `height()` read the level's build range. It also reads light and loaded state, through the game's own queries:
`brightness(LightLayer::Block | Sky, pos)` (`getBrightness`, the stored 0 to 15),
`raw_brightness(pos, sky_darken)` (`getRawBrightness`, which `CropBlock.randomTick` asks with 0)
and `is_loaded(pos)` (`Level.isLoaded`: inside the build height and in a loaded chunk),
`is_raining_at(pos)` (`Level.isRainingAt`, false where `pos` is not loaded) and `enabled_features()`
(`LevelReader.enabledFeatures`, a borrowed `FeatureFlags<'_>`). Neither allocates.
None of them loads a chunk. Light above the build height reads as full sky and below it as
none, and a chunk that isn't loaded reads as the game's query does (full sky, no block light),
so check `is_loaded` first. It also answers tag and fluid questions without a `Host`: `block_in_tag(block_key, tag)`,
`item_stack_in_tag(&stack, tag)`, `fluid_state(pos)` (`getFluidState`, `None` where the block read is),
`state_fluid_state(state)` and `fluid_in_tag(fluid_name, tag)`. Fluid reads return `FluidState` with a
`FluidKey`, amount, falling flag and engine-derived source flag; `is_empty()` and `is_source()` report
those states without a registry lookup. `block_state_id(name)` parses a block and properties, and `block_state_default(block)` gives a block's default state, as on `Host`; both return `None` for invalid input or unavailable context. Tag reads are `None` when authoritative tags are unavailable, including on a client level. Shared `Host::tag_members` and `Host::item_stack_in_tag` likewise return `None` on clients until server-sent tags are synchronized; they never substitute local resource-pack tags.
`LevelHost` has no random, by design: a draw from a read-only hook inside a level write would shift the level's random sequence away from vanilla's order. A hook that may draw is given a `LevelRandomHost`.
`LevelRandomHost` provides vanilla's `Level.random` so
mod draws come in vanilla's order. `LevelTicksHost` adds scheduling through
`native_schedule_block_tick` and `native_schedule_fluid_tick`, while
`LevelWriteHost` adds random access, `set_block`, `set_block_property`, `native_play_sound` and
`native_send_particles` and `native_level_event` (`levelEvent`, queued in order with the write's own events).
`set_block(pos, state, flags)` is `Level.setBlock` on a whole state, as `LevelRef::set_block` takes it: it
removes the old block's block entity and creates the new block's in the write (`LevelChunk.setBlockState`), runs
the neighbour and shape updates `flags` ask for, and queues the write's level events in order. A mod block entity
it creates starts with the data the `block_entity` hook gives, which is fired in the write and receives the level
(`LevelSide`, read-only: vanilla's `newBlockEntity` gets no level), as the `Host`'s `native_set_block` fires it. It returns true when the block
changed, and false off an authoritative server, for a position that is not loaded or outside the build height,
an invalid state, or the state the block already has. It never loads a chunk. The game owns
the tick queue; mods do not pass it back to these helpers.
Frame drawing through `DrawContext`, `BlockEntityDrawContext` and `WorldDrawContext` stays
in `mod_api`. Raw GPU access is native-only, unstable and outside that API, in
`mod_render::render` (`horizon-mod-render`). Register its listed frame functions through
`[[inject]]` with `access = "internals"` and `kind = "head"`; the ordinary listener registry
owns ordering and disabling. `render_needs` collects requests; `world_drawn` runs before
the hand and HUD, and `hand_drawn` runs after the hand, on the render thread.

| Removed path or registration | Current route |
| --- | --- |
| `mod_api::render` raw GPU types | `mod_render::render` (native escape hatch) |
| `RenderHooks` / `Hooks::render` | `[[inject]]` internals listeners on the three listed frame functions |
| API draw contexts | Unchanged; use these for API-typed drawing |
