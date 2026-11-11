//! Tasks declared by a server mod during connection configuration.

use crate::Identifier;

/// A vanilla configuration-phase custom payload on a mod's plugin channel.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct ConfigurationPayload {
    /// The plugin channel. The `minecraft` and `horizon` namespaces are reserved.
    pub channel: Identifier,
    /// The encoded bytes following the channel identifier.
    pub data: Vec<u8>,
}

impl ConfigurationPayload {
    /// Create a payload for a plugin channel the peer understands.
    pub fn new(channel: Identifier, data: Vec<u8>) -> Self {
        Self { channel, data }
    }
}

/// One task in a joining connection's configuration queue.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct ConfigurationTask {
    /// A unique task identifier within the connection.
    pub id: Identifier,
    /// Payloads to send, in order, when the task starts.
    pub payloads: Vec<ConfigurationPayload>,
    /// An exact serverbound plugin message to await after sending the payloads.
    pub reply: Option<ConfigurationPayload>,
}

impl ConfigurationTask {
    /// Create a task that completes as soon as its payloads have been sent.
    pub fn new(id: Identifier, payloads: Vec<ConfigurationPayload>) -> Self {
        Self {
            id,
            payloads,
            reply: None,
        }
    }

    /// Require the peer to acknowledge the task before the next task starts.
    pub fn awaiting(mut self, reply: ConfigurationPayload) -> Self {
        self.reply = Some(reply);
        self
    }
}
