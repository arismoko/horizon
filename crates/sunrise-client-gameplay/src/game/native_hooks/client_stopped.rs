/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// The client shutdown has completed.
    pub fn native_client_stopped(&mut self) {
        self.fire_client_stopped();
    }
}
