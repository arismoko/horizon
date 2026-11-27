impl crate::game::ServerGame<'_> {
    /// A line the server logged, with the overflow count on the first line of its batch.
    pub(in crate::game) fn native_console_line(
        &mut self,
        level: &str,
        thread: &str,
        message: &str,
        dropped: u64,
    ) {
        self.fire_console_line(level, thread, message, dropped);
    }
}
