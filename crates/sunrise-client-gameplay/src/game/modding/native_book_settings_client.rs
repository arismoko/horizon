//! Client-side player custom recipe book settings queries and requests.
use crate::game::ClientGame;
use mod_api::{
    PlayerRef,
    item::recipe_book::{RecipeBookSettings, RecipeBookTypeKey},
};

impl ClientGame<'_> {
    pub(in crate::game) fn player_synced_recipe_book_settings(
        &self,
        player: PlayerRef,
        book: &RecipeBookTypeKey,
    ) -> Option<RecipeBookSettings> {
        self.native_local_status(player)?;
        self.custom_book_settings(book)
    }

    pub(in crate::game) fn player_request_recipe_book_settings(
        &mut self,
        player: PlayerRef,
        book: &RecipeBookTypeKey,
        settings: RecipeBookSettings,
    ) -> bool {
        self.native_local_status(player).is_some()
            && self.request_custom_book_settings(book, settings)
    }
}
