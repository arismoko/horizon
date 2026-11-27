use super::*;

/// The hook points. Each does nothing in Sunrise.
pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// Declare categories for custom recipe types, with registered item icons.
    /// Asked lazily once per recipe table by `recipe_book_category`.
    /// The complete batch is validated atomically; any invalid declaration
    /// refuses the generation. Multiple categories may share a recipe type.
    /// This does not yet add tabs or book displays.
    fn native_recipe_book_categories(&mut self, categories: &mut Vec<RecipeBookCategory>) {
        crate::game::native_hooks::fire::FireHooks::fire_recipe_book_categories(self, categories);
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
