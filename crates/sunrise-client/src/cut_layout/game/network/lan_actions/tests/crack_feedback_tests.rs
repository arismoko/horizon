#![cfg(test)]
use std::sync::{Arc, atomic::AtomicI32, mpsc};
use sunrise_gameplay_common::game::lan_actions::C;
use sunrise_protocol::packets::configuration::ClientInformation;
use sunrise_sim::lan::{Joined, Out, PlayerSender, guest::Guest};

pub(in crate::game) fn observer(
    conn: u32,
    position: [f64; 3],
    dimension: &str,
) -> (Guest, mpsc::Receiver<Out>) {
    let (tx, rx) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn,
            uuid: u128::from(conn),
            name: format!("Crack{conn}"),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "test".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
        },
        conn as i32,
        position,
        2,
    );
    guest.placed = true;
    guest.loaded = true;
    guest.dimension = dimension.into();
    guest.on_ground = true;
    (guest, rx)
}

pub(in crate::game) fn cracks(rx: &mpsc::Receiver<Out>) -> Vec<(i32, [i32; 3], u8)> {
    let mut result = Vec::new();
    for out in rx.try_iter() {
        let bodies = match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(_) => panic!("unexpected disconnect"),
        };
        for body in bodies {
            let mut reader = sunrise_protocol::buf::Reader::new(&body);
            let id = reader.varint().unwrap();
            if let C::BlockDestruction(packet) = C::decode(id, reader.rest()).unwrap() {
                result.push((packet.id, packet.pos, packet.progress));
            }
        }
    }
    result
}

/// Exercise actual packet handling, dig feedback, and encoded observer packets without
/// starting a listener, a server loop, a window, or audio output.
#[test]
#[ignore = "daily"]
fn crack_packets_follow_start_clock_and_strict_observer_radius() {
    run_crack_packets_follow_start_clock_and_strict_observer_radius();
}

#[allow(unused_imports)]
pub(in crate::game) use crate::game::server_side::lan_actions_server::crack_feedback_tests::run_crack_packets_follow_start_clock_and_strict_observer_radius;
