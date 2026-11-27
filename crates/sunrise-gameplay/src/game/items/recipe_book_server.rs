use crate::game::ServerGame;
use mod_api::RecipeTransferCheck;

impl ServerGame<'_> {
    /// Asks `menu_transfer` of the part of mod menu type `menu` (the caller checked that one
    /// listens): its answer, and with `dry_run` the cells it named, both lists kept to the
    /// recipe's known layout (`clamp_cells` to `recipe_cells`: one input for stonecutting, the
    /// placement's slots, or `MAX_CHECK_CELLS` for a recipe the game can't inspect). Cells a
    /// part names outside a dry run are dropped. None when no part answers.
    pub(in crate::game::server_side) fn menu_transfer_answer(
        &mut self,
        player: u128,
        menu: &str,
        pos: Option<[i32; 3]>,
        recipe: &str,
        max: bool,
        dry_run: bool,
    ) -> Option<RecipeTransferCheck> {
        use sunrise_sim::lan::recipes::{clamp_cells, recipe_cells};
        let (mut result, mut missing, mut craftable) = (None, Vec::new(), Vec::new());
        self.native_menu_transfer(mod_api::__private::MenuTransferCall {
            player,
            menu,
            pos,
            recipe,
            max,
            dry_run,
            result: &mut result,
            missing_cells: &mut missing,
            craftable_cells: &mut craftable,
        });
        let result = result?;
        let cells = if dry_run {
            let recipes = self.recipes();
            let id = if recipe.contains(':') {
                std::borrow::Cow::Borrowed(recipe)
            } else {
                std::borrow::Cow::Owned(format!("minecraft:{recipe}"))
            };
            let kind = (recipes.by_type.iter())
                .find(|(_, list)| list.iter().any(|r| r.id == *id))
                .map(|(kind, _)| kind.as_str());
            recipe_cells(kind, recipes.placement(recipe).map(|p| p.slots.len()))
        } else {
            0
        };
        Some(RecipeTransferCheck {
            result,
            missing_cells: clamp_cells(&missing, cells),
            craftable_cells: clamp_cells(&craftable, cells),
        })
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;
