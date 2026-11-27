//! Admission for placement intents uses the actual connection's negotiated capability.
#[cfg(test)]
#[path = "settings_placement_tests.rs"]
mod tests;
use super::*;
impl Connection {
    pub fn receive_placement(
        &mut self,
        data: &[u8],
    ) -> Option<super::super::placement_sync::Frame> {
        use super::super::placement_sync::Frame;
        if !self.capability || self.authority.is_none() {
            log::warn!("[recipe book] ignored placement frame before settings negotiation");
            return None;
        }
        let Some(message) = settings_codec::decode(data) else {
            log::warn!(
                "[recipe book] ignored malformed placement payload ({} bytes)",
                data.len()
            );
            return None;
        };
        match message {
            Message::Placement(
                frame @ (Frame::Hello { .. } | Frame::Catalog { .. } | Frame::Place { .. }),
            ) => Some(frame),
            other => {
                log::warn!(
                    "[recipe book] ignored unexpected serverbound placement message: {other:?}"
                );
                None
            }
        }
    }
}
