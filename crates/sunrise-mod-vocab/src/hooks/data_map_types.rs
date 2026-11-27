//! The `data_map_types` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The server is assembling its data tables, as for
    /// `data_pack_files` (a world's first open and every `/reload`):
    /// push the data map types your mod registers (NeoForge's
    /// `RegisterDataMapTypesEvent`). A type is a namespaced `id`
    /// (`namespace:path`) for a `registry`: any the game names. Its values come
    /// from the world's data packs'
    /// `data/<namespace>/data_maps/<registry folder>/<path>.json` (the
    /// registry's path for a `minecraft` registry, `<namespace>/<path>`
    /// otherwise), and from files you push through `native_data_pack_files`
    /// under the key `data_maps/<namespace>:<registry folder>/<path>`.
    /// Each file has a `values` object mapping entry IDs and `#tags` to JSON,
    /// with an optional boolean `replace` and a `remove` list. Entry conditions
    /// (`neoforge:conditions`) are evaluated before the value, including on
    /// direct objects. A malformed layer is logged and skipped as a whole;
    /// otherwise newer values override older ones. Push a typed map's
    /// `DataMap::ty` and use the key's `data`
    /// (`ItemKey::data`, `EntityTypeKey::data`...) for an owned decoded value.
    /// By name, `data_map_value` borrows a raw tag and `data_map_entries`
    /// borrows the whole slice in entry ID order. Both borrow from the host;
    /// copy values only if you need them after the borrow. Server-side, not
    /// synced to clients.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_data_map_types",
        dispatch = "flat",
        side = "server",
        version = 2,
        contract = "collect",
        out = "types",
        neoforge = "RegisterDataMapTypesEvent"
    )]
    fn data_map_types(types: &mut Vec<DataMapType>);
}
