//! Server-side implementations of `side_ops::modapi`.
use crate::game::ServerGame;
use mod_api::AttachmentAccess;
use sunrise_gameplay_common::game::transactions::decision::Residence;

impl ServerGame<'_> {
    pub(in crate::game) fn server_attachment_map(
        &mut self,
        holder: mod_api::Holder,
        access: AttachmentAccess,
    ) -> Result<&mut mod_api::Attachments, mod_api::AttachmentError> {
        self.attachment_map_quiet(holder, access)
    }

    pub(in crate::game) fn server_entity_residence(&self, holder: mod_api::EntityRef) -> Residence {
        use sunrise_sim::join_seq::residency::table::EntityLookup;
        match self.server_resident_entity(holder.id()) {
            EntityLookup::Taken => Residence::Taken,
            EntityLookup::Found { .. } => Residence::Found,
            EntityLookup::Missing => Residence::Missing,
        }
    }
}
