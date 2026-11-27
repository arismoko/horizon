//! The `recipe_property_sets` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Items a mod's own recipe type takes in a station slot, added to the
    /// recipe property sets (`RecipeManager.RECIPE_PROPERTY_SETS`): what a
    /// furnace, blast furnace, smoker, campfire, smithing table slot or brewing
    /// stand slot accepts, and what a client predicts it accepts. Call
    /// [`RecipePropertySetsEvent::register`] once per set to add to; the vanilla
    /// recipes' own items stay in. Called once per load, after the recipes are
    /// resolved, with the `DataLoadContext` of the load that owns them (`origin()`
    /// `ServerDataLoad` on a server, `ClientPreload` on a client's own load),
    /// and again after `/reload`. Any valid namespaced key creates or extends a
    /// custom set (including non-vanilla `minecraft:` keys); bare keys name
    /// vanilla sets. Invalid keys, unknown bare vanilla keys, and air are logged
    /// and skipped, naming the contributor; unknown items are ignored. Later additions
    /// do not change the first contributor. Custom sets are synced to clients
    /// alongside vanilla sets. The answer must be the same every time.
    /// A client without this mod predicts vanilla here; the server corrects it.
    #[hook(
        version = 4,
        side = "both",
        contract = "collect",
        out = "additions",
        registration_append = "attributed",
        context = "data_load",
        target = "sunrise_sim::crafting::native_hooks::native_recipe_property_sets",
        neoforge = "RegisterRecipePropertiesEvent"
    )]
    fn recipe_property_sets(additions: &mut Vec<Answered<RecipePropertyAddition>>);
}
