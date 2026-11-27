use crate::game::ClientGame;
use mod_api::{PayloadAccepted, PayloadBytes, PayloadId, PayloadSendError};

impl ClientGame<'_> {
    pub(in crate::game) fn send_payload_to_server(
        &mut self,
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<PayloadAccepted, PayloadSendError> {
        let endpoint = self
            .client
            .remote
            .as_ref()
            .ok_or(PayloadSendError::Disconnected)?
            .session
            .payload_endpoint();
        self.queue_current_payload(endpoint, id, bytes)
    }

    /// NeoForge's `hasChannel`: the decision `send_payload_to_server` makes on the session's
    /// current epoch (`Epoch::sends`, which `Epoch::can_send` shares), with no bytes and no
    /// allocation, a no included.
    pub(in crate::game) fn has_channel(&self, id: &PayloadId) -> bool {
        self.client
            .remote
            .as_ref()
            .and_then(|remote| remote.session.payload_transport().epoch())
            .is_some_and(|epoch| epoch.sends(&id.0))
    }
}

use sunrise_gameplay_common::game::payload_transport::PayloadTransportGameExt1 as _;
