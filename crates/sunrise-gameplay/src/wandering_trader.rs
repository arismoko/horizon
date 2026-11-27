//! `WanderingTraderSpawner`'s clock and the `WanderingTraderData` saved with the world.
use crate::custom_spawner::{CustomSpawnerLevel, SpawnerParts};
use sunrise_core::{nbt::Tag, random::RandomSource};
use sunrise_sim::mobs::{Species, leash::Holder, spawn_rules::Reason, trader};

const TICK_DELAY: i32 = 1200;
const SPAWN_DELAY: i32 = 24000;
const MIN_CHANCE: i32 = 25;
const MAX_CHANCE: i32 = 75;
const ATTEMPTS: i32 = 10;

/// The spawner's clock and the `WanderingTraderData` saved with the world.
pub struct Spawner {
    tick_delay: i32,
    spawn_delay: i32,
    chance: i32,
    loaded: bool,
    dirty: bool,
}
impl Default for Spawner {
    fn default() -> Self {
        Self {
            tick_delay: TICK_DELAY,
            spawn_delay: SPAWN_DELAY,
            chance: MIN_CHANCE,
            loaded: false,
            dirty: false,
        }
    }
}
impl Spawner {
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }
    /// Reads the file's `data` compound (`None` when there is no file).
    pub fn load(&mut self, data: Option<&Tag>) {
        let get = |k: &str, d: i32| match data.and_then(|d| d.get(k)) {
            Some(Tag::Int(v)) => *v,
            _ => d,
        };
        self.spawn_delay = get("spawn_delay", SPAWN_DELAY);
        self.chance = get("spawn_chance", MIN_CHANCE);
        self.loaded = true;
    }
    /// The file to write, `None` unless loaded and changed.
    pub fn to_save(&self) -> Option<Tag> {
        if !self.loaded || !self.dirty {
            return None;
        }
        Some(Tag::compound([
            (
                "data",
                Tag::compound([
                    ("spawn_delay", Tag::Int(self.spawn_delay)),
                    ("spawn_chance", Tag::Int(self.chance)),
                ]),
            ),
            ("DataVersion", Tag::Int(sunrise_world::DATA_VERSION)),
        ]))
    }
    /// The file was written.
    pub fn saved(&mut self) {
        self.dirty = false;
    }
    /// One tick of the countdowns: the chance to roll against once the spawn delay runs out.
    pub fn due(&mut self) -> Option<i32> {
        self.tick_delay -= 1;
        if self.tick_delay > 0 {
            return None;
        }
        self.tick_delay = TICK_DELAY;
        self.spawn_delay -= TICK_DELAY;
        self.dirty = true;
        if self.spawn_delay > 0 {
            return None;
        }
        self.spawn_delay = SPAWN_DELAY;
        let chance = self.chance;
        self.chance = (chance + 25).clamp(MIN_CHANCE, MAX_CHANCE);
        Some(chance)
    }
    /// A trader appeared: the chance starts over.
    pub fn spawned(&mut self) {
        self.chance = MIN_CHANCE;
    }
}

/// What the wandering trader spawner calls on `ServerLevel` beyond the custom spawners' services.
pub trait TraderLevel: CustomSpawnerLevel {
    /// Highest motion-blocking (no leaves) block + 1 (`Heightmap.MOTION_BLOCKING_NO_LEAVES`).
    fn motion_top(&self, x: i32, z: i32) -> i32;
    /// Whether the biome at a block position is in a biome tag; false when its chunk is not loaded.
    fn chunk_biome_has(&self, tag: &str, pos: [i32; 3]) -> bool;
}

/// `WanderingTraderSpawner.tick`'s roll against `chance`, then the spawn; true when the chance starts over.
pub fn roll(l: &mut dyn TraderLevel, chance: i32) -> bool {
    l.parts().world.random.next_int_bound(100).unwrap_or(100) <= chance && spawn(l)
}

/// `WanderingTraderSpawner.spawn`.
fn spawn(l: &mut dyn TraderLevel) -> bool {
    let alive: Vec<[i32; 3]> = l
        .players()
        .into_iter()
        .filter(|p| p.alive)
        .map(|p| {
            [
                p.position.x.floor() as i32,
                p.position.y.floor() as i32,
                p.position.z.floor() as i32,
            ]
        })
        .collect();
    if alive.is_empty() {
        return true;
    }
    let pick = l
        .parts()
        .world
        .random
        .next_int_bound(alive.len() as i32)
        .unwrap_or(0);
    let player_pos = alive[pick as usize];
    if l.parts().world.random.next_int_bound(10).unwrap_or(1) != 0 {
        return false;
    }
    let reference = nearest_bell(l, player_pos, 48).unwrap_or(player_pos);
    let Some(pos) = find_spawn_near(l, reference, 48) else {
        return false;
    };
    if !has_space(l, pos) {
        return false;
    }
    if l.chunk_biome_has("minecraft:without_wandering_trader_spawns", pos) {
        return false;
    }
    let at = [
        f64::from(pos[0]) + 0.5,
        f64::from(pos[1]),
        f64::from(pos[2]) + 0.5,
    ];
    let Some(id) = l.spawn_mob("minecraft:wandering_trader", at, true, Reason::Event, None) else {
        return false;
    };
    for _ in 0..2 {
        spawn_llama(l, id, 4);
    }
    if let Some(m) = l.parts().mobs.iter_mut().find(|m| m.id == id) {
        trader::start_visit(m, reference);
    }
    true
}

/// `tryToSpawnLlamaFor`: a trader llama near the trader, on its lead.
fn spawn_llama(l: &mut dyn TraderLevel, trader: u32, radius: i32) {
    let Some(at) = l.parts().mobs.iter().find(|m| m.id == trader).map(|m| {
        [
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        ]
    }) else {
        return;
    };
    let Some(pos) = find_spawn_near(l, at, radius) else {
        return;
    };
    let spot = [
        f64::from(pos[0]) + 0.5,
        f64::from(pos[1]),
        f64::from(pos[2]) + 0.5,
    ];
    if let Some(id) = l.spawn_mob("minecraft:trader_llama", spot, false, Reason::Event, None)
        && let Some(m) = l.parts().mobs.iter_mut().find(|m| m.id == id)
    {
        // EntitySpawnReason.EVENT: grown up.
        m.extra.age = 0;
        m.extra.leash = Some(Holder::Mob(trader));
    }
}

/// `findSpawnPositionNear`: ten tries at a column near the reference where the trader could stand.
fn find_spawn_near(l: &mut dyn TraderLevel, reference: [i32; 3], radius: i32) -> Option<[i32; 3]> {
    for _ in 0..ATTEMPTS {
        let x = reference[0]
            + l.parts()
                .world
                .random
                .next_int_bound(radius * 2)
                .unwrap_or(0)
            - radius;
        let z = reference[2]
            + l.parts()
                .world
                .random
                .next_int_bound(radius * 2)
                .unwrap_or(0)
            - radius;
        if !l.parts().world.loaded(x, z) {
            continue;
        }
        let y = l.motion_top(x, z);
        if l.is_spawn_position_ok(
            "minecraft:wandering_trader",
            Species::WanderingTrader,
            [x, y, z],
        ) {
            return Some([x, y, z]);
        }
    }
    None
}

/// `hasEnoughSpace`: no collision in the 2 by 3 by 2 box.
fn has_space(l: &mut dyn TraderLevel, pos: [i32; 3]) -> bool {
    let SpawnerParts { world, facts, .. } = l.parts();
    (0..=1).all(|dx| {
        (0..=2).all(|dy| {
            (0..=1).all(|dz| {
                !facts
                    .get(world.block(pos[0] + dx, pos[1] + dy, pos[2] + dz))
                    .full_collision
            })
        })
    })
}

/// The closest bell (`PoiTypes.MEETING`) within `range` blocks.
fn nearest_bell(l: &mut dyn TraderLevel, from: [i32; 3], range: i32) -> Option<[i32; 3]> {
    let SpawnerParts { world, facts, .. } = l.parts();
    let mut best: Option<([i32; 3], i32)> = None;
    for x in from[0] - range..=from[0] + range {
        for z in from[2] - range..=from[2] + range {
            if !world.loaded(x, z) {
                continue;
            }
            for y in (from[1] - range).max(world.bottom_y())..=from[1] + range {
                if &*facts.get(world.block(x, y, z)).name != "minecraft:bell" {
                    continue;
                }
                let d = (x - from[0]).pow(2) + (y - from[1]).pow(2) + (z - from[2]).pow(2);
                if best.is_none_or(|b| d < b.1) {
                    best = Some(([x, y, z], d));
                }
            }
        }
    }
    best.map(|b| b.0)
}
