use super::*;
use sunrise_hooks::items::engine::CreativeOrder;
use sunrise_items::creative_catalog::{Catalog, Holders};

impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn client_creative_holders(&self) -> Option<Holders> {
        self.client.remote.as_ref().map(|remote| {
            sunrise_sim::item_names::live_holders(&remote.builtin, &remote.registries)
        })
    }

    pub(in crate::game) fn client_creative_catalog(&self) -> &Arc<Catalog> {
        self.client.creative_screen.0.catalog()
    }

    pub(in crate::game) fn client_reset_creative_screen(&mut self) {
        self.client.creative_screen.0.reset_contents();
    }

    /// The name components that replace the creative search's prepared name lines.
    pub(crate) fn refresh_creative_search_names(&mut self) {
        // Only when the search tab's stacks were made again, not every creative tick.
        let generation = self.client.creative_screen.0.search_generation();
        if self.client.creative_search_named == Some(generation) {
            return;
        }
        self.client.creative_search_named = Some(generation);
        let names = self
            .client
            .creative_screen
            .0
            .search_items()
            .iter()
            .map(|stack| {
                if sunrise_sim::stations::names_itself(stack) {
                    Some(self.common.stack_hover_name_text(stack))
                } else {
                    self.common.mod_item_name(stack)
                }
            })
            .collect();
        self.client.creative_screen.0.set_search_names(names);
    }

    /// The screen uses the completed cache, with the same holder definitions for search text.
    pub fn refresh_creative_tabs(&mut self) {
        let Ok(lists) = self.build_creative(self.creative_permissions()) else {
            return;
        };
        if let Some(published) = &self.common.creative_contents.published
            && Arc::ptr_eq(&lists, &published.lists)
        {
            self.client.creative_screen.0.publish(
                lists,
                published.holders.clone(),
                published.items.clone(),
                &self.common.pack.names,
            );
            self.refresh_creative_search_names();
        }
    }

    /// The built-in sorter is only a reader. A cold sort previews stock order
    /// without publishing anything or giving mods a creative callback.
    pub fn stock_sort_order(&self) -> CreativeOrder {
        let lists = self.stock_creative_lists();
        self.client.creative_screen.0.stock_order(&lists)
    }
}

use sunrise_gameplay_common::game::native_hooks::build_creative_mode_tab_contents::GameHooks as _;
