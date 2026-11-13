//! The `sleep_finished_time` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0331

pub trait Hooks {
    /// Enough players have slept through the night in level `dimension`
    /// and the clock is about to move to the morning (NeoForge's
    /// `SleepFinishedTimeEvent`, vanilla's `ServerLevel.tick` when it moves
    /// the clock to `wake_up_from_sleep`): `new_time` comes in as the next
    /// morning's day time and may be changed to wake everyone at another
    /// time. A time before `current` (the day time now) is refused, as
    /// `setTimeAddition` refuses one, and the morning stays. Unlike
    /// NeoForge, which refuses a bad value as it is set, the check runs once
    /// after every mod, so a later mod sees the value an earlier one left.
    /// On the server, only when the `advance_time` rule is on;
    /// `time_skip` then hears the jump to the time chosen here and
    /// may still cancel it. The players wake and the weather resets as
    /// usual either way. Set `canceled` and the clock is left where it is, as NeoForge's cancel leaves it:
    /// `time_skip` is not asked either.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "SleepFinishedTimeEvent",
        cancellable = true
    )]
    fn sleep_finished_time();
}
