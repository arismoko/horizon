use crate::game::lan_actions::guest_body::tests::travel::guest;
use sunrise_protocol::packets::{Empty, play::Serverbound as S};

pub(crate) fn send(game: &mut sunrise_gameplay::game::ServerGame<'_>, packet: S) {
    game.server.lan.as_mut().unwrap().hub.update_guest(1, |g| {
        use sunrise_sim::lan::guest::InboxItem;
        g.inbox.push_back(InboxItem::Packet(packet, None));
        g.inbox
            .push_back(InboxItem::Packet(S::ClientTickEnd(Empty), None));
    });
    game.server.level_pass.packets_drained = false;
    game.tick_lan_packets();
    // The packet phase holds output until the tick's usual network handoff.
    guest(game).sender.flush();
}
