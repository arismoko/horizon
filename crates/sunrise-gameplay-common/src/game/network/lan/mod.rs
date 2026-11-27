#[cfg(any(test, feature = "test-support"))]
pub mod world_spawn_sync_tests {

    use std::sync::mpsc;

    use sunrise_protocol::packets::play::{self, Clientbound as C};

    use sunrise_sim::lan::Out;

    pub fn spawns(rx: &mpsc::Receiver<Out>) -> Vec<play::SetDefaultSpawnPosition> {
        rx.try_iter()
            .flat_map(|out| match out {
                Out::Packet(body) => vec![body],
                Out::Packets(bodies) => bodies,
                Out::Disconnect(_) => panic!("unexpected disconnect"),
            })
            .filter_map(|body| {
                let mut r = sunrise_protocol::buf::Reader::new(&body);
                let id = r.varint().unwrap();
                match C::decode(id, r.rest()).unwrap() {
                    C::SetDefaultSpawnPosition(p) => Some(p),
                    _ => None,
                }
            })
            .collect()
    }
}
