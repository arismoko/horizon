#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    use std::sync::{
        Arc,
        atomic::{AtomicI32, AtomicUsize},
    };

    use sunrise_protocol::packets::configuration::ClientInformation;

    use sunrise_sim::lan::{Joined, PlayerSender, guest::Guest};

    pub fn guest(uuid: u128, conn: u32) -> Guest {
        let (tx, _) = std::sync::mpsc::channel();
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
            [0.0; 3],
            10,
        );
        g.placed = true;
        g
    }
}
