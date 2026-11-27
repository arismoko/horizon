use crate::game::ClientGame;
use mod_api::EntityRef;

impl ClientGame<'_> {
    pub(in crate::game) fn client_pick_section_move(&mut self, id: u32, to: [f64; 3]) {
        self.pick_section_move(id, to);
    }

    pub(in crate::game) fn client_pick_section_sync_moved_id(&mut self, id: u32) {
        self.pick_section_sync_moved_id(id);
    }

    pub(in crate::game) fn client_pick_section_remove(&mut self, id: u32) {
        self.pick_section_remove(id);
    }

    /// A client's entities after its tick: each one where its interpolation or its own move
    /// put it (its live position, not the packet's), and this window's player.
    pub fn client_entity_sections_stepped(&mut self) {
        let Some(scope) = self.entity_section_scope() else {
            return;
        };
        let own = self.own_entity_id();
        let mut sections = std::mem::take(&mut self.common.native_lifecycle.entity_sections);
        if let Some(r) = self.client.remote.as_ref() {
            for &id in r.entities.keys().filter(|&&id| id != own && id > 0) {
                if let Some(at) = self.entity_position(EntityRef::new(scope.epoch, id as u32)) {
                    sections.moved(scope, id as u32, None, at);
                }
            }
        }
        if let Some(player) = self.common.player.as_ref()
            && own > 0
        {
            let at = player.position;
            sections.moved(scope, own as u32, None, [at.x, at.y, at.z]);
        }
        self.common.native_lifecycle.entity_sections = sections;
        self.tell_entity_sections();
    }
    /// A client handles one packet from its server ([`Game::apply_packet`]). A packet that adds,
    /// moves or removes an entity is a position-update boundary. The add handler files an
    /// admitted entity untold; a move (`handleMoveEntity`, `handleEntityPositionSync`,
    /// `handleTeleportEntity`, `handleMovePlayer`'s `PlayerPosition`, a vehicle's) is told when
    /// it put the entity in another section, and `handleRespawn`'s new player starts over.
    /// An entity a move only starts interpolating moves at the client's entity tick.
    pub fn handle_packet(
        &mut self,
        packet: sunrise_protocol::packets::play::Clientbound,
    ) -> super::Result<()> {
        use sunrise_protocol::packets::play::Clientbound as C;
        if self.entity_section_scope().is_none() {
            return self.apply_packet(packet);
        }
        if let C::RemoveEntities(p) = &packet {
            for &id in &p.ids {
                self.entity_section_forget(id as u32);
            }
        }
        let respawned = matches!(packet, C::Respawn(_));
        let result = self.apply_packet(packet);
        if respawned {
            self.entity_section_forget(self.own_entity_id() as u32);
        }
        result
    }
}

use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
