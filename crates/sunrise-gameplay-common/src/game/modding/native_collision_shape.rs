//! The `collision_shape` Part method (vanilla's `getCollisionShape` with its `CollisionContext`).
//! The client library asks through `native_client_hooks` from inside a move; only a state whose
//! block has a subscribing part is converted, and a part that faults loses only its answer.
use super::api_values::{TryIntoApi, block_state};
use std::{cell::RefCell, rc::Rc, sync::Arc};
use sunrise_core::{geometry::VoxelShape, position::BlockPos};
use sunrise_world::{
    block_collision::BlockCollisionRegistry, native_load_context::LoopContext,
    part_collision::InParts, registry::BlockStateId,
};

thread_local! {
    /// The parts of the level being ticked, kept between ticks so their answers' storage and the
    /// names they share are not built again each tick.
    static PARTS: RefCell<Option<Rc<sunrise_sim::collision_parts::ModParts>>> =
        const { RefCell::new(None) };
}

/// Dispatches through the shared Part dispatcher, which restores only the failed
/// invocation's output on unwind. Native callers receive the already-built shape.
pub fn collision_shape(
    context: &LoopContext,
    pos: BlockPos,
    state: BlockStateId,
    query: &mod_api::CollisionQuery,
    shape: &mut Option<VoxelShape>,
) {
    if !block_state::block_listens(mod_api::HookId::CollisionShape, state) {
        return;
    }
    let Ok(api_state) = state.try_into_api() else {
        return;
    };
    let mut answer = None;
    mod_api::dispatch_collision_shape(
        &sunrise_level::api_context::LoopReader(context).context(),
        pos,
        api_state,
        query,
        &mut answer,
    );
    *shape = answer.map(mod_api::__private::voxel_shape_into_core);
}

/// Marks the states of `collision` whose parts answer `collision_shape`, and only them.
fn mark_part_states(collision: &BlockCollisionRegistry<'_>) {
    collision.set_part_states(
        (0..collision.registry().len())
            .map(|id| BlockStateId(id as u32))
            .filter(|state| block_state::block_listens(mod_api::HookId::CollisionShape, *state)),
    );
}

/// Enables the level's collision parts for this thread and dimension only when a part subscribes.
fn enter_parts(context: &Arc<LoopContext>, dimension: &str) -> Option<InParts> {
    mod_api::__private::has_scoped_hook(mod_api::HookId::CollisionShape).then(|| {
        let parts = PARTS.with_borrow_mut(|slot| {
            let parts = slot.get_or_insert_with(|| {
                Rc::new(sunrise_sim::collision_parts::ModParts::new(Arc::clone(
                    context,
                )))
            });
            parts.begin_tick(context, dimension);
            Rc::clone(parts)
        });
        InParts::enter(parts)
    })
}

impl crate::game::CommonGame<'_> {
    /// Marks the states whose parts answer `collision_shape` in the collision tables, so scans
    /// keep them in the cells around their own. Between ticks, after the parts are resolved.
    pub(in crate::game) fn mark_collision_shape_states(&self) {
        mark_part_states(self.collision);
    }

    /// The parts that answer this tick's collision queries, until the guard drops: none when no
    /// mod has a part that answers them, so the tick pays nothing.
    pub fn collision_parts(&self) -> Option<InParts> {
        enter_parts(&self.loop_context, self.dim.id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use sunrise_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs, LoopSide};

    #[derive(Clone, Copy)]
    enum Mode {
        Answer,
        Panic,
        Refuse,
        Observe,
    }
    /// One Part type per mod: a block takes a Part type once (`insert_part`).
    struct Part<const MOD: u8>(Mode, Arc<AtomicUsize>);
    #[mod_api::sunrise_part]
    impl<const MOD: u8> mod_api::BlockPart for Part<MOD> {
        fn collision_shape(
            &self,
            _: &dyn mod_api::LoopContext,
            event: &mut mod_api::CollisionShapeEvent<'_>,
        ) {
            self.1.fetch_add(1, Ordering::SeqCst);
            let half = [[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]];
            match self.0 {
                Mode::Answer => {
                    *event.shape = Some(mod_api::VoxelShape::try_from_boxes(&half).unwrap())
                }
                Mode::Panic => {
                    *event.shape = Some(mod_api::VoxelShape::empty());
                    panic!("a part that faults");
                }
                Mode::Refuse => {
                    if let Ok(shape) = mod_api::VoxelShape::try_from_boxes(&[[f64::NAN; 6]]) {
                        *event.shape = Some(shape);
                    }
                }
                Mode::Observe => {
                    assert!(event.shape.as_ref().unwrap().boxes().eq(half));
                    *event.shape = Some(mod_api::VoxelShape::empty());
                }
            }
        }
    }

    #[test]
    fn a_failed_shape_answer_does_not_erase_other_parts_on_the_same_block() {
        let calls: Vec<_> = (0..4).map(|_| Arc::new(AtomicUsize::new(0))).collect();
        struct Listener(Mode, Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Listener {
            fn register(
                &self,
                registrar: &mut dyn mod_api::Registrar,
                _: &mod_api::RegistrationContext,
            ) {
                let key = registrar.block_key("minecraft:stone");
                let calls = self.1.clone();
                match self.0 {
                    Mode::Answer => registrar.block_part(key, Part::<0>(self.0, calls)),
                    Mode::Panic => registrar.block_part(key, Part::<1>(self.0, calls)),
                    Mode::Refuse => registrar.block_part(key, Part::<2>(self.0, calls)),
                    Mode::Observe => registrar.block_part(key, Part::<3>(self.0, calls)),
                }
            }
        }
        let mods: mod_api::Mods = [
            ("shape_a", Mode::Answer),
            ("shape_b", Mode::Panic),
            ("shape_c", Mode::Refuse),
            ("shape_d", Mode::Observe),
        ]
        .into_iter()
        .zip(&calls)
        .map(|((id, mode), calls)| {
            mod_api::ModEntry::new(id, Box::new(Listener(mode, calls.clone())))
        })
        .collect::<Vec<_>>()
        .into();
        mod_platform::testing::with_mods(mods.clone(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
            let facts = r#"{"defaults":{"minecraft:air":3,"minecraft:stone":0,"minecraft:dirt":1,"minecraft:gravel":2},"states":[["s",false,false,"","minecraft:stone"],["d",false,false,"","minecraft:dirt"],["g",false,false,"","minecraft:gravel"],["a",true,false,"","minecraft:air"]]}"#;
            let registry =
                sunrise_world::registry::BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
            crate::game::api_values::resolve_block_definitions(&registry).unwrap();
            mod_api::__private::resolve_parts(
                &Default::default(),
                &Default::default(),
                mod_api::__private::KnownIds {
                    block: &|_| true,
                    item: &|_| false,
                },
            )
            .unwrap();
            let inputs = Arc::new(LoadInputs::new(Default::default(), std::env::temp_dir()));
            let context = LoadContext::new(DataLoadOrigin::ServerDataLoad, inputs, [])
                .loop_context(LoopSide::Server, [] as [&str; 0]);
            let ask = |state: u32| {
                let mut shape = None;
                collision_shape(
                    &context,
                    BlockPos::new(1, 64, 1),
                    BlockStateId(state),
                    &mod_api::CollisionQuery::default(),
                    &mut shape,
                );
                shape
            };
            for _ in 0..2 {
                assert!(ask(0).unwrap().is_empty());
            }
            for count in calls {
                assert_eq!(count.load(Ordering::SeqCst), 2);
            }
        });
    }

    /// `minecraft:air` (state 0) and `minecraft:stone` (state 1, a full cube) as the registry and
    /// the collision tables have them.
    fn stone_tables() -> (
        sunrise_world::registry::BlockRegistry,
        sunrise_core::nbt::Tag,
    ) {
        use sunrise_core::{
            geometry::{Axis, VoxelShape},
            nbt::Tag,
        };
        let names = || {
            ["minecraft:air", "minecraft:stone"]
                .map(|name| Tag::String(name.into()))
                .to_vec()
        };
        let registry = sunrise_world::registry::BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(sunrise_world::DATA_VERSION)),
            ("states", Tag::List(names())),
            (
                "defaults",
                Tag::compound([
                    ("minecraft:air", Tag::Int(0)),
                    ("minecraft:stone", Tag::Int(1)),
                ]),
            ),
        ]))
        .unwrap();
        let empty = VoxelShape::empty();
        let (min, max) = empty.discrete_bounds();
        let axes = empty.cube_coordinate_axes();
        let bits = |axis| {
            Tag::LongArray(
                empty
                    .coordinates(axis)
                    .iter()
                    .map(|n| n.to_bits() as i64)
                    .collect(),
            )
        };
        let air = Tag::compound([
            ("canonical", Tag::Byte(0)),
            ("cube", Tag::Byte(i8::from(empty.is_cube()))),
            (
                "cube_axes",
                Tag::Byte((axes[0] as i8) | ((axes[1] as i8) << 1) | ((axes[2] as i8) << 2)),
            ),
            ("x", bits(Axis::X)),
            ("y", bits(Axis::Y)),
            ("z", bits(Axis::Z)),
            ("occupied", Tag::ByteArray(vec![])),
            ("discrete_min", Tag::IntArray(min.to_vec())),
            ("discrete_max", Tag::IntArray(max.to_vec())),
        ]);
        let one = Tag::LongArray(vec![0, 1.0f64.to_bits() as i64]);
        let cube = Tag::compound([
            ("canonical", Tag::Byte(0)),
            ("cube", Tag::Byte(1)),
            ("cube_axes", Tag::Byte(7)),
            ("x", one.clone()),
            ("y", one.clone()),
            ("z", one),
            ("occupied", Tag::ByteArray(vec![1])),
            ("discrete_min", Tag::IntArray(vec![0; 3])),
            ("discrete_max", Tag::IntArray(vec![1; 3])),
        ]);
        let table = Tag::compound([
            ("DataVersion", Tag::Int(sunrise_world::DATA_VERSION)),
            ("states", Tag::List(names())),
            ("shapes", Tag::List(vec![cube, air])),
            ("cached_shapes", Tag::IntArray(vec![1, 0])),
        ]);
        (registry, table)
    }

    /// A block part alone opens the collision gate for player 7 in the overworld.
    #[test]
    fn a_registered_part_gives_a_moving_entity_its_shape() {
        use sunrise_core::geometry::{Aabb, Vec3};
        use sunrise_level::{chunk::Chunk, collisions::SceneCollisions, world::World};
        use sunrise_world::{
            collision_context::{CollisionContext, CollisionEntity, EntityContextKind},
            entity_obstruction::EntityIdentity,
        };
        static ASKED: AtomicUsize = AtomicUsize::new(0);
        struct Barrier;
        #[mod_api::sunrise_part]
        impl mod_api::BlockPart for Barrier {
            fn collision_shape(
                &self,
                _: &dyn mod_api::LoopContext,
                event: &mut mod_api::CollisionShapeEvent<'_>,
            ) {
                ASKED.fetch_add(1, Ordering::SeqCst);
                if event.query.kind() == mod_api::CollisionEntityKind::Player
                    && event.query.entity() == Some(7)
                    && event.query.dimension() == Some("minecraft:overworld")
                {
                    *event.shape = Some(mod_api::VoxelShape::empty());
                }
            }
        }
        #[derive(Default)]
        struct Walker;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Walker {
            fn register(
                &self,
                registrar: &mut dyn mod_api::Registrar,
                _: &mod_api::RegistrationContext,
            ) {
                let stone = registrar.block_key("minecraft:stone");
                registrar.block_part(stone, Barrier);
            }
        }
        let mods: mod_api::Mods =
            vec![mod_api::ModEntry::new("collision_walk", Box::new(Walker))].into();
        crate::game::native_hooks::install_client_hooks();
        mod_platform::testing::with_mods(mods.clone(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
            let (registry, table) = stone_tables();
            crate::game::api_values::resolve_block_definitions(&registry).unwrap();
            mod_api::__private::resolve_parts(
                &Default::default(),
                &Default::default(),
                mod_api::__private::KnownIds {
                    block: &|_| true,
                    item: &|_| false,
                },
            )
            .unwrap();
            let cached =
                sunrise_world::collision::CachedCollisionRegistry::from_nbt(&table, &registry)
                    .unwrap();
            let binding = BlockCollisionRegistry::new(Arc::new(cached)).unwrap();
            mark_part_states(&binding);
            let inputs = Arc::new(LoadInputs::new(Default::default(), std::env::temp_dir()));
            let context = Arc::new(
                LoadContext::new(DataLoadOrigin::ServerDataLoad, inputs, [])
                    .loop_context(LoopSide::Server, [] as [&str; 0]),
            );
            let mut world = World::new(BlockStateId(0), 0);
            world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            world.set(1, 64, 1, BlockStateId(1)).unwrap();
            // A player standing west of the stone walks a block east into it.
            let walk = |id: u64| {
                let body = Aabb::new(Vec3::new(0.3, 64.0, 1.2), Vec3::new(0.9, 65.8, 1.8));
                let delta = Vec3::new(1.0, 0.0, 0.0);
                let mut scene = SceneCollisions {
                    world: &world,
                    binding: &binding,
                    subject: None,
                };
                let mut gate = sunrise_sim::move_gate::armed(
                    &context,
                    id as u32,
                    "minecraft:player",
                    sunrise_world::entity_move::MoverType::Player,
                    delta,
                );
                let player = CollisionContext::for_entity(
                    CollisionEntity {
                        identity: EntityIdentity(id),
                        kind: EntityContextKind::Player,
                        y: 64.0,
                        descending: false,
                        main_hand: Some(sunrise_world::collision_context::CollisionItem::EMPTY),
                    },
                    false,
                );
                sunrise_sim::move_gate::scene_resolve_with_context(
                    &mut gate, &mut scene, body, delta, 0.6, true, player,
                )
                .unwrap()
                .x
            };
            ASKED.store(0, Ordering::SeqCst);
            // The level tick's parts, as the client and the server enter them.
            let parts = enter_parts(&context, "minecraft:overworld");
            assert!(
                parts.is_some(),
                "a part that answers collision_shape opens the tick's parts"
            );
            // The body's east face is at x = 0.9 and the stone's west face at x = 1.0, so a
            // stopped player moves 1.0 - 0.9 in f64, which is 0.09999999999999998, not 0.1.
            const TO_THE_FACE: f64 = 0.09999999999999998;
            assert_eq!(walk(7), 1.0, "the barrier is open for player 7");
            assert_eq!(walk(8), TO_THE_FACE, "player 8 stops at the stone's face");
            assert!(ASKED.load(Ordering::SeqCst) > 0);
            drop(parts);
            // The same position in another level is another block to the part.
            let parts = enter_parts(&context, "minecraft:the_nether");
            assert_eq!(walk(7), TO_THE_FACE, "the barrier is the overworld's");
            // Outside the tick's parts the stone is its own cube for everyone, and the part is
            // not asked.
            drop(parts);
            ASKED.store(0, Ordering::SeqCst);
            assert_eq!(walk(7), TO_THE_FACE);
            assert_eq!(ASKED.load(Ordering::SeqCst), 0);
        });
    }
}
