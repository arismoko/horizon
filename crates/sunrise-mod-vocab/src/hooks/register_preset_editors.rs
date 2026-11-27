//! World-preset editor registration.

pub trait Hooks {
    /// Register client editor factories for world-preset keys once at startup
    /// (NeoForge's `RegisterPresetEditorsEvent`). Use [`RegisterPresetEditorsEvent::register`] to
    /// bind each preset identifier directly to its factory. Any valid `<namespace>:<path>`
    /// identifier (including `minecraft:`) is accepted; a later registration for the same preset
    /// replaces the earlier one (last-writer-wins), and registering a vanilla preset
    /// (`minecraft:flat`, `minecraft:single_biome_surface`) overrides its built-in editor.
    /// Unregistered presets retain their built-in editor, or their disabled Customize button.
    #[notes = r#"## World preset editors

`register_preset_editors` is the client registration-phase counterpart of NeoForge's
`RegisterPresetEditorsEvent`. Call `event.register(preset, factory)` to bind a world-preset
identifier directly to a factory that creates a [`PresetEditor`] when the player clicks
Customize on Create World. The preset itself comes from the enabled data packs'
`worldgen/world_preset` JSON; registering an editor alone does not create a preset. Add the
preset to `data/minecraft/tags/worldgen/world_preset/normal.json` to offer it in Create
World's type selector; unlisted presets remain hidden.

Put editor-specific input and drawing in the bound `PresetEditor`. Generic screen hooks can identify
its screen as `world_preset_editor`. Input first reaches the corresponding screen Pre hook; characters
from a key, the input method, or the control socket reach `PresetEditor::char_typed` independently of
key cancellation. Keys consumed by the input method during composition do not reach the editor or
its footer; committed text uses the character path. An unhandled character reaches CharacterTyped.Post;
an unhandled key reaches KeyPressed.Post after the editor and the normal title controls decline it.

```rust,ignore
struct IslandsEditor;

impl PresetEditor for IslandsEditor {
    fn title(&self) -> Option<String> {
        Some("Islands Customization".into())
    }

    fn render(
        &mut self,
        draw: &mut dyn DrawContext,
        rect: [f32; 4],
        _draft: &mut PresetEditorDraft<'_>,
    ) {
        draw.fill([rect[0] + 20.0, rect[1] + 40.0, 80.0, 20.0], 0xFF00FF00);
    }
}

fn register_preset_editors(
    &self,
    _: &RegistrationContext,
    event: &mut RegisterPresetEditorsEvent<'_>,
) {
    event.register(Identifier::parse("example:islands").unwrap(), |_ctx| {
        Ok(Box::new(IslandsEditor))
    });
}
```

Any valid `<namespace>:<path>` identifier (including `minecraft:`) is accepted. A later
registration for the same preset replaces the earlier one (last-writer-wins), and registering
`minecraft:flat` or `minecraft:single_biome_surface` overrides the built-in Superflat or
Single Biome editor.

Customize opens `world_preset_editor`, with the menu background, header title, and Done and
Cancel footer buttons supplied by the game. The bound [`PresetEditor`] draws its UI before
`screen_render_post`, handles mouse clicks, keys, and typed characters, and edits the working
dimension stems through `PresetEditorDraft::set_dimensions`. Screen hooks still fire on
`world_preset_editor`, and `host.preset_editor_dimensions()` /
`host.set_preset_editor_dimensions(...)` remain available while it is open. Done commits the
validated draft to world creation; Cancel and Escape discard it. Neither path changes an
existing world. Keep custom drawing above the bottom 33 GUI pixels reserved for Done and
Cancel.

Returning `Err(reason)` from a factory logs a warning and leaves Create World open without
disabling the factory. Panicking in a factory or editor callback disables that mod's preset
editor callbacks for the session and surfaces an error toast. Dimension ids, stem structure
and flat-layer height bounds are checked before accepting an edit; registry resolution and
generator-specific decoding still run when opening the world.
"#]
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::register_preset_editors::native_register_preset_editors",
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        version = 2,
        contract = "collect",
        out = "editors",
        context = "registration",
        answered_by = "mod",
        registration_append = "custom",
        neoforge = "RegisterPresetEditorsEvent"
    )]
    fn register_preset_editors(editors: &mut Vec<Answered<PresetEditorFactoryRegistration>>);
}
