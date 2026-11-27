impl crate::game::ServerGame<'_> {
    /// A mod menu's recipe-book click, answered by the part of the open menu's type with a
    /// [`mod_api::RecipeBookPlacement`] (`Nothing`, `Ghost`, or `Grid`).
    #[inline(never)]
    pub(in crate::game) fn native_recipe_book_place(
        &mut self,
        call: mod_api::__private::RecipeBookPlaceCall<'_>,
    ) {
        self.fire_recipe_book_place(call);
    }
}
