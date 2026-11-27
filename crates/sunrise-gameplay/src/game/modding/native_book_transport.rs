//! Dispatch placement intents only after the shared connection boundary admitted them.
use super::native_book_net::book_payload;
use sunrise_protocol::packets::play::Clientbound as C;
use sunrise_sim::recipe_book::{placement_sync as sync, settings_sync::Message};
impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn send_guest_mod_book_placement(&self, conn: u32, frame: sync::Frame) {
        if let Some(guest) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            && let Some(payload) = book_payload(&Message::Placement(frame))
        {
            guest.send(&C::CustomPayload(payload));
        }
    }

    pub fn guest_mod_book_placement(&mut self, conn: u32, frame: sync::Frame) {
        match frame {
            sync::Frame::Hello { epoch, schema } => {
                let actual = self
                    .common
                    .native_net
                    .book_connections
                    .get(&conn)
                    .and_then(|connection| connection.authority.as_ref())
                    .map(|authority| authority.epoch);
                let response = if actual == Some(epoch) {
                    self.common
                        .native_net
                        .placement
                        .servers
                        .entry(conn)
                        .or_insert_with(|| super::native_book_placement::Server::new(epoch))
                        .peer
                        .hello(epoch, schema)
                } else {
                    Err(sync::Refusal::Stale)
                };
                let response = response.unwrap_or_else(|reason| sync::Frame::Refused {
                    epoch,
                    number: 0,
                    window: 0,
                    nonce: 0,
                    reason,
                });
                self.send_guest_mod_book_placement(conn, response);
            }
            request @ sync::Frame::Catalog {
                epoch,
                number,
                window,
                nonce,
                ..
            } => match self.server_mod_book_catalogue(conn, request) {
                Ok((start, bytes)) => {
                    self.send_guest_mod_book_placement(conn, start);
                    let mut offset = 0;
                    for part in bytes.chunks(sync::CHUNK_BYTES) {
                        self.send_guest_mod_book_placement(
                            conn,
                            sync::Frame::Part {
                                epoch,
                                number,
                                offset,
                                bytes: part.to_vec(),
                            },
                        );
                        offset += part.len() as u32;
                    }
                }
                Err(reason) => self.send_guest_mod_book_placement(
                    conn,
                    sync::Frame::Refused {
                        epoch,
                        number,
                        window,
                        nonce,
                        reason,
                    },
                ),
            },
            sync::Frame::Place {
                catalog,
                display,
                use_max,
            } => {
                self.lan_route_place_recipe(
                    conn,
                    super::lan_crafting::RecipePlacementIntent::ModBook {
                        catalog: &catalog,
                        display: &display,
                        use_max,
                    },
                );
            }
            _ => {}
        }
    }
}
