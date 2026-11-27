//! Client texture atlases a mod adds before each atlas build.

pub trait Hooks {
    /// Declare atlases beside the game's (`RegisterTextureAtlasesEvent`). Call
    /// `event.register(atlas)` with each `TextureAtlasRegistration`, which names the atlas
    /// definition that lists its sprites. Called at first client load and every resource reload,
    /// before textures are stitched.
    #[notes = r#"## Texture atlases

`register_texture_atlases` receives a registration context. Call `event.register(atlas)`
with a `TextureAtlasRegistration` giving an atlas id (`mymod:icons`) and its definition
(`mymod:icons` reads `assets/mymod/atlases/icons.json`). The definition is a `sources`
list like vanilla's atlas definitions, and resource packs change it the same way.

Each registered atlas has its own texture, stitched independently from the base
HUD and other registered atlases. Its sprites are exposed in the GUI lookup under
`TextureAtlasRegistration::sprite_name`:
sprite `mymod:heart` of `mymod:icons` is `mymod:gui/atlases/icons/mymod/heart`.
Look it up with `gui_sprite` in `texture_atlas_stitched` for `TextureAtlas::Gui`
and draw it with `DrawContext::sprite` by that name. Every reload lists the
sprites again from the definition as the packs now give it.

The hook runs at client startup and on the game-owning thread before reload
staging; re-submit the atlases on each call. Any namespace is allowed, including
`minecraft:` and `sunrise:`. Collisions with vanilla 26.3's built-in engine
atlases (`AtlasManager.KNOWN_ATLASES`, such as `minecraft:blocks` or
`minecraft:gui`) on either `definition` or `atlas`, and duplicate `definition`
or `atlas` ids, are logged as errors naming the registering mod and earlier
owner, keeping the first registration (matching NeoForge's
`ClientHooks.gatherTextureAtlases` and `RegisterTextureAtlasesEvent.register`).
There is no synthetic atlas or sprite count cap; only the GPU device's actual
`max_texture_dimension_2d` limit applies, with per-atlas isolation and
diagnostics so an overflowing or bad atlas never shrinks unrelated model
textures or discards other atlases or sprites. A registered atlas sprite
replaces a raw pack file at the same staged path with a warning (matching
vanilla's `TextureManager.register`); if two registered atlases stage the same
sprite path, the first stands and a warning names both."#]
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::register_texture_atlases::native_register_texture_atlases",
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "atlases",
        answered_by = "mod",
        registration_append = "register",
        context = "registration",
        neoforge = "RegisterTextureAtlasesEvent",
        version = 4
    )]
    fn register_texture_atlases(atlases: &mut Vec<Answered<TextureAtlasRegistration>>);
}
