//! The `ingredient` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Define one occurrence of a custom ingredient named by a non-`minecraft:` recipe entry.
    /// Return `None` for a kind you do not handle, or an [`IngredientDefinition`] with the
    /// ordered candidate IDs and a predicate that can capture the parsed metadata. The first
    /// enabled mod to answer, in dispatch order, owns the predicate. Returning `None` lets the
    /// next listener answer. If nobody answers, the recipe is refused with its file and kind.
    ///
    /// `data` is the entry's JSON converted to an NBT compound. Booleans become `Byte` 1/0;
    /// exact signed 64-bit integers (including `1.0`, `1e3` and `-0.0`) take the smallest of
    /// `Byte`, `Short`, `Int` and `Long`. Other numbers become `Float` when exact in `f32`,
    /// otherwise `Double`. Read numeric tags by value across variants. Malformed data is refused
    /// before calling any listener.
    ///
    /// Called once per occurrence, including nested ingredients, in the load owning the recipes:
    /// `ClientPreload` on the client's own load or `ServerDataLoad` on the server. A reload asks
    /// again; tables and network projections reuse the same answer. No field in the recipe JSON
    /// supplies an answer, and only data, not the predicate, is sent to clients. A client joining
    /// a remote server is not told `ServerDataLoad`.
    ///
    /// Duplicates are removed in first-seen order. A nonempty list filters out other items before
    /// the predicate runs. An empty answer keeps manual matching without an item filter but has
    /// no placement candidates for recipe-book placement, property sets or stonecutter selection.
    /// The predicate runs as its defining mod on the matching side, with the side's read-only
    /// loop context and a stack borrowed for the call. If that mod is disabled after load,
    /// the predicate rejects without invoking it.
    #[hook(
        side = "both",
        contract = "replace",
        out = "definition",
        answered_by = "mod",
        context = "data_load",
        version = 4,
        target = "sunrise_sim::crafting::native_hooks::native_ingredient",
        neoforge = "ICustomIngredient"
    )]
    fn ingredient(
        kind: &str,
        data: &sunrise_core::nbt::Compound,
        definition: &mut Option<mod_api::Answered<IngredientDefinition>>,
    );
}
