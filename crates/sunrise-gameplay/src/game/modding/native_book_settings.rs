//! Player-owned custom recipe book settings, separate from vanilla ordinals.
use crate::game::ServerGame;
use mod_api::{
    PlayerRef,
    item::recipe_book::{RecipeBookSettings, RecipeBookTypeKey},
};
use sunrise_sim::recipe_book::{BookFlags, placement_sync::ResourceId};

impl ServerGame<'_> {
    pub(in crate::game) fn player_set_recipe_book_settings(
        &mut self,
        player: PlayerRef,
        book: &RecipeBookTypeKey,
        settings: RecipeBookSettings,
    ) -> bool {
        if !book.is_registered() || !self.native_player_current(player) {
            return false;
        }
        let flags = BookFlags {
            open: settings.open,
            filtering: settings.filtering,
        };
        let id = ResourceId::from_registered_book(book);
        let mut publish = false;
        if !self.native_as_player_handle(player, |game| {
            if game.common.level.recipe_book.custom.get(id.as_str()) != flags {
                let deferred = game.journal_player_book(player, book);
                publish = game.common.level.recipe_book.custom.set(&id, flags) && !deferred;
            }
        }) {
            return false;
        }
        if publish {
            self.publish_player_book_settings(player, book.clone(), flags);
        }
        true
    }

    pub(in crate::game) fn publish_player_book_settings(
        &mut self,
        player: PlayerRef,
        book: RecipeBookTypeKey,
        flags: BookFlags,
    ) {
        self.sync_server_book_write(player, &book, flags);
        if mod_api::__private::listened(mod_api::HookId::PlayerRecipeBookSettingsChange)
            && let Some(name) = self.player_name(player).map(str::to_owned)
        {
            self.native_player_recipe_book_settings_change(
                player.uuid(),
                &name,
                mod_api::RecipeBookIdentity::Custom(book),
                flags.open,
                flags.filtering,
            );
        }
    }

    pub(in crate::game) fn player_recipe_book_settings(
        &self,
        player: PlayerRef,
        book: &RecipeBookTypeKey,
    ) -> Option<RecipeBookSettings> {
        if !book.is_registered() || !self.native_player_current(player) {
            return None;
        }
        let settings = match self.native_player_resting(player) {
            Some(guest) => &guest.recipe_book,
            None => &self.common.level.recipe_book,
        };
        let flags = settings.custom.get(book.as_str());
        Some(RecipeBookSettings {
            open: flags.open,
            filtering: flags.filtering,
        })
    }
}
