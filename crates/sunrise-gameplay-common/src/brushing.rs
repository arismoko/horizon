//! Brushable block counters: `BrushItem.USE_DURATION` and `BrushableBlockEntity` state.
use std::collections::HashMap;

/// `BrushItem.USE_DURATION`.
pub const USE_DURATION: u32 = 200;

/// A brushable block's counters. Vanilla never saves them: the block's
/// `dusted` state is all that is left after a reload.
#[derive(Default)]
pub struct BrushState {
    pub count: i32,
    pub resets_at: i64,
    pub cooldown_ends_at: i64,
    pub hit_face: Option<[i32; 3]>,
}

/// `BrushableBlockEntity.getCompletionState`: the `dusted` property.
pub fn completion_state(count: i32) -> i32 {
    match count {
        0 => 0,
        1..=2 => 1,
        3..=5 => 2,
        _ => 3,
    }
}

pub type Brushing = HashMap<[i32; 3], BrushState>;
