//! The `register_debug_renderers` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Collect world-space debug renderers at each client renderer-list refresh (NeoForge's
    /// `RegisterDebugRenderersEvent`). Push `DebugRenderer::new(draw)` for each renderer that
    /// should run until the next refresh. Use `event.is_debug_entry_enabled(id)` to gate a
    /// renderer on a vanilla or startup `DebugEntry` switch, or push unconditionally.
    #[notes = r#"## Debug renderers

`Hooks::register_debug_renderers` is the client counterpart of NeoForge 26.3's
`RegisterDebugRenderersEvent`, and like it fires during vanilla's
`DebugRenderer.refreshRendererList`: once as the client starts, on each login, on the first view
of a world since the start, and on the next view of a world after the debug selection changes.
That is a status set in the debug options or by a key, a profile loaded, the F3 overlay opened or
closed, or reduced debug info rebuilt. Respawn also invalidates the list for the next view.
Drawing the title or connect screen doesn't refresh anything. The hook receives a
`RegistrationContext`, so it reads no world. Each mod receives an empty `event.renderers`
batch and pushes `DebugRenderer::new(draw)` per active renderer.

Each refresh replaces the active list, so push every renderer that should draw until the next
refresh and capture its per-refresh state in the callback. To gate a renderer on a debug
options switch (either a built-in vanilla entry such as `"minecraft:chunk_borders"` or a
startup `DebugEntry::renderer(id)` from `Hooks::register_debug_entries`), check
`event.is_debug_entry_enabled(id)` before pushing `DebugRenderer::new(draw)`. Unconditional
renderers require no switch entry. Every submitted renderer from an enabled mod is kept in
registration order, with no ID-based filtering or deduplication.

For each world view, the client calls the installed callbacks in registration order,
under their registering mods' identities, after `submit_custom_geometry`. It passes the current
`ClientHost`, `WorldRenderView` (lending the frame's `Camera`, world partial tick, game time,
improved-transparency state, and visible `RenderableSection` slice) and the same
`WorldDrawContext`, so [its submission rules](#custom-world-geometry) apply and are shared.
Output clears every view, and callbacks don't run on the title or connect screens."#]
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::register_debug_renderers::native_register_debug_renderers",
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "renderers",
        context = "registration",
        answered_by = "mod",
        neoforge = "RegisterDebugRenderersEvent",
        version = 4
    )]
    fn register_debug_renderers(
        entries: &dyn mod_api::__private::DebugEntryStates,
        renderers: &mut Vec<Answered<DebugRenderer>>,
    );
}
