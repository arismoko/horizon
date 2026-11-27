//! Load and match hook adapters at the engine boundary.
use mod_api::{IngredientDefinition, RecipePropertyAddition};
use sunrise_world::native_load_context::LoadContext;

/// Ask for this occurrence's custom ingredient definition before parsing the recipe tables.
/// The first answer supplies ordered candidates and the owning mod's match predicate.
#[inline(never)]
pub fn native_ingredient(
    context: &sunrise_world::native_load_context::LoadContext,
    kind: &str,
    data: &sunrise_core::nbt::Compound,
    definition: &mut Option<mod_api::Answered<IngredientDefinition>>,
) {
    mod_api::dispatch_ingredient(
        &sunrise_level::api_context::LoadReader(context).context(),
        kind,
        data,
        definition,
    );
}
/// Items a mod's own recipe type takes in a station slot, added to the
/// recipe property sets (`RecipeManager.RECIPE_PROPERTY_SETS`): what a
/// furnace, blast furnace, smoker, campfire, smithing table slot or brewing
/// stand slot accepts, and what a client predicts it accepts. Listeners call
/// `event.register(addition)` once per set to add to; the vanilla recipes' own
/// items stay in. Called once per load, after the recipes are resolved, with
/// the `DataLoadContext` of the load that owns them (`origin()`
/// `ServerDataLoad` on a server, `ClientPreload` on a client's own load),
/// and again after `/reload`. New keys in manifest-owned or served namespaces
/// create custom input sets. Unknown vanilla keys, invalid or foreign new keys
/// and air are logged with the contributing mod and skipped; unregistered items
/// are ignored. The answer must be the same every time.
#[inline(never)]
pub fn native_recipe_property_sets(
    context: &LoadContext,
    additions: &mut Vec<mod_api::Answered<RecipePropertyAddition>>,
) {
    mod_api::dispatch_recipe_property_sets(
        &sunrise_level::api_context::LoadReader(context).context(),
        additions,
    );
}
/// Whether two of `stack`'s item may be combined in a crafting grid to repair it
/// (`IItemExtension.isCombineRepairable`, which `RepairItemRecipe.canCombine` asks) is being
/// decided: `repairable` starts with vanilla's answer, whether the stack has durability, and
/// the parts of the stack's item may change it. Asked inside recipe matching on both sides.
#[inline(never)]
pub fn native_is_combine_repairable(
    context: &sunrise_world::native_load_context::LoopContext,
    stack: mod_api::ItemStackView<'_>,
    repairable: &mut bool,
) {
    mod_api::dispatch_is_combine_repairable(
        &sunrise_level::api_context::LoopReader(context).context(),
        stack,
        repairable,
    );
}
