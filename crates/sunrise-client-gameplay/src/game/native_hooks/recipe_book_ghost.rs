#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// A mod menu's recipe-book ghosts, as the game maps them for the menu's layout: the part of the
    /// open menu's type may add to them, change them or clear them before they are drawn.
    #[inline(never)]
    pub(in crate::game) fn native_recipe_book_ghost(
        &self,
        call: mod_api::__private::RecipeBookGhostCall<'_>,
    ) {
        self.fire_recipe_book_ghost(call);
    }
}
