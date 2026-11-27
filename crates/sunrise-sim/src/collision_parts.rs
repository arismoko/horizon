//! A block's collision shape as its parts answer for the querying entity
//! (`BlockPart::collision_shape`, vanilla's `getCollisionShape` with its `CollisionContext`).
//!
//! The game installs [`ModParts`] around a level's tick ([`sunrise_world::part_collision`]); a
//! collision query that meets a state whose parts opt in asks it, every time, as vanilla asks
//! `getCollisionShape` for every block of every sweep. It runs on the thread that ticks the level and nowhere else, so a worker
//! reads the normal shape. The part holds no `Host`: it answers from the query.
use mod_api::{CollisionEntityKind, CollisionQuery};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    sync::Arc,
};
use sunrise_core::{geometry::VoxelShape, position::BlockPos};
use sunrise_world::{
    collision_context::{CollisionContext, EntityContextKind},
    native_load_context::LoopContext,
    part_collision::PartShapes,
    registry::BlockStateId,
};

/// The collision shape of a block state is being read for the query `query`
/// (`BlockBehaviour.getCollisionShape`). `shape` starts as `None`, which keeps the shape the block
/// would otherwise have, and the block's parts may lend a replacement shape for this query.
/// Fires on the side that ticks the entity, for every query that meets an opted-in state: a
/// step-up retry or a second sweep asks again, and a change to the mod's own state is seen by the
/// next query.
#[inline(never)]
pub fn native_collision_shape(
    context: &LoopContext,
    pos: BlockPos,
    state: BlockStateId,
    query: &CollisionQuery,
    shape: &mut Option<VoxelShape>,
) {
    crate::native_client_hooks::collision_shape(context, pos, state, query, shape);
}

/// The entity a query is for: the context's own, else the one whose move the move gate is
/// resolving; 0 for none.
fn entity_of(context: &CollisionContext<'_>) -> u32 {
    let own = context.entity().map_or(0, |entity| entity.0);
    // Entity ids are 32 bits on the wire.
    (if own == 0 {
        sunrise_world::moving_colliders::mover()
    } else {
        own
    }) as u32
}

/// The parts of one level's tick: answers collision queries from the mods. One value serves every
/// tick: [`Self::begin_tick`] points it at the tick's level and keeps the shared names.
pub struct ModParts {
    context: RefCell<Arc<LoopContext>>,
    /// The dimension id of the level being ticked, which every query names; none before the
    /// first tick.
    dimension: RefCell<Option<Arc<str>>>,
    /// Each level's dimension id, shared between its ticks: a server ticks its levels in turn on
    /// one thread, so switching levels looks one up rather than building it again.
    dimensions: RefCell<HashMap<String, Arc<str>>>,
    /// Whether a part is running: a query made inside it gets the normal shape, never a nested
    /// call.
    busy: Cell<bool>,
    /// The item ids queries have named, shared between them.
    items: RefCell<HashMap<String, Arc<str>>>,
    /// The fluid lists queries have named, shared between them.
    fluids: RefCell<Vec<(&'static [&'static str], Arc<[Arc<str>]>)>>,
}

impl ModParts {
    pub fn new(context: Arc<LoopContext>) -> Self {
        Self {
            context: RefCell::new(context),
            dimension: RefCell::new(None),
            dimensions: RefCell::default(),
            busy: Cell::new(false),
            items: RefCell::default(),
            fluids: RefCell::default(),
        }
    }

    /// Starts a tick of the level `context` belongs to, whose dimension id is `dimension`.
    pub fn begin_tick(&self, context: &Arc<LoopContext>, dimension: &str) {
        if !Arc::ptr_eq(&self.context.borrow(), context) {
            *self.context.borrow_mut() = Arc::clone(context);
        }
        let mut current = self.dimension.borrow_mut();
        if current.as_deref() != Some(dimension) {
            let mut dimensions = self.dimensions.borrow_mut();
            let shared = match dimensions.get(dimension) {
                Some(shared) => Arc::clone(shared),
                None => Arc::clone(
                    dimensions
                        .entry(dimension.to_owned())
                        .or_insert_with(|| dimension.into()),
                ),
            };
            *current = Some(shared);
        }
    }

    /// The shared name of an item id.
    fn shared_item(&self, item: &str) -> Arc<str> {
        let mut items = self.items.borrow_mut();
        match items.get(item) {
            Some(shared) => Arc::clone(shared),
            None => Arc::clone(items.entry(item.to_owned()).or_insert_with(|| item.into())),
        }
    }

    fn query(
        &self,
        context: &CollisionContext<'_>,
        entity: u32,
        held_item: Option<Arc<str>>,
        standable: Option<&'static [&'static str]>,
    ) -> CollisionQuery {
        let kind = match context.entity_kind() {
            EntityContextKind::Item => CollisionEntityKind::Item,
            EntityContextKind::Player => CollisionEntityKind::Player,
            EntityContextKind::Living => CollisionEntityKind::Living,
            EntityContextKind::Other | EntityContextKind::None => CollisionEntityKind::Other,
        };
        let mut query =
            CollisionQuery::new((entity != 0).then_some(entity), kind, context.bottom())
                .with_descending(context.is_descending())
                .with_placement(context.is_placement())
                .with_always_collide_with_fluid(context.always_collide_with_fluid());
        if let Some(item) = held_item {
            query = query.with_held_item(item);
        }
        if let Some(dimension) = &*self.dimension.borrow() {
            query = query.with_dimension(Arc::clone(dimension));
        }
        if let Some(names) = standable {
            let mut fluids = self.fluids.borrow_mut();
            let shared = match fluids.iter().find(|(known, _)| *known == names) {
                Some((_, shared)) => Arc::clone(shared),
                None => {
                    let shared: Arc<[Arc<str>]> =
                        names.iter().map(|name| Arc::from(*name)).collect();
                    fluids.push((names, Arc::clone(&shared)));
                    shared
                }
            };
            query = query.with_standable_fluids(shared);
        }
        query
    }
}

impl PartShapes for ModParts {
    fn shape(
        &self,
        state: BlockStateId,
        position: BlockPos,
        context: &CollisionContext<'_>,
        standable: &mut dyn FnMut() -> &'static [&'static str],
    ) -> Option<VoxelShape> {
        if self.busy.get() {
            return None;
        }
        let entity = entity_of(context);
        let held_item = context.held_item().map(|item| self.shared_item(item));
        let standable = context.living_entity().is_some().then(standable);
        let query = self.query(context, entity, held_item, standable);
        struct Running<'a>(&'a Cell<bool>);
        impl Drop for Running<'_> {
            fn drop(&mut self) {
                self.0.set(false);
            }
        }
        self.busy.set(true);
        let running = Running(&self.busy);
        let mut answer = None;
        native_collision_shape(&self.context.borrow(), position, state, &query, &mut answer);
        drop(running);
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_client_hooks::test_hooks::{
        COLLISION_ANSWER, COLLISION_ASKS, install, loop_context,
    };
    use std::rc::Rc;
    use sunrise_world::collision_context::CollisionEntity;
    use sunrise_world::entity_obstruction::EntityIdentity;

    fn parts() -> ModParts {
        install();
        COLLISION_ASKS.set(0);
        COLLISION_ANSWER.with_borrow_mut(|answer| *answer = None);
        ModParts::new(Arc::new(loop_context()))
    }
    fn entity(id: u64) -> CollisionContext<'static> {
        CollisionContext::for_entity(
            CollisionEntity {
                identity: EntityIdentity(id),
                kind: EntityContextKind::Player,
                y: 65.0,
                descending: false,
                main_hand: None,
            },
            false,
        )
    }
    fn ask(parts: &ModParts, id: u64, at: [i32; 3]) -> Option<VoxelShape> {
        parts.shape(
            BlockStateId(1),
            BlockPos::new(at[0], at[1], at[2]),
            &entity(id),
            &mut || &[],
        )
    }

    /// #5459: a part solid for an entity until its own flag flips answers the flip at the next
    /// query of the same tick, and a repeated question is asked again (vanilla keeps no memo).
    #[test]
    fn a_part_is_asked_for_every_query_and_sees_its_state_change_within_a_tick() {
        let parts = parts();
        let open = Rc::new(Cell::new(false));
        let flag = Rc::clone(&open);
        COLLISION_ANSWER.with_borrow_mut(|answer| {
            *answer = Some(Box::new(move |query| {
                query.entity().map(|_| {
                    if flag.get() {
                        Vec::new()
                    } else {
                        vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]
                    }
                })
            }));
        });
        let solid = |shape: Option<VoxelShape>| shape.is_some_and(|shape| !shape.is_empty());
        assert!(solid(ask(&parts, 7, [1, 64, 1])));
        open.set(true);
        assert!(
            !solid(ask(&parts, 7, [1, 64, 1])),
            "the same tick sees the flip"
        );
        // A step-up retry and a second sweep ask the same question again.
        ask(&parts, 7, [1, 64, 1]);
        assert_eq!(COLLISION_ASKS.get(), 3);
    }

    #[test]
    fn a_query_made_inside_a_part_gets_the_normal_shape_without_a_nested_call() {
        let parts = Rc::new(parts());
        let inner = Rc::clone(&parts);
        let nested = Rc::new(std::cell::RefCell::new(None));
        let seen = Rc::clone(&nested);
        COLLISION_ANSWER.with_borrow_mut(|answer| {
            *answer = Some(Box::new(move |_| {
                *seen.borrow_mut() = Some(ask(&inner, 9, [5, 64, 5]));
                Some(vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]])
            }));
        });
        assert!(ask(&parts, 7, [1, 64, 1]).is_some());
        assert!(matches!(*nested.borrow(), Some(None)));
        assert_eq!(COLLISION_ASKS.get(), 1, "the nested query asked nothing");
        // Outside the part the same position is asked for itself.
        assert!(ask(&parts, 9, [5, 64, 5]).is_some());
        assert_eq!(COLLISION_ASKS.get(), 2);
    }

    /// #6034: a query names the level it is made in, so a part keeping its own state by position
    /// can tell two dimensions apart; a tick of another level names that one. A level's id is
    /// shared between its ticks, not built again when the server comes back to it.
    #[test]
    fn a_query_names_the_dimension_of_the_level_being_ticked() {
        let parts = parts();
        let seen = Rc::new(std::cell::RefCell::new(Vec::new()));
        let record = Rc::clone(&seen);
        COLLISION_ANSWER.with_borrow_mut(|answer| {
            *answer = Some(Box::new(move |query| {
                // Keep the original dimension storage alive across level switches.
                record.borrow_mut().push(query.clone());
                None
            }));
        });
        ask(&parts, 7, [1, 64, 1]);
        let context = Arc::new(loop_context());
        for dimension in [
            "minecraft:overworld",
            "minecraft:the_nether",
            "minecraft:overworld",
        ] {
            parts.begin_tick(&context, dimension);
            ask(&parts, 7, [1, 64, 1]);
        }
        let seen = seen.borrow();
        let names: Vec<_> = seen.iter().map(CollisionQuery::dimension).collect();
        assert_eq!(
            names,
            [
                None,
                Some("minecraft:overworld"),
                Some("minecraft:the_nether"),
                Some("minecraft:overworld"),
            ]
        );
        assert_eq!(
            names[1].map(str::as_ptr),
            names[3].map(str::as_ptr),
            "the overworld's id is shared"
        );
    }

    #[test]
    fn the_query_carries_the_context_it_was_made_in() {
        let parts = parts();
        let seen = Rc::new(std::cell::RefCell::new(None));
        let record = Rc::clone(&seen);
        COLLISION_ANSWER.with_borrow_mut(|answer| {
            *answer = Some(Box::new(move |query| {
                *record.borrow_mut() = Some(query.clone());
                None
            }));
        });
        let context = entity(7).with_descending(true);
        parts.shape(
            BlockStateId(1),
            BlockPos::new(1, 64, 1),
            &context,
            &mut || &[],
        );
        let query = seen.borrow().clone().unwrap();
        assert_eq!(query.entity(), Some(7));
        assert_eq!(query.kind(), CollisionEntityKind::Player);
        assert!(query.is_descending() && !query.is_placement());
        // Standing at y 65: on the top of the block at 64, not above the block at 65.
        assert!(query.is_above(&[[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]], [1, 64, 1], false));
        assert!(!query.is_above(&[[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]], [1, 65, 1], false));
        // The move gate's mover stands in for an entity the context does not name.
        sunrise_world::moving_colliders::as_mover(11, || {
            parts.shape(
                BlockStateId(1),
                BlockPos::new(3, 64, 3),
                &CollisionContext::position(65.0),
                &mut || &[],
            );
        });
        assert_eq!(seen.borrow().as_ref().unwrap().entity(), Some(11));
    }
}

/// What an opted-in block costs a level's movers: `cargo test --release -p sunrise-client-lib
/// --lib collision_parts_bench -- --ignored --nocapture`.
#[cfg(test)]
mod collision_parts_bench {
    use super::*;
    use crate::collisions::SceneCollisions;
    use crate::native_client_hooks::test_hooks::{
        COLLISION_ANSWER, COLLISION_ASKS, install, loop_context,
    };
    use crate::world::World;
    use std::rc::Rc;
    use std::time::Instant;
    use sunrise_core::{
        geometry::{Aabb, Axis, Vec3},
        nbt::Tag,
    };
    use sunrise_world::{
        DATA_VERSION, block_collision::BlockCollisionRegistry, collision::CachedCollisionRegistry,
        part_collision::InParts, registry::BlockRegistry,
    };

    const MOVERS: u32 = 500;
    const TICKS: u32 = 200;

    /// `test:belt` (state 1) and `test:plain` (state 2), both declared as a full cube.
    fn tables() -> (BlockRegistry, Tag) {
        let names = || {
            ["minecraft:air", "test:belt", "test:plain"]
                .map(|name| Tag::String(name.into()))
                .to_vec()
        };
        let registry = BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("states", Tag::List(names())),
            (
                "defaults",
                Tag::compound([
                    ("minecraft:air", Tag::Int(0)),
                    ("test:belt", Tag::Int(1)),
                    ("test:plain", Tag::Int(2)),
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
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("states", Tag::List(names())),
            ("shapes", Tag::List(vec![cube, air])),
            ("cached_shapes", Tag::IntArray(vec![1, 0, 0])),
        ]);
        (registry, table)
    }

    /// Nanoseconds per move of `MOVERS` bodies walking along a row of `block`s.
    fn per_move(world: &World, binding: &BlockCollisionRegistry<'_>, with_parts: bool) -> f64 {
        let mut acc = 0.0;
        let context = Arc::new(loop_context());
        let start = Instant::now();
        for _ in 0..TICKS {
            // The game installs fresh parts for each level tick.
            let _parts =
                with_parts.then(|| InParts::enter(Rc::new(ModParts::new(Arc::clone(&context)))));
            for i in 0..MOVERS {
                let x = 1.2 + f64::from(i % 14);
                let z = 1.2 + f64::from(i / 14 % 14);
                let body = Aabb::new(Vec3::new(x, 65.0, z), Vec3::new(x + 0.6, 66.8, z + 0.6));
                let mut scene = SceneCollisions {
                    world,
                    binding,
                    subject: None,
                };
                let delta = Vec3::new(0.1, -0.08, 0.1);
                let mut gate = crate::move_gate::armed(
                    &context,
                    i + 1,
                    "minecraft:player",
                    sunrise_world::entity_move::MoverType::Player,
                    delta,
                );
                let moved =
                    crate::move_gate::scene_resolve(&mut gate, &mut scene, body, delta, 0.6, true)
                        .unwrap();
                acc += moved.y;
            }
        }
        std::hint::black_box(acc);
        start.elapsed().as_secs_f64() * 1.0e9 / f64::from(TICKS * MOVERS)
    }

    #[test]
    #[ignore = "daily"]
    fn collision_parts_bench() {
        install();
        let (registry, table) = tables();
        let cached = CachedCollisionRegistry::from_nbt(&table, &registry).unwrap();
        let binding = BlockCollisionRegistry::new(Arc::new(cached)).unwrap();
        // A floor of belts under every mover.
        let mut world = World::new(BlockStateId(0), 0);
        world
            .chunks
            .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
        for x in 0..16 {
            for z in 0..16 {
                world.set(x, 64, z, BlockStateId(1)).unwrap();
            }
        }
        COLLISION_ANSWER.with_borrow_mut(|answer| {
            *answer = Some(Box::new(|query| {
                // Belt-like: a carried entity gets the full block, the rest a clipped slab.
                Some(if query.entity().is_some_and(|id| id % 2 == 0) {
                    vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]
                } else {
                    vec![[0.0, 0.0, 0.0, 1.0, 0.75, 1.0]]
                })
            }));
        });
        println!("ns per move, {MOVERS} movers x {TICKS} ticks, belts under all of them:");
        let base = per_move(&world, &binding, false);
        println!("  no parts, belts not opted in (today):        {base:8.0}");
        let none = per_move(&world, &binding, true);
        println!("  parts installed, belts not opted in:         {none:8.0}");
        binding.set_part_states([BlockStateId(1)]);
        COLLISION_ASKS.set(0);
        let on = per_move(&world, &binding, true);
        println!(
            "  parts installed, belts opted in:             {on:8.0}  ({} calls per tick)",
            COLLISION_ASKS.get() / TICKS as usize
        );
    }
}
