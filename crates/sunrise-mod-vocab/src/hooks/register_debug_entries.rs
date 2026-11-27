//! The `register_debug_entries` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Register F3 debug entries once at client startup (NeoForge's `RegisterDebugEntriesEvent`).
    /// Each mod receives an empty batch. Push `DebugEntry::text` with its line callback or
    /// `DebugEntry::renderer` for a renderer toggle. Call [`DebugEntry::with_reduced_debug_info`]
    /// to opt an entry into showing under reduced debug info (default `false`, matching vanilla's
    /// `DebugScreenEntry.isAllowed`). Text callbacks run only while their entry is visible.
    /// Malformed or duplicate IDs (including built-in vanilla entry IDs) are logged and left out.
    #[notes = r#"## Debug screen entries

`Hooks::register_debug_entries` is the client registration-phase counterpart of NeoForge
26.3's `RegisterDebugEntriesEvent`. It receives a `RegistrationContext` once at client startup
and gives each mod an empty `event.entries` batch. Use any valid `<namespace>:<path>`
identifier (including custom `minecraft:<path>` IDs that do not collide with built-in vanilla
entries). A malformed or duplicate ID is logged and left out.

Declare text with `DebugEntry::text(id, group, lines)`. The `DebugGroup` chooses which
lines it joins: vanilla's `Position`, `Light`, `Memory`, `LookingAtBlock`, `LookingAtEntity`,
`LookingAtFluid` or `ChunkGeneration`, or `Named(id)`, a group of the mod's own that every
entry naming the same identifier joins. The client owns the callback for its lifetime. Only
while the entry is visible, the client calls the callback under the registering mod's identity,
with the current `&dyn ClientHost` and a `&mut Vec<String>` to append lines to. Legacy colour
codes (§) work. If the callback appends nothing, nothing joins the group. The lines are
drawn locally, never uploaded or recorded.

Declare a renderer switch with `DebugEntry::renderer(id)` (vanilla's `DebugEntryNoop`). It has
no text group or callback. Query its live state during
[`Hooks::register_debug_renderers`](#debug-renderers) with
`event.is_debug_entry_enabled(id)`.

Each entry appears in the corresponding text or renderer section of the debug options,
sorted by ID with vanilla's, and starts off. Its status (`alwaysOn`, `inOverlay`, `never`) is
saved in `debug-profile.json` under the full ID. By default, reduced debug info hides an entry
(matching vanilla's `DebugScreenEntry.isAllowed`); call `.with_reduced_debug_info(true)` on a
`DebugEntry::text` or `DebugEntry::renderer` to keep it allowed under reduced debug info.
Native text is appended after vanilla's entries in registration order."#]
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::register_debug_entries::native_register_debug_entries",
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "entries",
        context = "registration",
        answered_by = "mod",
        neoforge = "RegisterDebugEntriesEvent",
        version = 3
    )]
    fn register_debug_entries(entries: &mut Vec<Answered<DebugEntry>>);
}
