//! The engine side of `alter_ground`: the tree decorator's callback, and a mod's ground
//! provider as the decorator asks it. `mod_api` keeps only the API's provider traits.
use super::{
    api_values::block_state::Index, native_worldgen::WorkerResources, native_worldgen_parts,
};
use mod_api::{BlockState, GroundLevel, GroundProvider, GroundStateProvider, HookId, RandomSource};
use std::sync::Arc;
use sunrise_world::native_worldgen_hooks::{
    AlterGroundTree, GroundAccess, InvalidGroundState, ModGroundProvider,
};
use sunrise_world::registry::BlockStateId as EngineState;

/// The engine's `alter_ground` callback: `replaced` is set only when a mod put
/// its provider in place of the decorator's own. Nothing is built or read while
/// no mod listens.
pub fn alter_ground(tree: &AlterGroundTree<'_>, replaced: &mut Option<Arc<dyn ModGroundProvider>>) {
    if !mod_api::__private::listened(HookId::AlterGround) {
        return;
    }
    let _ = native_worldgen_parts::with_host(|host| {
        let mut provider = GroundProvider::Decorator;
        mod_api::world::ground::native_alter_ground(
            host,
            tree.logs,
            tree.leaves,
            tree.roots,
            tree.positions,
            &mut provider,
        );
        if let GroundProvider::Mod(provider) = provider
            && let Some(registry) = tree.registry
            && let Ok(worker) = WorkerResources::get(registry)
        {
            *replaced = Some(Arc::new(Engine {
                provider,
                index: worker.index.clone(),
                air: worker.air,
            }));
        }
    });
}

/// A mod's provider pinned to the placing world's index. Workers need no thread-local maps.
struct Engine {
    provider: Arc<dyn GroundStateProvider>,
    index: Arc<Index>,
    air: BlockState,
}
impl ModGroundProvider for Engine {
    fn state_at(
        &self,
        access: &mut dyn GroundAccess,
        pos: [i32; 3],
    ) -> Result<Option<EngineState>, InvalidGroundState> {
        let mut level = Level {
            access,
            index: &self.index,
            air: self.air,
            invalid_state: false,
        };
        let chosen = self.provider.state_at(&mut level, pos);
        if level.invalid_state {
            return Err(InvalidGroundState);
        }
        chosen
            .map(|state| self.index.to_engine(state).map_err(|_| InvalidGroundState))
            .transpose()
    }
}

/// The decorator's run, as a provider reads it. Failed crossings are refused after it answers.
struct Level<'a> {
    access: &'a mut dyn GroundAccess,
    index: &'a Index,
    air: BlockState,
    invalid_state: bool,
}
impl Level<'_> {
    fn convert_read(&mut self, state: EngineState) -> Option<BlockState> {
        self.index
            .to_api(state)
            .inspect_err(|_| self.invalid_state = true)
            .ok()
    }
}
impl GroundLevel for Level<'_> {
    fn block_state(&mut self, pos: [i32; 3]) -> BlockState {
        let state = self.access.block_state(pos);
        self.convert_read(state).unwrap_or(self.air)
    }
    fn decorator_state(&mut self, pos: [i32; 3]) -> Option<BlockState> {
        let state = self.access.decorator_state(pos)?;
        self.convert_read(state)
    }
    fn random(&mut self) -> &mut dyn RandomSource {
        self
    }
}
impl RandomSource for Level<'_> {
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        self.access.next_int_bound(bound)
    }
    fn next_long(&mut self) -> i64 {
        self.access.next_long()
    }
    fn next_float(&mut self) -> f32 {
        self.access.next_float()
    }
    fn next_double(&mut self) -> f64 {
        self.access.next_double()
    }
    fn next_boolean(&mut self) -> bool {
        self.access.next_boolean()
    }
    fn next_int(&mut self) -> i32 {
        self.access.next_int()
    }
    fn next_gaussian(&mut self) -> f64 {
        self.access.next_gaussian()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::api_values::{block_state, resolve_block_definitions};
    use mod_api::{AlterGroundEvent, Hooks, ModEntry, StateConstraint, WorldgenHost};
    use mod_platform::testing;
    use std::collections::BTreeMap;
    use std::sync::{Mutex, PoisonError};
    use sunrise_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs};
    use sunrise_world::native_worldgen_hooks::WorldgenRandom;
    use sunrise_world::registry::BlockRegistry;

    const AIR: EngineState = EngineState(0);
    const GRASS: EngineState = EngineState(1);
    const SNOWY_PODZOL: EngineState = EngineState(2);
    const MOSS: EngineState = EngineState(3);
    const MUD: EngineState = EngineState(4);
    const FACTS: &str = r#"{
        "defaults":{"minecraft:air":0,"minecraft:grass_block":1,"minecraft:podzol":5,"minecraft:moss_block":3,"minecraft:mud":4},
        "states":[["a",true,false,"","minecraft:air"], ["s",false,false,"","minecraft:grass_block"],
            ["s",false,false,"",{"id":"minecraft:podzol","properties":{"snowy":"true"}}],
            ["s",false,false,"","minecraft:moss_block"], ["s",false,false,"","minecraft:mud"],
            ["s",false,false,"",{"id":"minecraft:podzol","properties":{"snowy":"false"}}]]
    }"#;

    fn registry() -> BlockRegistry {
        let registry = BlockRegistry::from_facts_json(FACTS.as_bytes()).unwrap();
        resolve_block_definitions(&registry).unwrap();
        registry
    }

    /// An expected mod-facing state, resolved by name and properties, not by engine ID.
    fn canonical(id: &str, properties: &[StateConstraint]) -> BlockState {
        let metadata = mod_api::__private::block_definitions().unwrap();
        let key = mod_api::__private::block_key(id).unwrap();
        BlockState::of(metadata, key, properties).unwrap()
    }

    /// The server's context, published as a server's resource load does.
    fn publish() {
        let registries =
            BTreeMap::from([("block".to_owned(), vec!["minecraft:moss_block".to_owned()])]);
        let root = std::env::temp_dir().join("sunrise-alter-ground-no-settings");
        let load = LoadContext::new(
            DataLoadOrigin::ServerDataLoad,
            Arc::new(LoadInputs::new(registries, root)),
            [],
        );
        sunrise_world::native_worldgen_hooks::publish_context(Arc::new(load.worldgen_context([])));
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Draw {
        Decorator,
        Mod,
    }

    /// A tree on grass whose decorator turns grass to podzol, recording the order of its draws.
    #[derive(Default)]
    struct Meadow {
        draws: Vec<Draw>,
    }
    impl GroundAccess for Meadow {
        fn block_state(&mut self, [_, y, _]: [i32; 3]) -> EngineState {
            if y == 63 { GRASS } else { AIR }
        }
        fn decorator_state(&mut self, pos: [i32; 3]) -> Option<EngineState> {
            self.draws.push(Draw::Decorator);
            (self.block_state(pos) == GRASS).then_some(SNOWY_PODZOL)
        }
    }
    impl WorldgenRandom for Meadow {
        fn next_int_bound(&mut self, _: i32) -> i32 {
            self.draws.push(Draw::Mod);
            0
        }
        fn next_long(&mut self) -> i64 {
            self.draws.push(Draw::Mod);
            0
        }
        fn next_float(&mut self) -> f32 {
            self.draws.push(Draw::Mod);
            0.0
        }
        fn next_double(&mut self) -> f64 {
            self.draws.push(Draw::Mod);
            0.0
        }
        fn next_boolean(&mut self) -> bool {
            self.draws.push(Draw::Mod);
            false
        }
        fn next_int(&mut self) -> i32 {
            self.draws.push(Draw::Mod);
            0
        }
        fn next_gaussian(&mut self) -> f64 {
            self.draws.push(Draw::Mod);
            0.0
        }
    }

    const LOGS: [[i32; 3]; 2] = [[0, 64, 0], [0, 65, 0]];
    fn alter() -> Option<Arc<dyn ModGroundProvider>> {
        let _publishing = native_worldgen_parts::PUBLISHING
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let registry = registry();
        publish();
        let mut replaced = None;
        alter_ground(
            &AlterGroundTree {
                logs: &LOGS,
                leaves: &[],
                roots: &[],
                positions: &LOGS[..1],
                registry: Some(&registry),
            },
            &mut replaced,
        );
        replaced
    }

    /// Moss where the provider it found places snowy podzol on grass.
    struct Moss {
        wrapped: GroundProvider,
        grass: BlockState,
        snowy_podzol: BlockState,
        moss: BlockState,
    }
    impl Moss {
        fn new(wrapped: GroundProvider) -> Self {
            Self {
                wrapped,
                grass: canonical("minecraft:grass_block", &[]),
                snowy_podzol: canonical(
                    "minecraft:podzol",
                    &[StateConstraint::new("snowy", "true")],
                ),
                moss: canonical("minecraft:moss_block", &[]),
            }
        }
    }
    impl GroundStateProvider for Moss {
        fn state_at(&self, level: &mut dyn GroundLevel, pos: [i32; 3]) -> Option<BlockState> {
            let ground = level.block_state(pos);
            let found = self.wrapped.state_at(level, pos);
            if ground == self.grass && found == Some(self.snowy_podzol) {
                Some(self.moss)
            } else {
                found
            }
        }
    }
    /// Mud where the provider it found places moss, after one draw of its own.
    struct Mud {
        wrapped: GroundProvider,
        moss: BlockState,
        mud: BlockState,
    }
    impl GroundStateProvider for Mud {
        fn state_at(&self, level: &mut dyn GroundLevel, pos: [i32; 3]) -> Option<BlockState> {
            level.random().next_int_bound(2);
            let found = self.wrapped.state_at(level, pos);
            if found == Some(self.moss) {
                Some(self.mud)
            } else {
                found
            }
        }
    }

    /// Swaps in its provider, recording what the event and the context told it.
    struct Swaps {
        wrap: fn(GroundProvider) -> Arc<dyn GroundStateProvider>,
        heard: Arc<Mutex<Vec<String>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Swaps {
        fn alter_ground(&self, context: &dyn WorldgenHost, event: &mut AlterGroundEvent<'_>) {
            let blocks = context.registry("block").map(|names| names.join(","));
            self.heard.lock().unwrap().push(format!(
                "{:?} {:?} {}",
                event.positions,
                blocks,
                matches!(event.provider, GroundProvider::Decorator)
            ));
            *event.provider = GroundProvider::Mod((self.wrap)(event.provider.clone()));
        }
    }
    /// Listens and leaves the decorator's own provider.
    struct Keeps;
    #[mod_api::__subscriptions]
    impl Hooks for Keeps {
        fn alter_ground(&self, _: &dyn WorldgenHost, event: &mut AlterGroundEvent<'_>) {
            assert!(matches!(event.provider, GroundProvider::Decorator));
        }
    }

    #[test]
    fn with_no_listener_or_the_decorators_own_kept_nothing_is_replaced() {
        testing::with_mods(Vec::<ModEntry>::new().into(), || assert!(alter().is_none()));
        testing::with_mods(vec![ModEntry::new("keeps", Box::new(Keeps))].into(), || {
            assert!(alter().is_none());
        });
    }

    #[test]
    fn a_mod_swaps_the_provider_and_sees_the_tree_and_the_servers_registries() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new(
            "moss",
            Box::new(Swaps {
                wrap: |found| Arc::new(Moss::new(found)),
                heard: heard.clone(),
            }),
        )];
        let replaced = testing::with_mods(mods.into(), alter).expect("the mod's provider");
        assert_eq!(
            *heard.lock().unwrap(),
            [r#"[[0, 64, 0]] Some("minecraft:moss_block") true"#]
        );
        let mut meadow = Meadow::default();
        assert_eq!(replaced.state_at(&mut meadow, [3, 63, 0]), Ok(Some(MOSS)));
        // The wrapped decorator's own provider drew once, as it does alone.
        assert_eq!(meadow.draws, [Draw::Decorator]);
        assert_eq!(replaced.state_at(&mut meadow, [3, 64, 0]), Ok(None));
        assert_eq!(meadow.draws, [Draw::Decorator, Draw::Decorator]);
    }

    #[test]
    fn each_mod_wraps_the_provider_the_one_before_left() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![
            ModEntry::new(
                "moss",
                Box::new(Swaps {
                    wrap: |found| Arc::new(Moss::new(found)),
                    heard: heard.clone(),
                }),
            ),
            ModEntry::new(
                "mud",
                Box::new(Swaps {
                    wrap: |wrapped| {
                        Arc::new(Mud {
                            wrapped,
                            moss: canonical("minecraft:moss_block", &[]),
                            mud: canonical("minecraft:mud", &[]),
                        })
                    },
                    heard: heard.clone(),
                }),
            ),
        ];
        let replaced = testing::with_mods(mods.into(), alter).expect("the mods' provider");
        let decorator = r#"[[0, 64, 0]] Some("minecraft:moss_block")"#;
        assert_eq!(
            *heard.lock().unwrap(),
            [format!("{decorator} true"), format!("{decorator} false")]
        );
        // A worker has no owning-thread state index.
        std::thread::spawn(move || {
            let mut meadow = Meadow::default();
            assert_eq!(replaced.state_at(&mut meadow, [3, 63, 0]), Ok(Some(MUD)));
            // Mud's own draw, then the decorator's, in the order the providers asked.
            assert_eq!(meadow.draws, [Draw::Mod, Draw::Decorator]);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn a_canonical_state_missing_from_the_placing_registry_is_refused() {
        struct ChoosesMud(BlockState);
        impl GroundStateProvider for ChoosesMud {
            fn state_at(&self, _: &mut dyn GroundLevel, _: [i32; 3]) -> Option<BlockState> {
                Some(self.0)
            }
        }
        testing::with_mods(Vec::<ModEntry>::new().into(), || {
            let _full = registry();
            let air_only = BlockRegistry::from_facts_json(
                br#"{"defaults":{"minecraft:air":0},"states":[["a",true,false,"","minecraft:air"]]}"#
                    .as_slice(),
            )
            .unwrap();
            let provider = Engine {
                provider: Arc::new(ChoosesMud(canonical("minecraft:mud", &[]))),
                index: block_state::prepare(&air_only).unwrap(),
                air: canonical("minecraft:air", &[]),
            };
            assert_eq!(
                provider.state_at(&mut Meadow::default(), [3, 63, 0]),
                Err(InvalidGroundState)
            );
        });
    }
}
