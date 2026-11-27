//! The game-owned conversion boundary between API records and engine values.
//! Rust's orphan rule prevents foreign From/TryFrom implementations here.
#[path = "api_values/ingredient.rs"]
pub mod ingredient_values;
pub use mod_api::item::item_stack::{ItemStack as ApiStack, StackWriteError};
pub use std::num::NonZeroU32;
pub use std::sync::Arc;
pub use sunrise_core::nbt::{Compound, JavaString, Tag};
pub use sunrise_core::random::{LegacyRandom, RandomSource as EngineRandom};
pub use sunrise_items::items::{ItemStack as EngineStack, Items};
use sunrise_level::fast_hash::FxHashMap;
#[cfg(test)]
pub use sunrise_sim::parallel::ListenSnapshot;
pub use sunrise_sim::parallel::TickThread;
#[path = "api_values/aggregates.rs"]
pub mod aggregates;
#[path = "api_values/block_edit.rs"]
pub mod block_edit;
#[path = "api_values/block_state.rs"]
pub mod block_state;

pub struct LiveLevelRandom<'a>(pub(in crate::game) &'a mut LegacyRandom);

impl mod_api::RandomSource for LiveLevelRandom<'_> {
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

/// Lend the actual stream directly without unwind restoration or transaction journaling.
pub fn with_live_level_random(random: &mut LegacyRandom, draw: mod_api::RandomDraw<'_>) {
    draw.run(&mut LiveLevelRandom(random));
}

pub fn block_jump_factor(
    context: &sunrise_world::native_load_context::LoopContext,
    state: sunrise_world::registry::BlockStateId,
    factor: &mut f32,
) {
    if !block_state::block_listens(mod_api::HookId::BlockJumpFactor, state) {
        return;
    }
    let Ok(state) = state.try_into_api() else {
        return;
    };
    mod_api::dispatch_block_jump_factor(
        &sunrise_level::api_context::LoopReader(context).context(),
        state,
        factor,
    );
}
pub fn block_speed_factor(
    context: &sunrise_world::native_load_context::LoopContext,
    state: sunrise_world::registry::BlockStateId,
    factor: &mut f32,
) {
    if !block_state::block_listens(mod_api::HookId::BlockSpeedFactor, state) {
        return;
    }
    let Ok(state) = state.try_into_api() else {
        return;
    };
    mod_api::dispatch_block_speed_factor(
        &sunrise_level::api_context::LoopReader(context).context(),
        state,
        factor,
    );
}
pub fn is_ladder(
    context: &sunrise_world::native_load_context::LoopContext,
    entity: u32,
    pos: sunrise_core::position::BlockPos,
    state: sunrise_world::registry::BlockStateId,
    ladder: &mut bool,
) {
    if !block_state::block_listens(mod_api::HookId::IsLadder, state) {
        return;
    }
    let Ok(state) = state.try_into_api() else {
        return;
    };
    mod_api::dispatch_is_ladder(
        &sunrise_level::api_context::LoopReader(context).context(),
        entity,
        pos,
        state,
        ladder,
    );
}

/// Resolve the metadata default through the sole numeric reverse index, without allocation.
pub fn default_state(block: mod_api::BlockKey) -> Option<sunrise_world::registry::BlockStateId> {
    use mod_api::BlockMetadata;
    let metadata = mod_api::__private::block_definitions()?;
    let value = metadata.of(block, &[]).ok()?;
    block_state::current().ok()?.to_engine(value).ok()
}

pub fn behavior_block(
    block: mod_api::BlockKey,
    behaviours: &sunrise_world::level::Behaviours,
) -> Option<sunrise_world::level::states::BlockId> {
    Some(behaviours.block(default_state(block)?))
}
#[allow(unused_imports)]
pub use super::native_ground::alter_ground;
#[allow(unused_imports)]
pub use super::native_worldgen::prepare as prepare_block_states;

/// Publish the block state index prepared for this game. Between a tick's phases only: the
/// token is the tick thread's, which a phase's work can't take or capture.
pub fn install_block_states(_tick: &TickThread, prepared: Arc<block_state::Index>) {
    block_state::install(prepared);
}

/// Publish the parts table prepared for this game, and with it its listen index. Between a tick's
/// phases only, like [`install_block_states`]: a phase keeps the `ListenSnapshot` it captured.
pub fn install_resolved_parts(_tick: &TickThread, parts: Option<mod_api::Parts>) {
    mod_api::__private::install_resolved_parts(parts);
}

/// Who listens, as the tick thread holds it now, for a phase view to keep: None before this
/// game's block states are bound.
#[cfg(test)]
pub fn listen_snapshot(tick: &TickThread) -> Option<ListenSnapshot> {
    let index = block_state::current().ok()?;
    Some(ListenSnapshot::capture(
        tick,
        mod_api::__private::game_listen_index(),
        index.blocks(),
    ))
}
#[path = "api_values/landing_write.rs"]
pub mod landing_write;
#[path = "api_values/recipe_inputs.rs"]
pub mod recipe_inputs;

/// Publish a loaded item table's registry identities: the item keys in the table's
/// command-registry order, then the enchantment keys in the loaded registry's order.
/// Both tables load together, so one call keeps the two orderings from drifting.
pub fn resolve_item_registry(items: &sunrise_items::items::Items) -> Result<(), String> {
    mod_api::__private::resolve_items(items.registry_order.iter().map(|id| &**id)).map_err(
        |errors| {
            errors
                .into_iter()
                .map(|error| format!("mod {} requested unknown item {}", error.mod_id, error.id))
                .collect::<Vec<_>>()
                .join("\n")
        },
    )?;
    resolve_enchantment_registry(items)
}

/// Publish the loaded enchantment registry's keys in `RegistryDataLoader`'s order,
/// separately for a path that reloads only the data-pack tables.
pub fn resolve_enchantment_registry(items: &sunrise_items::items::Items) -> Result<(), String> {
    mod_api::__private::resolve_enchantments(items.enchantments.list.iter().map(|entry| &*entry.id))
        .map_err(|errors| errors.join("\n"))
}

/// Publish immutable metadata with completed keys before any mesh tables or callbacks use it.
/// Registry definitions do not change on pack reload; world-number mappings are separate.
pub fn resolve_block_definitions(
    registry: &sunrise_world::registry::BlockRegistry,
) -> Result<(), String> {
    resolve_block_definitions_with_role(registry, None)
}

/// [`resolve_block_definitions`], restricting block-entity ticker selectors to `role`'s logical side.
pub fn resolve_block_definitions_for_role(
    registry: &sunrise_world::registry::BlockRegistry,
    role: crate::game::execution_role::ExecutionRole,
) -> Result<(), String> {
    resolve_block_definitions_with_role(registry, Some(role))
}

fn resolve_block_definitions_with_role(
    registry: &sunrise_world::registry::BlockRegistry,
    role: Option<crate::game::execution_role::ExecutionRole>,
) -> Result<(), String> {
    // Without installed parts the completed keys land in one process-wide table that keeps the
    // first registry resolved; in a test process that is whichever test got there first, so a
    // test that installed no parts gets its own empty installation, as tests do their mods.
    #[cfg(test)]
    if !mod_api::__private::parts_installation::parts_installed() {
        mod_platform::__private::part_tests::install_parts_fixture(&[])
            .map_err(|errors| errors.join("\n"))?;
    }
    // A load entry point, not a phase's: the mint refuses a worker by name.
    let _tick = TickThread::mint();
    block_state::clear();
    if mod_api::__private::block_definitions().is_some() {
        return bind_worker_blocks_with_role(registry, role);
    }
    resolve_block_keys_from_registry(registry)?;
    bind_worker_blocks_with_role(registry, role)
}

#[cfg(any(test, feature = "test-support"))]
pub fn resolve_cached_block_definitions(
    cached_registry: &'static sunrise_world::registry::BlockRegistry,
) -> Result<&'static sunrise_world::registry::BlockRegistry, String> {
    let _tick = TickThread::mint();
    block_state::clear();
    if mod_api::__private::block_definitions().is_none() {
        resolve_block_keys_from_registry(cached_registry)?;
    }
    let metadata = mod_api::__private::block_definitions()
        .ok_or_else(|| "worker block metadata is unavailable".to_owned())?;
    if let Ok(worker) = super::native_worldgen::WorkerResources::get(cached_registry)
        && std::ptr::eq(worker.metadata, metadata)
    {
        block_state::install(worker.index.clone());
        return Ok(cached_registry);
    }
    // Caller requested custom block keys; fork a tiny wrapper sharing `Arc<Data>`.
    let forked = cached_registry.fork_wrapper();
    bind_worker_blocks(forked)?;
    Ok(forked)
}

pub fn resolve_block_keys_from_registry(
    registry: &sunrise_world::registry::BlockRegistry,
) -> Result<(), String> {
    let (ordered, blocks) = group_block_states(registry);
    mod_api::__private::resolve_block_keys(ordered.iter().map(String::as_str), |name| {
        let (states, ordinal) = &blocks[name];
        let refs: Vec<_> = states.iter().collect();
        sunrise_mod_runtime::block_definitions::BlockDefinition::new(&refs, *ordinal)
            .map_err(|error| format!("block {name}: {error:?}"))
    })
    .map_err(|errors| errors.join("\n"))
}

pub fn group_block_states(
    registry: &sunrise_world::registry::BlockRegistry,
) -> (
    Vec<String>,
    std::collections::HashMap<String, (Vec<std::collections::BTreeMap<String, String>>, u16)>,
) {
    let mut blocks = std::collections::HashMap::<
        String,
        Vec<(usize, std::collections::BTreeMap<String, String>)>,
    >::new();
    let mut ordered = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for index in 0..registry.len() {
        let state = registry
            .state(sunrise_world::registry::BlockStateId(index as u32))
            .expect("registry index is in bounds");
        if seen.insert(state.name.as_str()) {
            ordered.push(state.name.clone());
        }
        let props = state
            .properties
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        blocks
            .entry(state.name.clone())
            .or_default()
            .push((index, props));
    }
    let grouped = blocks
        .into_iter()
        .map(|(name, states)| {
            let default = registry
                .default_state(&name)
                .expect("registry owns block defaults");
            let ordinal = states
                .iter()
                .position(|(index, _)| *index == default.0 as usize)
                .expect("default belongs to block") as u16;
            let maps = states.into_iter().map(|(_, props)| props).collect();
            (name, (maps, ordinal))
        })
        .collect();
    (ordered, grouped)
}

pub fn bind_worker_blocks(registry: &sunrise_world::registry::BlockRegistry) -> Result<(), String> {
    bind_worker_blocks_with_role(registry, None)
}

pub fn bind_worker_blocks_for_role(
    registry: &sunrise_world::registry::BlockRegistry,
    role: crate::game::execution_role::ExecutionRole,
) -> Result<(), String> {
    bind_worker_blocks_with_role(registry, Some(role))
}

fn bind_worker_blocks_with_role(
    registry: &sunrise_world::registry::BlockRegistry,
    role: Option<crate::game::execution_role::ExecutionRole>,
) -> Result<(), String> {
    match role {
        Some(role) => block_state::bind_for_role(registry, role)?,
        None => block_state::bind(registry)?,
    }
    let index = block_state::current().map_err(|error| format!("worker index: {error:?}"))?;
    let retained =
        super::native_worldgen::retain(registry, index).inspect_err(|_| block_state::clear())?;
    block_state::install(retained);
    Ok(())
}

#[cfg(test)]
#[test]
pub fn a_test_without_parts_resolves_its_own_blocks_after_another_resolved_fewer() {
    // Another test thread, with no parts installed, resolves a registry of air alone first.
    std::thread::spawn(|| {
        let air =
            r#"{"defaults":{"minecraft:air":0},"states":[["air",true,false,"","minecraft:air"]]}"#;
        let air = sunrise_world::registry::BlockRegistry::from_facts_json(air.as_bytes()).unwrap();
        resolve_block_definitions(&air).unwrap();
    })
    .join()
    .unwrap();
    // This one, with no parts installed either, still resolves its own block.
    let own = r#"{"defaults":{"minecraft:air":0,"regression:own_block":1},"states":[
        ["air",true,false,"","minecraft:air"],
        ["b",false,false,"","regression:own_block"]
    ]}"#;
    let own = sunrise_world::registry::BlockRegistry::from_facts_json(own.as_bytes()).unwrap();
    resolve_block_definitions(&own).unwrap();
    let state = own
        .state(sunrise_world::registry::BlockStateId(1))
        .unwrap()
        .try_into_api();
    assert!(state.is_ok(), "{state:?}");
}

#[cfg(test)]
#[test]
pub fn block_definition_import_preserves_defaults_and_is_immutable_after_load() {
    let facts = r#"{"defaults":{"minecraft:air":0,"minecraft:stairs":2},"states":[
        ["air",true,false,"","minecraft:air"],
        ["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"north"}}],
        ["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"east"}}]
    ]}"#;
    mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
    let registry =
        sunrise_world::registry::BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
    resolve_block_definitions(&registry).unwrap();
    let blocks = mod_api::__private::block_definitions().unwrap();
    let key = blocks.names.slots["minecraft:stairs"] as usize;
    assert_eq!(blocks.definitions[key].of(&[]), Ok(1));
    assert_eq!(
        blocks.definitions[key].property(0, "facing"),
        Ok(Some("north"))
    );
    assert_eq!(
        blocks.definitions[key].property(1, "facing"),
        Ok(Some("east"))
    );
    let changed = facts.replace("east", "south");
    let registry =
        sunrise_world::registry::BlockRegistry::from_facts_json(changed.as_bytes()).unwrap();
    assert!(resolve_block_definitions(&registry).is_err());
    assert_eq!(
        mod_api::__private::block_definitions().unwrap().definitions[key].property(1, "facing"),
        Ok(Some("east"))
    );
}

#[allow(unused_imports)]
#[cfg(test)]
pub use crate::game::item_snapshot::ItemSnapshot;

impl TryIntoApi for sunrise_world::chunk::BlockState {
    type Api = mod_api::BlockState;
    type Error = mod_api::BlockStateError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let block = mod_api::__private::block_key(&self.name)
            .ok_or(mod_api::BlockStateError::UnknownBlock)?;
        let properties: Vec<_> = self
            .properties
            .iter()
            .map(|(name, value)| mod_api::StateConstraint::new(name.clone(), value.clone()))
            .collect();
        mod_api::BlockState::of(block_metadata_table()?, block, &properties)
    }
}

#[cfg(test)]
#[test]
pub fn block_values_keep_frozen_ordinals_when_world_state_ids_reorder() {
    use sunrise_world::registry::{BlockRegistry, BlockStateId};
    mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
    let first = r#"{"defaults":{"minecraft:air":0,"minecraft:stairs":2},"states":[["air",true,false,"","minecraft:air"],["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"north"}}],["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"east"}}]]}"#;
    let first = BlockRegistry::from_facts_json(first.as_bytes()).unwrap();
    resolve_block_definitions(&first).unwrap();
    let reordered = r#"{"defaults":{"minecraft:air":0,"minecraft:stairs":1},"states":[["air",true,false,"","minecraft:air"],["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"east"}}],["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"north"}}]]}"#;
    let reordered = BlockRegistry::from_facts_json(reordered.as_bytes()).unwrap();
    resolve_block_definitions(&reordered).unwrap();
    let east = reordered
        .state(BlockStateId(1))
        .unwrap()
        .try_into_api()
        .unwrap();
    let north = reordered
        .state(BlockStateId(2))
        .unwrap()
        .try_into_api()
        .unwrap();
    let metadata = block_metadata_table().unwrap();
    assert_eq!(east.ordinal(), 1);
    assert_eq!(north.ordinal(), 0);
    assert_eq!(east.property(metadata, "facing"), Ok(Some("east")));
    assert_eq!(north.property(metadata, "facing"), Ok(Some("north")));
    let mut invalid = reordered.state(BlockStateId(1)).unwrap().clone();
    invalid.properties.insert("facing".into(), "south".into());
    assert_eq!(
        invalid.try_into_api(),
        Err(mod_api::BlockStateError::InvalidValue(0))
    );
    invalid.name = "missing:block".into();
    assert_eq!(
        invalid.try_into_api(),
        Err(mod_api::BlockStateError::UnknownBlock)
    );
}

pub fn block_metadata_table()
-> Result<&'static sunrise_mod_runtime::block_definitions::Blocks, mod_api::BlockStateError> {
    mod_api::__private::block_definitions().ok_or(mod_api::BlockStateError::Unavailable)
}
/// Normalize an existing game write's answer without changing its implementation.
/// Generated API forwarders add the checked-input refusal around successful native answers.
pub trait IntoWriteResult {
    type Value;
    type Error;
    fn into_write_result(self) -> Result<Self::Value, Self::Error>;
}
impl<T> IntoWriteResult for Option<T> {
    type Value = Self;
    type Error = StackWriteError;
    fn into_write_result(self) -> Result<Self, Self::Error> {
        Ok(self)
    }
}
impl<T, E> IntoWriteResult for Result<T, E> {
    type Value = T;
    type Error = E;
    fn into_write_result(self) -> Self {
        self
    }
}
macro_rules! write_answers {
    ($($ty:ty),* $(,)?) => { $(
        impl IntoWriteResult for $ty {
            type Value = Self;
            type Error = StackWriteError;
            fn into_write_result(self) -> Result<Self, Self::Error> { Ok(self) }
        }
    )* };
}
#[allow(unused_imports)]
pub(crate) use write_answers;
write_answers!(bool, u32, ());

/// Convert an engine value into an owned API value after resolving registry identities.
pub trait TryIntoApi {
    type Api;
    type Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error>;
}

/// Tests use the production snapshot conversion, never a second fixture codec.
#[cfg(any(test, feature = "test-support"))]
pub fn test_snapshot<T: TryIntoApi + ?Sized>(value: &T) -> T::Api
where
    T::Error: std::fmt::Debug,
{
    value
        .try_into_api()
        .expect("fixture snapshot has resolved item keys and a nonempty count")
}

/// Validate a mod value against the game's registry before admitting it to engine state.
#[path = "api_values/content_declarations.rs"]
pub mod content_declarations;
#[allow(unused_imports)]
pub use content_declarations::{api_push_reaction, push_reaction};

#[path = "api_values/component_edit.rs"]
pub mod component_edit;
#[path = "api_values/stack_count.rs"]
pub mod stack_count;
#[allow(unused_imports)]
pub use stack_count::TryIntoEnginePreservingCount;

pub trait TryIntoEngine<Context: ?Sized = Items> {
    type Engine;
    type Error;
    fn try_into_engine(&self, context: &Context) -> Result<Self::Engine, Self::Error>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StackReadError {
    UnknownItem,
    Empty,
    InvalidComponents(mod_api::StackError),
}

impl StackReadError {
    pub fn into_write_error(self) -> StackWriteError {
        match self {
            Self::UnknownItem => StackWriteError::UnknownItem,
            Self::Empty => StackWriteError::EmptyEngineStack,
            Self::InvalidComponents(reason) => StackWriteError::InvalidComponents { reason },
        }
    }
}

/// Execute a native write and preserve its answer, including unit and original errors.
pub fn call_write<W: IntoWriteResult>(call: impl FnOnce() -> W) -> Result<W::Value, W::Error> {
    call().into_write_result()
}

/// Convert a successful stack-bearing write answer through its original error channel.
pub fn call_stack_write<W>(
    call: impl FnOnce() -> W,
) -> Result<<W::Value as TryIntoApi>::Api, W::Error>
where
    W: IntoWriteResult,
    W::Value: TryIntoApi<Error = StackReadError>,
    W::Error: From<StackWriteError>,
{
    call_write(call)?
        .try_into_api()
        .map_err(|error| error.into_write_error().into())
}

/// Convert a successful block-state write answer; a state the registry cannot name is the
/// write's own `InvalidState` error.
pub fn call_state_write<W>(
    call: impl FnOnce() -> W,
) -> Result<<W::Value as TryIntoApi>::Api, W::Error>
where
    W: IntoWriteResult,
    W::Value: TryIntoApi,
    W::Error: From<<W::Value as TryIntoApi>::Error>,
{
    call_write(call)?.try_into_api().map_err(Into::into)
}

macro_rules! scalar_values {
    ($($ty:ty),* $(,)?) => { $(
        impl TryIntoApi for $ty {
            type Api = Self;
            type Error = StackReadError;
            fn try_into_api(&self) -> Result<Self, Self::Error> { Ok(*self) }
        }
        impl TryIntoEngine for $ty {
            type Engine = Self;
            type Error = StackWriteError;
            fn try_into_engine(&self, _: &Items) -> Result<Self, Self::Error> { Ok(*self) }
        }
    )* };
}
#[allow(unused_imports)]
pub(crate) use scalar_values;
scalar_values!(u32, i32, f32, bool, ());

impl TryIntoApi for String {
    type Api = Self;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self, Self::Error> {
        Ok(self.clone())
    }
}

impl<T: TryIntoApi + ?Sized> TryIntoApi for &T {
    type Api = T::Api;
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        (*self).try_into_api()
    }
}
impl<C: ?Sized, T: TryIntoEngine<C> + ?Sized> TryIntoEngine<C> for &T {
    type Engine = T::Engine;
    type Error = T::Error;
    fn try_into_engine(&self, context: &C) -> Result<Self::Engine, Self::Error> {
        (*self).try_into_engine(context)
    }
}

impl<T: TryIntoApi> TryIntoApi for Option<T> {
    type Api = Option<T::Api>;
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        self.as_ref().map(TryIntoApi::try_into_api).transpose()
    }
}
impl<C: ?Sized, T: TryIntoEngine<C>> TryIntoEngine<C> for Option<T> {
    type Engine = Option<T::Engine>;
    type Error = T::Error;
    fn try_into_engine(&self, context: &C) -> Result<Self::Engine, Self::Error> {
        self.as_ref()
            .map(|value| value.try_into_engine(context))
            .transpose()
    }
}

impl<T: TryIntoApi, const N: usize> TryIntoApi for [T; N] {
    type Api = [T::Api; N];
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let mut output: [Option<T::Api>; N] = std::array::from_fn(|_| None);
        for (target, source) in output.iter_mut().zip(self) {
            *target = Some(source.try_into_api()?);
        }
        Ok(output.map(|value| value.expect("every array element converted")))
    }
}
impl<C: ?Sized, T: TryIntoEngine<C>, const N: usize> TryIntoEngine<C> for [T; N] {
    type Engine = [T::Engine; N];
    type Error = T::Error;
    fn try_into_engine(&self, context: &C) -> Result<Self::Engine, Self::Error> {
        let mut output: [Option<T::Engine>; N] = std::array::from_fn(|_| None);
        for (target, source) in output.iter_mut().zip(self) {
            *target = Some(source.try_into_engine(context)?);
        }
        Ok(output.map(|value| value.expect("every array element converted")))
    }
}
impl<T: TryIntoApi> TryIntoApi for [T] {
    type Api = Vec<T::Api>;
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        self.iter().map(TryIntoApi::try_into_api).collect()
    }
}
impl<T: TryIntoApi> TryIntoApi for std::sync::Arc<[T]> {
    type Api = std::sync::Arc<[T::Api]>;
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(self.as_ref().try_into_api()?.into())
    }
}
impl<T: TryIntoApi> TryIntoApi for Vec<T> {
    type Api = Vec<T::Api>;
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        self.iter().map(TryIntoApi::try_into_api).collect()
    }
}
impl<C: ?Sized, T: TryIntoEngine<C>> TryIntoEngine<C> for Vec<T> {
    type Engine = Vec<T::Engine>;
    type Error = T::Error;
    fn try_into_engine(&self, context: &C) -> Result<Self::Engine, Self::Error> {
        self.as_slice().try_into_engine(context)
    }
}
impl<C: ?Sized, T: TryIntoEngine<C>> TryIntoEngine<C> for [T] {
    type Engine = Vec<T::Engine>;
    type Error = T::Error;
    fn try_into_engine(&self, context: &C) -> Result<Self::Engine, Self::Error> {
        self.iter()
            .map(|value| value.try_into_engine(context))
            .collect()
    }
}
impl<C: ?Sized> TryIntoEngine<C> for (mod_api::Text, u32) {
    type Engine = (sunrise_game_data::text::Text, u32);
    type Error = mod_api::text::TextFormError;
    fn try_into_engine(&self, context: &C) -> Result<Self::Engine, Self::Error> {
        Ok((self.0.try_into_engine(context)?, self.1))
    }
}

impl<A: TryIntoApi, B: TryIntoApi<Error = A::Error>> TryIntoApi for (A, B) {
    type Api = (A::Api, B::Api);
    type Error = A::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok((self.0.try_into_api()?, self.1.try_into_api()?))
    }
}
impl<C: ?Sized> TryIntoEngine<C> for (mod_api::Text, mod_api::Text) {
    type Engine = (sunrise_game_data::text::Text, sunrise_game_data::text::Text);
    type Error = mod_api::text::TextFormError;
    fn try_into_engine(&self, context: &C) -> Result<Self::Engine, Self::Error> {
        Ok((
            self.0.try_into_engine(context)?,
            self.1.try_into_engine(context)?,
        ))
    }
}

impl TryIntoApi for crate::game::spill_common::BlockItemDrop {
    type Api = mod_api::ItemDrop;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(self.state.to_api(mod_api::ItemDrop::new(
            self.stack.try_into_api()?,
            self.position,
            self.velocity,
            self.pickup_delay,
        )))
    }
}
impl TryIntoEngine for mod_api::ItemDrop {
    type Engine = crate::game::spill_common::BlockItemDrop;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        let stack = self.stack.try_into_engine(items)?;
        Ok(Self::Engine::from_api(self, stack))
    }
}

/// The engine's display context for the API's, `None` for one this game doesn't have.
pub fn engine_display_context(
    context: mod_api::ItemDisplayContext,
) -> Option<sunrise_assets::item_model::DisplayContext> {
    use mod_api::ItemDisplayContext as Api;
    use sunrise_assets::item_model::DisplayContext as Engine;
    Some(match context {
        Api::None => Engine::None,
        Api::ThirdPersonLeftHand => Engine::ThirdPersonLeftHand,
        Api::ThirdPersonRightHand => Engine::ThirdPersonRightHand,
        Api::FirstPersonLeftHand => Engine::FirstPersonLeftHand,
        Api::FirstPersonRightHand => Engine::FirstPersonRightHand,
        Api::Head => Engine::Head,
        Api::Gui => Engine::Gui,
        Api::Ground => Engine::Ground,
        Api::Fixed => Engine::Fixed,
        Api::OnShelf => Engine::OnShelf,
        _ => return None,
    })
}

impl TryIntoApi for sunrise_sim::merchant::ItemCost {
    type Api = mod_api::ItemCost;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::ItemCost::new(self.stack.try_into_api()?))
    }
}
impl TryIntoEngine for mod_api::ItemCost {
    type Engine = sunrise_sim::merchant::ItemCost;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        // ItemCost's codec uses a positive Java int, not the normal stack's count limit.
        let maximum = i32::MAX as u32;
        if self.stack.count() > maximum {
            return Err(StackWriteError::CountExceeded { maximum });
        }
        let stack = self.stack.try_into_engine_preserving_count(items)?;
        if &*stack.item == "minecraft:air" {
            return Err(StackWriteError::EmptyEngineStack);
        }
        Ok(Self::Engine { stack })
    }
}
impl TryIntoApi for sunrise_sim::merchant::MerchantOffer {
    type Api = mod_api::MerchantOffer;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let mut offer = mod_api::MerchantOffer::new(
            self.base_cost_a.try_into_api()?,
            self.cost_b.try_into_api()?,
            self.result.try_into_api()?,
            self.max_uses,
            self.xp,
            self.price_multiplier,
        );
        offer.uses = self.uses;
        offer.reward_exp = self.reward_exp;
        offer.special_price_diff = self.special_price_diff;
        offer.demand = self.demand;
        Ok(offer)
    }
}
#[derive(Debug)]
pub enum OfferWriteError {
    NonFinitePrice,
    Stack(StackWriteError),
}

impl From<StackWriteError> for OfferWriteError {
    fn from(error: StackWriteError) -> Self {
        Self::Stack(error)
    }
}

/// A collect answer and a MerchantRef write admit offers through the same check. All i32
/// fields fit `MerchantOffer.CODEC`; their arithmetic follows Java wrapping in the engine.
pub fn checked_merchant_offer(
    offer: &mod_api::MerchantOffer,
    items: &Items,
) -> Result<sunrise_sim::merchant::MerchantOffer, OfferWriteError> {
    if !offer.price_multiplier.is_finite() {
        return Err(OfferWriteError::NonFinitePrice);
    }
    let result = offer.result.try_into_engine(items)?;
    if &*result.item == "minecraft:air" {
        return Err(StackWriteError::EmptyEngineStack.into());
    }
    Ok(sunrise_sim::merchant::MerchantOffer {
        base_cost_a: offer.base_cost_a.try_into_engine(items)?,
        cost_b: offer.cost_b.try_into_engine(items)?,
        result,
        uses: offer.uses,
        max_uses: offer.max_uses,
        reward_exp: offer.reward_exp,
        special_price_diff: offer.special_price_diff,
        demand: offer.demand,
        price_multiplier: offer.price_multiplier,
        xp: offer.xp,
    })
}

impl TryIntoEngine for mod_api::MerchantOffer {
    type Engine = sunrise_sim::merchant::MerchantOffer;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        checked_merchant_offer(self, items).map_err(|error| match error {
            // MerchantRef's existing write API returns StackWriteError; the decision coordinator
            // uses the typed offer refusal directly.
            OfferWriteError::NonFinitePrice => StackWriteError::InvalidMaximum,
            OfferWriteError::Stack(error) => error,
        })
    }
}

pub type StackReadMetadata = (mod_api::ItemKey, NonZeroU32, Option<u32>);

/// Resolve the item and check the count without copying the stack or encoding its patch.
pub fn stack_read_metadata(stack: &EngineStack) -> Result<StackReadMetadata, StackReadError> {
    let key = mod_api::__private::item_key(&stack.item).ok_or(StackReadError::UnknownItem)?;
    let count = NonZeroU32::new(stack.count).ok_or(StackReadError::Empty)?;
    let damage = stack
        .carries_component(None, "minecraft:damage")
        .then_some(stack.damage);
    Ok((key, count, damage))
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    pub static STACK_PROJECTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static STACK_OVERRIDE_PROJECTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Lend the stack to a whole dispatch. Readers share the first encoding of each requested
/// override (including a miss). The full patch is encoded only for an explicit owned read.
pub fn with_stack_view<R>(
    stack: Option<&EngineStack>,
    read: impl FnOnce(Option<mod_api::ItemStackView<'_>>) -> R,
) -> R {
    let Some((stack, metadata)) =
        stack.and_then(|stack| Some((stack, stack_read_metadata(stack).ok()?)))
    else {
        return read(None);
    };
    let owned = std::cell::OnceCell::new();
    let project = |metadata| {
        owned
            .get_or_init(|| project_stack(stack, metadata).ok())
            .clone()
    };
    let overrides = std::cell::RefCell::new(FxHashMap::<String, Option<Tag>>::default());
    let read_override = |id: &str| {
        let mut overrides = overrides.borrow_mut();
        if let Some(value) = overrides.get(id) {
            return value.clone();
        }
        #[cfg(any(test, feature = "test-support"))]
        STACK_OVERRIDE_PROJECTIONS.set(STACK_OVERRIDE_PROJECTIONS.get() + 1);
        let bare = id.strip_prefix('!').unwrap_or(id);
        let value =
            match sunrise_items::item_data::encode_extra_components_matching(stack, |name| {
                name == bare
            }) {
                Some(Tag::Compound(mut patch)) => patch.remove(&JavaString::from(id)),
                _ => None,
            };
        overrides.insert(id.to_owned(), value.clone());
        value
    };
    read(Some(mod_api::ItemStackView::resident_with_components(
        metadata,
        &project,
        &read_override,
    )))
}

/// Encode and normalize the patch using the scalars already read from this same stack.
pub fn project_stack(
    stack: &EngineStack,
    metadata: StackReadMetadata,
) -> Result<ApiStack, StackReadError> {
    #[cfg(any(test, feature = "test-support"))]
    STACK_PROJECTIONS.set(STACK_PROJECTIONS.get() + 1);
    let patch = sunrise_items::item_data::live_component_patch(stack, false);
    projected_stack(metadata, patch)
}

/// Build either a full or filtered projection with the same checked damage normalization.
pub fn projected_stack(
    (key, count, damage): StackReadMetadata,
    patch: Option<Tag>,
) -> Result<ApiStack, StackReadError> {
    let mut result = ApiStack::from_scalars(key, count, damage);
    if let Some(Tag::Compound(patch)) = patch {
        result
            .set_components(patch)
            .map_err(StackReadError::InvalidComponents)?;
    }
    Ok(result)
}

impl TryIntoApi for EngineStack {
    type Api = ApiStack;
    type Error = StackReadError;

    fn try_into_api(&self) -> Result<ApiStack, Self::Error> {
        project_stack(self, stack_read_metadata(self)?)
    }
}

impl TryIntoApi for sunrise_assets::item_model::StackView {
    type Api = ApiStack;
    type Error = StackReadError;

    fn try_into_api(&self) -> Result<ApiStack, StackReadError> {
        let count = u32::try_from(self.count)
            .ok()
            .filter(|&count| count > 0)
            .ok_or(StackReadError::Empty)?;
        let components = Tag::compound(self.patch.iter().map(|(name, value)| match value {
            Some(value) => (
                sunrise_core::nbt::JavaString::from(name.as_str()),
                sunrise_game_data::scoreboard::json_to_tag(value),
            ),
            None => (
                sunrise_core::nbt::JavaString::from(format!("!{name}").as_str()),
                Tag::Compound(Compound::new()),
            ),
        }));
        let key = mod_api::__private::item_key(&self.item).ok_or(StackReadError::UnknownItem)?;
        let mut stack = ApiStack::new(key, count).ok_or(StackReadError::Empty)?;
        let Tag::Compound(patch) = components else {
            unreachable!("model patch is constructed as a compound")
        };
        stack
            .set_components(patch)
            .map_err(StackReadError::InvalidComponents)?;
        Ok(stack)
    }
}

impl TryIntoEngine for mod_api::ItemResource {
    type Engine = EngineStack;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        self.as_stack().try_into_engine(items)
    }
}

/// Encode an API value for a read-only predicate or drawing, without imposing resident maxima.
/// Unknown keys and unrepresentable/invalid effect payloads still refuse conversion.
/// The write conversion below adds authoritative registry-dependent validation.
pub fn stack_value(value: &ApiStack) -> Result<EngineStack, StackWriteError> {
    let id = mod_api::__private::item_key_id(value.item()).ok_or(StackWriteError::UnknownItem)?;
    encode_stack(value, id.into())
}

pub fn encode_stack(value: &ApiStack, item: Arc<str>) -> Result<EngineStack, StackWriteError> {
    let stack = decode_stack_record(value, item);
    // Check the original override even if its value failed to decode.
    let patch = value.components().map(|patch| patch.as_nbt());
    content_declarations::validate_decoded_components(|name| {
        patch?.get(&JavaString::from(name)).map(|_| &stack)
    })?;
    Ok(stack)
}

/// Decode the record's component overrides without changing their saved keys.
pub fn decode_stack_record(value: &ApiStack, item: Arc<str>) -> EngineStack {
    let mut patch = value.components().map(|patch| patch.as_nbt().clone());
    // Let the existing component codec preserve explicit zero versus prototype inheritance.
    if value.damage_component() == Some(0) {
        patch
            .get_or_insert_with(Default::default)
            .insert(JavaString::from("minecraft:damage"), Tag::Int(0));
    }
    let patch = patch.map(Tag::Compound);
    let (_, extra) = sunrise_items::item_data::decode_components(patch.as_ref());
    EngineStack {
        item,
        count: value.count(),
        damage: value.damage(),
        extra,
    }
}

impl TryIntoApi for sunrise_sim::crafting::RecipeRule {
    type Api = mod_api::RecipeRule;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(match self {
            Self::Stonecutting { input, result } => mod_api::RecipeRule::Stonecutting {
                input: input.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::SmithingTransform {
                template,
                base,
                addition,
                result,
            } => mod_api::RecipeRule::SmithingTransform {
                template: template.try_into_api()?,
                base: base.try_into_api()?,
                addition: addition.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::SmithingTrim {
                template,
                base,
                addition,
                pattern,
            } => mod_api::RecipeRule::SmithingTrim {
                template: template.try_into_api()?,
                base: base.try_into_api()?,
                addition: addition.try_into_api()?,
                pattern: pattern.try_into_api()?,
            },
            Self::BannerDuplicate { banner, result } => mod_api::RecipeRule::BannerDuplicate {
                banner: banner.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::ShieldDecoration {
                banner,
                target,
                result,
            } => mod_api::RecipeRule::ShieldDecoration {
                banner: banner.try_into_api()?,
                target: target.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::Transmute {
                input,
                material,
                min,
                max,
                add_count,
                result,
            } => mod_api::RecipeRule::Transmute {
                input: input.try_into_api()?,
                material: material.try_into_api()?,
                min: *min,
                max: *max,
                add_count: *add_count,
                result: result.try_into_api()?,
            },
            Self::Dye {
                target,
                dye,
                result,
            } => mod_api::RecipeRule::Dye {
                target: target.try_into_api()?,
                dye: dye.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::Imbue {
                source,
                material,
                result,
            } => mod_api::RecipeRule::Imbue {
                source: source.try_into_api()?,
                material: material.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::BookCloning {
                source,
                material,
                min,
                max,
                result,
            } => mod_api::RecipeRule::BookCloning {
                source: source.try_into_api()?,
                material: material.try_into_api()?,
                min: *min,
                max: *max,
                result: result.try_into_api()?,
            },
            Self::FireworkRocket {
                shell,
                fuel,
                star,
                result,
            } => mod_api::RecipeRule::FireworkRocket {
                shell: shell.try_into_api()?,
                fuel: fuel.try_into_api()?,
                star: star.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::FireworkStar {
                dye,
                fuel,
                trail,
                twinkle,
                shapes,
                result,
            } => mod_api::RecipeRule::FireworkStar {
                dye: dye.try_into_api()?,
                fuel: fuel.try_into_api()?,
                trail: trail.try_into_api()?,
                twinkle: twinkle.try_into_api()?,
                shapes: shapes.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::FireworkStarFade {
                dye,
                target,
                result,
            } => mod_api::RecipeRule::FireworkStarFade {
                dye: dye.try_into_api()?,
                target: target.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::MapExtending {
                map,
                material,
                result,
            } => mod_api::RecipeRule::MapExtending {
                map: map.try_into_api()?,
                material: material.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::DecoratedPot {
                back,
                left,
                right,
                front,
                result,
            } => mod_api::RecipeRule::DecoratedPot {
                back: back.try_into_api()?,
                left: left.try_into_api()?,
                right: right.try_into_api()?,
                front: front.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::RepairItem => mod_api::RecipeRule::RepairItem,
        })
    }
}

impl TryIntoApi for sunrise_sim::crafting::RecipeTemplate {
    type Api = mod_api::RecipeTemplate;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::RecipeTemplate::new(
            self.item.clone(),
            self.count,
            self.components.clone(),
        ))
    }
}
impl TryIntoApi for sunrise_sim::crafting::RecipeTrimPattern {
    type Api = mod_api::RecipeTrimPattern;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(match self {
            Self::Reference(id) => mod_api::RecipeTrimPattern::Reference(id.clone()),
            Self::Inline {
                asset_id,
                description,
                decal,
            } => mod_api::RecipeTrimPattern::Inline {
                asset_id: asset_id.clone(),
                description: description.clone(),
                decal: *decal,
            },
        })
    }
}
impl TryIntoApi for sunrise_sim::crafting::FireworkShapeIngredient {
    type Api = mod_api::FireworkShapeIngredient;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::FireworkShapeIngredient::new(
            self.shape.try_into_api().unwrap(),
            self.ingredient.try_into_api()?,
        ))
    }
}
/// A recipe listing crosses one entry at a time: one unrepresentable recipe must not
/// hide the rest of its type. Other collection conversions remain all-or-nothing.
#[derive(Default)]
pub struct RecipeListing(pub std::sync::Arc<[sunrise_sim::crafting::RecipeSnapshot]>);
impl TryIntoApi for RecipeListing {
    type Api = std::sync::Arc<[mod_api::Recipe]>;
    type Error = std::convert::Infallible;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(self
            .0
            .iter()
            .filter_map(|recipe| match recipe.try_into_api() {
                Ok(recipe) => Some(recipe),
                Err(reason) => {
                    log::debug!(
                        "Skipping recipe {} in the mod API listing: {reason:?}",
                        recipe.id
                    );
                    None
                }
            })
            .collect())
    }
}
impl TryIntoApi for sunrise_sim::crafting::RecipeSnapshot {
    type Api = mod_api::Recipe;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let mut result =
            mod_api::Recipe::new(self.id.clone(), self.kind.clone(), self.json.clone());
        result.group = self.group.clone();
        result.ingredients = self.ingredients.try_into_api()?;
        result.results = self.results.try_into_api()?;
        result.cooking_time = self.cooking_time;
        result.experience = self.experience;
        result.rule = self.rule.try_into_api()?;
        Ok(result)
    }
}

/// Install the engine-facing callbacks; snapshots never enter engine storage.
pub fn install_stack_hooks() {
    sunrise_sim::native_stack_hooks::install(sunrise_sim::native_stack_hooks::StackHooks {
        enchantments: |stack, target, value| {
            if mod_api::__private::listened(mod_api::HookId::GetEnchantmentLevel) {
                with_stack_view(Some(stack), |view| {
                    if let Some(view) = view {
                        mod_api::item::enchantments::native_get_enchantment_level(
                            view, target, value,
                        );
                    }
                });
            }
        },
        attributes: |stack, value| {
            if mod_api::__private::listened(mod_api::HookId::ItemAttributeModifier) {
                with_stack_view(Some(stack), |view| {
                    if let Some(view) = view {
                        mod_api::item::item_attributes::native_item_attribute_modifier(view, value);
                    }
                });
            }
        },
    });
}

/// Adapt a player's menu `stillValid` check; the game asks only for a menu
/// whose type has a part answering it (`menu_still_valid_listens`, when it opened).
pub fn menu_still_valid(
    context: &sunrise_world::native_load_context::LoopContext,
    player: u128,
    menu: &str,
    pos: Option<[i32; 3]>,
    eye: [f64; 3],
    opening_item: Option<mod_api::MenuOpeningItem>,
    valid: &mut bool,
) {
    let reader = sunrise_level::api_context::LoopReader(context);
    let context = reader.context();
    mod_api::dispatch_menu_still_valid(&context, player, menu, pos, eye, opening_item, valid);
}

impl TryIntoApi for sunrise_sim::menu::AnvilCraftSnapshot {
    type Api = (Option<ApiStack>, Option<ApiStack>, ApiStack);
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok((
            self.left.try_into_api()?,
            self.right.try_into_api()?,
            self.output.try_into_api()?,
        ))
    }
}

#[cfg(test)]
pub mod attribute_boundary_tests {
    use super::EngineStack as ItemStack;
    use mod_api::{
        AttributeModifier, AttributeOperation, EquipmentSlotGroup, Hooks, Identifier,
        ItemAttributeModifierEvent, ModEntry,
    };
    use mod_platform::testing::with_mods;
    use std::borrow::Cow;
    use sunrise_items::item_data::ModifierRow;
    use sunrise_sim::item_attributes::attribute_modifiers_or;
    /// Makes a netherite chestplate heavier: +0.4 knockback resistance in
    /// the chest slot, and no armor toughness.
    struct Heavy;
    #[mod_api::__subscriptions]
    impl Hooks for Heavy {
        fn item_attribute_modifier(&self, event: &mut ItemAttributeModifierEvent<'_>) {
            if mod_api::__private::item_key_id(event.stack.item())
                != Some("minecraft:netherite_chestplate")
            {
                return;
            }
            let id = |text: &str| Identifier::parse(text).unwrap();
            let heavy = |amount| {
                AttributeModifier::new(id("test:heavy"), amount, AttributeOperation::AddValue)
            };
            let resistance = id("minecraft:knockback_resistance");
            assert!(
                event
                    .modifiers
                    .add(resistance.clone(), heavy(0.4), EquipmentSlotGroup::Chest)
            );
            // The id is taken on that attribute.
            assert!(
                !event
                    .modifiers
                    .add(resistance, heavy(1.0), EquipmentSlotGroup::Chest)
            );
            assert!(
                event
                    .modifiers
                    .remove_all_for(&id("minecraft:armor_toughness"))
            );
        }
    }

    fn chestplate() -> Vec<ModifierRow> {
        ["minecraft:armor", "minecraft:armor_toughness"]
            .map(|attribute| {
                ModifierRow::new(
                    attribute,
                    "minecraft:armor.chestplate",
                    3.0,
                    "add_value",
                    "chest",
                )
            })
            .to_vec()
    }

    /// A mod's edits are what the stack grants; a stack it leaves alone, and
    /// any stack with no mod listening, keeps the item's list borrowed.
    #[test]
    fn a_mod_edits_what_a_stack_grants() {
        super::prepare_stack_boundary_tests();
        let types = chestplate();
        let netherite = ItemStack::new("minecraft:netherite_chestplate", 1);
        let diamond = ItemStack::new("minecraft:diamond_chestplate", 1);
        with_mods(Vec::new().into(), || {
            assert!(matches!(
                attribute_modifiers_or(&netherite, &types),
                Cow::Borrowed(_)
            ));
        });
        with_mods(vec![ModEntry::new("heavy", Box::new(Heavy))].into(), || {
            assert_eq!(
                &*attribute_modifiers_or(&netherite, &types),
                [
                    types[0].clone(),
                    ModifierRow::new(
                        "minecraft:knockback_resistance",
                        "test:heavy",
                        0.4,
                        "add_value",
                        "chest"
                    ),
                ]
            );
            assert!(matches!(
                attribute_modifiers_or(&diamond, &types),
                Cow::Borrowed(_)
            ));
        });
    }

    /// A chestplate a server sent with modifiers of its own, worn for the first
    /// time: the first equipment pass (`collectEquipmentChanges` with nothing
    /// worn before) asks the mods about the decoded list, so the map's first
    /// values are the mod's answer.
    #[test]
    fn a_servers_stack_takes_the_mods_answer_on_first_equip() {
        use std::sync::Arc;
        use sunrise_core::vanilla_bytes::Writer;
        use sunrise_items::equipment::{CHEST, Equipment};
        use sunrise_protocol::buf::{Codec, Reader};
        use sunrise_protocol::item::{Slot, component};
        use sunrise_protocol::registry::{Builtin, Dynamic, Ids};
        use sunrise_sim::living_attributes::LivingAttributes;
        use sunrise_world::{attribute_map::ValueKind, attributes::entity::Catalog};
        super::prepare_stack_boundary_tests();
        let attributes = [
            "minecraft:armor",
            "minecraft:armor_toughness",
            "minecraft:knockback_resistance",
        ];
        let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mut components = vec![String::new(); 17];
        components[component::ATTRIBUTE_MODIFIERS as usize] =
            "minecraft:attribute_modifiers".into();
        let builtin = Builtin {
            registries: [
                (
                    "minecraft:item".to_owned(),
                    Ids::from_names(names(&["minecraft:air", "minecraft:netherite_chestplate"])),
                ),
                (
                    "minecraft:data_component_type".to_owned(),
                    Ids::from_names(components),
                ),
                (
                    "minecraft:attribute".to_owned(),
                    Ids::from_names(names(&attributes)),
                ),
            ]
            .into(),
        };
        // The server's patch: 10 armor and 4 toughness in the chest slot group (6).
        let mut wire = Writer::new();
        for v in [1, 1, 1, 0, component::ATTRIBUTE_MODIFIERS, 2] {
            wire.varint(v);
        }
        for (attribute, amount) in [(0, 10.0), (1, 4.0)] {
            wire.varint(attribute);
            wire.string("test:forged");
            wire.f64(amount);
            for v in [0, 6, 0] {
                wire.varint(v);
            }
        }
        let sent = Slot::read(&mut Reader::new(&wire.data)).unwrap().unwrap();
        let chestplate = sunrise_sim::item_sync::from_network(&sent, &builtin, &Dynamic::default());
        let bits = |n: f64| format!("{:016x}", n.to_bits());
        let catalog = Arc::new(
            Catalog::from_json(&serde_json::json!({
                "data_version": sunrise_world::DATA_VERSION,
                "attributes": attributes.map(|id| serde_json::json!({
                    "id": id, "default": bits(0.0), "min": bits(0.0), "max": bits(30.0),
                    "syncable": true
                })),
                "entities": {
                    "minecraft:armor_stand": serde_json::Map::from_iter(
                        attributes.map(|id| (id.to_owned(), serde_json::json!(bits(0.0))))
                    )
                }
            }))
            .unwrap(),
        );
        let items = crate::game::api_values::stack_limit_tests::catalog();
        let mut equipment = Equipment::default();
        equipment.set(CHEST, Some(chestplate));
        with_mods(vec![ModEntry::new("heavy", Box::new(Heavy))].into(), || {
            let mut live = LivingAttributes::default();
            live.collect_equipment_changes(
                &catalog,
                "minecraft:armor_stand",
                &equipment,
                &items,
                Vec::new,
            )
            .unwrap();
            let value = |name| {
                live.read(
                    &catalog,
                    "minecraft:armor_stand",
                    name,
                    ValueKind::Computed,
                    Vec::new,
                )
            };
            // The server's armor, the mod's knockback resistance, no toughness.
            assert_eq!(value("minecraft:armor"), Some(10.0));
            assert_eq!(value("minecraft:knockback_resistance"), Some(0.4));
            assert_eq!(value("minecraft:armor_toughness"), Some(0.0));
        });
    }
}

#[cfg(test)]
pub mod enchantment_boundary_tests {
    use super::EngineStack as ItemStack;
    use mod_api::{Component, GetEnchantmentLevelEvent, Hooks, Identifier, ModEntry, Tag};
    use mod_platform::testing::with_mods;
    use std::borrow::Cow;
    use std::sync::Arc;
    use sunrise_sim::enchantment_levels::get_enchantment_level;
    fn id(text: &str) -> Identifier {
        Identifier::parse(text).unwrap()
    }
    struct Note;
    impl Component for Note {
        type Value = i32;
        fn id(&self) -> &str {
            "test:note"
        }
        fn decode(&self, value: &Tag) -> Option<i32> {
            match value {
                Tag::Int(value) => Some(*value),
                _ => None,
            }
        }
        fn encode(&self, value: i32) -> Tag {
            Tag::Int(value)
        }
    }

    /// An iron sword fights as if it had Looting II and no Sharpness.
    struct Keen;
    #[mod_api::__subscriptions]
    impl Hooks for Keen {
        fn get_enchantment_level(&self, event: &mut GetEnchantmentLevelEvent<'_>) {
            if mod_api::__private::item_key_id(event.stack.item()) != Some("minecraft:iron_sword") {
                return;
            }
            let encoded = super::STACK_OVERRIDE_PROJECTIONS.get();
            assert_eq!(event.stack.get(&Note), Some(17));
            assert_eq!(event.stack.get(&Note), Some(17));
            assert_eq!(super::STACK_OVERRIDE_PROJECTIONS.get(), encoded + 1);
            let looting = id("minecraft:looting");
            if event.target.is_none_or(|target| *target == looting) {
                event.enchantments.upgrade(looting, 2);
            }
            event.enchantments.set(id("minecraft:sharpness"), 0);
        }
    }

    /// Gameplay reads see the mod's levels; the stack's own component, a
    /// stack the mod leaves alone and every stack with no mod listening
    /// keep theirs, borrowed.
    #[test]
    fn a_mod_changes_the_levels_gameplay_reads() {
        super::prepare_stack_boundary_tests();
        let _ = sunrise_items::native_enchantment_hooks::install(get_enchantment_level);
        let mut iron = ItemStack::new("minecraft:iron_sword", 1);
        iron.enchant("minecraft:sharpness", 3);
        iron.data_mut().set_other("test:note", Some(Tag::Int(17)));
        let mut diamond = ItemStack::new("minecraft:diamond_sword", 1);
        diamond.enchant("minecraft:sharpness", 3);
        with_mods(Vec::new().into(), || {
            assert!(matches!(iron.all_enchantments(), Cow::Borrowed(_)));
            assert_eq!(iron.enchantment_level("minecraft:sharpness"), 3);
        });
        with_mods(vec![ModEntry::new("keen", Box::new(Keen))].into(), || {
            let projected = super::STACK_PROJECTIONS.get();
            assert_eq!(
                &*iron.all_enchantments(),
                [(Arc::from("minecraft:looting"), 2)]
            );
            assert_eq!(iron.enchantment_level("minecraft:looting"), 2);
            assert_eq!(iron.enchantment_level("minecraft:sharpness"), 0);
            assert_eq!(iron.tag_enchantment_level("minecraft:sharpness"), 3);
            assert_eq!(iron.enchantments(), [(Arc::from("minecraft:sharpness"), 3)]);
            assert!(matches!(diamond.all_enchantments(), Cow::Borrowed(_)));
            assert_eq!(diamond.enchantment_level("minecraft:sharpness"), 3);
            assert_eq!(super::STACK_PROJECTIONS.get(), projected);
        });
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn prepare_stack_boundary_tests() {
    install_stack_hooks();
    install_parts(&[]).unwrap();
    resolve_stack_boundary_test_items();
}

#[cfg(any(test, feature = "test-support"))]
pub fn resolve_stack_boundary_test_items() {
    mod_api::__private::resolve_items([
        "minecraft:iron_sword",
        "minecraft:stick",
        "minecraft:diamond_sword",
        "minecraft:netherite_chestplate",
        "minecraft:diamond_chestplate",
        "minecraft:potion",
    ])
    .unwrap();
}

#[cfg(test)]
pub mod recipe_listing_tests {
    use super::*;
    #[test]
    fn a_bad_recipe_does_not_hide_the_good_recipe_of_its_type() {
        let mut registries = mod_api::testing::FakeRegistries::default();
        registries
            .resolve_items([mod_api::Identifier::parse("minecraft:stick").unwrap()])
            .unwrap();
        let mut good = sunrise_sim::crafting::RecipeSnapshot::new(
            "example:good",
            "example:machine",
            serde_json::json!({}).into(),
        );
        good.results = vec![EngineStack::new("minecraft:stick", 1)];
        let mut bad = sunrise_sim::crafting::RecipeSnapshot::new(
            "example:bad",
            "example:machine",
            serde_json::json!({}).into(),
        );
        bad.results = vec![EngineStack::new("example:unknown", 1)];
        let entries = RecipeListing(vec![good, bad].into())
            .try_into_api()
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "example:good");
        assert_eq!(
            mod_api::__private::item_key_id(entries[0].results[0].item()),
            Some("minecraft:stick")
        );
    }
}

#[cfg(test)]
pub mod brewed_boundary_tests {
    use super::*;
    struct NoHost;
    impl mod_api::Host for NoHost {}
    mod_api::impl_host_areas!(NoHost);
    impl mod_api::ServerHost for NoHost {}
    mod_api::impl_server_host_areas!(NoHost);
    struct Watch(std::sync::Arc<std::sync::Mutex<Vec<(u128, i32, String, u32)>>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Watch {
        fn player_brewed_potion(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::PlayerBrewedPotionEvent<'_>,
        ) {
            self.0.lock().unwrap().push((
                event.player,
                event.player_entity_id,
                mod_api::__private::item_key_id(event.stack.item())
                    .unwrap()
                    .to_owned(),
                event.stack.count(),
            ));
        }
    }
    #[test]
    fn brewed_event_reports_player_and_converted_stack() {
        prepare_stack_boundary_tests();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new(
                "watch",
                Box::new(Watch(seen.clone())),
            )]
            .into(),
            || {
                let stack = EngineStack::new("minecraft:potion", 1)
                    .try_into_api()
                    .unwrap();
                mod_api::dispatch_player_brewed_potion(&mut NoHost, 7, 41, &stack);
            },
        );
        assert_eq!(
            *seen.lock().unwrap(),
            [(7, 41, "minecraft:potion".to_owned(), 1)]
        );
    }
}

impl TryIntoApi for sunrise_level::containers::ModBlockEntityFields {
    type Api = mod_api::item::containers::ModBlockEntityFields;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::item::containers::ModBlockEntityFields {
            nbt: self.nbt.clone(),
            items: self.items.try_into_api()?,
            data: self.data.clone(),
        })
    }
}
impl TryIntoEngine for mod_api::item::containers::ModBlockEntityFields {
    type Engine = sunrise_level::containers::ModBlockEntityFields;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(sunrise_level::containers::ModBlockEntityFields {
            nbt: self.nbt.clone(),
            items: self.items.try_into_engine(items)?,
            data: self.data.clone(),
        })
    }
}
impl TryIntoApi for sunrise_level::containers::ModBlockEntity {
    type Api = mod_api::ModBlockEntity;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::ModBlockEntity::from_snapshot(
            self.id().to_owned(),
            (**self).try_into_api()?,
            mod_api::item::containers::ModRevision::from_snapshot(
                self.revision().identity(),
                self.revision().changes(),
            ),
        ))
    }
}

#[cfg(test)]
pub mod mutable_output_tests {
    use super::*;

    #[test]
    fn array_outputs_are_admitted_only_after_every_stack_passes_resident_limits() {
        prepare_stack_boundary_tests();
        let root =
            std::env::temp_dir().join(format!("sunrise-api-array-output-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("items.json"),
            r#"{"minecraft:stick":{"minecraft:max_stack_size":64}}"#,
        )
        .unwrap();
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let items =
            Items::catalog(&root, &sunrise_world::loot::PackOverlay::NONE, &defaults).unwrap();
        std::fs::remove_dir_all(root).unwrap();
        let key = mod_api::__private::item_key("minecraft:stick").unwrap();
        let mut api = [
            ApiStack::new(key, 8).unwrap(),
            ApiStack::new(key, 65).unwrap(),
        ];
        let mut output = [
            EngineStack::new("minecraft:stick", 3),
            EngineStack::new("minecraft:stick", 4),
        ];
        let before = output.clone();
        let converted = api.try_into_engine(&items);
        assert_eq!(
            converted,
            Err(StackWriteError::CountExceeded { maximum: 64 })
        );
        if let Ok(validated) = converted {
            output = validated;
        }
        assert_eq!(
            output, before,
            "a legal first element must not be applied before a rejected second element"
        );

        api[1].set_count(64).unwrap();
        output = api.try_into_engine(&items).unwrap();
        assert_eq!(
            output,
            [
                EngineStack::new("minecraft:stick", 8),
                EngineStack::new("minecraft:stick", 64)
            ]
        );
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod stack_limit_tests {
    use super::*;

    pub fn catalog() -> Items {
        prepare_stack_boundary_tests();
        fixture_catalog()
    }

    fn fixture_catalog() -> Items {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "sunrise-api-limits-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("items.json"), r#"{
            "minecraft:stick":{"minecraft:max_stack_size":64},
            "minecraft:iron_sword":{"minecraft:max_stack_size":1,"minecraft:max_damage":10,"minecraft:damage":7},
            "minecraft:diamond_sword":{"minecraft:max_stack_size":1,"minecraft:max_damage":100,"minecraft:damage":0}
        }"#).unwrap();
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let items =
            Items::catalog(&root, &sunrise_world::loot::PackOverlay::NONE, &defaults).unwrap();
        std::fs::remove_dir_all(root).unwrap();
        items
    }

    #[cfg(test)]
    /// Vanilla `DataComponents.DAMAGE` validates non-negative ints without capping by `max_damage`.
    #[test]
    fn a_write_preserves_damage_above_max_damage() {
        let items = catalog();
        let mut over = ApiStack::of(mod_api::__private::item_key("minecraft:iron_sword").unwrap());
        over.set_damage(1000);
        let definition = items.items.get("minecraft:iron_sword");
        let written = over.try_into_engine(&items).unwrap();
        assert_eq!(written.effective_damage(definition), 1000);
        assert!(sunrise_sim::stations::is_damageable(&items, &written));
        let read = stack_value(&over).unwrap();
        assert_eq!(read.effective_damage(definition), 1000);
    }

    #[cfg(test)]
    #[test]
    fn damage_and_max_damage_follow_vanilla_int_ranges() {
        use sunrise_core::vanilla_bytes::Writer;
        use sunrise_protocol::{
            buf::{Codec, Reader},
            item::{Component, Slot, component},
        };

        let items = catalog();
        let key = |id| mod_api::__private::item_key(id).unwrap();
        let plain = ApiStack::of(key("minecraft:stick"));
        assert!(plain.try_into_engine(&items).is_ok());
        let mut explicit = plain.clone();
        explicit.set_damage(0);
        let engine = explicit.try_into_engine(&items).unwrap();
        assert!(engine.carries_component(None, "minecraft:damage"));
        assert_eq!(
            sunrise_items::item_data::component_snapshot(&engine)
                .unwrap()
                .get("minecraft:damage"),
            Some(&Tag::Int(0))
        );
        explicit.set_damage(5);
        let engine = explicit.try_into_engine(&items).unwrap();
        assert_eq!(engine.damage, 5);
        explicit.set_damage(u32::MAX);
        assert_eq!(
            explicit.try_into_engine(&items),
            Err(StackWriteError::InvalidComponents {
                reason: mod_api::StackError::InvalidDamage
            })
        );

        let inherited = ApiStack::of(key("minecraft:iron_sword"));
        let definition = items.items.get("minecraft:iron_sword").unwrap();
        assert_eq!(
            inherited
                .try_into_engine(&items)
                .unwrap()
                .effective_damage(Some(definition)),
            7
        );
        let mut lower = inherited.clone();
        lower
            .set_components([(JavaString::from("minecraft:max_damage"), Tag::Int(5))].into())
            .unwrap();
        let engine = lower.try_into_engine(&items).unwrap();
        assert_eq!(engine.max_damage(Some(definition)), 5);
        assert_eq!(engine.effective_damage(Some(definition)), 7);

        let mut short_max = inherited.clone();
        short_max
            .set_components([(JavaString::from("minecraft:max_damage"), Tag::Short(20))].into())
            .unwrap();
        let engine = short_max.try_into_engine(&items).unwrap();
        assert_eq!(engine.max_damage(Some(definition)), 20);
        let packet_stack =
            sunrise_sim::lan::guest::stack_to_network(|_| 1, |_, _| None, &Some(engine));
        let mut writer = Writer::new();
        Slot::write(&packet_stack, &mut writer);
        let mut reader = Reader::new(&writer.data);
        let decoded = Slot::read(&mut reader).unwrap().unwrap();
        assert_eq!(
            decoded.component(component::MAX_DAMAGE),
            Some(&Component::Int(20))
        );

        for invalid in [Tag::Int(0), Tag::Int(-1), Tag::Short(0), Tag::Short(-1)] {
            let mut bad_max = inherited.clone();
            bad_max
                .set_components(
                    [(JavaString::from("minecraft:max_damage"), invalid.clone())].into(),
                )
                .unwrap();
            assert_eq!(
                bad_max.try_into_engine(&items),
                Err(StackWriteError::InvalidMaximum),
                "max_damage {invalid:?}"
            );
        }

        let mut removed = inherited;
        removed
            .set_components(
                [(
                    JavaString::from("!minecraft:max_damage"),
                    Tag::Compound(Default::default()),
                )]
                .into(),
            )
            .unwrap();
        let engine = removed.try_into_engine(&items).unwrap();
        assert_eq!(engine.max_damage(Some(definition)), 0);
        assert_eq!(engine.effective_damage(Some(definition)), 7);
        removed.clear_damage();
        let engine = removed.try_into_engine(&items).unwrap();
        assert_eq!(engine.max_damage(Some(definition)), 0);
        assert_eq!(engine.effective_damage(Some(definition)), 0);
        removed.set_damage(0);
        assert!(removed.try_into_engine(&items).is_ok());

        let mut oversized = ApiStack::of(key("minecraft:diamond_sword"));
        oversized.set_damage(i32::MAX as u32);
        assert_eq!(
            oversized.try_into_engine(&items).unwrap().damage,
            i32::MAX as u32
        );
        oversized.set_damage(u32::MAX);
        assert_eq!(
            oversized.try_into_engine(&items),
            Err(StackWriteError::InvalidComponents {
                reason: mod_api::StackError::InvalidDamage
            })
        );
    }

    #[cfg(test)]
    #[test]
    fn stack_maximum_respects_the_vanilla_codec_range() {
        let items = catalog();
        let key = mod_api::__private::item_key("minecraft:stick").unwrap();
        for maximum in [i32::MIN, -1, 0, 100, i32::MAX] {
            let mut stack = ApiStack::of(key);
            stack
                .set_components(
                    [(
                        JavaString::from("minecraft:max_stack_size"),
                        Tag::Int(maximum),
                    )]
                    .into(),
                )
                .unwrap();
            assert_eq!(
                stack.try_into_engine(&items),
                Err(StackWriteError::InvalidMaximum),
                "maximum {maximum}"
            );
        }
        let mut short_max = ApiStack::of(key);
        short_max
            .set_components([("minecraft:max_stack_size".into(), Tag::Short(1))].into())
            .unwrap();
        assert_eq!(
            short_max.try_into_engine(&items),
            Err(StackWriteError::InvalidMaximum)
        );
        for maximum in [1, 99] {
            let mut stack = ApiStack::new(key, maximum).unwrap();
            stack
                .set_components(
                    [(
                        JavaString::from("minecraft:max_stack_size"),
                        Tag::Int(maximum as i32),
                    )]
                    .into(),
                )
                .unwrap();
            assert_eq!(stack.try_into_engine(&items).unwrap().count, maximum);
        }
    }

    #[cfg(test)]
    #[test]
    fn production_snapshots_preserve_component_corpus_contents() {
        let items = catalog();
        let cases = [
            ("minecraft:iron_sword", None),
            (
                "minecraft:iron_sword",
                Some(Tag::compound([("minecraft:damage", Tag::Int(0))])),
            ),
            (
                "minecraft:iron_sword",
                Some(Tag::compound([("minecraft:damage", Tag::Int(5))])),
            ),
            (
                "minecraft:iron_sword",
                Some(Tag::compound([(
                    "!minecraft:damage",
                    Tag::Compound(Default::default()),
                )])),
            ),
            (
                "minecraft:stick",
                Some(Tag::compound([(
                    "minecraft:custom_name",
                    Tag::String("Named".into()),
                )])),
            ),
            (
                "minecraft:stick",
                Some(Tag::compound([(
                    "minecraft:custom_data",
                    Tag::compound([
                        ("list", Tag::List(vec![Tag::Int(3), Tag::Int(7)])),
                        ("bytes", Tag::ByteArray(vec![1, 2, 3])),
                    ]),
                )])),
            ),
            (
                "minecraft:iron_sword",
                Some(Tag::compound([("minecraft:max_stack_size", Tag::Int(1))])),
            ),
            (
                "minecraft:stick",
                Some(Tag::compound([("minecraft:max_stack_size", Tag::Int(99))])),
            ),
            (
                "minecraft:stick",
                Some(Tag::compound([(
                    "!minecraft:max_stack_size",
                    Tag::Compound(Default::default()),
                )])),
            ),
            (
                "minecraft:stick",
                Some(Tag::compound([(
                    "minecraft:container",
                    Tag::List(vec![Tag::compound([
                        ("slot", Tag::Int(0)),
                        (
                            "item",
                            Tag::compound([
                                ("id", Tag::String("minecraft:stick".into())),
                                ("count", Tag::Int(2)),
                            ]),
                        ),
                    ])]),
                )])),
            ),
            (
                "minecraft:stick",
                Some(Tag::compound([(
                    "minecraft:enchantments",
                    Tag::compound([("minecraft:sharpness", Tag::Int(2))]),
                )])),
            ),
        ];
        for (item, patch) in cases {
            let (damage, extra) = sunrise_items::item_data::decode_components(patch.as_ref());
            let original = EngineStack {
                damage,
                extra,
                ..EngineStack::new(item, 1)
            };
            // The engine's canonical component codec is independent of the snapshot adapter.
            let expected = sunrise_items::item_data::encode_extra_components(&original);
            let api = original.try_into_api().unwrap();
            assert_eq!(
                api.components()
                    .map(|components| Tag::Compound(components.as_nbt().clone())),
                expected,
                "read {patch:?}"
            );
            let back = api.try_into_engine(&items).unwrap();
            assert_eq!(
                sunrise_items::item_data::component_snapshot(&original),
                sunrise_items::item_data::component_snapshot(&back),
                "write {patch:?}"
            );
            assert_eq!(original, back, "round trip {patch:?}");
        }
    }

    #[cfg(test)]
    #[test]
    fn durability_and_stackability_are_mutually_exclusive() {
        let items = catalog();
        let mut stick = ApiStack::of(mod_api::__private::item_key("minecraft:stick").unwrap());
        stick
            .set_components([(JavaString::from("minecraft:max_damage"), Tag::Int(10))].into())
            .unwrap();
        assert_eq!(
            stick.try_into_engine(&items),
            Err(StackWriteError::StackableDamageable)
        );
        let mut sword = ApiStack::of(mod_api::__private::item_key("minecraft:iron_sword").unwrap());
        sword
            .set_components([(JavaString::from("minecraft:max_stack_size"), Tag::Int(2))].into())
            .unwrap();
        assert_eq!(
            sword.try_into_engine(&items),
            Err(StackWriteError::StackableDamageable)
        );
        stick
            .set_components(
                [
                    (JavaString::from("minecraft:max_damage"), Tag::Int(10)),
                    (JavaString::from("minecraft:max_stack_size"), Tag::Int(1)),
                ]
                .into(),
            )
            .unwrap();
        assert!(stick.try_into_engine(&items).is_ok());
        sword
            .set_components(
                [
                    (JavaString::from("minecraft:max_stack_size"), Tag::Int(2)),
                    (
                        JavaString::from("!minecraft:max_damage"),
                        Tag::Compound(Default::default()),
                    ),
                ]
                .into(),
            )
            .unwrap();
        sword.clear_damage();
        assert!(sword.try_into_engine(&items).is_ok());
    }

    #[cfg(test)]
    #[test]
    fn removed_stack_maximum_uses_the_absent_component_limit_of_one() {
        let items = catalog();
        let mut stack =
            ApiStack::new(mod_api::__private::item_key("minecraft:stick").unwrap(), 2).unwrap();
        stack
            .set_components(
                [(
                    JavaString::from("!minecraft:max_stack_size"),
                    Tag::Compound(Default::default()),
                )]
                .into(),
            )
            .unwrap();
        assert_eq!(
            stack.try_into_engine(&items),
            Err(StackWriteError::CountExceeded { maximum: 1 })
        );
        stack.set_count(1).unwrap();
        assert!(!stack.try_into_engine(&items).unwrap().carries_component(
            items.items.get("minecraft:stick"),
            "minecraft:max_stack_size"
        ));
    }

    #[cfg(test)]
    #[test]
    fn full_stack_write_round_trips_bare_damage() {
        mod_api::testing::with_parts_registration_probe(
            Default::default(),
            |_| (),
            |_, ()| {
                let mut registries = mod_api::testing::FakeRegistries::default();
                registries
                    .resolve_items([mod_api::Identifier::parse("minecraft:iron_sword").unwrap()])
                    .unwrap();
                let key = mod_api::__private::item_key("minecraft:iron_sword").unwrap();
                let items = fixture_catalog();
                let patch: Compound = [(JavaString::from("damage"), Tag::Int(4))].into();
                let mut draft = ApiStack::of(key);
                draft.set_components(patch.clone()).unwrap();

                let written = draft.try_into_engine(&items).unwrap();
                let round_trip = written.try_into_api().unwrap();
                assert_eq!(round_trip.damage_component(), None);
                assert_eq!(round_trip.components().unwrap().as_nbt(), &patch);
            },
        );
    }

    #[cfg(test)]
    #[test]
    fn damage_only_snapshots_preserve_presence_without_an_opaque_patch() {
        catalog();
        for damage in [0, 5] {
            let (damage, extra) = sunrise_items::item_data::decode_components(Some(
                &Tag::compound([("minecraft:damage", Tag::Int(damage))]),
            ));
            let engine = EngineStack {
                damage,
                extra,
                ..EngineStack::new("minecraft:iron_sword", 1)
            };
            let api = engine.try_into_api().unwrap();
            assert_eq!(api.damage_component(), Some(damage));
            assert!(api.components().is_none());
        }
    }

    #[cfg(test)]
    #[test]
    fn active_registry_limits_survive_world_replacement_and_owner_drop() {
        fn catalog_with(maximum: u32) -> Items {
            let mut items = catalog();
            items.items.get_mut("minecraft:stick").unwrap().max_stack = maximum;
            items
        }

        let base = Arc::new(catalog_with(4));
        let mut world = Arc::new(catalog_with(8));
        let server = Arc::new(catalog_with(16));
        let mut stack = ApiStack::of(mod_api::__private::item_key("minecraft:stick").unwrap());
        stack.set_count(12).unwrap();

        let snapshot = ItemSnapshot::capture(Some(&server), Some(&world), || {
            ItemSnapshot::Shared(Arc::clone(&base))
        });
        drop(server);
        assert_eq!(stack.try_into_engine(&snapshot).unwrap().count, 12);

        let snapshot = ItemSnapshot::capture(None, Some(&world), || {
            ItemSnapshot::Shared(Arc::clone(&base))
        });
        world = Arc::new(catalog_with(32));
        assert_eq!(
            stack.try_into_engine(&snapshot),
            Err(StackWriteError::CountExceeded { maximum: 8 })
        );
        drop(snapshot);
        let snapshot = ItemSnapshot::capture(None, Some(&world), || {
            ItemSnapshot::Shared(Arc::clone(&base))
        });
        drop(world);
        assert_eq!(stack.try_into_engine(&snapshot).unwrap().count, 12);

        let snapshot =
            ItemSnapshot::capture(None, None, || ItemSnapshot::Shared(Arc::clone(&base)));
        drop(base);
        assert_eq!(
            stack.try_into_engine(&snapshot),
            Err(StackWriteError::CountExceeded { maximum: 4 })
        );
    }
}

#[cfg(test)]
pub mod item_model_snapshot_tests {
    use super::*;
    use sunrise_assets::item_model::StackView;

    #[test]
    fn model_snapshots_keep_component_patches_without_runtime_bundle_selection() {
        prepare_stack_boundary_tests();
        let mut view = StackView::new("minecraft:stick", 2);
        view.bundle_selected = 3;
        view.patch
            .insert("minecraft:damage".into(), Some(serde_json::json!(7)));
        view.patch.insert(
            "example:charge".into(),
            Some(serde_json::json!({"amount":4})),
        );
        view.patch.insert("minecraft:custom_name".into(), None);
        let snapshot = view.try_into_api().unwrap();
        assert_eq!(
            snapshot.item(),
            mod_api::__private::item_key("minecraft:stick").unwrap()
        );
        assert_eq!(snapshot.count(), 2);
        assert_eq!(snapshot.damage(), 7);
        let patch = snapshot.components().unwrap().as_nbt();
        assert_eq!(
            patch
                .get(&JavaString::from("example:charge"))
                .unwrap()
                .get("amount"),
            Some(&Tag::Int(4))
        );
        assert!(
            patch
                .get(&JavaString::from("!minecraft:custom_name"))
                .is_some()
        );
        assert!(patch.get(&JavaString::from("bundle_selected")).is_none());
        assert_eq!(view.bundle_selected, 3);
    }

    #[test]
    fn empty_model_views_do_not_become_owned_stack_sentinels() {
        prepare_stack_boundary_tests();
        for count in [-1, 0] {
            assert_eq!(
                StackView::new("minecraft:stick", count).try_into_api(),
                Err(StackReadError::Empty)
            );
        }
        assert_eq!(
            StackView::new("example:unknown", 1).try_into_api(),
            Err(StackReadError::UnknownItem)
        );
    }
}

impl TryIntoApi for sunrise_game_data::suggestion::Suggestion {
    type Api = mod_api::Suggestion;
    type Error = std::convert::Infallible;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let mut value = mod_api::Suggestion::new(self.range, self.text.clone());
        value.tooltip.clone_from(&self.tooltip);
        value.int_value = self.int_value;
        Ok(value)
    }
}
impl TryIntoEngine for mod_api::Suggestion {
    type Engine = sunrise_game_data::suggestion::Suggestion;
    type Error = StackWriteError;
    fn try_into_engine(&self, _: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(Self::Engine {
            range: self.range,
            text: self.text.clone(),
            tooltip: self.tooltip.clone(),
            int_value: self.int_value,
        })
    }
}
#[cfg(test)]
pub mod completion_snapshot_tests {
    use super::*;
    #[test]
    fn completion_round_trip_keeps_range_tooltip_and_numeric_order() {
        let source = sunrise_game_data::suggestion::Suggestion {
            range: (4, 7),
            text: "42".into(),
            tooltip: Some("count".into()),
            int_value: Some(42),
        };
        let mut api = source.try_into_api().unwrap();
        api.text = "43".into();
        api.int_value = Some(43);
        let engine = api
            .try_into_engine(&Items::for_tests(Default::default()))
            .unwrap();
        assert_eq!(engine.range, (4, 7));
        assert_eq!(engine.tooltip.as_deref(), Some("count"));
        assert_eq!(engine.int_value, Some(43));
        assert_eq!(engine.apply("foo 123 bar"), "foo 43 bar");
        assert_eq!(source.text, "42");
    }
}

// Registration declarations do not need a live item registry.
impl TryIntoEngine<()> for mod_api::BlockEntityTypeDecl {
    type Engine = sunrise_mod_package::content::BlockEntityType;
    type Error = std::convert::Infallible;
    fn try_into_engine(&self, _: &()) -> Result<Self::Engine, Self::Error> {
        Ok(Self::Engine {
            id: self.id.clone(),
            blocks: self.blocks.clone(),
        })
    }
}
pub fn block_entity_declarations()
-> impl Iterator<Item = (&'static str, sunrise_mod_package::content::BlockEntityType)> {
    mod_api::__private::block_entity_types()
        .iter()
        .map(|(owner, decl)| (*owner, decl.try_into_engine(&()).unwrap()))
}

#[cfg(test)]
pub mod declaration_snapshot_tests {
    use super::*;
    #[test]
    fn declarations_keep_supported_blocks_and_rule_positions() {
        static KEY: std::sync::OnceLock<mod_api::BoolGameRuleKey> = std::sync::OnceLock::new();
        // Use registration, the owner of declaration validation.
        struct Mod;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Mod {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                r.declare_block_entity_type(
                    "machine",
                    vec!["test:second".into(), "test:first".into()],
                );
                r.game_rule_category("machines").unwrap();
                KEY.set(r.game_rule_bool("active", "test:machines", true).unwrap())
                    .unwrap();
            }
        }
        install_parts(&[mod_api::ModEntry::new("test", Box::new(Mod))]).unwrap();
        let (owner, declaration) = block_entity_declarations().next().unwrap();
        assert_eq!(owner, "test");
        assert_eq!(declaration.id, "test:machine");
        assert_eq!(declaration.blocks, ["test:second", "test:first"]);
        let registry = game_rule_registry();
        let key = *KEY.get().unwrap();
        assert_eq!(
            mod_api::__private::game_rule_bool_slot(key),
            registry.position("test:active")
        );
        assert_eq!(
            registry.rule("test:active").unwrap().default,
            sunrise_game_data::gamerules::Value::Bool(true)
        );
    }
}

#[path = "api_values/text_boundary.rs"]
pub mod text_boundary;
#[cfg(test)]
pub mod text_boundary_tests {
    use super::{TryIntoApi, TryIntoEngine};

    #[test]
    fn tooltip_pairs_round_trip_without_an_item_registry() {
        let lines = vec![(sunrise_game_data::text::Text::literal("tip"), 0xff00ff00)];
        let snapshot: Result<Vec<(mod_api::Text, u32)>, mod_api::text::TextFormError> =
            lines.try_into_api();
        let snapshot = snapshot.unwrap();
        assert_eq!(snapshot[0], (mod_api::Text::literal("tip"), 0xff00ff00));
        let restored: Result<
            Vec<(sunrise_game_data::text::Text, u32)>,
            mod_api::text::TextFormError,
        > = snapshot.try_into_engine(&());
        let restored = restored.unwrap();
        assert_eq!(restored[0].0.component(), lines[0].0.component());
        assert_eq!(restored[0].0.spans, lines[0].0.spans);
        assert_eq!(restored[0].1, lines[0].1);
    }

    #[test]
    fn nested_text_containers_need_no_item_registry_or_stack_error() {
        let pair = (
            mod_api::text::Text::literal("a"),
            mod_api::text::Text::literal("b"),
        );
        let values = Some(vec![[pair.clone(), pair]]);
        let reference = &values;
        let actual: Result<_, mod_api::text::TextFormError> = reference.try_into_engine(&());
        let actual = actual.unwrap().unwrap();
        assert_eq!(actual[0][1].0.plain(), "a");
        assert_eq!(actual[0][1].1.plain(), "b");
    }
}

pub fn placement_source_into_engine(
    source: mod_api::PlacementSource,
) -> Option<sunrise_world::level::PlacementSource> {
    match source {
        mod_api::PlacementSource::Player {
            yaw,
            pitch,
            sneaking,
        } if yaw.is_finite() && pitch.is_finite() => {
            Some(sunrise_world::level::PlacementSource::Player {
                yaw,
                pitch,
                sneaking,
            })
        }
        mod_api::PlacementSource::Directional { direction } => {
            Some(sunrise_world::level::PlacementSource::Directional { direction })
        }
        _ => None,
    }
}

/// A directional placement question vanilla throws on, as `LevelRef::state_for_placement`
/// refuses it.
pub fn placement_refusal_into_api(
    refusal: sunrise_world::level::DirectionalRefusal,
) -> mod_api::PlacementStateError {
    match refusal {
        sunrise_world::level::DirectionalRefusal::NoVerticalLook => {
            mod_api::PlacementStateError::NoVerticalLook
        }
        sunrise_world::level::DirectionalRefusal::ReplacingClickedCycle => {
            mod_api::PlacementStateError::ReplacingClickedCycle
        }
    }
}

pub fn api_game_rule_value(value: sunrise_game_data::gamerules::Value) -> mod_api::GameRuleValue {
    use sunrise_game_data::gamerules::Value;
    match value {
        Value::Bool(value) => mod_api::GameRuleValue::Bool(value),
        Value::Int(value) => mod_api::GameRuleValue::Int(value),
    }
}

impl TryIntoApi for sunrise_world::level::PlacementContext {
    type Api = mod_api::PlacementContext;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let item = if self.item.is_empty() {
            None
        } else {
            Some(mod_api::__private::item_key(&self.item).ok_or(StackReadError::UnknownItem)?)
        };
        let mut snapshot = mod_api::PlacementContext::new(self.pos, self.face, item);
        snapshot.hit_pos = self.hit_pos;
        snapshot.hit = [self.hit.x, self.hit.y, self.hit.z];
        snapshot.inside = self.inside;
        snapshot.source = match self.source {
            sunrise_world::level::PlacementSource::Player {
                yaw,
                pitch,
                sneaking,
            } => mod_api::PlacementSource::Player {
                yaw,
                pitch,
                sneaking,
            },
            sunrise_world::level::PlacementSource::Directional { direction } => {
                mod_api::PlacementSource::Directional { direction }
            }
        };
        snapshot.replace_clicked = self.replace_clicked;
        Ok(snapshot)
    }
}

#[cfg(test)]
pub mod placement_snapshot_tests {
    use super::*;
    #[test]
    fn placement_keeps_click_source_flags_and_empty_hand_without_fallback() {
        install_parts(&[]).unwrap();
        mod_api::__private::resolve_items(["minecraft:stone", "minecraft:shulker_box"]).unwrap();
        let mut engine = sunrise_world::level::PlacementContext {
            pos: sunrise_core::position::BlockPos::new(1, 2, 3),
            hit_pos: sunrise_core::position::BlockPos::new(4, 5, 6),
            face: sunrise_core::geometry::Direction::Up,
            hit: sunrise_core::geometry::Vec3::new(4.25, 5.5, 6.75),
            inside: true,
            source: sunrise_world::level::PlacementSource::Player {
                yaw: -45.0,
                pitch: 90.0,
                sneaking: true,
            },
            item: "minecraft:stone".into(),
            replace_clicked: true,
        };
        let snapshot = engine.try_into_api().unwrap();
        assert_eq!(snapshot.pos, sunrise_core::position::BlockPos::new(1, 2, 3));
        assert_eq!(
            snapshot.hit_pos,
            sunrise_core::position::BlockPos::new(4, 5, 6)
        );
        assert_eq!(snapshot.face, sunrise_core::geometry::Direction::Up);
        assert_eq!(snapshot.hit, [4.25, 5.5, 6.75]);
        assert!(snapshot.inside);
        assert_eq!(
            snapshot.source,
            mod_api::PlacementSource::Player {
                yaw: -45.0,
                pitch: 90.0,
                sneaking: true,
            }
        );
        assert!(snapshot.replace_clicked);
        assert_eq!(
            snapshot.item,
            mod_api::__private::item_key("minecraft:stone")
        );

        // Dispenser directional placement preserves the dispenser's facing.
        let dispenser_engine = sunrise_world::level::PlacementContext {
            pos: sunrise_core::position::BlockPos::new(10, 64, 2),
            hit_pos: sunrise_core::position::BlockPos::new(10, 64, 2),
            face: sunrise_core::geometry::Direction::Up,
            hit: sunrise_core::geometry::Vec3::new(10.5, 64.0, 2.5),
            inside: false,
            source: sunrise_world::level::PlacementSource::Directional {
                direction: sunrise_core::geometry::Direction::East,
            },
            item: "minecraft:shulker_box".into(),
            replace_clicked: true,
        };
        let dispenser_snapshot = dispenser_engine.try_into_api().unwrap();
        assert_eq!(
            dispenser_snapshot,
            mod_api::PlacementContext::directional(
                sunrise_core::position::BlockPos::new(10, 64, 2),
                sunrise_core::geometry::Direction::East,
                sunrise_core::geometry::Direction::Up,
                mod_api::__private::item_key("minecraft:shulker_box"),
            )
        );

        // Falling-block directional check: Directional { Down } with empty hand.
        engine.source = sunrise_world::level::PlacementSource::Directional {
            direction: sunrise_core::geometry::Direction::Down,
        };
        engine.item.clear();
        let mut falling = snapshot;
        falling.source = mod_api::PlacementSource::Directional {
            direction: sunrise_core::geometry::Direction::Down,
        };
        falling.item = None;
        assert_eq!(engine.try_into_api(), Ok(falling));

        engine.item = "missing:item".into();
        assert_eq!(engine.try_into_api(), Err(StackReadError::UnknownItem));

        // Non-finite player orientation is refused at the mod boundary.
        assert_eq!(
            placement_source_into_engine(mod_api::PlacementSource::Player {
                yaw: f32::NAN,
                pitch: 0.0,
                sneaking: false,
            }),
            None
        );
        assert_eq!(
            placement_source_into_engine(mod_api::PlacementSource::Player {
                yaw: 0.0,
                pitch: f32::INFINITY,
                sneaking: true,
            }),
            None
        );
        assert_eq!(
            placement_source_into_engine(mod_api::PlacementSource::Directional {
                direction: sunrise_core::geometry::Direction::Down,
            }),
            Some(sunrise_world::level::PlacementSource::Directional {
                direction: sunrise_core::geometry::Direction::Down,
            })
        );
    }
}

impl TryIntoApi for sunrise_game_data::gamerules::Value {
    type Api = mod_api::GameRuleValue;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(api_game_rule_value(*self))
    }
}

impl TryIntoEngine for mod_api::GameRuleValue {
    type Engine = sunrise_game_data::gamerules::Value;
    type Error = ();
    fn try_into_engine(&self, _: &Items) -> Result<Self::Engine, Self::Error> {
        match *self {
            Self::Bool(value) => Ok(Self::Engine::Bool(value)),
            Self::Int(value) => Ok(Self::Engine::Int(value)),
            // A kind this game has no rule of.
            _ => Err(()),
        }
    }
}

#[test]
pub fn game_rule_int_snapshot_preserves_full_range() {
    for value in [i32::MIN, -1, 0, 256, i32::MAX] {
        assert_eq!(
            sunrise_game_data::gamerules::Value::Int(value)
                .try_into_api()
                .unwrap(),
            mod_api::GameRuleValue::Int(value)
        );
    }
}

pub fn with_rule_schema<T>(f: impl FnOnce(&mod_api::__private::RuleSchema<'_>) -> T) -> T {
    let registry = sunrise_game_data::gamerules::rule_registry();
    f(&mod_api::__private::RuleSchema {
        category: &|id| registry.categories().iter().any(|category| category == id),
        position: &|id| registry.position(id),
        rule_count: u32::try_from(registry.available_rules(true).count())
            .expect("vanilla rule count fits u32"),
    })
}
#[cfg(test)]
pub(in crate::game) fn install_test_platform() {
    mod_platform::__private::test_platform::install();
}
pub fn install_parts(mods: &[mod_api::ModEntry]) -> Result<(), Vec<String>> {
    #[cfg(test)]
    install_test_platform();
    with_rule_schema(|schema| {
        (mod_api::__private::platform::platform()
            .map_err(|e| vec![e.to_string()])?
            .install_parts)(mods, schema, super::native_dimensions::valid_shape)
    })
}
pub fn game_rule_registry() -> sunrise_game_data::gamerules::RuleRegistry {
    let rules: Vec<_> = mod_api::__private::game_rules()
        .iter()
        .map(|rule| rule.try_into_engine(&()).unwrap())
        .collect();
    sunrise_game_data::gamerules::RuleRegistry::with_mod_rules(
        mod_api::__private::game_rule_categories(),
        &rules,
    )
    .expect("validated at registration")
}

#[path = "api_values/game_rule_values.rs"]
pub mod game_rule_values;
#[path = "api_values/loot_values.rs"]
pub mod loot_values;
#[path = "api_values/package_snapshot.rs"]
pub mod package_snapshot;
#[path = "api_values/particle_values.rs"]
pub mod particle_values;
#[allow(unused_imports)]
pub use package_snapshot::install_packages;
#[path = "api_values/phase_values.rs"]
pub mod phase_values;
#[path = "api_values/small_values.rs"]
pub mod small_values;

impl TryIntoApi for (EngineStack, [f64; 3]) {
    type Api = (ApiStack, [f64; 3]);
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok((self.0.try_into_api()?, self.1))
    }
}
impl TryIntoEngine for (ApiStack, [f64; 3]) {
    type Engine = (EngineStack, [f64; 3]);
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok((self.0.try_into_engine(items)?, self.1))
    }
}

impl TryIntoApi for (sunrise_game_data::text::Text, u32) {
    type Api = (mod_api::Text, u32);
    type Error = mod_api::text::TextFormError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok((self.0.try_into_api()?, self.1))
    }
}

impl TryIntoEngine for mod_api::ItemDisplayContext {
    type Engine = sunrise_assets::item_model::DisplayContext;
    type Error = ();
    fn try_into_engine(&self, _: &Items) -> Result<Self::Engine, Self::Error> {
        engine_display_context(*self).ok_or(())
    }
}

impl<T: TryIntoApi, E: Clone> TryIntoApi for Result<T, E> {
    type Api = Result<T::Api, E>;
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        match self {
            Ok(value) => value.try_into_api().map(Ok),
            Err(error) => Ok(Err(error.clone())),
        }
    }
}
