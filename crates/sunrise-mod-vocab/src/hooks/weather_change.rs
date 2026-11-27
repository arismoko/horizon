//! The `weather_change` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before an explicit server-wide weather parameter write by `/weather`, a
    /// mod or a GameTest weather environment. GameTest saves its level's actual
    /// weather type and proposes it again on teardown.
    /// `dimension` is the origin of the write, not its scope. `clear_ticks`
    /// is the requested clear timer and `weather_ticks` sets both the rain
    /// and thunder timers. These timers and the `raining` and `thundering`
    /// flags are read-only proposals: editing your event copy changes neither
    /// the write nor what other listeners see. Natural weather ticks and
    /// direct strength changes do not fire this hook.
    ///
    /// Return `Flow::Cancel` or set `event.cancelled` to skip the proposed write.
    /// A veto is accepted only on normal return and is logged with the mod's ID.
    /// A final veto makes `LevelRef::set_weather` return `false`. A panic is
    /// logged with the mod's ID and shown in an error toast; it disables only
    /// that mod's weather callback for the session. Phase and reentry refusals
    /// are logged each time without disabling the callback. A nested setter
    /// skips callbacks for this hook already running on the thread; other
    /// eligible listeners still run.
    /// A veto or failure does not undo Host writes.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_weather_change",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        version = 2,
        independent_listener = true,
        event_per_listener = true,
        neoforge = "none (NeoForge has no such event)",
        cancellable = true
    )]
    fn weather_change(
        dimension: &str,
        clear_ticks: i32,
        weather_ticks: i32,
        raining: bool,
        thundering: bool,
        cancel: &mut bool,
    );
}
