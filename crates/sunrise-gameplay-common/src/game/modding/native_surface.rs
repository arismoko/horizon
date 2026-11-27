//! The `friction` Part method (`IBlockExtension.getFriction`). The client library asks through
//! `native_client_hooks`; only a state whose block has a subscribing part is converted.
pub use super::api_values::{TryIntoApi, TryIntoEngine, block_state};
use super::{Dim, boats_common::Boat};
use mod_api::{
    __private::game::Fields, BlockEntityRef, BlockKey, BlockState, Compound, EntityRef,
    EntityTypeKey, FeatureFlags, FluidState, LevelHost, LevelReadError, LevelRef, LightLayer,
    WorldEpoch,
};
use std::cell::RefCell;
use sunrise_core::position::BlockPos;
use sunrise_items::items::Items;
use sunrise_level::world::World;
pub use sunrise_sim::motion::{FrictionLevel, read_friction};
use sunrise_world::{
    native_load_context::LoopSide,
    registry::{BlockRegistry, BlockStateId},
};

/// This side's level data for one tick, built once from the game's tables and lent to the motion,
/// mob and entity ticks that read friction. Fields borrow the game's tables one by one, so the
/// tick's other borrows of the game stand beside it (`game_side!`).
pub struct GameSide<'a> {
    pub dim: super::Dim,
    /// The level handle of the open world, absent before a world has an epoch.
    pub level: Option<LevelRef>,
    pub facts: &'a sunrise_items::blocks::BlockFacts,
    pub tables: super::native_block_tables::BlockTables<'a>,
    pub biomes: &'a sunrise_level::biome::Biomes,
    pub behaviours: &'a sunrise_world::level::Behaviours,
    pub features: &'a std::sync::Arc<[String]>,
    pub items: &'a Items,
    pub damage_types: &'a sunrise_world::damage_source::Catalog,
    pub player_equipment: sunrise_sim::health::PlayerEquipment<'a>,
    /// What a hit on the player in place would read beside this side's own data, as borrows:
    /// assembled into the hit's reads only when a hit builds its level
    /// ([`SideLevel::hit_reads`](sunrise_client::motion::SideLevel::hit_reads)).
    pub hit_borrows: HitBorrows<'a>,
    pub tag_items: Option<&'a Items>,
}

/// The game's tables a hit on the player in place reads that this side does not otherwise hold.
pub struct HitBorrows<'a> {
    pub living_types: sunrise_sim::health::LivingTypeKeys,
    pub attributes: &'a std::sync::Arc<sunrise_world::attributes::entity::Catalog>,
    pub player_effects: &'a sunrise_level::effects::PlayerEffects,
    pub player_uuid: Option<u128>,
    pub player_attachments: &'a mod_api::Attachments,
    pub entity_attachments: &'a sunrise_level::fast_hash::FxHashMap<u32, mod_api::Attachments>,
}

impl sunrise_sim::motion::SideLevel for GameSide<'_> {
    fn is_raining_at(&self, world: &World, pos: BlockPos) -> bool {
        world.is_loaded_at(pos.x, pos.y, pos.z)
            && crate::game::native_rain_read::rain_at(
                world,
                self.facts,
                self.biomes,
                self.behaviours,
                self.dim,
                pos,
            )
    }

    fn enabled_features(&self) -> &std::sync::Arc<[String]> {
        self.features
    }

    fn tag_items(&self) -> Option<&Items> {
        self.tag_items
    }

    fn player_equipment(&self) -> sunrise_sim::health::PlayerEquipment<'_> {
        self.player_equipment
    }

    fn hit_reads(&self) -> Option<sunrise_sim::health::HitReads<'_>> {
        let borrows = &self.hit_borrows;
        Some(sunrise_sim::health::HitReads {
            epoch: self.level?.epoch(),
            living_types: borrows.living_types,
            attributes: borrows.attributes,
            player_effects: Some(borrows.player_effects),
            player_equipment: self.player_equipment,
            player_uuid: borrows.player_uuid,
            player_attachments: Some(borrows.player_attachments),
            entity_attachments: Some(borrows.entity_attachments),
        })
    }

    fn damage_types(&self) -> &sunrise_world::damage_source::Catalog {
        self.damage_types
    }

    fn block_entity(&self, world: &World, pos: BlockPos) -> Option<BlockEntityRef> {
        crate::game::native_block_entity_read::world_block_entity(
            world,
            self.level?,
            [pos.x, pos.y, pos.z],
        )
    }

    fn block_entity_data(
        &self,
        world: &World,
        registry: &BlockRegistry,
        entity: BlockEntityRef,
    ) -> Option<Compound> {
        if self.level != Some(entity.fields().level) {
            return None;
        }
        crate::game::native_block_entity_read::world_block_entity_data(
            world, registry, self.items, entity,
        )
    }

    fn world_epoch(&self) -> Option<WorldEpoch> {
        self.level.map(LevelRef::epoch)
    }

    fn block_states(&self) -> Option<mod_api::BlockStates<'_>> {
        Some(mod_api::BlockStates::of(&self.tables))
    }
}

type Query = (*const World, BlockPos, BlockStateId, Option<EntityRef>);
thread_local! {
    static ACTIVE_FRICTION: RefCell<[Option<Query>; 16]> = const { RefCell::new([None; 16]) };
}

struct QueryGuard(usize);
impl Drop for QueryGuard {
    fn drop(&mut self) {
        ACTIVE_FRICTION.with_borrow_mut(|active| active[self.0] = None);
    }
}

/// The game's side of the friction evaluator. The no-subscriber path doesn't construct
/// a mod view or touch the re-entry stack.
pub fn friction(
    level: FrictionLevel<'_>,
    entity: Option<(u32, &str)>,
    pos: BlockPos,
    state: BlockStateId,
    friction: &mut f32,
) -> bool {
    if !block_state::block_listens(mod_api::HookId::Friction, state) {
        return true;
    }
    let entity = entity.and_then(|(id, kind)| {
        let handle = EntityRef::new(level.side.world_epoch()?, id);
        Some((handle, mod_api::__private::resolved_entity_type_key(kind)))
    });
    FrictionView(level, entity).adjust(pos, state, friction)
}

pub struct FrictionView<'a>(
    pub FrictionLevel<'a>,
    pub Option<(EntityRef, Option<EntityTypeKey>)>,
);

impl FrictionView<'_> {
    fn adjust(&self, pos: BlockPos, state: BlockStateId, friction: &mut f32) -> bool {
        let entity = self.1.map(|(entity, _)| entity);
        let query = (std::ptr::from_ref(self.0.world), pos, state, entity);
        let Some(_guard) = ACTIVE_FRICTION.with_borrow_mut(|active| {
            if active.contains(&Some(query)) {
                return None;
            }
            let slot = active.iter().position(Option::is_none)?;
            active[slot] = Some(query);
            Some(QueryGuard(slot))
        }) else {
            return false;
        };
        self.native_friction(pos, state, entity, friction);
        true
    }

    fn query(&self, [x, y, z]: [i32; 3], state: BlockStateId) -> Option<f32> {
        let own = (self.0.coefficient)(state)?;
        let mut friction = own;
        if block_state::block_listens(mod_api::HookId::Friction, state)
            && !self.adjust(BlockPos::new(x, y, z), state, &mut friction)
        {
            return None;
        }
        Some(if friction.is_finite() { friction } else { own })
    }
}

impl super::mod_api_host::SidedLevel for FrictionView<'_> {
    #[inline]
    fn client_side(&self) -> bool {
        self.0.context.side() == LoopSide::Client
    }
}

super::native_tags::impl_tags!(FrictionView<'_>, view, view.0.side.tag_items());

impl LevelHost for FrictionView<'_> {
    #[inline]
    fn block_states(&self) -> mod_api::BlockStates<'_> {
        struct NoTables;
        impl LevelHost for NoTables {
            fn block_entity_data(&self, _: BlockEntityRef) -> Option<Compound> {
                None
            }
        }
        static NONE: NoTables = NoTables;
        self.0
            .side
            .block_states()
            .unwrap_or_else(|| NONE.block_states())
    }

    #[inline]
    fn friction(&self, pos: [i32; 3], state: BlockState, entity: Option<EntityRef>) -> Option<f32> {
        let entity = match entity {
            Some(entity) => Some(self.1.filter(|&(known, _)| known == entity)?),
            None => None,
        };
        FrictionView(self.0, entity).query(pos, state.try_into_engine(&()).ok()?)
    }

    #[inline]
    fn entity_kind(&self, entity: EntityRef) -> Option<EntityTypeKey> {
        self.1.filter(|&(known, _)| known == entity)?.1
    }

    #[inline]
    fn block_state(&self, [x, y, z]: [i32; 3]) -> Option<BlockState> {
        self.0.world.block(x, y, z).try_into_api().ok()
    }

    #[inline]
    fn state_block(&self, state: BlockState) -> Option<&str> {
        self.0
            .registry
            .state(state.try_into_engine(&()).ok()?)
            .map(|state| &*state.name)
    }

    #[inline]
    fn block_state_id(&self, name: &str) -> Option<BlockState> {
        super::native_block_states::parse(
            &super::native_block_states::block_registries(self.0.registry),
            self.0.registry,
            name,
        )?
        .try_into_api()
        .ok()
    }

    #[inline]
    fn block_state_default(&self, block: BlockKey) -> Option<BlockState> {
        super::api_values::default_state(block)?.try_into_api().ok()
    }

    #[inline]
    fn state_string(&self, state: BlockState) -> Option<String> {
        self.0
            .registry
            .state(state.try_into_engine(&()).ok()?)
            .map(super::native_block_states::serialize)
    }

    #[inline]
    fn min_y(&self) -> i32 {
        self.0.world.shape.min_y
    }

    #[inline]
    fn height(&self) -> i32 {
        self.0.world.shape.height
    }

    #[inline]
    fn block_entity(&self, [x, y, z]: [i32; 3]) -> Option<BlockEntityRef> {
        self.0
            .side
            .block_entity(self.0.world, BlockPos::new(x, y, z))
    }

    #[inline]
    fn block_entity_data(&self, entity: BlockEntityRef) -> Option<Compound> {
        self.0
            .side
            .block_entity_data(self.0.world, self.0.registry, entity)
    }

    #[inline]
    fn tags(&self) -> &dyn mod_api::Tags {
        self
    }

    #[inline]
    fn fluid_state(&self, [x, y, z]: [i32; 3]) -> Option<FluidState> {
        if !self.0.world.is_loaded_at(x, y, z) {
            return None;
        }
        let states = self.0.side.block_states()?;
        let state = self.0.world.block(x, y, z).try_into_api().ok()?;
        state.fluid_state(&states)
    }

    #[inline]
    fn state_with_property(&self, state: BlockState, key: &str, value: &str) -> Option<BlockState> {
        super::native_hooks::with_property(
            self.0.registry,
            state.try_into_engine(&()).ok()?,
            key,
            value,
        )?
        .try_into_api()
        .ok()
    }

    #[inline]
    fn can_see_sky(&self, pos: [i32; 3]) -> Option<bool> {
        Some(crate::game::native_world_read_common::can_see_sky(
            self.0.world,
            pos,
        ))
    }

    #[inline]
    fn brightness(&self, layer: LightLayer, pos: [i32; 3]) -> i32 {
        crate::game::native_world_read_common::world_brightness(self.0.world, layer, pos)
    }

    #[inline]
    fn raw_brightness(&self, pos: [i32; 3], sky_darken: i32) -> i32 {
        let block = self.brightness(LightLayer::Block, pos);
        let sky = self.brightness(LightLayer::Sky, pos);
        block.max(sky.wrapping_sub(sky_darken))
    }

    #[inline]
    fn is_loaded(&self, [x, y, z]: [i32; 3]) -> bool {
        self.0.world.is_loaded_at(x, y, z)
    }

    #[inline]
    fn is_raining_at(&self, [x, y, z]: [i32; 3]) -> bool {
        self.0
            .side
            .is_raining_at(self.0.world, BlockPos::new(x, y, z))
    }

    #[inline]
    fn enabled_features(&self) -> FeatureFlags {
        FeatureFlags::new(self.0.side.enabled_features().clone())
    }
}

impl crate::game::CommonGame<'_> {}

pub trait NativeSurfaceGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    fn level_friction(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
        entity: Option<EntityRef>,
    ) -> Result<Option<f32>, LevelReadError> {
        let world = self.native_level_view_world(level).ok_or_else(|| {
            if self.side_level_handle_current(level) {
                LevelReadError::Unavailable
            } else {
                LevelReadError::Stale
            }
        })?;
        let facts = self.res.facts();
        if facts.states.get(state.0 as usize).is_none() {
            return Err(LevelReadError::Unavailable);
        }
        let level = FrictionLevel {
            world,
            registry: self.pack.assets.registry,
            context: &self.loop_context,
            side: &GameSide {
                dim: Dim::from_id(level.dimension_id()),
                level: Some(level),
                ..game_side!(
                    self,
                    crate::game::side_ops::world::WorldSideGameExt::side_game_side_meta(self)
                )
            },
            coefficient: &|id| facts.states.get(id.0 as usize).map(|facts| facts.friction),
        };
        let entity = entity
            .map(|entity| {
                self.entity_kind(entity)
                    .map(|kind| (entity, Some(kind)))
                    .ok_or(LevelReadError::Stale)
            })
            .transpose()?;
        Ok(FrictionView(level, entity).query(pos, state))
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeSurfaceGameExt1<'__sg> for G {}

use crate::game::side_ops::modapi_modding::ModapiModdingSideGameExt as _;

use crate::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

pub trait BoatFrictionGameExt<'g>: crate::game::SideGame<'g> {
    /// The friction of `state` at `pos` under the boat `b` (`AbstractBoat.getGroundFriction`,
    /// `Leashable.angularFriction`), from `own`, the block's value.
    fn boat_friction(&self, b: &Boat, pos: BlockPos, state: BlockStateId, own: f32) -> f32 {
        if !block_state::block_listens(mod_api::HookId::Friction, state) {
            return own;
        }
        let facts = self.res.facts();
        let level = FrictionLevel {
            world: &self.world,
            registry: self.pack.assets.registry,
            context: &self.loop_context,
            side: &game_side!(
                self,
                crate::game::side_ops::world::WorldSideGameExt::side_game_side_meta(self)
            ),
            coefficient: &|id| facts.states.get(id.0 as usize).map(|facts| facts.friction),
        };
        read_friction(level, Some((b.id, b.entity_type_name())), pos, state, own).unwrap_or(own)
    }
}
impl<'g, G: crate::game::SideGame<'g>> BoatFrictionGameExt<'g> for G {}
