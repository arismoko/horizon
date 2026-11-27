//! The `register_model_loaders` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Register the model loaders a mod adds (NeoForge's `ModelEvent.RegisterLoaders`). Asked once
    /// at start on the client, before resources load. Use [`RegisterModelLoadersEvent::register`]
    /// to bind each loader identifier to its schema and bake callback. Each model and set of
    /// resolved texture slots invokes that callback once per load or reload. The callback's quads
    /// replace the cuboid geometry unless the loader explicitly delegates to the cuboid decoder.
    /// An absent required loader rejects its declaring model file. A reference to that file uses
    /// the missing model at that link without discarding a descendant's own geometry.
    /// Duplicates and collisions with built-ins (`neoforge:composite`, `neoforge:obj`, `neoforge:empty`,
    /// `fusion:model`) are registration errors.
    #[notes = r#"## Model loaders

A model JSON may name a `loader`: NeoForge's `neoforge:composite`, `neoforge:obj` and `neoforge:empty` are built in, and a mod
registers one of its own (NeoForge's `ModelEvent.RegisterLoaders`) once at client startup via
`Hooks::register_model_loaders`. Call `event.register(id, schema, loader)` to bind a loader identifier to
its schema and bake callback. `ModelLoaderSchema::Custom` leaves every model field except `loader` to your
callback. Use `ModelLoaderSchema::ModelProperties` to decode standard fields such as `parent`, `textures`
and `display`, but supply your own geometry. Use `ModelLoaderSchema::Cuboids` to decode and bake the model's
cuboid elements alongside your quads.

```rust,ignore
fn register_model_loaders(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterModelLoadersEvent<'_>,
) {
    event.register(Identifier::parse("example:pillar").unwrap(), ModelLoaderSchema::ModelProperties, |_input| {
        // The north face of a 4-pixel post in the middle of the block.
        Ok(vec![ModelLoaderQuad {
            positions: [
                [0.625, 0.0, 0.375],
                [0.375, 0.0, 0.375],
                [0.375, 1.0, 0.375],
                [0.625, 1.0, 0.375],
            ],
            uvs: [[1.0, 1.0], [0.0, 1.0], [0.0, 0.0], [1.0, 0.0]],
            texture: ModelLoaderTexture::Slot("side".into()),
            cull: None,
            tint_index: -1,
            shade: true,
        }])
    });
}
```

`ModelLoaderInput` gives the callback the model identifier (`input.model`), the original JSON object as a
string (`input.config`), and the resolved standard texture slots (`input.textures`). A quad can name one of
those slots (`ModelLoaderTexture::Slot`) or directly name a sprite (`ModelLoaderTexture::Sprite`). Quads are
in block units, and the blockstate variant's rotation moves them whole. A model and effective slot set are
baked once per load, even when several variants reference them.

Any valid `<namespace>:<path>` identifier (including `minecraft:`) is accepted. Duplicate registrations and
collisions with `neoforge:composite`, `neoforge:obj`, `neoforge:empty` or `fusion:model` are registration errors; the first
registration stands. A loader ID written as a string, or as an object with `id` and no `optional` flag,
is required. When the loader is absent, only `"loader": {"id":"example:pillar","optional":true}` uses
ordinary cuboid geometry. An absent required loader rejects its declaring file and logs the error.
References to that file use the missing model at that link: a descendant can still supply its own
geometry. If a present loader returns `Err(reason)`, that bake uses the missing model and logs the
error. An empty successful quad list is not an error. Panicking also
disables that loader callback for the session and surfaces an error toast. A quad with a position or UV
that isn't finite, or a texture key the model doesn't define, is skipped by itself with a reason."#]
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::register_model_loaders::native_register_model_loaders",
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "loaders",
        context = "registration",
        answered_by = "mod",
        registration_append = "custom",
        neoforge = "ModelEvent.RegisterLoaders",
        version = 3
    )]
    fn register_model_loaders(loaders: &mut Vec<Answered<ModelLoaderRegistration>>);
}
