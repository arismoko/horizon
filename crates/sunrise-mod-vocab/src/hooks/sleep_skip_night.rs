//! The `sleep_skip_night` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Enough players sleep in level `dimension` to skip the night (vanilla's `ServerLevel.tick`,
    /// once `SleepStatus.areEnoughSleeping` and `areEnoughDeepSleeping` both pass): the game is
    /// about to move the clock to the morning when `advance_time` is on (`sleep_finished_time`),
    /// wake every sleeping player, and end the rain when `advance_weather` is on. `percentage` is
    /// the `players_sleeping_percentage` game rule, `active` the players that count toward
    /// sleeping (not spectators) and `sleeping` those in bed. Asked once per tick while this
    /// holds, on the server, before the clock moves.
    ///
    /// Set `canceled` and none of it happens this tick: the clock stays, nobody is woken, the
    /// weather is kept, and `sleep_finished_time` and `time_skip` are not asked. The players stay
    /// in bed; vanilla's own checks still wake each one, such as at daytime
    /// (`can_continue_sleeping`). The night is not skipped either when the listeners' own writes
    /// leave too few players asleep, such as a sleeper's timer set back with
    /// `PlayerRef::set_sleep_timer`. NeoForge has no such event: its `SleepFinishedTimeEvent`
    /// cancels only the clock move.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_sleep_skip_night",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "none (SleepFinishedTimeEvent cancels only the clock move)"
    )]
    fn sleep_skip_night(
        dimension: &str,
        percentage: i32,
        sleeping: i64,
        active: i64,
        canceled: &mut bool,
    );
}
