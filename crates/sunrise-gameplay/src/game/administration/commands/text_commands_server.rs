use super::Target;
use sunrise_protocol::packets::play::Clientbound as C;

impl crate::game::ServerGame<'_> {
    /// TitleCommand's `connection.send` for each player: a guest is sent the
    /// packet, the acting player queues it on `server.guest_packets`.
    pub(in crate::game::server_side) fn titles_or_packet(&mut self, players: &[Target], packet: C) {
        assert!(
            self.is_logical_server(),
            "titles_or_packet called on Client role"
        );
        for &t in players {
            match t {
                Target::Player if self.server.lan_acting.is_some() || self.server.lan.is_none() => {
                    self.server.guest_packets.push(packet.clone());
                }
                Target::Player => {}
                Target::Guest(conn) => {
                    if let Some(host) = self.server.lan.as_ref() {
                        host.hub.send_to(conn, &packet);
                    }
                }
                _ => {}
            }
        }
    }
}
