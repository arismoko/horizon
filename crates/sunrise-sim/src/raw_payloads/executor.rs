//! NETWORK callbacks have no World borrow. Their replies and main work publish on success.
use super::{
    Body, Endpoint, Incoming, Prepared, Side,
    endpoint::ConfigurationTask,
    main_queue::{
        BundleQueue, ClientEvent, ClientQueue, Direct, FailedMain, HostedEvent, MainRoute,
        ServerQueue,
    },
};
use mod_api::{
    __private::game::CallbackFailure, HandlerThread, IPayloadContext, PayloadAccepted,
    PayloadBytes, PayloadContextError, PayloadDirection, PayloadId, PayloadProtocol,
    PayloadSendError, PayloadWork, PayloadWorkStatus, PlayerRef, Text,
};
use std::sync::Arc;

enum After<P> {
    Reply(Prepared),
    Work(P),
}
enum FailedPublication<Ev> {
    Reply(PayloadSendError),
    Main(FailedMain<Ev>),
}
/// The staged reply container stays small enough to live inline in a handler's turn.
fn assert_staged_fits<P>() {
    const {
        assert!(std::mem::size_of::<After<P>>() <= 256);
    }
}

/// Called by the physical receiving executor. Main events use its ordinary ordered stream.
pub fn dispatch_client<Ev: ClientEvent>(
    incoming: Incoming,
    endpoint: Endpoint,
    main: ClientQueue<Ev>,
) {
    dispatch_on(incoming, endpoint, main);
}

/// A hosted guest's receiver: its delivery keeps the physical identity main orders on.
pub fn dispatch_server<Ev: HostedEvent>(
    incoming: Incoming,
    endpoint: Endpoint,
    main: ServerQueue<Ev>,
) {
    dispatch_on(incoming, endpoint, main);
}

fn dispatch_on<Ev, R: MainRoute<Ev>>(incoming: Incoming, endpoint: Endpoint, main: R) {
    assert_staged_fits::<R::Prepared>();
    let Some(connection) = endpoint.transport().connection.as_ref() else {
        return;
    };
    // Only dedicated configuration/client receivers may park their physical stream.
    let wait_epoch = main.parks_stream().then(|| incoming.epoch.clone());
    if let Some(incoming) = connection.queue_payload(incoming) {
        execute(incoming, &endpoint, &main, None, |reason| {
            endpoint.disconnect(reason);
        });
    }
    dispatch_pending_on(&endpoint, &main);
    if let Some(epoch) = wait_epoch {
        while connection.wait_payloads(&epoch) {
            dispatch_pending_on(&endpoint, &main);
        }
    }
}

/// `ClientNetworkRegistry.handleModdedPayload` invokes NETWORK listeners on
/// the caller, which is already main inside a bundle. The bundle is itself the
/// ordered main turn; don't enqueue these roots behind work received later.
pub fn dispatch_bundle(incoming: Incoming, endpoint: &Endpoint) -> Vec<Incoming> {
    let (queue, work) = BundleQueue::new();
    // A bundle holds roots inline and names no caller event type.
    execute::<(), _>(incoming, endpoint, &queue, None, |reason| {
        endpoint.disconnect(reason);
    });
    let mut pending = work.lock().unwrap();
    std::mem::take(&mut *pending)
}

/// The pooled play receiver calls this from its existing maintenance turn.
pub fn dispatch_pending_client<Ev: ClientEvent>(endpoint: &Endpoint, main: &ClientQueue<Ev>) {
    dispatch_pending_on(endpoint, main);
}

/// The pooled play receiver calls this from its existing maintenance turn.
pub fn dispatch_pending_server<Ev: HostedEvent>(endpoint: &Endpoint, main: &ServerQueue<Ev>) {
    dispatch_pending_on(endpoint, main);
}

fn dispatch_pending_on<Ev, R: MainRoute<Ev>>(endpoint: &Endpoint, main: &R) {
    let Some(connection) = endpoint.transport().connection.as_ref() else {
        return;
    };
    if !connection.payloads_pending() {
        return;
    }
    while let Some(incoming) = connection.next_payload() {
        execute(incoming, endpoint, main, None, |reason| {
            endpoint.disconnect(reason);
        });
    }
}

fn execute<Ev, R: MainRoute<Ev>>(
    mut incoming: Incoming,
    endpoint: &Endpoint,
    main: &R,
    credit: Option<super::ingress::ReceiveLease<Ev>>,
    disconnect: impl FnOnce(Text),
) {
    if !incoming.epoch.active() {
        return;
    }
    if matches!(incoming.body, Body::Work(_)) || incoming.receiver.thread == HandlerThread::Main {
        // The callback may have panicked while this delivery waited for its ordered turn.
        if incoming.receiver.poisoned() {
            return;
        }
        if matches!(incoming.body, Body::Payload(_)) {
            // Only the dedicated client reader uses the direct path. Hosted
            // payloads go through main preparation and publication, and bundle
            // roots use that same path but stay in the current ordered main turn.
            // A closed reader leaves the old epoch to main: an already-admitted
            // bundle may still have inline payloads to run.
            match main.direct_payload(incoming) {
                Direct::Taken => return,
                Direct::Refused(super::ingress::Refusal::Pressure) => {
                    endpoint.disconnect(failure("payload main refused: Pressure"));
                    return;
                }
                Direct::Refused(super::ingress::Refusal::Closed) => return,
                Direct::DeferToMain(restored) => incoming = restored,
            }
        }
        let epoch = incoming.epoch.clone();
        let prepared = match main.prepare(incoming, credit) {
            Ok(prepared) => prepared,
            Err((error, work_failure)) => {
                if work_failure.is_none() {
                    disconnect(failure(&format!("payload main refused: {error:?}")));
                }
                return;
            }
        };
        // The value stays outside the closure if invalidated before publication.
        let mut prepared = Some(prepared);
        let result = epoch.with_active(|| main.publish(prepared.take().unwrap()));
        drop(prepared);
        if let Some(Err(error)) = result {
            let reason = error.reason();
            drop(error);
            disconnect(failure(&format!("payload main refused: {reason:?}")));
        }
        return;
    }
    let Body::Payload(bytes) = incoming.take_body() else {
        unreachable!("work is sent to main above");
    };
    let mut context = NetworkContext {
        incoming: &incoming,
        endpoint,
        main,
        after: Vec::new(),
        disconnect: None,
        finish_task: None,
        failure: None,
    };
    let result = incoming.receiver.handle(bytes, &mut context);
    match context.failure.take().map_or(result, Err) {
        Err(CallbackFailure::Refused(error)) => {
            // Drop reservations and staged work before reporting failure; publish no partial actions.
            drop(context);
            if let Some(reason) = handler_failed(&incoming, &error) {
                disconnect(reason);
            }
        }
        Err(CallbackFailure::Panicked | CallbackFailure::Poisoned) => {
            // Already reported by the shared boundary. Drop only this callback's staged work.
        }
        Ok(()) => {
            let reason = context.disconnect.take();
            let finish_task = context.finish_task.take();
            let mut after = std::mem::take(&mut context.after);
            drop(context);
            let result = incoming.epoch.with_active(|| {
                // Never consume the container here: an early error must not drop
                // remaining opaque Work captures while the epoch gate is held.
                for action in &mut after {
                    match action.take().unwrap() {
                        After::Reply(reply) => reply.publish().map_err(FailedPublication::Reply)?,
                        After::Work(work) => main.publish(work).map_err(FailedPublication::Main)?,
                    }
                }
                if reason.is_none()
                    && let Some((session, ordinal)) = finish_task.as_ref()
                {
                    session.finish_ordinal_under_epoch(*ordinal);
                }
                Ok::<(), FailedPublication<Ev>>(())
            });
            drop(after);
            drop(finish_task);
            match result {
                Some(Err(error)) => {
                    let message = match &error {
                        FailedPublication::Reply(error) => {
                            format!("payload {} reply refused: {error:?}", incoming.id.0)
                        }
                        FailedPublication::Main(error) => {
                            format!("payload main refused: {:?}", error.reason())
                        }
                    };
                    drop(error);
                    disconnect(failure(&message));
                }
                Some(Ok(())) => {
                    if let Some(reason) = reason {
                        disconnect(reason);
                    }
                }
                // Phase/connection cancellation cancels the staged disconnect too.
                None => {}
            }
        }
    }
}
fn handler_failed(incoming: &Incoming, error: &str) -> Option<Text> {
    if incoming.raw {
        log::warn!(
            "Mod {} failed on raw channel {}: {error}",
            incoming.receiver.owner(),
            incoming.id.0
        );
        None
    } else {
        log::warn!("payload {} failed: {error}", incoming.id.0);
        Some(failure(&format!(
            "a mod's network handler failed ({})",
            mod_of(&incoming.id)
        )))
    }
}
/// The mod a channel belongs to: its namespace.
pub fn mod_of(id: &PayloadId) -> &str {
    id.0.split_once(':')
        .map_or(&*id.0, |(namespace, _)| namespace)
}
fn failure(message: &str) -> Text {
    Text::translate("disconnect.genericReason", vec![Text::literal(message)])
}
struct NetworkContext<'a, Ev, R: MainRoute<Ev>> {
    incoming: &'a Incoming,
    endpoint: &'a Endpoint,
    main: &'a R,
    after: Vec<Option<After<R::Prepared>>>,
    disconnect: Option<Text>,
    finish_task: Option<(Arc<dyn ConfigurationTask>, usize)>,
    failure: Option<CallbackFailure>,
}
impl<Ev, R: MainRoute<Ev>> IPayloadContext for NetworkContext<'_, Ev, R> {
    fn player(&self) -> Result<PlayerRef, PayloadContextError> {
        if !self.incoming.epoch.active() || self.disconnect.is_some() {
            return Err(PayloadContextError::Disconnected);
        }
        if self.incoming.epoch.phase == sunrise_protocol::payload_negotiation::Phase::Configuration
        {
            return Err(PayloadContextError::NoPlayer);
        }
        self.endpoint
            .transport()
            .player()
            .ok_or(PayloadContextError::NoPlayer)
    }
    fn flow(&self) -> PayloadDirection {
        match self.endpoint.side() {
            Side::Client => PayloadDirection::ToClient,
            Side::Server => PayloadDirection::ToServer,
        }
    }
    fn protocol(&self) -> PayloadProtocol {
        match self.incoming.epoch.phase {
            sunrise_protocol::payload_negotiation::Phase::Configuration => {
                PayloadProtocol::Configuration
            }
            sunrise_protocol::payload_negotiation::Phase::Play => PayloadProtocol::Play,
        }
    }
    fn server_host(&mut self) -> Result<&mut dyn mod_api::ServerHost, PayloadContextError> {
        if !self.incoming.epoch.active() || self.disconnect.is_some() {
            return Err(PayloadContextError::Disconnected);
        }
        if self.endpoint.side() != Side::Server {
            return Err(PayloadContextError::WrongSide);
        }
        Err(PayloadContextError::WrongThread)
    }
    fn client_host(&mut self) -> Result<&mut dyn mod_api::ClientHost, PayloadContextError> {
        if !self.incoming.epoch.active() || self.disconnect.is_some() {
            return Err(PayloadContextError::Disconnected);
        }
        if self.endpoint.side() != Side::Client {
            return Err(PayloadContextError::WrongSide);
        }
        Err(PayloadContextError::WrongThread)
    }
    fn reply(
        &mut self,
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<PayloadAccepted, PayloadSendError> {
        if self.disconnect.is_some() {
            return Err(PayloadSendError::Disconnected);
        }
        let reply = self.endpoint.prepare(&self.incoming.epoch, id, bytes)?;
        self.after.push(Some(After::Reply(reply)));
        #[cfg(any(test, feature = "test-payload-probe"))]
        super::staging_probe::prepared(&self.incoming.id.0, false);
        Ok(PayloadAccepted::Queued)
    }
    fn check_payload_token(&self, token: &mod_api::PayloadToken) -> Result<(), PayloadSendError> {
        mod_api::__private::game::check_payload_token(token)
    }
    /// Published at once, not staged with the handler's replies: a droppable frame is worthless
    /// late, and a relay must not hold it until the handler returns.
    fn send_to(
        &mut self,
        players: &[PlayerRef],
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<u32, PayloadSendError> {
        if self.disconnect.is_some() || self.failure.is_some() || !self.incoming.epoch.active() {
            return Err(PayloadSendError::Disconnected);
        }
        if self.endpoint.side() != Side::Server {
            return Err(PayloadSendError::WrongDirection(id.clone()));
        }
        self.endpoint.transport().local.send_to(players, id, bytes)
    }
    fn finish_configuration_task(
        &mut self,
        task_type: &mod_api::Identifier,
    ) -> Result<(), mod_api::ConfigurationTaskError> {
        if self.endpoint.side() != Side::Server
            || self.incoming.epoch.phase
                != sunrise_protocol::payload_negotiation::Phase::Configuration
        {
            return Err(mod_api::ConfigurationTaskError::WrongContext);
        }
        if self.disconnect.is_some() || !self.incoming.epoch.active() {
            return Err(mod_api::ConfigurationTaskError::Disconnected);
        }
        let session = match self.endpoint.configuration_session() {
            Some(weak) => weak
                .upgrade()
                .ok_or(mod_api::ConfigurationTaskError::Disconnected)?,
            None => return Err(mod_api::ConfigurationTaskError::NotRunning),
        };
        if self
            .endpoint
            .transport()
            .epoch()
            .is_none_or(|epoch| !Arc::ptr_eq(&epoch, &self.incoming.epoch))
        {
            return Err(mod_api::ConfigurationTaskError::Disconnected);
        }
        if self.finish_task.is_some() {
            return Err(mod_api::ConfigurationTaskError::NotRunning);
        }
        let ordinal = session.running_ordinal_for_type(task_type)?;
        self.finish_task = Some((session, ordinal));
        Ok(())
    }
    fn disconnect(&mut self, reason: Text) {
        self.disconnect = Some(reason);
    }
    fn enqueue_work(
        &mut self,
        work: Arc<dyn PayloadWork>,
    ) -> Result<PayloadWorkStatus, PayloadContextError> {
        if self.disconnect.is_some() || self.failure.is_some() {
            if let Err(error) = self.incoming.receiver.dispose_work(work) {
                self.failure = Some(error);
            }
            return Err(PayloadContextError::Disconnected);
        }
        let prepared = self
            .incoming
            .work(self.endpoint.transport(), work)
            .and_then(|work| self.main.prepare(work, None))
            .map_err(|(error, work_failure)| {
                self.failure = work_failure;
                error
            })?;
        self.after.push(Some(After::Work(prepared)));
        #[cfg(any(test, feature = "test-payload-probe"))]
        super::staging_probe::prepared(&self.incoming.id.0, true);
        Ok(PayloadWorkStatus::Queued)
    }
}
