//! The world clock state and the clock time markers of the timelines.

/// The Overworld's world clock.
pub const OVERWORLD_CLOCK: &str = "minecraft:overworld";

/// ServerClockManager.ServerClockInstance (the total ticks of the one the level in place follows
/// are the world's day time).
#[derive(Clone, Copy, Debug)]
pub struct WorldClock {
    pub partial: f32,
    pub rate: f32,
    pub paused: bool,
}
impl Default for WorldClock {
    fn default() -> Self {
        Self {
            partial: 0.0,
            rate: 1.0,
            paused: false,
        }
    }
}
/// A clock time marker from the timelines (ClockTimeMarker); `resolve` is None past `i64::MAX`.
#[derive(Clone, Copy, Debug)]
pub struct TimeMarker {
    pub ticks: i64,
    pub period: Option<i64>,
    pub show: bool,
}
impl TimeMarker {
    pub fn occurs_at(&self, total: i64) -> bool {
        match self.period {
            Some(p) => self.ticks == total.rem_euclid(p),
            None => self.ticks == total,
        }
    }
    pub fn resolve(&self, total: i64) -> Option<i64> {
        match self.period {
            Some(p) => {
                let d = self.ticks - total.rem_euclid(p);
                total.checked_add(if d > 0 { d } else { p + d })
            }
            None => Some(self.ticks),
        }
    }
}
