use super::api_values::{TryIntoApi, TryIntoEngine};
pub use mod_api::{AttachmentAccess, AttachmentError, Attachments, BlockPos, Direction, Holder};
use {
    sunrise_mod_runtime::attachments::{ATTACHMENTS_KEY, attachment_compound},
    sunrise_world::level::{LevelEvent, Ticks},
};

pub fn world_attachments(
    world: &mut sunrise_level::world::World,
    holder: mod_api::Holder,
) -> Result<&mut mod_api::Attachments, mod_api::AttachmentError> {
    if holder == Holder::Level {
        return Ok(&mut world.attachments);
    }
    if let Holder::BlockEntity([x, y, z]) = holder {
        let pos = (x, y, z);
        let chunk = world
            .chunks
            .get(&(x >> 4, z >> 4))
            .ok_or(AttachmentError::MissingHolder)?;
        attachment_compound(
            chunk
                .saved
                .as_ref()
                .and_then(|e| e.block_entity_sources.iter().find(|(p, _)| *p == pos))
                .and_then(|(_, tag)| tag.get(ATTACHMENTS_KEY)),
        )?;
        return world
            .block_entities
            .attachments_mut(pos)
            .ok_or(AttachmentError::MissingHolder);
    }
    let Holder::Chunk([x, z]) = holder else {
        return Err(AttachmentError::MissingHolder);
    };
    world.chunks.attachments_mut(&(x, z))
}

pub fn world_attachment_ref(
    world: &sunrise_level::world::World,
    holder: Holder,
) -> Result<Option<&Attachments>, AttachmentError> {
    match holder {
        Holder::Level => Ok(Some(&world.attachments)),
        Holder::Chunk([x, z]) => world.chunks.attachments(&(x, z)),
        Holder::BlockEntity([x, y, z]) => {
            let pos = (x, y, z);
            let chunk = world
                .chunks
                .get(&(x >> 4, z >> 4))
                .ok_or(AttachmentError::MissingHolder)?;
            attachment_compound(
                chunk
                    .saved
                    .as_ref()
                    .and_then(|extras| {
                        extras
                            .block_entity_sources
                            .iter()
                            .find(|(at, _)| *at == pos)
                    })
                    .and_then(|(_, tag)| tag.get(ATTACHMENTS_KEY)),
            )?;
            if world.block_entities.generation(&pos).is_none() {
                return Err(AttachmentError::MissingHolder);
            }
            Ok(world.block_entities.attachments(pos))
        }
        _ => Err(AttachmentError::MissingHolder),
    }
}

pub fn world_attachment_changed(
    world: &mut sunrise_level::world::World,
    holder: Holder,
    persist: bool,
    sync: bool,
) {
    if sync {
        world.attachment_syncs.insert(holder);
    }
    if persist {
        match holder {
            Holder::Chunk([x, z]) => {
                world.modified.insert((x, z));
            }
            Holder::BlockEntity([x, _, z]) => {
                world.modified.insert((x >> 4, z >> 4));
            }
            _ => {}
        }
    }
}

/// A level a both-side level hook reads, which knows the side it is on.
pub trait SidedLevel: mod_api::LevelHost {
    fn client_side(&self) -> bool;
}

impl SidedLevel for super::GameLevel<'_> {
    #[inline]
    fn client_side(&self) -> bool {
        sunrise_world::level::LevelAccess::is_client_side(self)
    }
}

impl<
    'a: 'b,
    'b,
    L: std::ops::Deref<Target = super::GameLevel<'a>>,
    T: std::ops::Deref<Target = Ticks>,
> SidedLevel for LevelTicks<'b, L, T>
{
    fn client_side(&self) -> bool {
        self.level.client_side()
    }
}

impl SidedLevel for super::native_hooks::HitLevel<'_> {
    #[inline]
    fn client_side(&self) -> bool {
        self.0.client_side()
    }
}

impl SidedLevel for super::native_hooks::LevelView<'_> {
    fn client_side(&self) -> bool {
        self.level.client_side()
    }
}

impl mod_api::LevelHost for super::native_hooks::LevelView<'_> {
    fn block_states(&self) -> mod_api::BlockStates<'_> {
        mod_api::LevelHost::block_states(self.level)
    }
    fn block_face_sturdy(
        &self,
        pos: [i32; 3],
        face: mod_api::BlockFace,
        support: mod_api::SupportType,
    ) -> Option<bool> {
        mod_api::LevelHost::block_face_sturdy(self.level, pos, face, support)
    }
    fn can_see_sky(&self, pos: [i32; 3]) -> Option<bool> {
        mod_api::LevelHost::can_see_sky(self.level, pos)
    }
    fn block_entity(&self, pos: [i32; 3]) -> Option<mod_api::BlockEntityRef> {
        mod_api::LevelHost::block_entity(self.level, pos)
    }
    fn has_item_capability(&self, pos: [i32; 3], side: Option<mod_api::Direction>) -> Option<bool> {
        mod_api::LevelHost::has_item_capability(self.level, pos, side)
    }
    fn container_size(&self, at: mod_api::BlockSide) -> Option<usize> {
        mod_api::LevelHost::container_size(self.level, at)
    }
    fn lend_container_stack(
        &self,
        at: mod_api::BlockSide,
        slot: usize,
        read: &mut dyn for<'a> FnMut(mod_api::ItemStackView<'a>),
    ) {
        mod_api::LevelHost::lend_container_stack(self.level, at, slot, read)
    }
    fn block_entity_part_snapshot(
        &self,
        entity: mod_api::BlockEntityRef,
        namespace: &str,
        name: &str,
    ) -> Result<mod_api::AttachmentBatch, mod_api::PartStateError> {
        mod_api::LevelHost::block_entity_part_snapshot(self.level, entity, namespace, name)
    }
    fn block_entity_data(&self, entity: mod_api::BlockEntityRef) -> Option<mod_api::Compound> {
        mod_api::LevelHost::block_entity_data(self.level, entity)
    }
    fn block_state(&self, pos: [i32; 3]) -> Option<mod_api::BlockState> {
        mod_api::LevelHost::block_state(self.level, pos)
    }
    fn friction(
        &self,
        pos: [i32; 3],
        state: mod_api::BlockState,
        entity: Option<mod_api::EntityRef>,
    ) -> Option<f32> {
        mod_api::LevelHost::friction(self.level, pos, state, entity)
    }
    fn state_block(&self, state: mod_api::BlockState) -> Option<&str> {
        mod_api::LevelHost::state_block(self.level, state)
    }
    fn block_state_id(&self, name: &str) -> Option<mod_api::BlockState> {
        mod_api::LevelHost::block_state_id(self.level, name)
    }
    fn block_state_default(&self, block: mod_api::BlockKey) -> Option<mod_api::BlockState> {
        mod_api::LevelHost::block_state_default(self.level, block)
    }
    fn state_string(&self, state: mod_api::BlockState) -> Option<String> {
        mod_api::LevelHost::state_string(self.level, state)
    }
    fn min_y(&self) -> i32 {
        mod_api::LevelHost::min_y(self.level)
    }
    fn height(&self) -> i32 {
        mod_api::LevelHost::height(self.level)
    }
    fn tags(&self) -> &dyn mod_api::Tags {
        mod_api::LevelHost::tags(self.level)
    }
    fn fluid_state(&self, pos: [i32; 3]) -> Option<mod_api::FluidState> {
        mod_api::LevelHost::fluid_state(self.level, pos)
    }
    fn state_with_property(
        &self,
        state: mod_api::BlockState,
        key: &str,
        value: &str,
    ) -> Option<mod_api::BlockState> {
        mod_api::LevelHost::state_with_property(self.level, state, key, value)
    }
    fn brightness(&self, layer: mod_api::LightLayer, pos: [i32; 3]) -> i32 {
        mod_api::LevelHost::brightness(self.level, layer, pos)
    }
    fn raw_brightness(&self, pos: [i32; 3], sky_darken: i32) -> i32 {
        mod_api::LevelHost::raw_brightness(self.level, pos, sky_darken)
    }
    fn is_loaded(&self, pos: [i32; 3]) -> bool {
        mod_api::LevelHost::is_loaded(self.level, pos)
    }
    fn is_raining_at(&self, pos: [i32; 3]) -> bool {
        mod_api::LevelHost::is_raining_at(self.level, pos)
    }
    fn enabled_features(&self) -> mod_api::FeatureFlags {
        mod_api::LevelHost::enabled_features(self.level)
    }
    fn signal(&self, pos: [i32; 3], direction: mod_api::Direction) -> Option<i32> {
        mod_api::LevelHost::signal(self.level, pos, direction)
    }
    fn direct_signal(&self, pos: [i32; 3], direction: mod_api::Direction) -> Option<i32> {
        mod_api::LevelHost::direct_signal(self.level, pos, direction)
    }
    fn direct_signal_to(&self, pos: [i32; 3]) -> Option<i32> {
        mod_api::LevelHost::direct_signal_to(self.level, pos)
    }
    fn best_neighbor_signal(&self, pos: [i32; 3]) -> Option<i32> {
        mod_api::LevelHost::best_neighbor_signal(self.level, pos)
    }
    fn has_neighbor_signal(&self, pos: [i32; 3]) -> Option<bool> {
        mod_api::LevelHost::has_neighbor_signal(self.level, pos)
    }
    fn collision_shape(&self, pos: [i32; 3]) -> Option<mod_api::VoxelShape> {
        mod_api::LevelHost::collision_shape(self.level, pos)
    }
}

pub(in crate::game) fn level_side(level: &impl SidedLevel) -> mod_api::LevelSide<'_> {
    if level.client_side() {
        mod_api::LevelSide::Client(level)
    } else {
        mod_api::LevelSide::Server(level)
    }
}

/// The prediction's block states and the live block entities it invalidated.
pub trait PredictedLevel: sunrise_world::level::LevelAccess {
    fn block_entity_removed(&self, pos: BlockPos) -> bool;
}

/// The read surface used by fire-odds Parts. Survival queries use the same callback dispatch as
/// gameplay while keeping their addressed world's immutable inputs.
pub trait FireOddsBase:
    SidedLevel + sunrise_world::level::LevelAccess + mod_api::LevelHost
{
    fn fire_world(&self) -> &sunrise_level::world::World;
    fn fire_behaviours(&self) -> &sunrise_world::level::Behaviours;
    fn fire_collision(&self) -> &sunrise_world::block_collision::BlockCollisionRegistry<'_>;
    fn fire_api_level(&self) -> Option<mod_api::LevelRef>;
    fn fire_capability_site(
        &self,
        at: mod_api::BlockSide,
        entity_removed: bool,
    ) -> Option<mod_api::BlockCapabilitySite>;
}

impl FireOddsBase for super::GameLevel<'_> {
    fn fire_world(&self) -> &sunrise_level::world::World {
        self.world
    }
    fn fire_behaviours(&self) -> &sunrise_world::level::Behaviours {
        self.behaviours
    }
    fn fire_collision(&self) -> &sunrise_world::block_collision::BlockCollisionRegistry<'_> {
        self.collision
    }
    fn fire_api_level(&self) -> Option<mod_api::LevelRef> {
        self.api_level
    }
    fn fire_capability_site(
        &self,
        at: mod_api::BlockSide,
        entity_removed: bool,
    ) -> Option<mod_api::BlockCapabilitySite> {
        let [x, y, z] = at.pos;
        self.containers()
            .capability_site(at, self.world.block(x, y, z), entity_removed)
    }
}

/// The level read by fire-odds listeners, including edits already made in a dry prediction.
pub struct FireOddsLevel<'a> {
    pub(in crate::game) base: &'a dyn FireOddsBase,
    pub(in crate::game) prediction: Option<&'a dyn PredictedLevel>,
}

impl FireOddsLevel<'_> {
    fn block_entity_removed(&self, [x, y, z]: [i32; 3]) -> bool {
        self.prediction
            .is_some_and(|level| level.block_entity_removed(BlockPos::new(x, y, z)))
    }

    fn block(&self, [x, y, z]: [i32; 3]) -> sunrise_world::registry::BlockStateId {
        let pos = BlockPos::new(x, y, z);
        match self.prediction {
            Some(level) => level.block_state(pos),
            None => sunrise_world::level::LevelAccess::block_state(self.base, pos),
        }
    }
}

impl SidedLevel for FireOddsLevel<'_> {
    #[inline]
    fn client_side(&self) -> bool {
        self.base.client_side()
    }
}

// Prediction uses the same implementation for reads not affected by staged blocks.
macro_rules! base_level_reads {
    ($(fn $method:ident($($arg:ident: $ty:ty),*) $(-> $ret:ty)?;)*) => {
        $(fn $method(&self, $($arg: $ty),*) $(-> $ret)? {
            mod_api::LevelHost::$method(self.base, $($arg),*)
        })*
    };
}

impl mod_api::LevelHost for FireOddsLevel<'_> {
    base_level_reads! {
        fn block_states() -> mod_api::BlockStates<'_>;
        fn container_size(at: mod_api::BlockSide) -> Option<usize>;
        fn lend_container_stack(at: mod_api::BlockSide, slot: usize, read: &mut dyn for<'a> FnMut(mod_api::ItemStackView<'a>));
        fn block_state_id(name: &str) -> Option<mod_api::BlockState>;
        fn block_state_default(block: mod_api::BlockKey) -> Option<mod_api::BlockState>;
        fn state_block(state: mod_api::BlockState) -> Option<&str>;
        fn state_string(state: mod_api::BlockState) -> Option<String>;
        fn min_y() -> i32;
        fn height() -> i32;
        fn tags() -> &dyn mod_api::Tags;
        fn state_with_property(state: mod_api::BlockState, key: &str, value: &str) -> Option<mod_api::BlockState>;
        fn brightness(layer: mod_api::LightLayer, pos: [i32; 3]) -> i32;
        fn raw_brightness(pos: [i32; 3], sky_darken: i32) -> i32;
        fn is_loaded(pos: [i32; 3]) -> bool;
        fn enabled_features() -> mod_api::FeatureFlags;
        fn is_raining() -> Option<bool>;
        fn sky_darken() -> Option<i32>;
    }

    #[inline]
    fn has_item_capability(&self, pos: [i32; 3], side: Option<mod_api::Direction>) -> Option<bool> {
        let level = self.base.fire_api_level()?;
        let at = mod_api::BlockSide { level, pos, side };
        let site = self
            .base
            .fire_capability_site(at, self.block_entity_removed(pos))?;
        Some(mod_api::__private::item_capability_present(self, site))
    }
    #[inline]
    fn block_entity(&self, pos: [i32; 3]) -> Option<mod_api::BlockEntityRef> {
        if self.block_entity_removed(pos) {
            return None;
        }
        mod_api::LevelHost::block_entity(self.base, pos)
    }
    #[inline]
    fn attachment_read_scope(&self) -> mod_api::AttachmentReadScope<'_> {
        mod_api::LevelHost::attachment_read_scope(self.base)
    }
    #[inline]
    fn block_entity_part_snapshot(
        &self,
        entity: mod_api::BlockEntityRef,
        namespace: &str,
        name: &str,
    ) -> Result<mod_api::AttachmentBatch, mod_api::PartStateError> {
        use mod_api::__private::game::Fields;
        let fields = entity.fields();
        // The live reader still diagnoses handles from another level.
        if self.base.fire_api_level() == Some(fields.level)
            && self.block_entity_removed(fields.position)
        {
            return Err(mod_api::PartStateError::Stale);
        }
        mod_api::LevelHost::block_entity_part_snapshot(self.base, entity, namespace, name)
    }
    #[inline]
    fn block_entity_data(&self, entity: mod_api::BlockEntityRef) -> Option<mod_api::Compound> {
        use mod_api::__private::game::Fields;
        if self.block_entity_removed(entity.fields().position) {
            return None;
        }
        mod_api::LevelHost::block_entity_data(self.base, entity)
    }
    #[inline]
    fn block_state(&self, pos: [i32; 3]) -> Option<mod_api::BlockState> {
        self.block(pos).try_into_api().ok()
    }
    #[inline]
    fn fluid_state(&self, pos: [i32; 3]) -> Option<mod_api::FluidState> {
        if !self.is_loaded(pos) {
            return None;
        }
        let state = self.block(pos).try_into_api().ok()?;
        state.fluid_state(&self.base.block_states())
    }
    #[inline]
    fn collision_shape(&self, pos: [i32; 3]) -> Option<mod_api::VoxelShape> {
        if !self.is_loaded(pos) {
            return None;
        }
        let [x, y, z] = pos;
        let state = self.block(pos);
        let binding = self.base.fire_collision();
        let world = self.base.fire_world();
        if !self.block_entity_removed(pos)
            && (world.block(x, y, z) == state || world.block_entities.contains_key(&(x, y, z)))
        {
            // Use the staged state but preserve the live entity's geometry when it survives.
            return Some(mod_api::__private::voxel_shape_from_core(
                super::native_state_shapes::collision_shape(binding, world, state, pos),
            ));
        }
        let mut scene = sunrise_level::collisions::LookupCollisions {
            block: &|x, y, z| self.block([x, y, z]),
            binding,
        };
        let shape = binding.collision_shape_with_context(
            &mut scene,
            state,
            BlockPos::new(x, y, z),
            &sunrise_world::collision_context::CollisionContext::empty(false),
        );
        Some(mod_api::__private::voxel_shape_from_core(
            shape.unwrap_or_else(|_| sunrise_core::geometry::VoxelShape::empty()),
        ))
    }
    #[inline]
    fn is_raining_at(&self, [x, y, z]: [i32; 3]) -> bool {
        if let Some(level) = self.prediction {
            self.is_loaded([x, y, z])
                && self
                    .base
                    .fire_behaviours()
                    .is_raining_at(level, BlockPos::new(x, y, z))
        } else {
            mod_api::LevelHost::is_raining_at(self.base, [x, y, z])
        }
    }
}

impl mod_api::LevelHost for super::GameLevel<'_> {
    #[inline]
    fn block_states(&self) -> mod_api::BlockStates<'_> {
        mod_api::BlockStates::of(&self.tables)
    }
    #[inline]
    fn block_face_sturdy(
        &self,
        pos: [i32; 3],
        face: mod_api::BlockFace,
        support: mod_api::SupportType,
    ) -> Option<bool> {
        if !self.native_authoritative {
            self.api_level?;
        }
        crate::game::native_world_read_common::block_face_sturdy(
            self.world,
            self.behaviours,
            self.collision,
            pos,
            face,
            support,
        )
    }
    #[inline]
    fn can_see_sky(&self, pos: [i32; 3]) -> Option<bool> {
        if !self.native_authoritative {
            self.api_level?;
        }
        Some(crate::game::native_world_read_common::can_see_sky(
            self.world, pos,
        ))
    }
    #[inline]
    fn has_item_capability(&self, pos: [i32; 3], side: Option<mod_api::Direction>) -> Option<bool> {
        let level = self.api_level?;
        let [x, y, z] = pos;
        let at = mod_api::BlockSide { level, pos, side };
        let site = self
            .containers()
            .capability_site(at, self.world.block(x, y, z), false)?;
        Some(mod_api::__private::item_capability_present(self, site))
    }
    #[inline]
    fn block_entity(&self, pos: [i32; 3]) -> Option<mod_api::BlockEntityRef> {
        crate::game::native_block_entity_read::world_block_entity(self.world, self.api_level?, pos)
    }
    #[inline]
    fn container_size(&self, at: mod_api::BlockSide) -> Option<usize> {
        if !self.native_authoritative || self.api_level != Some(at.level) {
            return None;
        }
        self.containers().capability_size(at)
    }
    #[inline]
    fn lend_container_stack(
        &self,
        at: mod_api::BlockSide,
        slot: usize,
        read: &mut dyn for<'a> FnMut(mod_api::ItemStackView<'a>),
    ) {
        if self.native_authoritative && self.api_level == Some(at.level) {
            self.containers().lend_capability_stack(at, slot, read);
        }
    }
    #[inline]
    fn attachment_read_scope(&self) -> mod_api::AttachmentReadScope<'_> {
        mod_api::__private::game::attachment_read_scope(self.attachment_query_scopes)
    }
    #[inline]
    fn block_entity_part_snapshot(
        &self,
        entity: mod_api::BlockEntityRef,
        namespace: &str,
        name: &str,
    ) -> Result<mod_api::AttachmentBatch, mod_api::PartStateError> {
        crate::game::native_block_entity_read::part_snapshot(
            self.world,
            self.api_level,
            self.native_authoritative,
            self.part_state_index,
            self.attachment_query_scopes,
            entity,
            (namespace, name),
        )
    }
    #[inline]
    fn block_entity_data(&self, entity: mod_api::BlockEntityRef) -> Option<mod_api::Compound> {
        use mod_api::__private::game::Fields;
        let fields = entity.fields();
        if self.api_level != Some(fields.level) {
            return None;
        }
        let [x, y, z] = fields.position;
        let key = (x, y, z);
        if self.world.block_entities.generation(&key) != Some(fields.generation) {
            return None;
        }
        let entity = self.world.block_entities.get(&key)?;
        let name = &self.facts.get(self.world.block(x, y, z)).name;
        crate::game::native_block_entity_read::save_without_metadata(
            self.world, key, entity, name, self.items,
        )
    }

    #[inline]
    fn block_state_id(&self, name: &str) -> Option<mod_api::BlockState> {
        super::native_block_states::parse(self.block_registries, self.registry, name)?
            .try_into_api()
            .ok()
    }
    #[inline]
    fn block_state_default(&self, block: mod_api::BlockKey) -> Option<mod_api::BlockState> {
        super::api_values::default_state(block)?.try_into_api().ok()
    }
    #[inline]
    fn block_state(&self, pos: [i32; 3]) -> Option<mod_api::BlockState> {
        sunrise_world::level::LevelAccess::block_state(self, BlockPos::new(pos[0], pos[1], pos[2]))
            .try_into_api()
            .ok()
    }
    #[inline]
    fn state_block(&self, state: mod_api::BlockState) -> Option<&str> {
        self.native_state_block(state.try_into_engine(&()).ok()?)
    }
    #[inline]
    fn state_string(&self, state: mod_api::BlockState) -> Option<String> {
        self.registry
            .state(state.try_into_engine(&()).ok()?)
            .map(super::native_block_states::serialize)
    }
    #[inline]
    fn min_y(&self) -> i32 {
        self.world.shape.min_y
    }
    #[inline]
    fn height(&self) -> i32 {
        self.world.shape.height
    }
    #[inline]
    fn tags(&self) -> &dyn mod_api::Tags {
        self
    }
    #[inline]
    fn fluid_state(&self, pos: [i32; 3]) -> Option<mod_api::FluidState> {
        let [x, y, z] = pos;
        if !(self.world.shape.contains_y(y) && self.world.loaded(x, z)) {
            return None;
        }
        self.tables
            .block_state_fluid_state(self.world.block(x, y, z))
            .ok()
            .flatten()
    }
    #[inline]
    fn collision_shape(&self, pos: [i32; 3]) -> Option<mod_api::VoxelShape> {
        let [x, y, z] = pos;
        if !(self.world.shape.contains_y(y) && self.world.loaded(x, z)) {
            return None;
        }
        Some(mod_api::__private::voxel_shape_from_core(
            super::native_state_shapes::collision_shape(
                self.collision,
                self.world,
                self.world.block(x, y, z),
                pos,
            ),
        ))
    }
    #[inline]
    fn state_with_property(
        &self,
        state: mod_api::BlockState,
        key: &str,
        value: &str,
    ) -> Option<mod_api::BlockState> {
        self.native_state_with_property(state.try_into_engine(&()).ok()?, key, value)?
            .try_into_api()
            .ok()
    }
    #[inline]
    fn brightness(&self, layer: mod_api::LightLayer, pos: [i32; 3]) -> i32 {
        crate::game::native_world_read_common::world_brightness(self.world, layer, pos)
    }
    #[inline]
    fn raw_brightness(&self, pos: [i32; 3], sky_darken: i32) -> i32 {
        sunrise_world::level::raw_brightness(
            self,
            BlockPos::new(pos[0], pos[1], pos[2]),
            sky_darken,
        )
    }
    #[inline]
    fn is_loaded(&self, pos: [i32; 3]) -> bool {
        self.world.is_loaded_at(pos[0], pos[1], pos[2])
    }
    #[inline]
    fn is_raining_at(&self, pos: [i32; 3]) -> bool {
        self.world.is_loaded_at(pos[0], pos[1], pos[2])
            && self
                .behaviours
                .is_raining_at(self, BlockPos::new(pos[0], pos[1], pos[2]))
    }
    #[inline]
    fn is_raining(&self) -> Option<bool> {
        Some(self.raining)
    }
    #[inline]
    fn sky_darken(&self) -> Option<i32> {
        Some(self.darken)
    }
    #[inline]
    fn enabled_features(&self) -> mod_api::FeatureFlags {
        mod_api::FeatureFlags::new(self.enabled_features.clone())
    }
}

impl mod_api::AttachmentHost for super::GameLevel<'_> {
    #[inline]
    fn validate_attachment(
        &self,
        namespace: &str,
        name: &str,
        sync: bool,
    ) -> Result<(), mod_api::AttachmentError> {
        mod_api::__private::validate_attachment_use(namespace, name, sync)
    }
    #[inline]
    fn validate_attachment_key(
        &self,
        write: &mod_api::AttachmentStoreKey<'_>,
    ) -> Result<(), mod_api::AttachmentError> {
        mod_api::__private::validate_attachment_key(write)
    }
    #[inline]
    fn read_attachments(
        &mut self,
        holder: Holder,
    ) -> Result<Option<&Attachments>, AttachmentError> {
        if !self.native_authoritative {
            return Err(AttachmentError::MissingHolder);
        }
        world_attachment_ref(self.world, holder)
    }
    #[inline]
    fn attachment_changed(&mut self, holder: Holder, persist: bool, sync: bool) {
        world_attachment_changed(self.world, holder, persist, sync);
    }
    #[inline]
    fn attachments(
        &mut self,
        holder: mod_api::Holder,
        _: AttachmentAccess,
    ) -> Result<&mut mod_api::Attachments, mod_api::AttachmentError> {
        if !self.native_authoritative {
            return Err(mod_api::AttachmentError::MissingHolder);
        }
        world_attachments(self.world, holder)
    }
}

impl mod_api::LevelRandomHost for super::GameLevel<'_> {
    #[inline]
    fn container_signal(&mut self, pos: [i32; 3]) -> Option<i32> {
        if !self.native_authoritative || !self.world.is_loaded_at(pos[0], pos[1], pos[2]) {
            return None;
        }
        Some(crate::game::machine_containers::container_signal(
            self,
            BlockPos::new(pos[0], pos[1], pos[2]),
        ))
    }
    #[inline]
    fn attachments(&mut self) -> &mut dyn mod_api::AttachmentStore {
        self
    }
    #[inline]
    fn lend_random(&mut self, draw: mod_api::RandomDraw<'_>) {
        if self.native_authoritative {
            super::api_values::with_live_level_random(&mut self.world.random, draw);
        }
    }
}

impl mod_api::LevelHost for super::native_hooks::HitLevel<'_> {
    #[inline]
    fn block_entity_data(&self, _: mod_api::BlockEntityRef) -> Option<mod_api::Compound> {
        None
    }

    #[inline]
    fn block_state_id(&self, name: &str) -> Option<mod_api::BlockState> {
        super::native_block_states::parse(self.0.block_registries()?, self.0.registry(), name)?
            .try_into_api()
            .ok()
    }
    #[inline]
    fn block_state_default(&self, block: mod_api::BlockKey) -> Option<mod_api::BlockState> {
        self.0.block_registries()?;
        super::api_values::default_state(block)?.try_into_api().ok()
    }
    #[inline]
    fn tags(&self) -> &dyn mod_api::Tags {
        self
    }
    #[inline]
    fn block_state(&self, pos: [i32; 3]) -> Option<mod_api::BlockState> {
        self.0.block(pos).try_into_api().ok()
    }
    #[inline]
    fn state_block(&self, state: mod_api::BlockState) -> Option<&str> {
        self.0
            .registry()
            .state(state.try_into_engine(&()).ok()?)
            .map(|s| &*s.name)
    }
    #[inline]
    fn state_string(&self, state: mod_api::BlockState) -> Option<String> {
        self.0
            .registry()
            .state(state.try_into_engine(&()).ok()?)
            .map(super::native_block_states::serialize)
    }
    #[inline]
    fn min_y(&self) -> i32 {
        self.0.shape().min_y
    }
    #[inline]
    fn height(&self) -> i32 {
        self.0.shape().height
    }
    #[inline]
    fn state_with_property(
        &self,
        state: mod_api::BlockState,
        key: &str,
        value: &str,
    ) -> Option<mod_api::BlockState> {
        super::native_hooks::with_property(
            self.0.registry(),
            state.try_into_engine(&()).ok()?,
            key,
            value,
        )?
        .try_into_api()
        .ok()
    }
}

impl mod_api::AttachmentHost for super::native_hooks::HitLevel<'_> {
    #[inline]
    fn validate_attachment(
        &self,
        namespace: &str,
        name: &str,
        sync: bool,
    ) -> Result<(), mod_api::AttachmentError> {
        mod_api::__private::validate_attachment_use(namespace, name, sync)
    }
    #[inline]
    fn validate_attachment_key(
        &self,
        write: &mod_api::AttachmentStoreKey<'_>,
    ) -> Result<(), mod_api::AttachmentError> {
        mod_api::__private::validate_attachment_key(write)
    }
    #[inline]
    fn read_attachments(
        &mut self,
        holder: Holder,
    ) -> Result<Option<&Attachments>, AttachmentError> {
        self.0.attachments_read(holder).map(Some)
    }
    #[inline]
    fn attachment_changed(&mut self, holder: Holder, _: bool, sync: bool) {
        if sync {
            self.0.attachment_sync_queued(holder);
        }
    }
    #[inline]
    fn attachments(
        &mut self,
        holder: mod_api::Holder,
        access: AttachmentAccess,
    ) -> Result<&mut mod_api::Attachments, mod_api::AttachmentError> {
        let _ = access;
        self.0.attachments(holder)
    }
    /// The hit's victim and causing entity lend their maps for reading only.
    #[inline]
    fn read_only_attachments(
        &self,
        holder: mod_api::Holder,
    ) -> Option<Result<Option<&mod_api::Attachments>, mod_api::AttachmentError>> {
        use mod_api::__private::LivingParty;
        let body = [LivingParty::Victim, LivingParty::Causing]
            .into_iter()
            .find_map(|party| {
                self.party_body(party)
                    .filter(|&body| super::native_living_body::body_holder(body) == Some(holder))
            })?;
        let reads = self.0.reads()?;
        Some(Ok(match (holder, body) {
            (mod_api::Holder::Entity(id), _) => {
                reads.entity_attachments.and_then(|maps| maps.get(&id))
            }
            // A parked guest's map is its hub row's; the player in place's is the game's.
            (mod_api::Holder::Player(uuid), sunrise_sim::health::LivingBody::Player(player)) => {
                match player.attachments {
                    Some(map) => Some(map),
                    None if reads.player_uuid == Some(uuid) => reads.player_attachments,
                    None => None,
                }
            }
            _ => None,
        }))
    }
}

impl mod_api::LevelRandomHost for super::native_hooks::HitLevel<'_> {
    #[inline]
    fn attachments(&mut self) -> &mut dyn mod_api::AttachmentStore {
        self
    }
    #[inline]
    fn lend_random(&mut self, draw: mod_api::RandomDraw<'_>) {
        super::api_values::with_live_level_random(self.0.random(), draw);
    }
}

impl mod_api::HitHost for super::native_hooks::HitLevel<'_> {
    #[inline]
    fn play_sound_at(
        &mut self,
        sound: &str,
        at: [f64; 3],
        source: mod_api::SoundSource,
        volume: f32,
        pitch: f32,
    ) -> bool {
        super::native_hooks::sound_id(sound)
            && self
                .0
                .hit_effects()
                .is_some_and(|hit| hit.play(sound, at, source, volume, pitch, None))
    }
    #[inline]
    fn play_sound_from(
        &mut self,
        entity: mod_api::EntityRef,
        sound: &str,
        source: mod_api::SoundSource,
        volume: f32,
        pitch: f32,
    ) -> bool {
        let Some(hit) = self.0.hit_effects() else {
            return false;
        };
        let Some((id, at, _)) = hit.entity(entity) else {
            return false;
        };
        super::native_hooks::sound_id(sound) && hit.play(sound, at, source, volume, pitch, Some(id))
    }
    #[inline]
    fn lend_entity_random(&mut self, entity: mod_api::EntityRef, draw: mod_api::RandomDraw<'_>) {
        if let Some((_, _, random)) = self.0.hit_effects().and_then(|hit| hit.entity(entity)) {
            super::api_values::with_live_level_random(random, draw);
        }
    }
    #[inline]
    fn victim(&self) -> Option<mod_api::LivingView<'_>> {
        mod_api::LivingView::new(self, mod_api::__private::LivingParty::Victim)
    }
    #[inline]
    fn causing(&self) -> Option<mod_api::LivingView<'_>> {
        mod_api::LivingView::new(self, mod_api::__private::LivingParty::Causing)
    }
}

impl super::native_hooks::HitLevel<'_> {
    /// The body `party` names in this hit: the victim the hurt code lends, or the causer its
    /// site lends when it is the hook's causing entity
    /// ([`DamageLevel::causing_body`](sunrise_client::health::DamageLevel::causing_body)).
    fn party_body(
        &self,
        party: mod_api::__private::LivingParty,
    ) -> Option<sunrise_sim::health::LivingBody<'_>> {
        let body = match party {
            mod_api::__private::LivingParty::Victim => self.0.victim(),
            mod_api::__private::LivingParty::Causing => self.0.causing_body(),
            _ => None,
        }?;
        Some(match body {
            // A lent attacking player is the player in place, whose UUID the reads know.
            sunrise_sim::health::LivingBody::Player(mut p) if p.uuid.is_none() => {
                p.uuid = self.0.reads().and_then(|reads| reads.player_uuid);
                sunrise_sim::health::LivingBody::Player(p)
            }
            body => body,
        })
    }
    /// The body `party` names with the hit's reads, which a view needs to read it; none where the
    /// hit lends no such body or was built without reads.
    fn party_reads(
        &self,
        party: mod_api::__private::LivingParty,
    ) -> Option<super::native_living_body::BodyReads<'_>> {
        Some(super::native_living_body::BodyReads {
            body: self.party_body(party)?,
            reads: self.0.reads()?,
        })
    }
}

/// A hit's victim and causer, read through the same functions as the `LivingRef` reads.
impl mod_api::__private::LivingReads for super::native_hooks::HitLevel<'_> {
    #[inline]
    fn living_entity(&self, party: mod_api::__private::LivingParty) -> Option<mod_api::EntityRef> {
        self.party_reads(party)?.living_entity(party)
    }
    #[inline]
    fn living_kind(
        &self,
        party: mod_api::__private::LivingParty,
    ) -> Option<mod_api::EntityTypeKey> {
        self.party_reads(party)?.living_kind(party)
    }
    #[inline]
    fn living_holder(&self, party: mod_api::__private::LivingParty) -> Option<mod_api::Holder> {
        self.party_reads(party)?.living_holder(party)
    }
    #[inline]
    fn living_health(&self, party: mod_api::__private::LivingParty) -> Option<f32> {
        self.party_reads(party)?.living_health(party)
    }
    #[inline]
    fn living_absorption(&self, party: mod_api::__private::LivingParty) -> Option<f32> {
        self.party_reads(party)?.living_absorption(party)
    }
    #[inline]
    fn living_max_health(&self, party: mod_api::__private::LivingParty) -> Option<f32> {
        self.party_reads(party)?.living_max_health(party)
    }
    #[inline]
    fn living_effect(
        &self,
        party: mod_api::__private::LivingParty,
        effect: mod_api::EffectKey,
    ) -> Option<mod_api::ActiveEffect> {
        self.party_reads(party)?.living_effect(party, effect)
    }
    #[inline]
    fn living_effects(
        &self,
        party: mod_api::__private::LivingParty,
    ) -> Option<Vec<mod_api::ActiveEffect>> {
        self.party_reads(party)?.living_effects(party)
    }
    #[inline]
    fn living_potion_affected(
        &self,
        party: mod_api::__private::LivingParty,
        effect: mod_api::EffectKey,
    ) -> bool {
        self.party_reads(party)
            .is_some_and(|reads| reads.living_potion_affected(party, effect))
    }
    #[inline]
    fn living_attribute_value(
        &self,
        party: mod_api::__private::LivingParty,
        id: &str,
    ) -> Option<f64> {
        self.party_reads(party)?.living_attribute_value(party, id)
    }
    #[inline]
    fn living_hand_item(
        &self,
        party: mod_api::__private::LivingParty,
        hand: mod_api::Hand,
        read: &mut dyn for<'a> FnMut(mod_api::ItemStackView<'a>),
    ) -> Option<()> {
        self.party_reads(party)?.living_hand_item(party, hand, read)
    }
    #[inline]
    fn living_no_ai(&self, party: mod_api::__private::LivingParty) -> Option<bool> {
        self.party_reads(party)?.living_no_ai(party)
    }
}

/// A level with the simulation's ticks beside it: `&mut Ticks` in a write or tick hook, `&Ticks`
/// where the listener only reads (placement and survival: [`LevelView`](super::native_hooks::LevelView)).
/// Reads need only a shared view of the ticks; scheduling and writes need them mutable.
pub struct LevelTicks<'b, L, T = &'b mut Ticks> {
    level: L,
    ticks: T,
    unpack_container_loot: bool,
    /// The blocks a preview standing in for the level changed, which the reads see over it.
    preview: Option<&'b sunrise_world::level::PreviewBlocks>,
    /// The level's and the ticks' borrow, which the delegating reads below are bounded by.
    borrow: std::marker::PhantomData<&'b ()>,
}

impl<'b, L, T> LevelTicks<'b, L, T> {
    pub fn new(level: L, ticks: T) -> Self {
        Self {
            level,
            ticks,
            unpack_container_loot: true,
            preview: None,
            borrow: std::marker::PhantomData,
        }
    }

    /// `place_liquid` reads stored slots whether it uses a direct or journalled host.
    pub(in crate::game) fn without_container_loot(mut self) -> Self {
        self.unpack_container_loot = false;
        self
    }

    /// This level as the preview that changed `preview` sees it (`None`: as it is).
    pub fn previewing(mut self, preview: Option<&'b sunrise_world::level::PreviewBlocks>) -> Self {
        self.preview = preview;
        self
    }
    /// The block the preview put at `pos`, if it changed it.
    fn previewed(&self, pos: BlockPos) -> Option<sunrise_world::registry::BlockStateId> {
        self.preview?.get(&pos).copied()
    }
}

impl<
    'a: 'b,
    'b,
    L: std::ops::Deref<Target = super::GameLevel<'a>>,
    T: std::ops::Deref<Target = Ticks>,
> mod_api::LevelHost for LevelTicks<'b, L, T>
{
    fn block_states(&self) -> mod_api::BlockStates<'_> {
        mod_api::LevelHost::block_states(&*self.level)
    }
    fn block_face_sturdy(
        &self,
        pos: [i32; 3],
        face: mod_api::BlockFace,
        support: mod_api::SupportType,
    ) -> Option<bool> {
        mod_api::LevelHost::block_face_sturdy(&*self.level, pos, face, support)
    }
    fn can_see_sky(&self, pos: [i32; 3]) -> Option<bool> {
        mod_api::LevelHost::can_see_sky(&*self.level, pos)
    }
    fn has_item_capability(&self, pos: [i32; 3], side: Option<Direction>) -> Option<bool> {
        let level = self.level.api_level?;
        let [x, y, z] = pos;
        let preview = self.previewed(BlockPos::new(x, y, z));
        let state = preview.unwrap_or_else(|| self.level.world.block(x, y, z));
        let site = self.level.containers().capability_site(
            mod_api::BlockSide { level, pos, side },
            state,
            preview.is_some(),
        )?;
        Some(mod_api::__private::item_capability_present(self, site))
    }
    fn block_entity(&self, pos: [i32; 3]) -> Option<mod_api::BlockEntityRef> {
        // A block the preview changed has no block entity left.
        if self
            .previewed(BlockPos::new(pos[0], pos[1], pos[2]))
            .is_some()
        {
            return None;
        }
        mod_api::LevelHost::block_entity(&*self.level, pos)
    }
    fn container_size(&self, at: mod_api::BlockSide) -> Option<usize> {
        if self
            .previewed(BlockPos::new(at.pos[0], at.pos[1], at.pos[2]))
            .is_some()
        {
            return None;
        }
        mod_api::LevelHost::container_size(&*self.level, at)
    }
    fn lend_container_stack(
        &self,
        at: mod_api::BlockSide,
        slot: usize,
        read: &mut dyn for<'view> FnMut(mod_api::ItemStackView<'view>),
    ) {
        if self
            .previewed(BlockPos::new(at.pos[0], at.pos[1], at.pos[2]))
            .is_some()
        {
            return;
        }
        mod_api::LevelHost::lend_container_stack(&*self.level, at, slot, read)
    }
    fn attachment_read_scope(&self) -> mod_api::AttachmentReadScope<'_> {
        mod_api::LevelHost::attachment_read_scope(&*self.level)
    }
    fn block_entity_part_snapshot(
        &self,
        entity: mod_api::BlockEntityRef,
        namespace: &str,
        name: &str,
    ) -> Result<mod_api::AttachmentBatch, mod_api::PartStateError> {
        mod_api::LevelHost::block_entity_part_snapshot(&*self.level, entity, namespace, name)
    }
    fn block_entity_data(&self, entity: mod_api::BlockEntityRef) -> Option<mod_api::Compound> {
        use mod_api::__private::game::Fields;
        let [x, y, z] = entity.fields().position;
        if self.previewed(BlockPos::new(x, y, z)).is_some() {
            return None;
        }
        mod_api::LevelHost::block_entity_data(&*self.level, entity)
    }
    fn block_state_id(&self, name: &str) -> Option<mod_api::BlockState> {
        mod_api::LevelHost::block_state_id(&*self.level, name)
    }
    fn block_state_default(&self, block: mod_api::BlockKey) -> Option<mod_api::BlockState> {
        mod_api::LevelHost::block_state_default(&*self.level, block)
    }
    fn block_state(&self, pos: [i32; 3]) -> Option<mod_api::BlockState> {
        match self.previewed(BlockPos::new(pos[0], pos[1], pos[2])) {
            Some(state) => state.try_into_api().ok(),
            None => mod_api::LevelHost::block_state(&*self.level, pos),
        }
    }
    fn state_block(&self, state: mod_api::BlockState) -> Option<&str> {
        mod_api::LevelHost::state_block(&*self.level, state)
    }
    fn state_string(&self, state: mod_api::BlockState) -> Option<String> {
        mod_api::LevelHost::state_string(&*self.level, state)
    }
    fn min_y(&self) -> i32 {
        mod_api::LevelHost::min_y(&*self.level)
    }
    fn height(&self) -> i32 {
        mod_api::LevelHost::height(&*self.level)
    }
    fn tags(&self) -> &dyn mod_api::Tags {
        mod_api::LevelHost::tags(&*self.level)
    }
    fn fluid_state(&self, pos: [i32; 3]) -> Option<mod_api::FluidState> {
        match self.previewed(BlockPos::new(pos[0], pos[1], pos[2])) {
            Some(state) => self
                .level
                .tables
                .block_state_fluid_state(state)
                .ok()
                .flatten(),
            None => mod_api::LevelHost::fluid_state(&*self.level, pos),
        }
    }
    fn state_with_property(
        &self,
        state: mod_api::BlockState,
        key: &str,
        value: &str,
    ) -> Option<mod_api::BlockState> {
        mod_api::LevelHost::state_with_property(&*self.level, state, key, value)
    }
    fn brightness(&self, layer: mod_api::LightLayer, pos: [i32; 3]) -> i32 {
        mod_api::LevelHost::brightness(&*self.level, layer, pos)
    }
    fn raw_brightness(&self, pos: [i32; 3], sky_darken: i32) -> i32 {
        mod_api::LevelHost::raw_brightness(&*self.level, pos, sky_darken)
    }
    fn is_loaded(&self, pos: [i32; 3]) -> bool {
        mod_api::LevelHost::is_loaded(&*self.level, pos)
    }
    fn is_raining_at(&self, pos: [i32; 3]) -> bool {
        mod_api::LevelHost::is_raining_at(&*self.level, pos)
    }
    fn is_raining(&self) -> Option<bool> {
        mod_api::LevelHost::is_raining(&*self.level)
    }
    fn sky_darken(&self) -> Option<i32> {
        mod_api::LevelHost::sky_darken(&*self.level)
    }
    fn enabled_features(&self) -> mod_api::FeatureFlags {
        mod_api::LevelHost::enabled_features(&*self.level)
    }
    fn signal(&self, pos: [i32; 3], direction: Direction) -> Option<i32> {
        self.read_signals(|s| s.signal(BlockPos::new(pos[0], pos[1], pos[2]), direction))
    }
    fn direct_signal(&self, pos: [i32; 3], direction: Direction) -> Option<i32> {
        self.read_signals(|s| s.direct_signal(BlockPos::new(pos[0], pos[1], pos[2]), direction))
    }
    fn direct_signal_to(&self, pos: [i32; 3]) -> Option<i32> {
        self.read_signals(|s| s.direct_signal_to(BlockPos::new(pos[0], pos[1], pos[2])))
    }
    fn best_neighbor_signal(&self, pos: [i32; 3]) -> Option<i32> {
        self.read_signals(|s| s.best_neighbor_signal(BlockPos::new(pos[0], pos[1], pos[2])))
    }
    fn has_neighbor_signal(&self, pos: [i32; 3]) -> Option<bool> {
        self.read_signals(|s| s.has_neighbor_signal(BlockPos::new(pos[0], pos[1], pos[2])))
    }
    fn collision_shape(&self, pos: [i32; 3]) -> Option<mod_api::VoxelShape> {
        let Some(state) = self.previewed(BlockPos::new(pos[0], pos[1], pos[2])) else {
            return mod_api::LevelHost::collision_shape(&*self.level, pos);
        };
        Some(mod_api::__private::voxel_shape_from_core(
            super::native_state_shapes::collision_shape(
                self.level.collision,
                self.level.world,
                state,
                pos,
            ),
        ))
    }
}

impl<'a: 'b, 'b, T: std::ops::Deref<Target = Ticks>> mod_api::LevelRandomHost
    for LevelTicks<'b, &mut super::GameLevel<'a>, T>
{
    fn container_signal(&mut self, pos: [i32; 3]) -> Option<i32> {
        if self.unpack_container_loot {
            return mod_api::LevelRandomHost::container_signal(&mut *self.level, pos);
        }
        let [x, y, z] = pos;
        (self.level.native_authoritative && self.level.world.is_loaded_at(x, y, z))
            .then(|| self.level.containers().stored_container_signal((x, y, z)))
    }
    fn attachments(&mut self) -> &mut dyn mod_api::AttachmentStore {
        mod_api::LevelRandomHost::attachments(&mut *self.level)
    }
    fn lend_random(&mut self, draw: mod_api::RandomDraw<'_>) {
        mod_api::LevelRandomHost::lend_random(&mut *self.level, draw);
    }
}

impl<
    'a: 'b,
    'b,
    L: std::ops::Deref<Target = super::GameLevel<'a>>,
    T: std::ops::DerefMut<Target = Ticks>,
> mod_api::LevelTicksHost for LevelTicks<'b, L, T>
{
    fn schedule_block_tick(&mut self, pos: [i32; 3], block: mod_api::BlockKey, delay: i32) -> bool {
        self.level
            .native_schedule_block_tick(&mut self.ticks, pos, block, delay)
    }
    fn schedule_fluid_tick(&mut self, pos: [i32; 3], fluid: &str, delay: i32) -> bool {
        self.level
            .native_schedule_fluid_tick(&mut self.ticks, pos, fluid, delay)
    }
}

impl<
    'a: 'b,
    'b,
    L: std::ops::Deref<Target = super::GameLevel<'a>>,
    T: std::ops::Deref<Target = Ticks>,
> LevelTicks<'b, L, T>
{
    /// A read of the level in place (over the preview's blocks) through the game's own
    /// `SignalGetter`, with these ticks' comparator outputs. A client's ticks don't hold the
    /// server's outputs: a read that needed one it lacks is unknown there, `None`.
    fn read_signals<R>(
        &self,
        read: impl FnOnce(&sunrise_world::level::Signals<'_, Self>) -> R,
    ) -> Option<R> {
        let behaviours = self.level.behaviours;
        let signals = if self.level.native_authoritative && self.preview.is_some() {
            behaviours.uncached_signals(self, &self.ticks)
        } else {
            behaviours.signals(self, &self.ticks)
        };
        let value = read(&signals);
        (!(self.level.client_side && signals.read_unknown_output())).then_some(value)
    }
}

/// The signal reads of the level in place, over the preview's blocks: a block the preview
/// changed has no block entity left, as `DryLevel` reads it.
impl<
    'a: 'b,
    'b,
    L: std::ops::Deref<Target = super::GameLevel<'a>>,
    T: std::ops::Deref<Target = Ticks>,
> sunrise_world::level::signal_access::SignalAccess for LevelTicks<'b, L, T>
{
    fn block_state(&self, pos: BlockPos) -> sunrise_world::registry::BlockStateId {
        self.previewed(pos)
            .unwrap_or_else(|| sunrise_world::level::LevelAccess::block_state(&*self.level, pos))
    }
    fn shulker_is_closed(&self, pos: BlockPos) -> Option<bool> {
        if self.previewed(pos).is_some() {
            return None;
        }
        sunrise_world::level::LevelAccess::shulker_is_closed(&*self.level, pos)
    }
    fn published_collision_shape(
        &self,
        pos: BlockPos,
        state: sunrise_world::registry::BlockStateId,
    ) -> Option<sunrise_core::geometry::VoxelShape> {
        if self.previewed(pos).is_some() {
            return None;
        }
        sunrise_world::level::LevelAccess::published_collision_shape(&*self.level, pos, state)
    }
    fn adjust_signal_source(
        &self,
        state: sunrise_world::registry::BlockStateId,
        vanilla: bool,
    ) -> bool {
        sunrise_world::level::LevelAccess::adjust_signal_source(&*self.level, state, vanilla)
    }
    fn has_weak_power_hook(&self) -> bool {
        sunrise_world::level::LevelAccess::has_weak_power_hook(&*self.level)
    }
    fn adjust_should_check_weak_power(
        &self,
        pos: BlockPos,
        state: sunrise_world::registry::BlockStateId,
        direction: Direction,
        vanilla: bool,
    ) -> bool {
        sunrise_world::level::LevelAccess::adjust_should_check_weak_power(
            &*self.level,
            pos,
            state,
            direction,
            vanilla,
        )
    }
    fn has_redstone_wire_connection_hook(&self) -> bool {
        sunrise_world::level::LevelAccess::has_redstone_wire_connection_hook(&*self.level)
    }
    fn adjust_should_redstone_wire_connect_to(
        &self,
        pos: BlockPos,
        state: sunrise_world::registry::BlockStateId,
        direction: Option<Direction>,
        vanilla: bool,
    ) -> bool {
        sunrise_world::level::LevelAccess::adjust_should_redstone_wire_connect_to(
            &*self.level,
            pos,
            state,
            direction,
            vanilla,
        )
    }
    fn adjust_signal(
        &self,
        pos: BlockPos,
        state: sunrise_world::registry::BlockStateId,
        direction: Direction,
        direct: bool,
        vanilla: i32,
    ) -> i32 {
        sunrise_world::level::LevelAccess::adjust_signal(
            &*self.level,
            pos,
            state,
            direction,
            direct,
            vanilla,
        )
    }
    fn block_entity_signal(
        &self,
        pos: BlockPos,
        state: sunrise_world::registry::BlockStateId,
        direction: Direction,
    ) -> i32 {
        if self.previewed(pos).is_some() {
            return 0;
        }
        sunrise_world::level::LevelAccess::block_entity_signal(&*self.level, pos, state, direction)
    }
}

impl<'a: 'b, 'b, T: std::ops::DerefMut<Target = Ticks>> mod_api::LevelWriteHost
    for LevelTicks<'b, &mut super::GameLevel<'a>, T>
{
    fn set_block(
        &mut self,
        pos: [i32; 3],
        state: mod_api::BlockState,
        flags: mod_api::BlockUpdateFlags,
    ) -> bool {
        let Ok(state) = state.try_into_engine(&()) else {
            return false;
        };
        self.level
            .native_set_block(&mut self.ticks, pos, state, flags.bits())
    }

    fn remove_block(&mut self, pos: [i32; 3], moved: bool) -> bool {
        self.level.remove_block_write(&mut self.ticks, pos, moved)
    }

    fn set_block_property(
        &mut self,
        pos: [i32; 3],
        key: &str,
        value: &str,
        flags: mod_api::BlockUpdateFlags,
    ) -> bool {
        self.level
            .native_set_block_property(&mut self.ticks, pos, key, value, flags.bits())
    }

    fn play_sound(
        &mut self,
        sound: &str,
        at: [f64; 3],
        source: mod_api::SoundSource,
        volume: f32,
        pitch: f32,
    ) -> bool {
        self.level
            .native_play_sound(sound, at, source, volume, pitch)
    }

    fn send_particles(
        &mut self,
        options: &mod_api::ParticleOptions,
        state: Option<mod_api::BlockState>,
        at: [f64; 3],
        spread: [f64; 3],
        speed: f64,
        count: i32,
    ) -> bool {
        let Ok(state) = state.try_into_engine(&()) else {
            return false;
        };
        let Ok(options) = options.try_into_engine(self.level.items) else {
            return false;
        };
        self.level
            .native_send_particles(&options, state, at, spread, speed, count)
    }

    fn level_event(&mut self, id: i32, pos: [i32; 3], data: i32) -> bool {
        self.level.native_level_event(id, pos, data)
    }
}

super::native_tags::impl_tags!(
    super::GameLevel<'_>,
    level,
    level.native_authoritative.then_some(level.items),
    level
        .native_authoritative
        .then_some(level.biome_keys)
        .flatten()
);
super::native_tags::impl_tags!(super::native_hooks::HitLevel<'_>, level, level.0.tags());

/// Whether level event `id` can play with `data`: the events that carry a
/// block state (2001, 2014, 3008) need `data` to be one of the `states`.
pub fn level_event_data_ok(id: i32, data: i32, states: usize) -> bool {
    !matches!(id, 2001 | 2014 | 3008) || usize::try_from(data).is_ok_and(|d| d < states)
}

impl super::GameLevel<'_> {
    /// `Level.levelEvent(null, id, pos, data)` for a hook that holds the
    /// level's write host: the event joins the ones the block behaviour
    /// emits, in order, and plays when the write that runs the hook is
    /// done (`process_level_events`), with the guests told as for any event
    /// the behaviour raises. False off an authoritative server, or for an
    /// event that carries a block state (see `level_level_event`)
    /// when `data` isn't one.
    pub(in crate::game) fn native_level_event(
        &mut self,
        id: i32,
        pos: [i32; 3],
        data: i32,
    ) -> bool {
        if !self.native_authoritative || !level_event_data_ok(id, data, self.facts.states.len()) {
            return false;
        }
        self.events.push(LevelEvent::LevelEvent {
            id,
            pos: sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]),
            data,
            source: None,
        });
        true
    }
}
