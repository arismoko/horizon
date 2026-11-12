//! Owned application messages and work retain ingress credit through root completion.
use super::{
    Transport,
    connection::{Epoch, IngressLease, MainTask},
};
use horizon_protocol::{packets::custom_payload::CustomPayload, payload_negotiation::Phase};
use mod_api::{
    __private::game::PayloadReceiver, PayloadBytes, PayloadContextError, PayloadId,
    PayloadSendError, PayloadWork,
};
use std::sync::Arc;

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
}
const _: () = assert!(std::mem::size_of::<Incoming>() <= 256);
pub enum Classified {
    Application(Incoming),
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
        let receiver = match epoch.peer.receiver(phase, &packet.channel) {
            Ok(receiver) => receiver.clone(),
            Err(PayloadSendError::UnknownPayload(_)) => return Ok(Classified::Other(packet)),
            Err(error) => return Err(fail(error, packet)),
        };
        let owned = packet
            .channel
            .capacity()
            .saturating_add(packet.data.capacity());
        let lease = transport.reserve_ingress(owned).map_err(|error| Failure {
            id: PayloadId(packet.channel.clone()),
            error,
        })?;
        let main = epoch.main_task();
        Ok(Classified::Application(Self {
            epoch,
            id: PayloadId(packet.channel),
            body: Body::Payload(PayloadBytes(packet.data)),
            receiver,
            main,
            lease: Arc::new(lease),
            work_lease: None,
        }))
    }
    /// Reserve before cloning the channel or retaining the guest's work resource.
    pub fn work(
        &self,
        transport: &Transport,
        work: Arc<dyn PayloadWork>,
    ) -> Result<Self, PayloadContextError> {
        if !self.epoch.active() {
            return Err(PayloadContextError::Disconnected);
        }
        let lease = transport
            .reserve_ingress(self.id.0.len())
            .map_err(|error| match error {
                PayloadSendError::Backpressure => PayloadContextError::Backpressure,
                _ => PayloadContextError::Disconnected,
            })?;
        Ok(Self {
            epoch: self.epoch.clone(),
            id: self.id.clone(),
            receiver: self.receiver.clone(),
            main: self.epoch.main_task(),
            body: Body::Work(work),
            lease: self.lease.clone(),
            work_lease: Some(lease),
        })
    }
}
