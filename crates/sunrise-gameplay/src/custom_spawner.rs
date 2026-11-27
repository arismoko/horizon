//! What the custom spawners (`CustomSpawner`: cats, the wandering trader, the village siege) call on `ServerLevel`.

use std::collections::HashSet;
use sunrise_core::geometry::Vec3;
use sunrise_game_data::gamerules::RuleIndex;
use sunrise_items::blocks::BlockFacts;
use sunrise_level::world::World;
use sunrise_sim::{
    join_seq::JoinList,
    mobs::{Mob, Species, raider, spawn_rules},
};

/// `SectionPos` of a block position.
pub fn section(p: [i32; 3]) -> (i32, i32, i32) {
    (p[0] >> 4, p[1] >> 4, p[2] >> 4)
}

/// `PoiManager.DistanceTracker`: sections from the nearest village centre, 7 when farther than 6.
pub fn sections_to_village(centers: &HashSet<(i32, i32, i32)>, at: (i32, i32, i32)) -> i32 {
    centers
        .iter()
        .map(|c| {
            (c.0 - at.0)
                .abs()
                .max((c.1 - at.1).abs())
                .max((c.2 - at.2).abs())
        })
        .min()
        .map_or(7, |d| d.min(7))
}

/// The level's data a custom spawner reads and writes, as one borrow of disjoint parts.
pub struct SpawnerParts<'a> {
    pub world: &'a mut World,
    pub mobs: &'a mut JoinList<Mob>,
    pub facts: &'a BlockFacts,
}

/// A player as a custom spawner sees one.
pub struct SpawnerPlayer {
    pub uuid: u128,
    pub position: Vec3,
    pub alive: bool,
    pub spectator: bool,
}

/// What the custom spawners call on `ServerLevel`; cold, so used as `&mut dyn`.
pub trait CustomSpawnerLevel {
    /// The level's data, borrowed for no longer than one statement between services.
    fn parts(&mut self) -> SpawnerParts<'_>;
    /// `Level.dimension() == Level.OVERWORLD`.
    fn overworld(&self) -> bool;
    /// `GameRules.getBoolean` of a rule.
    fn rule(&self, rule: RuleIndex) -> bool;
    /// `ServerLevel.players()` in the order the level lists them.
    fn players(&self) -> Vec<SpawnerPlayer>;
    /// The village centres' sections, as `PoiManager` knows them.
    fn village_centers(&self) -> HashSet<(i32, i32, i32)>;
    /// `SpawnPlacements.checkSpawnRules` at a block position.
    fn is_spawn_position_ok(&self, kind: &str, species: Species, pos: [i32; 3]) -> bool;
    /// A mob spawned with `finalizeSpawn`, snapped to `yaw` when given; its facing then takes no draw.
    fn spawn_mob(
        &mut self,
        kind: &str,
        at: [f64; 3],
        persistent: bool,
        reason: spawn_rules::Reason,
        yaw: Option<f32>,
    ) -> Option<u32>;
    /// A raider or event mob created and added for a patrol, raid or siege.
    fn spawn_event_mob(
        &mut self,
        kind: &str,
        at: [f64; 3],
        leader: bool,
        reason: raider::Reason,
    ) -> Option<u32>;
    /// The entries of a structure tag.
    fn structure_tag(&self, tag: &str) -> Vec<String>;
    /// The structures with a piece at a block position.
    fn structures_at(&self, pos: [i32; 3]) -> Vec<String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_to_village_is_a_chebyshev_distance_capped_at_seven() {
        let mut centers = HashSet::new();
        assert_eq!(sections_to_village(&centers, (0, 0, 0)), 7);
        centers.insert(section([40, 64, 40]));
        assert_eq!(sections_to_village(&centers, (2, 4, 2)), 0);
        assert_eq!(sections_to_village(&centers, (4, 4, 2)), 2);
        assert_eq!(sections_to_village(&centers, (-10, 4, 2)), 7);
        assert_eq!(section([-1, -17, 15]), (-1, -2, 0));
    }
}
