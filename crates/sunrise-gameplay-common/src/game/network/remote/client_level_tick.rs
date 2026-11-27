use super::*;

#[cfg(test)]
#[path = "boss_darken_tests.rs"]
pub(in crate::game) mod boss_darken_tests;

/// `ClientClockManager.tick` for one clock: it moves on by `delta` game ticks at its rate, the
/// part of a tick it makes carried to the next. None, and the clock left as it was, when its total
/// would pass the range of a long: a server's times must not wrap or panic the client.
pub fn advance_clock(clock: &mut play::ClockState, delta: i64) -> Option<()> {
    let partial = f64::from(clock.partial_tick) + delta as f64 * f64::from(clock.rate);
    let full = partial.floor();
    if !(i64::MIN as f64..i64::MAX as f64).contains(&full) {
        return None;
    }
    clock.total_ticks = clock.total_ticks.checked_add(full as i64)?;
    clock.partial_tick = (partial - full) as f32;
    Some(())
}
