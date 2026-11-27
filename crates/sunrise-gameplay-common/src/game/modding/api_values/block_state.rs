//! Owning-thread, load-bound numeric bridge. Workers receive prepared values instead.
use super::{TryIntoApi, TryIntoEngine};
use mod_api::{BlockState, BlockStateError, LogicalSide};
use std::{cell::RefCell, sync::Arc};
use sunrise_level::block_entities::{ModTickerStates, Ticking};
use sunrise_world::registry::{BlockRegistry, BlockStateId};

/// A fully checked index, inert until explicitly installed by the owning game thread.
pub struct Index {
    forward: Vec<BlockState>,
    mod_tickers: [Arc<ModTickerStates>; 2],
    reverse: std::collections::HashMap<mod_api::BlockKey, Vec<Option<BlockStateId>>>,
    /// Each state's block key, shared with the phase views that tell who listens.
    #[cfg(test)]
    blocks: Arc<[mod_api::BlockKey]>,
}
thread_local! {
    static INDEX: RefCell<Option<Arc<Index>>> = const { RefCell::new(None) };
}

/// Routing reads the precomputed key only: no event value is converted here. Inside a tick
/// phase's worker the phase's `ListenScope` answers, so a worker never gets "no listener" from
/// the empty tables of a thread that owns no game.
pub fn block_listens(hook: mod_api::HookId, state: BlockStateId) -> bool {
    if let Some(listens) = mod_api::__private::scoped_state_listens(hook, state.0) {
        return listens;
    }
    mod_api::__private::has_scoped_hook(hook)
        && INDEX.with_borrow(|index| {
            index
                .as_ref()
                .and_then(|index| index.forward.get(state.0 as usize))
                .is_some_and(|value| mod_api::__private::block_key_listens(hook, value.block()))
        })
}

/// Whether every part on `state` that answers `explosion_resistance` declares the answer pure.
pub fn explosion_resistance_pure(state: BlockStateId) -> bool {
    if let Some(pure) = mod_api::__private::scoped_explosion_resistance_pure(state.0) {
        return pure;
    }
    INDEX.with_borrow(|index| {
        index
            .as_ref()
            .and_then(|index| index.forward.get(state.0 as usize))
            .is_some_and(|value| mod_api::__private::explosion_resistance_key_pure(value.block()))
    })
}

pub fn clear() {
    INDEX.with_borrow_mut(|index| *index = None);
}
impl AsRef<[BlockState]> for Index {
    /// Every engine state as a mod reads it, by state id.
    fn as_ref(&self) -> &[BlockState] {
        &self.forward
    }
}
pub fn snapshot() -> Result<Box<[BlockState]>, BlockStateError> {
    Ok(current()?.forward.clone().into_boxed_slice())
}
pub fn bind(registry: &BlockRegistry) -> Result<(), String> {
    clear();
    install(prepare(registry)?);
    Ok(())
}

/// Bind the numeric bridge restricting block-entity ticker selectors to `role`'s logical side.
pub fn bind_for_role(
    registry: &BlockRegistry,
    role: crate::game::execution_role::ExecutionRole,
) -> Result<(), String> {
    clear();
    install(prepare_for_role(registry, role)?);
    Ok(())
}

pub(in crate::game) fn install(prepared: Arc<Index>) {
    INDEX.with_borrow_mut(|index| *index = Some(prepared));
}

/// Prepare without activating thread-local state. A refused reload keeps its old live tables.
pub(crate) fn prepare(registry: &BlockRegistry) -> Result<Arc<Index>, String> {
    prepare_with_role(registry, None)
}

/// Prepare without activating thread-local state, evaluating only `role`'s ticker selectors.
pub fn prepare_for_role(
    registry: &BlockRegistry,
    role: crate::game::execution_role::ExecutionRole,
) -> Result<Arc<Index>, String> {
    prepare_with_role(registry, Some(role))
}

fn prepare_with_role(
    registry: &BlockRegistry,
    role: Option<crate::game::execution_role::ExecutionRole>,
) -> Result<Arc<Index>, String> {
    let forward = (0..registry.len())
        .map(|id| {
            registry
                .state(BlockStateId(id as u32))
                .expect("registry index")
                .try_into_api()
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("block state index: {error:?}"))?;
    let mut reverse = std::collections::HashMap::<_, Vec<Option<BlockStateId>>>::new();
    for (id, state) in forward.iter().enumerate() {
        let states = reverse.entry(state.block()).or_default();
        states.resize(states.len().max(state.ordinal() as usize + 1), None);
        if states[state.ordinal() as usize]
            .replace(BlockStateId(id as u32))
            .is_some()
        {
            return Err("duplicate block state in numeric registry".into());
        }
    }
    #[cfg(test)]
    let blocks = forward.iter().map(|state| state.block()).collect();
    let mod_tickers = prepare_mod_tickers(registry, &forward, role);
    Ok(Arc::new(Index {
        forward,
        mod_tickers,
        reverse,
        #[cfg(test)]
        blocks,
    }))
}
pub fn mod_ticker_states(side: LogicalSide) -> Arc<ModTickerStates> {
    current().map_or_else(
        |_| Arc::default(),
        |index| {
            index.mod_tickers[match side {
                LogicalSide::Server => 0,
                LogicalSide::Client => 1,
            }]
            .clone()
        },
    )
}

/// Bake `EntityBlock.getTicker` for the registered type and every state of its accepted blocks.
fn prepare_mod_tickers(
    registry: &BlockRegistry,
    values: &[BlockState],
    role: Option<crate::game::execution_role::ExecutionRole>,
) -> [Arc<ModTickerStates>; 2] {
    use sunrise_game_data::block_entity_types;
    use sunrise_level::fast_hash::FxHashMap;
    let mut tables = [ModTickerStates::default(), ModTickerStates::default()];
    let eval_server = role.is_none_or(|r| r.is_logical_server());
    let eval_client = role.is_none_or(|r| r.is_client());
    let Some(types) = block_entity_types::current() else {
        return tables.map(Arc::new);
    };
    let mut by_block = FxHashMap::<_, Vec<_>>::default();
    for (block, kind) in types.memberships() {
        if let Some(binding) = types.registered(kind)
            && ((eval_server && binding.server_ticker.is_some())
                || (eval_client && binding.client_ticker.is_some()))
        {
            by_block.entry(block).or_default().push(binding);
        }
    }
    if by_block.is_empty() {
        return tables.map(Arc::new);
    }
    let metadata = mod_api::__private::block_definitions().expect("prepared block metadata");
    let selectors: FxHashMap<_, _> = mod_api::__private::block_entity_ticker_selectors()
        .iter()
        .map(|selector| (selector.kind, selector))
        .collect();
    for (id, &state) in values.iter().enumerate() {
        let numeric = id as u32;
        let block = &registry
            .state(BlockStateId(numeric))
            .expect("registry index")
            .name;
        let Some(bindings) = by_block.get(block.as_str()) else {
            continue;
        };
        for binding in bindings {
            for (table, (kind, side, enabled)) in tables.iter_mut().zip([
                (binding.server_ticker, LogicalSide::Server, eval_server),
                (binding.client_ticker, LogicalSide::Client, eval_client),
            ]) {
                if !enabled {
                    continue;
                }
                let Some(kind) = kind else {
                    continue;
                };
                let selected = selectors.get(&kind).is_none_or(|selector| {
                    selector
                        .owner
                        .call("block entity ticker", "ticker_when", || {
                            (selector.select)(metadata, state, side)
                        })
                        .unwrap_or(false)
                });
                table.insert(
                    (numeric, binding.numeric_id),
                    if selected {
                        Ticking::Yes
                    } else {
                        Ticking::Idle
                    },
                );
            }
        }
    }
    tables.map(Arc::new)
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)] // TODO(#2273): move the module to the end of the file.
mod tests {
    use super::*;
    #[test]
    fn numeric_block_index_routes_only_subscribed_blocks() {
        struct Part;
        #[mod_api::sunrise_part]
        impl mod_api::BlockPart for Part {
            fn analog_output(
                &self,
                _: &mut dyn mod_api::LevelRandomHost,
                event: &mut mod_api::AnalogOutputEvent,
            ) {
                event.output += 1;
            }
        }
        struct Listener;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Listener {
            fn register(
                &self,
                registrar: &mut dyn mod_api::Registrar,
                _: &mod_api::RegistrationContext,
            ) {
                let key = registrar.block_key("minecraft:stone");
                registrar.block_part(key, Part);
            }
        }
        let mods: mod_api::Mods =
            vec![mod_api::ModEntry::new("routing", Box::new(Listener))].into();
        mod_platform::testing::with_mods(mods.clone(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
            let facts = r#"{"defaults":{"minecraft:air":2,"minecraft:stone":0,"minecraft:dirt":1},"states":[["s",false,false,"","minecraft:stone"],["d",false,false,"","minecraft:dirt"],["a",true,false,"","minecraft:air"]]}"#;
            let registry = BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
            super::super::resolve_block_definitions(&registry).unwrap();
            mod_api::__private::resolve_parts(
                &Default::default(),
                &Default::default(),
                mod_api::__private::KnownIds {
                    block: &|_| true,
                    item: &|_| false,
                },
            )
            .unwrap();
            assert!(block_listens(
                mod_api::HookId::AnalogOutput,
                BlockStateId(0)
            ));
            let start = std::time::Instant::now();
            for _ in 0..1_000 {
                assert!(!block_listens(
                    mod_api::HookId::AnalogOutput,
                    std::hint::black_box(BlockStateId(1))
                ));
            }
            println!(
                "actual Index and Parts: 1000 unrelated-block guards in {} ns",
                start.elapsed().as_nanos()
            );
        });
    }

    /// The listen snapshot answers on any thread and keeps the tables it captured, however the tick
    /// thread's own tables change after; installing and resolving refuse a phase's worker by name.
    #[test]
    fn a_phase_keeps_the_listen_snapshot_it_captured() {
        use sunrise_sim::parallel::TickThread;
        struct Part;
        #[mod_api::sunrise_part]
        impl mod_api::BlockPart for Part {
            fn analog_output(
                &self,
                _: &mut dyn mod_api::LevelRandomHost,
                event: &mut mod_api::AnalogOutputEvent,
            ) {
                event.output += 1;
            }
        }
        struct Listener;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Listener {
            fn register(
                &self,
                registrar: &mut dyn mod_api::Registrar,
                _: &mod_api::RegistrationContext,
            ) {
                let key = registrar.block_key("minecraft:stone");
                registrar.block_part(key, Part);
            }
        }
        let mods: mod_api::Mods =
            vec![mod_api::ModEntry::new("routing", Box::new(Listener))].into();
        mod_platform::testing::with_mods(mods.clone(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
            let facts = r#"{"defaults":{"minecraft:air":2,"minecraft:stone":0,"minecraft:dirt":1},"states":[["s",false,false,"","minecraft:stone"],["d",false,false,"","minecraft:dirt"],["a",true,false,"","minecraft:air"]]}"#;
            let registry = BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
            super::super::resolve_block_definitions(&registry).unwrap();
            mod_api::__private::resolve_parts(
                &Default::default(),
                &Default::default(),
                mod_api::__private::KnownIds {
                    block: &|_| true,
                    item: &|_| false,
                },
            )
            .unwrap();
            let hook = mod_api::HookId::AnalogOutput;
            let tick = TickThread::mint();
            let snapshot = super::super::listen_snapshot(&tick).unwrap();
            assert!(snapshot.listens(hook, BlockStateId(0)));
            assert!(!snapshot.listens(hook, BlockStateId(1)));
            assert!(!snapshot.listens(hook, BlockStateId(77)), "no such state");

            // Pool threads own no game, so their thread-locals are empty; inside the snapshot's
            // scope `block_listens` gives the tick thread's answer for every state.
            let states: Vec<BlockStateId> = (0..3).map(BlockStateId).collect();
            let on_tick: Vec<bool> = states.iter().map(|&s| block_listens(hook, s)).collect();
            assert_eq!(on_tick, [true, false, false]);
            let on_pool = sunrise_sim::parallel::par_map_on(&states, 1, 4, |part, out| {
                let ids: Vec<_> = part
                    .iter()
                    .map(|&s| (s, std::thread::current().id()))
                    .collect();
                out.extend(ids.into_iter().map(|(s, thread)| {
                    (
                        thread,
                        snapshot.enter(|| block_listens(hook, s)),
                        block_listens(hook, s),
                    )
                }));
            });
            let scoped: Vec<bool> = on_pool.iter().map(|(_, scoped, _)| *scoped).collect();
            assert_eq!(scoped, on_tick);
            // Outside the scope a pool thread is still told "no listener" by the empty thread-local;
            // the main thread (this one) is never among them with a different answer.
            for (thread, _, bare) in &on_pool {
                if *thread != std::thread::current().id() {
                    assert!(!bare);
                }
            }

            // The tick thread replaces its tables: the captured snapshot keeps its answers.
            mod_api::__private::clear_game_parts();
            clear();
            assert!(!block_listens(hook, BlockStateId(0)));
            assert!(snapshot.listens(hook, BlockStateId(0)));

            // An installer is the tick thread's: a phase's worker can't mint the token, and
            // resolving a registry there is refused by name before it touches a table.
            sunrise_core::tick_phase::as_tick_phase(|| {
                assert!(std::panic::catch_unwind(TickThread::mint).is_err());
                let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    super::super::resolve_block_definitions(&registry)
                }));
                assert!(refused.is_err());
            });
            assert!(current().is_err(), "the refused resolve installed nothing");
        });
    }

    #[test]
    fn numeric_block_index_prepared_token_activates_only_on_fresh_owner() {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let facts = r#"{"defaults":{"minecraft:air":0,"minecraft:stone":1},"states":[["a",true,false,"","minecraft:air"],["s",false,false,"","minecraft:stone"]]}"#;
        let registry = BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
        super::super::resolve_block_definitions(&registry).unwrap();
        let stone = BlockStateId(1).try_into_api().unwrap();
        let prepared = prepare(&registry).unwrap();
        let reload = prepare(&registry).unwrap();
        std::thread::spawn(move || {
            assert_eq!(
                BlockStateId(1).try_into_api(),
                Err(BlockStateError::Unavailable)
            );
            install(prepared);
            assert_eq!(BlockStateId(1).try_into_api(), Ok(stone));
            assert_eq!(stone.try_into_engine(&()), Ok(BlockStateId(1)));
            install(reload);
            assert_eq!(BlockStateId(1).try_into_api(), Ok(stone));
        })
        .join()
        .unwrap();
    }
    #[test]
    fn numeric_block_index_reorders_refuses_and_clears_failed_rebind() {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let facts = r#"{"defaults":{"minecraft:air":0,"minecraft:stairs":2},"states":[["a",true,false,"","minecraft:air"],["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"north"}}],["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"east"}}]]}"#;
        let first = BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
        super::super::resolve_block_definitions(&first).unwrap();
        let north = BlockStateId(1).try_into_api().unwrap();
        let east = BlockStateId(2).try_into_api().unwrap();
        assert_eq!((north.ordinal(), east.ordinal()), (0, 1));
        assert_eq!(east.try_into_engine(&()), Ok(BlockStateId(2)));
        assert_eq!(
            BlockStateId(u32::MAX).try_into_api(),
            Err(BlockStateError::UnknownState)
        );
        let bad = mod_api::__private::block_state_from_ordinal(east.block(), u16::MAX);
        assert_eq!(bad.try_into_engine(&()), Err(BlockStateError::UnknownState));
        std::thread::spawn(|| {
            assert_eq!(
                BlockStateId(0).try_into_api(),
                Err(BlockStateError::Unavailable)
            )
        })
        .join()
        .unwrap();
        let reordered = facts
            .replace("north", "TEMP")
            .replace("east", "north")
            .replace("TEMP", "east");
        let reordered = BlockRegistry::from_facts_json(reordered.as_bytes()).unwrap();
        super::super::resolve_block_definitions(&reordered).unwrap();
        assert_eq!(BlockStateId(1).try_into_api(), Ok(east));
        assert_eq!(north.try_into_engine(&()), Ok(BlockStateId(2)));
        assert_eq!(snapshot().unwrap()[1], east);
        let broken = facts.replace("east", "south");
        let broken = BlockRegistry::from_facts_json(broken.as_bytes()).unwrap();
        assert!(super::super::resolve_block_definitions(&broken).is_err());
        assert_eq!(east.try_into_engine(&()), Err(BlockStateError::Unavailable));
        assert_eq!(
            BlockStateId(1).try_into_api(),
            Err(BlockStateError::Unavailable)
        );
        assert!(snapshot().is_err());
    }
}
/// Capture this registry's immutable index explicitly; never resolves on a worker.
pub fn current() -> Result<Arc<Index>, BlockStateError> {
    INDEX.with_borrow(|index| index.clone().ok_or(BlockStateError::Unavailable))
}
impl Index {
    /// Each state's block key, by state number.
    #[cfg(test)]
    pub(in crate::game) fn blocks(&self) -> Arc<[mod_api::BlockKey]> {
        self.blocks.clone()
    }
    /// Route on workers using the feature-pinned published Parts table, without conversion.
    pub(crate) fn worldgen_listens(&self, hook: mod_api::HookId, state: BlockStateId) -> bool {
        self.forward.get(state.0 as usize).is_some_and(|value| {
            mod_api::__private::worldgen_block_key_listens(hook, value.block())
        })
    }

    pub fn to_api(&self, state: BlockStateId) -> Result<BlockState, BlockStateError> {
        self.forward
            .get(state.0 as usize)
            .copied()
            .ok_or(BlockStateError::UnknownState)
    }
    pub fn to_engine(&self, state: BlockState) -> Result<BlockStateId, BlockStateError> {
        let states = self
            .reverse
            .get(&state.block())
            .filter(|states| !states.is_empty())
            .ok_or(BlockStateError::UnknownBlock)?;
        states
            .get(state.ordinal() as usize)
            .copied()
            .flatten()
            .ok_or(BlockStateError::UnknownState)
    }
}
impl TryIntoApi for BlockStateId {
    type Api = BlockState;
    type Error = BlockStateError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        INDEX.with_borrow(|index| {
            index
                .as_ref()
                .ok_or(BlockStateError::Unavailable)?
                .to_api(*self)
        })
    }
}
impl<C: ?Sized> TryIntoEngine<C> for BlockState {
    type Engine = BlockStateId;
    type Error = BlockStateError;
    fn try_into_engine(&self, _: &C) -> Result<Self::Engine, Self::Error> {
        INDEX.with_borrow(|index| {
            index
                .as_ref()
                .ok_or(BlockStateError::Unavailable)?
                .to_engine(*self)
        })
    }
}
