//! Read configuration while the server thread advances the task queue.
use super::super::configuration_tasks::{ConfigurationRequest, Configured, Session};
use super::*;

pub(super) fn configure(
    c: &mut ConfigIo<'_>,
    shared: &Shared,
    player: u128,
    name: &str,
    info: &mut ClientInformation,
    transport: &Arc<Transport>,
    negotiation: &mut sunrise_protocol::payload_negotiation::Negotiation,
) -> Result<Option<Configured>, String> {
    let Some(requests) = &shared.configuration_requests else {
        return Ok(None);
    };
    let connection = shared
        .next_configuration_connection()
        .ok_or("Configuration connection identifiers exhausted")?;
    let sender = c
        .task_sender(transport.clone())
        .map_err(|e| e.to_string())?;
    let session = Configured(Session::new_for_player(
        sender,
        shared.prepare_spawn.then_some(player),
    ));
    requests
        .send(ConfigurationRequest {
            connection,
            player,
            name: name.to_owned(),
            session: session.0.clone(),
        })
        .map_err(|e| e.to_string())?;
    c.set_configuration_session(Arc::downgrade(&session.0));
    let result = wait(c, info, transport, negotiation, &session.0);
    c.clear_configuration_session();
    c.reader
        .set_read_timeout(Some(READ_TIMEOUT))
        .map_err(|e| e.to_string())?;
    result
        .inspect_err(|error| {
            let reason = crate::text::Text::translate_with_fallback(
                "sunrise.configuration.error",
                crate::lang::Language::load(&[], crate::lang::DEFAULT)
                    .get("sunrise.configuration.error")
                    .unwrap(),
                &[crate::text::Text::literal(error)],
            );
            c.disconnect(super::super::data::component_nbt(&reason.component()));
        })
        .map(|()| Some(session))
}
fn wait(
    c: &mut ConfigIo<'_>,
    info: &mut ClientInformation,
    transport: &Transport,
    negotiation: &mut sunrise_protocol::payload_negotiation::Negotiation,
    session: &Session,
) -> Result<(), String> {
    // Wake to observe main-thread completion, including synchronous task starts.
    // The connection's idle/keep-alive checks still own its lifetime.
    c.reader
        .set_read_timeout(Some(Duration::from_millis(50)))
        .map_err(|e| e.to_string())?;
    let start = Instant::now();
    let mut last_received = start;
    let mut next_keep_alive = start + KEEP_ALIVE_EVERY;
    let mut outstanding = None;
    loop {
        if session.closed() {
            return Err("Configuration connection closed".into());
        }
        if let Some(result) = session.result() {
            return result;
        }
        let now = Instant::now();
        if now >= next_keep_alive {
            if outstanding.is_some() {
                return Err(sunrise_protocol::Error::timed_out().to_string());
            }
            let id = i64::try_from(start.elapsed().as_millis()).unwrap_or(0);
            c.send(&cfg::Clientbound::KeepAlive(cfg::KeepAlive { id }).encode())
                .map_err(|e| e.to_string())?;
            outstanding = Some(id);
            next_keep_alive = now + KEEP_ALIVE_EVERY;
        }
        if now.duration_since(last_received) >= READ_TIMEOUT {
            return Err(sunrise_protocol::Error::timed_out().to_string());
        }
        let (id, bytes) = match c.recv() {
            Ok(packet) => packet,
            Err(error) if error.is_timed_out() => continue,
            Err(error) => return Err(error.to_string()),
        };
        last_received = Instant::now();
        match receive_configuration(c, id, &bytes, transport, negotiation)? {
            Some(cfg::Serverbound::ClientInformation(value)) => *info = value,
            Some(cfg::Serverbound::KeepAlive(value)) if outstanding == Some(value.id) => {
                outstanding = None
            }
            Some(cfg::Serverbound::FinishConfiguration(_)) => {
                return Err("Unexpected configuration task completion".into());
            }
            _ => {}
        }
    }
}
