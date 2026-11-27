//! Client placement transport state for mod recipe books.
use super::native_book_net_client::book_payload;
use crate::game::ClientGame;
use sunrise_protocol::packets::play::Serverbound as S;
use sunrise_sim::recipe_book::{placement_sync as sync, settings_sync::Message};

impl ClientGame<'_> {
    pub(in crate::game::client_side) fn send_mod_book_placement(&mut self, frame: sync::Frame) {
        if let Some(payload) = book_payload(&Message::Placement(frame)) {
            self.send(S::CustomPayload(payload));
        }
    }

    pub fn begin_mod_book_placement(&mut self) {
        let Some(epoch) = self.common.native_net.book_view.epoch else {
            return;
        };
        if self.common.native_net.placement.client.epoch() == Some(epoch) {
            return;
        }
        let supported = mod_api::RecipeBookLayout::registered_menus()
            .iter()
            .any(|menu| {
                mod_api::RecipeBookLayout::registered(menu).is_some_and(|(book, _, _)| {
                    self.common.native_net.book_view.contains(book.as_str())
                })
            });
        if !supported {
            return;
        }
        let message = self.common.native_net.placement.client.hello(epoch);
        self.send_mod_book_placement(message);
    }

    pub(in crate::game) fn clear_mod_book_placement_screen(&mut self) {
        self.common.native_net.placement.client.close();
        self.common.native_net.placement.view = None;
        self.reset_mod_book_placement_ui();
    }

    pub fn ensure_mod_book_catalog(&mut self) {
        let Some(window) = self.native_book_placement_window() else {
            if self.common.native_net.placement.client.window().is_some()
                || self.common.native_net.placement.view.is_some()
            {
                self.clear_mod_book_placement_screen();
            }
            return;
        };
        if self.common.native_net.placement.client.window() == Some(&window) {
            return;
        }
        if let Some(frame) = self.common.native_net.placement.client.open(window) {
            self.common.native_net.placement.view = None;
            self.reset_mod_book_placement_ui();
            self.send_mod_book_placement(frame);
        }
    }

    pub(in crate::game) fn client_invalidate_mod_book_placement_catalogues(&mut self) {
        self.clear_mod_book_placement_screen();
        self.ensure_mod_book_catalog();
    }
}
