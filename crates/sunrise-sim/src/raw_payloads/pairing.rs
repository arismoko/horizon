//! One already-framed attachment sync and its prepaid outbox reservation.
//! Kept through deferred tracking callbacks and refunded when written or dropped.
use super::connection::Epoch;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

pub struct PairingPayload {
    pub(crate) bytes: Vec<u8>,
    pub(crate) epoch: Arc<Epoch>,
    pub(crate) pending: Arc<AtomicUsize>,
    pub(crate) cost: usize,
}
impl Drop for PairingPayload {
    fn drop(&mut self) {
        self.pending.fetch_sub(self.cost, Ordering::Relaxed);
    }
}
