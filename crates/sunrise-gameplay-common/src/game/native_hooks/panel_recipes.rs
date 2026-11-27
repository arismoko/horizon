//! `RecipesClientHost::panel_recipes`: a mod's `RecipePage` shown in its item
//! panel's place, on the one path the panel's recipe view has (`native_panel_recipes`, then
//! `Panel::show_recipes`). The API's types are converted to the renderer's here, at the boundary.
use super::*;

/// The renderer's view of an API page: its title, tabs, selected tab and recipes, none of them cut
/// yet (`RecipePage::new` cuts them). `None` is no recipes, which shows the items again.
pub fn page_views(
    page: Option<mod_api::RecipePage>,
) -> (String, Vec<mod_api::Stack>, u32, Vec<NativeRecipeView>) {
    let Some(page) = page else {
        return (String::new(), Vec::new(), 0, Vec::new());
    };
    let views = page.recipes.into_iter().filter_map(recipe_view).collect();
    (page.title, page.tabs, page.tab, views)
}

fn recipe_view(view: mod_api::RecipeView) -> Option<NativeRecipeView> {
    let layout = match view.layout {
        mod_api::RecipeLayout::Grid => NativeRecipeLayout::Grid,
        mod_api::RecipeLayout::Cooking => NativeRecipeLayout::Cooking,
        mod_api::RecipeLayout::Single => NativeRecipeLayout::Single,
        mod_api::RecipeLayout::Smithing => NativeRecipeLayout::Smithing,
        mod_api::RecipeLayout::Brewing => NativeRecipeLayout::Brewing,
        _ => {
            log::warn!(
                "E_RECIPE_LAYOUT_UNSUPPORTED: panel_recipes refused an unsupported recipe layout"
            );
            return None;
        }
    };
    Some(NativeRecipeView {
        layout,
        width: view.width,
        height: view.height,
        slots: view.slots,
        results: view
            .results
            .into_iter()
            .map(|result| NativeRecipeResult {
                stacks: result.stacks,
                chance: result.chance,
            })
            .collect(),
        fluid_inputs: view.fluid_inputs,
        fluid_outputs: view.fluid_outputs,
        note: view.note,
        action: view.action,
    })
}
