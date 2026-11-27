//! Private custom-book synchronization, admitted by its own configuration capability.
use crate::game::ServerGame;
use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicU64, Ordering},
};
use sunrise_protocol::packets::{configuration::CustomPayload, play::Clientbound as C};
use sunrise_sim::recipe_book::{
    placement_sync::ResourceId,
    settings_codec,
    settings_sync::{CHANNEL, Message},
};
static EPOCH: AtomicU64 = AtomicU64::new(1);

pub(super) fn book_payload(message: &Message) -> Option<CustomPayload> {
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
impl ServerGame<'_> {
    pub(in crate::game::server_side) fn sync_replaced_book_settings(&mut self, uuid: u128) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let Some(guest) = host.hub.guests.iter().find(|guest| guest.uuid == uuid) else {
            return;
        };
        let Some(authority) = self
            .common
            .native_net
            .book_connections
            .get_mut(&guest.conn)
            .and_then(|connection| connection.authority.as_mut())
        else {
            return;
        };
        let Some(revision) = authority.revision.checked_add(1) else {
            return;
        };
        authority.revision = revision;
        if let Some(payload) = book_payload(&authority.snapshot(&guest.recipe_book.custom)) {
            guest.send(&C::CustomPayload(payload));
        }
    }

    pub(in crate::game::server_side) fn clear_guest_book_connection(&mut self, conn: u32) {
        self.server.guest_combat.0.remove(&conn);
        self.common.native_net.book_connections.remove(&conn);
        self.common.native_net.placement.servers.remove(&conn);
    }
    pub(in crate::game::server_side) fn notify_vanilla_book_settings(
        &mut self,
        uuid: u128,
        name: &str,
        book: i32,
        open: bool,
        filtering: bool,
    ) {
        let book = match book {
            0 => mod_api::RecipeBookIdentity::Crafting,
            1 => mod_api::RecipeBookIdentity::Furnace,
            2 => mod_api::RecipeBookIdentity::BlastFurnace,
            3 => mod_api::RecipeBookIdentity::Smoker,
            _ => return,
        };
        self.native_player_recipe_book_settings_change(uuid, name, book, open, filtering);
    }

    pub(in crate::game::server_side) fn sync_server_book_write(
        &mut self,
        player: mod_api::PlayerRef,
        book: &mod_api::RecipeBookTypeKey,
        flags: sunrise_sim::recipe_book::BookFlags,
    ) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let Some(guest) = host
            .hub
            .guests
            .iter()
            .find(|g| g.uuid == player.uuid() && g.placed)
        else {
            return;
        };
        let Some(authority) = self
            .common
            .native_net
            .book_connections
            .get_mut(&guest.conn)
            .and_then(|connection| connection.authority.as_mut())
        else {
            return;
        };
        if let Some(message) = authority.server_update(book.as_str(), flags)
            && let Some(payload) = book_payload(&message)
        {
            guest.send(&C::CustomPayload(payload));
        }
    }

    pub fn guest_book_settings(&mut self, conn: u32, data: &[u8]) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        if !host.hub.guest(conn).is_some_and(|guest| guest.placed) {
            return;
        }
        let connection = self
            .common
            .native_net
            .book_connections
            .entry(conn)
            .or_default();
        if data.first() == Some(&settings_codec::PLACEMENT_TAG) {
            let frame = connection.receive_placement(data);
            if let Some(frame) = frame {
                self.guest_mod_book_placement(conn, frame);
            }
            return;
        }
        let changed = host
            .hub
            .guests
            .update_connection(conn, |guest| {
                let reply =
                    connection.receive(data, &mut guest.recipe_book.custom, registered, || {
                        EPOCH.fetch_add(1, Ordering::Relaxed)
                    })?;
                if let Some(payload) = book_payload(&reply.message) {
                    guest.send(&C::CustomPayload(payload));
                }
                reply
                    .changed
                    .map(|(id, flags)| (guest.uuid, guest.name.clone(), id, flags))
            })
            .flatten();
        if let Some((uuid, name, id, flags)) = changed
            && let Some(key) = mod_api::RecipeBookTypeKey::registered()
                .into_iter()
                .find(|key| key.as_str() == id.as_str())
        {
            self.native_player_recipe_book_settings_change(
                uuid,
                &name,
                mod_api::RecipeBookIdentity::Custom(key),
                flags.open,
                flags.filtering,
            );
        }
    }
}
