use crate::game::Dim;
use mod_api::AttachmentHolder;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use sunrise_mod_runtime::attachments::{AttachmentBatch, index::AttachmentIndex};

pub(in crate::game) type Indexes = HashMap<Dim, Arc<Mutex<AttachmentIndex<AttachmentHolder>>>>;

impl crate::game::CommonGame<'_> {
    pub fn attachment_validate_write(
        &self,
        token: mod_api::AttachmentWriteToken,
    ) -> Result<(), mod_api::AttachmentRowError> {
        mod_api::__private::game::validate_attachment_write(
            &token,
            self.attachment_indexes.get(&self.dim).map(Arc::as_ref),
        )
    }

    pub fn attachment_query(
        &self,
        key: &mod_api::AttachmentCaptureKey<'_>,
    ) -> Result<mod_api::AttachmentBatch, mod_api::AttachmentError> {
        let (namespace, name) = key.id();
        let batch = self
            .attachment_indexes
            .get(&self.dim)
            .map_or_else(AttachmentBatch::empty, |index| {
                AttachmentBatch::new(index.lock().unwrap().snapshot_named(namespace, name))
            });
        Ok(batch
            .in_scope(&self.transactions.attachment_query_scopes)
            .into())
    }

    pub fn attachment_read_scope(
        &self,
    ) -> Result<mod_api::AttachmentReadScope<'_>, mod_api::AttachmentRowError> {
        Ok(self.attachment_frame_borrow())
    }

    pub fn attachment_frame_borrow(&self) -> mod_api::AttachmentReadScope<'_> {
        mod_api::__private::game::attachment_read_scope(&self.transactions.attachment_query_scopes)
    }
}
