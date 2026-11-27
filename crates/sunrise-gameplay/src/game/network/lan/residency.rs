//! Bind actual network members to the existing residency journal.
use super::*;

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn bind_guest_residency(&self, hub: &mut Hub) {
        hub.payload_player_epoch = self.common.native_lifecycle.epoch();
        hub.guests.bind_residency(
            &self.common.resident_entities.table.borrow().producer(),
            |id| sunrise_gameplay_common::game::Dim::from_id(id).key(),
        );
    }
}

impl crate::game::ServerGame<'_> {
    /// The start of a phase vanilla runs before ChunkMap.tick and Sunrise
    /// among the entities' ticks (ServerChunkCache.tickChunks' spawning and
    /// thunder, the raids): the counter to give `lan_synced_since`.
    pub(in crate::game::server_side) fn lan_synced_from(&mut self) -> Option<i32> {
        self.lan_ids_so_far()
    }
}
