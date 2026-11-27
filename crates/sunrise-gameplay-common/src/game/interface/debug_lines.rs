//! The F3 overlay's text entries (26.3 `DebugEntry*`): what each enabled
//! entry of `debug-profile.json` reports to the `DebugScreenDisplayer`, in
//! `DebugScreenEntryList.rebuildCurrentList`'s order (by identifier). The
//! entries that read the server's level ask the integrated server for their
//! lines (`ServerDebug`); on another server they show what vanilla shows
//! there, nothing or `??`.
use std::collections::HashSet;

/// `DebugEntryLookingAt.RANGE`.
pub const LOOKING_AT_RANGE: f64 = 20.0;

/// The entries whose lines come from the server thread.
pub const SERVER_ENTRIES: [&str; 4] = [
    "chunk_source_stats",
    "entity_spawn_counts",
    "heightmap",
    "local_difficulty",
];

/// What the server thread reports for the window's player
/// (`DebugScreenOverlay.getServerChunk` and its `ServerLevel`).
#[derive(Clone, Copy, Debug, Default)]
pub struct ServerDebug {
    /// The server chunk's heightmaps at the feet (`keepAfterWorldgen`).
    pub heights: Option<[i32; 4]>,
    pub difficulty: Option<f32>,
    pub spawn_counts: Option<(i32, [i32; 8])>,
    pub chunks: Option<(usize, usize, usize)>,
}

impl crate::game::CommonGame<'_> {
    /// `Level.getEntityCount` over the entities Sunrise keeps.
    pub fn debug_entity_count(&self) -> usize {
        self.mobs.iter().filter(|m| !m.removed).count()
            + self.dropped.len()
            + self.orbs.len()
            + self.paintings.len()
            + self.item_frames.len()
            + self.armor_stands.len()
            + self.boats.len()
            + self.carts.len()
            + self.crystals.len()
            + self.creatures.projectiles.len()
    }

    /// The sections holding a mob, an item or an orb
    /// (`EntitySectionStorage.count`).
    pub fn debug_entity_sections(&self) -> usize {
        let sections: HashSet<[i32; 3]> = self
            .mobs
            .iter()
            .filter(|m| !m.removed)
            .map(|m| [m.position.x, m.position.y, m.position.z])
            .chain(
                self.dropped
                    .iter()
                    .map(|i| [i.position.x, i.position.y, i.position.z]),
            )
            .chain(
                self.orbs
                    .iter()
                    .map(|o| [o.position.x, o.position.y, o.position.z]),
            )
            .map(|p| p.map(|v| (v.floor() as i32) >> 4))
            .collect();
        sections.len()
    }
}
