//! One bounded writer owner. Receiving callbacks never block their own receive loop
//! while a large reply waits for the peer to drain its socket or memory pipe.
use mod_api::{PayloadFrameStage, PayloadId, PayloadSendError, SendPacketError};
use std::sync::{
    Arc, Condvar, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};
use sunrise_mod_runtime::byte_channel::{self, Permit, ReserveError, Sender, TrySendError};
use sunrise_protocol::payload_negotiation::Phase;
use sunrise_protocol::{
    Error, Result,
    connection::{Closer, ConnWriter, Framing},
    frame::FrameSize,
};

use std::thread::JoinHandle;

const CAPACITY: usize = 16 * 1024 * 1024;
// Covers the pending transaction journal record as well as the queue's own node.
const PUBLICATION_OVERHEAD: usize = 512;
enum WriteJob {
    Packet(Vec<u8>),
    Contents(Vec<u8>),
    Disconnect(Vec<u8>),
    /// A `Droppable` send is waiting: the thread frames what it has room for.
    Droppable,
}

#[derive(Clone)]
pub struct QueuedWriter(Arc<Inner>);
pub struct Inner {
    #[cfg(any(test, feature = "test-payload-probe"))]
    // attempts, successful send returns, active input bytes, active calls.
    // ConnWriter::send_contents returns after write_all; it does not flush.
    probe: Arc<[std::sync::atomic::AtomicUsize; 4]>,
    sender: Mutex<Option<Sender<WriteJob>>>,
    droppable: Arc<std::sync::OnceLock<Arc<super::droppable::Queue>>>,
    /// A wake-up is already in the channel.
    droppable_woken: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<ConnWriter>>>,
    framing: Framing,
    closer: Arc<Closer>,
    alive: Arc<AtomicBool>,
    retired: AtomicBool,
    terminal: Arc<AtomicBool>,
    done: Arc<(Mutex<bool>, Condvar)>,
    phase: Mutex<(u64, Phase)>,
}
impl std::ops::Deref for QueuedWriter {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

/// Prepared once, charged immediately, invisible to the writer until root commit.
pub struct PreparedPayload {
    permit: Permit<WriteJob>,
    job: WriteJob,
    alive: Arc<AtomicBool>,
}
impl PreparedPayload {
    pub fn publish(self) -> std::result::Result<(), PayloadSendError> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(PayloadSendError::Disconnected);
        }
        self.permit
            .publish(self.job)
            .map_err(|_| PayloadSendError::Disconnected)
    }
}
impl crate::raw_payloads::PreparedSend for PreparedPayload {
    fn publish(self: Box<Self>) -> std::result::Result<(), PayloadSendError> {
        (*self).publish()
    }
}
enum PrepareFailure {
    Closed,
    Full,
    /// Full only because unpublished held packets hold the credit: the writer is healthy.
    Held,
    Encode(Error),
}

/// Why a play packet could not be sent.
#[derive(Debug)]
pub enum SendFailure {
    /// Only unpublished held packets hold the credit: the writer is healthy.
    Held,
    Failed(Error),
}

impl SendFailure {
    fn into_error(self) -> Error {
        match self {
            Self::Held => Error::new("client writer credit held"),
            Self::Failed(error) => error,
        }
    }
}

/// Why a play packet could not be reserved.
#[derive(Debug)]
pub enum PrepareRefusal {
    /// The held packets alone fill the writer's credit: refuse this send, keep the writer
    /// and the holds.
    Held,
    /// The writer or the packet is at fault.
    Failed(SendPacketError),
}

impl PrepareRefusal {
    /// What the mod sees.
    pub fn error(self) -> SendPacketError {
        match self {
            Self::Held => SendPacketError::Backpressure,
            Self::Failed(error) => error,
        }
    }
}

/// An exact physical writer and protocol incarnation, never a Play/CFG boolean.
pub struct PreparedPacket {
    writer: Weak<Inner>,
    incarnation: (u64, u64),
    payload: PreparedPayload,
}
impl PreparedPacket {
    /// Physical session serial and play generation this reservation belongs to.
    pub fn incarnation(&self) -> (u64, u64) {
        self.incarnation
    }
    /// Publish using the original credit; never append behind a protocol ACK or terminal send.
    pub fn try_publish(self) -> std::result::Result<(), SendPacketError> {
        let writer = self.writer.upgrade().ok_or(SendPacketError::NotConnected)?;
        let phase = writer.phase.lock().unwrap();
        if phase.0 != self.incarnation.1 || phase.1 != Phase::Play {
            return Err(SendPacketError::NotInPlay);
        }
        let sender = writer.sender.lock().unwrap();
        if sender.is_none() {
            return Err(SendPacketError::NotConnected);
        }
        self.payload
            .publish()
            .map_err(|_| SendPacketError::NotConnected)
    }
}
impl QueuedWriter {
    pub fn play_incarnation(&self) -> Option<u64> {
        let phase = self.phase.lock().unwrap();
        (phase.1 == Phase::Play
            && self.alive.load(Ordering::Acquire)
            && !self.is_terminal()
            && !self.retired.load(Ordering::Acquire))
        .then_some(phase.0)
    }

    #[cfg(any(test, feature = "test-payload-probe"))]
    pub fn exhaust_play_credit(&self, session_serial: u64) -> Vec<PreparedPacket> {
        let mut held = Vec::new();
        let chunk = vec![0x5au8; 32_767];
        loop {
            match self.prepare_packet(chunk.clone(), session_serial, None) {
                Ok(packet) => held.push(packet),
                Err(PrepareRefusal::Held) => break,
                Err(error) => panic!("unexpected prepare_packet error: {error:?}"),
            }
        }
        let overhead = PUBLICATION_OVERHEAD + byte_channel::message_overhead::<WriteJob>();
        let mut remaining = {
            let sender = self.sender.lock().unwrap();
            let stats = sender.as_ref().unwrap().stats();
            assert_eq!(stats.queued_items, 0);
            CAPACITY - stats.reserved_bytes
        };
        if remaining < overhead {
            held.pop().expect("at least one held packet");
            let sender = self.sender.lock().unwrap();
            let stats = sender.as_ref().unwrap().stats();
            remaining = CAPACITY - stats.reserved_bytes;
        }
        let exact = vec![0x5au8; remaining - overhead];
        held.push(
            self.prepare_packet(exact, session_serial, None)
                .expect("exact tail fills remaining writer credit"),
        );
        let sender = self.sender.lock().unwrap();
        let stats = sender.as_ref().unwrap().stats();
        assert_eq!(stats.queued_items, 0);
        assert_eq!(stats.reserved_bytes, CAPACITY);
        held
    }
    #[cfg(any(test, feature = "test-payload-probe"))]
    pub fn diagnostics(&self) -> String {
        let stats = match self.sender.try_lock() {
            Ok(sender) => sender.as_ref().map(|s| s.stats()),
            Err(_) => return "writer sender busy/poisoned".into(),
        };
        let window = stats.map(|s| CAPACITY.saturating_sub(s.reserved_bytes));
        format!(
            "writer window={window:?} capacity={CAPACITY} alive={} terminal={} retired={} stats={stats:?} send_attempts={} send_ok={} active_send_input_bytes={} active_sends={} flush_count=unavailable(no flush call)",
            self.alive.load(Ordering::Acquire),
            self.terminal.load(Ordering::Acquire),
            self.retired.load(Ordering::Acquire),
            self.probe[0].load(Ordering::Relaxed),
            self.probe[1].load(Ordering::Relaxed),
            self.probe[2].load(Ordering::Relaxed),
            self.probe[3].load(Ordering::Relaxed)
        )
    }
    /// Revoke unpublished old-play reservations, queue the ACK, and change
    /// incarnation under the generic-publication gate.
    /// Raw publication uses its separate epoch gate; this never acquires it.
    pub fn transition_protocol(&self, protocol: Phase, ack: &[u8]) -> Result<()> {
        let mut phase = self.phase.lock().unwrap();
        let generation = phase
            .0
            .checked_add(1)
            .ok_or_else(|| Error::new("network phase incarnation exhausted"))?;
        if !self.alive.load(Ordering::Acquire) {
            return Err(Error::new("client writer disconnected"));
        }
        let sender = self.sender.lock().unwrap();
        let Some(sender) = sender.as_ref() else {
            return Err(Error::new("writer retired"));
        };
        if phase.1 == Phase::Play {
            sender.revoke_pending();
        }
        self.send_with(sender, ack, false)
            .map_err(SendFailure::into_error)?;
        *phase = (generation, protocol);
        Ok(())
    }
    pub fn prepare_packet(
        &self,
        body: Vec<u8>,
        session_serial: u64,
        expected_generation: Option<u64>,
    ) -> std::result::Result<PreparedPacket, PrepareRefusal> {
        let phase = self.phase.lock().unwrap();
        let (generation, protocol) = *phase;
        if protocol != Phase::Play
            || expected_generation.is_some_and(|expected| expected != generation)
        {
            return Err(PrepareRefusal::Failed(SendPacketError::NotInPlay));
        }
        let payload = self
            .prepare_contents(body, true)
            .map_err(|error| match error {
                PrepareFailure::Closed => PrepareRefusal::Failed(SendPacketError::NotConnected),
                PrepareFailure::Full => PrepareRefusal::Failed(SendPacketError::Backpressure),
                PrepareFailure::Held => PrepareRefusal::Held,
                PrepareFailure::Encode(error) => {
                    PrepareRefusal::Failed(match error.frame_limit() {
                        Some(detail) => SendPacketError::FrameTooLarge {
                            size: detail.size as u64,
                            limit: detail.limit as u64,
                        },
                        None => SendPacketError::EncodeFailed {
                            message: error.to_string(),
                        },
                    })
                }
            })?;
        Ok(PreparedPacket {
            writer: Arc::downgrade(&self.0),
            incarnation: (session_serial, generation),
            payload,
        })
    }
    pub fn new(writer: ConnWriter) -> Result<Self> {
        let framing = writer.framing();
        Self::with_framing(writer, framing)
    }
    pub fn with_framing(mut writer: ConnWriter, framing: Framing) -> Result<Self> {
        let closer = Arc::new(writer.closer()?);
        let alive = Arc::new(AtomicBool::new(true));
        let (sender, receiver) = byte_channel::channel(CAPACITY).expect("nonzero writer capacity");
        #[cfg(any(test, feature = "test-payload-probe"))]
        let probe = Arc::new(std::array::from_fn::<_, 4, _>(|_| {
            std::sync::atomic::AtomicUsize::new(0)
        }));
        #[cfg(any(test, feature = "test-payload-probe"))]
        let writing = probe.clone();
        let running = alive.clone();
        let terminal = Arc::new(AtomicBool::new(false));
        let closing = terminal.clone();
        let done = Arc::new((Mutex::new(false), Condvar::new()));
        let finished = done.clone();
        let droppable: Arc<std::sync::OnceLock<Arc<super::droppable::Queue>>> = Arc::default();
        let droppable_woken = Arc::new(AtomicBool::new(false));
        let (serving, woken) = (droppable.clone(), droppable_woken.clone());
        let worker = std::thread::Builder::new()
            .name("sunrise-net-write".into())
            .spawn(move || {
                while let Ok(job) = receiver.recv() {
                    let disconnect = matches!(&job, WriteJob::Disconnect(_));
                    #[cfg(any(test, feature = "test-payload-probe"))]
                    {
                        let bytes = match &job {
                            WriteJob::Packet(b)
                            | WriteJob::Contents(b)
                            | WriteJob::Disconnect(b) => b.len(),
                            WriteJob::Droppable => 0,
                        };
                        writing[0].fetch_add(1, Ordering::Relaxed);
                        writing[2].store(bytes, Ordering::Relaxed);
                        writing[3].store(1, Ordering::Relaxed);
                    }
                    let mut result = match job {
                        WriteJob::Packet(body) | WriteJob::Disconnect(body) => writer.send(&body),
                        WriteJob::Contents(contents) => writer.send_contents(&contents),
                        WriteJob::Droppable => {
                            woken.store(false, Ordering::Release);
                            Ok(())
                        }
                    };
                    // The writer is free: frame the waiting `Droppable` sends before the next job.
                    if result.is_ok()
                        && !disconnect
                        && let Some(queue) = serving.get()
                    {
                        while let Some(body) = queue.pop(std::time::Instant::now()) {
                            result = writer.send(&body);
                            if result.is_err() {
                                break;
                            }
                        }
                    }
                    #[cfg(any(test, feature = "test-payload-probe"))]
                    {
                        if result.is_ok() {
                            writing[1].fetch_add(1, Ordering::Relaxed);
                        }
                        writing[2].store(0, Ordering::Relaxed);
                        writing[3].store(0, Ordering::Relaxed);
                    }
                    if let Err(error) = &result {
                        log::warn!("[net] failed to flush outgoing packet: {error}");
                    }
                    if result.is_err() || disconnect {
                        running.store(false, Ordering::Release);
                        writer.shutdown();
                        break;
                    }
                }
                if closing.load(Ordering::Acquire) {
                    running.store(false, Ordering::Release);
                    writer.shutdown();
                }
                let (lock, cvar) = &*finished;
                *lock.lock().unwrap() = true;
                cvar.notify_all();
                writer
            })
            .map_err(|error| Error::new(format!("cannot start client writer: {error}")))?;
        Ok(Self(Arc::new(Inner {
            #[cfg(any(test, feature = "test-payload-probe"))]
            probe,
            sender: Mutex::new(Some(sender)),
            droppable,
            droppable_woken,
            worker: Mutex::new(Some(worker)),
            retired: AtomicBool::new(false),
            terminal,
            done,
            framing,
            closer,
            alive,
            phase: Mutex::new((0, Phase::Configuration)),
        })))
    }
    /// Stop acceptance, cancel unpublished reservations and drain published writes.
    pub fn drain_and_take(&self) -> Result<ConnWriter> {
        let mut sender = self.sender.lock().unwrap();
        self.retired.store(true, Ordering::Release);
        sender.take();
        drop(sender);
        let worker = self
            .worker
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| Error::new("writer already taken"))?;
        let writer = worker.join().map_err(|_| Error::new("writer panicked"))?;
        if !self.alive.swap(false, Ordering::AcqRel) {
            return Err(Error::new("writer disconnected"));
        }
        Ok(writer)
    }
    /// Serve `queue`'s `Droppable` sends from this writer's thread.
    pub fn attach_droppable(&self, queue: Arc<super::droppable::Queue>) {
        let inner = Arc::downgrade(&self.0);
        queue.set_wake(Box::new(move || {
            let Some(inner) = inner.upgrade() else { return };
            if inner.droppable_woken.swap(true, Ordering::AcqRel) {
                return;
            }
            let sender = inner.sender.lock().unwrap();
            let sent = sender
                .as_ref()
                .is_some_and(|s| s.try_send(WriteJob::Droppable, 0).is_ok());
            if !sent {
                inner.droppable_woken.store(false, Ordering::Release);
            }
        }));
        let _ = self.droppable.set(queue);
    }
    pub fn is_terminal(&self) -> bool {
        self.terminal.load(Ordering::Acquire)
    }
    pub fn accepts_packets(&self) -> bool {
        !self.is_terminal() && self.alive.load(Ordering::Acquire)
    }
    pub fn framing(&self) -> Framing {
        self.framing
    }
    pub fn payload_capacity(&self) -> usize {
        CAPACITY
    }
    pub fn shutdown(&self) {
        if self.retired.load(Ordering::Acquire) {
            return;
        }
        self.alive.store(false, Ordering::Release);
        if !self.retired.load(Ordering::Acquire) {
            self.closer.close();
        }
        self.sender.lock().unwrap().take();
    }
    pub fn send(&self, body: &[u8]) -> Result<()> {
        self.send_inner(body, false)
    }
    /// Enqueue a hold-excluded protocol reply against published and non-revocable
    /// credit only, so unpublished held play packets cannot starve it while the
    /// peer is draining normally.
    pub fn send_bypassing_held(&self, body: &[u8]) -> Result<()> {
        self.send_inner(body, true)
    }
    fn send_inner(&self, body: &[u8], bypass_revocable: bool) -> Result<()> {
        self.send_checked(body, bypass_revocable)
            .map_err(SendFailure::into_error)
    }
    /// A play send that tells held-packet pressure (refused, writer untouched) from a failure
    /// of the writer itself (which closes it).
    pub fn send_play(&self, body: &[u8]) -> std::result::Result<(), SendFailure> {
        self.send_checked(body, false)
    }
    fn send_checked(
        &self,
        body: &[u8],
        bypass_revocable: bool,
    ) -> std::result::Result<(), SendFailure> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(SendFailure::Failed(Error::new(
                "client writer disconnected",
            )));
        }
        let sender = self.sender.lock().unwrap();
        let Some(sender) = sender.as_ref() else {
            return Err(SendFailure::Failed(Error::new("writer retired")));
        };
        self.send_with(sender, body, bypass_revocable)
    }
    fn send_with(
        &self,
        sender: &Sender<WriteJob>,
        body: &[u8],
        bypass_revocable: bool,
    ) -> std::result::Result<(), SendFailure> {
        let owned = body.to_vec();
        let capacity = owned.capacity();
        let result = if bypass_revocable {
            sender.try_send_excluding_revocable(WriteJob::Packet(owned), capacity)
        } else {
            sender.try_send(WriteJob::Packet(owned), capacity)
        };
        match result {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_))
                if !bypass_revocable && sender.fits_without_revocable(capacity) =>
            {
                Err(SendFailure::Held)
            }
            Err(TrySendError::Full(_)) => {
                self.alive.store(false, Ordering::Release);
                self.closer.close();
                Err(SendFailure::Failed(Error::new(
                    "client writer backpressure",
                )))
            }
            Err(TrySendError::Disconnected(_)) => Err(SendFailure::Failed(Error::new(
                "client writer disconnected",
            ))),
        }
    }
    /// Disconnect/transfer reached in an ordered bundle: stop accepting new
    /// writes, but let the writer send replies that came before the close.
    pub fn close_after_pending(&self) {
        let mut sender = self.sender.lock().unwrap();
        if self.seal_droppable()
            && let Some(sender) = sender.as_ref()
            && let Ok(marker) = sender.reserve_terminal(0)
        {
            // Unpublished reliable reservations can fill the channel without
            // leaving any queued job to wake the writer and poll this lane.
            let _ = marker.publish(WriteJob::Droppable);
        }
        self.terminal.store(true, Ordering::Release);
        sender.take();
    }

    /// Stop accepting writes, drain preceding replies, and wait for the transport shutdown to complete.
    pub fn close_and_wait(&self) {
        self.close_after_pending();
        let (lock, cvar) = &*self.done;
        let mut done = lock.lock().unwrap();
        if !*done {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while !*done {
                let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now())
                else {
                    break;
                };
                let (guard, timeout) = cvar.wait_timeout(done, remaining).unwrap();
                done = guard;
                if timeout.timed_out() {
                    break;
                }
            }
        }
        let finished = *done;
        drop(done);
        self.alive.store(false, Ordering::Release);
        if !finished {
            // A stalled peer can block `write_all`; close the transport to unblock the worker.
            self.closer.close();
        }
        if let Some(worker) = self.worker.lock().unwrap().take() {
            let _ = worker.join();
        }
        self.closer.close();
    }

    fn seal_droppable(&self) -> bool {
        self.droppable.get().is_some_and(|queue| queue.seal())
    }

    pub fn disconnect(&self, body: Vec<u8>) {
        let mut sender = self.sender.lock().unwrap();
        self.seal_droppable();
        self.terminal.store(true, Ordering::Release);
        let Some(queue) = sender.take() else {
            return;
        };
        queue.revoke_pending();
        let bytes = body.capacity();
        if queue.try_send(WriteJob::Disconnect(body), bytes).is_err() {
            self.closer.close();
        }
    }
    pub fn prepare_payload(
        &self,
        body: Vec<u8>,
        id: &PayloadId,
    ) -> std::result::Result<PreparedPayload, PayloadSendError> {
        self.prepare_contents(body, false)
            .map_err(|failure| match failure {
                PrepareFailure::Closed => PayloadSendError::Disconnected,
                PrepareFailure::Full | PrepareFailure::Held => PayloadSendError::Backpressure,
                PrepareFailure::Encode(error) => {
                    if let Some(detail) = error.frame_limit() {
                        PayloadSendError::FrameTooLarge {
                            id: id.clone(),
                            stage: match detail.stage {
                                FrameSize::Encoded => PayloadFrameStage::Encoded,
                                FrameSize::Uncompressed => PayloadFrameStage::Uncompressed,
                            },
                            size: detail.size as u64,
                            limit: detail.limit as u64,
                        }
                    } else {
                        PayloadSendError::EncodeFailed(error.to_string())
                    }
                }
            })
    }
    fn prepare_contents(
        &self,
        body: Vec<u8>,
        revocable: bool,
    ) -> std::result::Result<PreparedPayload, PrepareFailure> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(PrepareFailure::Closed);
        }
        let contents = self
            .framing
            .contents_owned(body)
            .map_err(PrepareFailure::Encode)?;
        self.prepare_job(WriteJob::Contents(contents), revocable)
    }
    fn prepare_job(
        &self,
        job: WriteJob,
        revocable: bool,
    ) -> std::result::Result<PreparedPayload, PrepareFailure> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(PrepareFailure::Closed);
        }
        let capacity = match &job {
            WriteJob::Packet(bytes) | WriteJob::Contents(bytes) | WriteJob::Disconnect(bytes) => {
                bytes.capacity()
            }
            WriteJob::Droppable => 0,
        };
        let sender = self.sender.lock().unwrap();
        let sender = sender.as_ref().ok_or(PrepareFailure::Closed)?;
        let charge = capacity.saturating_add(PUBLICATION_OVERHEAD);
        let permit = if revocable {
            sender.reserve_revocable(charge)
        } else {
            sender.reserve(charge)
        }
        .map_err(|error| match error {
            ReserveError::Full if sender.fits_without_revocable(capacity) => PrepareFailure::Held,
            ReserveError::Full => PrepareFailure::Full,
            ReserveError::Disconnected => PrepareFailure::Closed,
        })?;
        Ok(PreparedPayload {
            permit,
            job,
            alive: self.alive.clone(),
        })
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        if !self.retired.load(Ordering::Acquire) && !self.terminal.load(Ordering::Acquire) {
            self.closer.close();
        }
    }
}

#[cfg(test)]
mod tests;
