//! What the block an entity stands on or in gives it, as the block's parts answer: its friction
//! (`IBlockExtension.getFriction`), its jump and speed factors, and whether it is a ladder.
use crate::{blocks::BlockFacts, world::World};
use sunrise_core::position::BlockPos;
use sunrise_world::{
    movement_properties::BlockFactorEdits,
    native_load_context::LoopContext,
    registry::{BlockRegistry, BlockStateId},
};

/// The side-local level data beyond the world's blocks that a read-only level view needs,
/// which the game lends to each motion, mob and entity tick: weather needs the biome and
/// block-behaviour tables, a block entity's handle needs the level it is in.
pub trait SideLevel {
    /// `Level.isRainingAt` in `world`, which this side's level owns.
    fn is_raining_at(&self, world: &World, pos: BlockPos) -> bool;
    /// `LevelReader.enabledFeatures` as this side knows it.
    fn enabled_features(&self) -> &std::sync::Arc<[String]>;
    /// The merged tag tables: the server's, a client's from the join, none before they exist.
    fn tag_items(&self) -> Option<&crate::items::Items>;
    /// Borrow the equipment of the player whose movement this side is driving.
    fn player_equipment(&self) -> crate::health::PlayerEquipment<'_> {
        [None; 8]
    }
    /// Whether a living target not lent to the mob tick still has a live body.
    fn living_target_alive(&self, _id: u32) -> bool {
        false
    }
    /// What a hit on that player needs to name and read the living entities it lends, and what
    /// the view a heal or movement hook lends of an entity this side ticks needs; none where
    /// this side has no open world. Only a hit's level and such a hook's listeners ask.
    fn hit_reads(&self) -> Option<crate::health::HitReads<'_>> {
        None
    }
    /// The damage-type registry of this side's current level.
    fn damage_types(&self) -> &sunrise_world::damage_source::Catalog;
    /// The resident block entity at `pos` of `world`, as a handle of this side's level.
    fn block_entity(&self, world: &World, pos: BlockPos) -> Option<mod_api::BlockEntityRef>;
    /// Saved data for a resident handle of this side's level; never loads chunks or loot.
    fn block_entity_data(
        &self,
        world: &World,
        registry: &BlockRegistry,
        entity: mod_api::BlockEntityRef,
    ) -> Option<mod_api::Compound>;
    /// The world open that owns motion entities on this side.
    fn world_epoch(&self) -> Option<mod_api::WorldEpoch> {
        None
    }
    /// The read-only block-state and fluid fact tables of this side. Only dry test/probe sides
    /// have no tables.
    fn block_states(&self) -> Option<mod_api::BlockStates<'_>> {
        None
    }
    /// The world's frozen variant registries (server's world tables, or client's synced tables).
    fn variant_tables(&self) -> Option<&crate::mobs::variants::VariantTables> {
        None
    }
}

/// Level data for tests and probes that do not have a live game.
#[cfg(any(test, feature = "test-dry-side"))]
pub(crate) struct DrySide<'a>(pub &'a sunrise_world::damage_source::Catalog);

#[cfg(any(test, feature = "test-dry-side"))]
impl SideLevel for DrySide<'_> {
    fn is_raining_at(&self, _: &World, _: BlockPos) -> bool {
        false
    }
    fn enabled_features(&self) -> &std::sync::Arc<[String]> {
        static FEATURES: std::sync::LazyLock<std::sync::Arc<[String]>> =
            std::sync::LazyLock::new(std::sync::Arc::default);
        &FEATURES
    }
    fn tag_items(&self) -> Option<&crate::items::Items> {
        None
    }
    fn damage_types(&self) -> &sunrise_world::damage_source::Catalog {
        self.0
    }
    fn block_entity(&self, _: &World, _: BlockPos) -> Option<mod_api::BlockEntityRef> {
        None
    }
    fn block_entity_data(
        &self,
        _: &World,
        _: &BlockRegistry,
        _: mod_api::BlockEntityRef,
    ) -> Option<mod_api::Compound> {
        None
    }
}

/// A side without any live level data, for tests and probes that never read it.
#[cfg(any(test, feature = "test-dry-side"))]
pub fn dry_side() -> &'static dyn SideLevel {
    static CATALOG: std::sync::LazyLock<sunrise_world::damage_source::Catalog> =
        std::sync::LazyLock::new(sunrise_world::damage_source::Catalog::default);
    static SIDE: std::sync::LazyLock<DrySide<'static>> =
        std::sync::LazyLock::new(|| DrySide(&CATALOG));
    &*SIDE
}

/// The read-only level a subscribed block part reads: friction, bounce restitution, sound type
/// and a client's step-on. Borrows the actual world, this side's level data and the caller's
/// friction coefficient table; it carries no write access or entity snapshot.
#[derive(Clone, Copy)]
pub struct FrictionLevel<'a> {
    pub world: &'a World,
    pub registry: &'a BlockRegistry,
    pub context: &'a LoopContext,
    pub side: &'a dyn SideLevel,
    pub coefficient: &'a dyn Fn(BlockStateId) -> Option<f32>,
}

/// `IBlockExtension.getFriction` of a state at a position. A cyclic or over-deep mod
/// query gets no answer. A non-finite answer keeps the state's own coefficient; any finite one is used as given.
#[inline(never)]
pub fn read_friction(
    level: FrictionLevel<'_>,
    entity: Option<(u32, &str)>,
    pos: BlockPos,
    state: BlockStateId,
    own: f32,
) -> Option<f32> {
    let mut friction = own;
    if !crate::native_client_hooks::friction(level, entity, pos, state, &mut friction) {
        return None;
    }
    Some(if friction.is_finite() { friction } else { own })
}

/// The friction of the block at `pos` for the entity on it, starting at its `StateFacts` value.
pub fn block_friction(
    context: &LoopContext,
    side: &dyn SideLevel,
    entity: (u32, &str),
    world: &World,
    registry: &BlockRegistry,
    facts: &BlockFacts,
    pos: BlockPos,
) -> f32 {
    let state = world.block(pos.x, pos.y, pos.z);
    let own = facts.get(state).friction;
    let level = FrictionLevel {
        world,
        registry,
        context,
        side,
        coefficient: &|id| facts.states.get(id.0 as usize).map(|facts| facts.friction),
    };
    read_friction(level, Some(entity), pos, state, own).unwrap_or(own)
}

/// A block state's jump factor (`Block.getJumpFactor`) is being read (beyond NeoForge: no event).
/// Its parts may change it. Fires on replayed ticks too, so a listener must be a pure function of
/// its inputs.
#[inline(never)]
pub fn native_block_jump_factor(context: &LoopContext, state: BlockStateId, factor: &mut f32) {
    crate::native_client_hooks::block_jump_factor(context, state, factor);
}

/// A block state's speed factor (`Block.getSpeedFactor`) is being read (beyond NeoForge: no
/// event). Its parts may change it. Fires on replayed ticks too, so a listener must be a pure
/// function of its inputs.
#[inline(never)]
pub fn native_block_speed_factor(context: &LoopContext, state: BlockStateId, factor: &mut f32) {
    crate::native_client_hooks::block_speed_factor(context, state, factor);
}

/// Whether a block is a ladder to a living entity (`IBlockExtension.isLadder`, which
/// `LivingEntity.onClimbable` asks of the block the entity stands in) is being decided. `ladder`
/// starts with the `#minecraft:climbable` answer and the block's parts may change it. Fires on
/// replayed ticks too, so a listener must be a pure function of its inputs.
#[inline(never)]
pub fn native_is_ladder(
    context: &LoopContext,
    entity: u32,
    pos: BlockPos,
    state: BlockStateId,
    ladder: &mut bool,
) {
    crate::native_client_hooks::is_ladder(context, entity, pos, state, ladder);
}

/// Whether a block stops a sneaking player's slide down a climbable as scaffolding does
/// (`IBlockExtension.isScaffolding`, which `LivingEntity.handleOnClimbable` asks of the block the
/// player stands in) is being decided. `scaffolding` starts with the `minecraft:scaffolding`
/// answer and the block's parts may change it. Fires on replayed ticks too, so a listener must be
/// a pure function of its inputs.
#[inline(never)]
pub fn native_is_scaffolding(
    context: &LoopContext,
    entity: u32,
    pos: BlockPos,
    state: BlockStateId,
    scaffolding: &mut bool,
) {
    let hook = mod_api::HookId::IsScaffolding;
    let scoped = mod_api::__private::scoped_state_listens(hook, state.0);
    if !scoped.unwrap_or_else(|| mod_api::__private::has_scoped_hook(hook)) {
        return;
    }
    let Some(state) = crate::block_contact::state_to_api(state) else {
        return;
    };
    if scoped.is_none() && !mod_api::__private::block_key_listens(hook, state.block()) {
        return;
    }
    let reader = sunrise_level::api_context::LoopReader(context);
    mod_api::dispatch_is_scaffolding(&reader.context(), entity, pos, state, scaffolding);
}

/// The factors the `block_jump_factor` and `block_speed_factor` parts give the blocks an entity
/// reads, for the loop of the side that ticks it.
pub(crate) struct ModBlockFactors<'a>(pub &'a LoopContext);
impl BlockFactorEdits for ModBlockFactors<'_> {
    fn jump(&self, state: BlockStateId, mut factor: f32) -> f32 {
        native_block_jump_factor(self.0, state, &mut factor);
        factor
    }
    fn speed(&self, state: BlockStateId, mut factor: f32) -> f32 {
        native_block_speed_factor(self.0, state, &mut factor);
        factor
    }
}
