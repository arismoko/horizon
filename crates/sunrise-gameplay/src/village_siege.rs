//! `VillageSiege`: one night in ten, zombies spawn near a village a player stands in.
use crate::custom_spawner::{CustomSpawnerLevel, section, sections_to_village};
use std::collections::HashSet;
use sunrise_core::{geometry::Vec3, random::RandomSource};
use sunrise_game_data::gamerules::vanilla::{SPAWN_MOBS, SPAWN_MONSTERS};
use sunrise_sim::mobs::raider;

/// What `VillageSiege` calls on `ServerLevel` besides the custom spawner services.
pub trait SiegeLevel: CustomSpawnerLevel {
    /// `ServerLevel.getSkyDarken`.
    fn sky_darken(&self) -> i32;
    /// `Heightmap.Types.WORLD_SURFACE` at a column.
    fn surface_height(&self, x: i32, z: i32) -> i32;
    /// Whether a biome tag holds the biome at a block position.
    fn biome_tag(&self, tag: &str, pos: [i32; 3]) -> bool;
    /// `Monster.checkMonsterSpawnRules` for a siege zombie, drawing from the level's random.
    fn check_zombie_spawn_rules(&mut self, pos: [i32; 3]) -> bool;
    /// The `village_siege` hook: true when a mod cancels the siege.
    fn village_siege_cancelled(&mut self, player: u128, attempted: [f64; 3]) -> bool;
}

/// `VillageSiege`: whether tonight is a siege, whether the spot is chosen,
/// and the zombies still to spawn there.
#[derive(Default)]
pub struct VillageSiege {
    pub tonight: bool,
    pub set_up: bool,
    pub zombies_to_spawn: i32,
    pub next_spawn_time: i32,
    pub spot: [i32; 3],
}

/// `ServerLevel.isVillage`: within one section of a village.
pub fn is_village(centers: &HashSet<(i32, i32, i32)>, pos: [i32; 3]) -> bool {
    sections_to_village(centers, section(pos)) <= 1
}

fn block_of(p: Vec3) -> [i32; 3] {
    [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32]
}

impl VillageSiege {
    /// `VillageSiege.tick`: at midnight one night in ten, 20 zombies spawn
    /// two ticks apart near a village 32 blocks from a player standing in it.
    pub fn tick(&mut self, l: &mut dyn SiegeLevel) {
        if !l.overworld() {
            return;
        }
        let enemies = l.rule(SPAWN_MOBS) && l.rule(SPAWN_MONSTERS);
        if l.sky_darken() < 4 || !enemies {
            self.tonight = false;
            self.set_up = false;
            return;
        }
        // ClockTimeMarkers.ROLL_VILLAGE_SIEGE: tick 18000 of the 24000 tick day.
        let world = l.parts().world;
        if world.day_time.rem_euclid(24000) == 18000 {
            self.tonight = world.random.next_int_bound(10).unwrap_or(1) == 0;
        }
        if !self.tonight {
            return;
        }
        if !self.set_up {
            if !self.try_to_setup_siege(l) {
                return;
            }
            self.set_up = true;
        }
        if self.next_spawn_time > 0 {
            self.next_spawn_time -= 1;
            return;
        }
        self.next_spawn_time = 2;
        if self.zombies_to_spawn > 0 {
            self.try_spawn_siege_zombie(l);
            self.zombies_to_spawn -= 1;
        } else {
            self.tonight = false;
        }
    }

    /// `VillageSiege.tryToSetupSiege`: the first player standing in a village
    /// picks a spot 32 blocks away (ten tries); true when one stood in a
    /// village, even if no spot was found.
    fn try_to_setup_siege(&mut self, l: &mut dyn SiegeLevel) -> bool {
        let centers = l.village_centers();
        for player in l.players() {
            if player.spectator {
                continue;
            }
            let center = block_of(player.position);
            if !is_village(&centers, center)
                || l.biome_tag("minecraft:without_zombie_sieges", center)
            {
                continue;
            }
            for _ in 0..10 {
                let angle = l.parts().world.random.next_float() * std::f32::consts::TAU;
                let spot = [
                    center[0] + (angle.cos() * 32.0).floor() as i32,
                    center[1],
                    center[2] + (angle.sin() * 32.0).floor() as i32,
                ];
                if let Some(attempted_pos) = Self::find_siege_spawn_pos(l, &centers, spot) {
                    if l.village_siege_cancelled(player.uuid, attempted_pos) {
                        return false;
                    }
                    self.spot = spot;
                    self.next_spawn_time = 0;
                    self.zombies_to_spawn = 20;
                    break;
                }
            }
            return true;
        }
        false
    }

    /// `VillageSiege.trySpawn`: one zombie, in a random yaw.
    fn try_spawn_siege_zombie(&mut self, l: &mut dyn SiegeLevel) {
        let centers = l.village_centers();
        let Some(at) = Self::find_siege_spawn_pos(l, &centers, self.spot) else {
            return;
        };
        let Some(id) = l.spawn_event_mob("minecraft:zombie", at, false, raider::Reason::Event)
        else {
            return;
        };
        let parts = l.parts();
        let yaw = parts.world.random.next_float() * 360.0;
        if let Some(m) = parts.mobs.iter_mut().find(|m| m.id == id) {
            (m.yaw, m.previous_yaw, m.body_yaw, m.previous_body_yaw) = (yaw, yaw, yaw, yaw);
            (m.head_yaw, m.previous_head_yaw) = (yaw, yaw);
        }
    }

    /// `VillageSiege.findRandomSpawnPos`: ten tries within 8 blocks of the
    /// spot for a village column where a zombie may spawn.
    fn find_siege_spawn_pos(
        l: &mut dyn SiegeLevel,
        centers: &HashSet<(i32, i32, i32)>,
        spot: [i32; 3],
    ) -> Option<[f64; 3]> {
        for _ in 0..10 {
            let world = &mut *l.parts().world;
            let x = spot[0] + world.random.next_int_bound(16).unwrap_or(0) - 8;
            let z = spot[2] + world.random.next_int_bound(16).unwrap_or(0) - 8;
            if !world.chunks.contains_key(&(x >> 4, z >> 4)) {
                continue;
            }
            let y = l.surface_height(x, z);
            if is_village(centers, [x, y, z]) && l.check_zombie_spawn_rules([x, y, z]) {
                return Some([f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5]);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::custom_spawner::{SpawnerParts, SpawnerPlayer};
    use std::sync::Arc;
    use sunrise_core::random::{LegacyRandom, LevelRandom};
    use sunrise_game_data::gamerules::RuleIndex;
    use sunrise_items::blocks::BlockFacts;
    use sunrise_level::{chunk::Chunk, world::World};
    use sunrise_sim::{
        join_seq::JoinList,
        mobs::{Mob, Species, spawn_rules},
    };
    use sunrise_world::registry::BlockStateId;

    struct Fake {
        world: World,
        mobs: JoinList<Mob>,
        facts: BlockFacts,
        spawned: usize,
        checks: usize,
    }

    impl Fake {
        fn new(seed: i64, day_time: i64) -> Self {
            let mut world = World::new(BlockStateId(0), 0);
            for cx in -8..8 {
                for cz in -8..8 {
                    world
                        .chunks
                        .insert((cx, cz), Arc::new(Chunk::empty(cx, cz)));
                }
            }
            world.day_time = day_time;
            world.random = LevelRandom::for_level(LegacyRandom::new(seed));
            Self {
                world,
                mobs: JoinList::default(),
                facts: BlockFacts {
                    states: vec![],
                    air: BlockStateId(0),
                    woods: Default::default(),
                    sign_texts: Default::default(),
                },
                spawned: 0,
                checks: 0,
            }
        }
    }

    impl CustomSpawnerLevel for Fake {
        fn parts(&mut self) -> SpawnerParts<'_> {
            SpawnerParts {
                world: &mut self.world,
                mobs: &mut self.mobs,
                facts: &self.facts,
            }
        }
        fn overworld(&self) -> bool {
            true
        }
        fn rule(&self, _rule: RuleIndex) -> bool {
            true
        }
        fn players(&self) -> Vec<SpawnerPlayer> {
            vec![SpawnerPlayer {
                uuid: 1,
                position: Vec3::new(0.5, 64.0, 0.5),
                alive: true,
                spectator: false,
            }]
        }
        fn village_centers(&self) -> HashSet<(i32, i32, i32)> {
            (-4..4)
                .flat_map(|x| (-4..4).map(move |z| (x, 4, z)))
                .collect()
        }
        fn is_spawn_position_ok(&self, _: &str, _: Species, _: [i32; 3]) -> bool {
            true
        }
        fn spawn_mob(
            &mut self,
            _: &str,
            _: [f64; 3],
            _: bool,
            _: spawn_rules::Reason,
            _: Option<f32>,
        ) -> Option<u32> {
            None
        }
        fn spawn_event_mob(
            &mut self,
            _: &str,
            _: [f64; 3],
            _: bool,
            _: raider::Reason,
        ) -> Option<u32> {
            self.spawned += 1;
            Some(self.spawned as u32)
        }
        fn structure_tag(&self, _: &str) -> Vec<String> {
            vec![]
        }
        fn structures_at(&self, _: [i32; 3]) -> Vec<String> {
            vec![]
        }
    }

    impl SiegeLevel for Fake {
        fn sky_darken(&self) -> i32 {
            15
        }
        fn surface_height(&self, _: i32, _: i32) -> i32 {
            64
        }
        fn biome_tag(&self, _: &str, _: [i32; 3]) -> bool {
            false
        }
        fn check_zombie_spawn_rules(&mut self, _: [i32; 3]) -> bool {
            self.checks += 1;
            self.world.random.next_int_bound(4).unwrap_or(0) != 0
        }
        fn village_siege_cancelled(&mut self, _: u128, _: [f64; 3]) -> bool {
            false
        }
    }

    fn night(seed: i64, day_time: i64) -> (VillageSiege, Fake) {
        let mut level = Fake::new(seed, day_time);
        let mut siege = VillageSiege::default();
        siege.tick(&mut level);
        (siege, level)
    }

    #[test]
    fn the_roll_at_tick_18000_is_one_in_ten_and_other_ticks_draw_nothing() {
        let (siege, mut level) = night(1, 17999);
        assert!(!siege.tonight);
        let mut fresh = Fake::new(1, 0);
        assert_eq!(level.world.random.next_int(), fresh.world.random.next_int());
        let hits = (0..200).filter(|&s| night(s, 18000).0.tonight).count();
        assert!((5..=40).contains(&hits), "{hits} sieges in 200 seeds");
    }

    #[test]
    fn a_siege_night_spawns_twenty_zombies_two_ticks_apart() {
        let seed = (0..200).find(|&s| night(s, 18000).0.tonight).unwrap();
        let (mut siege, mut level) = night(seed, 18000);
        let mut ticks = 1;
        while siege.tonight && ticks < 200 {
            level.world.day_time = 18001;
            siege.tick(&mut level);
            ticks += 1;
        }
        assert_eq!(level.spawned, 20);
        assert!(level.checks >= 20);
        assert_eq!(siege.zombies_to_spawn, 0);
    }
}
