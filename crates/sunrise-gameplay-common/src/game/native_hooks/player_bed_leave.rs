use std::cell::RefCell;

thread_local! {
    /// The players whose `player_bed_leave` the mods are hearing on this
    /// thread: a listener that wakes the same player again wakes them
    /// without a second event.
    pub static WAKING: RefCell<Vec<u128>> = const { RefCell::new(Vec::new()) };
}
