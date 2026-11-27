//! The host's typed payload token check: what every typed send and reply runs first.
use mod_api::{PayloadSendError, PayloadToken};

impl crate::game::CommonGame<'_> {
    /// Check `token` against this load's frozen typed declarations.
    pub fn check_payload_token(&self, token: &PayloadToken) -> Result<(), PayloadSendError> {
        mod_api::__private::game::check_payload_token(token)
    }
}

impl crate::game::CommonGame<'_> {
    /// The drop counts of `Droppable` channel `id`.
    pub fn payload_drop_counts(
        &self,
        id: &mod_api::PayloadId,
    ) -> Result<Option<mod_api::PayloadDropCounts>, mod_api::LevelReadError> {
        Ok(mod_api::__private::game::payload_drop_counts(id))
    }
}
