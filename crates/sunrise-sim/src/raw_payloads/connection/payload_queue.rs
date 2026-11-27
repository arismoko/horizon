//! One physical connection's ordered receiving turns, including a main root in flight.
use super::{Connection, Epoch};
use crate::raw_payloads::Incoming;
use std::sync::atomic::Ordering;
use std::{
    collections::VecDeque,
    sync::{Arc, Weak},
};

#[derive(Default)]
pub(super) struct PayloadQueue {
    pending: VecDeque<Incoming>,
    active: bool,
}

/// Finishing or cancelling a root releases the next receiving turn, never invokes it.
pub(crate) struct PayloadTurn {
    connection: Weak<Connection>,
}
impl Drop for PayloadTurn {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.upgrade() {
            let mut queue = connection.payloads.lock().unwrap();
            queue.active = false;
            drop(queue);
            connection.payload_ready.notify_all();
        }
    }
}

impl Connection {
    /// An idle modded connection's maintenance turn only reads this flag.
    pub fn payloads_pending(&self) -> bool {
        self.payload_pending.load(Ordering::Acquire)
    }

    pub(in crate::raw_payloads) fn queue_payload(
        self: &Arc<Self>,
        mut incoming: Incoming,
    ) -> Option<Incoming> {
        let mut queue = self.payloads.lock().unwrap();
        if queue.active || !queue.pending.is_empty() {
            queue.pending.push_back(incoming);
            self.payload_pending.store(true, Ordering::Release);
            return None;
        }
        // The normal, uncontended path needs no queue allocation.
        queue.active = true;
        incoming.turn = Some(PayloadTurn {
            connection: Arc::downgrade(self),
        });
        Some(incoming)
    }

    pub(in crate::raw_payloads) fn next_payload(self: &Arc<Self>) -> Option<Incoming> {
        let mut queue = self.payloads.lock().unwrap();
        if queue.active {
            return None;
        }
        let mut incoming = queue.pending.pop_front()?;
        self.payload_pending
            .store(!queue.pending.is_empty(), Ordering::Release);
        queue.active = true;
        incoming.turn = Some(PayloadTurn {
            connection: Arc::downgrade(self),
        });
        Some(incoming)
    }

    /// Park a dedicated receiver until the active root finishes or its epoch ends.
    pub(crate) fn wait_payloads(&self, epoch: &Epoch) -> bool {
        let queue = self.payloads.lock().unwrap();
        let queue = self
            .payload_ready
            .wait_while(queue, |queue| {
                let waiting = queue.active && epoch.active();
                #[cfg(test)]
                if waiting {
                    super::test_hooks::notify();
                }
                waiting
            })
            .unwrap();
        epoch.active() && !queue.pending.is_empty()
    }

    pub(super) fn clear_payloads(&self) {
        let mut queue = self.payloads.lock().unwrap();
        let pending = std::mem::take(&mut queue.pending);
        self.payload_pending.store(false, Ordering::Release);
        drop(queue);
        self.payload_ready.notify_all();
        // Opaque mod work must never be destroyed under the queue mutex.
        drop(pending);
    }
}
