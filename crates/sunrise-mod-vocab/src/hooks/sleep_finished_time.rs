//! The `sleep_finished_time` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Enough players have slept through the night in level `dimension`
    /// and the clock is about to move to the morning (NeoForge's
    /// `SleepFinishedTimeEvent`, vanilla's `ServerLevel.tick` when it moves
    /// the clock to `wake_up_from_sleep`). `adjustment` says where the clock
    /// goes and may be changed: it comes in as
    /// `Marker(minecraft:wake_up_from_sleep)`, vanilla's own, and a mod may
    /// set `Absolute`, `Relative` (from `current`, the day time now) or
    /// another `Marker`; a later mod reads what an earlier one left.
    /// `new_time` is the morning vanilla resolved, read only, for a mod that
    /// computes from it. Contract version `2`: in version `1`, `new_time`
    /// was the value a mod wrote; a time is now written as `Absolute`.
    /// `current` and the result are times of the level's own clock (its
    /// dimension type's default clock, else the Overworld's). The game
    /// resolves `adjustment` once after every mod: a time before `current`,
    /// a time past `i64::MAX`, or a marker the level's clock does not define
    /// is refused, as `setTimeAddition` refuses one, and the clock moves to
    /// the morning. Only the Overworld's clock defines markers today, so on
    /// any other clock every `Marker`, vanilla's included, falls back to the
    /// morning. When even the morning is past `i64::MAX`, `new_time` is
    /// `current` and the clock stays. On the server, only when the `advance_time`
    /// rule is on. Set `canceled` to leave the clock where it is; the players wake
    /// and weather resets as usual either way. This is the only wake-up clock event.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_sleep_finished_time",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "SleepFinishedTimeEvent",
        cancellable = true,
        version = 3
    )]
    fn sleep_finished_time(
        dimension: &str,
        current: i64,
        new_time: i64,
        adjustment: &mut ClockAdjustment,
        canceled: &mut bool,
    );
}
