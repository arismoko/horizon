//! Typed bounded ingress. Charges survive moves into actor backlogs and decoders.
//! Each owner supplies its value's allocation/staging policy; this layer never
//! guesses from encoded length or invokes user callbacks under a queue lock.
pub use byte_channel::{OwnedReceipt, ReceiveLease, Stats};
use std::{cell::Cell, fmt, marker::PhantomData, sync::mpsc::TryRecvError};
use sunrise_mod_runtime::byte_channel;

pub struct Producer<T> {
    sender: byte_channel::Sender<T>,
    charge: fn(&T) -> usize,
}
impl<T> Clone for Producer<T> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            charge: self.charge,
        }
    }
}
/// One actor owns receiving turns. Like std's Receiver, this is Send but not Sync.
pub struct Consumer<T> {
    receiver: byte_channel::Receiver<T>,
    _single: PhantomData<Cell<()>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Pressure,
    Closed,
}
impl From<byte_channel::ReserveError> for Refusal {
    fn from(error: byte_channel::ReserveError) -> Self {
        match error {
            byte_channel::ReserveError::Full => Self::Pressure,
            byte_channel::ReserveError::Disconnected => Self::Closed,
        }
    }
}
pub struct Refused<T> {
    pub reason: Refusal,
    pub value: T,
}
impl<T> fmt::Debug for Refused<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.reason, f)
    }
}
impl<T> fmt::Display for Refused<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.reason {
            Refusal::Pressure => "incoming queue resource pressure",
            Refusal::Closed => "incoming receiver closed",
        })
    }
}
impl<T> std::error::Error for Refused<T> {}
pub struct Prepared<T> {
    value: T,
    permit: byte_channel::Permit<T>,
}
impl<T> Prepared<T> {
    /// Inspect the prepared value before publishing.
    pub fn value(&self) -> &T {
        &self.value
    }
    /// Consume the value inline, keeping its reservation until dispatch completes.
    pub fn consume<R>(self, run: impl FnOnce(T) -> R) -> R {
        let Self { value, permit } = self;
        let result = run(value);
        drop(permit);
        result
    }
    /// Failure returns the owned value, so its resources can be dropped after
    /// releasing any publication gate held by the caller.
    pub fn publish(self) -> Result<(), Refused<T>> {
        self.permit.publish(self.value).map_err(|value| Refused {
            reason: Refusal::Closed,
            value,
        })
    }
    /// Publish every prepared item from the same channel atomically under one lock.
    /// On closure, returns all owned values so they can be dropped outside any lock
    /// or publication gate.
    pub fn publish_batch(items: Vec<Self>) -> Result<(), Vec<Refused<T>>> {
        let pairs = items
            .into_iter()
            .map(|prepared| (prepared.permit, prepared.value))
            .collect();
        byte_channel::Permit::publish_batch(pairs).map_err(|values| {
            values
                .into_iter()
                .map(|value| Refused {
                    reason: Refusal::Closed,
                    value,
                })
                .collect()
        })
    }
}
pub fn channel<T>(
    capacity: usize,
    charge: fn(&T) -> usize,
) -> Result<(Producer<T>, Consumer<T>), byte_channel::InvalidCapacity> {
    let (sender, receiver) = byte_channel::channel(capacity)?;
    Ok((
        Producer { sender, charge },
        Consumer {
            receiver,
            _single: PhantomData,
        },
    ))
}
impl<T> Producer<T> {
    /// Reserve before reporting acceptance. Allocation traversal occurs before
    /// taking the channel lock, and all failure values leave that lock intact.
    pub fn prepare(&self, value: T) -> Result<Prepared<T>, Refused<T>> {
        self.prepare_with(value, byte_channel::Sender::reserve)
    }
    fn prepare_with(
        &self,
        value: T,
        reserve: impl FnOnce(
            &byte_channel::Sender<T>,
            usize,
        ) -> Result<byte_channel::Permit<T>, byte_channel::ReserveError>,
    ) -> Result<Prepared<T>, Refused<T>> {
        match reserve(&self.sender, (self.charge)(&value)) {
            Ok(permit) => Ok(Prepared { value, permit }),
            Err(error) => Err(Refused {
                reason: error.into(),
                value,
            }),
        }
    }
    /// Prepare an inline delivery from a dequeued value's credit on this channel.
    pub fn prepare_replacing(
        &self,
        value: T,
        credit: ReceiveLease<T>,
    ) -> Result<Prepared<T>, Refused<T>> {
        match credit.replace((self.charge)(&value)) {
            Ok(permit) => Ok(Prepared { value, permit }),
            Err(error) => Err(Refused {
                reason: error.into(),
                value,
            }),
        }
    }
    pub fn send(&self, value: T) -> Result<(), Refused<T>> {
        self.prepare(value)?.publish()
    }
    /// Apply backpressure on the receiving thread without refusing this record.
    pub fn send_wait(&self, value: T) -> Result<(), Refused<T>> {
        let charge = (self.charge)(&value);
        self.sender.send_wait(value, charge).map_err(|error| {
            use byte_channel::TrySendError;
            match error {
                TrySendError::Full(value) => Refused {
                    value,
                    reason: Refusal::Pressure,
                },
                TrySendError::Disconnected(value) => Refused {
                    value,
                    reason: Refusal::Closed,
                },
            }
        })
    }

    #[cfg(feature = "test-payload-probe")]
    pub fn hold_test_credit(&self, total_bytes: usize) -> byte_channel::Permit<T> {
        self.sender
            .reserve(total_bytes - byte_channel::message_overhead::<T>())
            .expect("hold the remaining test ingress credit")
    }

    pub fn send_terminal(&self, value: T) -> Result<(), Refused<T>> {
        self.prepare_with(value, byte_channel::Sender::reserve_terminal)?
            .publish()
    }

    pub fn stats(&self) -> Stats {
        self.sender.stats()
    }
}
impl<T> Consumer<T> {
    pub fn stats(&self) -> Stats {
        self.receiver.stats()
    }
    pub fn try_recv_owned(&self) -> Result<OwnedReceipt<T>, TryRecvError> {
        self.receiver.try_recv_owned()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn recv_timeout_owned(
        &self,
        timeout: std::time::Duration,
    ) -> Result<OwnedReceipt<T>, std::sync::mpsc::RecvTimeoutError> {
        self.receiver.recv_timeout_owned(timeout)
    }
    #[cfg(any(test, feature = "test-support", feature = "test-payload-probe"))]
    pub fn wait_for_parked_reader(&self) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while self.receiver.waiting_for_space() == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "reader did not park for credit"
            );
            std::thread::yield_now();
        }
    }
    pub fn close_reader_admission(&self) {
        self.receiver.close_wait_admission();
    }
    /// Only values already published at the beginning of this turn. New replies
    /// append behind these values and cannot feed back into the same actor turn.
    pub fn take_turn(&self) -> Vec<OwnedReceipt<T>> {
        let count = self.stats().queued_items;
        (0..count)
            .map_while(|_| self.try_recv_owned().ok())
            .collect()
    }
}
