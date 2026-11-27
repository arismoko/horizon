//! Entity model layer registration, baking and drawing.

pub trait Hooks {
    /// Register suppliers for entity model layers (NeoForge's
    /// `EntityRenderersEvent.RegisterLayerDefinitions`). Called once at client startup.
    /// Use [`RegisterLayerDefinitionsEvent::register`] to bind each layer ID to its supplier;
    /// accepted suppliers run once when the layer table is frozen. A supplier panic is
    /// logged and shown in a client error toast naming its mod; other suppliers still run.
    /// Bake the realized definitions through `ClientHost::bake_layer`. API @API:model_layers@.
    #[notes = r#"## Entity model layers

`Hooks::register_layer_definitions` is the client registration-phase counterpart of NeoForge 26.3's
`EntityRenderersEvent.RegisterLayerDefinitions`. It receives a `RegistrationContext` once at
client startup. Call `event.register(id, supplier)` to stage a supplier that builds a
`LayerDefinition`. Dedicated and integrated server roles do not fire this client event.
IDs have the form `<namespace>:<path>#<layer>`. Write the model `Identifier` in its full
`namespace:path` form even when a half is empty; `<layer>` is any suffix after the first `#`. Any valid ID that does not collide
with an existing vanilla layer definition is accepted; duplicate mod registrations for the same ID
overwrite earlier suppliers (last-writer-wins) before freeze. Malformed IDs, collisions with
existing vanilla layer definitions, or an unavailable/invalid vanilla `entity-models.json` key
table fail startup freeze before any supplier runs or the layer table publishes.

The game calls each winning supplier once when assembling the frozen layer table. Overwritten
suppliers never run. A supplier that is disabled, refuses or panics leaves no definition.
A panic names the mod and `build_layer_definition` in the shared error log and client error
toast; other suppliers, including that mod's other suppliers, still run. Bakes and resource
reloads do not call suppliers again.

A `LayerDefinition` holds a `MeshDefinition` of named parts whose `CubeListBuilder` boxes sit
at a `PartPose`, on a texture of the given size. Texture offsets, mirroring and
`CubeDeformation` match vanilla's model builders. This listener is compiled in the API tests:

```rust,ignore
{{#include ../../../../crates/sunrise-client/tests/it/mod_api_hooks.rs:register_layer_definitions}}
```

`ClientHost::bake_layer` bakes a registered layer or a vanilla layer by its
`entity-models.json` name (`minecraft:zombie#main`) into an owned `ModelPart` tree. All
layers are indexed once per client resource load and reload rather than read from disk on
each bake; callers should still bake once and reuse the owned `ModelPart`, calling
`reset_all_poses` and setting the parts' positions and rotations each frame as `setupAnim`
does.

Draw the tree with `BlockEntityDrawContext::model_part`, using the same draw context the mob
or block entity submit hook received. There is no separate Host drawing helper. Positions
are texels, +y down and the front towards -z; convert the mob's frame with scale
`[1.0, -1.0, -1.0]` then translation `[0.0, -1.501, 0.0]`, as `LivingEntityRenderer` does.
The sprite is the entity texture ID and the tint is `0xRRGGBB`."#]
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::register_layer_definitions::native_register_layer_definitions",
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "layers",
        context = "registration",
        answered_by = "mod",
        registration_append = "custom",
        neoforge = "EntityRenderersEvent.RegisterLayerDefinitions",
        version = 5
    )]
    fn register_layer_definitions(layers: &mut Vec<Answered<LayerDefinitionRegistration>>);
}
