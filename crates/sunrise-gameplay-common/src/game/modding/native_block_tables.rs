//! The read-only tables behind `BlockStates`: the facts a block state carries, read from the
//! game's shared resources alone. `Game` and every level view hand out this one view, so a part
//! reads the same facts in a level callback as through a `Host`.
use super::{
    CommonGame, PackResources, Resources, WorldRest,
    api_values::TryIntoEngine,
    native_world_read_common::{api_fluid_state, engine_fluid_kind},
};
use mod_api::{
    BlockKey, BlockLanding, BlockSoundType, BlockState, BlockStateRange, FluidDefinition, FluidKey,
    FluidState, FluidStateError, FluidTypeProperties, PathType, StateCollision, StateConstraint,
    StateMatchError, StateProperties, WeatheringAge,
};
use sunrise_core::geometry::VoxelShape;
use sunrise_items::items::ItemStack;
use sunrise_mod_package::content::FluidMeasures;
use sunrise_world::fluid::FluidKind;
use sunrise_world::level::{fluid_drip_particle, fluid_explosion_resistance, fluid_randomly_ticks};
use sunrise_world::registry::BlockStateId;

/// Borrowed views of the game's block registry, facts and behaviour tables.
#[derive(Clone, Copy)]
pub struct BlockTables<'a> {
    res: &'a Resources,
    pack: &'a PackResources,
}

impl<'a> BlockTables<'a> {
    pub fn new(res: &'a Resources, pack: &'a PackResources) -> Self {
        Self { res, pack }
    }

    pub fn of(game: &'a CommonGame<'_>) -> Self {
        Self::new(&game.res, &game.pack)
    }

    /// The item table the generated forwarders convert item values with.
    pub(in crate::game) fn items(&self) -> &sunrise_items::items::Items {
        self.res.items()
    }

    /// `BlockState.getShape` with an empty getter at the origin (`EmptyBlockGetter.INSTANCE`,
    /// `BlockPos.ZERO`): the state's outline boxes in block-local space. Empty for a state with
    /// no outline (air); None for an unknown state.
    pub fn block_state_outline(
        &self,
        state: BlockStateId,
    ) -> Result<Option<&'a [[f64; 6]]>, mod_api::LevelReadError> {
        Ok(self
            .pack
            .assets
            .registry
            .state(state)
            .map(|_| self.res.facts().get(state).outline.as_ref()))
    }

    /// `BlockState.getFluidState`: the fluid state `state` holds (water for a waterlogged state,
    /// the empty fluid for most). None for an unknown state.
    pub fn block_state_fluid_state(
        &self,
        state: BlockStateId,
    ) -> Result<Option<FluidState>, mod_api::LevelReadError> {
        Ok(self
            .pack
            .assets
            .registry
            .state(state)
            .and_then(|_| api_fluid_state(self.res.behaviours().fluid(state))))
    }

    /// `Fluid.isSame`: whether `fluid` and `other` are the same fluid family. False when either
    /// key has no world fluid.
    pub fn fluid_is_same(
        &self,
        fluid: FluidKey,
        other: FluidKey,
    ) -> Result<bool, mod_api::LevelReadError> {
        Ok(match (engine_fluid_kind(fluid), engine_fluid_kind(other)) {
            (Some(fluid), Some(other)) => fluid.same_family(other),
            _ => false,
        })
    }

    /// `Fluid.getBucket`: the bucket item that picks `fluid` up. None for `minecraft:empty`,
    /// for fluids without a bucket, for a named item no item has, and for one of the game's own
    /// buckets that would not keep it (the empty bucket, another fluid's or a mob's).
    pub fn fluid_bucket(
        &self,
        fluid: FluidKey,
    ) -> Result<Option<mod_api::ItemKey>, mod_api::LevelReadError> {
        let Some(fluid_id) = mod_api::__private::fluid_id(fluid) else {
            return Ok(None);
        };
        let kind = FluidKind::from_name(fluid_id);
        let Some(item) = (match kind {
            Some(FluidKind::Mod(mod_fluid) | FluidKind::ModFlowing(mod_fluid)) => {
                sunrise_sim::mod_content::fluid_bucket(&mod_fluid.get().id)
                    .and_then(|bucket| mod_api::__private::item_key(&bucket))
            }
            Some(kind) => {
                sunrise_world::level::fluid_bucket(kind).and_then(mod_api::__private::item_key)
            }
            None => sunrise_sim::mod_content::fluid_bucket(fluid_id)
                .and_then(|bucket| mod_api::__private::item_key(&bucket)),
        }) else {
            return Ok(None);
        };
        // The game's own buckets keep what they hold: filling the empty one, or one of another
        // fluid or a mob, would lose this fluid.
        let Some(id) = mod_api::__private::item_key_id(item) else {
            return Ok(None);
        };
        Ok(match sunrise_items::items::bucket_contents(id) {
            Some((sunrise_world::level::BucketContent::Fluid(held), ..))
                if kind.is_some_and(|kind| kind.same_family(held)) =>
            {
                Some(item)
            }
            Some(_) => None,
            None if id == "minecraft:bucket" => None,
            None => Some(item),
        })
    }

    /// `BucketItem.content`: the fluid a filled bucket the game registers holds.
    pub fn bucket_fluid(
        &self,
        bucket: mod_api::ItemKey,
    ) -> Result<Option<FluidKey>, mod_api::LevelReadError> {
        let Some(item) = mod_api::__private::item_key_id(bucket) else {
            return Ok(None);
        };
        Ok(match sunrise_items::items::bucket_contents(item) {
            Some((sunrise_world::level::BucketContent::Fluid(kind), ..)) => {
                mod_api::__private::fluid_key(kind.name())
            }
            _ => None,
        })
    }

    /// `Fluid.getPickupSound`: the sound event a bucket plays as it picks `fluid` up.
    pub fn fluid_pickup_sound(
        &self,
        fluid: FluidKey,
    ) -> Result<Option<&'static str>, mod_api::LevelReadError> {
        Ok(match engine_fluid_kind(fluid) {
            Some(kind) => sunrise_world::level::fluid_pickup_sound(kind),
            None => self
                .fluid_bucket(fluid)?
                .map(|_| "minecraft:item.bucket.fill"),
        })
    }

    /// `BucketItem.playEmptySound` / NeoForge `FluidType.getSound(BUCKET_EMPTY)`:
    /// the sound event a bucket plays as it empties `fluid`.
    pub fn fluid_empty_sound(
        &self,
        fluid: FluidKey,
    ) -> Result<Option<&'static str>, mod_api::LevelReadError> {
        Ok(match engine_fluid_kind(fluid) {
            Some(kind) => sunrise_world::level::fluid_empty_sound(kind),
            None => self
                .fluid_bucket(fluid)?
                .map(|_| "minecraft:item.bucket.empty"),
        })
    }

    /// The exported definition of a world fluid, or a tank-only identity.
    pub fn fluid_state_definition(&self, fluid: FluidKey) -> Option<FluidDefinition<'a>> {
        let id = mod_api::__private::fluid_id(fluid)?;
        let Some(definition) = self.res.behaviours().fluid_definitions().get(id) else {
            return Some(FluidDefinition::TankOnly);
        };
        let default = api_fluid_state(definition.default)?;
        Some(FluidDefinition::World {
            default,
            properties: StateProperties::of(&definition.properties),
        })
    }

    /// A registered fluid's type properties: its block's name for a world fluid, its
    /// `FluidType` numbers (a mod's from its fluid file, block or none), vanilla's `FluidTags.WATER` and `FluidTags.LAVA` behaviours by its family. None for a
    /// key the registry doesn't hold.
    pub fn fluid_type_properties(&self, fluid: FluidKey) -> Option<FluidTypeProperties> {
        let id = mod_api::__private::fluid_id(fluid)?;
        let behaviours = self.res.behaviours();
        let block = behaviours.fluid_definitions().get(id).map(|definition| {
            self.res
                .facts()
                .get(behaviours.legacy_block(definition.default))
        });
        let description_id = match &block {
            Some(block) => super::native_block_states::description_id("block", &block.name)?,
            None => super::native_block_states::description_id("fluid_type", id)?,
        };
        let kind = FluidKind::from_name(id);
        let water = kind.is_some_and(|kind| kind.same_family(FluidKind::Water));
        let lava = kind.is_some_and(|kind| kind.same_family(FluidKind::Lava));
        let measures = fluid_measures(id, kind);
        Some(FluidTypeProperties {
            description_id,
            light_level: measures.light_level,
            fall_distance_modifier: match (water, lava) {
                (true, _) => 0.0,
                (_, true) => 0.5,
                _ => 1.0,
            },
            can_push_entity: water || lava,
            can_swim: water,
            can_drown: water,
            can_extinguish: water,
            can_hydrate: water,
            supports_boating: water,
            explosion_resistance: kind.map_or(0.0, fluid_explosion_resistance),
            randomly_ticks: kind.is_some_and(fluid_randomly_ticks),
            drip_particle: kind.and_then(fluid_drip_particle).map(str::to_owned),
            temperature: measures.temperature,
            density: measures.density,
            viscosity: measures.viscosity,
        })
    }

    /// `FluidState.createLegacyBlock` for `state`, through the one table the engine converts
    /// fluid states with. None for a state no fluid can be in: no world form, an amount above
    /// 8, a zero amount for anything but the empty fluid, or a source that isn't full.
    /// The host forwarder converts this engine ID through the checked block-state index.
    pub fn fluid_legacy_block(&self, state: FluidState) -> Option<BlockStateId> {
        let engine = super::native_block_states::engine_fluid_state(state)?;
        Some(self.res.behaviours().legacy_block(engine))
    }

    /// Validate fluid constraints against the exported property definition.
    pub fn fluid_state_validate(
        &self,
        fluid: FluidKey,
        constraints: &[StateConstraint],
    ) -> Result<(), FluidStateError> {
        let definition = mod_api::__private::fluid_id(fluid)
            .and_then(|id| self.res.behaviours().fluid_definitions().get(id))
            .ok_or(FluidStateError::NoWorldState)?;
        for (index, constraint) in constraints.iter().enumerate() {
            if constraints[..index]
                .iter()
                .any(|previous| previous.name == constraint.name)
            {
                return Err(FluidStateError::Duplicate(index as u32));
            }
            super::native_block_states::match_state_properties(
                &definition.properties,
                true,
                |_, _| true,
                std::slice::from_ref(constraint),
            )
            .map_err(|error| match error {
                StateMatchError::UnknownProperty(_) => {
                    FluidStateError::UnknownProperty(index as u32)
                }
                StateMatchError::InvalidValue(_) => FluidStateError::InvalidValue(index as u32),
                _ => unreachable!("StateMatchError: unhandled variant"),
            })?;
        }
        Ok(())
    }

    pub fn block_state_piston_push_reaction(
        &self,
        state: BlockState,
    ) -> Result<Option<mod_api::PushReaction>, mod_api::LevelReadError> {
        use mod_api::{BlockStateError, LevelReadError};
        let rest = self.res.rest.get().ok_or(LevelReadError::Unavailable)?;
        let state = match state.try_into_engine(&()) {
            Ok(state) => state,
            Err(BlockStateError::UnknownBlock | BlockStateError::UnknownState) => return Ok(None),
            Err(_) => return Err(LevelReadError::Unavailable),
        };
        Ok(Some(super::api_values::api_push_reaction(
            rest.behaviours.piston_reaction(state),
        )))
    }

    /// Require published weathering tables before resolving the API state to its engine ID.
    fn weathering_state(&self, state: BlockState) -> Option<(&WorldRest, BlockStateId)> {
        Some((self.res.rest.get()?, state.try_into_engine(&()).ok()?))
    }

    pub fn block_weathering_age(&self, state: BlockState) -> Option<WeatheringAge> {
        let (rest, state) = self.weathering_state(state)?;
        Some(match rest.behaviours.weathering_age(state)? {
            0 => WeatheringAge::Unaffected,
            1 => WeatheringAge::Exposed,
            2 => WeatheringAge::Weathered,
            3 => WeatheringAge::Oxidized,
            _ => return None,
        })
    }

    pub fn block_next_weathering_stage(&self, state: BlockState) -> Option<BlockStateId> {
        let (rest, state) = self.weathering_state(state)?;
        rest.behaviours.weathering_next(state)
    }

    pub fn block_previous_weathering_stage(&self, state: BlockState) -> Option<BlockStateId> {
        let (rest, state) = self.weathering_state(state)?;
        rest.behaviours.weathering_previous(state)
    }

    pub fn block_state_waxed(&self, state: BlockState) -> Option<BlockStateId> {
        let (_, state) = self.weathering_state(state)?;
        self.honeycomb_waxed(state)
    }

    pub fn block_state_unwaxed(&self, state: BlockState) -> Option<BlockStateId> {
        let (rest, state) = self.weathering_state(state)?;
        if let Some(unwaxed) = rest.behaviours.mapped_unwaxed(state) {
            return Some(unwaxed);
        }
        let registry = self.pack.assets.registry;
        let transformers = self.res.transformers();
        registry
            .resolve(&transformers.unwaxed(registry, registry.state(state)?)?)
            .ok()
    }

    /// `BlockState.canBeReplaced()`: whether placing a block where this state stands
    /// replaces it (air, grass, water, snow layers). None for an unknown state.
    pub fn block_state_replaceable(&self, state: BlockStateId) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).replaceable)
    }
    /// `BlockBehaviour.Properties.destroyTime`, as `BlockState.getDestroySpeed` reads it:
    /// stone 1.5, obsidian 50, bedrock -1 (it can't be broken). None for an unknown state.
    pub fn block_state_destroy_speed(&self, state: BlockStateId) -> Option<f32> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).hardness)
    }
    /// `BlockState.requiresCorrectToolForDrops()`. None for an unknown state.
    pub fn block_state_requires_correct_tool(&self, state: BlockStateId) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).requires_tool)
    }
    /// The explosion resistance the block of this state declares (`Block.getExplosionResistance`):
    /// stone 6, obsidian 1200, bedrock 3,600,000. None for an unknown state.
    pub fn block_state_explosion_resistance(&self, state: BlockStateId) -> Option<f32> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).explosion_resistance)
    }
    /// `BlockState.getSoundType()`: its volume, pitch and the sound events for breaking,
    /// stepping on, placing, hitting and falling on it. None for an unknown state.
    pub fn block_state_sound(&self, state: BlockStateId) -> Option<BlockSoundType> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).sounds.clone())
    }
    /// `BlockState.hasBlockEntity()`. None for an unknown state.
    pub fn block_state_has_block_entity(&self, state: BlockStateId) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).block_entity)
    }
    /// `BlockState.getMapColor`: the state's `MapColor.id`, 0 for none. None for an unknown state.
    pub fn block_state_map_color(&self, state: BlockStateId) -> Option<u8> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).map_color)
    }
    /// Decode a packed pixel with the same palette and shade arithmetic as map rendering.
    pub fn block_state_map_color_rgba(&self, packed: u8) -> Option<[u8; 4]> {
        Some(crate::game::maps::packed_rgba(packed))
    }
    /// `BlockState.getLightEmission()`. None for an unknown state.
    pub fn block_state_light_emission(&self, state: BlockStateId) -> Option<u8> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).emission)
    }
    /// `BlockBehaviour.getFriction()` of the state's block. None for an unknown state.
    pub fn block_state_friction(&self, state: BlockStateId) -> Option<f32> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).friction)
    }

    /// `Block.getSpeedFactor()` of the state's block. None for an unknown state.
    pub fn block_state_speed_factor(&self, state: BlockStateId) -> Option<f32> {
        self.pack.assets.registry.state(state)?;
        let block = self.res.motion().blocks.get(state).ok()?;
        Some(block.coefficients.speed_factor)
    }

    /// `Block.getJumpFactor()` of the state's block. None for an unknown state.
    pub fn block_state_jump_factor(&self, state: BlockStateId) -> Option<f32> {
        self.pack.assets.registry.state(state)?;
        let block = self.res.motion().blocks.get(state).ok()?;
        Some(block.coefficients.jump_factor)
    }

    /// The state's collision boxes with an empty collision context, and whether they hold at every
    /// position. None for an unknown state.
    pub fn block_state_collision(&self, state: BlockStateId) -> Option<StateCollision> {
        self.pack.assets.registry.state(state)?;
        let collision = self.res.collision();
        Some(match collision.empty_context_shape(state).ok()? {
            None => StateCollision::Positional,
            Some(shape) if collision.has_context_rule(state) => {
                StateCollision::Contextual(voxel_shape_boxes(&shape))
            }
            Some(shape) => StateCollision::Fixed(voxel_shape_boxes(&shape)),
        })
    }

    /// The bounce restitution and fall-distance reduction the state's block declares. None for an
    /// unknown state.
    pub fn block_state_landing(&self, state: BlockStateId) -> Option<BlockLanding> {
        self.pack.assets.registry.state(state)?;
        let coefficients = self.res.motion().blocks.get(state).ok()?.coefficients;
        Some(BlockLanding {
            bounce_restitution: coefficients.bounce_restitution,
            fall_distance_reduction: coefficients.fall_distance_reduction,
        })
    }

    /// Static walking path type of the state after per-state `isPathfindable` resolution. None for
    /// an unknown state, and before the game's remaining resources are loaded.
    pub fn block_state_path_type(&self, state: BlockStateId) -> Option<PathType> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.rest.get()?.mob_data.path_type(state))
    }
    /// `BlockState.isSolidRender()`. None for an unknown state.
    pub fn block_state_is_solid_render(&self, state: BlockStateId) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).solid_render)
    }

    /// `BlockState.isSolid()`, the exported `legacySolid`. None for an unknown state.
    pub fn block_state_is_solid(&self, state: BlockStateId) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).legacy_solid)
    }

    /// The cached `isCollisionShapeFullBlock` at the origin with an empty getter. None for an
    /// unknown state, and for a block with a dynamic shape, which vanilla never caches.
    pub fn block_state_collision_is_full_block(&self, state: BlockStateId) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        if self.res.behaviours().data(state).dynamic_shape {
            return None;
        }
        Some(self.res.facts().get(state).full_collision)
    }

    /// `BlockState.propagatesSkylightDown()`. None for an unknown state.
    pub fn block_state_propagates_skylight_down(&self, state: BlockStateId) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).skylight_down)
    }

    /// `BlockState.isAir()`, the exported per-state flag. None for an unknown state.
    pub fn block_state_is_air(&self, state: BlockStateId) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.behaviours().is_air(state))
    }
    /// `PoiTypes.forState`, mods' additions included. None for an unknown state or one in no
    /// POI type.
    pub fn block_state_poi_type(&self, state: BlockStateId) -> Option<mod_api::Identifier> {
        let table = &self.res.stations().professions;
        let state = self.pack.assets.registry.state(state)?;
        mod_api::Identifier::parse(table.pois.name(table.pois.classify(state)?)?).ok()
    }
    /// `BlockState.isRedstoneConductor` for a state alone, not a block in a level.
    pub fn block_state_is_redstone_conductor(&self, state: BlockStateId) -> Option<bool> {
        self.res.behaviours().state_redstone_conductor(state)
    }
    /// `instanceof` for the vanilla classes of [`mod_api::BlockClass`], from the block's exported
    /// class chain: a class and its subclasses, an interface by what the block implements. None
    /// for an unknown state.
    pub fn block_state_class(
        &self,
        state: BlockStateId,
        class: mod_api::BlockClass,
    ) -> Result<Option<bool>, mod_api::LevelReadError> {
        use mod_api::BlockClass;
        use sunrise_world::level::classes::{Class, Interface};
        if self.pack.assets.registry.state(state).is_none() {
            return Ok(None);
        }
        let data = self.res.behaviours().data(state);
        Ok(Some(match class {
            BlockClass::Stair => data.is(Class::Stair),
            BlockClass::Entity => data.implements(Interface::Entity),
            BlockClass::IronBars => data.is(Class::IronBars),
            BlockClass::StainedGlassPane => data.is(Class::StainedGlassPane),
            BlockClass::TrapDoor => data.is(Class::TrapDoor),
            BlockClass::Bonemealable => data.implements(Interface::Bonemealable),
            BlockClass::Slab => data.is(Class::Slab),
            BlockClass::Wall => data.is(Class::Wall),
            BlockClass::Fence => data.is(Class::Fence),
        }))
    }

    /// `instanceof LiquidBlockContainer`, `SimpleWaterloggedBlock` included as its subinterface.
    /// None for an unknown state.
    pub fn block_state_is_liquid_block_container(&self, state: BlockStateId) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.behaviours().is_liquid_container(state))
    }
    /// `instanceof SimpleWaterloggedBlock`, from the block's exported interfaces. None for an
    /// unknown state.
    pub fn block_state_is_simple_waterlogged(&self, state: BlockStateId) -> Option<bool> {
        use sunrise_world::level::classes::Interface;
        self.pack.assets.registry.state(state)?;
        Some(
            self.res
                .behaviours()
                .data(state)
                .implements(Interface::SimpleWaterlogged),
        )
    }
    /// The immutable local ordinals of `block`, independent of current world ID order.
    pub fn block_states_of(&self, block: BlockKey) -> Option<BlockStateRange> {
        mod_api::__private::block_state_range(block).ok()
    }

    /// The properties of a resolved block, borrowed from the command registry.
    pub fn block_state_definition(&self, block: BlockKey) -> Option<StateProperties<'a>> {
        let default = super::api_values::default_state(block)?;
        let state = self.pack.assets.registry.state(default)?;
        let properties = self.pack.block_registries.blocks.get(&state.name)?;
        Some(StateProperties::of(properties))
    }

    /// Match against the existing command registry definitions without allocating on the call.
    pub fn block_state_matches(
        &self,
        state: BlockStateId,
        block: BlockKey,
        constraints: &[StateConstraint],
    ) -> Option<Result<bool, StateMatchError>> {
        let default = super::api_values::default_state(block)?;
        let registry = self.pack.assets.registry;
        let target = registry.state(default)?;
        let state = registry.state(state)?;
        let definition = self.pack.block_registries.blocks.get(&target.name)?;
        Some(super::native_block_states::match_state_properties(
            definition,
            state.name == target.name,
            |name, value| {
                state
                    .properties
                    .get(name)
                    .is_some_and(|actual| actual == value)
            },
            constraints,
        ))
    }

    /// `ItemStack.isCorrectToolForDrops(state)`: the stack's own tool rules, with no block
    /// requirement, player, position or hook. None for an unknown state.
    pub fn block_state_correct_tool_for_drops(
        &self,
        stack: &ItemStack,
        state: BlockStateId,
    ) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        Some(
            self.res
                .items()
                .correct_for_drops(Some(stack), &self.res.facts().get(state).name),
        )
    }

    pub fn block_metadata_name(
        &self,
        block: mod_api::BlockKey,
    ) -> Result<&'static str, mod_api::BlockStateError> {
        mod_api::BlockMetadata::name(super::api_values::block_metadata_table()?, block)
    }
    pub fn block_metadata_property(
        &self,
        state: BlockState,
        name: &str,
    ) -> Result<Option<&'static str>, mod_api::BlockStateError> {
        mod_api::BlockMetadata::property(super::api_values::block_metadata_table()?, state, name)
    }

    pub fn block_metadata_visit_properties(
        &self,
        state: BlockState,
        visit: &mut dyn FnMut(&str, &str),
    ) -> Result<(), mod_api::BlockStateError> {
        mod_api::BlockMetadata::visit_properties(
            super::api_values::block_metadata_table()?,
            state,
            visit,
        )
    }
    pub fn block_metadata_with(
        &self,
        state: BlockState,
        name: &str,
        value: &str,
    ) -> Result<BlockState, mod_api::BlockStateError> {
        mod_api::BlockMetadata::with(
            super::api_values::block_metadata_table()?,
            state,
            name,
            value,
        )
    }
    pub fn block_metadata_of(
        &self,
        block: mod_api::BlockKey,
        properties: &[mod_api::StateConstraint],
    ) -> Result<BlockState, mod_api::BlockStateError> {
        mod_api::BlockMetadata::of(
            super::api_values::block_metadata_table()?,
            block,
            properties,
        )
    }

    pub fn block_metadata_key(&self, id: &str) -> Option<mod_api::BlockKey> {
        mod_api::BlockMetadata::key(super::api_values::block_metadata_table().ok()?, id)
    }

    pub fn block_metadata_state_definition(
        &self,
        block: mod_api::BlockKey,
    ) -> Result<StateProperties<'static>, mod_api::BlockStateError> {
        mod_api::BlockMetadata::state_definition(super::api_values::block_metadata_table()?, block)
    }

    /// The published definitions themselves, shared: the same immutable tables every read here
    /// borrows.
    pub fn block_metadata_snapshot(
        &self,
    ) -> Option<std::sync::Arc<dyn mod_api::BlockMetadata + Send + Sync>> {
        Some(mod_api::__private::block_definitions_shared()?)
    }

    /// The waxed twin of `state`, its properties kept: the world's
    /// `neoforge:waxables` value first (`DataMapHooks.getBlockWaxed`), else
    /// vanilla's `WAXABLES`, read from the axe's wax-off transforms. None where
    /// the block doesn't wax.
    pub fn honeycomb_waxed(&self, state: BlockStateId) -> Option<BlockStateId> {
        if let Some(waxed) = self.res.behaviours().mapped_waxed(state) {
            return Some(waxed);
        }
        let registry = self.pack.assets.registry;
        let waxed = self
            .res
            .transformers()
            .waxed(registry, registry.state(state)?)?;
        registry.resolve(&waxed).ok()
    }
}

/// Convert a `VoxelShape`'s AABBs into `[min_x, min_y, min_z, max_x, max_y, max_z]` tuples.
pub fn voxel_shape_boxes(shape: &VoxelShape) -> Vec<[f64; 6]> {
    shape
        .boxes()
        .iter()
        .map(|bounds| {
            let min = bounds.min_position();
            let max = bounds.max_position();
            [min.x, min.y, min.z, max.x, max.y, max.z]
        })
        .collect()
}

/// `FluidType.getTemperature`, `getDensity`, `getViscosity` and `getLightLevel` of the registered fluid `id`,
/// whose world kind is `kind`. NeoForgeMod's EMPTY_TYPE and LAVA_TYPE state their own; water
/// keeps `FluidType.Properties`' defaults, and a mod's fluid, flowing or not, what its source's
/// file states.
fn fluid_measures(id: &str, kind: Option<FluidKind>) -> FluidMeasures {
    match kind {
        Some(FluidKind::Empty) => FluidMeasures {
            temperature: 0,
            density: 0,
            viscosity: 0,
            light_level: 0,
        },
        Some(kind) if kind.same_family(FluidKind::Lava) => FluidMeasures {
            temperature: 1300,
            density: 3000,
            viscosity: 6000,
            light_level: 15,
        },
        Some(kind) if kind.same_family(FluidKind::Water) => FluidMeasures::default(),
        _ => {
            let source = kind
                .and_then(FluidKind::mod_fluid)
                .map_or(id, |fluid| fluid.get().id.as_str());
            sunrise_sim::mod_content::fluid_measures(source).unwrap_or_default()
        }
    }
}
