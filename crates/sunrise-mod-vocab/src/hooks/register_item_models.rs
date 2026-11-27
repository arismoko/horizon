//! The `register_item_models` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Register the item model node types a mod adds (NeoForge's `RegisterItemModelsEvent`). Asked
    /// once at start on the client, before resources load. Use [`RegisterItemModelsEvent::register`]
    /// to bind each type identifier directly to its decode-time factory, which parses the node's
    /// JSON object through [`ItemModelConfig`] once on each load/reload, declares any nested child item models through
    /// [`ItemModelChildren`], and returns the per-stack callback that pushes [`ItemModelLayer`]
    /// choices referencing those child keys. Any valid [`Identifier`] (including `minecraft:`) is
    /// accepted; a later registration for the same id replaces the earlier one (last-writer-wins),
    /// and registering a built-in vanilla or engine item model type id overrides that built-in when
    /// decoding item definitions.
    #[notes = r#"## Item model types

An item definition (`assets/<namespace>/items/<path>.json`) is a tree of model nodes: vanilla's `minecraft:model`,
`condition`, `select`, `range_dispatch`, `composite`, `special` and the rest. A mod adds a node type of its own
(NeoForge's `RegisterItemModelsEvent`) once at client startup in `Hooks::register_item_models` via
`event.register(id, decode)`: `decode` owns the node's schema and reads its original JSON object through
`ItemModelConfig::field` or `ItemModelConfig::decode` once on each load and reload. It
declares nested child item models on `children: &mut dyn ItemModelChildren` (`add_named_models`, `add_field`,
`add_list`, `add_object`, or `add_model`), and returns the bound per-stack callback.

Child models registered through `children` are ordinary item model nodes, decoded and baked by the game.
Per stack, the bound callback pushes an `ItemModelLayer` for each child key to draw, in order:

```rust,ignore
fn register_item_models(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterItemModelsEvent<'_>,
) {
    event.register(
        Identifier::parse("example:charge").unwrap(),
        |_config, children| {
            children.add_named_models()?;
            Ok(|_host, input, layers| {
                let name = if input.stack.count() > 1 { "full" } else { "empty" };
                layers.push(ItemModelLayer::new(name));
            })
        },
    );
}
```

```json
{"model": {"type": "example:charge",
  "models": {"empty": {"type": "minecraft:model", "model": "example:item/cell_empty"},
             "full": {"type": "minecraft:model", "model": "example:item/cell_full"}}}}
```

Call `children.use_transformation()?` to decode the standard `transformation` field and apply it to every model the node draws. Otherwise no standard transform is read; your schema can use the name for something else. A key not declared on `children` draws nothing.
A layer also owns whose display transform its models draw under: `ItemModelLayer::new` keeps each model's
own transform for the display context (a hand, a screen, the ground), while
`.with_item_transform(ItemTransformMode::None)` draws its models, and every model nested under them, with no
display transform, only centred as every item is (use it where the node sits in a model that already placed
the item, so the transform is not applied twice).

Any valid `Identifier` (including `minecraft:`) is accepted; a later registration for the same id replaces
the earlier one (last-writer-wins, matching NeoForge's `LateBoundIdMapper`), and registering a built-in
vanilla or engine item model type id overrides that built-in when decoding item definitions (including
vanilla schemas whose child models live in arrays or arbitrary JSON fields rather than a `models` object).
Returning `Err(reason)` from the decoder rejects the enclosing item definition without falling back to a
built-in; panicking in the decoder or per-stack callback disables that mod's callback for the session and
surfaces an error toast. A node type no mod registered is an error naming it: the item definition holding it
isn't loaded, and the item draws the missing model."#]
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::register_item_models::native_register_item_models",
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        version = 2,
        contract = "collect",
        out = "models",
        context = "registration",
        answered_by = "mod",
        registration_append = "custom",
        neoforge = "RegisterItemModelsEvent"
    )]
    fn register_item_models(models: &mut Vec<Answered<ItemModelRegistration>>);
}
