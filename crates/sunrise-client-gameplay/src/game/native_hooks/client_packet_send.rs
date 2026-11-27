use mod_api::ClientPacket;

/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// A play packet the window's game is about to send to the server
    /// (`Connection.send`): every packet of the play phase it sends, the
    /// selected slot that goes out before an attack or a use included, in the
    /// order it sends them. Not heard: what the network thread sends itself
    /// (keep-alives, configuration pongs, cookies, the configuration phase), the answers the
    /// connection sends in whichever phase it is in (a resource pack's status, a
    /// dialog's custom click action), and far terrain requests. Cancel drops the
    /// packet: the game goes on as if it had been sent. Client-side, on any
    /// server, single player's own included. Every packet exposes its protocol
    /// type id; custom payloads also expose their channel and raw bytes.
    /// Play-phase pongs pass through this hook; mods cancel by returning `Flow::Cancel`.
    pub(in crate::game) fn native_client_packet_send(
        &self,
        packet: &ClientPacket<'_>,
        canceled: &mut bool,
    ) {
        self.fire_client_packet_send(packet, canceled);
    }
}
