//! Configuration transport has no player. Its replies share the engine writer queue.
use crate::raw_payloads::{
    Incoming, Transport,
    queued_writer::{PreparedPayload, QueuedWriter},
};
use mod_api::{PayloadId, PayloadSendError};
use std::sync::Arc;
use sunrise_protocol::{
    connection::Connection, packets::configuration as cfg, payload_negotiation::Phase,
};

#[derive(Clone)]
pub struct ConfigSender {
    writer: QueuedWriter,
    transport: Arc<Transport>,
}
impl ConfigSender {
    pub fn framing(&self) -> sunrise_protocol::connection::Framing {
        self.writer.framing()
    }
    pub(crate) fn payload_capacity(&self) -> usize {
        self.writer.payload_capacity()
    }
    pub fn payload_transport(&self) -> &Arc<Transport> {
        &self.transport
    }
    pub fn prepare_payload(
        &self,
        body: Vec<u8>,
        id: &PayloadId,
    ) -> Result<PreparedPayload, PayloadSendError> {
        self.writer.prepare_payload(body, id)
    }
    pub(super) fn send_packet(&self, body: &[u8]) -> sunrise_protocol::Result<()> {
        self.writer.send(body)
    }
    pub fn disconnect(&self, reason: sunrise_core::nbt::Tag) {
        self.transport.close();
        self.writer
            .disconnect(cfg::Clientbound::Disconnect(cfg::Disconnect { reason }).encode());
    }
    #[cfg(test)]
    pub(crate) fn test_sender(transport: Arc<Transport>) -> (Self, Connection) {
        let (peer, server) = sunrise_protocol::connection::Pipe::pair();
        let mut server = Connection::local(server);
        let peer = Connection::local(peer);
        let writer = QueuedWriter::new(server.writer.take()).unwrap();
        (Self { writer, transport }, peer)
    }
}

/// The engine's channels that every configuration loop reads itself, kept into play.
#[derive(Default)]
pub(super) struct EngineChannels {
    /// What the client announced in `minecraft:register`. Allocated on the first
    /// `minecraft:register` / `minecraft:unregister`: most connections never send one.
    pub announced: Option<Arc<super::announced_channels::AnnouncedChannels>>,
    /// Its `sunrise:levels` answer.
    pub viewed_levels: super::viewed_levels::capability::Capability,
    /// Its `sunrise:menu_extra` revision answer.
    pub menu_extra_rev: super::menu_extra_rev::Capability,
}
impl EngineChannels {
    /// Whether `payload` was one of these channels' and is taken, or the reason that closes the
    /// connection.
    pub fn receive(&mut self, payload: &cfg::CustomPayload) -> Result<bool, String> {
        // ServerCommonPacketListenerImpl hands the lists to the plugin messenger, in
        // configuration as in play: they stay on the connection.
        if super::announced_channels::is_list_channel(&payload.channel) {
            self.announced
                .get_or_insert_with(Arc::default)
                .apply(&payload.channel, &payload.data);
            return Ok(true);
        }
        if payload.channel == sunrise_protocol::viewed_levels::CHANNEL {
            self.viewed_levels
                .receive(&payload.data)
                .map_err(|error| error.to_string())?;
            return Ok(true);
        }
        if payload.channel == sunrise_protocol::menu_extra::CHANNEL {
            self.menu_extra_rev
                .receive(&payload.data)
                .map_err(|error| error.to_string())?;
            return Ok(true);
        }
        Ok(false)
    }
}

/// One captured configuration outbox: the reservation keeps its prepaid body credit.
struct ConfigAdmission {
    sender: ConfigSender,
    framing: sunrise_protocol::connection::Framing,
    capacity: usize,
}
impl crate::raw_payloads::Admission for ConfigAdmission {
    fn framing(&self) -> sunrise_protocol::connection::Framing {
        self.framing
    }
    fn capacity(&self) -> usize {
        self.capacity
    }
    fn accepts_packets(&self) -> bool {
        true
    }
    fn prepare(
        self: Box<Self>,
        body: Vec<u8>,
        id: &PayloadId,
    ) -> Result<Box<dyn crate::raw_payloads::PreparedSend>, PayloadSendError> {
        ConfigSender::prepare_payload(&self.sender, body, id)
            .map(|prepared| Box::new(prepared) as Box<dyn crate::raw_payloads::PreparedSend>)
    }
}
impl crate::raw_payloads::Output for ConfigSender {
    fn transport(&self) -> &Arc<Transport> {
        &self.transport
    }
    fn side(&self) -> crate::raw_payloads::Side {
        crate::raw_payloads::Side::Server
    }
    fn admit(
        &self,
        _: Phase,
        _: &PayloadId,
    ) -> Result<Box<dyn crate::raw_payloads::Admission>, PayloadSendError> {
        Ok(Box::new(ConfigAdmission {
            sender: self.clone(),
            framing: self.framing(),
            capacity: self.payload_capacity(),
        }))
    }
    fn disconnect(&self, reason: &crate::text::Text) {
        ConfigSender::disconnect(self, crate::lan::data::component_nbt(&reason.component()));
    }
    #[cfg(any(test, feature = "test-payload-probe"))]
    fn diagnostics(&self) -> Box<dyn Fn() -> String + Send + Sync> {
        Box::new(|| "configuration sender not observed".into())
    }
}

pub(super) struct ConfigIo<'a> {
    connection: &'a mut Connection,
    pub sender: Option<ConfigSender>,
    events: crate::raw_payloads::ingress::Producer<super::NetEvent>,
    handed_off: bool,
    pub channels: EngineChannels,
    configuration_session: Option<std::sync::Weak<super::configuration_tasks::Session>>,
    /// One output for this configuration connection, shared by its deliveries.
    payload_output: Option<Arc<dyn crate::raw_payloads::Output>>,
}
impl<'a> ConfigIo<'a> {
    pub fn new(
        connection: &'a mut Connection,
        transport: Arc<Transport>,
        events: crate::raw_payloads::ingress::Producer<super::NetEvent>,
    ) -> sunrise_protocol::Result<Self> {
        let sender = if transport.local.manifest.entries().is_empty() {
            None
        } else {
            Some(ConfigSender {
                writer: QueuedWriter::new(connection.writer.take())?,
                transport,
            })
        };
        Ok(Self {
            connection,
            sender,
            events,
            handed_off: false,
            channels: EngineChannels::default(),
            configuration_session: None,
            payload_output: None,
        })
    }
    pub(super) fn set_configuration_session(
        &mut self,
        session: std::sync::Weak<super::configuration_tasks::Session>,
    ) {
        self.configuration_session = Some(session);
    }
    pub(super) fn clear_configuration_session(&mut self) {
        self.configuration_session = None;
    }
    pub fn send(&mut self, body: &[u8]) -> sunrise_protocol::Result<()> {
        match &self.sender {
            Some(sender) => sender.writer.send(body),
            None => self.connection.send(body),
        }
    }
    /// Give configuration tasks the same writer used by application payload replies.
    pub(super) fn task_sender(
        &mut self,
        transport: Arc<Transport>,
    ) -> sunrise_protocol::Result<ConfigSender> {
        if self.sender.is_none() {
            self.sender = Some(ConfigSender {
                writer: QueuedWriter::new(self.connection.writer.take())?,
                transport,
            });
            self.payload_output = None;
        }
        Ok(self.sender.as_ref().unwrap().clone())
    }
    /// Asks a Sunrise-branded client how many levels besides its own it can view.
    pub fn ask_viewed_levels(&mut self) -> sunrise_protocol::Result<()> {
        let query = self.channels.viewed_levels.query();
        self.send(&cfg::Clientbound::CustomPayload(query).encode())
    }
    /// Asks a Sunrise-branded client which `sunrise:menu_extra` layout it reads.
    pub fn ask_menu_extra_rev(&mut self) -> sunrise_protocol::Result<()> {
        let query = self.channels.menu_extra_rev.query();
        self.send(&cfg::Clientbound::CustomPayload(query).encode())
    }
    pub fn application(
        &mut self,
        payload: cfg::CustomPayload,
    ) -> Result<Option<cfg::CustomPayload>, String> {
        match self.channels.receive(&payload) {
            Ok(true) => return Ok(None),
            Ok(false) => {}
            Err(error) => {
                self.disconnect(sunrise_core::nbt::Tag::String(error.as_str().into()));
                return Err(error);
            }
        }
        let Some(sender) = self.sender.clone() else {
            return Ok(Some(payload));
        };
        match Incoming::classify(&sender.transport, Phase::Configuration, payload)
            .map_err(|error| error.to_string())?
        {
            crate::raw_payloads::Classified::Other(payload) => Ok(Some(payload)),
            crate::raw_payloads::Classified::Skipped | crate::raw_payloads::Classified::Raw(_) => {
                Ok(None)
            }
            crate::raw_payloads::Classified::Application(incoming) => {
                let session = self.configuration_session.clone().map(|session| {
                    session as std::sync::Weak<dyn crate::raw_payloads::ConfigurationTask>
                });
                let output = self
                    .payload_output
                    .get_or_insert_with(|| {
                        Arc::new(sender.clone()) as Arc<dyn crate::raw_payloads::Output>
                    })
                    .clone();
                let endpoint = crate::raw_payloads::Endpoint::shared_configuration(output, session);
                crate::raw_payloads::dispatch_server(
                    incoming,
                    endpoint.clone(),
                    crate::raw_payloads::ServerQueue::new(self.events.clone(), endpoint, None),
                );
                Ok(None)
            }
        }
    }
    pub fn disconnect(&mut self, reason: sunrise_core::nbt::Tag) {
        match &self.sender {
            Some(sender) => sender.disconnect(reason),
            None => {
                let _ = self
                    .connection
                    .send(&cfg::Clientbound::Disconnect(cfg::Disconnect { reason }).encode());
            }
        }
    }
    pub fn finish(&mut self) -> Result<(), String> {
        if let Some(sender) = &self.sender {
            self.connection.writer = sender.writer.drain_and_take().map_err(|e| e.to_string())?;
        }
        self.handed_off = true;
        Ok(())
    }
}
impl std::ops::Deref for ConfigIo<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.connection
    }
}
impl std::ops::DerefMut for ConfigIo<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        self.connection
    }
}
impl Drop for ConfigIo<'_> {
    fn drop(&mut self) {
        if !self.handed_off
            && let Some(sender) = &self.sender
        {
            sender.transport.close();
            if !sender.writer.is_terminal() {
                sender.writer.shutdown();
            }
        }
    }
}
