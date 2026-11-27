//! A mod menu whose recipe book layout is `Mod`: its part answers the click (`recipe_book_place`)
//! with a [`mod_api::RecipeBookPlacement`] before the canonical placer runs.
use mod_api::RecipeBookPlacement;
use sunrise_sim::{
    menu::{INVENTORY_MENU, place::custom_plan::MenuPlacement},
    recipe_book::placement_sync::Refusal,
};

/// `InventoryMenu`'s crafting grid, by menu slot, row by row (its result is slot 0).
const INVENTORY_GRID: [u16; 4] = [1, 2, 3, 4];

impl crate::game::ServerGame<'_> {
    /// Ask the part of the open menu's type how to answer a click on `recipe`, with the player
    /// swapped in. One dispatch returns the placement answer before the canonical placer runs.
    pub(in crate::game) fn ask_mod_book_place(
        &mut self,
        uuid: u128,
        menu_id: &'static str,
        recipe: &str,
        use_max_items: bool,
        creative: bool,
    ) -> Result<RecipeBookPlacement, Refusal> {
        if !mod_api::__private::menu_listens(mod_api::HookId::RecipeBookPlace, menu_id) {
            return Err(Refusal::Unsupported);
        }
        let mut placement = RecipeBookPlacement::Nothing;
        let ran = self.native_as_player(uuid, |game| {
            game.native_recipe_book_place(mod_api::__private::RecipeBookPlaceCall {
                menu: menu_id,
                recipe,
                use_max_items,
                creative,
                placement: &mut placement,
            });
        });
        if !ran {
            return Err(Refusal::Changed);
        }
        Ok(placement)
    }

    /// `AbstractCraftingMenu.handlePlacement` on the player's own `InventoryMenu`, taken over by
    /// the Parts attached to [`INVENTORY_MENU`] that listen to `recipe_book_place`, with the
    /// player swapped in. `Grid` is accepted only as the inventory's 2×2 crafting grid (menu
    /// slots 1 to 4, row by row, clearing none but those), which runs the game's own placement
    /// (vanilla's `super.handlePlacement`); any other `Grid` is refused with a warning and places
    /// nothing, as `Nothing` does.
    pub(in crate::game) fn inventory_book_place(
        &mut self,
        uuid: u128,
        recipe: &str,
        use_max_items: bool,
        creative: bool,
    ) -> RecipeBookPlacement {
        match self.ask_mod_book_place(uuid, INVENTORY_MENU, recipe, use_max_items, creative) {
            Ok(RecipeBookPlacement::Grid {
                width,
                height,
                slots,
                clear,
            }) if !(width == 2
                && height == 2
                && slots == INVENTORY_GRID
                && clear.iter().all(|slot| INVENTORY_GRID.contains(slot))) =>
            {
                log::warn!(
                    "[recipe book] refused Grid placement for menu {INVENTORY_MENU} recipe {recipe}: only its 2x2 grid, slots {INVENTORY_GRID:?}, places",
                );
                RecipeBookPlacement::Nothing
            }
            Ok(placement) => placement,
            Err(_) => RecipeBookPlacement::Nothing,
        }
    }

    /// Resolve a `RecipeBookPlacement::Grid` answer against the menu's current state. Every
    /// refused grid input reports loudly on each attempt without applying the grid.
    pub(in crate::game) fn resolve_mod_book_grid(
        &self,
        menu_id: &str,
        size: usize,
        hidden: &[usize],
        recipe: &str,
        plan: &MenuPlacement,
        width: u16,
        height: u16,
        slots: &[u16],
        clear: &[u16],
    ) -> Result<MenuPlacement, Refusal> {
        let grid_plan = match plan.in_grid(width, height, slots, clear, size) {
            Ok(grid_plan) => grid_plan,
            Err(reason) => {
                log::warn!(
                    "[recipe book] refused Grid placement for menu {menu_id} recipe {recipe}: {reason}",
                );
                return Err(Refusal::Invalid);
            }
        };
        for &slot in grid_plan.cleared_slots() {
            if hidden.contains(&slot) {
                log::warn!(
                    "[recipe book] refused Grid placement for menu {menu_id} recipe {recipe}: slot {slot} is hidden",
                );
                return Err(Refusal::Invalid);
            }
        }
        Ok(grid_plan)
    }
}
