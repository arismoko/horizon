//! A server mod's chunk areas for one player (#3716, step 9b'): `PlayerRef::set_chunk_areas`
//! on the hub's viewed areas (`lan::viewed_levels`), which own the caps, the tickets and what is
//! sent. Each mod's areas are kept apart by the id of the mod whose call this is.
use crate::game::ServerGame;
use mod_api::{
    ChunkArea, ChunkAreaMode, ChunkAreaRefusal, ChunkCap, ChunkPos, LevelRef, PlayerRef,
};
use sunrise_sim::lan::viewed_levels::areas as engine;

fn mode(mode: ChunkAreaMode) -> engine::ChunkAreaMode {
    match mode {
        ChunkAreaMode::Loaded => engine::ChunkAreaMode::Loaded,
        ChunkAreaMode::Ticking => engine::ChunkAreaMode::Ticking,
    }
}

fn cap(cap: engine::ChunkCap) -> ChunkCap {
    match cap {
        engine::ChunkCap::LoadedPerPlayer => ChunkCap::LoadedPerPlayer,
        engine::ChunkCap::TickingPerPlayer => ChunkCap::TickingPerPlayer,
        engine::ChunkCap::LoadedPerLevel => ChunkCap::LoadedPerLevel,
        engine::ChunkCap::TickingPerLevel => ChunkCap::TickingPerLevel,
    }
}

/// The hub's refusal as the API's, its level a handle of the open `level` names.
fn refusal(
    refused: engine::ChunkAreaRefusal,
    level: impl Fn(&str) -> LevelRef,
) -> ChunkAreaRefusal {
    use engine::ChunkAreaRefusal as R;
    match refused {
        R::NotOnline => ChunkAreaRefusal::NotOnline,
        R::RadiusTooLarge { radius, max } => ChunkAreaRefusal::RadiusTooLarge { radius, max },
        R::CenterOutOfRange {
            center: (x, z),
            max,
        } => ChunkAreaRefusal::CenterOutOfRange {
            center: ChunkPos::new(x, z),
            max,
        },
        R::TooManyAreas { asked, max } => ChunkAreaRefusal::TooManyAreas { asked, max },
        R::TooManyLevels { asked, cap } => ChunkAreaRefusal::TooManyLevels { asked, cap },
        R::TooManyChunks {
            cap: hit,
            level: name,
            asked,
            limit,
        } => ChunkAreaRefusal::TooManyChunks {
            cap: cap(hit),
            level: level(&name),
            asked,
            limit,
        },
    }
}

impl ServerGame<'_> {
    /// For a server mod: its areas for `player`, in place of its last ones, through
    /// `Hub::set_viewed_areas`. Refused, changing nothing, for a player not online, a level
    /// handle of an earlier open, a call outside a mod's hook, and what the hub refuses.
    pub(in crate::game) fn player_set_chunk_areas(
        &mut self,
        player: PlayerRef,
        areas: &[ChunkArea],
    ) -> Result<(), ChunkAreaRefusal> {
        let owner =
            mod_api::content::crash::current_native_mod().ok_or(ChunkAreaRefusal::Unavailable)?;
        let epoch = self
            .common
            .native_lifecycle
            .epoch()
            .ok_or(ChunkAreaRefusal::NotOnline)?;
        if self.online_player_record(player).is_none()
            && self.native_acting_player() != Some(player)
        {
            return Err(ChunkAreaRefusal::NotOnline);
        }
        if areas.iter().any(|a| a.level.epoch() != epoch) {
            return Err(ChunkAreaRefusal::Unavailable);
        }
        if areas.len() > engine::MAX_AREAS {
            return Err(ChunkAreaRefusal::TooManyAreas {
                asked: areas.len() as u32,
                max: engine::MAX_AREAS as u32,
            });
        }
        let asked = areas
            .iter()
            .map(|a| engine::ChunkArea {
                level: a.level.dimension_id().to_owned(),
                center: (a.center.x, a.center.z),
                radius: a.radius,
                mode: mode(a.mode),
            })
            .collect();
        let host = self
            .server
            .lan
            .as_mut()
            .ok_or(ChunkAreaRefusal::NotOnline)?;
        host.hub
            .set_viewed_areas(player.uuid(), owner, asked)
            .map_err(|refused| {
                refusal(refused, |id| {
                    LevelRef::new(epoch, mod_api::__private::dimension_key(id))
                })
            })
    }
}
