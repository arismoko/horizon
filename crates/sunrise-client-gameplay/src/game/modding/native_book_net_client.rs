//! Client custom-book synchronization, admitted by its own configuration capability.
use crate::game::ClientGame;
use std::collections::BTreeSet;
use sunrise_protocol::packets::{configuration::CustomPayload, play::Serverbound as S};
use sunrise_sim::recipe_book::{
    placement_sync::ResourceId,
    settings_codec,
    settings_sync::{CHANNEL, Message},
};

pub(in crate::game) fn book_payload(message: &Message) -> Option<CustomPayload> {
    match settings_codec::encode(message) {
        Ok(data) => Some(CustomPayload {
            channel: CHANNEL.into(),
            data,
        }),
        Err(error) => {
            log::warn!("[recipe book] cannot send {CHANNEL}: {error}");
            None
        }
    }
}

fn registered() -> BTreeSet<ResourceId> {
    mod_api::RecipeBookTypeKey::registered()
        .into_iter()
        .map(|key| ResourceId::from_registered_book(&key))
        .collect()
}

impl ClientGame<'_> {
    pub fn reset_book_view(&mut self) {
        self.common.native_net.placement.client = Default::default();
        self.clear_mod_book_placement_screen();
        self.common.native_net.book_view = Default::default();
        self.common.native_net.book_request = 0;
        self.common.native_net.book_ready = false;
        self.build_screen();
    }

    /// Authoritative view for a custom menu panel; None until negotiation and snapshot finish.
    pub(in crate::game::client_side) fn custom_book_settings(
        &self,
        book: &mod_api::RecipeBookTypeKey,
    ) -> Option<mod_api::RecipeBookSettings> {
        let view = &self.common.native_net.book_view;
        if !view.ready || !view.contains(book.as_str()) {
            return None;
        }
        let flags = view.books.get(book.as_str());
        Some(mod_api::RecipeBookSettings {
            open: flags.open,
            filtering: flags.filtering,
        })
    }

    /// Queue intent without changing the authoritative or saved pair.
    pub fn request_custom_book_settings(
        &mut self,
        book: &mod_api::RecipeBookTypeKey,
        flags: mod_api::RecipeBookSettings,
    ) -> bool {
        if self.custom_book_settings(book).is_none() {
            return false;
        }
        let Some(number) = self.common.native_net.book_request.checked_add(1) else {
            return false;
        };
        let Some(epoch) = self.common.native_net.book_view.epoch else {
            return false;
        };
        let message = Message::Request {
            epoch,
            number,
            key: ResourceId::from_registered_book(book),
            flags: sunrise_sim::recipe_book::BookFlags {
                open: flags.open,
                filtering: flags.filtering,
            },
        };
        let Some(payload) = book_payload(&message) else {
            return false;
        };
        self.common.native_net.book_request = number;
        self.send(S::CustomPayload(payload));
        true
    }

    pub(in crate::game::client_side) fn begin_book_negotiation(&mut self) {
        self.reset_book_view();
        if self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.custom_books && r.joined)
            && let Some(payload) = book_payload(&Message::Hello)
        {
            self.send(S::CustomPayload(payload));
        }
    }

    pub fn remote_book_settings(&mut self, data: &[u8]) {
        if !self
            .client
            .remote
            .as_ref()
            .is_some_and(|remote| remote.custom_books && remote.joined)
        {
            return;
        }
        let Some(message) = settings_codec::decode(data) else {
            log::warn!(
                "[recipe book] ignored malformed settings payload ({} bytes)",
                data.len()
            );
            return;
        };
        if let Message::Placement(frame) = message {
            if self.common.native_net.book_ready && self.common.native_net.book_view.ready {
                self.remote_mod_book_placement(frame);
            } else {
                log::warn!("[recipe book] ignored placement frame before settings sync completed");
            }
            return;
        }
        if message == Message::Ready {
            if self.common.native_net.book_ready {
                log::warn!("[recipe book] ignored duplicate settings ready");
                return;
            }
            self.common.native_net.book_ready = true;
            if let Some(payload) = book_payload(&Message::Offer(registered().into_iter().collect()))
            {
                self.send(S::CustomPayload(payload));
            }
            return;
        }
        if !self.common.native_net.book_ready
            || !matches!(message, Message::Snapshot { .. } | Message::Update { .. })
        {
            log::warn!(
                "[recipe book] ignored unexpected clientbound settings message: {message:?}"
            );
            return;
        }
        let previous_epoch = self.common.native_net.book_view.epoch;
        if !self
            .common
            .native_net
            .book_view
            .receive(message, &registered())
        {
            log::warn!("[recipe book] ignored stale or invalid settings update");
            return;
        }
        if previous_epoch != self.common.native_net.book_view.epoch {
            self.common.native_net.placement.client = Default::default();
            self.clear_mod_book_placement_screen();
        }
        self.build_screen();
        if self.common.native_net.book_view.ready {
            self.begin_mod_book_placement();
            self.ensure_mod_book_catalog();
        }
    }
}
