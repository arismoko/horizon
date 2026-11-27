//! `CatSpawner`'s clock and its picks that need no level services: where a cat may try to spawn, and whether a village or a hut takes one.
use crate::custom_spawner::{CustomSpawnerLevel, section, sections_to_village};
use sunrise_core::{
    geometry::Vec3,
    random::{LevelRandom, RandomSource},
};
use sunrise_level::chunk_map::ChunkMap;
use sunrise_sim::mobs::{Mob, Species, spawn_rules::Reason};

const TICK_DELAY: i32 = 1200;

/// `CatSpawner.nextTick`.
#[derive(Default)]
pub struct CatSpawner {
    pub next_tick: i32,
}
impl CatSpawner {
    /// One tick of the countdown: true every 1200 ticks, the first time on the first tick.
    pub fn due(&mut self) -> bool {
        self.next_tick -= 1;
        if self.next_tick > 0 {
            return false;
        }
        self.next_tick = TICK_DELAY;
        true
    }
}

/// `CatSpawner.tick` after the countdown: a cat near a random player, in a village or a swamp hut.
pub fn spawn_near_player(l: &mut dyn CustomSpawnerLevel) {
    // ServerLevel.getRandomPlayer: one of the living players.
    let alive: Vec<Vec3> = l
        .players()
        .into_iter()
        .filter(|p| p.alive)
        .map(|p| p.position)
        .collect();
    let parts = l.parts();
    let Some(pos) = candidate(&mut parts.world.random, &alive, &parts.world.chunks) else {
        return;
    };
    if !l.is_spawn_position_ok("minecraft:cat", Species::Cat, pos) {
        return;
    }
    if sections_to_village(&l.village_centers(), section(pos)) <= 2 {
        if village_takes_cat(l.parts().mobs, pos) {
            spawn_cat(l, pos, false);
        }
    } else {
        let hut = l.structure_tag("minecraft:cats_spawn_in");
        if l.structures_at(pos).iter().any(|s| hut.contains(s))
            && hut_takes_cat(l.parts().mobs, pos)
        {
            spawn_cat(l, pos, true);
        }
    }
}

/// `spawnCat`: a natural spawn, then `snapTo(spawnPos, 0.0F, 0.0F)`, so the cat faces yaw 0 and its facing takes no draw.
pub fn spawn_cat(l: &mut dyn CustomSpawnerLevel, pos: [i32; 3], persistent: bool) {
    let at = [
        f64::from(pos[0]) + 0.5,
        f64::from(pos[1]),
        f64::from(pos[2]) + 0.5,
    ];
    let Some(id) = l.spawn_mob("minecraft:cat", at, persistent, Reason::Natural, Some(0.0)) else {
        return;
    };
    // `Cat.finalizeSpawn` asks `#cats_spawn_as_black` about the origin: `snapTo` runs after it, so a hut does not make it black.
    let black = l.structure_tag("minecraft:cats_spawn_as_black");
    let at_origin = l.structures_at([0, 0, 0]);
    if at_origin.iter().any(|s| black.contains(s))
        && let Some(m) = l.parts().mobs.iter_mut().find(|m| m.id == id)
    {
        m.extra.variant = 10;
    }
}

/// One of the living players in `alive`, drawn from the level random, then the two offsets; the
/// position when the chunks within 10 blocks of it are loaded (`hasChunksAt(pos +- 10)`).
/// Draws nothing when `alive` is empty.
pub fn candidate(random: &mut LevelRandom, alive: &[Vec3], chunks: &ChunkMap) -> Option<[i32; 3]> {
    if alive.is_empty() {
        return None;
    }
    let pick = random.next_int_bound(alive.len() as i32).unwrap_or(0);
    let p = alive[pick as usize];
    let mut offset = || {
        let n = 8 + random.next_int_bound(24).unwrap_or(0);
        if random.next_boolean() { -n } else { n }
    };
    let x = offset();
    let z = offset();
    let pos = [
        p.x.floor() as i32 + x,
        p.y.floor() as i32,
        p.z.floor() as i32 + z,
    ];
    let loaded = ((pos[0] - 10) >> 4..=(pos[0] + 10) >> 4).all(|cx| {
        ((pos[2] - 10) >> 4..=(pos[2] + 10) >> 4).all(|cz| chunks.contains_key(&(cx, cz)))
    });
    loaded.then_some(pos)
}

/// `spawnInVillage`: more than four occupied beds within 48 blocks and fewer than five cats around.
pub fn village_takes_cat(mobs: &[Mob], pos: [i32; 3]) -> bool {
    // PoiManager.getCountInRange(HOME, IS_OCCUPIED): beds villagers have claimed.
    let mut beds: Vec<[i32; 3]> = mobs
        .iter()
        .filter(|m| m.species() == Species::Villager && m.alive())
        .filter_map(|m| m.extra.bed)
        .filter(|b| {
            let d = |i: usize| i64::from(b[i] - pos[i]);
            d(0).abs() <= 48
                && d(2).abs() <= 48
                && d(0).pow(2) + d(1).pow(2) + d(2).pow(2) <= 48 * 48
        })
        .collect();
    beds.sort_unstable();
    beds.dedup();
    beds.len() > 4 && cats_near(mobs, pos, 48.0) < 5
}

/// `spawnInHut`: no cat within 16 blocks.
pub fn hut_takes_cat(mobs: &[Mob], pos: [i32; 3]) -> bool {
    cats_near(mobs, pos, 16.0) == 0
}

/// Cats in `AABB(pos).inflate(radius, 8, radius)`.
pub fn cats_near(mobs: &[Mob], pos: [i32; 3], radius: f64) -> usize {
    let [x, y, z] = pos.map(f64::from);
    let min = Vec3::new(x - radius, y - 8.0, z - radius);
    let max = Vec3::new(x + 1.0 + radius, y + 9.0, z + 1.0 + radius);
    mobs.iter()
        .filter(|m| {
            m.species() == Species::Cat && m.alive() && m.bounds().intersects_corners(min, max)
        })
        .count()
}
