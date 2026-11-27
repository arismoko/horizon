#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    use std::sync::mpsc;

    use sunrise_protocol::{buf::Reader, packets::play::Clientbound as C};

    use sunrise_sim::lan::Out;

    /// The packets sent since the last call.
    pub fn sent(rx: &mpsc::Receiver<Out>) -> Vec<C> {
        rx.try_iter()
            .flat_map(|out| match out {
                Out::Packet(body) => vec![body],
                Out::Packets(bodies) => bodies,
                _ => Vec::new(),
            })
            .map(|body| {
                let mut reader = Reader::new(&body);
                let id = reader.varint().unwrap();
                C::decode(id, reader.rest()).unwrap()
            })
            .collect()
    }
    /// The content menu type shared by the mod-menu fixtures.
    pub const MOD_MENU: &str = "listen:status";
}
