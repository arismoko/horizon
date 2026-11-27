//! Read-only `BlockState.canSurvive` through a level handle.
use super::super::api_values::TryIntoApi;
use super::super::{
    mod_api_host::{FireOddsBase, FireOddsLevel, SidedLevel},
    native_hooks::LevelView,
};
use super::ReadLevel;
use crate::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
use crate::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
use crate::game::side_ops::modapi_modding::ModapiModdingSideGameExt as _;
use crate::game::side_ops::world::WorldSideGameExt as _;
use crate::game::tables::WorldTablesExt as _;
use mod_api::{BlockState, LevelHost, LevelReadError, LevelRef};
use sunrise_core::{position::BlockPos, random::LegacyRandom};
use sunrise_level::world::World;
use sunrise_world::{
    level::{LevelAccess, Ticks, signal_access::SignalAccess},
    registry::BlockStateId,
};

/// A writable simulation facade over immutable world data. Only scratch ticks and random state
/// can change; survival hooks receive the same read-only LevelHost the level handle exposes.
struct SurvivalReadLevel<'a, 'g, G: crate::game::SideGame<'g>> {
    read: ReadLevel<'a, 'g, G>,
    signal_ticks: &'a Ticks,
    random: LegacyRandom,
}

super::super::native_tags::impl_tags!(
    SurvivalReadLevel<'_, 'g, G>,
    level,
    level.read.game.native_tag_items();
    generics('g, G: crate::game::SideGame<'g>)
);

impl<'g, G: crate::game::SideGame<'g>> LevelHost for SurvivalReadLevel<'_, 'g, G> {
    fn block_states(&self) -> mod_api::BlockStates<'_> {
        LevelHost::block_states(&self.read)
    }
    fn block_face_sturdy(
        &self,
        pos: [i32; 3],
        face: mod_api::BlockFace,
        support: mod_api::SupportType,
    ) -> Option<bool> {
        LevelHost::block_face_sturdy(&self.read, pos, face, support)
    }
    fn can_see_sky(&self, pos: [i32; 3]) -> Option<bool> {
        LevelHost::can_see_sky(&self.read, pos)
    }
    fn block_entity(&self, pos: [i32; 3]) -> Option<mod_api::BlockEntityRef> {
        LevelHost::block_entity(&self.read, pos)
    }
    fn container_size(&self, at: mod_api::BlockSide) -> Option<usize> {
        LevelHost::container_size(&self.read, at)
    }
    fn lend_container_stack(
        &self,
        at: mod_api::BlockSide,
        slot: usize,
        read: &mut dyn for<'a> FnMut(mod_api::ItemStackView<'a>),
    ) {
        LevelHost::lend_container_stack(&self.read, at, slot, read)
    }
    fn block_entity_part_snapshot(
        &self,
        entity: mod_api::BlockEntityRef,
        namespace: &str,
        name: &str,
    ) -> Result<mod_api::AttachmentBatch, mod_api::PartStateError> {
        LevelHost::block_entity_part_snapshot(&self.read, entity, namespace, name)
    }
    fn block_entity_data(&self, entity: mod_api::BlockEntityRef) -> Option<mod_api::Compound> {
        LevelHost::block_entity_data(&self.read, entity)
    }
    fn block_state(&self, pos: [i32; 3]) -> Option<BlockState> {
        let [x, y, z] = pos;
        self.read.world().block(x, y, z).try_into_api().ok()
    }
    fn friction(
        &self,
        pos: [i32; 3],
        state: BlockState,
        entity: Option<mod_api::EntityRef>,
    ) -> Option<f32> {
        LevelHost::friction(&self.read, pos, state, entity)
    }
    fn state_block(&self, state: BlockState) -> Option<&str> {
        LevelHost::state_block(&self.read, state)
    }
    fn block_state_id(&self, name: &str) -> Option<BlockState> {
        LevelHost::block_state_id(&self.read, name)
    }
    fn block_state_default(&self, block: mod_api::BlockKey) -> Option<BlockState> {
        LevelHost::block_state_default(&self.read, block)
    }
    fn state_string(&self, state: BlockState) -> Option<String> {
        LevelHost::state_string(&self.read, state)
    }
    fn min_y(&self) -> i32 {
        self.read.world().shape.min_y
    }
    fn height(&self) -> i32 {
        self.read.world().shape.height
    }
    fn tags(&self) -> &dyn mod_api::Tags {
        self
    }
    fn fluid_state(&self, pos: [i32; 3]) -> Option<mod_api::FluidState> {
        LevelHost::fluid_state(&self.read, pos)
    }
    fn state_with_property(&self, state: BlockState, key: &str, value: &str) -> Option<BlockState> {
        LevelHost::state_with_property(&self.read, state, key, value)
    }
    fn brightness(&self, layer: mod_api::LightLayer, pos: [i32; 3]) -> i32 {
        crate::game::native_world_read_common::world_brightness(self.read.world(), layer, pos)
    }
    fn raw_brightness(&self, pos: [i32; 3], sky_darken: i32) -> i32 {
        self.brightness(mod_api::LightLayer::Block, pos).max(
            self.brightness(mod_api::LightLayer::Sky, pos)
                .wrapping_sub(sky_darken),
        )
    }
    fn is_loaded(&self, pos: [i32; 3]) -> bool {
        let [x, y, z] = pos;
        self.read.world().is_loaded_at(x, y, z)
    }
    fn is_raining_at(&self, pos: [i32; 3]) -> bool {
        LevelHost::is_raining_at(&self.read, pos)
    }
    fn enabled_features(&self) -> mod_api::FeatureFlags {
        LevelHost::enabled_features(&self.read)
    }
    fn signal(&self, pos: [i32; 3], direction: mod_api::Direction) -> Option<i32> {
        self.read_signals(|signals| {
            signals.signal(BlockPos::new(pos[0], pos[1], pos[2]), direction)
        })
    }
    fn direct_signal(&self, pos: [i32; 3], direction: mod_api::Direction) -> Option<i32> {
        self.read_signals(|signals| {
            signals.direct_signal(BlockPos::new(pos[0], pos[1], pos[2]), direction)
        })
    }
    fn direct_signal_to(&self, pos: [i32; 3]) -> Option<i32> {
        self.read_signals(|signals| signals.direct_signal_to(BlockPos::new(pos[0], pos[1], pos[2])))
    }
    fn best_neighbor_signal(&self, pos: [i32; 3]) -> Option<i32> {
        self.read_signals(|signals| {
            signals.best_neighbor_signal(BlockPos::new(pos[0], pos[1], pos[2]))
        })
    }
    fn has_neighbor_signal(&self, pos: [i32; 3]) -> Option<bool> {
        self.read_signals(|signals| {
            signals.has_neighbor_signal(BlockPos::new(pos[0], pos[1], pos[2]))
        })
    }
    fn collision_shape(&self, pos: [i32; 3]) -> Option<mod_api::VoxelShape> {
        LevelHost::collision_shape(&self.read, pos)
    }
}

impl<'g, G: crate::game::SideGame<'g>> FireOddsBase for SurvivalReadLevel<'_, 'g, G> {
    fn fire_world(&self) -> &World {
        self.read.world()
    }
    fn fire_behaviours(&self) -> &sunrise_world::level::Behaviours {
        self.read.game.res.behaviours()
    }
    fn fire_collision(&self) -> &sunrise_world::block_collision::BlockCollisionRegistry<'_> {
        self.read.game.collision
    }
    fn fire_api_level(&self) -> Option<LevelRef> {
        Some(self.read.level)
    }
    fn fire_capability_site(
        &self,
        at: mod_api::BlockSide,
        entity_removed: bool,
    ) -> Option<mod_api::BlockCapabilitySite> {
        self.read.game.side_read_level_capability_site(
            self.read.world(),
            self.read.level,
            at,
            entity_removed,
        )
    }
}

impl<'g, G: crate::game::SideGame<'g>> SurvivalReadLevel<'_, 'g, G> {
    fn read_signals<R>(
        &self,
        read: impl FnOnce(&sunrise_world::level::Signals<'_, Self>) -> R,
    ) -> Option<R> {
        let signals = self
            .read
            .game
            .res
            .behaviours()
            .uncached_signals(self, self.signal_ticks);
        let answer = read(&signals);
        (!(self.client_side() && signals.read_unknown_output())).then_some(answer)
    }
}

impl<'g, G: crate::game::SideGame<'g>> SidedLevel for SurvivalReadLevel<'_, 'g, G> {
    fn client_side(&self) -> bool {
        !self.read.game.is_logical_server()
    }
}

impl<'g, G: crate::game::SideGame<'g>> sunrise_world::level::BlockEntityNeighbors
    for SurvivalReadLevel<'_, 'g, G>
{
}

impl<'g, G: crate::game::SideGame<'g>> LevelAccess for SurvivalReadLevel<'_, 'g, G> {
    fn within_world_border(&self, pos: BlockPos) -> bool {
        self.read
            .border()
            .geometry()
            .is_within_bounds(f64::from(pos.x), f64::from(pos.z), 0.0)
    }
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.read.world().block(pos.x, pos.y, pos.z)
    }
    fn set_raw(&mut self, _pos: BlockPos, _state: BlockStateId) -> Option<BlockStateId> {
        None
    }
    fn is_loaded(&self, pos: BlockPos) -> bool {
        self.read.world().is_loaded_at(pos.x, pos.y, pos.z)
    }
    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }
    fn game_time(&self) -> i64 {
        self.read.world().game_time
    }
    fn min_y(&self) -> i32 {
        self.read.world().shape.min_y
    }
    fn max_y(&self) -> i32 {
        self.read.world().shape.max_y()
    }
    fn sky_light(&self, pos: BlockPos) -> i32 {
        i32::from(self.read.world().light(pos.x, pos.y, pos.z) >> 4)
    }
    fn block_light(&self, pos: BlockPos) -> i32 {
        i32::from(self.read.world().light(pos.x, pos.y, pos.z) & 15)
    }
    fn world_surface(&self, x: i32, z: i32) -> i32 {
        self.read
            .world()
            .world_surface(x, z, |state| self.read.game.res.behaviours().is_air(state))
    }
    fn redstone_experiments(&self) -> bool {
        self.read
            .game
            .level
            .enabled_features
            .iter()
            .any(|feature| feature == "minecraft:redstone_experiments")
    }
    fn dimension(&self) -> sunrise_core::dimension::Dimension {
        self.read.dimension()
    }
    fn is_client_side(&self) -> bool {
        self.client_side()
    }
    fn shulker_is_closed(&self, pos: BlockPos) -> Option<bool> {
        SignalAccess::shulker_is_closed(&self.read, pos)
    }
    fn published_collision_shape(
        &self,
        pos: BlockPos,
        state: BlockStateId,
    ) -> Option<sunrise_core::geometry::VoxelShape> {
        SignalAccess::published_collision_shape(&self.read, pos, state)
    }
    fn has_native_block_entity(&self, state: BlockStateId) -> bool {
        self.read.world().block_entities.stateful_entity(state.0)
    }
    fn adjust_signal_source(&self, state: BlockStateId, vanilla: bool) -> bool {
        SignalAccess::adjust_signal_source(&self.read, state, vanilla)
    }
    fn has_weak_power_hook(&self) -> bool {
        SignalAccess::has_weak_power_hook(&self.read)
    }
    fn adjust_should_check_weak_power(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: sunrise_core::geometry::Direction,
        vanilla: bool,
    ) -> bool {
        SignalAccess::adjust_should_check_weak_power(&self.read, pos, state, direction, vanilla)
    }
    fn has_redstone_wire_connection_hook(&self) -> bool {
        SignalAccess::has_redstone_wire_connection_hook(&self.read)
    }
    fn adjust_should_redstone_wire_connect_to(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: Option<sunrise_core::geometry::Direction>,
        vanilla: bool,
    ) -> bool {
        SignalAccess::adjust_should_redstone_wire_connect_to(
            &self.read, pos, state, direction, vanilla,
        )
    }
    fn adjust_signal(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: sunrise_core::geometry::Direction,
        direct: bool,
        vanilla: i32,
    ) -> i32 {
        SignalAccess::adjust_signal(&self.read, pos, state, direction, direct, vanilla)
    }
    fn block_entity_signal(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: sunrise_core::geometry::Direction,
    ) -> i32 {
        SignalAccess::block_entity_signal(&self.read, pos, state, direction)
    }
    fn before_fire_odds(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        face: sunrise_core::geometry::Direction,
        odds: &mut Option<(i32, i32)>,
    ) {
        FireOddsLevel {
            base: self,
            prediction: None,
        }
        .native_fire_odds(self.read.dimension(), state, pos, face, odds);
    }
    fn before_can_survive(
        &self,
        _ticks: &Ticks,
        _preview: Option<&sunrise_world::level::PreviewBlocks>,
        pos: BlockPos,
        state: BlockStateId,
        survives: &mut Option<bool>,
    ) -> bool {
        let mut inert = false;
        LevelView::new(self, self.read.world()).native_can_survive(
            self.read.dimension(),
            pos,
            state,
            survives,
            &mut inert,
        );
        inert
    }
    fn before_can_sustain_plant(
        &self,
        _ticks: &Ticks,
        _preview: Option<&sunrise_world::level::PreviewBlocks>,
        state: BlockStateId,
        ground: BlockStateId,
        ground_pos: BlockPos,
        sustains: &mut Option<bool>,
    ) -> bool {
        let mut inert = false;
        LevelView::new(self, self.read.world()).native_can_sustain_plant(
            self.read.dimension(),
            state,
            ground,
            ground_pos,
            sustains,
            &mut inert,
        );
        inert
    }
    fn before_may_place_on(
        &self,
        _ticks: &Ticks,
        _preview: Option<&sunrise_world::level::PreviewBlocks>,
        state: BlockStateId,
        ground: BlockStateId,
        ground_pos: BlockPos,
        may: &mut Option<bool>,
    ) -> bool {
        let mut inert = false;
        LevelView::new(self, self.read.world()).native_may_place_on(
            self.read.dimension(),
            state,
            ground,
            ground_pos,
            may,
            &mut inert,
        );
        inert
    }
}

pub trait NativeSurviveGameExt<'g>: crate::game::SideGame<'g> {
    fn native_level_read_ticks<'a>(&'a self, level: LevelRef) -> Option<&'a Ticks>
    where
        'g: 'a,
    {
        if level.dimension() == self.native_level_key() {
            Some(&self.block_ticks)
        } else if self.is_logical_server() {
            self.side_parked_block_ticks(level.dimension_id())
        } else {
            None
        }
    }

    /// The open level, its signal ticks and the position a read-only block query at `[x, y, z]`
    /// for `state` reads: a loaded, in-bounds position of a current level and a known state.
    fn native_level_block_query<'a>(
        &'a self,
        level: LevelRef,
        [x, y, z]: [i32; 3],
        state: BlockStateId,
    ) -> Result<(&'a World, &'a Ticks, BlockPos), LevelReadError>
    where
        'g: 'a,
    {
        let world = self.native_level_read_world(level).ok_or_else(|| {
            if self.level_handle_current(level) {
                LevelReadError::Unavailable
            } else {
                LevelReadError::Stale
            }
        })?;
        if self.pack.assets.registry.state(state).is_none() {
            return Err(LevelReadError::Unavailable);
        }
        let ticks = self
            .native_level_read_ticks(level)
            .ok_or(LevelReadError::Unavailable)?;
        if !(-30_000_000..30_000_000).contains(&x)
            || !(-30_000_000..30_000_000).contains(&z)
            || !world.is_loaded_at(x, y, z)
        {
            return Err(LevelReadError::Unavailable);
        }
        Ok((world, ticks, BlockPos::new(x, y, z)))
    }

    fn native_level_can_survive(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
    ) -> Result<bool, LevelReadError> {
        let (world, ticks, pos) = self.native_level_block_query(level, pos, state)?;
        let _query =
            super::super::native_hooks::enter_survival_query(world as *const _, pos, state)
                .ok_or(LevelReadError::Unavailable)?;
        let read = ReadLevel::new(self, level, world);
        let mut query = SurvivalReadLevel {
            read,
            signal_ticks: ticks,
            random: LegacyRandom::new(0),
        };
        Ok(self
            .res
            .behaviours()
            .can_survive_read_only(&mut query, ticks, pos, state))
    }

    /// `FireBlock.getIgniteOdds` and `getBurnOdds` of `state` at `pos` from its side `face`, as
    /// fire reads them, the mods' `fire_odds` answers included.
    fn native_level_fire_odds(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
        face: sunrise_core::geometry::Direction,
    ) -> Result<(i32, i32), LevelReadError> {
        let (world, ticks, pos) = self.native_level_block_query(level, pos, state)?;
        let _query =
            super::super::native_hooks::enter_survival_query(world as *const _, pos, state)
                .ok_or(LevelReadError::Unavailable)?;
        let read = ReadLevel::new(self, level, world);
        let mut query = SurvivalReadLevel {
            read,
            signal_ticks: ticks,
            random: LegacyRandom::new(0),
        };
        Ok(self
            .res
            .behaviours()
            .fire_odds_read_only(&mut query, ticks, pos, state, face))
    }
}

impl<'g, G: crate::game::SideGame<'g>> NativeSurviveGameExt<'g> for G {}
