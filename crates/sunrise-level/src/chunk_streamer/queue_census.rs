//! What the streamer's queues hold, for `sunrise-ctl memory` and captures.
use super::{AuxiliaryJobs, ChunkStreamer, Queued};

/// What the streamer's work queues hold, each as (entries, estimated bytes).
#[derive(Debug, Default)]
pub struct QueueCensus {
    /// Jobs queued and chunks generating.
    pub generation: (usize, usize),
    /// Chunks being lit and the edit log their jobs replay.
    pub light: (usize, usize),
}

impl<A: AuxiliaryJobs> ChunkStreamer<A> {
    /// The queues' lengths and the bytes of their tables, from capacities. For `sunrise-ctl memory`.
    pub fn queue_census(&self) -> QueueCensus {
        use std::mem::size_of;
        let table = |capacity: usize, entry: usize| capacity * (entry + 1);
        let queued = {
            let queue = self.shared.queue.lock().unwrap();
            (
                queue.len(),
                queue.capacity() * size_of::<Queued<A::Payload>>(),
            )
        };
        QueueCensus {
            generation: (
                queued.0 + self.state.generating.len(),
                queued.1 + table(self.state.generating.capacity(), size_of::<(i32, i32)>()),
            ),
            light: (
                self.state.lighting.len(),
                table(
                    self.state.lighting.capacity(),
                    size_of::<((i32, i32), super::LightJob)>(),
                ) + self.state.light_log.capacity() * size_of::<(u64, (i32, i32, i32))>()
                    + self.state.light_scratch.capacity() * size_of::<(i64, i32, i32)>(),
            ),
        }
    }
}
