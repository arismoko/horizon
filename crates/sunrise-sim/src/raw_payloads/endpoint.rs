//! Physical reply targets usable by either receiving executor without borrowing Game.
//!
//! The endpoint knows the raw-payload rules (authority, size limits, droppable
//! queues, framing preflight) but nothing about who owns the other end: a solo
//! client session and a hosted guest both implement [`Output`].
use super::{Side, Transport, connection::Epoch, encode, encoded_len, pairing::PairingPayload};
use mod_api::{PayloadBytes, PayloadFrameStage, PayloadId, PayloadSendError, Text};
use std::sync::{Arc, Weak};
use sunrise_protocol::{connection::Framing, frame, payload_negotiation::Phase};

/// The write side of one physical connection: where a prepared reply goes.
pub trait Output: Send + Sync {
    fn transport(&self) -> &Arc<Transport>;
    fn side(&self) -> Side;
    /// Capture the connection's current writer once: preflight and the
    /// reservation then act on that same writer. Only a connection with no
    /// writer installed refuses here; a stopped writer answers its captured
    /// limits and its reservation decides whether it still takes the body.
    fn admit(&self, phase: Phase, id: &PayloadId) -> Result<Box<dyn Admission>, PayloadSendError>;
    /// Close this physical connection for `reason`.
    fn disconnect(&self, reason: &crate::text::Text);
    /// Per-endpoint diagnostics for the payload probe, never retaining the owner.
    #[cfg(any(test, feature = "test-payload-probe"))]
    fn diagnostics(&self) -> Box<dyn Fn() -> String + Send + Sync>;
}

/// One captured write side: its frame limits and the reservation that publishes
/// the body. Capturing it once preserves the reliable preflight order.
pub trait Admission: Send {
    fn framing(&self) -> Framing;
    fn capacity(&self) -> usize;
    /// A writer that stopped accepting packets still answers its captured limits.
    /// Reliable sends leave that verdict to the reservation; droppable sends
    /// refuse it before preflight.
    fn accepts_packets(&self) -> bool;
    /// Reserve an encoded packet body; the result publishes it after the handler commits.
    fn prepare(
        self: Box<Self>,
        body: Vec<u8>,
        id: &PayloadId,
    ) -> Result<Box<dyn PreparedSend>, PayloadSendError>;
}

/// A reserved outgoing frame: publishing it is the only thing a reply can do.
pub trait PreparedSend: Send {
    fn publish(self: Box<Self>) -> Result<(), PayloadSendError>;
}

/// A hosted guest's output: it can also frame a body into an entity pairing
/// bundle. Only a play connection that belongs to a guest implements it.
pub trait HostedOutput: Output {
    fn admit_hosted(
        &self,
        phase: Phase,
        id: &PayloadId,
    ) -> Result<Box<dyn HostedAdmission>, PayloadSendError>;
}

/// A hosted admission, whose reservation can become a pairing body.
pub trait HostedAdmission: Admission {
    fn prepare_hosted(
        self: Box<Self>,
        body: Vec<u8>,
        id: &PayloadId,
    ) -> Result<Box<dyn HostedPreparedSend>, PayloadSendError>;
}

/// A hosted reservation that can hand its bytes and prepaid credit to a pairing bundle.
pub trait HostedPreparedSend: PreparedSend {
    fn into_pairing(self: Box<Self>, epoch: Arc<Epoch>) -> Arc<PairingPayload>;
}

/// The configuration task a handler finishes, named without its host type.
pub trait ConfigurationTask: Send + Sync {
    fn running_ordinal_for_type(
        &self,
        task_type: &mod_api::Identifier,
    ) -> Result<usize, mod_api::ConfigurationTaskError>;
    fn finish_ordinal_under_epoch(&self, ordinal: usize);
    fn finish_type(
        &self,
        task_type: &mod_api::Identifier,
    ) -> Result<(), mod_api::ConfigurationTaskError>;
}

#[derive(Clone)]
pub struct Endpoint {
    output: Arc<dyn Output>,
    session: Option<Weak<dyn ConfigurationTask>>,
}
/// Retains no endpoint, sender, producer, writer, transport or payload lease.
#[cfg(any(test, feature = "test-payload-probe"))]
pub(super) struct Observation {
    transport: std::sync::Weak<Transport>,
    side: Side,
    diagnostics: Box<dyn Fn() -> String + Send + Sync>,
}
#[cfg(any(test, feature = "test-payload-probe"))]
impl Observation {
    pub(super) fn same_target(&self, endpoint: &Endpoint) -> bool {
        self.transport.ptr_eq(&Arc::downgrade(endpoint.transport())) && self.side == endpoint.side()
    }
    pub(super) fn diagnostics(&self) -> String {
        let connection = match self.transport.upgrade() {
            Some(transport) => transport
                .connection
                .as_ref()
                .map(|c| c.diagnostics())
                .unwrap_or_else(|| "no connection".into()),
            None => "transport owner dropped".into(),
        };
        format!("{:?}: {connection}; {}", self.side, (self.diagnostics)())
    }
}
pub enum Prepared {
    /// A reply a physical output reserved.
    Frame(Box<dyn PreparedSend>),
    /// A `Droppable` send: it reserves nothing, and publication queues it for the writer.
    Droppable(
        Arc<super::droppable::Queue>,
        Arc<mod_api::__private::game::DropCounters>,
        Vec<u8>,
    ),
}
impl Prepared {
    pub fn publish(self) -> Result<(), PayloadSendError> {
        match self {
            Self::Frame(send) => send.publish(),
            Self::Droppable(queue, counters, body) => {
                queue.push(std::time::Instant::now(), &counters, body);
                Ok(())
            }
        }
    }
}

/// A hosted guest's endpoint: it alone can prepare a body for a pairing bundle,
/// so the wrong connection cannot compile its way into one.
pub struct HostedEndpoint(Arc<dyn HostedOutput>);
impl HostedEndpoint {
    pub fn new(output: impl HostedOutput + 'static) -> Self {
        Self(Arc::new(output))
    }
    pub fn shared(output: Arc<dyn HostedOutput>) -> Self {
        Self(output)
    }
    pub fn transport(&self) -> &Arc<Transport> {
        self.0.transport()
    }
    /// Reserve a reply and keep its prepaid credit inside the pairing body.
    pub fn prepare_pairing(
        &self,
        epoch: &Arc<Epoch>,
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<Arc<PairingPayload>, PayloadSendError> {
        #[cfg(any(test, feature = "test-payload-probe"))]
        super::staging_probe::capture(&id.0, &self.endpoint());
        epoch.can_send(id)?;
        let limit = sunrise_protocol::packets::custom_payload::CLIENTBOUND_LIMIT;
        if epoch.peer.local.is_raw(&id.0) && bytes.0.len() > limit {
            return Err(PayloadSendError::PayloadTooLarge {
                id: id.clone(),
                size: bytes.0.len() as u64,
                limit: limit as u64,
            });
        }
        let len = encoded_len(epoch.phase, self.0.side(), id, bytes.0.len());
        let admission = self.0.admit_hosted(epoch.phase, id)?;
        preflight(admission.framing(), admission.capacity(), len, id)?;
        let body = encode(epoch.phase, self.0.side(), id, bytes);
        Ok(admission
            .prepare_hosted(body, id)?
            .into_pairing(epoch.clone()))
    }
    /// The shared endpoint a delivery carries.
    pub fn endpoint(&self) -> Endpoint {
        Endpoint::shared(self.0.clone())
    }
}
impl Endpoint {
    /// Wrap one write side. A connection builds its output once and hands it to
    /// [`Endpoint::shared`] for every delivery, so a delivery allocates nothing.
    pub fn new(output: impl Output + 'static) -> Self {
        Self::shared(Arc::new(output))
    }
    /// Wrap an output the physical connection already owns and shares.
    pub fn shared(output: Arc<dyn Output>) -> Self {
        Self {
            output,
            session: None,
        }
    }
    /// A hosted guest's play connection.
    pub fn server(output: impl Output + 'static) -> Self {
        Self::new(output)
    }
    /// A hosted guest's shared play connection.
    pub fn shared_server(output: Arc<dyn Output>) -> Self {
        Self::shared(output)
    }
    /// A hosted guest's configuration connection, with the task session it finishes.
    pub fn configuration(
        output: impl Output + 'static,
        session: Option<Weak<dyn ConfigurationTask>>,
    ) -> Self {
        Self::shared_configuration(Arc::new(output), session)
    }
    /// A hosted guest's configuration connection over an output it already owns.
    pub fn shared_configuration(
        output: Arc<dyn Output>,
        session: Option<Weak<dyn ConfigurationTask>>,
    ) -> Self {
        Self { output, session }
    }
    pub fn configuration_session(&self) -> Option<&Weak<dyn ConfigurationTask>> {
        self.session.as_ref()
    }
    pub fn side(&self) -> Side {
        self.output.side()
    }
    pub fn transport(&self) -> &Arc<Transport> {
        self.output.transport()
    }
    #[cfg(any(test, feature = "test-payload-probe"))]
    pub(super) fn observation(&self) -> Observation {
        Observation {
            transport: Arc::downgrade(self.transport()),
            side: self.side(),
            diagnostics: self.output.diagnostics(),
        }
    }
    /// Peer authority is checked before bytes are copied. Publication belongs to the root.
    pub fn prepare(
        &self,
        epoch: &Arc<Epoch>,
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<Prepared, PayloadSendError> {
        #[cfg(any(test, feature = "test-payload-probe"))]
        super::staging_probe::capture(&id.0, self);
        epoch.can_send(id)?;
        // A raw channel keeps vanilla's clientbound custom-payload limit, by name.
        let limit = sunrise_protocol::packets::custom_payload::CLIENTBOUND_LIMIT;
        if epoch.peer.local.is_raw(&id.0) && bytes.0.len() > limit {
            return Err(PayloadSendError::PayloadTooLarge {
                id: id.clone(),
                size: bytes.0.len() as u64,
                limit: limit as u64,
            });
        }
        let len = encoded_len(epoch.phase, self.side(), id, bytes.0.len());
        if epoch.peer.local.has_droppable()
            && let Some(counters) = epoch.peer.local.droppable(&id.0)
        {
            return self.prepare_droppable(epoch, id, bytes, len, counters);
        }
        let admission = self.output.admit(epoch.phase, id)?;
        preflight(admission.framing(), admission.capacity(), len, id)?;
        let body = encode(epoch.phase, self.side(), id, bytes);
        admission.prepare(body, id).map(Prepared::Frame)
    }
    /// A `Droppable` send takes no reservation: it is bounded by its own queue. Frame limits
    /// still refuse here, so no waiting send can fail framing later.
    fn prepare_droppable(
        &self,
        epoch: &Arc<Epoch>,
        id: &PayloadId,
        bytes: PayloadBytes,
        len: usize,
        counters: &Arc<mod_api::__private::game::DropCounters>,
    ) -> Result<Prepared, PayloadSendError> {
        let admission = self.output.admit(epoch.phase, id)?;
        if !admission.accepts_packets() {
            return Err(PayloadSendError::Disconnected);
        }
        preflight(admission.framing(), usize::MAX, len, id)?;
        let queue = self
            .transport()
            .connection
            .as_ref()
            .and_then(|connection| connection.droppable())
            .ok_or(PayloadSendError::Disconnected)?;
        Ok(Prepared::Droppable(
            queue.clone(),
            counters.clone(),
            encode(epoch.phase, self.side(), id, bytes),
        ))
    }
    /// A failure closes this physical connection only; a client reason joins its ordered events.
    pub fn disconnect(&self, reason: Text) {
        let reason = crate::api_text::to_serialized_engine(&reason).unwrap_or_else(|error| {
            crate::text::Text::tr(
                "disconnect.genericReason",
                &[&format!("invalid payload disconnect reason: {error:?}")],
            )
        });
        self.transport().close();
        self.output.disconnect(&reason);
    }
}
fn preflight(
    framing: Framing,
    capacity: usize,
    len: usize,
    id: &PayloadId,
) -> Result<(), PayloadSendError> {
    let frame_limit = match framing {
        Framing::Memory => None,
        Framing::Network(None) => Some((PayloadFrameStage::Encoded, frame::MAX_FRAME)),
        Framing::Network(Some(_)) => {
            Some((PayloadFrameStage::Uncompressed, frame::MAX_UNCOMPRESSED))
        }
    };
    if let Some((stage, limit)) = frame_limit
        && len > limit
    {
        return Err(PayloadSendError::FrameTooLarge {
            id: id.clone(),
            stage,
            size: len as u64,
            limit: limit as u64,
        });
    }
    if len > capacity {
        return Err(PayloadSendError::Backpressure);
    }
    Ok(())
}
