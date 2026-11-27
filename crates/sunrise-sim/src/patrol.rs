//! `PatrolSpawner`: every ten to eleven minutes of daylight, one time in five,
//! a pillager patrol gathers 24 to 47 blocks from a random player who is not
//! near a village. The level is read through [`PatrolLevel`] and the pillagers
//! are made by the caller, so the game and the differential test
//! (`tools/test_raid_parity.py`) run the same decisions and random draws.
use crate::mobs::{
    MobType,
    spawn_rules::{self, Reason, SpawnLevel},
};
use sunrise_core::random::RandomSource;

/// What the patrol spawner reads from the level.
pub trait PatrolLevel: SpawnLevel {
    /// The `spawn_patrols` game rule.
    fn spawn_patrols(&self) -> bool;
    /// `Level.isBrightOutside`: no fixed time and a sky darkening below 4.
    fn bright_outside(&self) -> bool;
    /// The level's players, in `level.players()` order: block position and
    /// whether the player is a spectator.
    fn patrol_players(&self) -> Vec<([i32; 3], bool)>;
    /// `ServerLevel.sectionsToVillage` (`PoiManager`): 0 to 7.
    fn sections_to_village(&self, pos: [i32; 3]) -> i32;
    /// The chunk at chunk coordinates is loaded (`hasChunk`).
    fn has_chunk(&self, cx: i32, cz: i32) -> bool;
    /// The `gameplay/can_pillager_patrol_spawn` environment attribute.
    fn can_pillager_patrol_spawn(&self, pos: [i32; 3]) -> bool;
    /// `getCurrentDifficultyAt(pos).getEffectiveDifficulty()`.
    fn effective_difficulty(&self, pos: [i32; 3]) -> f32;
    /// `Heightmap.Types.MOTION_BLOCKING_NO_LEAVES` height of a column.
    fn motion_blocking_no_leaves(&self, x: i32, z: i32) -> i32;
}

/// `ServerLevel.isCloseToVillage`.
pub fn close_to_village<L: PatrolLevel + ?Sized>(l: &L, pos: [i32; 3], sections: i32) -> bool {
    sections <= 6 && l.sections_to_village(pos) <= sections
}

/// `Level.hasChunksAt(x0, z0, x1, z1)`.
pub fn has_chunks_at<L: PatrolLevel + ?Sized>(l: &L, x0: i32, z0: i32, x1: i32, z1: i32) -> bool {
    (x0 >> 4..=x1 >> 4).all(|cx| (z0 >> 4..=z1 >> 4).all(|cz| l.has_chunk(cx, cz)))
}

/// A pillager the spawner asks for: where, and whether it leads the patrol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Member {
    pub pos: [i32; 3],
    pub leader: bool,
}

/// `PatrolSpawner` (its countdown lives in memory: a fresh level tries at once).
#[derive(Clone, Debug, Default)]
pub struct PatrolSpawner {
    pub next_tick: i32,
}

impl PatrolSpawner {
    /// `PatrolSpawner.tick`. `spawn_enemies` is the chunk source's flag
    /// (`spawn_mobs` and `spawn_monsters`); `random` is the level random.
    /// `spawn` creates, finalizes and adds a pillager at a position that
    /// passed the spawn checks (`EntityType.create` succeeding), and says
    /// whether it did.
    pub fn tick<L: PatrolLevel, R: RandomSource>(
        &mut self,
        level: &L,
        spawn_enemies: bool,
        pillager: &MobType,
        random: &mut R,
        spawn: &mut dyn FnMut(&L, Member, &mut R) -> bool,
    ) {
        if !spawn_enemies || !level.spawn_patrols() {
            return;
        }
        self.next_tick -= 1;
        if self.next_tick > 0 {
            return;
        }
        self.next_tick += 12000 + random.next_int_bound(1200).unwrap_or(0);
        if !level.bright_outside() || random.next_int_bound(5).unwrap_or(1) != 0 {
            return;
        }
        let players = level.patrol_players();
        if players.is_empty() {
            return;
        }
        let (player, spectator) =
            players[random.next_int_bound(players.len() as i32).unwrap_or(0) as usize];
        if spectator || close_to_village(level, player, 2) {
            return;
        }
        let x = (24 + random.next_int_bound(24).unwrap_or(0))
            * if random.next_boolean() { -1 } else { 1 };
        let z = (24 + random.next_int_bound(24).unwrap_or(0))
            * if random.next_boolean() { -1 } else { 1 };
        let mut pos = [player[0] + x, player[1], player[2] + z];
        if !has_chunks_at(level, pos[0] - 10, pos[2] - 10, pos[0] + 10, pos[2] + 10)
            || !level.can_pillager_patrol_spawn(pos)
        {
            return;
        }
        let group = level.effective_difficulty(pos).ceil() as i32 + 1;
        for i in 0..group {
            pos[1] = level.motion_blocking_no_leaves(pos[0], pos[2]);
            let leader = i == 0;
            if !spawn_member(level, pillager, pos, leader, random, spawn) && leader {
                break;
            }
            pos[0] += random.next_int_bound(5).unwrap_or(0) - random.next_int_bound(5).unwrap_or(0);
            pos[2] += random.next_int_bound(5).unwrap_or(0) - random.next_int_bound(5).unwrap_or(0);
        }
    }
}

/// `PatrolSpawner.spawnPatrolMember`: an empty spawn block for a pillager
/// whose `checkPatrollingMonsterSpawnRules` pass.
fn spawn_member<L: PatrolLevel, R: RandomSource>(
    level: &L,
    pillager: &MobType,
    pos: [i32; 3],
    leader: bool,
    random: &mut R,
    spawn: &mut dyn FnMut(&L, Member, &mut R) -> bool,
) -> bool {
    let [x, y, z] = pos;
    if !spawn_rules::is_valid_empty_spawn(pillager.species, level, x, y, z) {
        return false;
    }
    if !spawn_rules::patrolling_monster_rules(pillager.species, level, Reason::Patrol, x, y, z) {
        return false;
    }
    // EntityType.create checks canSpawn after the patrol's placement predicate.
    if level.peaceful() && !pillager.allowed_in_peaceful() {
        return false;
    }
    spawn(level, Member { pos, leader }, random)
}

/// `PatrollingMonster.findPatrolTarget` from a block position.
pub fn patrol_target(at: [i32; 3], random: &mut impl RandomSource) -> [i32; 3] {
    let dx = -500 + random.next_int_bound(1000).unwrap_or(0);
    let dz = -500 + random.next_int_bound(1000).unwrap_or(0);
    [at[0] + dx, at[1], at[2] + dz]
}

/// The boolean gameplay attributes patrols and raids read
/// (`gameplay/can_pillager_patrol_spawn`, `gameplay/can_start_raid`): the
/// dimension's value, the biome's, then the dimension's timelines (the early
/// game timeline forbids patrols for the first 120,000 ticks of the
/// overworld clock).
#[derive(Clone, Debug, Default)]
pub struct GameplayAttributes {
    /// attribute -> dimension type -> value.
    dimensions: std::collections::HashMap<String, std::collections::HashMap<String, bool>>,
    /// attribute -> biome -> value.
    biomes: std::collections::HashMap<String, std::collections::HashMap<String, bool>>,
    /// attribute -> dimension type -> boolean tracks (keyframes, modifier).
    tracks: std::collections::HashMap<String, std::collections::HashMap<String, Vec<BoolTrack>>>,
}
#[derive(Clone, Debug)]
struct BoolTrack {
    keyframes: Vec<(i64, bool)>,
    period: Option<i64>,
    modifier: String,
}
impl BoolTrack {
    /// `KeyframeTrackSampler.sample` for a value that is not interpolated.
    fn sample(&self, ticks: i64) -> bool {
        let t = match self.period {
            Some(p) if p > 0 => ticks.rem_euclid(p),
            _ => ticks,
        };
        let mut value = self.keyframes[0].1;
        for (at, v) in &self.keyframes {
            if t >= *at {
                value = *v;
            }
        }
        value
    }
    fn apply(&self, base: bool, ticks: i64) -> bool {
        let arg = self.sample(ticks);
        match self.modifier.as_str() {
            "and" => base && arg,
            "or" => base || arg,
            "xor" => base ^ arg,
            "nand" => !(base && arg),
            "nor" => !(base || arg),
            "xnor" => base == arg,
            _ => arg,
        }
    }
}
pub const CAN_PILLAGER_PATROL_SPAWN: &str = "minecraft:gameplay/can_pillager_patrol_spawn";
pub const CAN_START_RAID: &str = "minecraft:gameplay/can_start_raid";
impl GameplayAttributes {
    /// Reads the dimension types, biomes and timelines under `root/data`, and the timeline
    /// tags from `root` and the world's packs (`overlay`).
    pub fn load(
        root: &std::path::Path,
        overlay: &sunrise_world::loot::PackOverlay,
    ) -> std::io::Result<Self> {
        use serde_json::Value;
        let data = root.join("data/minecraft");
        let read = |p: std::path::PathBuf| -> Option<Value> {
            serde_json::from_slice(&std::fs::read(p).ok()?).ok()
        };
        let list = |dir: &str| -> Vec<(String, Value)> {
            let mut out = Vec::new();
            if let Ok(entries) = std::fs::read_dir(data.join(dir)) {
                for e in entries.flatten() {
                    let path = e.path();
                    if let (Some(stem), Some(v)) = (
                        path.file_stem().and_then(|s| s.to_str()).map(str::to_owned),
                        read(path.clone()),
                    ) {
                        out.push((format!("minecraft:{stem}"), v));
                    }
                }
            }
            out
        };
        let keys = [CAN_PILLAGER_PATROL_SPAWN, CAN_START_RAID];
        let mut out = Self::default();
        let timelines: std::collections::HashMap<String, Value> =
            list("timeline").into_iter().collect();
        let tags = sunrise_world::tag_layers::load(root, "timeline", overlay, |id| {
            timelines.contains_key(id)
        })?;
        for (dim, v) in list("dimension_type") {
            for key in keys {
                if let Some(b) = v["attributes"][key].as_bool() {
                    out.dimensions
                        .entry(key.into())
                        .or_default()
                        .insert(dim.clone(), b);
                }
            }
            let mut names = Vec::new();
            let entries = match &v["timelines"] {
                Value::Array(list) => list.as_slice(),
                value => std::slice::from_ref(value),
            };
            for id in entries.iter().filter_map(Value::as_str) {
                if let Some(tag) = id.strip_prefix('#') {
                    if let Ok(tag) = sunrise_core::identifier::Identifier::parse(tag) {
                        names.extend(
                            tags.get(tag.to_string().as_str())
                                .into_iter()
                                .flatten()
                                .cloned(),
                        );
                    }
                } else {
                    names.push(id.to_owned());
                }
            }
            for name in names {
                let Some(t) = timelines.get(&name) else {
                    continue;
                };
                for key in keys {
                    let track = &t["tracks"][key];
                    let Some(frames) = track["keyframes"].as_array() else {
                        continue;
                    };
                    let keyframes: Vec<(i64, bool)> = frames
                        .iter()
                        .filter_map(|f| Some((f["ticks"].as_i64()?, f["value"].as_bool()?)))
                        .collect();
                    if keyframes.is_empty() {
                        continue;
                    }
                    out.tracks
                        .entry(key.into())
                        .or_default()
                        .entry(dim.clone())
                        .or_default()
                        .push(BoolTrack {
                            keyframes,
                            period: t["period_ticks"].as_i64(),
                            modifier: track["modifier"].as_str().unwrap_or("override").to_owned(),
                        });
                }
            }
        }
        for (biome, v) in list("worldgen/biome") {
            for key in keys {
                if let Some(b) = v["attributes"][key].as_bool() {
                    out.biomes
                        .entry(key.into())
                        .or_default()
                        .insert(biome.clone(), b);
                }
            }
        }
        Ok(out)
    }
    /// The attribute in a dimension type and biome at an overworld clock time.
    pub fn value(&self, key: &str, dimension: &str, biome: &str, clock: i64) -> bool {
        let mut v = self
            .dimensions
            .get(key)
            .and_then(|m| m.get(dimension))
            .copied()
            .unwrap_or(true);
        if let Some(b) = self.biomes.get(key).and_then(|m| m.get(biome)) {
            v = *b;
        }
        for t in self
            .tracks
            .get(key)
            .and_then(|m| m.get(dimension))
            .into_iter()
            .flatten()
        {
            v = t.apply(v, clock);
        }
        v
    }
}

#[cfg(test)]
mod tag_cycle_tests {
    use super::*;
    #[test]
    fn an_optional_timeline_cycle_does_not_block_a_required_reference() {
        let root = tempfile::tempdir().unwrap();
        let write = |path: &str, value: serde_json::Value| {
            let path = root.path().join(format!("data/minecraft/{path}.json"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, value.to_string()).unwrap();
        };
        write(
            "tags/timeline/a",
            serde_json::json!({"values":["#minecraft:b"]}),
        );
        write(
            "tags/timeline/b",
            serde_json::json!({"values":[{"id":"#minecraft:a", "required":false}, "minecraft:blocked"]}),
        );
        write(
            "timeline/blocked",
            serde_json::json!({"tracks":{(CAN_PILLAGER_PATROL_SPAWN):{"keyframes":[{"ticks":0,"value":false}]}}}),
        );
        for (dimension, tag) in [("one", "a"), ("two", "b")] {
            write(
                &format!("dimension_type/{dimension}"),
                serde_json::json!({"timelines":format!("#minecraft:{tag}")}),
            );
        }
        let attributes =
            GameplayAttributes::load(root.path(), &sunrise_world::loot::PackOverlay::NONE).unwrap();
        for dimension in ["minecraft:one", "minecraft:two"] {
            assert!(!attributes.value(CAN_PILLAGER_PATROL_SPAWN, dimension, "minecraft:plains", 0));
        }
    }

    /// A data pack's timeline tag file binds like the game's own: the raid code's attribute for a
    /// dimension type that names the tag follows the pack's entry (`TagLoader.load`).
    #[test]
    fn a_pack_timeline_tag_reaches_the_raid_attributes() {
        let root = tempfile::tempdir().unwrap();
        let write = |path: &str, value: serde_json::Value| {
            let path = root.path().join(format!("data/minecraft/{path}.json"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, value.to_string()).unwrap();
        };
        write(
            "timeline/blocked",
            serde_json::json!({"tracks":{(CAN_PILLAGER_PATROL_SPAWN):{"keyframes":[{"ticks":0,"value":false}]}}}),
        );
        write(
            "dimension_type/one",
            serde_json::json!({"timelines":"#minecraft:from_pack"}),
        );
        let attributes = |files: &[(String, Vec<Vec<u8>>)]| {
            GameplayAttributes::load(root.path(), &sunrise_world::loot::PackOverlay::new(files))
                .unwrap()
        };
        let value = |attributes: &GameplayAttributes| {
            attributes.value(
                CAN_PILLAGER_PATROL_SPAWN,
                "minecraft:one",
                "minecraft:plains",
                0,
            )
        };
        assert!(value(&attributes(&[])));
        let files = [(
            "tags/minecraft:timeline/from_pack".to_owned(),
            vec![
                br#"{"values":["minecraft:blocked",{"id":"minecraft:absent","required":false}]}"#
                    .to_vec(),
            ],
        )];
        assert!(!value(&attributes(&files)));
    }
}
