#[allow(unused_variables)]
impl SideOps for crate::game::server_side::ServerState {
    type Game<'g> = crate::game::ServerGame<'g>;

    fn side_attachment_map<'__side_g>(
        game: &'__side_g mut Self::Game<'_>,
        holder: mod_api::Holder,
        access: AttachmentAccess,
    ) -> std::result::Result<&'__side_g mut mod_api::Attachments, mod_api::AttachmentError> {
        game.server_attachment_map(holder, access)
    }
    fn side_entity_residence(game: &Self::Game<'_>, holder: mod_api::EntityRef) -> Residence {
        game.server_entity_residence(holder)
    }
}
use mod_api::AttachmentAccess;
use sunrise_gameplay_common::game::side_ops::modapi::SideOps;
use sunrise_gameplay_common::game::transactions::decision::Residence;
