#[cfg(feature = "client")]
use sunrise_client_gameplay::game::remote::riptide_tests::OWNER;
#[cfg(feature = "client")]
use sunrise_protocol::connection::ConnReader;
#[cfg(feature = "client")]
use sunrise_protocol::packets::Empty;
#[cfg(feature = "client")]
use sunrise_protocol::packets::play::Serverbound as S;
#[cfg(feature = "client")]
use sunrise_sim::lan::guest::InboxItem;

/// Transfer the client's real play packets to the integrated server's normal
/// connection drain. The test drives ticks without scheduling two game loops.
#[cfg(feature = "client")]
pub(crate) fn client_tick_to_server(
    client: &sunrise_client_gameplay::game::ClientGame<'_>,
    peer: &mut ConnReader,
    server: &mut sunrise_gameplay::game::ServerGame<'_>,
) -> Vec<S> {
    client.send(S::ClientTickEnd(Empty));
    let mut packets = Vec::new();
    loop {
        let (id, bytes) = peer.recv().unwrap();
        let packet = S::decode(id, &bytes).unwrap();
        let end = matches!(packet, S::ClientTickEnd(_));
        packets.push(packet);
        if end {
            break;
        }
    }
    let hub = &mut server.server.lan.as_mut().unwrap().hub;
    hub.update_guest(OWNER, |g| {
        g.inbox
            .extend(packets.iter().cloned().map(|p| InboxItem::Packet(p, None)));
    })
    .unwrap();
    server.server.level_pass = Default::default();
    server.tick_lan_packets();
    let hub = &mut server.server.lan.as_mut().unwrap().hub;
    hub.end_tick();
    hub.guest(OWNER).unwrap().sender.flush();
    packets
}
