//! The `register_block_state_models` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Register the block-state definition types a mod adds (NeoForge's
    /// `RegisterBlockStateModels`, its `registerDefinition`). Asked once at start on the client,
    /// before resources load. Use [`RegisterBlockStateModelsEvent::register`] to bind each type
    /// identifier to its provider callback; a blockstate file whose `neoforge:definition_type`
    /// names a registered id invokes that provider directly on every load and reload. Any valid
    /// `<namespace>:<path>` identifier is accepted; a later registration for the same id replaces
    /// the earlier one.
    #[notes = r#"## Block-state definition types

A blockstate file (`assets/<namespace>/blockstates/<block>.json`) picks each state's model with vanilla's
`variants` or `multipart`. A mod registers a definition type of its own (NeoForge's
`RegisterBlockStateModels`) once at client startup via `Hooks::register_block_state_models`. Call
`event.register(id, provider)` to bind the type identifier directly to its provider callback; when a
blockstate file names it in `neoforge:definition_type`, the game invokes the bound provider on every load
and reload without scanning other mods.

```rust,ignore
fn register_block_state_models(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterBlockStateModelsEvent<'_>,
) {
    event.register(Identifier::parse("example:by_power").unwrap(), |input| {
        // {"neoforge:definition_type": "example:by_power", "on": "example:block/lamp_on", "off": "example:block/lamp_off"}
        let (Some(Tag::String(on)), Some(Tag::String(off))) =
            (input.config.get("on"), input.config.get("off"))
        else {
            return Err("needs \"on\" and \"off\" models".into());
        };
        Ok(input
            .states
            .iter()
            .enumerate()
            .map(|(state, properties)| {
                let lit = properties.get("powered").is_some_and(|value| value == "true");
                BlockStateModelPick::Model {
                    state: state as u32,
                    model: (if lit { on } else { off }).to_string_lossy(),
                    turns: [0; 3],
                    uvlock: false,
                }
            })
            .collect())
    });
}
```

`BlockStateDefinitionInput` hands the provider the block's identifier (`input.block`), the file's whole JSON
object converted to a `Tag` (`input.config`), and the block's states in registry order (`input.states`),
each as its property names and values. A pick names a state by its index there. Any valid
`<namespace>:<path>` identifier (including `minecraft:`) is accepted; a later registration for the same ID
replaces the earlier one (last-writer-wins) with a warning naming both mods.

The file is refused whole, with its name and a reason logged, when the type is not registered, the provider
returns `Err(reason)` or pushes `BlockStateModelPick::Refuse`, a pick names a state the block lacks, names
one state twice or a model id that doesn't read, or no state is picked. Returning `Err` reports the refusal
each time without disabling the provider; panicking disables that provider for the session and surfaces an
error toast. A file several packs provide binds each pack's layer on its own, as vanilla does: a refused
layer leaves the states to the layers below it."#]
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::register_block_state_models::native_register_block_state_models",
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "definitions",
        context = "registration",
        answered_by = "mod",
        registration_append = "custom",
        neoforge = "RegisterBlockStateModels",
        version = 2
    )]
    fn register_block_state_models(
        definitions: &mut Vec<Answered<BlockStateDefinitionRegistration>>,
    );
}
