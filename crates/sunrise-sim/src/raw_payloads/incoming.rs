//! Owned application messages and work retain ingress credit through root completion.
use super::{
    Transport,
    connection::{Epoch, IngressLease, MainTask},
};
use mod_api::{
    __private::game::{CallbackFailure, PayloadReceiver},
    PayloadBytes, PayloadContextError, PayloadId, PayloadSendError, PayloadWork,
};
use std::sync::Arc;
use std::time::Instant;
use sunrise_protocol::{packets::custom_payload::CustomPayload, payload_negotiation::Phase};

pub enum Body {
    Payload(PayloadBytes),
    Work(Arc<dyn PayloadWork>),
}
pub struct Incoming {
    pub epoch: Arc<Epoch>,
    pub id: PayloadId,
    pub body: Body,
    pub receiver: PayloadReceiver,
    pub main: MainTask,
    /// Not refunded by moving from I/O to Hub, backlog, decoder barrier or executor.
    pub lease: Arc<IngressLease>,
    /// A queued work resource may retain its original payload buffer. Keep that
    /// original credit as well as this additional work record's envelope credit.
    pub work_lease: Option<IngressLease>,
    /// A raw channel's delivery to one of its declaring mods: a handler's
    /// error is that mod's fault, not the peer's, and doesn't end the session.
    pub raw: bool,
    pub(super) turn: Option<super::connection::PayloadTurn>,
}
const _: () = assert!(std::mem::size_of::<Incoming>() <= 256);
pub enum Classified {
    Application(Incoming),
    /// A raw channel's message, once per non-poisoned receiver in load order.
    Raw(Vec<Incoming>),
    /// A negotiated packet whose receiving callback has already panicked.
    Skipped,
    Other(CustomPayload),
}
#[derive(Debug)]
pub struct Failure {
    pub id: PayloadId,
    pub error: PayloadSendError,
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "payload {} refused: {:?}", self.id.0, self.error)
    }
}
impl Incoming {
    /// Owned transport buffers and reference storage, excluding this inline record.
    /// Work is conservatively charged its associated input per queued reference;
    /// arbitrary module-owned captures belong to the module, not a traversable packet.
    pub fn owned_heap_bytes(&self) -> usize {
        let body = match &self.body {
            Body::Payload(bytes) => bytes.0.capacity(),
            Body::Work(_) => self.lease.owned_bytes,
        };
        self.id
            .0
            .capacity()
            .saturating_add(body)
            .saturating_add(std::mem::size_of::<IngressLease>() + 2 * std::mem::size_of::<usize>())
    }
    pub fn new(
        transport: &Transport,
        phase: Phase,
        packet: CustomPayload,
    ) -> Result<Self, PayloadSendError> {
        match Self::classify(transport, phase, packet).map_err(|failure| failure.error)? {
            Classified::Application(incoming) => Ok(incoming),
            Classified::Raw(mut incoming) if incoming.len() == 1 => Ok(incoming.remove(0)),
            Classified::Raw(_) => Err(PayloadSendError::EncodeFailed(
                "a raw channel has several receivers".into(),
            )),
            Classified::Skipped => Err(PayloadSendError::EncodeFailed(
                "the receiving callback was skipped".into(),
            )),
            Classified::Other(packet) => {
                Err(PayloadSendError::UnknownPayload(PayloadId(packet.channel)))
            }
        }
    }
    /// One successful receiving-slot lookup; ownership fallback is only on a miss.
    pub fn classify(
        transport: &Transport,
        phase: Phase,
        packet: CustomPayload,
    ) -> Result<Classified, Failure> {
        let Some(epoch) = transport.epoch() else {
            return if transport.local.owns(&packet.channel) {
                Err(Failure {
                    id: PayloadId(packet.channel),
                    error: PayloadSendError::NotNegotiated,
                })
            } else {
                Ok(Classified::Other(packet))
            };
        };
        let fail = |error, packet: CustomPayload| Failure {
            id: PayloadId(packet.channel),
            error,
        };
        if !epoch.active() {
            return Err(fail(PayloadSendError::StaleEpoch, packet));
        }
        if epoch.phase != phase {
            return Err(fail(
                PayloadSendError::WrongProtocol(PayloadId(packet.channel.clone())),
                packet,
            ));
        }
        // A negotiated engine message goes on to the engine's own decoder.
        if epoch.peer.engine_receives(phase, &packet.channel) {
            return Ok(Classified::Other(packet));
        }
        let receiver = match epoch.peer.receiver(phase, &packet.channel) {
            Ok(receiver) => receiver,
            Err(PayloadSendError::UnknownPayload(_)) => {
                return Self::raw(transport, &epoch, phase, packet);
            }
            Err(error) => return Err(fail(error, packet)),
        };
        // Negotiation still applies to a poisoned channel, but no callback credit is needed.
        if receiver.poisoned() {
            return Ok(Classified::Skipped);
        }
        // A client over the serverbound byte budget loses the send, counted, never the session.
        if epoch.peer.local.has_droppable()
            && epoch.peer.side == super::Side::Server
            && let Some(counters) = epoch.peer.local.droppable(&packet.channel)
            && let Some(queue) = transport
                .connection
                .as_ref()
                .and_then(|connection| connection.droppable())
            && !queue.admit(Instant::now(), packet.data.len())
        {
            counters
                .budget
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Ok(Classified::Raw(Vec::new()));
        }
        let owned = packet
            .channel
            .capacity()
            .saturating_add(packet.data.capacity());
        let lease = transport.reserve_ingress(owned).map_err(|error| Failure {
            id: PayloadId(packet.channel.clone()),
            error,
        })?;
        let receiver = receiver.clone();
        let main = epoch.main_task();
        Ok(Classified::Application(Self {
            epoch,
            id: PayloadId(packet.channel),
            body: Body::Payload(PayloadBytes(packet.data)),
            receiver,
            main,
            lease: Arc::new(lease),
            work_lease: None,
            raw: false,
            turn: None,
        }))
    }
    /// A serverbound play message on a raw channel: one delivery for each
    /// declaring mod, in load order, each charged its own ingress credit. An
    /// undeclared or send-only channel stays an unrelated plugin channel.
    fn raw(
        transport: &Transport,
        epoch: &Arc<Epoch>,
        phase: Phase,
        packet: CustomPayload,
    ) -> Result<Classified, Failure> {
        let receivers = transport.local.raw_receivers(&packet.channel);
        if phase != Phase::Play || epoch.peer.side != super::Side::Server || receivers.is_empty() {
            return Ok(Classified::Other(packet));
        }
        let mut deliveries = Vec::with_capacity(receivers.len());
        for receiver in receivers {
            if receiver.poisoned() {
                continue;
            }
            let owned = packet.channel.capacity().saturating_add(packet.data.len());
            let lease = transport.reserve_ingress(owned).map_err(|error| Failure {
                id: PayloadId(packet.channel.clone()),
                error,
            })?;
            deliveries.push(Self {
                epoch: epoch.clone(),
                id: PayloadId(packet.channel.clone()),
                body: Body::Payload(PayloadBytes(packet.data.clone())),
                receiver: receiver.clone(),
                main: epoch.main_task(),
                lease: Arc::new(lease),
                work_lease: None,
                raw: true,
                turn: None,
            });
        }
        Ok(Classified::Raw(deliveries))
    }
    /// Take the body without releasing its receiving turn or ingress credit.
    pub fn take_body(&mut self) -> Body {
        std::mem::replace(&mut self.body, Body::Payload(PayloadBytes(Vec::new())))
    }
    /// Dispose of any call-owned work under its owning mod's panic policy.
    pub(super) fn dispose_body(&mut self) -> Result<(), CallbackFailure> {
        match self.take_body() {
            Body::Work(work) => self.receiver.dispose_work(work),
            Body::Payload(_) => Ok(()),
        }
    }
    /// Reserve before cloning the channel or retaining the mod's work resource.
    pub(super) fn work(
        &self,
        transport: &Transport,
        work: Arc<dyn PayloadWork>,
    ) -> Result<Self, (PayloadContextError, Option<CallbackFailure>)> {
        if !self.epoch.active() {
            return Err((
                PayloadContextError::Disconnected,
                self.receiver.dispose_work(work).err(),
            ));
        }
        let lease = match transport.reserve_ingress(self.id.0.len()) {
            Ok(lease) => lease,
            Err(error) => {
                let error = match error {
                    PayloadSendError::Backpressure => PayloadContextError::Backpressure,
                    _ => PayloadContextError::Disconnected,
                };
                return Err((error, self.receiver.dispose_work(work).err()));
            }
        };
        Ok(Self {
            epoch: self.epoch.clone(),
            id: self.id.clone(),
            receiver: self.receiver.clone(),
            main: self.epoch.main_task(),
            body: Body::Work(work),
            lease: self.lease.clone(),
            work_lease: Some(lease),
            raw: self.raw,
            turn: None,
        })
    }
}
impl Drop for Incoming {
    fn drop(&mut self) {
        let _ = self.dispose_body();
    }
}
