//! `SpawnPlacements`: each entity type's placement type
//! (`SpawnPlacementTypes.isSpawnPositionOk`) and spawn predicate
//! (`checkSpawnRules`), read through [`SpawnLevel`] so the game's world and a
//! differential test world answer the same questions. Random draws follow
//! vanilla's order.
use super::{MobType, Species, data};
use crate::{
    blocks::{BlockFacts, Fluid},
    chunk::Chunk,
};
use mod_api::{BlockPos, SpawnPlacementContext, SpawnPlacementLevel};
use sunrise_core::random::{RandomSource, slime_chunk_random};
use sunrise_world::{dimension_type::DimensionFacts, registry::BlockStateId};

/// `EntitySpawnReason`: the mod API's own enum.
pub use mod_api::SpawnReason as Reason;
/// `EntitySpawnReason.isSpawner`.
pub fn is_spawner(reason: Reason) -> bool {
    matches!(reason, Reason::Spawner | Reason::TrialSpawner)
}
/// `EntitySpawnReason.ignoresLightRequirements`.
pub fn ignores_light(reason: Reason) -> bool {
    reason == Reason::TrialSpawner
}
/// What spawn checks read from a level.
pub trait SpawnLevel {
    /// `WorldBorder.isWithinBounds(BlockPos)`. Unbounded test levels allow all positions.
    fn within_world_border(&self, _x: i32, _z: i32) -> bool {
        true
    }

    fn state(&self, x: i32, y: i32, z: i32) -> BlockStateId;
    fn facts(&self) -> &BlockFacts;
    fn mob_data(&self) -> &data::MobData;
    fn block_tag(&self, tag: &str, block: &str) -> bool;
    fn biome_tag(&self, tag: &str, x: i32, y: i32, z: i32) -> bool;
    /// `getBrightness(LightLayer.SKY / BLOCK)`.
    fn sky_light(&self, x: i32, y: i32, z: i32) -> i32;
    fn block_light(&self, x: i32, y: i32, z: i32) -> i32;
    fn sky_darken(&self) -> i32;
    fn thundering(&self) -> bool;
    fn sea_level(&self) -> i32;
    /// `getHeight(WORLD_SURFACE, x, z)`: above the highest non-air block.
    fn world_surface(&self, x: i32, z: i32) -> i32;
    fn peaceful(&self) -> bool;
    fn dimension_type(&self) -> &DimensionFacts;
    /// The `gameplay/surface_slime_spawn_chance` environment attribute.
    fn surface_slime_chance(&self, x: i32, y: i32, z: i32) -> f32;
    fn world_seed(&self) -> i64;
    /// `getNearestPlayer(x, y, z, distance, true) != null`: a survival or
    /// adventure player that close.
    fn player_near(&self, x: f64, y: f64, z: f64, distance: f64) -> bool;
    /// `MobSpawnEvent.SpawnPlacementCheck`: answer after the type's vanilla or registered
    /// predicates, with the same random stream they used. Only the game's level tells mods.
    fn placement_check(
        &self,
        _entity: &str,
        _reason: Reason,
        _at: [i32; 3],
        default_result: bool,
        _random: &mut dyn mod_api::RandomSource,
    ) -> bool {
        default_result
    }
}

fn name<L: SpawnLevel>(l: &L, x: i32, y: i32, z: i32) -> &str {
    &l.facts().get(l.state(x, y, z)).name
}
fn fluid<L: SpawnLevel>(l: &L, x: i32, y: i32, z: i32) -> Fluid {
    l.facts().get(l.state(x, y, z)).fluid
}
fn flags<L: SpawnLevel>(l: &L, x: i32, y: i32, z: i32) -> u16 {
    l.mob_data().state(l.state(x, y, z)).flags
}
fn tag<L: SpawnLevel>(l: &L, tag: &str, x: i32, y: i32, z: i32) -> bool {
    l.block_tag(tag, name(l, x, y, z))
}
/// `getRawBrightness(pos, darkening)`.
fn raw_brightness<L: SpawnLevel>(l: &L, x: i32, y: i32, z: i32, darkening: i32) -> i32 {
    l.block_light(x, y, z).max(l.sky_light(x, y, z) - darkening)
}
fn can_see_sky<L: SpawnLevel>(l: &L, x: i32, y: i32, z: i32) -> bool {
    l.sky_light(x, y, z) >= 15
}
/// `LevelReader.canSeeSkyFromBelowWater`.
fn can_see_sky_from_below_water<L: SpawnLevel>(l: &L, x: i32, y: i32, z: i32) -> bool {
    let sea = l.sea_level();
    if y >= sea {
        return can_see_sky(l, x, y, z);
    }
    if !can_see_sky(l, x, sea, z) {
        return false;
    }
    ((y + 1)..sea).all(|yy| {
        let f = l.facts().get(l.state(x, yy, z));
        // `BlockState.liquid()` is water, lava and the bubble column. Kelp,
        // seagrass and waterlogged blocks hold water and still stop the scan.
        f.dampening == 0 || liquid_block(&f.name)
    })
}
/// `BlockState.liquid()`: the three blocks whose property is set.
fn liquid_block(name: &str) -> bool {
    matches!(
        name,
        "minecraft:water" | "minecraft:lava" | "minecraft:bubble_column"
    )
}

/// `EntityType.fireImmune`.
pub fn fire_immune(s: Species) -> bool {
    use Species as S;
    matches!(
        s,
        S::Blaze
            | S::Ghast
            | S::MagmaCube
            | S::Shulker
            | S::Strider
            | S::Vex
            | S::Warden
            | S::Wither
            | S::WitherSkeleton
            | S::Zoglin
            | S::ZombifiedPiglin
    )
}
/// The block tag an entity type is `immuneTo`.
fn immune_tag(s: Species) -> Option<&'static str> {
    use Species as S;
    match s {
        S::Fox => Some("minecraft:fox_immune_to"),
        S::PolarBear => Some("minecraft:polar_bear_immune_to"),
        S::SnowGolem => Some("minecraft:snow_golem_immune_to"),
        S::Stray => Some("minecraft:stray_immune_to"),
        S::Wither => Some("minecraft:wither_immune_to"),
        S::WitherSkeleton => Some("minecraft:wither_skeleton_immune_to"),
        _ => None,
    }
}
/// `BlockState.isValidSpawn(level, pos, type)` for the block at a position.
pub fn is_valid_spawn<L: SpawnLevel>(s: Species, l: &L, x: i32, y: i32, z: i32) -> bool {
    let bit = match s {
        Species::Ocelot | Species::Parrot => data::VALID_SPAWN_LEAVES,
        Species::PolarBear => data::VALID_SPAWN_ICE,
        _ => data::VALID_SPAWN,
    };
    valid_spawn_surface(l, x, y, z, bit, fire_immune(s))
}

fn valid_spawn_surface<L: SpawnLevel>(
    l: &L,
    x: i32,
    y: i32,
    z: i32,
    bit: u16,
    fire_immune: bool,
) -> bool {
    let state = l.state(x, y, z);
    if &*l.facts().get(state).name == "minecraft:magma_block" {
        fire_immune
    } else {
        l.mob_data().state(state).flags & bit != 0
    }
}
/// `NaturalSpawner.isValidEmptySpawnBlock` for an entity type.
pub fn is_valid_empty_spawn<L: SpawnLevel>(s: Species, l: &L, x: i32, y: i32, z: i32) -> bool {
    // The exported flag is the zombie's answer; only blocks a zombie finds
    // dangerous can answer differently for another type.
    let state = l.state(x, y, z);
    if l.mob_data().state(state).flags & data::VALID_EMPTY_SPAWN != 0 {
        return true;
    }
    let f = l.facts().get(state);
    let block = &*f.name;
    let burning =
        data::vanilla_burning_block(block, f.emission > 0, l.block_tag("minecraft:fire", block));
    let other = matches!(
        block,
        "minecraft:wither_rose"
            | "minecraft:sweet_berry_bush"
            | "minecraft:cactus"
            | "minecraft:powder_snow"
    );
    if !burning && !other {
        return false;
    }
    let immune = immune_tag(s).is_some_and(|t| l.block_tag(t, block));
    let dangerous = !immune && (burning && !fire_immune(s) || other);
    !dangerous
        && !f.full_collision
        && f.fluid == Fluid::None
        && !l.block_tag("minecraft:prevent_mob_spawning_inside", block)
}
/// `SpawnPlacements.isSpawnPositionOk` for the entity type `entity`, of species `s`.
pub fn is_spawn_position_ok<L: SpawnLevel>(
    entity: &str,
    s: Species,
    l: &L,
    x: i32,
    y: i32,
    z: i32,
) -> bool {
    let placement = l.mob_data().entity_placement(entity, s);
    if placement != data::Placement::NoRestrictions && !l.within_world_border(x, z) {
        return false;
    }
    match placement {
        data::Placement::OnGround => {
            is_valid_spawn(s, l, x, y - 1, z)
                && is_valid_empty_spawn(s, l, x, y, z)
                && is_valid_empty_spawn(s, l, x, y + 1, z)
        }
        data::Placement::InWater => {
            fluid(l, x, y, z) == Fluid::Water
                && flags(l, x, y + 1, z) & data::REDSTONE_CONDUCTOR == 0
        }
        data::Placement::InLava => fluid(l, x, y, z) == Fluid::Lava,
        data::Placement::NoRestrictions => true,
    }
}
/// `Monster.isDarkEnoughToSpawn`.
pub fn is_dark_enough<L: SpawnLevel>(
    l: &L,
    random: &mut impl RandomSource,
    x: i32,
    y: i32,
    z: i32,
) -> bool {
    if l.sky_light(x, y, z) > random.next_int_bound(32).unwrap_or(0) {
        return false;
    }
    let dim = l.dimension_type();
    let limit = dim.monster_spawn_block_light_limit;
    if limit < 15 && l.block_light(x, y, z) > limit {
        return false;
    }
    let darken = if l.thundering() { 10 } else { l.sky_darken() };
    let brightness = raw_brightness(l, x, y, z, darken);
    brightness <= dim.monster_spawn_light_level.sample(random).unwrap_or(0)
}
/// `Animal.isBrightEnoughToSpawn`.
fn bright<L: SpawnLevel>(l: &L, x: i32, y: i32, z: i32) -> bool {
    raw_brightness(l, x, y, z, 0) > 8
}
/// `Mob.checkMobSpawnRules`.
fn mob_rules<L: SpawnLevel>(s: Species, l: &L, reason: Reason, x: i32, y: i32, z: i32) -> bool {
    is_spawner(reason) || is_valid_spawn(s, l, x, y - 1, z)
}
/// `Monster.checkMonsterSpawnRules`, independent of registered spawn placements.
pub fn monster_rules<L: SpawnLevel>(
    s: Species,
    l: &L,
    reason: Reason,
    random: &mut impl RandomSource,
    x: i32,
    y: i32,
    z: i32,
) -> bool {
    (ignores_light(reason) || is_dark_enough(l, random, x, y, z))
        && mob_rules(s, l, reason, x, y, z)
}
/// `PatrollingMonster.checkPatrollingMonsterSpawnRules`, independent of registered placements.
pub(crate) fn patrolling_monster_rules<L: SpawnLevel>(
    s: Species,
    l: &L,
    reason: Reason,
    x: i32,
    y: i32,
    z: i32,
) -> bool {
    l.block_light(x, y, z) <= 8 && mob_rules(s, l, reason, x, y, z)
}
/// `Animal.checkAnimalSpawnRules` with the given spawnable-on tag.
fn animal_rules<L: SpawnLevel>(l: &L, reason: Reason, on: &str, x: i32, y: i32, z: i32) -> bool {
    let bright = ignores_light(reason) || bright(l, x, y, z);
    tag(l, on, x, y - 1, z) && bright
}
/// `WaterAnimal.checkSurfaceWaterAnimalSpawnRules`.
fn surface_water<L: SpawnLevel>(l: &L, x: i32, y: i32, z: i32) -> bool {
    let sea = l.sea_level();
    y >= sea - 13
        && y <= sea
        && fluid(l, x, y - 1, z) == Fluid::Water
        && name(l, x, y + 1, z) == "minecraft:water"
}

/// Registry identity and vanilla spawn facts of the resolved entity type.
#[derive(Clone, Copy)]
pub struct SpawnEntityType<'a> {
    pub id: &'a str,
    pub species: Option<Species>,
    pub allowed_in_peaceful: bool,
}

impl<'a> From<&'a MobType> for SpawnEntityType<'a> {
    fn from(kind: &'a MobType) -> Self {
        Self {
            id: &kind.id,
            species: Some(kind.species),
            allowed_in_peaceful: kind.allowed_in_peaceful(),
        }
    }
}

/// `SpawnPlacements.checkSpawnRules` for the resolved entity type, then
/// `MobSpawnEvent.SpawnPlacementCheck` through [`SpawnLevel::placement_check`].
pub fn check_spawn_rules<L: SpawnLevel, R: RandomSource + Clone>(
    kind: SpawnEntityType<'_>,
    l: &L,
    reason: Reason,
    random: &mut R,
    at: [i32; 3],
) -> bool {
    if !kind.allowed_in_peaceful && l.peaceful() {
        return false;
    }
    let [x, y, z] = at;
    let vanilla = |random: &mut R| match kind.species {
        Some(s) => placement_predicate(s, l, reason, random, x, y, z),
        // The dragon has no Species row, but SpawnPlacements registers it with
        // Mob.checkMobSpawnRules, including access to magma (it is fire immune).
        None if kind.id == "minecraft:ender_dragon" => {
            is_spawner(reason) || valid_spawn_surface(l, x, y - 1, z, data::VALID_SPAWN, true)
        }
        // SpawnPlacements.checkSpawnRules accepts types without an entry.
        None => true,
    };
    let allowed = match l.mob_data().mod_spawns.get(kind.id) {
        Some(row) => {
            let pos = BlockPos::new(x, y, z);
            let test = |p: &data::OwnedSpawnPredicate, random: &mut R| {
                if !p.owner.is_enabled() {
                    return None;
                }
                p.owner
                    .call_with_panic_rollback(
                        "spawn placement",
                        "spawn_placement_predicate",
                        random,
                        |random| {
                            let mut ctx = SpawnContext {
                                level: SpawnLevelView(l),
                                random: SpawnRandom(random),
                            };
                            p.predicate.test(row.kind, &mut ctx, reason, pos)
                        },
                    )
                    .ok()
            };
            match &row.predicates {
                // NeoForge installs only the selected replacement, even when it fails.
                data::SpawnPredicates::Replacement(predicate) => {
                    test(predicate, random).unwrap_or(false)
                }
                data::SpawnPredicates::Combined { original, or, and } => {
                    let original = match original {
                        Some(predicate) => test(predicate, random).unwrap_or(false),
                        None => vanilla(random),
                    };
                    (original || or.iter().any(|p| test(p, random) == Some(true)))
                        && and.iter().all(|p| test(p, random) != Some(false))
                }
            }
        }
        None => vanilla(random),
    };
    l.placement_check(kind.id, reason, at, allowed, &mut SpawnRandom(random))
}

struct SpawnLevelView<'a, L>(&'a L);

impl<L: SpawnLevel> SpawnPlacementLevel for SpawnLevelView<'_, L> {
    fn block_name(&self, pos: BlockPos) -> &str {
        name(self.0, pos.x, pos.y, pos.z)
    }

    fn block_in_tag(&self, pos: BlockPos, id: &str) -> bool {
        tag(self.0, id, pos.x, pos.y, pos.z)
    }

    fn biome_in_tag(&self, pos: BlockPos, id: &str) -> bool {
        self.0.biome_tag(id, pos.x, pos.y, pos.z)
    }

    fn sky_light(&self, pos: BlockPos) -> i32 {
        self.0.sky_light(pos.x, pos.y, pos.z)
    }

    fn block_light(&self, pos: BlockPos) -> i32 {
        self.0.block_light(pos.x, pos.y, pos.z)
    }

    fn is_bright_enough_to_spawn(&self, pos: BlockPos) -> bool {
        bright(self.0, pos.x, pos.y, pos.z)
    }
}

struct SpawnContext<'a, L, R> {
    level: SpawnLevelView<'a, L>,
    random: SpawnRandom<'a, R>,
}

impl<L: SpawnLevel, R: RandomSource> SpawnPlacementContext for SpawnContext<'_, L, R> {
    fn level(&self) -> &dyn SpawnPlacementLevel {
        &self.level
    }

    fn random(&mut self) -> &mut dyn mod_api::RandomSource {
        &mut self.random
    }

    fn is_dark_enough_to_spawn(&mut self, pos: BlockPos) -> bool {
        is_dark_enough(self.level.0, self.random.0, pos.x, pos.y, pos.z)
    }
}

struct SpawnRandom<'a, R>(&'a mut R);

impl<R: RandomSource> mod_api::RandomSource for SpawnRandom<'_, R> {
    fn next_int(&mut self) -> i32 {
        self.0.next_int()
    }

    fn next_int_bound(&mut self, bound: i32) -> i32 {
        self.0.next_int_bound(bound).unwrap_or(0)
    }

    fn next_long(&mut self) -> i64 {
        self.0.next_long()
    }

    fn next_boolean(&mut self) -> bool {
        self.0.next_boolean()
    }

    fn next_float(&mut self) -> f32 {
        self.0.next_float()
    }

    fn next_double(&mut self) -> f64 {
        self.0.next_double()
    }

    fn next_gaussian(&mut self) -> f64 {
        self.0.next_gaussian()
    }
}

/// The type's spawn predicate (`SpawnPlacements.Data.predicate`).
fn placement_predicate<L: SpawnLevel>(
    s: Species,
    l: &L,
    reason: Reason,
    random: &mut impl RandomSource,
    x: i32,
    y: i32,
    z: i32,
) -> bool {
    use Species as S;
    match s {
        S::Axolotl => tag(l, "minecraft:axolotls_spawnable_on", x, y - 1, z),
        S::Cod | S::Pufferfish | S::Salmon | S::Dolphin | S::Squid => surface_water(l, x, y, z),
        S::Drowned => {
            if fluid(l, x, y - 1, z) != Fluid::Water && !is_spawner(reason) {
                return false;
            }
            let frequent = l.biome_tag("minecraft:more_frequent_drowned_spawns", x, y, z);
            let can_spawn = !l.peaceful()
                && (ignores_light(reason) || is_dark_enough(l, random, x, y, z))
                && (is_spawner(reason) || fluid(l, x, y, z) == Fluid::Water);
            if !can_spawn || !is_spawner(reason) && reason != Reason::Reinforcement {
                if frequent {
                    random.next_int_bound(15).unwrap_or(1) == 0 && can_spawn
                } else {
                    random.next_int_bound(40).unwrap_or(1) == 0
                        && y < l.sea_level() - 5
                        && can_spawn
                }
            } else {
                true
            }
        }
        S::Guardian | S::ElderGuardian => {
            (random.next_int_bound(20).unwrap_or(1) == 0
                || !can_see_sky_from_below_water(l, x, y, z))
                && !l.peaceful()
                && (is_spawner(reason) || fluid(l, x, y, z) == Fluid::Water)
                && fluid(l, x, y - 1, z) == Fluid::Water
        }
        S::TropicalFish => {
            fluid(l, x, y - 1, z) == Fluid::Water
                && name(l, x, y + 1, z) == "minecraft:water"
                && (l.biome_tag(
                    "minecraft:allows_tropical_fish_spawns_at_any_height",
                    x,
                    y,
                    z,
                ) || surface_water(l, x, y, z))
        }
        S::Armadillo => {
            tag(l, "minecraft:armadillo_spawnable_on", x, y - 1, z) && bright(l, x, y, z)
        }
        S::Bat => {
            if y >= l.world_surface(x, z) || random.next_boolean() {
                return false;
            }
            if raw_brightness(l, x, y, z, l.sky_darken()) > random.next_int_bound(4).unwrap_or(0) {
                return false;
            }
            tag(l, "minecraft:bats_spawnable_on", x, y - 1, z) && mob_rules(s, l, reason, x, y, z)
        }
        S::Blaze | S::Breeze | S::Zoglin => mob_rules(s, l, reason, x, y, z),
        S::Camel => tag(l, "minecraft:camels_spawnable_on", x, y - 1, z) && bright(l, x, y, z),
        S::CamelHusk | S::Husk | S::Parched => {
            monster_rules(s, l, reason, random, x, y, z)
                && (is_spawner(reason) || can_see_sky(l, x, y, z))
        }
        S::Chicken
        | S::Cow
        | S::Donkey
        | S::HappyGhast
        | S::Horse
        | S::Llama
        | S::Mule
        | S::Pig
        | S::Sheep
        | S::Cat
        | S::Panda
        | S::TraderLlama => animal_rules(l, reason, "minecraft:animals_spawnable_on", x, y, z),
        S::Endermite | S::Silverfish => {
            if !mob_rules(s, l, reason, x, y, z) {
                return false;
            }
            is_spawner(reason)
                || !l.player_near(
                    f64::from(x) + 0.5,
                    f64::from(y) + 0.5,
                    f64::from(z) + 0.5,
                    5.0,
                )
        }
        S::IronGolem
        | S::SnowGolem
        | S::Villager
        | S::Phantom
        | S::Shulker
        | S::WanderingTrader => mob_rules(s, l, reason, x, y, z),
        S::Frog => tag(l, "minecraft:frogs_spawnable_on", x, y - 1, z) && bright(l, x, y, z),
        S::Ghast => {
            !l.peaceful()
                && random.next_int_bound(20).unwrap_or(1) == 0
                && mob_rules(s, l, reason, x, y, z)
        }
        S::GlowSquid => {
            y <= l.sea_level() - 33
                && raw_brightness(l, x, y, z, 0) == 0
                && name(l, x, y, z) == "minecraft:water"
        }
        S::Goat => tag(l, "minecraft:goats_spawnable_on", x, y - 1, z) && bright(l, x, y, z),
        S::MagmaCube => !l.peaceful(),
        S::SulfurCube => true,
        S::Mooshroom => {
            tag(l, "minecraft:mooshrooms_spawnable_on", x, y - 1, z) && bright(l, x, y, z)
        }
        S::Nautilus => {
            let sea = l.sea_level();
            y >= sea - 25
                && y <= sea - 5
                && fluid(l, x, y - 1, z) == Fluid::Water
                && name(l, x, y + 1, z) == "minecraft:water"
        }
        S::Ocelot => random.next_int_bound(3).unwrap_or(0) != 0,
        S::Parrot => tag(l, "minecraft:parrots_spawnable_on", x, y - 1, z) && bright(l, x, y, z),
        S::Hoglin | S::Piglin => name(l, x, y - 1, z) != "minecraft:nether_wart_block",
        S::Pillager => patrolling_monster_rules(s, l, reason, x, y, z),
        S::PolarBear => {
            if l.biome_tag("minecraft:polar_bears_spawn_on_alternate_blocks", x, y, z) {
                bright(l, x, y, z)
                    && tag(
                        l,
                        "minecraft:polar_bears_spawnable_on_alternate",
                        x,
                        y - 1,
                        z,
                    )
            } else {
                animal_rules(l, reason, "minecraft:animals_spawnable_on", x, y, z)
            }
        }
        S::Rabbit => tag(l, "minecraft:rabbits_spawnable_on", x, y - 1, z) && bright(l, x, y, z),
        S::SkeletonHorse => {
            if is_spawner(reason) {
                ignores_light(reason) || bright(l, x, y, z)
            } else {
                animal_rules(l, reason, "minecraft:animals_spawnable_on", x, y, z)
            }
        }
        S::Slime => slime_rules(s, l, reason, random, x, y, z),
        S::Stray => {
            let mut sky = y + 1;
            while name(l, x, sky, z) == "minecraft:powder_snow" {
                sky += 1;
            }
            monster_rules(s, l, reason, random, x, y, z)
                && (is_spawner(reason) || can_see_sky(l, x, sky - 1, z))
        }
        S::Strider => {
            let mut up = y + 1;
            while fluid(l, x, up, z) == Fluid::Lava {
                up += 1;
            }
            l.facts().is_air(l.state(x, up, z))
        }
        S::Turtle => {
            y < l.sea_level() + 4 && tag(l, "minecraft:sand", x, y - 1, z) && bright(l, x, y, z)
        }
        S::Wolf => tag(l, "minecraft:wolves_spawnable_on", x, y - 1, z) && bright(l, x, y, z),
        S::ZombifiedPiglin => {
            !l.peaceful() && name(l, x, y - 1, z) != "minecraft:nether_wart_block"
        }
        S::Fox => tag(l, "minecraft:foxes_spawnable_on", x, y - 1, z) && bright(l, x, y, z),
        S::Bogged
        | S::CaveSpider
        | S::Creeper
        | S::Enderman
        | S::Giant
        | S::Skeleton
        | S::Spider
        | S::Witch
        | S::Wither
        | S::WitherSkeleton
        | S::Creaking
        | S::Zombie
        | S::ZombieHorse
        | S::ZombieVillager
        | S::Evoker
        | S::Illusioner
        | S::Ravager
        | S::Vex
        | S::Vindicator
        | S::Warden => monster_rules(s, l, reason, random, x, y, z),
        // Types without a SpawnPlacements registration.
        _ => true,
    }
}
/// `Slime.checkSlimeSpawnRules`.
fn slime_rules<L: SpawnLevel>(
    s: Species,
    l: &L,
    reason: Reason,
    random: &mut impl RandomSource,
    x: i32,
    y: i32,
    z: i32,
) -> bool {
    if l.peaceful() {
        return false;
    }
    if is_spawner(reason) {
        return mob_rules(s, l, reason, x, y, z);
    }
    if l.biome_tag("minecraft:allows_surface_slime_spawns", x, y, z) && y > 50 && y < 70 {
        let chance = l.surface_slime_chance(x, y, z);
        if random.next_float() < chance
            && raw_brightness(l, x, y, z, l.sky_darken()) <= random.next_int_bound(8).unwrap_or(0)
        {
            return mob_rules(s, l, reason, x, y, z);
        }
    }
    let slime_chunk = slime_chunk_random(x >> 4, z >> 4, l.world_seed(), 987_234_911)
        .next_int_bound(10)
        .unwrap_or(1)
        == 0;
    if random.next_int_bound(10).unwrap_or(1) == 0 && slime_chunk && y < 40 {
        return mob_rules(s, l, reason, x, y, z);
    }
    false
}

impl super::MobWorld<'_> {
    /// Tests a spawn biome tag using the biome table index from the level.
    #[inline]
    pub fn spawn_biome_tag(&self, tag: &str, biome: Option<u16>) -> bool {
        biome.is_some_and(|id| {
            self.data
                .biome_has(tag.trim_start_matches('#'), &self.biomes.get(id).name)
        })
    }

    /// Reads the sky layer used by spawn checks. The light read is deferred
    /// so levels whose sky is treated as dark do not look up a chunk.
    #[inline]
    pub fn spawn_sky_light(&self, light: impl FnOnce() -> u8) -> i32 {
        // Kept as before: vanilla's End has sky light, but Sunrise reads 0 there.
        if matches!(
            self.dimension_type.id.as_str(),
            "minecraft:the_nether" | "minecraft:the_end"
        ) {
            return 0;
        }
        i32::from(light() >> 4)
    }

    /// Reads WORLD_SURFACE with the caller's lookup result for this column.
    #[inline]
    pub fn spawn_world_surface(&self, x: i32, z: i32, chunk: Option<&Chunk>) -> i32 {
        self.world
            .world_surface_with_chunk(x, z, chunk, |id| self.facts.is_air(id))
    }
}

/// Gets the block layer used by spawn checks from the level's packed light.
#[inline]
pub fn spawn_block_light(light: u8) -> i32 {
    i32::from(light & 15)
}

impl SpawnLevel for super::MobWorld<'_> {
    fn within_world_border(&self, x: i32, z: i32) -> bool {
        self.border
            .is_none_or(|border| border.is_within_bounds(f64::from(x), f64::from(z), 0.0))
    }
    fn state(&self, x: i32, y: i32, z: i32) -> BlockStateId {
        self.world.block(x, y, z)
    }
    fn facts(&self) -> &BlockFacts {
        self.facts
    }
    fn mob_data(&self) -> &data::MobData {
        self.data
    }
    fn block_tag(&self, tag: &str, block: &str) -> bool {
        self.has_tag(tag, block)
    }
    fn biome_tag(&self, tag: &str, x: i32, y: i32, z: i32) -> bool {
        self.spawn_biome_tag(tag, self.world.noise_biome(x, y, z))
    }
    fn sky_light(&self, x: i32, y: i32, z: i32) -> i32 {
        self.spawn_sky_light(|| self.world.light(x, y, z))
    }
    fn block_light(&self, x: i32, y: i32, z: i32) -> i32 {
        spawn_block_light(self.world.light(x, y, z))
    }
    fn sky_darken(&self) -> i32 {
        super::MobWorld::sky_darken(self)
    }
    fn thundering(&self) -> bool {
        self.thundering
    }
    fn sea_level(&self) -> i32 {
        self.sea_level
    }
    fn world_surface(&self, x: i32, z: i32) -> i32 {
        self.spawn_world_surface(x, z, self.world.chunk_at(x >> 4, z >> 4))
    }
    fn peaceful(&self) -> bool {
        self.difficulty.effective == 0.0
    }
    fn dimension_type(&self) -> &DimensionFacts {
        self.dimension_type
    }
    fn surface_slime_chance(&self, _: i32, _: i32, _: i32) -> f32 {
        let phase = self.day_time.div_euclid(24000).rem_euclid(8) as usize;
        0.5 * super::spawning::MOON_BRIGHTNESS[phase]
    }
    fn world_seed(&self) -> i64 {
        self.world_seed
    }
    fn player_near(&self, x: f64, y: f64, z: f64, distance: f64) -> bool {
        self.selected_player().is_some_and(|p| {
            let d = (p.position.x - x).powi(2)
                + (p.position.y - y).powi(2)
                + (p.position.z - z).powi(2);
            p.alive && !p.creative && !p.spectator && d < distance * distance
        })
    }
}

#[cfg(test)]
mod underwater_sky_tests {
    use super::*;
    use crate::mobs::world::with_built_test_world;
    use sunrise_core::random::LegacyRandom;
    use sunrise_world::{block_state_properties::BlockProperties, chunk::BlockState};

    fn with_props(
        registry: &sunrise_world::registry::BlockRegistry,
        name: &str,
        props: &[(&str, &str)],
    ) -> sunrise_world::registry::BlockStateId {
        let mut properties = BlockProperties::new();
        for (key, value) in props {
            properties.insert((*key).to_owned(), (*value).to_owned());
        }
        registry
            .resolve(&BlockState {
                name: format!("minecraft:{name}"),
                properties,
            })
            .unwrap()
    }

    /// `Guardian.checkGuardianSpawnRules`: below sea level the 1-in-20 roll is
    /// skipped when `canSeeSkyFromBelowWater` is false. Kelp (dampening 1, not
    /// `liquid()`) makes it false. A bubble column is `liquid()`, so the scan
    /// continues and the roll is still required.
    #[test]
    #[ignore = "daily"]
    fn kelp_hides_the_sky_from_a_guardian_and_a_bubble_column_does_not() {
        let ran = with_built_test_world(
            &[],
            |world, registry| {
                world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
                );
                let water = registry.default_state("minecraft:water").unwrap();
                let kelp = with_props(registry, "kelp", &[("age", "0")]);
                let column = with_props(registry, "bubble_column", &[("drag", "true")]);
                for x in [0, 1] {
                    world.set(x, 49, 0, water).unwrap();
                    world.set(x, 50, 0, water).unwrap();
                }
                world.set(0, 55, 0, kelp).unwrap();
                world.set(1, 55, 0, column).unwrap();
            },
            |w, _| {
                // Sea level in this world is 63, so y=55 sits in the scan.
                assert_eq!(w.sea_level(), 63);
                let mut guardian = (*super::super::sensor_scan_tests::kind()).clone();
                guardian.id = "minecraft:guardian".into();
                guardian.species = Species::Guardian;
                let mut probe = LegacyRandom::new(1);
                assert_ne!(
                    probe.next_int_bound(20).unwrap(),
                    0,
                    "seed 1 must miss the 1-in-20"
                );
                let mut random = LegacyRandom::new(1);
                assert!(
                    check_spawn_rules(
                        (&guardian).into(),
                        w,
                        Reason::Natural,
                        &mut random,
                        [0, 50, 0]
                    ),
                    "kelp between the guardian and the sea hides the sky"
                );
                let mut random = LegacyRandom::new(1);
                assert!(
                    !check_spawn_rules(
                        (&guardian).into(),
                        w,
                        Reason::Natural,
                        &mut random,
                        [1, 50, 0]
                    ),
                    "a bubble column is liquid(), so the sky still shows"
                );
            },
        );
        // Without the vanilla data under .local/sunrise-client the world
        // cannot be built: the test has nothing to check.
        assert!(ran, "vanilla data is missing");
    }
}

#[cfg(test)]
mod light_draw_tests {
    use super::*;
    use crate::mobs::{MobWorld, world::with_built_test_world};
    use sunrise_core::random::LegacyRandom;

    /// `Monster.isDarkEnoughToSpawn`: the sky roll draws one bound-32 int, then the
    /// dimension type's light level draws as its provider does (uniform 0..=7 one
    /// bound-8 int, a constant none).
    #[test]
    #[ignore = "daily"]
    fn dark_enough_draws_follow_the_dimension_type_provider() {
        assert!(
            with_built_test_world(
                &[],
                |world, _| {
                    world.chunks.insert(
                        (0, 0),
                        std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
                    );
                },
                |w, _| {
                    for (type_id, draws) in [
                        ("minecraft:overworld", &[32, 8][..]),
                        ("minecraft:the_nether", &[32][..]),
                        ("minecraft:the_end", &[32][..]),
                    ] {
                        let level = MobWorld {
                            side: crate::motion::dry_side(),
                            dimension_type: DimensionFacts::vanilla(type_id).unwrap(),
                            ..w.clone()
                        };
                        let mut random = LegacyRandom::new(42);
                        let dark = is_dark_enough(&level, &mut random, 0, 70, 0);
                        let mut expected = LegacyRandom::new(42);
                        let rolls: Vec<i32> = draws
                            .iter()
                            .map(|bound| expected.next_int_bound(*bound).unwrap())
                            .collect();
                        assert_eq!(random.next_int(), expected.next_int(), "{type_id}");
                        let light = level
                            .block_light(0, 70, 0)
                            .max(level.sky_light(0, 70, 0) - level.sky_darken());
                        let test = rolls.get(1).copied().unwrap_or(match type_id {
                            "minecraft:the_nether" => 7,
                            _ => 15,
                        });
                        assert_eq!(
                            dark,
                            rolls[0] >= level.sky_light(0, 70, 0) && light <= test,
                            "{type_id}"
                        );
                    }
                },
            ),
            "vanilla data is missing"
        );
    }
}

/// Native registrations answer the type's geometry and predicate without
/// changing the template species or bypassing the placement-check hook.
#[cfg(test)]
mod mod_spawn_rule_tests {
    use super::super::spawn_selection::Heightmap as Surface;
    use super::*;
    use crate::blocks::{RenderShape, StateFacts};
    use data::Placement;
    use mod_api::__private::{SpawnPlacementDeclaration, parts_installation::entity_type_key};
    use mod_api::{
        EntityTypeKey, Heightmap,
        SpawnPlacementOperation::{And, Or, Replace},
        SpawnPlacementPredicate, SpawnPlacementType,
    };
    use std::{
        cell::RefCell,
        sync::{Arc, Mutex},
    };
    use sunrise_core::random::LegacyRandom;

    const AIR: BlockStateId = BlockStateId(0);
    const GRASS: BlockStateId = BlockStateId(1);
    const STONE: BlockStateId = BlockStateId(2);

    // java.util.Random(7).nextInt(), in order.
    const JAVA_DRAWS: [i32; 7] = [
        -1_156_638_823,
        -1_552_468_968,
        -1_077_308_326,
        41_356_089,
        1_495_978_761,
        2_107_132_509,
        -441_191_359,
    ];

    type PredicateCalls = Arc<Mutex<Vec<(&'static str, i32)>>>;

    fn recording_predicate(
        calls: &PredicateCalls,
        name: &'static str,
        passes: bool,
    ) -> impl SpawnPlacementPredicate + use<> {
        let calls = Arc::clone(calls);
        move |_: EntityTypeKey, ctx: &mut dyn SpawnPlacementContext, _: Reason, _: BlockPos| {
            let draw = ctx.random().next_int();
            calls.lock().unwrap().push((name, draw));
            passes
        }
    }

    fn state(name: &str, full: bool) -> StateFacts {
        let sound = mod_api::SoundEventKey::parse("").unwrap();
        StateFacts {
            name: Arc::from(name),
            emission: 0,
            dynamic_light: false,
            dampening: 0,
            skylight_down: true,
            shape_occlusion: false,
            can_occlude: full,
            solid_render: full,
            hardness: 1.0,
            requires_tool: false,
            replaceable: !full,
            random_ticks: false,
            block_entity: false,
            render_shape: RenderShape::Model,
            spawn_terrain_particles: true,
            destroy_overlay: true,
            full_collision: full,
            friction: 0.6,
            explosion_resistance: 1.0,
            item: Arc::from(name),
            pick_item: Arc::from(name),
            sounds: mod_api::BlockSoundType {
                identity: mod_api::BlockSoundTypeIdentity::Other,
                volume: 1.0,
                pitch: 1.0,
                break_sound: sound.clone(),
                step: sound.clone(),
                place: sound.clone(),
                hit: sound.clone(),
                fall: sound,
            },
            fluid: Fluid::None,
            fluid_level: 0,
            fluid_falling: false,
            waterlogged: false,
            random_offset: false,
            offset_limits: None,
            outline_offset: false,
            legacy_solid: full,
            sturdy_full: 0,
            classes: Arc::from([]),
            outline: Arc::from([]),
            outline_edges: Arc::from([]),
            map_color: 0,
        }
    }

    /// A flat world: grass at y 0, stone at x 5 up to y 1, air above. Tests
    /// can set light, biome tags, whether the level is Peaceful, and the hook's answer.
    struct Flat {
        facts: BlockFacts,
        data: data::MobData,
        sky: i32,
        bright_block: Option<BlockPos>,
        peaceful: bool,
        /// The biome tags the spot's biome is in.
        biome_tags: Vec<&'static str>,
        /// What the hook is told the rule answered, and what it answers.
        heard: RefCell<Vec<bool>>,
        hook: Option<bool>,
    }
    impl Flat {
        fn new(declarations: &[SpawnPlacementDeclaration]) -> Self {
            let facts = BlockFacts {
                states: vec![
                    state("minecraft:air", false),
                    state("minecraft:grass_block", true),
                    state("minecraft:stone", true),
                ],
                air: AIR,
                woods: Default::default(),
                sign_texts: Default::default(),
            };
            let mut data = data::MobData::fallback(&facts);
            data.vanilla_spawn_placements = Some(
                [
                    "minecraft:zombie".to_owned(),
                    "minecraft:shulker".to_owned(),
                ]
                .into(),
            );
            data.placements
                .insert("minecraft:zombie".to_owned(), Placement::OnGround);
            data.placements
                .insert("minecraft:shulker".to_owned(), Placement::NoRestrictions);
            let mut level = Self {
                facts,
                data,
                sky: 15,
                bright_block: None,
                peaceful: false,
                biome_tags: vec!["test:haunted"],
                heard: RefCell::default(),
                hook: None,
            };
            level.install(declarations).unwrap();
            level
        }
        fn install(&mut self, declarations: &[SpawnPlacementDeclaration]) -> Result<(), String> {
            self.data.install_spawn_placements(declarations, |id| {
                matches!(
                    id,
                    "minecraft:zombie"
                        | "minecraft:shulker"
                        | "minecraft:arrow"
                        | "ruby:ghoul"
                        | "ruby:floater"
                        | "test:trader"
                )
            })
        }
        /// `SpawnPlacements.checkSpawnRules` for a type of the zombie species.
        fn check(&self, entity: &str, reason: Reason, at: [i32; 3]) -> bool {
            let kind = mob_kind(entity, Species::Zombie);
            check_spawn_rules((&kind).into(), self, reason, &mut LegacyRandom::new(7), at)
        }
    }

    #[test]
    fn dragon_placement_keeps_its_vanilla_surface_requirement() {
        let mut level = Flat::new(&[]);
        level.peaceful = true; // The dragon is allowed in Peaceful.
        let dragon = SpawnEntityType {
            id: "minecraft:ender_dragon",
            species: None,
            allowed_in_peaceful: true,
        };
        let mut random = LegacyRandom::new(7);
        // Mob.checkMobSpawnRules accepts a spawner in midair, but needs a
        // valid surface for a natural spawn. Neither path draws random numbers.
        assert!(!check_spawn_rules(
            dragon,
            &level,
            Reason::Natural,
            &mut random,
            [0, 2, 0]
        ));
        assert!(check_spawn_rules(
            dragon,
            &level,
            Reason::Spawner,
            &mut random,
            [0, 2, 0]
        ));
        assert!(check_spawn_rules(
            dragon,
            &level,
            Reason::Natural,
            &mut random,
            [0, 1, 0]
        ));
        assert_eq!(random.next_int(), JAVA_DRAWS[0]);
    }

    fn mob_kind(entity: &str, species: Species) -> MobType {
        let mut kind = (*super::super::sensor_scan_tests::kind()).clone();
        kind.id = entity.into();
        kind.species = species;
        kind.flags.not_in_peaceful = species == Species::Zombie;
        kind
    }
    impl SpawnLevel for Flat {
        fn state(&self, x: i32, y: i32, _: i32) -> BlockStateId {
            match (x, y) {
                (5, ..=1) => STONE,
                (_, 0) => GRASS,
                _ => AIR,
            }
        }
        fn facts(&self) -> &BlockFacts {
            &self.facts
        }
        fn mob_data(&self) -> &data::MobData {
            &self.data
        }
        fn block_tag(&self, tag: &str, block: &str) -> bool {
            tag == "test:ghoul_ground" && block == "minecraft:grass_block"
        }
        fn biome_tag(&self, tag: &str, _: i32, _: i32, _: i32) -> bool {
            self.biome_tags.contains(&tag)
        }
        fn sky_light(&self, _: i32, _: i32, _: i32) -> i32 {
            self.sky
        }
        fn block_light(&self, x: i32, y: i32, z: i32) -> i32 {
            if self.bright_block == Some(BlockPos::new(x, y, z)) {
                15
            } else {
                0
            }
        }
        fn sky_darken(&self) -> i32 {
            0
        }
        fn thundering(&self) -> bool {
            false
        }
        fn sea_level(&self) -> i32 {
            63
        }
        fn world_surface(&self, _: i32, _: i32) -> i32 {
            1
        }
        fn peaceful(&self) -> bool {
            self.peaceful
        }
        fn dimension_type(&self) -> &DimensionFacts {
            DimensionFacts::vanilla("minecraft:overworld").unwrap()
        }
        fn surface_slime_chance(&self, _: i32, _: i32, _: i32) -> f32 {
            0.0
        }
        fn world_seed(&self) -> i64 {
            0
        }
        fn player_near(&self, _: f64, _: f64, _: f64, _: f64) -> bool {
            false
        }
        fn placement_check(
            &self,
            _: &str,
            _: Reason,
            _: [i32; 3],
            default_result: bool,
            _: &mut dyn mod_api::RandomSource,
        ) -> bool {
            self.heard.borrow_mut().push(default_result);
            self.hook.unwrap_or(default_result)
        }
    }

    fn ghoul(predicate: impl SpawnPlacementPredicate) -> SpawnPlacementDeclaration {
        SpawnPlacementDeclaration {
            id: "ruby:ghoul".to_owned(),
            kind: entity_type_key(0),
            mod_id: "ruby",
            placement: Some(SpawnPlacementType::OnGround),
            heightmap: Some(Heightmap::OceanFloor),
            predicate: Arc::new(predicate),
            operation: Replace,
        }
    }

    fn bright_ghoul(
        _kind: EntityTypeKey,
        ctx: &mut dyn SpawnPlacementContext,
        reason: Reason,
        pos: BlockPos,
    ) -> bool {
        ctx.level()
            .block_in_tag(pos.offset(0, -1, 0), "test:ghoul_ground")
            && ctx.level().biome_in_tag(pos, "test:haunted")
            && (reason == Reason::TrialSpawner || ctx.level().is_bright_enough_to_spawn(pos))
    }

    #[test]
    fn natural_placement_uses_the_types_peaceful_flag_not_its_species() {
        let mut trader = (*super::super::sensor_scan_tests::kind()).clone();
        trader.id = "test:trader".into();
        trader.species = Species::WanderingTrader;
        trader.flags.not_in_peaceful = true;

        let mut level = Flat::new(&[SpawnPlacementDeclaration {
            id: "test:trader".into(),
            kind: entity_type_key(5),
            mod_id: "test",
            placement: Some(SpawnPlacementType::OnGround),
            heightmap: Some(Heightmap::OceanFloor),
            predicate: Arc::new(
                |_: EntityTypeKey, _: &mut dyn SpawnPlacementContext, _: Reason, _: BlockPos| true,
            ),
            operation: Replace,
        }]);
        let check = |level: &Flat| {
            check_spawn_rules(
                (&trader).into(),
                level,
                Reason::Natural,
                &mut LegacyRandom::new(7),
                [0, 1, 0],
            )
        };
        assert!(check(&level));
        level.heard.borrow_mut().clear();
        level.peaceful = true;
        assert!(!check(&level));
        assert!(
            level.heard.borrow().is_empty(),
            "Peaceful admission comes before placement rules"
        );
    }

    /// Ground, biome and light each refuse on their own. The ghoul's callback
    /// passes in daylight at the same spot where its zombie template fails.
    #[test]
    fn the_rule_checks_ground_biome_and_light_in_place_of_the_templates_predicate() {
        let mut level = Flat::new(&[ghoul(bright_ghoul)]);
        let on_grass = [0, 1, 0];
        assert!(level.check("ruby:ghoul", Reason::Natural, on_grass));
        assert!(
            !level.check("minecraft:zombie", Reason::Natural, on_grass),
            "the vanilla zombie still needs the dark"
        );
        // The block below, stone, is not in the ground tag.
        assert!(!level.check("ruby:ghoul", Reason::Natural, [5, 2, 0]));
        // The biome is not in the tag.
        level.biome_tags.clear();
        assert!(!level.check("ruby:ghoul", Reason::Natural, on_grass));
        level.biome_tags.push("test:haunted");
        // Light 8 is not above 8; a trial spawner ignores light.
        level.sky = 8;
        assert!(!level.check("ruby:ghoul", Reason::Natural, on_grass));
        assert!(level.check("ruby:ghoul", Reason::TrialSpawner, on_grass));
        level.sky = 9;
        assert!(level.check("ruby:ghoul", Reason::Natural, on_grass));
    }

    #[test]
    fn a_dark_rule_checks_light_at_the_position_the_predicate_requests() {
        let mut level = Flat::new(&[ghoul(
            |_: EntityTypeKey,
             ctx: &mut dyn SpawnPlacementContext,
             reason: Reason,
             pos: BlockPos| {
                reason == Reason::TrialSpawner || ctx.is_dark_enough_to_spawn(pos.offset(1, 0, 0))
            },
        )]);
        let on_grass = [0, 1, 0];
        level.sky = 0;
        level.bright_block = Some(BlockPos::new(0, 1, 0));
        assert!(level.check("ruby:ghoul", Reason::Natural, on_grass));
        level.bright_block = Some(BlockPos::new(1, 1, 0));
        assert!(!level.check("ruby:ghoul", Reason::Natural, on_grass));
        level.sky = 15;
        level.bright_block = None;
        assert!(!level.check("ruby:ghoul", Reason::Natural, on_grass));
    }

    #[test]
    fn a_predicate_and_its_darkness_check_advance_the_callers_random() {
        let mut level = Flat::new(&[]);
        level.sky = 0;
        let mut rng = LegacyRandom::new(7);
        let mut context = SpawnContext {
            level: SpawnLevelView(&level),
            random: SpawnRandom(&mut rng),
        };

        // Java's Random(7): the refused bound draws nothing, and the Overworld's
        // darkness check then draws nextInt(32) and nextInt(8).
        assert_eq!(context.random().next_int_bound(0), 0);
        assert_eq!(context.random().next_int(), JAVA_DRAWS[0]);
        assert!(context.is_dark_enough_to_spawn(BlockPos::new(0, 1, 0)));
        assert_eq!(rng.next_int(), JAVA_DRAWS[3]);
    }

    #[test]
    #[ignore = "daily"]
    fn a_panicking_non_mob_replacement_restores_draws_and_disables_only_itself() {
        // Install the process-wide logger only in an exact-selected child.
        const TEST: &str = "mobs::spawn_rules::mod_spawn_rule_tests::a_panicking_non_mob_replacement_restores_draws_and_disables_only_itself";
        const CHILD: &str = "SUNRISE_SPAWN_PLACEMENT_DIAGNOSTIC_TEST";
        if !std::env::var_os(CHILD).is_some_and(|value| value == TEST) {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([TEST, "--exact", "--ignored"])
                .env(CHILD, TEST)
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success() && stdout.contains("test result: ok. 1 passed;"),
                "{TEST} did not pass in its own process:\n{stdout}{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        const BROKEN: &str = "broken_arrow_placement";
        const HEALTHY: &str = "healthy_spawn_placement";
        crate::test_log::install();
        crate::test_log::clear();
        let calls = PredicateCalls::default();
        let failed_calls = Arc::clone(&calls);
        let mut level = Flat::new(&[
            SpawnPlacementDeclaration {
                id: "minecraft:arrow".into(),
                kind: entity_type_key(0),
                mod_id: HEALTHY,
                placement: Some(SpawnPlacementType::NoRestrictions),
                heightmap: Some(Heightmap::MotionBlockingNoLeaves),
                predicate: Arc::new(recording_predicate(&calls, "original", true)),
                operation: Replace,
            },
            SpawnPlacementDeclaration {
                id: "minecraft:arrow".into(),
                kind: entity_type_key(0),
                mod_id: BROKEN,
                placement: None,
                heightmap: None,
                predicate: Arc::new(
                    move |_: EntityTypeKey,
                          ctx: &mut dyn SpawnPlacementContext,
                          _: Reason,
                          _: BlockPos|
                          -> bool {
                        failed_calls
                            .lock()
                            .unwrap()
                            .push(("broken", ctx.random().next_int()));
                        panic!("test arrow predicate panic");
                    },
                ),
                operation: Replace,
            },
            SpawnPlacementDeclaration {
                id: "minecraft:shulker".into(),
                kind: entity_type_key(1),
                mod_id: HEALTHY,
                placement: None,
                heightmap: None,
                predicate: Arc::new(recording_predicate(&calls, "healthy", false)),
                operation: Replace,
            },
        ]);
        let arrow = SpawnEntityType {
            id: "minecraft:arrow",
            species: None,
            allowed_in_peaceful: true,
        };
        let mut rng = LegacyRandom::new(7);
        assert!(!check_spawn_rules(
            arrow,
            &level,
            Reason::Spawner,
            &mut rng,
            [0, 1, 0]
        ));
        assert_eq!(rng.next_int(), JAVA_DRAWS[0]);

        // The check hook can override the failed answer. Neither the disabled
        // replacement nor the original consumes a draw on this attempt.
        level.hook = Some(true);
        assert!(check_spawn_rules(
            arrow,
            &level,
            Reason::TrialSpawner,
            &mut rng,
            [0, 1, 0]
        ));
        assert_eq!(rng.next_int(), JAVA_DRAWS[1]);

        level.hook = None;
        let shulker = mob_kind("minecraft:shulker", Species::Shulker);
        assert!(!check_spawn_rules(
            (&shulker).into(),
            &level,
            Reason::Spawner,
            &mut rng,
            [0, 1, 0]
        ));
        assert_eq!(
            rng.next_int(),
            JAVA_DRAWS[3],
            "keep the healthy veto's draw"
        );
        assert_eq!(*level.heard.borrow(), [false, false, false]);
        assert_eq!(
            *calls.lock().unwrap(),
            [("broken", JAVA_DRAWS[0]), ("healthy", JAVA_DRAWS[2])]
        );
        let reports = crate::test_log::records();
        let panics: Vec<_> = reports
            .iter()
            .filter(|(level, message)| {
                *level == log::Level::Error && message.contains("spawn_placement_predicate")
            })
            .collect();
        assert_eq!(panics.len(), 1, "{panics:?}");
        assert!(panics[0].1.contains(BROKEN));
        assert!(panics[0].1.contains("test arrow predicate panic"));
    }

    /// `mob_spawn_placement_check` still runs after the rule and is told its
    /// answer as `default_result`; it may overturn it either way.
    #[test]
    fn the_per_check_hook_hears_the_rules_answer() {
        let mut level = Flat::new(&[ghoul(bright_ghoul)]);
        level.hook = Some(true);
        assert!(level.check("ruby:ghoul", Reason::Natural, [5, 2, 0]));
        level.hook = Some(false);
        assert!(!level.check("ruby:ghoul", Reason::Natural, [0, 1, 0]));
        assert_eq!(*level.heard.borrow(), [false, true]);
    }

    #[test]
    fn a_declared_placement_type_and_heightmap_replace_the_species_ones() {
        let level = Flat::new(&[SpawnPlacementDeclaration {
            id: "ruby:floater".to_owned(),
            kind: entity_type_key(1),
            mod_id: "ruby",
            placement: Some(SpawnPlacementType::NoRestrictions),
            heightmap: Some(Heightmap::OceanFloor),
            predicate: Arc::new(
                |_: EntityTypeKey, _: &mut dyn SpawnPlacementContext, _: Reason, _: BlockPos| true,
            ),
            operation: And,
        }]);
        // Mid-air with nothing below: no restrictions allows it, a zombie's
        // `ON_GROUND` does not, and the same type on grass is fine.
        assert!(is_spawn_position_ok(
            "ruby:floater",
            Species::Zombie,
            &level,
            3,
            10,
            0
        ));
        assert!(!is_spawn_position_ok(
            "ruby:undeclared",
            Species::Zombie,
            &level,
            3,
            10,
            0
        ));
        assert!(is_spawn_position_ok(
            "ruby:undeclared",
            Species::Zombie,
            &level,
            0,
            1,
            0
        ));
        assert_eq!(
            level.data.placement("ruby:floater"),
            Placement::NoRestrictions
        );
        assert_eq!(level.data.heightmap("ruby:floater"), Surface::OceanFloor);
        assert_eq!(
            level.data.heightmap("ruby:undeclared"),
            Surface::MotionBlockingNoLeaves
        );
    }

    #[test]
    fn composed_predicates_short_circuit_in_order_on_the_callers_random() {
        for (case, original, second_or, second_and, expected_calls, passes) in [
            (
                "the original succeeds, then an AND fails",
                true,
                false,
                false,
                &["original", "and-1", "and-2"][..],
                false,
            ),
            (
                "an OR succeeds, then an AND fails",
                false,
                true,
                false,
                &["original", "or-1", "or-2", "and-1", "and-2"][..],
                false,
            ),
            (
                "an OR and all ANDs succeed",
                false,
                true,
                true,
                &["original", "or-1", "or-2", "and-1", "and-2", "and-3"][..],
                true,
            ),
            (
                "neither the original nor any OR succeeds",
                false,
                false,
                true,
                &["original", "or-1", "or-2", "or-3"][..],
                false,
            ),
        ] {
            let calls = PredicateCalls::default();
            let declarations = [
                (Or, "original", original),
                (And, "and-1", true),
                (Or, "or-1", false),
                (And, "and-2", second_and),
                (Or, "or-2", second_or),
                (Or, "or-3", false),
                (And, "and-3", true),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, (operation, name, answer))| SpawnPlacementDeclaration {
                operation,
                placement: (i == 0).then_some(SpawnPlacementType::OnGround),
                heightmap: (i == 0).then_some(Heightmap::OceanFloor),
                ..ghoul(recording_predicate(&calls, name, answer))
            })
            .collect::<Vec<_>>();
            let level = Flat::new(&declarations);
            let mut rng = LegacyRandom::new(7);
            // The template zombie always passes for a trial spawner. The first
            // custom row has to be the original, not an OR after that template.
            let actual = check_spawn_rules(
                (&mob_kind("ruby:ghoul", Species::Zombie)).into(),
                &level,
                Reason::TrialSpawner,
                &mut rng,
                [0, 1, 0],
            );
            assert_eq!(actual, passes, "{case}");
            let expected: Vec<_> = expected_calls.iter().copied().zip(JAVA_DRAWS).collect();
            assert_eq!(*calls.lock().unwrap(), expected, "{case}");
            assert_eq!(rng.next_int(), JAVA_DRAWS[expected_calls.len()], "{case}");
        }
    }

    #[test]
    fn only_the_last_replacement_runs_without_resetting_geometry_or_changing_a_sibling() {
        let at @ [x, y, z] = [0, 8, 0];
        let position_ok = |entity: &str, level: &Flat| {
            is_spawn_position_ok(entity, Species::Zombie, level, x, y, z)
        };
        // Both answers matter: a passing replacement bypasses AND, and a failing one bypasses OR.
        for latest in [false, true] {
            let mut level = Flat::new(&[]);
            // Fill the shared species cache before installing an override for its vanilla type.
            assert!(!position_ok("minecraft:zombie", &level));
            let calls = PredicateCalls::default();
            let declarations = [
                ("and-before", "ruby", And, None, None, false),
                (
                    "first-replace",
                    "quartz",
                    Replace,
                    Some(SpawnPlacementType::NoRestrictions),
                    Some(Heightmap::OceanFloor),
                    !latest,
                ),
                ("or-between", "ruby", Or, None, None, true),
                ("last-replace", "jade", Replace, None, None, latest),
                ("and-after", "quartz", And, None, None, false),
                ("or-after", "ruby", Or, None, None, true),
            ]
            .map(|(name, mod_id, operation, placement, heightmap, answer)| {
                SpawnPlacementDeclaration {
                    id: "minecraft:zombie".to_owned(),
                    kind: entity_type_key(2),
                    mod_id,
                    placement,
                    heightmap,
                    predicate: Arc::new(recording_predicate(&calls, name, answer)),
                    operation,
                }
            });
            level.install(&declarations).unwrap();

            assert!(position_ok("minecraft:zombie", &level));
            assert_eq!(
                level.data.heightmap("minecraft:zombie"),
                Surface::OceanFloor
            );
            assert!(!position_ok("ruby:ghoul", &level));
            assert_eq!(
                level.data.heightmap("ruby:ghoul"),
                Surface::MotionBlockingNoLeaves
            );

            level.hook = Some(!latest);
            assert_eq!(
                level.check("minecraft:zombie", Reason::TrialSpawner, at),
                !latest
            );
            level.hook = None;
            assert!(level.check("ruby:ghoul", Reason::TrialSpawner, at));
            assert_eq!(*level.heard.borrow(), [latest, true]);
            assert_eq!(*calls.lock().unwrap(), [("last-replace", JAVA_DRAWS[0])]);
        }
    }

    #[test]
    fn registration_requires_prepared_vanilla_placement_membership() {
        let mut level = Flat::new(&[]);
        level.data.vanilla_spawn_placements = None;
        assert_eq!(
            level.install(&[ghoul(bright_ghoul)]).unwrap_err(),
            "mod ruby spawn placement for ruby:ghoul: prepare resources to load vanilla spawn placement membership"
        );
        assert_eq!(
            level.data.heightmap("ruby:ghoul"),
            Surface::MotionBlockingNoLeaves
        );
    }

    #[test]
    fn registration_validates_geometry_and_loaded_types() {
        let mut level = Flat::new(&[]);
        for (placement, heightmap) in [
            (Some(SpawnPlacementType::OnGround), None),
            (None, Some(Heightmap::OceanFloor)),
        ] {
            let row = SpawnPlacementDeclaration {
                placement,
                heightmap,
                ..ghoul(bright_ghoul)
            };
            assert_eq!(
                level.install(&[row]).unwrap_err(),
                "mod ruby spawn placement for ruby:ghoul: the first registration needs both a placement and a heightmap"
            );
            for operation in [And, Or] {
                let extension = SpawnPlacementDeclaration {
                    mod_id: "jade",
                    placement,
                    heightmap,
                    operation,
                    ..ghoul(bright_ghoul)
                };
                let original = SpawnPlacementDeclaration {
                    operation: And,
                    ..ghoul(bright_ghoul)
                };
                assert_eq!(
                    level.install(&[original, extension]).unwrap_err(),
                    "mod jade spawn placement for ruby:ghoul: only Replace may supply placement or heightmap for an existing entry"
                );
                assert_eq!(
                    level.data.heightmap("ruby:ghoul"),
                    Surface::MotionBlockingNoLeaves,
                    "the valid prefix of the refused batch must not publish its OceanFloor override"
                );
            }
        }
        let unknown = SpawnPlacementDeclaration {
            id: "ruby:missing".to_owned(),
            kind: entity_type_key(4),
            ..ghoul(bright_ghoul)
        };
        assert_eq!(
            level.install(&[unknown]).unwrap_err(),
            "mod ruby spawn placement for ruby:missing: no such loaded entity type"
        );

        let calls = PredicateCalls::default();
        let shulker = SpawnPlacementDeclaration {
            id: "minecraft:shulker".to_owned(),
            kind: entity_type_key(3),
            mod_id: "ruby",
            placement: None,
            heightmap: None,
            predicate: Arc::new(recording_predicate(&calls, "shulker-and", false)),
            operation: And,
        };
        level.install(&[shulker]).unwrap();
        assert_eq!(
            level.data.placement("minecraft:shulker"),
            Placement::NoRestrictions
        );
        assert!(!check_spawn_rules(
            (&mob_kind("minecraft:shulker", Species::Shulker)).into(),
            &level,
            Reason::TrialSpawner,
            &mut LegacyRandom::new(7),
            [0, 1, 0],
        ));
        assert_eq!(*calls.lock().unwrap(), [("shulker-and", JAVA_DRAWS[0])]);
    }

    #[test]
    fn peaceful_rejects_the_disallowed_species_before_predicate_hook_or_random_draw() {
        let calls = PredicateCalls::default();
        let mut level = Flat::new(&[ghoul(recording_predicate(&calls, "ghoul-replace", true))]);
        level.peaceful = true;
        level.hook = Some(true);
        let mut rng = LegacyRandom::new(7);
        assert!(!check_spawn_rules(
            (&mob_kind("ruby:ghoul", Species::Zombie)).into(),
            &level,
            Reason::Natural,
            &mut rng,
            [0, 1, 0],
        ));
        assert!(calls.lock().unwrap().is_empty());
        assert!(level.heard.borrow().is_empty());
        assert_eq!(rng.next_int(), JAVA_DRAWS[0]);
    }
}
