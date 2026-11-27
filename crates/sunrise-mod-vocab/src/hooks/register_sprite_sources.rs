//! The `register_sprite_sources` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Register the atlas sprite source types a mod adds (NeoForge's `RegisterSpriteSourcesEvent`).
    /// Asked once at start on the client, before resources load. Use
    /// [`RegisterSpriteSourcesEvent::register`] to bind each type identifier directly to its
    /// provider callback; an entry of an atlas definition's `sources` whose `type` is a registered
    /// id invokes that provider at its place in the list, on every load and reload. Any valid
    /// `<namespace>:<path>` identifier (including `minecraft:`) is accepted; a later registration
    /// for the same id replaces the earlier one, and registering a built-in vanilla source id
    /// overrides that built-in when decoding atlas definitions.
    #[notes = r#"## Sprite sources

An atlas definition (`assets/<namespace>/atlases/<path>.json`) lists the sources that give the atlas its
sprites: vanilla's `directory`, `single`, `filter`, `unstitch` and `paletted_permutations`. A mod registers a
source type of its own (NeoForge's `RegisterSpriteSourcesEvent`) once at client startup via
`Hooks::register_sprite_sources`. Call `event.register(id, provider)` to bind the type identifier directly to
its provider callback; when an atlas definition lists an entry of that type, the game invokes the bound
provider on every load and reload without scanning other mods.

```rust,ignore
fn register_sprite_sources(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterSpriteSourcesEvent<'_>,
) {
    event.register(Identifier::parse("example:checker").unwrap(), |input| {
        // A JSON number arrives as the narrowest tag that holds it: `8` is a `Tag::Byte`.
        let side = match input.config.get("size") {
            Some(Tag::Byte(size)) => (*size).clamp(1, 64) as u32,
            _ => 16,
        };
        let rgba = (0..side * side)
            .flat_map(|i| {
                if (i / side + i % side) % 2 == 0 {
                    [255, 0, 255, 255]
                } else {
                    [0, 0, 0, 255]
                }
            })
            .collect();
        Ok(vec![SpriteSourceOutput::Pixels {
            sprite: Identifier::parse("example:checker").unwrap(),
            width: side,
            height: side,
            rgba,
        }])
    });
}
```

Use it as an entry of a definition's `sources`, `{"type": "example:checker", "size": 8}`: `SpriteSourceInput`
hands the provider the atlas identifier (`input.atlas`), the entry's whole JSON object converted to a `Tag`
(`input.config`), and read-only access to the load's resources (`input.resources`). A provider returns
`Ok(outputs)` of `SpriteSourceOutput`, which apply in order as vanilla's `SpriteSource.Output` calls do:
`Texture` adds a texture of the resources as a sprite, `Pixels` adds pixels the mod made, and `Remove` drops
a sprite by id (vanilla's `removeAll`). A sprite of an id listed already replaces the earlier one, as a
built-in source's does. A source never sees what the entries before it listed: output is write-only.

Any valid `<namespace>:<path>` identifier (including `minecraft:`) is accepted; a later registration for the
same id replaces the earlier one (last-writer-wins, matching NeoForge's `LateBoundIdMapper`) with a warning
naming both mods. Registering a built-in vanilla type (`minecraft:directory`, `minecraft:single`,
`minecraft:filter`, `minecraft:unstitch`, `minecraft:paletted_permutations`) overrides that built-in when
decoding atlas definitions, with a warning naming the overriding mod. Returning `Err(reason)` logs the error
and lists no sprites for that entry without disabling the provider; panicking disables that provider for the
session and surfaces an error toast.

`input.resources` is the `ResourceManager` vanilla hands `SpriteSource.run`, read-only: `read(path)` gives
the winning copy of a resource (`assets/<namespace>/textures/<path>.png`), `read_stack(path)` every pack's
copy lowest first, and `list(prefix)` the paths under a prefix. A source can make a sprite from the pixels
of a texture any pack has, a player's retexture included. It is borrowed for the single callback invocation.

Each entry is asked once per atlas each time the client prepares its resources, and the answer is kept
for everything that reads the atlas, so a source that counts or varies its answers is still seen as one
answer. Sprite sources run in every client atlas that lists them (including blocks, items, GUI and
particles, and atlases declared by `register_texture_atlases`). Each atlas is asked under its own id, so
two atlases sharing a definition each get their own question. Each atlas keeps its own sprites, by its own id: when the blocks
and items atlases both list an id, a model's material is the items atlas's, and `texture_sprites` finds
the blocks atlas's sprite of that id (and every sprite a source made in the blocks atlas, though no model
names it) after `texture_atlas_stitched`. A sprite only the items atlas lists has no block array to be found
in. An entry whose type no mod registered fails its definition, as an unknown type fails it in vanilla, and
the log names the type and the entry's place. A `Texture` the resources lack, and `Pixels` whose length
isn't `width * height * 4` or whose side is over 32768 pixels, are logged and skipped one by one."#]
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::register_sprite_sources::native_register_sprite_sources",
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        version = 3,
        contract = "collect",
        out = "sources",
        context = "registration",
        answered_by = "mod",
        registration_append = "custom",
        neoforge = "RegisterSpriteSourcesEvent"
    )]
    fn register_sprite_sources(sources: &mut Vec<Answered<SpriteSourceRegistration>>);
}
