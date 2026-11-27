impl crate::game::ServerGame<'_> {
    /// `SleepStatus.sleepersNeeded` in level `dimension`: `needed` arrives with vanilla's
    /// count of the `active` players at `percentage`. Server-side.
    pub(in crate::game) fn native_sleepers_needed(
        &self,
        dimension: &str,
        percentage: i32,
        active: i64,
        needed: &mut i64,
    ) {
        self.fire_sleepers_needed(dimension, percentage, active, needed);
    }
}
