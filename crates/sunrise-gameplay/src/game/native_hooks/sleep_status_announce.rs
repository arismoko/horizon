use sunrise_game_data::text::Text;

impl crate::game::ServerGame<'_> {
    /// The sleep-status line (`ServerLevel.announceSleepStatus`) is about to be sent to every
    /// player, after the game's own checks passed. Change `line` to replace it, or set
    /// `canceled` and nobody gets it.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game) fn native_sleep_status_announce(
        &mut self,
        dimension: &str,
        sleeping: i64,
        active: i64,
        percentage: i32,
        needed: i64,
        skipping_night: bool,
        line: &mut Text,
        canceled: &mut bool,
    ) {
        self.fire_sleep_status_announce(
            dimension,
            sleeping,
            active,
            percentage,
            needed,
            skipping_night,
            line,
            canceled,
        );
    }
}
