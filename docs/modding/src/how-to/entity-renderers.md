# Drawing an entity type with your own renderer

A client mod can draw every entity of a type with code of its own. This is the counterpart of
NeoForge 26.3's `EntityRenderersEvent.RegisterRenderers` (`registerEntityRenderer`): the mod
registers one renderer per entity type at start, and the game hands each entity of that type in
view to it, once a frame. Native and wasm mods register and draw the same way.

## 1. Write the renderer

A renderer implements `EntityRenderer`. Its `submit` gets the entity's frame and a draw context, the
same one a block entity or mob submit hook draws with (`quad`, `cube`, `block`, `item`,
`model_part`, `push`, `pop`, `translate`, `rotate_about`, `scale`):

```rust,ignore
use mod_api::*;
use std::sync::Arc;

struct Bolt;
impl EntityRenderer for Bolt {
    fn submit(
        &self,
        entity: &EntityRenderState,
        draw: &mut dyn BlockEntityDrawContext,
    ) -> Result<(), String> {
        // The pose starts at the entity, not turned: face the way it flies.
        draw.rotate_about([0.0, 1.0, 0.0], -entity.yaw, [0.0; 3]);
        draw.rotate_about([1.0, 0.0, 0.0], entity.pitch, [0.0; 3]);
        let face = Some(CubeFace::new("example:entity/bolt"));
        draw.cube([-1.0, -1.0, -4.0], [1.0, 1.0, 4.0], [face; 6], 0xFFFFFF);
        Ok(())
    }
}
```

| `EntityRenderState` | Meaning |
| --- | --- |
| `id` | The entity's network id, as mod messages name it. |
| `partial` | The frame's partial tick, 0 to 1. |
| `age` | Ticks lived plus the partial tick (`ageInTicks`). |
| `yaw`, `pitch` | The way it faces, in degrees, interpolated to the frame (0 is south, positive pitch looks down). |

The pose stack starts at the entity's interpolated position, in blocks, and is not turned. `draw`
also has the partial tick, the game time and the entity's light. What `submit` draws replaces the
type's own drawing; the glowing outline still draws around it. Every entity of the type shares the
one renderer, so keep per-entity values out of it: read them from the state. A renderer has no
`Host`; one that needs the world reads it in a hook and keeps what it read. A frame is drawn afresh,
so `draw.keep()` does nothing here.

## 2. Register it

```rust,ignore
fn register_entity_renderers(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterEntityRenderersEvent<'_>,
) {
    event.renderers.register("example:bolt".into(), Arc::new(Bolt));
}
```

The hook is asked once at start on the client, before resources load. The game keeps the
registrations and resolves each type's renderer on every resource load, the first one and every
reload (F3+T, a resource pack change), so a frame dispatches by slot, with no lookup by name and no
scan over the mods.

- A later registration for a type replaces the earlier one silently, whichever mod made it, as in
  NeoForge, also for a vanilla type.
- A type the game does not know is logged with its id and left out; nothing panics.
- `submit` returns `Ok(())` to keep what it drew. A renderer that returns an `Err`, panics, or (in a
  wasm mod) has its call refused or trapped is logged once and asked no more until the next resource
  load (a reload resolves it again); what it drew that frame is dropped, and until then its type draws
  as it would without one.

## What draws through registered renderers

The table covers every entity type, and each family's draw route reads it in turn. Today the content
mods' thrown types read it: a `data/<id>/horizon/entity_type/<name>.json` with the
`minecraft:snowball` template ([content mods](content-mods.md)) draws its renderer's geometry instead
of its flying item, and a thrown type with no renderer keeps its item. A registration for a type whose
family does not read the table yet (a boat, a display, a vanilla projectile, a mob) is kept silently,
and the type keeps its own drawing until its family does. Those families, the extra layers on living
renderers (`AddLayers`) and a mod's general entity types follow #860; they register the same way.
