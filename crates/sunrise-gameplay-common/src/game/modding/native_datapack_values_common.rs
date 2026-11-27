//! Engine-side decoding and read-only evaluation of vanilla codec values (`IntProvider`,
//! `BlockStateProvider`, `BlockPredicate`, `HolderSet` for `minecraft:block`) inside mods'
//! custom datapack registries (#5368 part 2).
//!
//! Each load/reload (and each synced client snapshot) builds a fresh [`DatapackVanillaValues`]
//! table with a monotonically increasing non-zero generation number. Handles copied out of
//! `level.datapack_read` carry `(generation, index)`; after `/reload` or a client snapshot swap,
//! any older handle fails with [`mod_api::DatapackValueError::Stale`] rather than panicking.

use super::api_values::TryIntoApi;
use super::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
use super::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
use super::{Dim, SideGame};
use mod_api::{
    __private::VanillaValueDecoder, BlockKey, BlockPredicateValue, BlockState,
    BlockStateProviderValue, DatapackValueError, HolderSetValue, IntProviderValue, LevelRef,
    RandomSource as ModRandomSource,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use sunrise_core::dimension::Dimension as LevelDimension;
use sunrise_core::geometry::{Aabb, Direction, Vec3, VoxelShape};
use sunrise_core::identifier::Identifier;
use sunrise_core::position::BlockPos;
use sunrise_core::random::{
    InvalidBound, LegacyRandom, PositionalRandom, RandomSource as EngineRandomSource,
};
use sunrise_items::blocks::BlockFacts;
use sunrise_level::biome::Biomes;
use sunrise_level::world::World;
use sunrise_world::fluid::FluidKind;
use sunrise_world::heightmap::HeightmapType;
use sunrise_world::holder_set::{self, HolderLookup};
use sunrise_world::level::{Behaviours, BlockEntityNeighbors, LevelAccess, Ticks};
use sunrise_world::registry::{BlockRegistry, BlockStateId};
use sunrise_world::worldgen::anchor::WorldGenerationContext;
use sunrise_world::worldgen::biome::BiomeId;
use sunrise_world::worldgen::biome_predicate::BiomePredicateBindings;
use sunrise_world::worldgen::block_feature::integer_properties;
use sunrise_world::worldgen::block_predicate::BlockPredicate;
use sunrise_world::worldgen::feature::FeatureWorld;
use sunrise_world::worldgen::placement::{PlacementContext, PlacementQueryError};
use sunrise_world::worldgen::state_provider::{
    BlockStateProvider, StateProviderBindings, StateProviderCatalog,
};
use sunrise_world::worldgen::value_provider::IntProvider;

/// Monotonic non-zero generation counter shared across loads; generation `0` is reserved for
/// unbound handles and never matches any live table.
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

/// One load's decoded vanilla codec values, swapped atomically on `/reload` and client snapshot
/// replacement.
pub struct DatapackVanillaValues {
    generation: u64,
    int_providers: Vec<IntProvider>,
    block_state_providers: Vec<BlockStateProvider>,
    block_predicates: Vec<BlockPredicate>,
    block_holder_sets: Vec<BTreeSet<BlockStateId>>,
    biome_ids: BTreeMap<String, BiomeId>,
}

impl DatapackVanillaValues {
    fn int_provider(&self, value: IntProviderValue) -> Result<&IntProvider, DatapackValueError> {
        if self.generation != value.generation() {
            return Err(DatapackValueError::Stale);
        }
        self.int_providers
            .get(value.index() as usize)
            .ok_or(DatapackValueError::Stale)
    }

    fn block_state_provider(
        &self,
        value: BlockStateProviderValue,
    ) -> Result<&BlockStateProvider, DatapackValueError> {
        if self.generation != value.generation() {
            return Err(DatapackValueError::Stale);
        }
        self.block_state_providers
            .get(value.index() as usize)
            .ok_or(DatapackValueError::Stale)
    }

    fn block_predicate(
        &self,
        value: BlockPredicateValue,
    ) -> Result<&BlockPredicate, DatapackValueError> {
        if self.generation != value.generation() {
            return Err(DatapackValueError::Stale);
        }
        self.block_predicates
            .get(value.index() as usize)
            .ok_or(DatapackValueError::Stale)
    }

    fn block_holder_set(
        &self,
        value: HolderSetValue,
    ) -> Result<&BTreeSet<BlockStateId>, DatapackValueError> {
        if self.generation != value.generation() {
            return Err(DatapackValueError::Stale);
        }
        self.block_holder_sets
            .get(value.index() as usize)
            .ok_or(DatapackValueError::Stale)
    }
}

/// Decodes vanilla codec JSON during `DatapackCandidate::build` or client snapshot
/// re-binding, accumulating live objects into one [`DatapackVanillaValues`] table.
pub struct VanillaValueTableBuilder<'a> {
    generation: u64,
    bindings: StateProviderBindings<'static>,
    catalog: Option<&'a StateProviderCatalog>,
    block_tags: BTreeMap<String, Vec<String>>,
    biome_ids: BTreeMap<String, BiomeId>,
    int_providers: Vec<IntProvider>,
    block_state_providers: Vec<BlockStateProvider>,
    block_predicates: Vec<BlockPredicate>,
    block_holder_sets: Vec<BTreeSet<BlockStateId>>,
}

impl<'a> VanillaValueTableBuilder<'a> {
    pub fn new(
        registry: &'static BlockRegistry,
        behaviours: &Behaviours,
        block_tags: &HashMap<String, HashSet<String>>,
        fluid_tags: &HashMap<String, HashSet<String>>,
        biome_names: &[&str],
        biome_tags: &HashMap<String, HashSet<String>>,
        catalog: Option<&'a StateProviderCatalog>,
    ) -> Result<Self, String> {
        let resolved_block_tags = resolve_block_tags(registry, block_tags);
        let resolved_fluid_tags = resolve_fluid_tags(fluid_tags);
        let biome_ids: BTreeMap<String, BiomeId> = biome_names
            .iter()
            .enumerate()
            .map(|(i, name)| ((*name).to_owned(), BiomeId(i as u32)))
            .collect();
        let resolved_biome_tags = resolve_biome_tags(&biome_ids, biome_tags);
        let biomes = Arc::new(
            BiomePredicateBindings::new(&biome_ids, &resolved_biome_tags)
                .map_err(|e| format!("biome predicate bindings: {e}"))?,
        );
        let bindings = StateProviderBindings::new(
            registry,
            integer_properties(registry),
            &resolved_block_tags,
        )
        .map_err(|e| format!("state provider bindings: {e}"))?
        .with_bound_properties(behaviours.bound_physical_properties())
        .with_fluid_tags(&resolved_fluid_tags)
        .map_err(|e| format!("fluid predicate bindings: {e}"))?
        .with_biomes(biomes);
        let generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
        Ok(Self {
            generation,
            bindings,
            catalog,
            block_tags: resolved_block_tags,
            biome_ids,
            int_providers: Vec::new(),
            block_state_providers: Vec::new(),
            block_predicates: Vec::new(),
            block_holder_sets: Vec::new(),
        })
    }

    pub fn finish(self) -> DatapackVanillaValues {
        DatapackVanillaValues {
            generation: self.generation,
            int_providers: self.int_providers,
            block_state_providers: self.block_state_providers,
            block_predicates: self.block_predicates,
            block_holder_sets: self.block_holder_sets,
            biome_ids: self.biome_ids,
        }
    }
}

pub fn resolve_block_tags(
    registry: &BlockRegistry,
    tags: &HashMap<String, HashSet<String>>,
) -> BTreeMap<String, Vec<String>> {
    tags.iter()
        .filter_map(|(tag, members)| {
            let key = Identifier::parse(tag).ok()?.to_string();
            let mut entries: Vec<String> = members
                .iter()
                .filter_map(|member| {
                    let canonical = Identifier::parse(member).ok()?.to_string();
                    registry.default_state(&canonical).map(|_| canonical)
                })
                .collect();
            entries.sort_unstable();
            entries.dedup();
            Some((key, entries))
        })
        .collect()
}

fn resolve_fluid_tags(tags: &HashMap<String, HashSet<String>>) -> BTreeMap<String, Vec<String>> {
    tags.iter()
        .filter_map(|(tag, members)| {
            let key = Identifier::parse(tag).ok()?.to_string();
            let mut entries: Vec<String> = members
                .iter()
                .filter_map(|member| {
                    let canonical = Identifier::parse(member).ok()?.to_string();
                    FluidKind::from_name(&canonical).map(|_| canonical)
                })
                .collect();
            entries.sort_unstable();
            entries.dedup();
            Some((key, entries))
        })
        .collect()
}

fn resolve_biome_tags(
    biomes: &BTreeMap<String, BiomeId>,
    tags: &HashMap<String, HashSet<String>>,
) -> BTreeMap<String, Vec<String>> {
    tags.iter()
        .filter_map(|(tag, members)| {
            let key = Identifier::parse(tag).ok()?.to_string();
            let mut entries: Vec<String> = members
                .iter()
                .filter_map(|member| {
                    let canonical = Identifier::parse(member).ok()?.to_string();
                    biomes.contains_key(&canonical).then_some(canonical)
                })
                .collect();
            entries.sort_unstable();
            entries.dedup();
            Some((key, entries))
        })
        .collect()
}

struct BlockHolderLookup<'a> {
    registry: &'a BlockRegistry,
    tags: &'a BTreeMap<String, Vec<String>>,
}

impl HolderLookup for BlockHolderLookup<'_> {
    type Id = BlockStateId;

    fn what(&self) -> &str {
        "block"
    }

    fn entry(&self, id: &str) -> Result<Vec<BlockStateId>, String> {
        let canonical = Identifier::parse(id)
            .map_err(|_| format!("invalid block identifier {id:?}"))?
            .to_string();
        let state = self
            .registry
            .default_state(&canonical)
            .ok_or_else(|| format!("unknown block {canonical}"))?;
        Ok(vec![state])
    }

    fn tag(&self, tag: &str) -> Result<Vec<BlockStateId>, String> {
        let canonical = Identifier::parse(tag)
            .map_err(|_| format!("invalid block tag {tag:?}"))?
            .to_string();
        let members = self
            .tags
            .get(&canonical)
            .ok_or_else(|| format!("missing block tag #{canonical}"))?;
        members
            .iter()
            .map(|name| {
                self.registry
                    .default_state(name)
                    .ok_or_else(|| format!("unknown block {name} in tag #{canonical}"))
            })
            .collect()
    }

    fn all(&self) -> Result<Vec<BlockStateId>, String> {
        let mut defaults = BTreeSet::new();
        for index in 0..self.registry.len() {
            if let Some(state) = self.registry.state(BlockStateId(index as u32))
                && let Some(default) = self.registry.default_state(&state.name)
            {
                defaults.insert(default);
            }
        }
        Ok(defaults.into_iter().collect())
    }
}

impl VanillaValueDecoder for VanillaValueTableBuilder<'_> {
    fn decode_int_provider(
        &mut self,
        json: &str,
        min: i32,
        max: i32,
    ) -> Result<IntProviderValue, String> {
        let provider = IntProvider::from_json(json).map_err(|e| e.to_string())?;
        // Matches vanilla `IntProviders.validate(minValue, maxValue, value)`.
        let [lo, hi] = provider.bounds();
        if lo < min || hi > max {
            return Err(format!(
                "int provider bounds [{lo}, {hi}] outside [{min}, {max}]"
            ));
        }
        let index = u32::try_from(self.int_providers.len())
            .map_err(|_| "int provider table overflow".to_owned())?;
        self.int_providers.push(provider);
        Ok(IntProviderValue::from_raw(self.generation, index))
    }

    fn decode_block_state_provider(
        &mut self,
        json: &str,
    ) -> Result<BlockStateProviderValue, String> {
        let provider = match self.catalog {
            Some(catalog) => catalog.provider(json, &self.bindings),
            None => BlockStateProvider::from_json(json, &self.bindings),
        }
        .map_err(|e| e.to_string())?;
        let index = u32::try_from(self.block_state_providers.len())
            .map_err(|_| "block state provider table overflow".to_owned())?;
        self.block_state_providers.push(provider);
        Ok(BlockStateProviderValue::from_raw(self.generation, index))
    }

    fn decode_block_predicate(&mut self, json: &str) -> Result<BlockPredicateValue, String> {
        let predicate = BlockPredicate::from_json_with_blocks(json, &self.bindings)
            .map_err(|e| e.to_string())?;
        let index = u32::try_from(self.block_predicates.len())
            .map_err(|_| "block predicate table overflow".to_owned())?;
        self.block_predicates.push(predicate);
        Ok(BlockPredicateValue::from_raw(self.generation, index))
    }

    fn decode_holder_set(&mut self, registry: &str, json: &str) -> Result<HolderSetValue, String> {
        if registry != "minecraft:block" {
            return Err(format!("unsupported holder set registry {registry}"));
        }
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|e| format!("invalid holder set json: {e}"))?;
        let lookup = BlockHolderLookup {
            registry: self.bindings.registry(),
            tags: &self.block_tags,
        };
        let states: BTreeSet<BlockStateId> =
            holder_set::resolve(&value, &lookup)?.into_iter().collect();
        let index = u32::try_from(self.block_holder_sets.len())
            .map_err(|_| "holder set table overflow".to_owned())?;
        self.block_holder_sets.push(states);
        Ok(HolderSetValue::from_raw(self.generation, index))
    }
}

/// Adapts a mod's `&mut dyn mod_api::RandomSource` to `sunrise_core::random::RandomSource`,
/// validating bounds so a custom mod RNG returning out-of-range values never panics engine
/// indexing.
struct ModRandom<'a>(&'a mut dyn ModRandomSource);

/// Unreachable positional factory satisfying `EngineRandomSource::Positional`; neither
/// `IntProvider::sample` nor `BlockStateProvider::get_state` forks a positional random.
struct ModPositional<'a>(std::marker::PhantomData<&'a ()>);

impl<'a> PositionalRandom for ModPositional<'a> {
    type Source = ModRandom<'a>;
    fn at(&self, _: BlockPos) -> Self::Source {
        unreachable!("IntProvider and BlockStateProvider never fork a positional RandomSource")
    }
    fn from_seed(&self, _: i64) -> Self::Source {
        unreachable!("IntProvider and BlockStateProvider never fork a positional RandomSource")
    }
    fn from_hash_of(&self, _: &str) -> Self::Source {
        unreachable!("IntProvider and BlockStateProvider never fork a positional RandomSource")
    }
    fn from_hash_of_utf16(&self, _: &[u16]) -> Self::Source {
        unreachable!("IntProvider and BlockStateProvider never fork a positional RandomSource")
    }
}

impl<'a> EngineRandomSource for ModRandom<'a> {
    type Positional = ModPositional<'a>;

    fn set_seed(&mut self, _seed: i64) {}

    fn next_int(&mut self) -> i32 {
        self.0.next_int()
    }

    fn next_int_bound(&mut self, bound: i32) -> Result<i32, InvalidBound> {
        if bound <= 0 {
            return Err(InvalidBound);
        }
        let value = self.0.next_int_bound(bound);
        if !(0..bound).contains(&value) {
            return Err(InvalidBound);
        }
        Ok(value)
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

    fn fork_positional(&mut self) -> Self::Positional {
        ModPositional(std::marker::PhantomData)
    }

    fn fork(&mut self) -> Self {
        unreachable!("IntProvider and BlockStateProvider never fork a RandomSource")
    }
}

/// Read-only `LevelAccess` over one loaded `World` column, used only by `BlockPredicate::WouldSurvive`
/// (`Behaviours::can_survive`). Never writes.
struct ReadOnlyLevel<'a> {
    world: &'a World,
    facts: &'a BlockFacts,
    biomes: &'a Biomes,
    dimension: LevelDimension,
    sea_level: i32,
    /// Only `LevelAccess::random` asks for it. `WouldSurvivePredicate.test` calls
    /// `BlockState.canSurvive(LevelReader, BlockPos)`, which takes no random and draws none, and
    /// `can_survive` never reads this stream, so its seed is not observable.
    random: LegacyRandom,
}

impl BlockEntityNeighbors for ReadOnlyLevel<'_> {}

impl LevelAccess for ReadOnlyLevel<'_> {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.world.block(pos.x, pos.y, pos.z)
    }

    fn set_raw(&mut self, _pos: BlockPos, _state: BlockStateId) -> Option<BlockStateId> {
        None
    }

    fn is_loaded(&self, pos: BlockPos) -> bool {
        self.world.loaded(pos.x, pos.z)
    }

    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }

    fn game_time(&self) -> i64 {
        self.world.game_time
    }

    fn min_y(&self) -> i32 {
        self.world.shape.min_y
    }

    fn max_y(&self) -> i32 {
        self.world.shape.max_y()
    }

    fn sea_level(&self) -> i32 {
        self.sea_level
    }

    fn sky_light(&self, pos: BlockPos) -> i32 {
        i32::from(self.world.light(pos.x, pos.y, pos.z) >> 4)
    }

    fn block_light(&self, pos: BlockPos) -> i32 {
        i32::from(self.world.light(pos.x, pos.y, pos.z) & 0x0f)
    }

    fn world_surface(&self, x: i32, z: i32) -> i32 {
        self.world.world_surface(x, z, |s| self.facts.is_air(s))
    }

    fn biome_name(&self, pos: BlockPos) -> Option<&str> {
        let chunk = self.world.chunks.get(&(pos.x >> 4, pos.z >> 4))?;
        Some(&self.biomes.get(chunk.biome(pos.x, pos.y, pos.z)).name)
    }

    fn dimension(&self) -> LevelDimension {
        self.dimension
    }
}

/// Side-specific state and obstruction hooks for datapack vanilla-value evaluation.
pub trait DatapackValuesSide<'g>: SideGame<'g> {
    fn active_datapack_vanilla_values(&self) -> Option<&DatapackVanillaValues>;
    fn datapack_block_tags(&self) -> &HashMap<String, HashSet<String>>;
    fn is_datapack_block_unobstructed(
        &self,
        level: LevelRef,
        world: &World,
        position: [i32; 3],
    ) -> Result<bool, PlacementQueryError>;
}

/// Shared cell AABB and lazy block-shape blocker predicate for `is_datapack_block_unobstructed`.
pub fn datapack_cell_and_blocker(position: [i32; 3]) -> (Aabb, impl FnMut(Aabb) -> bool) {
    let cell = Aabb::new(
        Vec3::new(
            f64::from(position[0]),
            f64::from(position[1]),
            f64::from(position[2]),
        ),
        Vec3::new(
            f64::from(position[0]) + 1.0,
            f64::from(position[1]) + 1.0,
            f64::from(position[2]) + 1.0,
        ),
    );
    let mut cell_shape: Option<VoxelShape> = None;
    let obstructs = move |blocker: Aabb| {
        if !blocker.intersects(cell) {
            return false;
        }
        let shape = cell_shape.get_or_insert_with(|| {
            VoxelShape::block().moved(Vec3::new(
                f64::from(position[0]),
                f64::from(position[1]),
                f64::from(position[2]),
            ))
        });
        sunrise_sim::mobs::spawn_selection::blocks_spawn_shape(shape, cell, blocker)
    };
    (cell, obstructs)
}

/// Active-world entity obstruction check shared by both sides.
pub fn datapack_active_world_entities_unobstructed<'g, G: SideGame<'g>>(
    game: &G,
    level: LevelRef,
    cell: Aabb,
    mut obstructs: impl FnMut(Aabb) -> bool,
    skip_mob: impl Fn(&sunrise_sim::mobs::Mob) -> bool,
) -> Result<bool, PlacementQueryError> {
    let common = game.common();
    let mut mob_obstructs = false;
    let _ = game
        .level_visit_entities_in(
            level,
            cell.min_position().components(),
            cell.max_position().components(),
            &mut |entity| {
                let Some(index) = common.mob_index.find_ref(&common.mobs, entity.id()) else {
                    return std::ops::ControlFlow::Continue(());
                };
                let mob = &common.mobs[index];
                if !mob.removed && !skip_mob(mob) && obstructs(mob.bounds()) {
                    mob_obstructs = true;
                    return std::ops::ControlFlow::Break(());
                }
                std::ops::ControlFlow::Continue(())
            },
        )
        .map_err(|_| PlacementQueryError::Unavailable)?;
    if mob_obstructs
        || common
            .dragon
            .as_ref()
            .is_some_and(|dragon| obstructs(dragon.body_bounds()))
    {
        return Ok(false);
    }
    let entities = crate::game::entity::entity_lists_of!(common);
    Ok(!entities
        .non_mob_building_blocker_boxes(
            common
                .carts
                .iter()
                .map(crate::game::minecarts::Minecart::bounds),
            &common.crystals,
        )
        .any(obstructs))
}

/// Read-only `FeatureWorld + PlacementContext` view over a live level (server active/parked or
/// client level) for evaluating `BlockPredicate::test` and `BlockStateProvider::get_state`.
pub struct DatapackReadWorld<'a, 'g, G: DatapackValuesSide<'g>> {
    game: &'a G,
    level: LevelRef,
    world: &'a World,
    behaviours: &'a Behaviours,
    registry: &'static BlockRegistry,
    facts: &'a BlockFacts,
    biomes: &'a Biomes,
    biome_ids: &'a BTreeMap<String, BiomeId>,
    block_tags: &'a HashMap<String, HashSet<String>>,
    dimension: LevelDimension,
    sea_level: i32,
    _marker: std::marker::PhantomData<&'g ()>,
}

impl<'g, G: DatapackValuesSide<'g>> FeatureWorld for DatapackReadWorld<'_, 'g, G> {
    type Error = PlacementQueryError;

    fn block(&mut self, position: [i32; 3]) -> Result<BlockStateId, Self::Error> {
        Ok(self.world.block(position[0], position[1], position[2]))
    }

    fn set_block(
        &mut self,
        _position: [i32; 3],
        _state: BlockStateId,
        _flags: i32,
    ) -> Result<bool, Self::Error> {
        Err(PlacementQueryError::Unavailable)
    }
}

impl<'g, G: DatapackValuesSide<'g>> PlacementContext for DatapackReadWorld<'_, 'g, G> {
    fn block_registry(&self) -> Option<&'static BlockRegistry> {
        Some(self.registry)
    }

    fn block_state(&mut self, position: [i32; 3]) -> Result<BlockStateId, PlacementQueryError> {
        Ok(self.world.block(position[0], position[1], position[2]))
    }

    fn block_survives(
        &mut self,
        state: BlockStateId,
        position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        let mut level = ReadOnlyLevel {
            world: self.world,
            facts: self.facts,
            biomes: self.biomes,
            dimension: self.dimension,
            sea_level: self.sea_level,
            random: LegacyRandom::new(0),
        };
        let mut ticks = Ticks::default();
        Ok(self.behaviours.can_survive(
            &mut level,
            &mut ticks,
            BlockPos::new(position[0], position[1], position[2]),
            state,
        ))
    }

    fn face_sturdy(
        &mut self,
        position: [i32; 3],
        direction: Direction,
    ) -> Result<bool, PlacementQueryError> {
        let state = self.world.block(position[0], position[1], position[2]);
        Ok(self.behaviours.is_face_sturdy(state, direction))
    }

    fn unobstructed_block(&mut self, position: [i32; 3]) -> Result<bool, PlacementQueryError> {
        self.game
            .is_datapack_block_unobstructed(self.level, self.world, position)
    }

    fn biome_identity(&mut self, position: [i32; 3]) -> Result<BiomeId, PlacementQueryError> {
        let chunk = self
            .world
            .chunks
            .get(&(position[0] >> 4, position[2] >> 4))
            .ok_or(PlacementQueryError::Unavailable)?;
        let name = &self
            .biomes
            .get(chunk.biome(position[0], position[1], position[2]))
            .name;
        self.biome_ids
            .get(name)
            .copied()
            .ok_or(PlacementQueryError::Unavailable)
    }

    fn block_matches_tag(
        &mut self,
        position: [i32; 3],
        tag: &str,
    ) -> Result<bool, PlacementQueryError> {
        let state = self.world.block(position[0], position[1], position[2]);
        let Some(info) = self.registry.state(state) else {
            return Ok(false);
        };
        let key = tag.trim_start_matches('#');
        let canonical = Identifier::parse(key)
            .map(|id| id.to_string())
            .unwrap_or_else(|_| key.to_owned());
        Ok(self
            .block_tags
            .get(&canonical)
            .is_some_and(|members| members.contains(&info.name)))
    }

    fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(self.world.shape.max_y() - 1)
    }

    fn sea_level(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(self.sea_level)
    }

    fn generation(&self) -> WorldGenerationContext {
        let min_y = self.world.shape.min_y;
        WorldGenerationContext::new(min_y, self.world.shape.max_y() - min_y, self.sea_level)
    }

    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(self.world.shape.min_y)
    }

    fn height(
        &mut self,
        _kind: HeightmapType,
        _x: i32,
        _z: i32,
    ) -> Result<i32, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
}

pub trait GameDatapackValues<'g>: DatapackValuesSide<'g> {
    fn datapack_read_world<'a>(
        &'a self,
        level: LevelRef,
        table: &'a DatapackVanillaValues,
    ) -> Result<DatapackReadWorld<'a, 'g, Self>, DatapackValueError>
    where
        Self: Sized,
        'g: 'a,
    {
        let common = self.common();
        let world = self
            .native_level_read_world(level)
            .ok_or(DatapackValueError::Unavailable)?;
        let dimension = Dim::from_id(level.dimension_id()).level();
        let sea_level = Dim::sea_level_of_id(level.dimension_id());
        let biomes = common
            .world_biomes
            .as_deref()
            .unwrap_or_else(|| common.res.biomes());
        let block_tags = self.datapack_block_tags();
        Ok(DatapackReadWorld {
            game: self,
            level,
            world,
            behaviours: common.res.behaviours(),
            registry: common.pack.assets.registry,
            facts: common.res.facts(),
            biomes,
            biome_ids: &table.biome_ids,
            block_tags,
            dimension,
            sea_level,
            _marker: std::marker::PhantomData,
        })
    }

    fn datapack_int_provider_sample(
        &self,
        value: IntProviderValue,
        random: &mut dyn ModRandomSource,
    ) -> Result<i32, DatapackValueError> {
        let table = self
            .active_datapack_vanilla_values()
            .ok_or(DatapackValueError::Stale)?;
        let provider = table.int_provider(value)?;
        provider
            .sample(&mut ModRandom(random))
            .map_err(|_| DatapackValueError::EvaluationFailed)
    }

    fn datapack_block_state_provider_state(
        &self,
        value: BlockStateProviderValue,
        level: LevelRef,
        random: &mut dyn ModRandomSource,
        pos: [i32; 3],
    ) -> Result<BlockState, DatapackValueError>
    where
        Self: Sized,
    {
        let table = self
            .active_datapack_vanilla_values()
            .ok_or(DatapackValueError::Stale)?;
        let provider = table.block_state_provider(value)?;
        let mut view = self.datapack_read_world(level, table)?;
        let state = provider
            .get_state(&mut view, &mut ModRandom(random), pos)
            .map_err(|err| match err {
                sunrise_world::worldgen::state_provider::StateProviderError::Query(_)
                | sunrise_world::worldgen::state_provider::StateProviderError::World(_) => {
                    DatapackValueError::Unavailable
                }
                _ => DatapackValueError::EvaluationFailed,
            })?;
        state
            .try_into_api()
            .map_err(|_| DatapackValueError::EvaluationFailed)
    }

    fn datapack_block_predicate_test(
        &self,
        value: BlockPredicateValue,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Result<bool, DatapackValueError>
    where
        Self: Sized,
    {
        let table = self
            .active_datapack_vanilla_values()
            .ok_or(DatapackValueError::Stale)?;
        let predicate = table.block_predicate(value)?;
        let mut view = self.datapack_read_world(level, table)?;
        predicate
            .test(&mut view, pos)
            .map_err(|_| DatapackValueError::Unavailable)
    }

    fn datapack_holder_set_contains(
        &self,
        value: HolderSetValue,
        block: BlockKey,
    ) -> Result<bool, DatapackValueError> {
        let table = self
            .active_datapack_vanilla_values()
            .ok_or(DatapackValueError::Stale)?;
        let set = table.block_holder_set(value)?;
        let default = self
            .common()
            .block_state_default(block)
            .ok_or(DatapackValueError::UnknownBlock)?;
        Ok(set.contains(&default))
    }
}

impl<'g, G: DatapackValuesSide<'g>> GameDatapackValues<'g> for G {}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{LegacyRandomSource, RandomSource};

    /// A hostile mod RNG that returns an out-of-range value from `next_int_bound`: must return
    /// `EvaluationFailed` rather than panicking inside engine indexing.
    struct HostileRng;
    impl RandomSource for HostileRng {
        fn next_int(&mut self) -> i32 {
            0
        }
        fn next_int_bound(&mut self, bound: i32) -> i32 {
            bound
        }
        fn next_long(&mut self) -> i64 {
            0
        }
        fn next_boolean(&mut self) -> bool {
            false
        }
        fn next_float(&mut self) -> f32 {
            0.0
        }
        fn next_double(&mut self) -> f64 {
            0.0
        }
        fn next_gaussian(&mut self) -> f64 {
            0.0
        }
    }

    #[test]
    fn datapack_vanilla_values_table_rejects_stale_and_hostile_rng() {
        let table = DatapackVanillaValues {
            generation: 2,
            int_providers: vec![
                IntProvider::from_json(
                    r#"{"type":"minecraft:uniform","min_inclusive":1,"max_inclusive":4}"#,
                )
                .unwrap(),
            ],
            block_state_providers: Vec::new(),
            block_predicates: vec![
                BlockPredicate::from_json(r#"{"type":"minecraft:true"}"#).unwrap(),
            ],
            block_holder_sets: vec![BTreeSet::from([BlockStateId(7)])],
            biome_ids: BTreeMap::new(),
        };
        // Earlier generation and out-of-bounds index both return Stale.
        assert_eq!(
            table.int_provider(IntProviderValue::from_raw(1, 0)).err(),
            Some(DatapackValueError::Stale)
        );
        assert_eq!(
            table.int_provider(IntProviderValue::from_raw(2, 1)).err(),
            Some(DatapackValueError::Stale)
        );
        assert_eq!(
            table
                .block_state_provider(BlockStateProviderValue::from_raw(1, 0))
                .err(),
            Some(DatapackValueError::Stale)
        );
        assert_eq!(
            table
                .block_predicate(BlockPredicateValue::from_raw(1, 0))
                .err(),
            Some(DatapackValueError::Stale)
        );
        assert_eq!(
            table.block_holder_set(HolderSetValue::from_raw(1, 0)).err(),
            Some(DatapackValueError::Stale)
        );
        // Live generation succeeds, and a hostile RNG returns Err without panicking.
        let provider = table
            .int_provider(IntProviderValue::from_raw(2, 0))
            .unwrap();
        let mut hostile = HostileRng;
        assert!(provider.sample(&mut ModRandom(&mut hostile)).is_err());
        let mut ok_rng = LegacyRandomSource::new(42);
        let value = provider.sample(&mut ModRandom(&mut ok_rng)).unwrap();
        assert!((1..=4).contains(&value));
    }
}
