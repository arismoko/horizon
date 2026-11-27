//! A receiving main provider reserves capacity before enqueue_work reports Queued.
use super::{
    Endpoint, Incoming,
    ingress::{Prepared as PreparedEvent, Producer, ReceiveLease, Refusal, Refused},
};
use mod_api::{__private::game::CallbackFailure, PayloadContextError};

/// The client's ordered main union: a bare admitted payload becomes this event.
pub trait ClientEvent: Sized {
    fn raw_payload(incoming: Incoming) -> Self;
    fn into_raw_payload(self) -> Incoming;
}

/// A hosted guest's ordered main union: each delivery keeps its physical
/// identity and reply target, so a client union never names a guest turn.
pub trait HostedEvent: Sized {
    fn delivery(conn: Option<u32>, endpoint: Endpoint, incoming: Incoming) -> Self;
    fn into_incoming(self) -> Incoming;
}

type BundleWork = std::sync::Arc<std::sync::Mutex<Vec<Incoming>>>;

/// One physical client session's ordered stream.
pub struct ClientQueue<Ev> {
    events: Producer<Ev>,
}
impl<Ev> ClientQueue<Ev> {
    pub fn new(events: Producer<Ev>) -> Self {
        Self { events }
    }
}

/// One hosted guest's ordered stream, with the connection and endpoint its
/// deliveries carry to main.
pub struct ServerQueue<Ev> {
    pub events: Producer<Ev>,
    pub endpoint: Endpoint,
    pub conn: Option<u32>,
}
impl<Ev> ServerQueue<Ev> {
    pub fn new(events: Producer<Ev>, endpoint: Endpoint, conn: Option<u32>) -> Self {
        Self {
            events,
            endpoint,
            conn,
        }
    }
}

/// What a client reader does with a wire record it has already admitted.
pub(crate) enum Direct {
    /// The ordered stream took it.
    Taken,
    /// The stream refused it; only pressure ends the session.
    Refused(Refusal),
    /// This turn has no direct stream (bundle or hosted guest).
    DeferToMain(Incoming),
}

/// A failed publication keeps the refused event alive until after the caller
/// releases its epoch gate. Boxing happens only on failure.
pub(crate) struct FailedMain<Ev>(Box<Refused<Ev>>);
impl<Ev> FailedMain<Ev> {
    pub fn reason(&self) -> Refusal {
        self.0.reason
    }
}

/// The ordered turn one executor publishes staged work into. The client and
/// hosted queues implement it with their own event constructors, so neither
/// side can name the other's turn.
pub(crate) trait MainRoute<Ev> {
    /// This route's staged work: a prepared event, or a bundle root.
    type Prepared;
    fn prepare(
        &self,
        incoming: Incoming,
        credit: Option<ReceiveLease<Ev>>,
    ) -> Result<Self::Prepared, (PayloadContextError, Option<CallbackFailure>)>;
    fn publish(&self, prepared: Self::Prepared) -> Result<(), FailedMain<Ev>>;
    /// The client reader's fast path for a wire record that is not work.
    fn direct_payload(&self, incoming: Incoming) -> Direct {
        Direct::DeferToMain(incoming)
    }
    /// Only dedicated configuration/client receivers park their physical stream;
    /// a hosted play receiver continues in its own maintenance turn.
    fn parks_stream(&self) -> bool {
        true
    }
}

fn refusal(reason: Refusal) -> PayloadContextError {
    match reason {
        Refusal::Pressure => PayloadContextError::Backpressure,
        Refusal::Closed => PayloadContextError::Disconnected,
    }
}

impl<Ev> MainRoute<Ev> for ClientQueue<Ev>
where
    Ev: ClientEvent,
{
    type Prepared = PreparedEvent<Ev>;
    fn prepare(
        &self,
        incoming: Incoming,
        credit: Option<ReceiveLease<Ev>>,
    ) -> Result<Self::Prepared, (PayloadContextError, Option<CallbackFailure>)> {
        let event = Ev::raw_payload(incoming);
        match credit {
            Some(credit) => self.events.prepare_replacing(event, credit),
            None => self.events.prepare(event),
        }
        .map_err(|error| {
            let mut incoming = error.value.into_raw_payload();
            (refusal(error.reason), incoming.dispose_body().err())
        })
    }
    fn publish(&self, prepared: Self::Prepared) -> Result<(), FailedMain<Ev>> {
        prepared
            .publish()
            .map_err(|error| FailedMain(Box::new(error)))
    }
    fn direct_payload(&self, incoming: Incoming) -> Direct {
        match self.events.send_wait(Ev::raw_payload(incoming)) {
            Ok(()) => Direct::Taken,
            Err(error) => Direct::Refused(error.reason),
        }
    }
}

/// The bundle turn is itself an ordered main turn: it holds roots inline.
pub(crate) struct BundleQueue(BundleWork);
impl BundleQueue {
    pub(crate) fn new() -> (Self, BundleWork) {
        let work = BundleWork::default();
        (Self(work.clone()), work)
    }
}
impl<Ev> MainRoute<Ev> for BundleQueue {
    type Prepared = Box<(BundleWork, Incoming)>;
    fn parks_stream(&self) -> bool {
        false
    }
    fn prepare(
        &self,
        incoming: Incoming,
        _credit: Option<ReceiveLease<Ev>>,
    ) -> Result<Self::Prepared, (PayloadContextError, Option<CallbackFailure>)> {
        // Incoming (including enqueued work) already holds its payload
        // lease. This root stays inside the one charged bundle turn.
        Ok(Box::new((self.0.clone(), incoming)))
    }
    fn publish(&self, prepared: Self::Prepared) -> Result<(), FailedMain<Ev>> {
        let (pending, incoming) = *prepared;
        pending.lock().unwrap().push(incoming);
        Ok(())
    }
}

impl<Ev> MainRoute<Ev> for ServerQueue<Ev>
where
    Ev: HostedEvent,
{
    type Prepared = PreparedEvent<Ev>;
    fn parks_stream(&self) -> bool {
        self.conn.is_none()
    }
    fn prepare(
        &self,
        incoming: Incoming,
        _credit: Option<ReceiveLease<Ev>>,
    ) -> Result<Self::Prepared, (PayloadContextError, Option<CallbackFailure>)> {
        self.events
            .prepare(Ev::delivery(self.conn, self.endpoint.clone(), incoming))
            .map_err(|error| {
                let mut incoming = error.value.into_incoming();
                (refusal(error.reason), incoming.dispose_body().err())
            })
    }
    fn publish(&self, prepared: Self::Prepared) -> Result<(), FailedMain<Ev>> {
        prepared
            .publish()
            .map_err(|error| FailedMain(Box::new(error)))
    }
}
