#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    use std::sync::{Arc, mpsc};

    /// Guests 12 and 13 with senders to read and the given brands.
    pub fn listen(
        game: &mut crate::game::ServerGame<'_>,
        brands: [&str; 2],
    ) -> Vec<mpsc::Receiver<sunrise_sim::lan::Out>> {
        let mut receivers = Vec::new();
        for (conn, brand) in [12, 13].into_iter().zip(brands) {
            let (tx, rx) = mpsc::channel();
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(conn, |g| {
                    g.sender = sunrise_sim::lan::PlayerSender::new(tx, Arc::default());
                    g.brand = brand.into();
                });
            receivers.push(rx);
        }
        receivers
    }
}
