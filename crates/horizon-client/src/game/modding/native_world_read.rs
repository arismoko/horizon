//! Read-only block, tag and fluid queries for native mods.
use super::{Dim, Game};
use horizon_client::{items::Items, world::World};
use horizon_world::{
    chunk::BlockState,
    fluid::{FluidKind, FluidState as EngineFluidState},
    level::Behaviours,
    registry::{BlockRegistry, BlockStateId},
};
use mod_api::BlockKey;
use mod_api::{FluidState, LevelRef, NativeLifecycleContext, UnresolvedFluid};

/// Why a server block cannot be read.
#[allow(dead_code)]
#[derive(Debug, PartialEq, Eq)]
pub(super) enum NativeBlockReadError {
    /// The context is stale or the call is outside its server tick hook.
    InactiveContext,
    /// The dimension is unknown or is not currently open on the server.
    UnavailableDimension,
    OutOfBounds,
    Unloaded,
    InvalidState,
}

thread_local! {
    // Like the key-to-state mapping, this snapshot belongs to the thread resolving the world.
    static BLOCK_TAGS: std::cell::RefCell<std::collections::HashMap<String, horizon_world::level::data::BlockSet>> = Default::default();
}

/// Bind the world's merged tags alongside its block mappings, including the resource-only case.
/// Never cache on Items: its string tables also serve tools and client network snapshots.
pub(super) fn prepare_block_tags(
    items: &Items,
    mut resolve: impl FnMut(&str) -> Option<horizon_world::level::states::BlockId>,
) -> std::collections::HashMap<String, horizon_world::level::data::BlockSet> {
    items
        .block_tags
        .iter()
        .map(|(tag, members)| {
            let mut set = horizon_world::level::data::BlockSet::default();
            for name in members {
                if let Some(block) = resolve(name) {
                    set.insert(block);
                }
            }
            (tag.clone(), set)
        })
        .collect()
}

pub(super) fn install_block_tags(
    tags: std::collections::HashMap<String, horizon_world::level::data::BlockSet>,
) {
    BLOCK_TAGS.with(|snapshot| *snapshot.borrow_mut() = tags);
}

/// One numeric key mapping, one tag-name lookup and one bit test; no block name lookup.
/// An unavailable owning-thread mapping is not a missing tag.
pub(super) fn block_in_tag(block: BlockKey, tag: &str) -> Option<bool> {
    let block = mod_api::__private::resolved_block(block)?.block;
    let tag = horizon_client::command::args::normalized(tag);
    Some(BLOCK_TAGS.with(|snapshot| {
        snapshot
            .borrow()
            .get(tag.as_ref())
            .is_some_and(|set| set.contains(block))
    }))
}

/// `BlockState.getFluidState`. None for an id no state has.
pub(super) fn state_fluid(
    behaviours: &Behaviours,
    registry: &BlockRegistry,
    state: BlockStateId,
) -> Option<FluidState> {
    registry.state(state)?;
    api_fluid_state(behaviours.fluid(state))
}

/// Convert the engine's state using the load-owned kind-to-key mapping.
pub(super) fn api_fluid_state(state: EngineFluidState) -> Option<FluidState> {
    Some(mod_api::__private::engine_fluid_state(
        mod_api::__private::world_fluid_key(state.kind())?,
        state,
    ))
}

/// `FluidState.is(TagKey)` over the merged fluid tags. The empty fluid is in none.
pub(super) fn fluid_in_tag(items: &Items, fluid: &str, tag: &str) -> bool {
    items.fluid_in_tag(
        &horizon_client::command::args::normalized(tag),
        &horizon_client::command::args::normalized(fluid),
    )
}

/// The fluid a registry id names, with or without `minecraft:`.
fn fluid_kind(fluid: &str) -> Option<FluidKind> {
    FluidKind::from_name(&horizon_client::command::args::normalized(fluid))
}

impl Game<'_> {
    /// The saved name for an engine-issued unresolved fluid.
    pub(super) fn unresolved_fluid_id(&self, fluid: UnresolvedFluid) -> String {
        mod_api::__private::unresolved_fluid_name(fluid)
    }

    /// The merged tag tables, once the world has built them. None on a server's client, whose
    /// tags are not the server's, and before the world has loaded its tags (the world lock is
    /// installed after they are built).
    pub(super) fn native_tag_items(&self) -> Option<&Items> {
        (!self.options.execution_role.is_client() && self.session_lock.is_some())
            .then(|| self.items())
    }

    /// Whether the resolved block is in the world's merged block tag. Unknown tags return false;
    /// unavailable authoritative tags or owning-thread block mappings return None.
    pub(super) fn block_in_tag(&self, block: BlockKey, tag: &str) -> Option<bool> {
        self.native_tag_items()?;
        block_in_tag(block, tag)
    }

    /// The outline of `state` as `BlockState.getShape` answers it with an empty getter at the
    /// origin (`EmptyBlockGetter.INSTANCE`, `BlockPos.ZERO`): its boxes, each `[min x, y, z,
    /// max x, y, z]` in the block's own space, with no random offset and none of a world's
    /// context (a state that reads its neighbours or block entity answers what its state alone
    /// gives). Empty for a state with no outline, such as air. None for an id no state has.
    pub(super) fn state_outline(&self, state: BlockStateId) -> Option<Vec<[f64; 6]>> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).outline.to_vec())
    }

    /// The fluid state `state` holds (`BlockState.getFluidState`): water for a waterlogged state,
    /// the empty fluid for most. None for an id no state has.
    pub(super) fn state_fluid_state(&self, state: BlockStateId) -> Option<FluidState> {
        state_fluid(self.res.behaviours(), self.pack.assets.registry, state)
    }

    /// Whether `fluid` is in the fluid tag `tag` (`FluidState.is(TagKey)`, `minecraft:water` or
    /// `water`), over the merged fluid tags. An unknown tag is false. None as for
    /// `block_in_tag`.
    pub(super) fn fluid_in_tag(&self, fluid: &str, tag: &str) -> Option<bool> {
        Some(fluid_in_tag(self.native_tag_items()?, fluid, tag))
    }

    /// The fluid state at `pos` of the level `level` names (`BlockGetter.getFluidState`). None
    /// for a stale handle, a chunk that isn't loaded or a `y` outside the world: a read never
    /// loads a chunk.
    pub(super) fn level_fluid_state(&self, level: LevelRef, pos: [i32; 3]) -> Option<FluidState> {
        let state = self.level_block_state(level, pos)?;
        self.state_fluid_state(state)
    }

    /// Whether `fluid` and `other` are the same fluid (`Fluid.isSame`): a source and its
    /// flowing form are, so `minecraft:water` and `minecraft:flowing_water` are the same, water
    /// and lava not. Names are fluid ids, with or without `minecraft:`; false when either
    /// names no fluid.
    pub(super) fn fluid_is_same(&self, fluid: &str, other: &str) -> bool {
        match (fluid_kind(fluid), fluid_kind(other)) {
            (Some(fluid), Some(other)) => fluid.same_family(other),
            _ => false,
        }
    }

    /// The bucket item that picks `fluid` up (`Fluid.getBucket`), `minecraft:water_bucket` for
    /// water and flowing water. None for `minecraft:empty`, whose bucket is air, and for a name
    /// no fluid has.
    pub(super) fn fluid_bucket(&self, fluid: &str) -> Option<&'static str> {
        horizon_world::level::fluid_bucket(fluid_kind(fluid)?)
    }

    /// The sound event a bucket plays as it picks `fluid` up (`Fluid.getPickupSound`),
    /// `minecraft:item.bucket.fill_lava` for lava and flowing lava. None for
    /// `minecraft:empty`, which has none, and for a name no fluid has.
    pub(super) fn fluid_pickup_sound(&self, fluid: &str) -> Option<&'static str> {
        horizon_world::level::fluid_pickup_sound(fluid_kind(fluid)?)
    }

    /// Read a loaded block in a server tick hook, without loading a chunk.
    /// The returned name and properties are owned and remain valid after the hook.
    #[allow(dead_code)]
    pub(super) fn native_server_block(
        &self,
        context: &NativeLifecycleContext,
        dimension: &str,
        pos: [i32; 3],
    ) -> Result<BlockState, NativeBlockReadError> {
        if !self.native_lifecycle.permits_read(context) {
            return Err(NativeBlockReadError::InactiveContext);
        }
        let dim = match dimension {
            "minecraft:overworld" => Dim::Overworld,
            "minecraft:the_nether" => Dim::Nether,
            "minecraft:the_end" => Dim::End,
            _ => return Err(NativeBlockReadError::UnavailableDimension),
        };
        let world = if dim == self.dim {
            Some(&self.world)
        } else {
            self.levels.parked_world(dim)
        }
        .ok_or(NativeBlockReadError::UnavailableDimension)?;
        read_block(world, self.pack.assets.registry, pos)
    }
}

/// Resolve an existing world state, preserving unloaded and invalid states.
fn read_block(
    world: &World,
    registry: &BlockRegistry,
    [x, y, z]: [i32; 3],
) -> Result<BlockState, NativeBlockReadError> {
    if !(-30_000_000..30_000_000).contains(&x)
        || !(-30_000_000..30_000_000).contains(&z)
        || !world.shape.contains_y(y)
    {
        return Err(NativeBlockReadError::OutOfBounds);
    }
    if !world.loaded(x, z) {
        return Err(NativeBlockReadError::Unloaded);
    }
    registry
        .state(world.block(x, y, z))
        .cloned()
        .ok_or(NativeBlockReadError::InvalidState)
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::*;
    use horizon_client::chunk::Chunk;
    use horizon_client::items::TagRegistry;
    use mod_api::LevelHost;
    use std::sync::Arc;

    #[test]
    fn fluid_state_conversion_preserves_source_and_empty_invariants() {
        mod_api::__private::install(Default::default());
        crate::game::api_values::register_game_keys([]);
        for kind in FluidKind::ALL {
            let engine =
                EngineFluidState::new(kind, if kind == FluidKind::Empty { 0 } else { 8 }, true);
            let api = api_fluid_state(engine).unwrap();
            assert_eq!(api.is_empty(), kind == FluidKind::Empty);
            assert_eq!(api.is_source(), engine.is_source());
            assert_eq!(mod_api::__private::fluid_id(api.fluid), Some(kind.name()));
        }
        let source = api_fluid_state(EngineFluidState::new(FluidKind::Water, 8, true)).unwrap();
        let flowing =
            api_fluid_state(EngineFluidState::new(FluidKind::FlowingWater, 8, false)).unwrap();
        assert!(source.falling && source.is_source());
        assert_eq!(flowing.amount, 8);
        assert!(!flowing.is_source());
    }

    #[test]
    fn numeric_block_tags_rebind_the_merged_snapshot_without_changing_keys() {
        use horizon_world::level::states::BlockId;
        use mod_api::__private::{ResolvedBlock, part_tests, resolve_blocks};
        mod_api::__private::install(Default::default());
        crate::game::api_values::register_game_keys([
            "minecraft:stone".into(),
            "test:machine".into(),
        ]);
        let stone = part_tests::block_key("stone");
        let machine = part_tests::block_key("test:machine");
        let mut items = Items::for_tests(Default::default());
        items
            .block_tags
            .insert("test:machines".into(), ["test:machine".into()].into());
        items.block_tags.insert(
            "minecraft:base_stone_overworld".into(),
            ["minecraft:stone".into()].into(),
        );
        for (stone_index, machine_index) in [(2, 71), (96, 5)] {
            let resolve = |name: &str| match name {
                "minecraft:stone" => Some(BlockId(stone_index)),
                "test:machine" => Some(BlockId(machine_index)),
                _ => None,
            };
            resolve_blocks(|name| {
                Some(ResolvedBlock {
                    block: resolve(name)?,
                    default: BlockStateId(0),
                    states: mod_api::BlockStateRange::new(BlockStateId(0), 1),
                })
            })
            .unwrap();
            install_block_tags(prepare_block_tags(&items, resolve));
            assert_eq!(block_in_tag(machine, "test:machines"), Some(true));
            assert_eq!(block_in_tag(stone, "test:machines"), Some(false));
            assert_eq!(block_in_tag(stone, "base_stone_overworld"), Some(true));
            assert_eq!(
                block_in_tag(machine, "minecraft:base_stone_overworld"),
                Some(false)
            );
            assert_eq!(block_in_tag(machine, "test:missing"), Some(false));
            assert_eq!(machine, part_tests::block_key("test:machine"));
        }
        // A replaced merged tag snapshot must discard previous membership, not union it.
        items
            .block_tags
            .insert("test:machines".into(), ["minecraft:stone".into()].into());
        install_block_tags(prepare_block_tags(&items, |name| match name {
            "minecraft:stone" => Some(BlockId(96)),
            "test:machine" => Some(BlockId(5)),
            _ => None,
        }));
        assert_eq!(block_in_tag(machine, "test:machines"), Some(false));
        assert_eq!(block_in_tag(stone, "test:machines"), Some(true));
        // Valid key, no owning world: never answer false using another thread's tags.
        assert_eq!(
            std::thread::spawn(move || block_in_tag(stone, "test:machines"))
                .join()
                .unwrap(),
            None
        );
    }

    #[test]
    fn reads_only_loaded_in_bounds_states() {
        let facts = br#"{"defaults":{"minecraft:air":0,"test:pillar":1},"states":[
            ["air",true,false,"","minecraft:air"],
            ["pillar",false,false,"",{"id":"test:pillar","properties":{"axis":"x"}}]
        ]}"#;
        let registry = BlockRegistry::from_facts_json(&facts[..]).unwrap();
        let mut world = World::new(BlockStateId(0), 0);
        world.chunks.insert((-1, 0), Arc::new(Chunk::empty(-1, 0)));
        world.set(-1, 64, 1, BlockStateId(1)).unwrap();
        // The Nether's build range starts at y 0 (its level's shape).
        let mut nether = World::new(BlockStateId(0), 0);
        nether.shape = horizon_client::chunk::LevelShape::NETHER;
        nether.chunks.insert((-1, 0), Arc::new(Chunk::empty(-1, 0)));
        let block = read_block(&world, &registry, [-1, 64, 1]).unwrap();
        assert_eq!(block.name, "test:pillar");
        assert_eq!(block.properties["axis"], "x");
        assert_eq!(
            read_block(&world, &registry, [-1, 65, 1]).unwrap().name,
            "minecraft:air"
        );
        assert_eq!(
            read_block(&world, &registry, [0, 64, 1]),
            Err(NativeBlockReadError::Unloaded)
        );
        assert_eq!(
            read_block(&nether, &registry, [-1, -1, 1]),
            Err(NativeBlockReadError::OutOfBounds)
        );
        assert_eq!(
            read_block(&world, &registry, [30_000_000, 64, 1]),
            Err(NativeBlockReadError::OutOfBounds)
        );
        world.set(-1, 64, 1, BlockStateId(99)).unwrap();
        assert_eq!(
            read_block(&world, &registry, [-1, 64, 1]),
            Err(NativeBlockReadError::InvalidState)
        );
        assert_eq!(world.chunks.len(), 1, "reads must not load chunks");
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn recipes_of_type_answers_vanilla_types_from_the_live_table() {
        with_game(|game| {
            let smelting = game.recipes_of_type("minecraft:smelting");
            let iron = smelting
                .iter()
                .find(|recipe| recipe.id == "minecraft:iron_ingot_from_smelting_raw_iron")
                .expect("vanilla's smelting recipe of raw iron");
            assert_eq!(iron.json["cookingtime"], 200);
            assert_eq!(iron.cooking_time, Some(200));
            assert_eq!(iron.experience, Some(0.7));
            assert_eq!(iron.ingredients.len(), 1);
            assert!(iron.ingredients[0].items.contains("minecraft:raw_iron"));
            assert_eq!(iron.results[0].item.as_ref(), "minecraft:iron_ingot");
            let shaped = game.recipes_of_type("minecraft:crafting_shaped");
            let bucket = shaped
                .iter()
                .find(|recipe| recipe.id == "minecraft:bucket")
                .expect("vanilla's bucket recipe");
            assert_eq!(bucket.ingredients.len(), 3, "its pattern's three cells");
            assert_eq!(bucket.results[0].item.as_ref(), "minecraft:bucket");
            assert_eq!(bucket.cooking_time, None);
            assert!(
                !game
                    .recipes_of_type("minecraft:campfire_cooking")
                    .is_empty()
            );
            assert!(game.recipes_of_type("nope:missing").is_empty());
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn tag_and_fluid_reads_follow_vanilla_and_load_nothing() {
        with_game(|game| {
            let dir =
                std::env::temp_dir().join(format!("horizon-tag-reads-{}", std::process::id()));
            game.session_lock = Some(horizon_client::save::SessionLock::acquire(&dir).unwrap());
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let water = registry.default_state("minecraft:water").unwrap();
            let slab = game
                .block_state_id("minecraft:oak_slab[waterlogged=true]")
                .unwrap();
            let dry_slab = game.block_state_id("minecraft:oak_slab").unwrap();
            let lava = registry.default_state("minecraft:lava").unwrap();

            // Members come in the tag file's order, in each registry, with or without the
            // namespace; an unknown tag lists nothing.
            let oak = [
                "oak_log",
                "oak_wood",
                "stripped_oak_log",
                "stripped_oak_wood",
            ]
            .map(|name| format!("minecraft:{name}"));
            let engine_members = |registry, tag: &str| {
                game.native_tag_items().map(|items| {
                    items
                        .tag_members(registry, &horizon_client::command::args::normalized(tag))
                        .unwrap_or_default()
                        .to_vec()
                })
            };
            for registry in [TagRegistry::Block, TagRegistry::Item] {
                assert_eq!(
                    engine_members(registry, "oak_logs").as_deref(),
                    Some(&oak[..])
                );
            }
            {
                let host: &dyn mod_api::Host = game;
                let tags = host.tags();
                let oak_tag = tags
                    .key(&mod_api::Identifier::parse("minecraft:oak_logs").unwrap())
                    .unwrap();
                assert_eq!(
                    tags.items(oak_tag)
                        .map(|keys| keys.into_iter().map(|key| key.id(host)).collect::<Vec<_>>())
                        .as_deref(),
                    Some(&oak[..])
                );
                let item = host
                    .item_key(&mod_api::Identifier::parse("minecraft:oak_log").unwrap())
                    .unwrap();
                assert_eq!(tags.is_in_tag(item, oak_tag), Some(true));
            }
            assert_eq!(
                engine_members(TagRegistry::Fluid, "minecraft:water").unwrap(),
                ["minecraft:water", "minecraft:flowing_water"]
            );
            assert_eq!(
                engine_members(TagRegistry::Item, "nope:missing"),
                Some(Vec::new())
            );

            // A full block is one unit box, a bottom slab half of it, air none, and an id no
            // state has is unknown.
            assert_eq!(
                game.state_outline(stone),
                Some(vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]])
            );
            assert_eq!(
                game.state_outline(dry_slab),
                Some(vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]])
            );
            let air = registry.default_state("minecraft:air").unwrap();
            assert_eq!(game.state_outline(air), Some(Vec::new()));
            assert_eq!(game.state_outline(BlockStateId(u32::MAX)), None);

            assert_eq!(
                game.block_in_tag(
                    mod_api::__private::part_tests::block_key("minecraft:stone"),
                    "minecraft:base_stone_overworld"
                ),
                Some(true)
            );
            assert_eq!(
                game.block_in_tag(
                    mod_api::__private::part_tests::block_key("stone"),
                    "base_stone_overworld"
                ),
                Some(true)
            );
            assert_eq!(
                game.block_in_tag(
                    mod_api::__private::part_tests::block_key("minecraft:stone"),
                    "minecraft:logs"
                ),
                Some(false)
            );
            assert_eq!(
                game.block_in_tag(
                    mod_api::__private::part_tests::block_key("minecraft:stone"),
                    "nope:missing"
                ),
                Some(false)
            );

            assert_eq!(
                game.state_fluid_state(water),
                api_fluid_state(EngineFluidState::WATER)
            );
            assert_eq!(
                game.fluid_in_tag("minecraft:water", "minecraft:water"),
                Some(true)
            );
            assert_eq!(game.fluid_in_tag("water", "water"), Some(true));
            assert_eq!(
                game.fluid_in_tag("minecraft:water", "minecraft:lava"),
                Some(false)
            );
            assert_eq!(
                mod_api::__private::fluid_id(game.state_fluid_state(lava).unwrap().fluid).unwrap(),
                "minecraft:lava"
            );
            assert_eq!(
                game.fluid_in_tag("minecraft:lava", "minecraft:lava"),
                Some(true)
            );
            assert_eq!(
                game.fluid_in_tag("minecraft:lava", "minecraft:water"),
                Some(false)
            );
            assert_eq!(
                game.fluid_in_tag("minecraft:empty", "minecraft:water"),
                Some(false)
            );
            assert_eq!(
                game.state_fluid_state(slab),
                api_fluid_state(EngineFluidState::WATER)
            );
            assert_eq!(
                game.state_fluid_state(dry_slab),
                api_fluid_state(EngineFluidState::EMPTY)
            );
            assert_eq!(
                game.state_fluid_state(stone),
                api_fluid_state(EngineFluidState::EMPTY)
            );
            assert_eq!(game.state_fluid_state(BlockStateId(u32::MAX)), None);

            // The world holds a chunk with water in it; a neighbour is not loaded.
            let mut chunk = Chunk::empty(0, 0);
            chunk.set(1, 64, 1, slab.0);
            game.world.chunks.insert((0, 0), Arc::new(chunk));
            let level = game.level(game.dim.id()).unwrap();
            let chunks = game.world.chunks.len();
            assert_eq!(
                game.level_fluid_state(level, [1, 64, 1]),
                api_fluid_state(EngineFluidState::WATER)
            );
            assert_eq!(
                game.level_fluid_state(level, [2, 64, 1]),
                api_fluid_state(EngineFluidState::EMPTY)
            );
            assert_eq!(game.level_fluid_state(level, [16, 64, 1]), None);
            assert_eq!(game.level_fluid_state(level, [1, 4000, 1]), None);
            game.with_level(|_, host, _| {
                // Not an authoritative level: tags are not the server's, so they read None.
                assert_eq!(
                    host.block_in_tag(
                        mod_api::__private::part_tests::block_key("minecraft:stone"),
                        "minecraft:base_stone_overworld"
                    ),
                    None
                );
                assert_eq!(
                    host.fluid_in_tag("minecraft:water", "minecraft:water"),
                    None
                );
                assert_eq!(
                    host.fluid_state([1, 64, 1]),
                    api_fluid_state(EngineFluidState::WATER)
                );
            });
            game.options.dedicated = Some(Vec::new());
            let bounds = game.level_height(level).unwrap();
            let parsed = game.block_state_id("minecraft:oak_slab[waterlogged=true]");
            let default = game.block_state_default(mod_api::__private::part_tests::block_key(
                "minecraft:oak_slab",
            ));
            game.with_level(|_, host, ticks| {
                assert_eq!(
                    host.block_state_id("minecraft:oak_slab[waterlogged=true]"),
                    parsed
                );
                assert_eq!(
                    host.block_state_default(mod_api::__private::part_tests::block_key(
                        "minecraft:oak_slab"
                    )),
                    default
                );
                assert_eq!(host.block_state_id("minecraft:oak_slab[bad=true]"), None);

                let wrapped = super::super::mod_api_host::LevelTicks {
                    level: &*host,
                    ticks,
                };
                assert_eq!(
                    wrapped.block_state_id("minecraft:oak_slab[waterlogged=true]"),
                    parsed
                );
                assert_eq!(
                    wrapped.block_state_default(mod_api::__private::part_tests::block_key(
                        "minecraft:oak_slab"
                    )),
                    default
                );
                assert_eq!(wrapped.block_state_id("minecraft:oak_slab[bad=true]"), None);

                assert_eq!(host.state_string(stone).as_deref(), Some("minecraft:stone"));
                assert!(
                    host.state_string(slab)
                        .unwrap()
                        .contains("waterlogged=true")
                );
                assert_eq!(host.state_string(BlockStateId(u32::MAX)), None);
                assert_eq!((host.min_y(), host.height()), bounds);
                assert_eq!(
                    host.block_in_tag(
                        mod_api::__private::part_tests::block_key("minecraft:stone"),
                        "minecraft:base_stone_overworld"
                    ),
                    Some(true)
                );
                assert_eq!(
                    host.block_in_tag(
                        mod_api::__private::part_tests::block_key("minecraft:stone"),
                        "minecraft:logs"
                    ),
                    Some(false)
                );
                assert_eq!(
                    host.fluid_state([1, 64, 1]),
                    api_fluid_state(EngineFluidState::WATER)
                );
                assert_eq!(host.fluid_state([16, 64, 1]), None);
                assert_eq!(
                    host.state_fluid_state(slab),
                    api_fluid_state(EngineFluidState::WATER)
                );
                assert_eq!(
                    host.fluid_in_tag("minecraft:water", "minecraft:water"),
                    Some(true)
                );
                assert_eq!(
                    host.fluid_in_tag("minecraft:empty", "minecraft:water"),
                    Some(false)
                );
            });
            {
                use mod_api::LevelHost;
                let hit =
                    super::super::native_hooks::HitLevel(horizon_client::health::DamageLevel::of(
                        &mut game.world,
                        game.pack.assets.registry,
                        Some(world_items!(game)),
                        Some(&game.pack.block_registries),
                        &game.damage_hooks,
                    ));
                assert_eq!(
                    hit.block_state_id("minecraft:oak_slab[waterlogged=true]"),
                    parsed
                );
                assert_eq!(
                    hit.block_state_default(mod_api::__private::part_tests::block_key(
                        "minecraft:oak_slab"
                    )),
                    default
                );
                assert_eq!(hit.block_state_id("minecraft:oak_slab[bad=true]"), None);

                assert_eq!(hit.state_string(stone).as_deref(), Some("minecraft:stone"));
                assert!(hit.state_string(slab).unwrap().contains("waterlogged=true"));
                assert_eq!(hit.state_string(BlockStateId(u32::MAX)), None);
                assert_eq!((hit.min_y(), hit.height()), bounds);
                assert_eq!(
                    hit.block_in_tag(
                        mod_api::__private::part_tests::block_key("minecraft:stone"),
                        "minecraft:base_stone_overworld"
                    ),
                    Some(true)
                );
                assert_eq!(
                    hit.fluid_in_tag("minecraft:water", "minecraft:water"),
                    Some(true)
                );
            }
            {
                use mod_api::LevelHost;
                let hit =
                    super::super::native_hooks::HitLevel(horizon_client::health::DamageLevel::of(
                        &mut game.world,
                        game.pack.assets.registry,
                        None,
                        None,
                        &game.damage_hooks,
                    ));
                assert_eq!(hit.block_state_id("minecraft:stone"), None);
                assert_eq!(
                    hit.block_state_default(mod_api::__private::part_tests::block_key(
                        "minecraft:stone"
                    )),
                    None
                );
                assert_eq!(
                    hit.block_in_tag(
                        mod_api::__private::part_tests::block_key("minecraft:stone"),
                        "minecraft:logs"
                    ),
                    None
                );
                assert_eq!(hit.fluid_in_tag("minecraft:water", "minecraft:water"), None);
            }
            assert_eq!(game.world.chunks.len(), chunks, "nothing was loaded");
            game.options.execution_role = super::super::execution_role::ExecutionRole::Client;
            let host: &dyn mod_api::Host = game;
            let tags = host.tags();
            let oak_tag = tags
                .key(&mod_api::Identifier::parse("minecraft:oak_logs").unwrap())
                .unwrap();
            let item = host
                .item_key(&mod_api::Identifier::parse("minecraft:oak_log").unwrap())
                .unwrap();
            assert_eq!(tags.items(oak_tag), None);
            assert_eq!(tags.is_in_tag(item, oak_tag), None);
            game.session_lock = None;
            let _ = std::fs::remove_dir_all(dir);
        });
    }

    /// Sources and their flowing forms are one fluid, the empty fluid has no bucket and no
    /// pickup sound, and a name no fluid has answers nothing, through a mod's `Host`.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn fluid_sameness_buckets_and_pickup_sounds_follow_vanilla() {
        with_game(|game| {
            let host: &dyn mod_api::Host = game;
            assert!(host.fluid_is_same("minecraft:water", "minecraft:flowing_water"));
            assert!(host.fluid_is_same("flowing_lava", "lava"));
            assert!(host.fluid_is_same("minecraft:empty", "minecraft:empty"));
            assert!(!host.fluid_is_same("minecraft:water", "minecraft:lava"));
            assert!(!host.fluid_is_same("minecraft:water", "minecraft:empty"));
            assert!(!host.fluid_is_same("nope:missing", "nope:missing"));

            assert_eq!(
                host.fluid_bucket("minecraft:flowing_water"),
                Some("minecraft:water_bucket")
            );
            assert_eq!(host.fluid_bucket("lava"), Some("minecraft:lava_bucket"));
            assert_eq!(host.fluid_bucket("minecraft:empty"), None);
            assert_eq!(host.fluid_bucket("nope:missing"), None);

            assert_eq!(
                host.fluid_pickup_sound("minecraft:water"),
                Some("minecraft:item.bucket.fill")
            );
            assert_eq!(
                host.fluid_pickup_sound("minecraft:flowing_lava"),
                Some("minecraft:item.bucket.fill_lava")
            );
            assert_eq!(host.fluid_pickup_sound("minecraft:empty"), None);
            assert_eq!(host.fluid_pickup_sound("nope:missing"), None);
        });
    }
}
