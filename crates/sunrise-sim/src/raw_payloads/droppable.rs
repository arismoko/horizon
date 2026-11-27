//! `Droppable` payload delivery: one pre-sized queue per connection, before vanilla's framing,
//! compression and encryption. A send waits here until the connection's writer thread has room
//! and takes it; while it waits it is dropped and counted past the byte watermark, after
//! [`STALE`], at the switch to play, and over the byte budget. A packet that left the queue is
//! never removed again, so the byte stream with a dropped send equals the stream without it.
use mod_api::__private::game::DropCounters;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock, atomic::Ordering::Relaxed},
    time::{Duration, Instant},
};

/// A send that waited this long is dropped, oldest first.
pub const STALE: Duration = Duration::from_millis(250);
/// Waiting bytes past this drop the oldest waiting send: about 250 ms of the default budget.
pub const WATERMARK: usize = 64 << 10;
/// Waiting sends held at most; the queue is allocated once.
const SLOTS: usize = 256;
/// Server to client bytes per second by default.
pub const CLIENTBOUND_BUDGET: u32 = 256 << 10;
/// Client to server bytes per second by default.
pub const SERVERBOUND_BUDGET: u32 = 16 << 10;

/// Byte budgets per second; 0 turns `Droppable` delivery off in that direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budgets {
    pub clientbound: u32,
    pub serverbound: u32,
}
impl Budgets {
    /// The defaults, raised or lowered by `SUNRISE_DROPPABLE_CLIENTBOUND_BYTES_PER_SECOND` and
    /// `SUNRISE_DROPPABLE_SERVERBOUND_BYTES_PER_SECOND` (set by `server.toml`, `[engine] droppable_*`).
    pub fn from_env() -> Self {
        Self::read(|name| std::env::var(name).ok())
    }
    fn read(get: impl Fn(&str) -> Option<String>) -> Self {
        let read = |name: &str, default: u32| match get(name) {
            Some(value) => value.trim().parse().unwrap_or_else(|_| {
                log::warn!("{name} is not a byte count: using {default}");
                default
            }),
            None => default,
        };
        Self {
            clientbound: read(
                "SUNRISE_DROPPABLE_CLIENTBOUND_BYTES_PER_SECOND",
                CLIENTBOUND_BUDGET,
            ),
            serverbound: read(
                "SUNRISE_DROPPABLE_SERVERBOUND_BYTES_PER_SECOND",
                SERVERBOUND_BUDGET,
            ),
        }
    }
}

/// Bytes allowed per second, with one second of burst.
struct Bucket {
    rate: u32,
    level: u64,
    at: Instant,
}
impl Bucket {
    fn new(rate: u32, now: Instant) -> Self {
        Self {
            rate,
            level: u64::from(rate),
            at: now,
        }
    }
    fn take(&mut self, now: Instant, bytes: usize) -> bool {
        let elapsed = now.saturating_duration_since(self.at);
        self.at = now;
        let refill = (u128::from(self.rate) * elapsed.as_nanos() / 1_000_000_000) as u64;
        self.level = (self.level + refill).min(u64::from(self.rate));
        match self.level.checked_sub(bytes as u64) {
            Some(left) => {
                self.level = left;
                true
            }
            None => false,
        }
    }
    fn refund(&mut self, bytes: usize) {
        self.level = (self.level + bytes as u64).min(u64::from(self.rate));
    }
}

struct Waiting {
    since: Instant,
    counters: Arc<DropCounters>,
    body: Vec<u8>,
}
struct Inner {
    accepting: bool,
    waiting: VecDeque<Waiting>,
    bytes: usize,
    send: Bucket,
    receive: Option<Bucket>,
}
impl Inner {
    fn drop_front(&mut self, why: fn(&DropCounters) -> &std::sync::atomic::AtomicU64) {
        if let Some(gone) = self.waiting.pop_front() {
            self.bytes -= gone.body.len();
            self.send.refund(gone.body.len());
            why(&gone.counters).fetch_add(1, Relaxed);
        }
    }
    fn drop_stale(&mut self, now: Instant) {
        while self
            .waiting
            .front()
            .is_some_and(|w| now.saturating_duration_since(w.since) > STALE)
        {
            self.drop_front(|c| &c.stale);
        }
    }
}

/// One connection's waiting `Droppable` sends.
pub struct Queue {
    inner: Mutex<Inner>,
    wake: OnceLock<Box<dyn Fn() + Send + Sync>>,
}
impl Queue {
    /// `send` is this side's outgoing budget, `receive` the limit it enforces on the peer
    /// (the server only).
    pub fn new(send: u32, receive: Option<u32>) -> Arc<Self> {
        let now = Instant::now();
        Arc::new(Self {
            inner: Mutex::new(Inner {
                accepting: true,
                waiting: VecDeque::with_capacity(SLOTS),
                bytes: 0,
                send: Bucket::new(send, now),
                receive: receive.map(|rate| Bucket::new(rate, now)),
            }),
            wake: OnceLock::new(),
        })
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// How the connection's writer thread is woken when a send arrives; set once.
    pub fn set_wake(&self, wake: Box<dyn Fn() + Send + Sync>) {
        let _ = self.wake.set(wake);
    }
    /// Queue an encoded packet body: over budget, stale sends go first (oldest first), then
    /// this one; past the watermark the oldest waiting send goes.
    pub fn push(&self, now: Instant, counters: &Arc<DropCounters>, body: Vec<u8>) {
        let mut inner = self.lock();
        if !inner.accepting {
            return;
        }
        let len = body.len();
        if !inner.send.take(now, len) {
            inner.drop_stale(now);
            if !inner.send.take(now, len) {
                counters.budget.fetch_add(1, Relaxed);
                return;
            }
        }
        if inner.waiting.len() == SLOTS {
            inner.drop_front(|c| &c.watermark);
        }
        inner.bytes += len;
        inner.waiting.push_back(Waiting {
            since: now,
            counters: counters.clone(),
            body,
        });
        while inner.bytes > WATERMARK && inner.waiting.len() > 1 {
            inner.drop_front(|c| &c.watermark);
        }
        drop(inner);
        if let Some(wake) = self.wake.get() {
            wake();
        }
    }
    /// Stop taking new sends, including ones prepared earlier. Keep any
    /// published entries available to the draining writer.
    pub(crate) fn seal(&self) -> bool {
        let mut inner = self.lock();
        inner.accepting = false;
        !inner.waiting.is_empty()
    }
    /// The oldest waiting send that is not stale, for the writer thread that has room.
    pub fn pop(&self, now: Instant) -> Option<Vec<u8>> {
        let mut inner = self.lock();
        inner.drop_stale(now);
        let next = inner.waiting.pop_front()?;
        inner.bytes -= next.body.len();
        Some(next.body)
    }
    /// Whether the peer may send `len` more bytes (the server's serverbound budget).
    pub fn admit(&self, now: Instant, len: usize) -> bool {
        self.lock()
            .receive
            .as_mut()
            .is_none_or(|bucket| bucket.take(now, len))
    }
    /// Drop everything waiting: the connection left configuration, or ended.
    pub fn clear(&self) {
        let mut inner = self.lock();
        while !inner.waiting.is_empty() {
            inner.drop_front(|c| &c.switch);
        }
    }
}

#[cfg(test)]
#[path = "droppable_tests.rs"]
mod tests;
