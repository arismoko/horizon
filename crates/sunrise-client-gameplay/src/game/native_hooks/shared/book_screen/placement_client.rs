use super::*;

impl crate::game::ClientGame<'_> {
    pub(crate) fn native_book_placement_scope_matches(
        &self,
        window: i32,
        nonce: u64,
        menu: &str,
        book: &str,
    ) -> bool {
        self.shown_active_native_screen().is_some_and(|screen| {
            screen.menu.window == window
                && screen.menu.name == menu
                && screen.layout_revision == nonce
                && screen
                    .layout()
                    .recipe_book
                    .as_ref()
                    .is_some_and(|key| key.as_str() == book)
        })
    }
    pub fn native_book_placement_window_matches(
        &self,
        window: &sunrise_sim::recipe_book::placement_sync::Window,
    ) -> bool {
        self.native_book_placement_scope_matches(
            window.window,
            window.nonce,
            window.menu.as_str(),
            window.book.as_str(),
        )
    }
    pub fn native_book_placement_catalog_matches(
        &self,
        identity: &sunrise_sim::recipe_book::placement_sync::CatalogIdentity,
    ) -> bool {
        self.native_book_placement_scope_matches(
            identity.window,
            identity.nonce,
            identity.menu.as_str(),
            identity.book.as_str(),
        )
    }
    pub(crate) fn native_book_placement_window(
        &self,
    ) -> Option<sunrise_sim::recipe_book::placement_sync::Window> {
        let screen = self.shown_active_native_screen()?;
        let menu = screen.menu.name.as_str();
        mod_api::RecipeBookLayout::registered(menu)?;
        Some(sunrise_sim::recipe_book::placement_sync::Window {
            window: screen.menu.window,
            nonce: screen.layout_revision,
            menu: menu.to_owned().try_into().ok()?,
            book: screen
                .layout()
                .recipe_book
                .as_ref()?
                .as_str()
                .to_owned()
                .try_into()
                .ok()?,
        })
    }
    pub(crate) fn apply_screen_with_mod_book(&mut self, screen: Screen, owner: Option<&str>) {
        // Publish the accepted native screen before building its layout. A
        // staged screen is not shown, even on an in-place Inventory open.
        if screen == Screen::Inventory
            && let Some(native) = self.client.mod_screen.0.as_mut()
        {
            native.shown = true;
        }
        self.apply_screen(screen, owner);
        if self.native_screen_shown() {
            if self
                .native_net
                .placement
                .client
                .window()
                .is_some_and(|window| !self.native_book_placement_window_matches(window))
            {
                self.clear_mod_book_placement_screen();
            }
            self.begin_mod_book_placement();
            self.ensure_mod_book_catalog();
        } else if self.native_net.placement.client.window().is_some()
            || self.native_net.placement.view.is_some()
        {
            self.clear_mod_book_placement_screen();
        }
    }
}
