use crate::game::ClientGame;
use mod_api::Holder;

impl ClientGame<'_> {
    pub(in crate::game) fn client_bind_constructed_entity_attachments(&mut self, _id: u32) {}

    pub(in crate::game::client_side) fn forget_remote_entity_construction(&mut self, id: i32) {
        self.common
            .forget_entity_initialization(Holder::Entity(id as u32));
        if let Some(uuid) = self
            .client
            .remote
            .as_ref()
            .and_then(|r| r.level_players.get(&id))
            .copied()
        {
            self.common
                .forget_entity_initialization(Holder::Player(uuid));
        }
    }
}
