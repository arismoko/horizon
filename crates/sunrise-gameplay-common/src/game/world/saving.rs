//! The save screen freezes simulation, but keeps pumping window events while
//! snapshots are prepared in small batches and the region workers drain.
use std::{
    collections::VecDeque,
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ClientQuit {
    Running,
    Requested,
    Stopping,
}

pub struct ClosingSave {
    pub quit: bool,
    pub close: bool,
    pub reply: Option<std::sync::mpsc::Sender<String>>,
    #[allow(dead_code)]
    pub started: Instant,
    pub stage: Stage,
}
#[allow(dead_code)]
pub enum Stage {
    Generation,
    Chunks(VecDeque<(i32, i32)>),
    Entities(VecDeque<(i32, i32)>),
    Writes(Option<Receiver<std::io::Result<()>>>),
    Server,
    Quit,
}

// Bound the work in both sparse and block-entity-heavy worlds. A single chunk
// is indivisible; check the time budget after each snapshot, not each batch.
#[allow(dead_code)]
pub const SNAPSHOTS_PER_FRAME: usize = 16;
#[allow(dead_code)]
pub const SNAPSHOT_BUDGET: Duration = Duration::from_millis(2);
#[allow(dead_code)]
pub const CLOSING_SNAPSHOTS_PER_FRAME: usize = 256;
#[allow(dead_code)]
pub const CLOSING_SNAPSHOT_BUDGET: Duration = Duration::from_millis(10);
