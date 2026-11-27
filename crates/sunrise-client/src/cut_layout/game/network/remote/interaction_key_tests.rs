#[cfg(feature = "client")]
use super::*;
#[cfg(feature = "client")]
use mod_api::{ClientHost, InteractionKeyMappingTriggeredEvent};

#[cfg(feature = "client")]
pub(crate) struct KeyControl {
    pub(crate) cancel: bool,
    pub(crate) swing: bool,
    pub(crate) hand: Hand,
    pub(crate) calls: Arc<Mutex<Vec<Hand>>>,
    pub(crate) packets: Arc<Mutex<Vec<String>>>,
}
#[cfg(feature = "client")]
#[mod_api::__subscriptions]
impl Hooks for KeyControl {
    fn client_packet_send(
        &self,
        _: &dyn ClientHost,
        e: &mut mod_api::ClientPacketSendEvent<'_>,
    ) -> Flow {
        self.packets.lock().unwrap().push(e.packet.id.to_string());
        Flow::Continue
    }
    fn interaction_key_mapping_triggered(
        &self,
        _: &mut dyn ClientHost,
        e: &mut InteractionKeyMappingTriggeredEvent,
    ) -> Flow {
        self.calls.lock().unwrap().push(e.hand);
        if e.hand == self.hand {
            e.swing_hand = self.swing;
            if self.cancel {
                return Flow::Cancel;
            }
        }
        Flow::Continue
    }
}

#[cfg(feature = "client")]
pub(crate) struct FirstSilent(pub(crate) std::sync::atomic::AtomicBool);
#[cfg(feature = "client")]
#[mod_api::__subscriptions]
impl Hooks for FirstSilent {
    fn interaction_key_mapping_triggered(
        &self,
        _: &mut dyn ClientHost,
        e: &mut InteractionKeyMappingTriggeredEvent,
    ) -> Flow {
        if !self.0.swap(true, std::sync::atomic::Ordering::Relaxed) {
            e.swing_hand = false;
        }
        Flow::Continue
    }
}
