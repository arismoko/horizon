//! Position writes captured while a lifted mob list is being ticked.
//!
//! Mob ticks cannot call back through `Game` while their world view is borrowed. Keep the
//! write-side record separate from `Mob`; the game drains it as soon as the current mob's tick
//! is complete, preserving every teleport before the ordinary movement write.

use horizon_core::geometry::Vec3;
use std::cell::RefCell;

#[derive(Clone, Copy, Debug)]
pub struct PositionWrite {
    pub id: u32,
    pub from: Option<Vec3>,
    pub to: Vec3,
}

impl PositionWrite {
    pub const fn new(id: u32, from: Option<Vec3>, to: Vec3) -> Self {
        Self { id, from, to }
    }

    pub const fn captured(id: u32, from: Vec3, to: Vec3) -> Self {
        Self::new(id, Some(from), to)
    }

    pub const fn baseline(id: u32, to: Vec3) -> Self {
        Self::new(id, None, to)
    }
}

#[derive(Default)]
struct State {
    enabled: bool,
    writes: Vec<PositionWrite>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Start collecting writes for one lifted creature tick.
pub fn begin(enabled: bool) {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.enabled = enabled;
        state.writes.clear();
    });
}

/// Capture a committed teleport while the current mob is being ticked.
pub fn record(id: u32, from: Vec3, to: Vec3) {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.enabled {
            state.writes.push(PositionWrite::captured(id, from, to));
        }
    });
}

/// Move the current mob's writes into the game's per-tick record.
pub fn drain_for(id: u32, out: &mut Vec<PositionWrite>) -> usize {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let count = state
            .writes
            .iter()
            .take_while(|write| write.id == id)
            .count();
        out.extend(state.writes.drain(..count));
        count
    })
}

/// Stop collecting and retain the allocation for the next tick.
pub fn finish() {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.enabled = false;
        state.writes.clear();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_multiple_writes_for_one_mob_in_order() {
        begin(true);
        record(7, Vec3::new(15.5, 64.0, 0.5), Vec3::new(16.5, 64.0, 0.5));
        record(7, Vec3::new(16.5, 64.0, 0.5), Vec3::new(32.5, 64.0, 0.5));
        record(8, Vec3::ZERO, Vec3::ONE);
        let mut out = Vec::new();
        drain_for(7, &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].id, 7);
        assert_eq!(out[0].from.unwrap().x, 15.5);
        assert_eq!(out[1].to.x, 32.5);
        finish();
    }

    #[test]
    fn disabled_capture_does_not_record() {
        begin(false);
        record(7, Vec3::ZERO, Vec3::ONE);
        let mut out = Vec::new();
        drain_for(7, &mut out);
        assert!(out.is_empty());
        finish();
    }
}
