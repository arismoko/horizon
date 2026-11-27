impl crate::game::ServerGame<'_> {
    /// `MinecraftServer.onTickRateChanged`, before the autosave wait is adjusted after a
    /// server tick-rate write or the end of a `/tick sprint`.
    pub(in crate::game) fn native_server_tick_rate_changed(&mut self) {
        self.fire_server_tick_rate_changed();
    }
}
