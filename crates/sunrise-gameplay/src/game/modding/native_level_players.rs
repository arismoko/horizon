//! Level player visits and chunk/entity tracking queries. The level visit
//! resolves UUIDs from the spawn code's player enumeration so a guest's turn
//! names its player once, excluding anyone who cannot resolve.
use crate::game::ServerGame;
pub use core::ops::ControlFlow;
use mod_api::{ChunkRef, EntityReadError, EntityRef, LevelReadError, LevelRef, PlayerRef};

impl ServerGame<'_> {
    /// `ServerLevel.players`: the online players of an open server level, each once.
    pub(in crate::game) fn level_visit_online_players(
        &self,
        level: LevelRef,
        visit: &mut dyn FnMut(PlayerRef) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, LevelReadError> {
        self.native_level_world_result(level)?;
        Ok(self
            .level_player_uuids(level.dimension_id())
            .filter_map(|uuid| self.player(uuid))
            .try_for_each(visit))
    }

    /// `ChunkMap.TrackedEntity.seenBy`: the players of the level in place whose tracker holds
    /// the entity, other than the entity itself.
    pub fn entity_visit_trackers(
        &self,
        entity: EntityRef,
        visit: &mut dyn FnMut(PlayerRef) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, EntityReadError> {
        let target = self.native_server_entity_target(entity)?;
        if self.target_level(target).key() != self.common.dim.key() {
            return Err(EntityReadError::Unavailable);
        }
        let id = entity.id() as i32;
        Ok(self
            .server
            .lan
            .iter()
            .flat_map(|h| h.hub.guests.iter().map(move |g| (&h.hub, g)))
            .filter(|(hub, g)| hub.here(g) && g.entity_id != id && g.tracker.is_tracking(id))
            .filter_map(|(_, g)| self.player(g.uuid))
            .try_for_each(visit))
    }

    /// `ChunkMap.getPlayers(pos, false)`: the players the chunk was sent to and is
    /// inside the view of.
    pub(in crate::game) fn chunk_visit_watchers(
        &self,
        chunk: ChunkRef,
        visit: &mut dyn FnMut(PlayerRef) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, LevelReadError> {
        self.native_level_world_result(chunk.level())?;
        let dimension = chunk.level().dimension_id();
        let [x, z] = chunk.pos();
        Ok(self
            .server
            .lan
            .iter()
            .flat_map(|h| h.hub.guests.iter())
            .filter(|g| g.tracks_chunk(dimension, (x, z)))
            .filter_map(|g| self.player(g.uuid))
            .try_for_each(visit))
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
