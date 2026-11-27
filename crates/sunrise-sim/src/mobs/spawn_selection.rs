//! `NaturalSpawner`: which mob spawns where and when. The spawn state
//! (`createState`: category counts, the spawn potential's charges, the
//! per-player local caps), the filtered categories and their global and local
//! caps, `spawnForChunk` and `spawnCategoryForPosition` (the three packs of up
//! to four attempts, the distance and respawn checks, `getRandomSpawnMobAt`
//! over `mobsAt`: the nether fortress bounds, structure spawn overrides by
//! piece or bounding box, then the biome's natural mob spawns), the per-type
//! checks of `isValidSpawnPostitionForType` and `isValidPositionForMob`
//! (each class's `checkSpawnRules` walk target value, `checkSpawnObstruction`,
//! `removeWhenFarAway`, cluster and group sizes), and
//! `spawnMobsForChunkGeneration`. Everything reads the level through
//! [`NaturalLevel`] so the game and the differential test world
//! (tools/test_natural_spawn_parity.py) run the same code; random draws follow
//! vanilla's order.
use super::spawn_rules::{self, Reason, SpawnLevel};
use super::{MobType, Species};
use crate::biome::SpawnEntry;
use crate::fast_hash::FxHashMap;
use mod_api::{SpawnCheck, SpawnFlags, SpawnLimits};
use serde_json::Value;
use std::{
    borrow::{Borrow, Cow},
    collections::HashMap,
    fs,
    path::Path,
};
use sunrise_core::{
    geometry::{Aabb, ShapeOp, Vec3, VoxelShape},
    random::RandomSource,
};

pub use crate::biome::{SpawnCost, parse_costs, parse_entries};

thread_local! {
    static ATTEMPT_COUNTS: std::cell::Cell<[u64; 2]> = const { std::cell::Cell::new([0; 2]) };
}
/// Reset or publish once per natural-spawn tick, not once per chunk.
pub fn attempt_counters(publish: bool) {
    if !crate::spans::enabled() {
        return;
    }
    let counts = ATTEMPT_COUNTS.with(|c| c.replace([0; 2]));
    if publish {
        sunrise_perf_format::counter("natural_spawning.attempts", &[]).add(counts[0]);
        sunrise_perf_format::counter("natural_spawning.spawns", &[]).add(counts[1]);
    }
}
fn attempt_count(index: usize) {
    if crate::spans::enabled() {
        ATTEMPT_COUNTS.with(|c| {
            let mut counts = c.get();
            counts[index] += 1;
            c.set(counts);
        });
    }
}

/// `MobCategory`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Monster,
    Creature,
    Ambient,
    Axolotls,
    UndergroundWaterCreature,
    WaterCreature,
    WaterAmbient,
    Misc,
}
impl Category {
    pub const ALL: [Self; 8] = [
        Self::Monster,
        Self::Creature,
        Self::Ambient,
        Self::Axolotls,
        Self::UndergroundWaterCreature,
        Self::WaterCreature,
        Self::WaterAmbient,
        Self::Misc,
    ];
    /// `NaturalSpawner.SPAWNING_CATEGORIES`: all but MISC, in enum order.
    pub const SPAWNING: [Self; 7] = [
        Self::Monster,
        Self::Creature,
        Self::Ambient,
        Self::Axolotls,
        Self::UndergroundWaterCreature,
        Self::WaterCreature,
        Self::WaterAmbient,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Monster => "monster",
            Self::Creature => "creature",
            Self::Ambient => "ambient",
            Self::Axolotls => "axolotls",
            Self::UndergroundWaterCreature => "underground_water_creature",
            Self::WaterCreature => "water_creature",
            Self::WaterAmbient => "water_ambient",
            Self::Misc => "misc",
        }
    }
    pub fn from_name(name: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|c| c.name() == name)
            .unwrap_or(Self::Misc)
    }
    fn index(self) -> usize {
        self as usize
    }
    /// `getMaxInstancesPerChunk` under a level's `limits` (vanilla's are
    /// [`SpawnLimits::VANILLA`]); MISC's is -1, as it never spawns.
    pub fn max(self, limits: &SpawnLimits) -> i64 {
        i64::from(match self {
            Self::Monster => limits.monster,
            Self::Creature => limits.creature,
            Self::Ambient => limits.ambient,
            Self::Axolotls => limits.axolotls,
            Self::UndergroundWaterCreature => limits.underground_water_creature,
            Self::WaterCreature => limits.water_creature,
            Self::WaterAmbient => limits.water_ambient,
            Self::Misc => return -1,
        })
    }
    pub fn friendly(self) -> bool {
        self != Self::Monster
    }
    pub fn persistent(self) -> bool {
        matches!(self, Self::Creature | Self::Misc)
    }
    pub fn despawn_distance(self) -> i32 {
        if self == Self::WaterAmbient { 64 } else { 128 }
    }
}

/// `PathfinderMob.checkSpawnRules`: the class's `getWalkTargetValue` at the
/// spawn position must not be negative (plain `Mob`s accept any position).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WalkTarget {
    NotPathfinder,
    Zero,
    Animal,
    Mooshroom,
    Turtle,
    Monster,
    Giant,
    Guardian,
    Silverfish,
    Strider,
    Hoglin,
    Bee,
    HappyGhast,
}
fn walk_target(s: Species) -> WalkTarget {
    use Species as S;
    use WalkTarget as W;
    match s {
        S::Bat | S::Slime | S::MagmaCube | S::SulfurCube | S::Ghast | S::Phantom => {
            W::NotPathfinder
        }
        // Monster subclasses that override it with 0.
        S::Enderman | S::Creaking | S::Pillager | S::Warden => W::Zero,
        S::Giant => W::Giant,
        S::Guardian | S::ElderGuardian => W::Guardian,
        S::Silverfish => W::Silverfish,
        S::Zombie
        | S::Husk
        | S::Drowned
        | S::ZombieVillager
        | S::ZombifiedPiglin
        | S::Skeleton
        | S::Stray
        | S::Bogged
        | S::Parched
        | S::WitherSkeleton
        | S::Creeper
        | S::Spider
        | S::CaveSpider
        | S::Endermite
        | S::Witch
        | S::Vindicator
        | S::Evoker
        | S::Illusioner
        | S::Ravager
        | S::Piglin
        | S::PiglinBrute
        | S::Zoglin
        | S::Blaze
        | S::Vex
        | S::Breeze
        | S::Wither => W::Monster,
        S::Mooshroom => W::Mooshroom,
        S::Turtle => W::Turtle,
        S::Strider => W::Strider,
        S::Hoglin => W::Hoglin,
        S::Bee => W::Bee,
        S::HappyGhast => W::HappyGhast,
        // Animal subclasses with their own value.
        S::Axolotl | S::Nautilus | S::ZombieNautilus => W::Zero,
        S::Pig
        | S::Cow
        | S::Sheep
        | S::Chicken
        | S::Rabbit
        | S::Horse
        | S::Donkey
        | S::Mule
        | S::Llama
        | S::TraderLlama
        | S::SkeletonHorse
        | S::ZombieHorse
        | S::Camel
        | S::CamelHusk
        | S::Goat
        | S::Panda
        | S::PolarBear
        | S::Fox
        | S::Wolf
        | S::Cat
        | S::Ocelot
        | S::Parrot
        | S::Armadillo
        | S::Frog
        | S::Sniffer => W::Animal,
        // Other PathfinderMobs: water animals, golems, villagers, allays...
        _ => W::Zero,
    }
}
/// Each class's `checkSpawnObstruction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Obstruction {
    /// `Mob`: no liquid in the box and no entity blocking it.
    NoLiquidUnobstructed,
    Unobstructed,
    Ocelot,
    NoLiquid,
}
fn obstruction(s: Species) -> Obstruction {
    use Species as S;
    match s {
        S::Squid
        | S::GlowSquid
        | S::Dolphin
        | S::Axolotl
        | S::Cod
        | S::Salmon
        | S::TropicalFish
        | S::Pufferfish
        | S::Tadpole
        | S::Nautilus
        | S::ZombieNautilus
        | S::Guardian
        | S::ElderGuardian
        | S::Strider
        | S::Drowned => Obstruction::Unobstructed,
        S::Ocelot => Obstruction::Ocelot,
        S::Ravager => Obstruction::NoLiquid,
        // ZombifiedPiglin's override is the Mob default's two tests; iron
        // golems and wardens never spawn naturally.
        _ => Obstruction::NoLiquidUnobstructed,
    }
}
/// `Mob.removeWhenFarAway` of a freshly created mob.
fn removed_when_far(s: Species) -> bool {
    use Species as S;
    match s {
        // Animal, golems, villagers, traders, allays and wardens never;
        // cats and ocelots only past 2400 ticks; chickens as jockeys.
        S::Axolotl
        | S::Cod
        | S::Salmon
        | S::TropicalFish
        | S::Pufferfish
        | S::Tadpole
        | S::CamelHusk
        | S::ZombieHorse
        | S::Hoglin
        | S::Nautilus
        | S::ZombieNautilus => true,
        S::Villager
        | S::WanderingTrader
        | S::IronGolem
        | S::SnowGolem
        | S::Shulker
        | S::Allay
        | S::Warden => false,
        _ => !is_animal(s),
    }
}
fn is_animal(s: Species) -> bool {
    use Species as S;
    matches!(walk_target(s), WalkTarget::Animal)
        || matches!(
            s,
            S::Axolotl
                | S::Nautilus
                | S::ZombieNautilus
                | S::Mooshroom
                | S::Turtle
                | S::Strider
                | S::Hoglin
                | S::Bee
                | S::HappyGhast
        )
}
/// `Mob.getMaxSpawnClusterSize`.
pub fn max_cluster_size(s: Species) -> i32 {
    use Species as S;
    match s {
        S::Horse
        | S::Donkey
        | S::Mule
        | S::Llama
        | S::TraderLlama
        | S::SkeletonHorse
        | S::ZombieHorse
        | S::Camel
        | S::CamelHusk => 6,
        S::Cod | S::TropicalFish | S::Pufferfish | S::Tadpole | S::Wolf => 8,
        S::Salmon => 5,
        S::Ghast | S::HappyGhast | S::Pillager => 1,
        _ => 4,
    }
}
/// `EntityType.canSpawnFarFromPlayer`: creatures, misc, pillagers, shulkers.
fn spawns_far_from_player(s: Species, category: Category) -> bool {
    matches!(category, Category::Creature | Category::Misc)
        || matches!(s, Species::Pillager | Species::Shulker)
}

/// What natural spawning reads from a level beyond the spawn rules.
pub trait NaturalLevel: SpawnLevel {
    /// `MobSpawnEvent.PositionCheck`: what server mods answer for the new mob
    /// of type `entity` at `at`, described by `pending`; `Default` leaves the
    /// checks to vanilla.
    fn position_check(
        &self,
        _entity: &str,
        _reason: Reason,
        _at: [f64; 3],
        _pending: Pending<'_>,
    ) -> SpawnCheck {
        SpawnCheck::Default
    }
    /// A spawner's configured entity data, loaded once before its position check
    /// (`BaseSpawner.serverTick`'s `EntityType.loadEntityRecursive`): the level keeps the
    /// loaded tree, lends its root to the check and hands the same tree on to finalization and
    /// admission. False when the data cannot be loaded.
    fn load_configured(
        &self,
        _entity: &sunrise_core::nbt::Compound,
        _id: &str,
        _at: [f64; 3],
        _yaw: f32,
    ) -> bool {
        true
    }
    /// The position check just asked refused the mob: what the level made
    /// for it is dropped.
    fn position_refused(&self) {}
    /// Whether server mods hear `potential_spawns`.
    fn spawn_hooks(&self) -> bool {
        false
    }
    /// `LevelEvent.PotentialSpawns`: server mods change the possible spawns
    /// at a position.
    fn potential_spawns(
        &self,
        _category: Category,
        _pos: [i32; 3],
        _entries: &mut Vec<SpawnEntry>,
    ) {
    }
    fn min_y(&self) -> i32;
    /// `getMaxY()`: the highest buildable block.
    fn max_y(&self) -> i32;
    /// `DimensionType.ambientLight`.
    fn ambient_light(&self) -> f32;
    fn has_ceiling(&self) -> bool;
    /// The registered living entity type.
    fn mob_type(&self, entity: &str) -> Option<&MobType>;
    /// The `natural_mob_spawns` lists by category at a position.
    fn biome_spawns(&self, x: i32, y: i32, z: i32) -> Option<&HashMap<String, Vec<SpawnEntry>>>;
    /// The `natural_mob_spawns` spawn cost of a type at a position.
    fn spawn_cost(&self, entity: &str, x: i32, y: i32, z: i32) -> Option<SpawnCost>;
    /// The `creature_world_gen_spawn_probability` attribute at a position.
    fn creature_probability(&self, x: i32, y: i32, z: i32) -> f32;
    /// Every structure whose references reach the position's chunk and
    /// whose id `wanted` takes, in the chunk's reference order.
    fn structures_at(
        &self,
        x: i32,
        y: i32,
        z: i32,
        wanted: &dyn Fn(&str) -> bool,
    ) -> Vec<StructureAt>;
    fn structure_spawns(&self) -> &StructureSpawns;
    /// Where the players `getNearestPlayer(x, y, z, -1, false)` measures to
    /// are: every one that is not a spectator.
    fn nearest_player_positions(&self) -> &[[f64; 3]];
    /// Those of [`Self::nearest_player_positions`] within `extent` of
    /// `(x, z)` on both axes, and possibly others: the caller tests each. A
    /// level with many players indexes them ([`PlayerSquares`]).
    fn player_positions_near(&self, x: f64, z: f64, extent: f64, out: &mut Vec<[f64; 3]>) {
        let _ = (x, z, extent);
        out.extend_from_slice(self.nearest_player_positions());
    }
    /// The world spawn point, when it is in this dimension.
    fn respawn(&self) -> Option<[i32; 3]>;
    /// `ServerLevel.canSpawnEntitiesInChunk`.
    fn can_spawn_in_chunk(&self, cx: i32, cz: i32) -> bool;
    /// `noCollision(null, box)`: blocks under an empty collision context and
    /// entities that can be collided with.
    fn no_collision(&self, bounds: Aabb) -> bool;
    /// `isUnobstructed(entity)`: no entity blocking building overlaps, including
    /// entities already accepted by this spawn pass.
    fn unobstructed(&self, bounds: Aabb) -> bool;
    /// The `spawn_mobs` game rule.
    fn spawn_mobs(&self) -> bool;
    /// `getHeight(heightmap, x, z)`.
    fn heightmap(&self, kind: Heightmap, x: i32, z: i32) -> i32;
    /// `isPathfindable(PathComputationType.LAND)` of the block.
    fn pathfindable_land(&self, x: i32, y: i32, z: i32) -> bool;
}

/// The heightmap types `SpawnPlacements` registers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Heightmap {
    WorldSurface,
    OceanFloor,
    MotionBlocking,
    MotionBlockingNoLeaves,
}
impl Heightmap {
    pub(super) fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "WORLD_SURFACE" => Self::WorldSurface,
            "OCEAN_FLOOR" => Self::OceanFloor,
            "MOTION_BLOCKING" => Self::MotionBlocking,
            "MOTION_BLOCKING_NO_LEAVES" => Self::MotionBlockingNoLeaves,
            _ => return None,
        })
    }
}

impl From<mod_api::Heightmap> for Heightmap {
    fn from(heightmap: mod_api::Heightmap) -> Self {
        match heightmap {
            mod_api::Heightmap::WorldSurface => Self::WorldSurface,
            mod_api::Heightmap::OceanFloor => Self::OceanFloor,
            mod_api::Heightmap::MotionBlocking => Self::MotionBlocking,
            _ => Self::MotionBlockingNoLeaves,
        }
    }
}

/// `Heightmap.Types`' predicates: WORLD_SURFACE (not air), OCEAN_FLOOR
/// (`#blocks_motion_in_heightmap`), MOTION_BLOCKING (that or a fluid) and
/// MOTION_BLOCKING_NO_LEAVES (`#blocks_motion_in_heightmap_no_leaves` or a fluid).
pub fn heightmap_blocks<L: SpawnLevel>(l: &L, kind: Heightmap, x: i32, y: i32, z: i32) -> bool {
    let state = l.state(x, y, z);
    let f = l.facts().get(state);
    let fluid = f.fluid != crate::blocks::Fluid::None;
    match kind {
        Heightmap::WorldSurface => !l.facts().is_air(state),
        Heightmap::OceanFloor => l.block_tag("minecraft:blocks_motion_in_heightmap", &f.name),
        Heightmap::MotionBlocking => {
            fluid || l.block_tag("minecraft:blocks_motion_in_heightmap", &f.name)
        }
        Heightmap::MotionBlockingNoLeaves => {
            fluid || l.block_tag("minecraft:blocks_motion_in_heightmap_no_leaves", &f.name)
        }
    }
}
/// `LevelReader.getLightLevelDependentMagicValue`.
fn magic_light<L: NaturalLevel>(l: &L, x: i32, y: i32, z: i32) -> f32 {
    let raw = l
        .block_light(x, y, z)
        .max(l.sky_light(x, y, z) - l.sky_darken());
    let v = raw as f32 / 15.0;
    let curved = v / (4.0 - 3.0 * v);
    curved + l.ambient_light() * (1.0 - curved)
}
fn cost_from_light<L: NaturalLevel>(l: &L, x: i32, y: i32, z: i32) -> f32 {
    magic_light(l, x, y, z) - 0.5
}
fn name_at<L: NaturalLevel>(l: &L, x: i32, y: i32, z: i32) -> &str {
    &l.facts().get(l.state(x, y, z)).name
}
fn fluid_at<L: NaturalLevel>(l: &L, x: i32, y: i32, z: i32) -> crate::blocks::Fluid {
    l.facts().get(l.state(x, y, z)).fluid
}
/// `checkSpawnRules(level, NATURAL)` of a freshly created mob at a block.
pub(crate) fn mob_spawn_rules<L: NaturalLevel>(s: Species, l: &L, x: i32, y: i32, z: i32) -> bool {
    use crate::blocks::Fluid;
    let value = match walk_target(s) {
        WalkTarget::NotPathfinder | WalkTarget::Zero => 0.0,
        WalkTarget::Animal => {
            if name_at(l, x, y - 1, z) == "minecraft:grass_block" {
                10.0
            } else {
                cost_from_light(l, x, y, z)
            }
        }
        WalkTarget::Mooshroom => {
            if name_at(l, x, y - 1, z) == "minecraft:mycelium" {
                10.0
            } else {
                cost_from_light(l, x, y, z)
            }
        }
        WalkTarget::Turtle => {
            if fluid_at(l, x, y, z) == Fluid::Water
                || l.block_tag("minecraft:sand", name_at(l, x, y - 1, z))
            {
                10.0
            } else {
                cost_from_light(l, x, y, z)
            }
        }
        WalkTarget::Monster => -cost_from_light(l, x, y, z),
        WalkTarget::Giant => cost_from_light(l, x, y, z),
        WalkTarget::Guardian => {
            if fluid_at(l, x, y, z) == Fluid::Water {
                10.0 + cost_from_light(l, x, y, z)
            } else {
                -cost_from_light(l, x, y, z)
            }
        }
        WalkTarget::Silverfish => {
            if infested_host(name_at(l, x, y - 1, z)) {
                10.0
            } else {
                -cost_from_light(l, x, y, z)
            }
        }
        // A new strider is not in lava yet.
        WalkTarget::Strider => {
            if fluid_at(l, x, y, z) == Fluid::Lava {
                10.0
            } else {
                0.0
            }
        }
        // A new hoglin remembers no repellent.
        WalkTarget::Hoglin => {
            if name_at(l, x, y - 1, z) == "minecraft:crimson_nylium" {
                10.0
            } else {
                0.0
            }
        }
        WalkTarget::Bee => {
            if l.facts().is_air(l.state(x, y, z)) {
                10.0
            } else {
                0.0
            }
        }
        WalkTarget::HappyGhast => {
            let air = |dy| l.facts().is_air(l.state(x, y + dy, z));
            if !air(0) {
                0.0
            } else if air(-1) && !air(-2) {
                10.0
            } else {
                5.0
            }
        }
    };
    value >= 0.0
}
/// `InfestedBlock.isCompatibleHostBlock`.
pub fn infested_host(block: &str) -> bool {
    matches!(
        block,
        "minecraft:stone"
            | "minecraft:cobblestone"
            | "minecraft:stone_bricks"
            | "minecraft:mossy_stone_bricks"
            | "minecraft:cracked_stone_bricks"
            | "minecraft:chiseled_stone_bricks"
            | "minecraft:deepslate"
    )
}
/// `LevelReader.containsAnyLiquid`: the blocks from the box's minimum corner
/// to the block holding its maximum corner.
pub fn contains_any_liquid<L: SpawnLevel>(l: &L, b: Aabb) -> bool {
    let (lo, hi) = (b.min_position(), b.max_position());
    let (x0, y0, z0) = (
        lo.x.floor() as i32,
        lo.y.floor() as i32,
        lo.z.floor() as i32,
    );
    let (x1, y1, z1) = (
        hi.x.floor() as i32,
        hi.y.floor() as i32,
        hi.z.floor() as i32,
    );
    (x0..=x1).any(|x| {
        (y0..=y1).any(|y| {
            (z0..=z1).any(|z| l.facts().get(l.state(x, y, z)).fluid != crate::blocks::Fluid::None)
        })
    })
}
/// `EntityDimensions.makeBoundingBox`: the half width is a float.
pub fn spawn_box(x: f64, y: f64, z: f64, width: f32, height: f32) -> Aabb {
    spawn_box_at([x, y, z], f64::from(width / 2.0), f64::from(height))
}

pub(super) fn spawn_box_at([x, y, z]: [f64; 3], half_width: f64, height: f64) -> Aabb {
    Aabb::new(
        Vec3::new(x - half_width, y, z - half_width),
        Vec3::new(x + half_width, y + height, z + half_width),
    )
}
/// `EntityType.spawnDimensionsScale`: cube mobs are checked for room at
/// their largest spawn size.
fn spawn_dimensions_scale(s: Species) -> f32 {
    match s {
        Species::Slime | Species::MagmaCube => 4.0,
        Species::SulfurCube => 2.0,
        _ => 1.0,
    }
}
/// The half-width and height used by `EntityType.getSpawnAABB`.
pub(super) fn spawn_extents(s: Option<Species>, width: f32, height: f32) -> (f64, f64) {
    let scale = s.map_or(1.0, spawn_dimensions_scale);
    (f64::from(scale * width / 2.0), f64::from(scale * height))
}

/// `EntityType.getSpawnAABB`.
pub fn spawn_aabb(s: Species, x: f64, y: f64, z: f64, width: f32, height: f32) -> Aabb {
    let (half, height) = spawn_extents(Some(s), width, height);
    spawn_box_at([x, y, z], half, height)
}
/// The mob a position check is about, which the site has not created yet
/// (vanilla creates it before the check).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pending<'a> {
    /// The yaw the site creates it with, drawn before the check.
    pub yaw: f32,
    /// Configured spawner data, loaded into the candidate before the check.
    pub configured: Option<&'a sunrise_core::nbt::Compound>,
}
/// `Mob.checkSpawnRules` and `checkSpawnObstruction` of a freshly created
/// mob, unless a mod forces the answer (`MobSpawnEvent.PositionCheck`); then
/// `vanilla` does not run.
pub fn position_ok<L: NaturalLevel>(
    l: &L,
    entity: &str,
    reason: Reason,
    at: [f64; 3],
    pending: Pending<'_>,
    vanilla: impl FnOnce() -> bool,
) -> bool {
    let ok = match l.position_check(entity, reason, at, pending) {
        SpawnCheck::Succeed => true,
        SpawnCheck::Fail => false,
        _ => vanilla(),
    };
    if !ok {
        l.position_refused();
    }
    ok
}
/// `EntityGetter.isUnobstructed`'s shape test after selecting an entity with
/// `blocksBuilding`. A broad-phase AABB overlap alone is not an obstruction.
pub fn blocks_spawn_shape(shape: &VoxelShape, query: Aabb, blocker: Aabb) -> bool {
    blocker.intersects(query)
        && VoxelShape::from_box(blocker).is_ok_and(|other| {
            shape
                .join_is_not_empty(&other, ShapeOp::AND)
                .unwrap_or(false)
        })
}

/// `checkSpawnObstruction` of a freshly created mob with the given box.
/// Generation skips the entity query (`WorldGenRegion.getEntities` is empty).
pub fn spawn_obstruction<L: NaturalLevel>(
    s: Species,
    l: &L,
    bounds: Aabb,
    check_entities: bool,
    x: i32,
    y: i32,
    z: i32,
) -> bool {
    let unobstructed = || !check_entities || l.unobstructed(bounds);
    match obstruction(s) {
        Obstruction::NoLiquidUnobstructed => !contains_any_liquid(l, bounds) && unobstructed(),
        Obstruction::Unobstructed => unobstructed(),
        Obstruction::NoLiquid => !contains_any_liquid(l, bounds),
        Obstruction::Ocelot => {
            if unobstructed() && !contains_any_liquid(l, bounds) {
                if y < l.sea_level() {
                    return false;
                }
                let below = name_at(l, x, y - 1, z);
                if below == "minecraft:grass_block" || l.block_tag("minecraft:leaves", below) {
                    return true;
                }
            }
            false
        }
    }
}

/// A structure's spawn override for one category.
#[derive(Clone, Debug)]
pub struct Override {
    /// `bounding_box: piece` (else the whole structure's box).
    pub piece: bool,
    pub spawns: Vec<SpawnEntry>,
}
/// Every structure's `spawn_overrides`, by structure id then category.
#[derive(Clone, Debug, Default)]
pub struct StructureSpawns {
    /// Fx-hashed: natural spawning asks for every structure reaching every
    /// position it tries.
    pub overrides: FxHashMap<String, FxHashMap<String, Override>>,
}
impl StructureSpawns {
    /// Read a fresh generation, including mod namespaces and nested structure ids.
    pub fn load(root: &Path) -> Self {
        fn insert(
            overrides: &mut FxHashMap<String, FxHashMap<String, Override>>,
            id: String,
            v: &Value,
        ) {
            let map: FxHashMap<String, Override> = v["spawn_overrides"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(category, o)| {
                    (
                        category.clone(),
                        Override {
                            piece: o["bounding_box"].as_str() == Some("piece"),
                            spawns: parse_entries(&o["spawns"]),
                        },
                    )
                })
                .collect();
            overrides.insert(id, map);
        }
        let mut overrides = FxHashMap::default();
        if let Some(Value::Object(entries)) = fs::read(root.join("worldgen/structures.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        {
            for (id, value) in entries {
                if let Some(value) = value
                    .as_str()
                    .and_then(|text| serde_json::from_str::<Value>(text).ok())
                {
                    insert(&mut overrides, id, &value);
                }
            }
        }
        for ns in fs::read_dir(root.join("data"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let dir = ns.path().join("worldgen/structure");
            let Ok(files) = crate::json_folder::list(&dir) else {
                continue;
            };
            for (name, path) in files {
                let Some(v) = fs::read(path)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                else {
                    continue;
                };
                insert(
                    &mut overrides,
                    format!("{}:{}", ns.file_name().to_string_lossy(), name),
                    &v,
                );
            }
        }
        Self { overrides }
    }
    pub fn get(&self, structure: &str, category: &str) -> Option<&Override> {
        self.overrides.get(structure)?.get(category)
    }
}

/// A structure whose references reach a position's chunk.
#[derive(Clone, Debug)]
pub struct StructureAt {
    pub id: String,
    /// A piece of one of its starts holds the position.
    pub in_piece: bool,
    /// One of its starts' bounding boxes holds the position.
    pub in_bounds: bool,
}

/// `NetherFortressStructure.FORTRESS_ENEMIES`.
pub fn fortress_enemies() -> &'static [SpawnEntry] {
    static ENEMIES: std::sync::LazyLock<Vec<SpawnEntry>> = std::sync::LazyLock::new(|| {
        let e = |entity: &str, weight, min, max, constant| SpawnEntry {
            entity: entity.to_owned(),
            weight,
            min,
            max,
            constant,
        };
        vec![
            e("minecraft:blaze", 10, 2, 3, false),
            e("minecraft:zombified_piglin", 5, 4, 4, true),
            e("minecraft:wither_skeleton", 8, 5, 5, true),
            e("minecraft:skeleton", 2, 5, 5, true),
            e("minecraft:magma_cube", 3, 4, 4, true),
        ]
    });
    &ENEMIES
}

/// `NaturalSpawner.mobsAt`: the fortress bounds on nether bricks, then
/// `ChunkGenerator.getMobsAt` (the first referenced structure whose override
/// for the category holds the position), else the biome's list, as mods
/// change it. Borrowed from the level when no mod hears it: a spawn attempt
/// looks it up up to twice.
pub fn mobs_at<L: NaturalLevel>(
    l: &L,
    category: Category,
    x: i32,
    y: i32,
    z: i32,
) -> Cow<'_, [SpawnEntry]> {
    let entries = vanilla_mobs_at(l, category, x, y, z);
    if !l.spawn_hooks() {
        return Cow::Borrowed(entries);
    }
    let mut entries = entries.to_vec();
    l.potential_spawns(category, [x, y, z], &mut entries);
    Cow::Owned(entries)
}

fn vanilla_mobs_at<L: NaturalLevel>(
    l: &L,
    category: Category,
    x: i32,
    y: i32,
    z: i32,
) -> &[SpawnEntry] {
    // Only a fortress or a structure overriding this category's spawns can
    // change the list, so only theirs are looked up (the references reaching
    // most chunks are mineshafts, which override nothing).
    let overrides = l.structure_spawns();
    let monster = category == Category::Monster;
    let structures = l.structures_at(x, y, z, &|id| {
        (monster && id == "minecraft:fortress") || overrides.get(id, category.name()).is_some()
    });
    if category == Category::Monster
        && structures
            .iter()
            .any(|s| s.id == "minecraft:fortress" && s.in_bounds)
        && name_at(l, x, y - 1, z) == "minecraft:nether_bricks"
    {
        return fortress_enemies();
    }
    for s in &structures {
        if let Some(o) = overrides.get(&s.id, category.name())
            && (if o.piece { s.in_piece } else { s.in_bounds })
        {
            return &o.spawns;
        }
    }
    l.biome_spawns(x, y, z)
        .and_then(|b| b.get(category.name()))
        .map_or(&[], Vec::as_slice)
}

fn total_weight(list: &[SpawnEntry]) -> i64 {
    list.iter().map(|e| i64::from(e.weight)).sum()
}

fn has_weight(list: &[SpawnEntry]) -> bool {
    total_weight(list) > 0
}

/// `WeightedList.getRandom`.
pub fn weighted_pick<'a>(
    list: &'a [SpawnEntry],
    random: &mut impl RandomSource,
) -> Option<&'a SpawnEntry> {
    let total = total_weight(list);
    if total <= 0 {
        return None;
    }
    // Keep vanilla's exact draw for representable totals. Larger modded lists use
    // one unsigned long modulo the total, a bounded, slightly biased fallback.
    let mut selection = if total <= i64::from(i32::MAX) {
        i64::from(random.next_int_bound(total as i32).ok()?)
    } else {
        ((random.next_long() as u64) % total as u64) as i64
    };
    for e in list {
        selection -= i64::from(e.weight);
        if selection < 0 {
            return Some(e);
        }
    }
    None
}

/// `SpawnerData.count().sample(random)`: a constant draws nothing.
pub fn sample_count(entry: &SpawnEntry, random: &mut impl RandomSource) -> u32 {
    if entry.constant {
        entry.min
    } else {
        let span = entry.max.saturating_sub(entry.min) as i32 + 1;
        entry.min + random.next_int_bound(span).unwrap_or(0) as u32
    }
}
/// `SpawnerData.equals`: the type and the count provider (the weight is not
/// part of it; a constant and a one-value uniform differ).
fn same_spawner(a: &SpawnEntry, b: &SpawnEntry) -> bool {
    a.entity == b.entity && a.constant == b.constant && a.min == b.min && a.max == b.max
}

/// An entity counted by `NaturalSpawner.createState`.
#[derive(Clone, Debug)]
pub struct Counted<'a> {
    pub entity: &'a str,
    pub category: Category,
    /// `blockPosition()`.
    pub block: [i32; 3],
    pub is_mob: bool,
    /// `isPersistenceRequired() || requiresCustomPersistence()`.
    pub persistent: bool,
}
/// The players by the eight-chunk square their chunk is in: a chunk finds the
/// players within 8 chunks of it (the natural spawn chunk tracker's range,
/// and more than the 128 blocks spawning asks for) without a look at every
/// player. With players spread over the map, each spawning chunk and each
/// counted mob looked at all of them.
#[derive(Clone, Debug, Default)]
pub struct PlayerCells {
    cells: FxHashMap<(i32, i32), Vec<(u32, i32, i32)>>,
}
impl PlayerCells {
    /// From each player's block x and z, in the players' order.
    pub fn new(positions: impl IntoIterator<Item = (f64, f64)>) -> Self {
        let mut cells: FxHashMap<(i32, i32), Vec<(u32, i32, i32)>> = FxHashMap::default();
        for (i, (x, z)) in positions.into_iter().enumerate() {
            let (cx, cz) = ((x.floor() as i32) >> 4, (z.floor() as i32) >> 4);
            cells
                .entry((cx >> 3, cz >> 3))
                .or_default()
                .push((i as u32, cx, cz));
        }
        Self { cells }
    }
    /// The players whose chunk is within 8 chunks of (cx, cz) on both axes,
    /// in the players' order.
    pub fn within_8(&self, cx: i32, cz: i32, out: &mut Vec<usize>) {
        out.clear();
        for kx in cx.saturating_sub(8) >> 3..=cx.saturating_add(8) >> 3 {
            for kz in cz.saturating_sub(8) >> 3..=cz.saturating_add(8) >> 3 {
                let Some(players) = self.cells.get(&(kx, kz)) else {
                    continue;
                };
                out.extend(
                    players
                        .iter()
                        .filter(|&&(_, px, pz)| (px - cx).abs().max((pz - cz).abs()) <= 8)
                        .map(|&(i, _, _)| i as usize),
                );
            }
        }
        out.sort_unstable();
    }
}

/// A player as the spawn caps see it.
#[derive(Clone, Copy, Debug)]
pub struct SpawnPlayer {
    pub position: [f64; 3],
    pub spectator: bool,
}

/// `NaturalSpawner.SpawnState` with its `PotentialCalculator` and
/// `LocalMobCapCalculator`.
#[derive(Clone, Debug)]
pub struct SpawnState {
    spawnable_chunks: i32,
    /// The level's `getMaxInstancesPerChunk` values.
    limits: SpawnLimits,
    counts: [i32; 8],
    charges: Vec<([i32; 3], f64)>,
    players: Vec<SpawnPlayer>,
    cells: PlayerCells,
    /// Each chunk asked about: its players' run in `near_list`.
    players_near: FxHashMap<(i32, i32), (u32, u32)>,
    near_list: Vec<usize>,
    /// The mobs counted near each player (`playerMobCounts`), by category.
    player_counts: Vec<[i32; 8]>,
    /// Scratch for [`Nearest`] and `players_near`, kept to be reused.
    nearby: Vec<[f64; 3]>,
    tracked: Vec<usize>,
    /// The position and charge of the last `can_spawn`; `after_spawn` always
    /// follows it for the same type and position.
    last_checked: Option<([i32; 3], f64)>,
}
/// The allocations of a [`SpawnState`], handed on from one pass to the next
/// (`SpawnState::new_in`, `SpawnState::into_parts`). Only looked up by key or
/// indexed, never iterated, so what a pass left in them is not observable.
#[derive(Debug, Default)]
pub struct SpawnBuffers {
    charges: Vec<([i32; 3], f64)>,
    players_near: FxHashMap<(i32, i32), (u32, u32)>,
    near_list: Vec<usize>,
    player_counts: Vec<[i32; 8]>,
    nearby: Vec<[f64; 3]>,
    tracked: Vec<usize>,
}
impl SpawnState {
    /// `getSpawnableChunkCount` and `getMobCategoryCounts` (by
    /// [`Category::ALL`]), for the debug overlay's spawn counts.
    pub fn counts(&self) -> (i32, [i32; 8]) {
        (self.spawnable_chunks, self.counts)
    }
    /// `NaturalSpawner.createState`: every non-persistent entity of a
    /// spawning category in a loaded chunk adds its biome's spawn charge, its
    /// category count and (for mobs) the local caps of the players near it.
    pub fn new<'a, L: NaturalLevel, I>(
        l: &L,
        spawnable_chunks: i32,
        players: Vec<SpawnPlayer>,
        entities: I,
        chunk_loaded: impl Fn(i32, i32) -> bool,
    ) -> Self
    where
        I: IntoIterator,
        I::Item: std::borrow::Borrow<Counted<'a>>,
    {
        let buffers = SpawnBuffers::default();
        Self::new_in(
            l,
            spawnable_chunks,
            players,
            buffers,
            entities,
            chunk_loaded,
        )
    }
    /// [`SpawnState::new`] over the allocations of an earlier pass.
    pub fn new_in<'a, L: NaturalLevel, I>(
        l: &L,
        spawnable_chunks: i32,
        players: Vec<SpawnPlayer>,
        buffers: SpawnBuffers,
        entities: I,
        chunk_loaded: impl Fn(i32, i32) -> bool,
    ) -> Self
    where
        I: IntoIterator,
        I::Item: std::borrow::Borrow<Counted<'a>>,
    {
        let mut state = Self::uncounted_in(spawnable_chunks, players, buffers);
        for e in entities {
            let e = e.borrow();
            if e.is_mob && e.persistent || e.category == Category::Misc {
                continue;
            }
            let [x, y, z] = e.block;
            if !chunk_loaded(x >> 4, z >> 4) {
                continue;
            }
            if let Some(cost) = l.spawn_cost(e.entity, x, y, z) {
                state.add_charge(e.block, cost.charge);
            }
            if e.is_mob {
                state.add_local(x >> 4, z >> 4, e.category);
            }
            state.counts[e.category.index()] += 1;
        }
        state
    }
    /// The state before any entity is counted, over the allocations of an earlier pass.
    fn uncounted_in(
        spawnable_chunks: i32,
        players: Vec<SpawnPlayer>,
        mut buffers: SpawnBuffers,
    ) -> Self {
        buffers.charges.clear();
        buffers.players_near.clear();
        buffers.near_list.clear();
        buffers.player_counts.clear();
        buffers.player_counts.resize(players.len(), [0; 8]);
        Self {
            spawnable_chunks,
            limits: SpawnLimits::VANILLA,
            counts: [0; 8],
            charges: buffers.charges,
            player_counts: buffers.player_counts,
            cells: PlayerCells::new(players.iter().map(|p| (p.position[0], p.position[2]))),
            players,
            players_near: buffers.players_near,
            near_list: buffers.near_list,
            nearby: buffers.nearby,
            tracked: buffers.tracked,
            last_checked: None,
        }
    }
    #[cfg(test)]
    fn uncounted(spawnable_chunks: i32, players: Vec<SpawnPlayer>) -> Self {
        Self::uncounted_in(spawnable_chunks, players, SpawnBuffers::default())
    }
    /// The state with the level's own spawn limits in place of vanilla's.
    pub fn with_limits(mut self, limits: SpawnLimits) -> Self {
        self.limits = limits;
        self
    }
    /// Reclaims the player buffer and the allocations for the next pass.
    pub fn into_parts(self) -> (Vec<SpawnPlayer>, SpawnBuffers) {
        let buffers = SpawnBuffers {
            charges: self.charges,
            players_near: self.players_near,
            near_list: self.near_list,
            player_counts: self.player_counts,
            nearby: self.nearby,
            tracked: self.tracked,
        };
        (self.players, buffers)
    }
    fn add_charge(&mut self, pos: [i32; 3], charge: f64) {
        if charge != 0.0 {
            self.charges.push((pos, charge));
        }
    }
    /// `ChunkMap.getPlayersCloseForSpawning`: none past the natural spawn
    /// chunk tracker's range of 8 chunks from every player, else the
    /// non-spectators within 128 blocks of the chunk's centre.
    fn players_near(&mut self, cx: i32, cz: i32) -> std::ops::Range<usize> {
        let (list, players, cells) = (&mut self.near_list, &self.players, &self.cells);
        let tracked = &mut self.tracked;
        let (start, len) = *self.players_near.entry((cx, cz)).or_insert_with(|| {
            let start = list.len() as u32;
            // Every player within 128 blocks of the chunk's centre stands in
            // a chunk within 8 of it: those 8 chunks hold every answer.
            cells.within_8(cx, cz, tracked);
            if !tracked.is_empty() {
                let (x, z) = (f64::from(cx * 16 + 8), f64::from(cz * 16 + 8));
                list.extend(tracked.iter().copied().filter(|&i| {
                    let p = &players[i];
                    let (dx, dz) = (x - p.position[0], z - p.position[2]);
                    !p.spectator && dx * dx + dz * dz < 16384.0
                }));
            }
            (start, list.len() as u32 - start)
        });
        start as usize..(start + len) as usize
    }
    fn add_local(&mut self, cx: i32, cz: i32, category: Category) {
        for i in self.players_near(cx, cz) {
            self.player_counts[self.near_list[i]][category.index()] += 1;
        }
    }
    /// `canSpawnForCategoryGlobal`, in wide arithmetic: a mod's limit times
    /// the chunks may pass `i32`, which vanilla's limits never do, so they
    /// divide as vanilla's `int`s.
    pub fn can_spawn_global(&self, category: Category) -> bool {
        let max = category.max(&self.limits) * i64::from(self.spawnable_chunks) / 289;
        i64::from(self.counts[category.index()]) < max
    }
    /// `canSpawnForCategoryLocal` (`LocalMobCapCalculator.canSpawn`).
    pub fn can_spawn_local(&mut self, category: Category, cx: i32, cz: i32) -> bool {
        // A player with nothing counted yet can spawn while the cap is above
        // zero, as vanilla's always are.
        let near = self.players_near(cx, cz);
        let max = category.max(&self.limits);
        self.near_list[near]
            .iter()
            .any(|&p| i64::from(self.player_counts[p][category.index()]) < max)
    }
    /// `getFilteredSpawningCategories`.
    pub fn filtered_categories(
        &self,
        spawn_enemies: bool,
        spawn_persistent: bool,
    ) -> Vec<Category> {
        Category::SPAWNING
            .into_iter()
            .filter(|&c| {
                (spawn_enemies || c.friendly())
                    && (spawn_persistent || !c.persistent())
                    && self.can_spawn_global(c)
            })
            .collect()
    }
    /// The categories a level's spawn pass tries: `filtered_categories` with
    /// the `spawn_monsters` rule and the level's `flags` (Sunrise's, set by
    /// mods) both letting monsters through, and the other categories only
    /// while `flags.friendlies` is on. Both flags on is vanilla's choice.
    pub fn policy_categories(
        &self,
        flags: SpawnFlags,
        spawn_monsters: bool,
        spawn_persistent: bool,
    ) -> Vec<Category> {
        let mut categories =
            self.filtered_categories(flags.monsters && spawn_monsters, spawn_persistent);
        categories.retain(|c| flags.friendlies || !c.friendly());
        categories
    }
    /// `SpawnState.canSpawn`: the spawn potential's energy change must stay
    /// within the type's budget where it has a spawn cost.
    fn can_spawn<L: NaturalLevel>(&mut self, l: &L, entity: &str, pos: [i32; 3]) -> bool {
        match l.spawn_cost(entity, pos[0], pos[1], pos[2]) {
            None => {
                self.last_checked = Some((pos, 0.0));
                true
            }
            Some(cost) => {
                self.last_checked = Some((pos, cost.charge));
                self.energy_change(pos, cost.charge) <= cost.energy_budget
            }
        }
    }
    /// `PotentialCalculator.getPotentialEnergyChange`.
    fn energy_change(&self, pos: [i32; 3], charge: f64) -> f64 {
        if charge == 0.0 {
            return 0.0;
        }
        let mut change = 0.0;
        for (p, c) in &self.charges {
            let d = [
                f64::from(p[0] - pos[0]),
                f64::from(p[1] - pos[1]),
                f64::from(p[2] - pos[2]),
            ];
            let dist = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            change += if dist == 0.0 {
                f64::INFINITY
            } else {
                c / dist.sqrt()
            };
        }
        change * charge
    }
    /// `SpawnState.afterSpawn`.
    fn after_spawn(
        &mut self,
        spawn_cost: impl FnOnce(&str, [i32; 3]) -> Option<SpawnCost>,
        entity: &str,
        category: Category,
        pos: [i32; 3],
    ) {
        let charge = match &self.last_checked {
            Some((p, c)) if *p == pos => *c,
            _ => spawn_cost(entity, pos).map_or(0.0, |c| c.charge),
        };
        self.add_charge(pos, charge);
        self.counts[category.index()] += 1;
        self.add_local(pos[0] >> 4, pos[2] >> 4, category);
    }
}

/// What a spawn pass did, in vanilla's order (for differential tests).
#[derive(Clone, Debug, PartialEq)]
pub enum Event<'a> {
    /// `getFilteredSpawningCategories`.
    Categories(Vec<Category>),
    /// `canSpawnForCategoryLocal` for the chunk.
    Local(Category, bool),
    /// `getRandomPosWithin`'s start.
    Start(Category, [i32; 3]),
    /// An attempt at a position (the nearest player lookup).
    Try(i32, i32),
    /// The type passed `isValidSpawnPostitionForType`; the spawn state's
    /// potential test answered.
    Test(&'a str, [i32; 3], bool),
    /// A mob spawned.
    Spawn(&'a str, [f64; 3], f32),
}
/// A mob the spawner is about to add: the level runs `finalizeSpawn`,
/// makes it visible to the pass's entity queries and returns its group data.
#[derive(Clone, Debug)]
pub struct SpawnRequest<'a> {
    pub entity: &'a str,
    pub species: Species,
    pub position: [f64; 3],
    pub yaw: f32,
    /// Mobs of this group spawned before (the `SpawnGroupData`'s size).
    pub group_index: u32,
    pub reason: Reason,
}
#[derive(Clone, Copy, Debug)]
pub struct Spawned {
    /// `TropicalFish.isSchool`.
    pub school: bool,
}

/// `NaturalSpawner.spawnForChunk` for the categories already filtered.
#[allow(clippy::too_many_arguments)]
pub fn spawn_for_chunk<L: NaturalLevel, R: RandomSource + Clone>(
    l: &L,
    random: &mut R,
    state: &mut SpawnState,
    categories: &[Category],
    chunk: (i32, i32),
    spawn: &mut dyn FnMut(&L, &SpawnRequest, &mut R) -> Option<Spawned>,
    cluster_size: &mut dyn FnMut(&L, &SpawnRequest, i32) -> i32,
    trace: &mut dyn FnMut(Event<'_>),
) {
    for &category in categories {
        let local = state.can_spawn_local(category, chunk.0, chunk.1);
        trace(Event::Local(category, local));
        if local {
            spawn_category_for_chunk(
                l,
                random,
                state,
                category,
                chunk,
                spawn,
                cluster_size,
                trace,
            );
        }
    }
}
/// `NaturalSpawner.spawnCategoryForChunk`: `getRandomPosWithin` (a column of
/// the chunk, a height up to one above the world surface), then the position's pass.
pub fn spawn_category_for_chunk<L: NaturalLevel, R: RandomSource + Clone>(
    l: &L,
    random: &mut R,
    state: &mut SpawnState,
    category: Category,
    chunk: (i32, i32),
    spawn: &mut dyn FnMut(&L, &SpawnRequest, &mut R) -> Option<Spawned>,
    cluster_size: &mut dyn FnMut(&L, &SpawnRequest, i32) -> i32,
    trace: &mut dyn FnMut(Event<'_>),
) {
    let start = spawn_start(l, random, chunk);
    trace(Event::Start(category, start));
    if start[1] > l.min_y() {
        spawn_category_for_position(
            l,
            random,
            state,
            category,
            chunk,
            start,
            spawn,
            cluster_size,
            trace,
        );
    }
}

/// NaturalSpawner.getRandomPosWithin.
pub fn spawn_start(
    l: &impl NaturalLevel,
    random: &mut impl RandomSource,
    chunk: (i32, i32),
) -> [i32; 3] {
    let x = chunk.0 * 16 + random.next_int_bound(16).unwrap_or(0);
    let z = chunk.1 * 16 + random.next_int_bound(16).unwrap_or(0);
    let top = l.heightmap(Heightmap::WorldSurface, x, z);
    let min = l.min_y();
    let y = min + random.next_int_bound(top - min + 1).unwrap_or(0);
    [x, y, z]
}

/// `NaturalSpawner.spawnCategoryForPosition`, for probes which model insertion locally.
#[allow(clippy::too_many_arguments)]
pub fn spawn_category_for_position<L: NaturalLevel, R: RandomSource + Clone>(
    l: &L,
    random: &mut R,
    state: &mut SpawnState,
    category: Category,
    chunk: (i32, i32),
    start: [i32; 3],
    spawn: &mut dyn FnMut(&L, &SpawnRequest, &mut R) -> Option<Spawned>,
    cluster_size: &mut dyn FnMut(&L, &SpawnRequest, i32) -> i32,
    trace: &mut dyn FnMut(Event<'_>),
) {
    let Some(mut cluster) = SpawnCluster::new(l, state, category, chunk, start) else {
        return;
    };
    while let Some(candidate) = cluster.next_candidate(l, random, state, trace) {
        let request = cluster.request(candidate);
        let group_index = request.group_index;
        let Some(result) = spawn(l, &request, random) else {
            continue;
        };
        cluster.after_spawn(
            |entity, [x, y, z]| l.spawn_cost(entity, x, y, z),
            state,
            candidate,
            candidate.block,
            trace,
        );
        let request = SpawnRequest {
            group_index,
            ..cluster.request(candidate)
        };
        let size = cluster_size(l, &request, max_cluster_size(candidate.species));
        cluster.check_limit(candidate, size, result.school);
    }
    cluster.finish(state);
}

/// The natural spawner's search at one start, resumable between the position/finalize checks
/// (which read the level) and the insertion attempt (which needs the owning level).
pub struct SpawnCluster {
    start: [i32; 3],
    category: Category,
    chunk: (i32, i32),
    nearest: Nearest,
    count: u32,
    groups: u8,
    group_index: u32,
    current: Option<(SpawnEntry, Option<Category>, [i32; 3])>,
    x: i32,
    z: i32,
    attempt: i32,
    max: i32,
}

#[derive(Clone, Copy)]
pub struct SpawnCandidate {
    block: [i32; 3],
    position: [f64; 3],
    yaw: f32,
    species: Species,
    category: Category,
}

impl SpawnCandidate {
    /// `Mob.getMaxSpawnClusterSize` for this candidate's species.
    pub fn vanilla_limit(self) -> i32 {
        max_cluster_size(self.species)
    }
}

impl SpawnCluster {
    pub fn new(
        l: &impl NaturalLevel,
        state: &mut SpawnState,
        category: Category,
        chunk: (i32, i32),
        start: [i32; 3],
    ) -> Option<Self> {
        let [x, y, z] = start;
        if l.mob_data().state(l.state(x, y, z)).flags & super::data::REDSTONE_CONDUCTOR != 0 {
            return None;
        }
        Some(Self {
            start,
            category,
            chunk,
            nearest: Nearest::new(start, std::mem::take(&mut state.nearby)),
            count: 0,
            groups: 0,
            group_index: 0,
            current: None,
            x,
            z,
            attempt: 0,
            max: 0,
        })
    }

    pub fn next_candidate(
        &mut self,
        l: &impl NaturalLevel,
        random: &mut (impl RandomSource + Clone),
        state: &mut SpawnState,
        trace: &mut dyn FnMut(Event<'_>),
    ) -> Option<SpawnCandidate> {
        let [x0, y, z0] = self.start;
        loop {
            if self.attempt >= self.max {
                if self.groups >= 3 {
                    return None;
                }
                self.groups += 1;
                (self.x, self.z) = (x0, z0);
                self.current = None;
                self.group_index = 0;
                self.attempt = 0;
                self.max = (random.next_float() * 4.0).ceil() as i32;
                continue;
            }
            self.attempt += 1;
            self.x += random.next_int_bound(6).unwrap_or(0) - random.next_int_bound(6).unwrap_or(0);
            self.z += random.next_int_bound(6).unwrap_or(0) - random.next_int_bound(6).unwrap_or(0);
            let (x, z) = (self.x, self.z);
            let (xx, zz) = (f64::from(x) + 0.5, f64::from(z) + 0.5);
            trace(Event::Try(x, z));
            attempt_count(0);
            let Some(distance) = self.nearest.distance_sqr(l, xx, f64::from(y), zz) else {
                continue;
            };
            if !right_distance(l, self.chunk, [x, y, z], distance) {
                continue;
            }
            if self.current.is_none() {
                let list = if self.category == Category::WaterAmbient
                    && l.biome_tag("minecraft:reduce_water_ambient_spawns", x, y, z)
                    && random.next_float() < 0.98
                {
                    None
                } else {
                    // Only the chosen entry needs to outlive the read-only level view.
                    weighted_pick(&mobs_at(l, self.category, x, y, z), random).cloned()
                };
                let Some(entry) = list else {
                    self.attempt = self.max;
                    continue;
                };
                self.max = sample_count(&entry, random) as i32;
                let type_category = l
                    .mob_type(&entry.entity)
                    .map(|t| Category::from_name(&t.category));
                self.current = Some((entry, type_category, [x, y, z]));
            }
            let (entry, type_category, picked_at) =
                self.current.as_ref().unwrap_or_else(|| unreachable!());
            let Some(type_category) = *type_category else {
                continue;
            };
            let Some(kind) = l.mob_type(&entry.entity) else {
                continue;
            };
            // The list the entry was picked from holds it where it was picked.
            let listed = *picked_at == [x, y, z];
            if !valid_position_for_type(
                l,
                random,
                self.category,
                entry,
                kind,
                type_category,
                listed,
                [x, y, z],
                distance,
            ) {
                continue;
            }
            let (species, width, height) = (kind.species, kind.width as f32, kind.height as f32);
            let passed = state.can_spawn(l, &entry.entity, [x, y, z]);
            trace(Event::Test(&entry.entity, [x, y, z], passed));
            if !passed {
                continue;
            }
            // getMobForSpawn (a type the game cannot create, which would end
            // the pass, never passes isValidSpawnPostitionForType).
            let yaw = random.next_float() * 360.0;
            let bounds = spawn_box(xx, f64::from(y), zz, width, height);
            let despawn = f64::from(type_category.despawn_distance().pow(2));
            let far = distance > despawn && removed_when_far(species);
            if far
                || !position_ok(
                    l,
                    &entry.entity,
                    Reason::Natural,
                    [xx, f64::from(y), zz],
                    Pending {
                        yaw,
                        configured: None,
                    },
                    || {
                        mob_spawn_rules(species, l, x, y, z)
                            && spawn_obstruction(species, l, bounds, true, x, y, z)
                    },
                )
            {
                continue;
            }
            return Some(SpawnCandidate {
                block: [x, y, z],
                position: [xx, f64::from(y), zz],
                yaw,
                species,
                category: type_category,
            });
        }
    }

    pub fn request(&self, candidate: SpawnCandidate) -> SpawnRequest<'_> {
        let (entry, _, _) = self
            .current
            .as_ref()
            .expect("candidate belongs to the selected spawn group");
        SpawnRequest {
            entity: &entry.entity,
            species: candidate.species,
            position: candidate.position,
            yaw: candidate.yaw,
            group_index: self.group_index,
            reason: Reason::Natural,
        }
    }

    /// Account for an attempted insertion, even if the level declined admission.
    pub fn after_spawn(
        &mut self,
        spawn_cost: impl FnOnce(&str, [i32; 3]) -> Option<SpawnCost>,
        state: &mut SpawnState,
        candidate: SpawnCandidate,
        block: [i32; 3],
        trace: &mut dyn FnMut(Event<'_>),
    ) {
        attempt_count(1);
        self.group_index += 1;
        self.count += 1;
        self.nearest.ready = false;
        let (entry, _, _) = self
            .current
            .as_ref()
            .expect("candidate belongs to the selected spawn group");
        state.after_spawn(spawn_cost, &entry.entity, candidate.category, block);
        trace(Event::Spawn(
            &entry.entity,
            candidate.position,
            candidate.yaw,
        ));
    }

    /// Compare the cluster size only after the spawn callback.
    pub fn check_limit(&mut self, candidate: SpawnCandidate, limit: i32, school: bool) {
        if i64::from(self.count) >= i64::from(limit) {
            self.groups = 3;
            self.attempt = self.max;
        } else if candidate.species == Species::TropicalFish && !school {
            self.attempt = self.max;
        }
    }

    pub fn finish(self, state: &mut SpawnState) {
        state.nearby = self.nearest.nearby;
    }
}
/// `getNearestPlayer(x, y, z, -1, false)`'s squared distance for the
/// attempts of one start. The spawn checks only compare it with 24² and the
/// despawn distances (128² at most), so it is measured only to the players
/// that can be within 128 blocks of an attempt near the start: when none is,
/// the nearest reads as infinitely far, which every check answers as it
/// answers the real distance. An attempt that wanders farther than
/// `MARGIN` from the start measures every player.
struct Nearest {
    /// The players that can be within 128 blocks, once an attempt asked.
    nearby: Vec<[f64; 3]>,
    /// Whether `nearby` holds the players for this start.
    ready: bool,
    start: (f64, f64),
}
impl Nearest {
    const REACH: f64 = 128.0;
    const MARGIN: f64 = 64.0;
    /// Over `buffer`'s allocation (the contents are dropped).
    fn new(start: [i32; 3], buffer: Vec<[f64; 3]>) -> Self {
        Self {
            nearby: buffer,
            ready: false,
            start: (f64::from(start[0]) + 0.5, f64::from(start[2]) + 0.5),
        }
    }
    fn distance_sqr<L: NaturalLevel>(&mut self, l: &L, x: f64, y: f64, z: f64) -> Option<f64> {
        let all = l.nearest_player_positions();
        if all.is_empty() {
            return None;
        }
        let distance = |p: &[f64; 3]| (p[0] - x).powi(2) + (p[1] - y).powi(2) + (p[2] - z).powi(2);
        let (sx, sz) = self.start;
        if (x - sx).abs() > Self::MARGIN || (z - sz).abs() > Self::MARGIN {
            return all.iter().map(distance).reduce(f64::min);
        }
        // The others are farther than REACH, across, from every point within
        // MARGIN of the start. Every one of them is within MARGIN + REACH of
        // the start on both axes: a level with many players hands over only
        // those (the nearest is the same in any order).
        if !self.ready {
            self.ready = true;
            self.nearby.clear();
            l.player_positions_near(sx, sz, Self::MARGIN + Self::REACH, &mut self.nearby);
            self.nearby.retain(|p| {
                let dx = ((p[0] - sx).abs() - Self::MARGIN).max(0.0);
                let dz = ((p[2] - sz).abs() - Self::MARGIN).max(0.0);
                dx * dx + dz * dz <= Self::REACH * Self::REACH
            });
        }
        Some(
            self.nearby
                .iter()
                .map(distance)
                .fold(f64::INFINITY, f64::min),
        )
    }
}
/// Player positions by the 256-block square they stand in, for
/// [`NaturalLevel::player_positions_near`]: every spawn start asks for the
/// players near it, and a server's players are spread over the map.
pub struct PlayerSquares {
    squares: FxHashMap<(i64, i64), Vec<[f64; 3]>>,
    /// Positions too far out (or not numbers) for a square: always handed over.
    unsquared: Vec<[f64; 3]>,
}
impl PlayerSquares {
    const SIDE: f64 = 256.0;
    /// Further out than any world border.
    const FAR: f64 = 1.0e9;
    pub fn new(positions: &[[f64; 3]]) -> Self {
        let mut squares = FxHashMap::<(i64, i64), Vec<[f64; 3]>>::default();
        let mut unsquared = Vec::new();
        for &p in positions {
            if p[0].abs() < Self::FAR && p[2].abs() < Self::FAR {
                squares.entry(Self::square(p[0], p[2])).or_default().push(p);
            } else {
                unsquared.push(p);
            }
        }
        Self { squares, unsquared }
    }
    fn square(x: f64, z: f64) -> (i64, i64) {
        (
            (x / Self::SIDE).floor() as i64,
            (z / Self::SIDE).floor() as i64,
        )
    }
    /// Every position within `extent` (at most a square's side) of `(x, z)`
    /// on both axes, and some others.
    pub fn near(&self, x: f64, z: f64, extent: f64, out: &mut Vec<[f64; 3]>) {
        out.extend_from_slice(&self.unsquared);
        if !(x.abs() < Self::FAR && z.abs() < Self::FAR && extent <= Self::SIDE) {
            out.extend(self.squares.values().flatten().copied());
            return;
        }
        let (sx, sz) = Self::square(x, z);
        for dx in -1..=1 {
            for dz in -1..=1 {
                if let Some(square) = self.squares.get(&(sx + dx, sz + dz)) {
                    out.extend_from_slice(square);
                }
            }
        }
    }
}
/// `isRightDistanceToPlayerAndSpawnPoint`.
fn right_distance<L: NaturalLevel>(l: &L, chunk: (i32, i32), pos: [i32; 3], distance: f64) -> bool {
    if distance <= 576.0 {
        return false;
    }
    if let Some(r) = l.respawn() {
        let d = [
            f64::from(r[0]) + 0.5 - (f64::from(pos[0]) + 0.5),
            f64::from(r[1]) + 0.5 - f64::from(pos[1]),
            f64::from(r[2]) + 0.5 - (f64::from(pos[2]) + 0.5),
        ];
        if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < 24.0 * 24.0 {
            return false;
        }
    }
    let (cx, cz) = (pos[0] >> 4, pos[2] >> 4);
    (cx, cz) == chunk || l.can_spawn_in_chunk(cx, cz)
}
/// `isValidSpawnPostitionForType`, for the entry's type (`mob_type`).
/// `listed`: the position is where the entry was picked from `mobsAt`.
#[allow(clippy::too_many_arguments)]
fn valid_position_for_type<L: NaturalLevel, R: RandomSource + Clone>(
    l: &L,
    random: &mut R,
    category: Category,
    entry: &SpawnEntry,
    kind: &MobType,
    type_category: Category,
    listed: bool,
    pos: [i32; 3],
    distance: f64,
) -> bool {
    let (species, width, height) = (kind.species, kind.width as f32, kind.height as f32);
    if type_category == Category::Misc {
        return false;
    }
    let despawn = f64::from(type_category.despawn_distance().pow(2));
    if !spawns_far_from_player(species, type_category) && distance > despawn {
        return false;
    }
    let [x, y, z] = pos;
    let hooked = l.spawn_hooks();
    let in_list = || {
        mobs_at(l, category, x, y, z)
            .iter()
            .any(|e| same_spawner(e, entry))
    };
    // `canSpawnMobAt` comes first in vanilla, and mods hear each of its
    // lookups. Without them it and `isSpawnPositionOk` only read the level,
    // so their order cannot change the answer: the few block reads of the
    // placement turn most positions away before the spawn list (structures,
    // biome) is looked up again.
    if hooked && !in_list() {
        return false;
    }
    if !spawn_rules::is_spawn_position_ok(&entry.entity, species, l, x, y, z) {
        return false;
    }
    if !hooked && !listed && !in_list() {
        return false;
    }
    if !spawn_rules::check_spawn_rules(kind.into(), l, Reason::Natural, random, pos) {
        return false;
    }
    l.no_collision(spawn_aabb(
        species,
        f64::from(x) + 0.5,
        f64::from(y),
        f64::from(z) + 0.5,
        width,
        height,
    ))
}

/// What chunk generation spawning did (for differential tests).
#[derive(Clone, Debug, PartialEq)]
pub enum GenEvent {
    /// An attempt at a column (the heightmap lookup).
    Column(i32, i32),
    Spawn(String, [f64; 3], f32),
}
/// `NaturalSpawner.spawnMobsForChunkGeneration`: creature packs placed on
/// the chunk's surface. `rules` is the region's random, which the spawn
/// predicates and `spawn` (finalizeSpawn) draw from.
#[allow(clippy::too_many_arguments)]
pub fn spawn_mobs_for_chunk_generation<
    L: NaturalLevel,
    R: RandomSource,
    Q: RandomSource + Clone,
>(
    l: &L,
    source: [i32; 3],
    chunk: (i32, i32),
    random: &mut R,
    rules: &mut Q,
    spawn: &mut dyn FnMut(&L, &SpawnRequest, &mut Q) -> bool,
    trace: &mut dyn FnMut(GenEvent),
) {
    if !l.spawn_mobs() {
        return;
    }
    let [sx, sy, sz] = source;
    let mobs = l
        .biome_spawns(sx, sy, sz)
        .and_then(|b| b.get("creature"))
        .cloned()
        .unwrap_or_default();
    if !has_weight(&mobs) {
        return;
    }
    let probability = l.creature_probability(sx, sy, sz);
    let (xo, zo) = (chunk.0 * 16, chunk.1 * 16);
    while random.next_float() < probability {
        let Some(entry) = weighted_pick(&mobs, random) else {
            continue;
        };
        let count = sample_count(entry, random);
        let mut group_index = 0u32;
        let mut x = xo + random.next_int_bound(16).unwrap_or(0);
        let mut z = zo + random.next_int_bound(16).unwrap_or(0);
        let (start_x, start_z) = (x, z);
        let kind = l.mob_type(&entry.entity);
        for _ in 0..count {
            let mut success = false;
            let mut attempts = 0;
            while !success && attempts < 4 {
                attempts += 1;
                let Some(kind) = kind else {
                    break;
                };
                let (species, width, height) =
                    (kind.species, kind.width as f32, kind.height as f32);
                let pos = top_non_colliding(l, &entry.entity, x, z);
                trace(GenEvent::Column(x, z));
                if spawn_rules::is_spawn_position_ok(
                    &entry.entity,
                    species,
                    l,
                    pos[0],
                    pos[1],
                    pos[2],
                ) {
                    let fx = f64::from(x).clamp(
                        f64::from(xo) + f64::from(width),
                        f64::from(xo) + 16.0 - f64::from(width),
                    );
                    let fz = f64::from(z).clamp(
                        f64::from(zo) + f64::from(width),
                        f64::from(zo) + 16.0 - f64::from(width),
                    );
                    let fy = f64::from(pos[1]);
                    let (bx, bz) = (fx.floor() as i32, fz.floor() as i32);
                    if !l.no_collision(spawn_aabb(species, fx, fy, fz, width, height))
                        || !spawn_rules::check_spawn_rules(
                            kind.into(),
                            l,
                            Reason::ChunkGeneration,
                            rules,
                            [bx, pos[1], bz],
                        )
                    {
                        continue;
                    }
                    let yaw = random.next_float() * 360.0;
                    let bounds = spawn_box(fx, fy, fz, width, height);
                    if position_ok(
                        l,
                        &entry.entity,
                        Reason::ChunkGeneration,
                        [fx, fy, fz],
                        Pending {
                            yaw,
                            configured: None,
                        },
                        || {
                            mob_spawn_rules(species, l, bx, pos[1], bz)
                                && spawn_obstruction(species, l, bounds, false, bx, pos[1], bz)
                        },
                    ) {
                        let request = SpawnRequest {
                            entity: &entry.entity,
                            species,
                            position: [fx, fy, fz],
                            yaw,
                            group_index,
                            reason: Reason::ChunkGeneration,
                        };
                        if spawn(l, &request, rules) {
                            group_index += 1;
                            trace(GenEvent::Spawn(entry.entity.clone(), [fx, fy, fz], yaw));
                            success = true;
                        }
                    }
                }
                x += random.next_int_bound(5).unwrap_or(0) - random.next_int_bound(5).unwrap_or(0);
                z += random.next_int_bound(5).unwrap_or(0) - random.next_int_bound(5).unwrap_or(0);
                while x < xo || x >= xo + 16 || z < zo || z >= zo + 16 {
                    x = start_x + random.next_int_bound(5).unwrap_or(0)
                        - random.next_int_bound(5).unwrap_or(0);
                    z = start_z + random.next_int_bound(5).unwrap_or(0)
                        - random.next_int_bound(5).unwrap_or(0);
                }
            }
        }
    }
}
/// `getTopNonCollidingPos`: the type's heightmap, under the ceiling in
/// dimensions that have one, then the placement type's adjustment.
fn top_non_colliding<L: NaturalLevel>(l: &L, entity: &str, x: i32, z: i32) -> [i32; 3] {
    let heightmap = l.mob_data().heightmap(entity);
    let mut y = l.heightmap(heightmap, x, z);
    if l.has_ceiling() {
        loop {
            y -= 1;
            if l.facts().is_air(l.state(x, y, z)) {
                break;
            }
        }
        loop {
            y -= 1;
            if !(l.facts().is_air(l.state(x, y, z)) && y > l.min_y()) {
                break;
            }
        }
    }
    if l.mob_data().placement(entity) == super::data::Placement::OnGround
        && l.pathfindable_land(x, y - 1, z)
    {
        y -= 1;
    }
    [x, y, z]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_weights_do_not_overflow_u32() {
        let entry = SpawnEntry {
            entity: "minecraft:zombie".into(),
            weight: i32::MAX as u32,
            min: 1,
            max: 1,
            constant: true,
        };
        assert_eq!(
            total_weight(&[entry.clone(), entry.clone(), entry]),
            3 * i64::from(i32::MAX)
        );
        assert!(has_weight(&[
            entry_for_test(),
            entry_for_test(),
            entry_for_test()
        ]));
        assert!(has_weight(&[entry_for_test(), entry_for_test()]));
        assert!(!has_weight(&[]));
        let mut zero = entry_for_test();
        zero.weight = 0;
        assert!(!has_weight(&[zero]));
        let mut random = sunrise_core::random::LegacyRandom::new(42);
        assert!(
            weighted_pick(
                &[entry_for_test(), entry_for_test(), entry_for_test()],
                &mut random
            )
            .is_some()
        );
    }

    #[test]
    fn in_range_picks_match_old_draw() {
        for weights in [[1, 1, 1], [1, 3, 7], [5, 0, 2]] {
            let list: Vec<_> = weights
                .into_iter()
                .enumerate()
                .map(|(i, weight)| {
                    let mut entry = entry_for_test();
                    entry.entity = i.to_string();
                    entry.weight = weight;
                    entry
                })
                .collect();
            for seed in 0..16 {
                let mut actual = sunrise_core::random::LegacyRandom::new(seed);
                let mut old = sunrise_core::random::LegacyRandom::new(seed);
                let total: i64 = list.iter().map(|e| i64::from(e.weight)).sum();
                let mut selection = i64::from(old.next_int_bound(total as i32).unwrap());
                let expected = list
                    .iter()
                    .find(|e| {
                        selection -= i64::from(e.weight);
                        selection < 0
                    })
                    .unwrap();
                assert_eq!(
                    weighted_pick(&list, &mut actual).unwrap().entity,
                    expected.entity
                );
                assert_eq!(actual.next_long(), old.next_long());
            }
        }
    }

    fn entry_for_test() -> SpawnEntry {
        SpawnEntry {
            entity: "minecraft:zombie".into(),
            weight: i32::MAX as u32,
            min: 1,
            max: 1,
            constant: true,
        }
    }

    /// A state over `chunks` spawnable chunks with one player standing in chunk (0, 0).
    fn one_player(chunks: i32, limits: SpawnLimits) -> SpawnState {
        let player = SpawnPlayer {
            position: [8.0, 64.0, 8.0],
            spectator: false,
        };
        SpawnState::uncounted(chunks, vec![player]).with_limits(limits)
    }

    /// Vanilla's limits: one player's 289 chunks hold 70 monsters, so a count of 69 still
    /// spawns and 70 doesn't; 144 chunks hold 70 * 144 / 289 = 34 (34.88 rounded down); and a
    /// player near the chunk with 70 monsters around refuses it locally.
    #[test]
    fn vanilla_limits_cap_as_vanilla_does() {
        let mut state = one_player(289, SpawnLimits::VANILLA);
        state.counts[Category::Monster.index()] = 69;
        assert!(state.can_spawn_global(Category::Monster));
        state.counts[Category::Monster.index()] = 70;
        assert!(!state.can_spawn_global(Category::Monster));
        let mut fewer = one_player(144, SpawnLimits::VANILLA);
        fewer.counts[Category::Monster.index()] = 33;
        assert!(fewer.can_spawn_global(Category::Monster));
        fewer.counts[Category::Monster.index()] = 34;
        assert!(!fewer.can_spawn_global(Category::Monster));
        // Ten creatures, fifteen ambient mobs.
        state.counts[Category::Creature.index()] = 10;
        state.counts[Category::Ambient.index()] = 14;
        assert!(!state.can_spawn_global(Category::Creature));
        assert!(state.can_spawn_global(Category::Ambient));
        state.player_counts[0][Category::Monster.index()] = 69;
        assert!(state.can_spawn_local(Category::Monster, 0, 0));
        state.player_counts[0][Category::Monster.index()] = 70;
        assert!(!state.can_spawn_local(Category::Monster, 0, 0));
    }

    /// A level's own limit moves both caps: a limit of 0 stops the category with nothing
    /// counted, and a raised one lets the 70th monster through. A mod's largest limit over the
    /// most chunks is counted without overflowing.
    #[test]
    fn a_levels_limits_move_both_caps() {
        let none = SpawnLimits {
            monster: 0,
            ..SpawnLimits::VANILLA
        };
        let mut state = one_player(289, none);
        assert!(!state.can_spawn_global(Category::Monster));
        assert!(!state.can_spawn_local(Category::Monster, 0, 0));
        assert!(
            state.can_spawn_global(Category::Creature),
            "the others keep theirs"
        );
        let raised = SpawnLimits {
            monster: 100,
            ..SpawnLimits::VANILLA
        };
        let mut state = one_player(289, raised);
        state.counts[Category::Monster.index()] = 99;
        state.player_counts[0][Category::Monster.index()] = 99;
        assert!(state.can_spawn_global(Category::Monster));
        assert!(state.can_spawn_local(Category::Monster, 0, 0));
        state.counts[Category::Monster.index()] = 100;
        assert!(!state.can_spawn_global(Category::Monster));
        let most = SpawnLimits {
            monster: u32::MAX,
            ..SpawnLimits::VANILLA
        };
        let mut state = one_player(i32::MAX, most);
        state.counts[Category::Monster.index()] = i32::MAX;
        state.player_counts[0][Category::Monster.index()] = i32::MAX;
        assert!(state.can_spawn_global(Category::Monster));
        assert!(state.can_spawn_local(Category::Monster, 0, 0));
    }

    /// The flags take categories away from vanilla's choice and never add one back: the
    /// `spawn_monsters` rule off keeps monsters out whatever the flag says.
    #[test]
    fn a_levels_flags_take_categories_away() {
        use Category as C;
        let state = one_player(289, SpawnLimits::VANILLA);
        let flags = |monsters, friendlies| SpawnFlags {
            monsters,
            friendlies,
        };
        let friendly = vec![
            C::Creature,
            C::Ambient,
            C::Axolotls,
            C::UndergroundWaterCreature,
            C::WaterCreature,
            C::WaterAmbient,
        ];
        let mut all = vec![C::Monster];
        all.extend(friendly.iter().copied());
        assert_eq!(state.policy_categories(flags(true, true), true, true), all);
        assert_eq!(
            state.policy_categories(flags(false, true), true, true),
            friendly
        );
        assert_eq!(
            state.policy_categories(flags(true, true), false, true),
            friendly
        );
        assert_eq!(
            state.policy_categories(flags(true, false), true, true),
            [C::Monster]
        );
        assert_eq!(state.policy_categories(flags(false, false), true, true), []);
        // Creatures still wait for the persistent pass.
        let without_creatures: Vec<_> = all.into_iter().filter(|&c| c != C::Creature).collect();
        assert_eq!(
            state.policy_categories(flags(true, true), true, false),
            without_creatures
        );
    }

    /// Attempts and spawns counted while spans are on reach the perf counters once published, and a reset publishes nothing.
    #[test]
    fn attempt_counters_publish_what_the_tick_counted() {
        let attempts = sunrise_perf_format::counter("natural_spawning.attempts", &[]);
        let spawns = sunrise_perf_format::counter("natural_spawning.spawns", &[]);
        let before = (attempts.get(), spawns.get());
        crate::spans::enable(true);
        attempt_counters(false);
        for _ in 0..7 {
            attempt_count(0);
        }
        for _ in 0..3 {
            attempt_count(1);
        }
        attempt_counters(true);
        attempt_counters(true);
        crate::spans::enable(false);
        assert_eq!((attempts.get() - before.0, spawns.get() - before.1), (7, 3));
    }

    /// The grid finds the players whose chunk is within 8 of a chunk on both
    /// axes, in their order, as a look at every player does.
    #[test]
    fn player_cells_find_what_a_scan_finds() {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = |m: f64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 1_000_000) as f64 / 1_000_000.0 * 2.0 * m - m
        };
        // Spread over 20 000 blocks, with clusters and players on chunk and cell edges.
        let mut players: Vec<(f64, f64)> =
            (0..300).map(|_| (next(10_000.0), next(10_000.0))).collect();
        players.extend([
            (0.0, 0.0),
            (-0.5, 127.9),
            (128.0, -128.0),
            (-1.0e-9, 16.0),
            (2047.99, -2048.0),
        ]);
        let cells = PlayerCells::new(players.iter().copied());
        let chunk = |(x, z): (f64, f64)| ((x.floor() as i32) >> 4, (z.floor() as i32) >> 4);
        let mut found = Vec::new();
        for &(x, z) in players.iter().step_by(7) {
            let (pcx, pcz) = chunk((x, z));
            for (cx, cz) in [
                (pcx, pcz),
                (pcx + 8, pcz - 8),
                (pcx + 9, pcz),
                (pcx - 3, pcz + 12),
                (0, 0),
            ] {
                cells.within_8(cx, cz, &mut found);
                let scan: Vec<usize> = (0..players.len())
                    .filter(|&i| {
                        let (px, pz) = chunk(players[i]);
                        (px - cx).abs().max((pz - cz).abs()) <= 8
                    })
                    .collect();
                assert_eq!(found, scan, "chunk ({cx}, {cz})");
            }
        }
    }

    /// The squares hand back every player within the extent of a point on
    /// both axes, also for points on square edges and players past the
    /// border; a nearest-player search over them finds what a full one does.
    #[test]
    fn player_squares_keep_every_near_player() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = |m: f64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 1_000_000) as f64 / 1_000_000.0 * 2.0 * m - m
        };
        let mut players: Vec<[f64; 3]> = (0..200)
            .map(|_| [next(3_000.0), next(100.0), next(3_000.0)])
            .collect();
        players.extend([
            [0.0, 64.0, 0.0],
            [256.0, 64.0, -256.0],
            [-1.0e-9, 64.0, 255.999],
            [3.0e9, 64.0, 0.0],
        ]);
        let squares = PlayerSquares::new(&players);
        let mut near = Vec::new();
        let points = players.iter().map(|p| (p[0] + 100.0, p[2] - 200.0)).chain([
            (0.0, 0.0),
            (255.5, -0.5),
            (2.0e9, 0.0),
        ]);
        for (x, z) in points {
            for extent in [24.0, 160.0, 256.0, 300.0] {
                near.clear();
                squares.near(x, z, extent, &mut near);
                for p in &players {
                    if (p[0] - x).abs() <= extent && (p[2] - z).abs() <= extent {
                        assert!(near.contains(p), "{p:?} near ({x}, {z}) within {extent}");
                    }
                }
                let nearest = |list: &[[f64; 3]]| {
                    list.iter()
                        .map(|p| (p[0] - x).powi(2) + (p[2] - z).powi(2))
                        .filter(|d| *d <= extent * extent)
                        .fold(f64::INFINITY, f64::min)
                };
                assert_eq!(
                    nearest(&near),
                    nearest(&players),
                    "({x}, {z}) within {extent}"
                );
            }
        }
    }
}
