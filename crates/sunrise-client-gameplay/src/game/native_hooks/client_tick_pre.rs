/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Once a client tick, before the window's tick runs (`ClientTickEvent.Pre`).
    pub fn native_client_tick_pre(&mut self) {
        self.fire_client_tick_pre();
    }
}
