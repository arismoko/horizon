//! The `create_skull_models` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Skull models for a mod's own skull types (NeoForge's
    /// `EntityRenderersEvent.CreateSkullModels`): call `event.register(model)` with one
    /// `SkullModel` per type, giving its `kind`, model layer and default texture. A
    /// `minecraft:head` item model may then name the kind, so the head draws in hand, in a GUI
    /// and in an item frame. Called once at client startup. Any valid canonical
    /// `<namespace>:<path>` identifier (including `minecraft:` for non-vanilla paths) is accepted;
    /// invalid identifiers, the seven built-in vanilla skull types (`minecraft:skeleton`,
    /// `minecraft:wither_skeleton`, `minecraft:player`, `minecraft:zombie`, `minecraft:creeper`,
    /// `minecraft:piglin`, `minecraft:dragon`), duplicate registrations, entries with a layer
    /// neither a mod nor vanilla has, or entries with an empty texture are logged and left out.
    /// API @API:create_skull_models@.
    #[notes = r#"## Skull models

`Hooks::create_skull_models` is the client registration-phase counterpart of NeoForge 26.3's
`EntityRenderersEvent.CreateSkullModels`. It receives a `RegistrationContext`, once at client
startup after `layer_definitions`. Call `event.register(model)` with a `SkullModel` per skull
type: its `kind` (`mymod:goblin`), the model layer the head is baked from, and the texture
used when the item model names none (vanilla's `SkullBlockRenderer.SKIN_BY_TYPE`).

The layer is one a mod registered in `layer_definitions` (`mymod:goblin_head#main`) or a vanilla
layer by its `entity-models.json` name (`minecraft:zombie_head#main`). It is baked once, at
registration. Its `head` part is posed as vanilla's skull heads are (no turn); a mod type has no
animation, as only vanilla's dragon and piglin heads animate.

This listener is compiled in the API tests:

```rust,ignore
{{#include ../../../../crates/sunrise-client/tests/it/mod_api_hooks.rs:create_skull_models}}
```

A registered kind is a name a `minecraft:head` item model can use, exactly as a vanilla one:

```json
{ "model": { "type": "minecraft:special", "base": "mymod:item/goblin_head",
  "model": { "type": "minecraft:head", "kind": "mymod:goblin" } } }
```

The head then draws wherever that item model draws: in hand, in a GUI, in an item frame and
dropped. Worn in a head slot it draws as other head items do, in the item's `head` display
transform; the skull's own placement on a head, like a placed skull, needs the item to be a mod
skull block. The model's optional `texture` replaces the default as it does for
vanilla's kinds. A kind nobody registered is refused when the item model loads, as an unknown
vanilla kind is.

Any valid canonical `<namespace>:<path>` identifier (including `minecraft:` for non-vanilla paths)
is accepted. An invalid identifier, one of the seven built-in vanilla skull types
(`minecraft:skeleton`, `minecraft:wither_skeleton`, `minecraft:player`, `minecraft:zombie`,
`minecraft:creeper`, `minecraft:piglin`, `minecraft:dragon`), or a kind registered already (by any
mod) is logged as an error and left out, as `registerSkullModel` refuses duplicate types. So is an
entry whose layer neither a mod nor vanilla has, or whose texture is empty. Skull types are frozen
once at startup. Placed skull blocks of a mod type are not drawn by this hook: a mod block
declaring that it is a skull is not covered yet."#]
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::create_skull_models::native_create_skull_models",
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "models",
        registration_append = "register",
        context = "registration",
        neoforge = "EntityRenderersEvent.CreateSkullModels",
        version = 3
    )]
    fn create_skull_models(models: &mut Vec<SkullModel>);
}
