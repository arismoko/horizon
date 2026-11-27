//! Reloads and authoritative knowledge changes revoke published display identities.
use sunrise_sim::recipe_book::placement_sync::Frame;
impl crate::game::ServerGame<'_> {
    fn invalidate_mod_book_placement_connection(&mut self, conn: u32) {
        let Some(server) = self.common.native_net.placement.servers.get_mut(&conn) else {
            return;
        };
        server.clear();
        if let Some(epoch) = self
            .common
            .native_net
            .book_connections
            .get(&conn)
            .and_then(|connection| connection.authority.as_ref())
            .map(|authority| authority.epoch)
        {
            self.send_guest_mod_book_placement(conn, Frame::Invalidate { epoch });
        }
    }
    #[allow(dead_code)]
    pub(in crate::game) fn server_invalidate_mod_book_placement_catalogues(&mut self) {
        let connections: Vec<_> = self
            .common
            .native_net
            .placement
            .servers
            .keys()
            .copied()
            .collect();
        for conn in connections {
            self.invalidate_mod_book_placement_connection(conn);
        }
    }
    pub(in crate::game::server_side) fn invalidate_mod_book_placement_knowledge(&mut self) {
        let Some(uuid) = self.server.lan_acting_uuid else {
            return;
        };
        let connection = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guests.iter().find(|guest| guest.uuid == uuid))
            .map(|guest| guest.conn);
        if let Some(conn) = connection {
            self.invalidate_mod_book_placement_connection(conn);
        }
    }
}
