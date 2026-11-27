//! Shared spike logging for mod column queries.

impl crate::game::CommonGame<'_> {
    /// A read's time logged with `SUNRISE_SPIKES`, as an edit's is.
    pub fn native_read_logged(&self, started: std::time::Instant, what: &str, columns: usize) {
        let ms = started.elapsed().as_secs_f32() * 1000.0;
        if self.options.spikes.is_some_and(|threshold| ms > threshold) {
            log::info!("Sunrise: slow mod {what} {ms:.1} ms ({columns} columns)");
        }
    }
}
