//! The `register_item_tint_sources` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Register the item tint sources a mod adds (NeoForge's
    /// `RegisterColorHandlersEvent.ItemTintSources`). Asked once at start on the client, before
    /// resources load. Use [`RegisterItemTintSourcesEvent::register`] to bind each tint source
    /// identifier directly to its decode-time factory, which parses the tint entry's JSON object
    /// once on each load/reload and returns the per-stack ARGB colour callback. Any valid
    /// `<namespace>:<path>` identifier (including `minecraft:`) is accepted; a later registration
    /// for the same id replaces the earlier one (last-writer-wins), and registering a built-in
    /// vanilla tint source id overrides that built-in when decoding item definitions.
    #[notes = r#"## Item tint sources

An item tint source (NeoForge's `RegisterColorHandlersEvent.ItemTintSources`) colours a greyscale item per
stack. Register it once at client startup in `Hooks::register_item_tint_sources` via
`event.register(id, decode)`: `decode` parses the tint entry's JSON object (as a `Tag`) once when an item
definition is loaded or reloaded and returns the bound per-stack ARGB callback.

```rust,ignore
fn register_item_tint_sources(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterItemTintSourcesEvent<'_>,
) {
    event.register(Identifier::parse("example:charge").unwrap(), |config| {
        let default = match config.get("default") {
            Some(Tag::Int(value)) => *value as u32,
            _ => 0xFFFFFF,
        };
        Ok(move |_host, input| {
            let rgb = if input.stack.count() > 1 { 0x55FF55 } else { default };
            0xFF000000 | rgb
        })
    });
}
```

Use it as an entry of a `minecraft:model`'s `tints`, `{"type": "example:charge", "default": 65280}`: the
entry's place in the list is the `tintindex` it colours. Any valid `<namespace>:<path>` identifier (including
`minecraft:`) is accepted; a later registration for the same id replaces the earlier one (last-writer-wins,
matching NeoForge's `LateBoundIdMapper`), and registering a built-in vanilla tint source id overrides that
built-in when decoding item definitions. Returning `Err(reason)` from the decoder rejects the enclosing item
definition without falling back to a built-in; panicking in the decoder or per-stack callback disables that
mod's callback for the session and surfaces an error toast. A foreign source no mod registers draws white,
decided once at load."#]
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::register_item_tint_sources::native_register_item_tint_sources",
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        version = 2,
        contract = "collect",
        out = "sources",
        context = "registration",
        answered_by = "mod",
        registration_append = "custom",
        neoforge = "RegisterColorHandlersEvent.ItemTintSources"
    )]
    fn register_item_tint_sources(sources: &mut Vec<Answered<ItemTintSourceRegistration>>);
}
