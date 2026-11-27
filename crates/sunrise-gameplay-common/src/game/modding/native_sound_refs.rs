#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicI32, AtomicUsize},
        mpsc::Receiver,
    };
    use sunrise_protocol::packets::{configuration::ClientInformation, play::Clientbound};
    use sunrise_sim::lan::{Joined, Out, PlayerSender, guest::Guest};

    pub fn guest(conn: u32, uuid: u128, at: [f64; 3]) -> (Guest, Receiver<Out>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut g = Guest::new(
            Joined {
                conn,
                uuid,
                name: format!("guest{conn}"),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            conn as i32,
            at,
            10,
        );
        g.placed = true;
        (g, rx)
    }

    pub fn packets(rx: &Receiver<Out>) -> Vec<Clientbound> {
        let mut out = Vec::new();
        while let Ok(Out::Packet(body)) = rx.try_recv() {
            let mut reader = sunrise_protocol::buf::Reader::new(&body);
            let id = reader.varint().unwrap();
            if let Ok(p) = Clientbound::decode(id, reader.rest()) {
                out.push(p);
            }
        }
        out
    }
}
