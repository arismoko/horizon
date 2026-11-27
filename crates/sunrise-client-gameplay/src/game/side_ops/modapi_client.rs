//! Client-side implementations of `side_ops::modapi`.
use crate::game::ClientGame;
use mod_api::AttachmentAccess;
use sunrise_gameplay_common::game::transactions::decision::Residence;

impl ClientGame<'_> {
    pub(in crate::game) fn client_attachment_map(
        &mut self,
        holder: mod_api::Holder,
        _access: AttachmentAccess,
    ) -> std::result::Result<&mut mod_api::Attachments, mod_api::AttachmentError> {
        let Some(remote) = self.client.remote.as_mut() else {
            return Err(mod_api::AttachmentError::MissingHolder);
        };
        remote.attachments.entry(holder).or_default();
        self.bind_remote_attachment(holder);
        let remote = self.client.remote.as_mut().unwrap();
        Ok(remote.attachments.get_mut(&holder).unwrap())
    }

    pub(in crate::game) fn client_entity_residence(&self, holder: mod_api::EntityRef) -> Residence {
        let Some(remote) = self.client.remote.as_ref() else {
            return Residence::Missing;
        };
        if self.common.native_lifecycle.epoch() != Some(holder.epoch()) {
            return Residence::Missing;
        }
        let Ok(id) = i32::try_from(holder.id()) else {
            return Residence::Missing;
        };
        if (id == remote.entity_id && self.common.player.is_some())
            || self.common.dragon.as_ref().is_some_and(|dragon| {
                sunrise_gameplay_common::game::dragon_common::dragon_part_index(
                    dragon.id,
                    holder.id(),
                )
                .is_some()
            })
            || remote.tracked(id).is_some()
        {
            Residence::Found
        } else {
            Residence::Missing
        }
    }
}
