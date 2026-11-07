//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `block_states` area, implemented by the game and reached through [`Host`].
#[horizon_mod_api_macros::host_area]
pub trait BlockStatesHost {
    /// Side: both.
    /// Read a loaded, light-emitting block. No implicit chunk loading.
    #[access(WorldOnly)]
    fn lit_block_name(&self, pos: [i32; 3]) -> Option<&str>;
    /// Side: both.
    /// Property `key` of block state `state` (a machine's `facing`, from the
    /// `state` `native_block_entity_submit` gets), if it has one: no lookup in
    /// the world, so cheap each frame.
    #[access(Metadata)]
    fn block_state_property(&self, state: BlockStateId, key: &str) -> Option<&str>;
    /// Side: both.
    /// Resolve a registered fluid identifier.
    #[access(WorldOnly)]
    fn fluid_key(&self, id: &str) -> Option<FluidKey>;
    /// Side: both.
    /// The state `name` names, read as `/setblock` reads a block
    /// (`BlockStateParser.parseForBlock`): `minecraft:oak_stairs[facing=east]`,
    /// `stone` for `minecraft:stone`, the properties left out at their
    /// default. Its id is vanilla's `Block.getId`. None for an unknown block,
    /// a property it doesn't have or a value it can't take, or block NBT
    /// (`{...}`); and, as `/setblock`'s argument, for anything after the
    /// block.
    #[access(Metadata)]
    fn block_state_id(&self, name: &str) -> Option<BlockStateId>;
    /// Side: both.
    /// The state `state` as `/setblock` takes it back
    /// (`BlockStateParser.serialize`): its block, then each property in
    /// name order, `minecraft:oak_stairs[facing=east,half=bottom,...]`, or
    /// the block alone for one with no properties. None for an id no state
    /// has (`Block.stateById` gives air for one).
    #[access(Metadata)]
    fn block_state_string(&self, state: BlockStateId) -> Option<String>;
    /// Side: both.
    /// The default state of `block` (`minecraft:oak_stairs`, or `oak_stairs`):
    /// `Block.defaultBlockState`, the one a block takes when nothing else
    /// says. None for an unknown block.
    #[access(Metadata)]
    fn block_state_default(&self, block: BlockKey) -> Option<BlockStateId>;
    /// Side: both.
    /// `Item.getCraftingRemainder`: Horizon currently stores the remainder's
    /// item id, so the result is a single plain stack.
    #[access(Metadata)]
    fn item_crafting_remainder(&self, stack: &ItemStack) -> Option<ItemStack>;
    /// Side: both.
    /// `Block.asItem`: the block's item name, including air for a block with
    /// no item; None if the block name is unknown.
    #[access(Metadata)]
    fn block_item(&self, block: BlockKey) -> Option<String>;
}
/// The server host's methods of the `block_states` area, implemented by the game and reached through [`ServerHost`].
#[horizon_mod_api_macros::host_area]
pub trait BlockStatesServerHost {
    /// Side: server.
    /// Sets property `key` of the block at `pos` to `value`, as a furnace's
    /// tick sets its `lit` (`Level.setBlock` with `UPDATE_ALL`: neighbours
    /// and clients are told); true when the block changed, false when the
    /// block has no such property or value, or the property already has
    /// `value` (as `Level.setBlock` for an unchanged state). The block
    /// entity stays.
    #[access(WorldOnly)]
    fn set_block_property(&mut self, pos: [i32; 3], key: &str, value: &str) -> bool;
    /// Side: server.
    /// `native_set_block_property` with `Level.setBlock`'s `flags`, as a
    /// brewing stand's tick sets its bottles with `flags::UPDATE_CLIENTS`.
    /// Players get the write when `UPDATE_CLIENTS` is set; false when this
    /// window isn't a server, and in each case `native_set_block_property`
    /// returns false.
    #[access(WorldOnly)]
    fn set_block_property_with(
        &mut self,
        pos: [i32; 3],
        key: &str,
        value: &str,
        flags: i32,
    ) -> bool;
}
/// The handles' reads of the `block_states` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait BlockStatesBackend {
    /// Side: both.
    /// `BlockState.canBeReplaced()`: whether placing a block where this state stands
    /// replaces it (air, grass, water, snow layers). None for an unknown state.
    #[access(Metadata)]
    fn block_state_replaceable(&self, state: BlockStateId) -> Option<bool>;
    /// Side: both.
    /// `BlockBehaviour.Properties.destroyTime`, as `BlockState.getDestroySpeed` reads it:
    /// stone 1.5, obsidian 50, bedrock -1 (it can't be broken). None for an unknown state.
    #[access(Metadata)]
    fn block_state_destroy_speed(&self, state: BlockStateId) -> Option<f32>;
    /// Side: both.
    /// `BlockState.getSoundType()`: its volume, pitch and the sound events for breaking,
    /// stepping on, placing, hitting and falling on it. None for an unknown state.
    #[access(Metadata)]
    fn block_state_sound(&self, state: BlockStateId) -> Option<BlockSoundType>;
    /// Side: both.
    /// `BlockState.hasBlockEntity()`. None for an unknown state.
    #[access(Metadata)]
    fn block_state_has_block_entity(&self, state: BlockStateId) -> Option<bool>;
    /// Side: both.
    /// The states of `block` (`StateDefinition.getPossibleStates`), as the consecutive ids the
    /// registry numbers them with. None for an unknown block.
    #[access(Metadata)]
    fn block_states_of(&self, block: BlockKey) -> Option<BlockStateRange>;
    /// Side: both.
    /// The properties of a resolved block, copied for the mod as plain data.
    #[access(Metadata)]
    fn block_state_definition(&self, block: BlockKey) -> Option<Vec<StateProperty>>;
    /// Side: both.
    /// Match against the existing command registry definitions without allocating on the call.
    #[access(Metadata)]
    fn block_state_matches(
        &self,
        state: BlockStateId,
        block: BlockKey,
        constraints: &[StateConstraint],
    ) -> Option<Result<bool, StateMatchError>>;
    /// Side: both.
    /// A registered fluid's world default and legal properties, or TankOnly.
    /// None means the owning host has no resolved registry.
    #[access(Metadata)]
    fn fluid_state_definition(&self, fluid: FluidKey) -> Option<FluidDefinition>;
    /// Side: both.
    /// Validate partial property constraints in input order with indexed errors.
    /// An unavailable or tank-only definition returns NoWorldState.
    #[access(Metadata)]
    fn fluid_state_validate(
        &self,
        fluid: FluidKey,
        constraints: &[StateConstraint],
    ) -> Result<(), FluidStateError>;
}
