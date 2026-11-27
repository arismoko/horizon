//! The `register_range_select_item_model_property` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Register the numeric `minecraft:range_dispatch` item model properties a mod adds (NeoForge's
    /// `RegisterRangeSelectItemModelPropertyEvent`). Asked once at start on the client, before
    /// resources load. Use [`RegisterRangeSelectItemModelPropertyEvent::register`] to bind each
    /// property identifier directly to its decode-time factory, which parses the `range_dispatch`
    /// node's JSON object once on each load/reload and returns the per-stack numeric callback. Any
    /// valid `<namespace>:<path>` identifier (including `minecraft:`) is accepted; a later
    /// registration for the same id replaces the earlier one (last-writer-wins), and registering a
    /// built-in vanilla property id overrides that built-in when decoding item definitions.
    #[notes = r#"## Numeric item model properties

A client mod adds a numeric `minecraft:range_dispatch` property (NeoForge's
`RegisterRangeSelectItemModelPropertyEvent`) once at client startup in
`Hooks::register_range_select_item_model_property` via `event.register(id, decode)`: `decode` parses the
`range_dispatch` node's JSON object (as a `Tag`) once on each load and reload and returns the bound
per-stack numeric callback.

```rust,ignore
fn register_range_select_item_model_property(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterRangeSelectItemModelPropertyEvent<'_>,
) {
    event.register(Identifier::parse("example:fill").unwrap(), |_config| {
        Ok(|_host, input| input.stack.count() as f32 / 64.0)
    });
}
```

Use `"property": "example:fill"` in the item's `minecraft:range_dispatch`: the model's `scale` multiplies the
number, then the highest threshold at or below it wins, else `fallback`, as for vanilla's properties. Any
valid `<namespace>:<path>` identifier (including `minecraft:`) is accepted; a later registration for the same
id replaces the earlier one (last-writer-wins, matching NeoForge's `LateBoundIdMapper`), and registering a
built-in vanilla property id overrides that built-in when decoding item definitions. Returning `Err(reason)`
from the decoder rejects the enclosing item definition without falling back to a built-in; panicking in the
decoder or per-stack callback disables that mod's callback for the session and surfaces an error toast. A
foreign property no mod registers reads 0, decided once at load."#]
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::register_range_select_item_model_property::native_register_range_select_item_model_property",
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        version = 2,
        contract = "collect",
        out = "properties",
        context = "registration",
        answered_by = "mod",
        registration_append = "custom",
        neoforge = "RegisterRangeSelectItemModelPropertyEvent"
    )]
    fn register_range_select_item_model_property(
        properties: &mut Vec<Answered<RangeSelectPropertyRegistration>>,
    );
}
