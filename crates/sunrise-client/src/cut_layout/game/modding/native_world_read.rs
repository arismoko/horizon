#![cfg(test)]

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;

#[cfg(test)]
mod tests {
    use mod_api::LevelHost;
    use std::sync::Arc;
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_gameplay_common::game::native_hooks::base::BaseGameExt1;
    use sunrise_items::items::TagRegistry;
    use sunrise_level::chunk::{Chunk, Light};
    use sunrise_world::registry::BlockRegistry;
    use {
        crate::game::native_world_read_common::*,
        sunrise_gameplay_common::game::native_world_read_common::{
            BlockStateId, EngineFluidState, FluidKind, Items, World, api_fluid_state, can_see_sky,
            fluid_shape,
        },
    };

    #[test]
    fn sky_visibility_uses_the_published_sky_layer() {
        let mut world = World::new(BlockStateId(0), 0);
        let y = world.shape.min_y;
        let above = world.shape.max_y() + 16;
        let mut chunk = Chunk::empty(0, 0);
        Arc::make_mut(&mut chunk.sections[0]).light = Light::Uniform(0xef);
        world.chunks.insert((0, 0), Arc::new(chunk));
        assert!(
            !can_see_sky(&world, [0, y, 0]),
            "block light is not sky light"
        );
        assert!(can_see_sky(&world, [0, above, 0]));
        assert!(can_see_sky(&world, [32, y, 0]));
        world.shape.sky_light = false;
        assert!(!can_see_sky(&world, [0, above, 0]));
        assert!(!can_see_sky(&world, [32, y, 0]));
        assert!(!world.loaded(32, 0));
    }

    #[test]
    fn a_fluid_shape_is_full_under_the_same_fluid_and_its_own_height_otherwise() {
        let source = EngineFluidState::WATER;
        let shallow = EngineFluidState::new(FluidKind::FlowingWater, 3, false);
        let lava = EngineFluidState::new(FluidKind::Lava, 8, false);
        assert_eq!(fluid_shape(EngineFluidState::EMPTY, Some(source)), None);
        // FlowingFluid.getOwnHeight: amount / 9, so a source is 8/9 high under air.
        let top = |shape: Option<[f64; 6]>| shape.map(|b| b[4]);
        assert_eq!(
            top(fluid_shape(source, None)),
            Some(f64::from(8.0f32 / 9.0))
        );
        assert_eq!(
            top(fluid_shape(shallow, None)),
            Some(f64::from(3.0f32 / 9.0))
        );
        // Flowing water under a source is the same fluid: a full block.
        assert_eq!(top(fluid_shape(shallow, Some(source))), Some(1.0));
        assert_eq!(
            top(fluid_shape(source, Some(lava))),
            Some(f64::from(8.0f32 / 9.0))
        );
        assert_eq!(
            fluid_shape(source, Some(source)),
            Some([0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
        );
    }

    #[test]
    fn fluid_state_conversion_preserves_source_and_empty_invariants() {
        mod_platform::__private::install(Default::default());
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
    fn api_block_tags_rebind_the_merged_snapshot_without_changing_keys() {
        use mod_platform::__private::part_tests;
        use sunrise_gameplay_common::game::native_tags;
        mod_platform::__private::install(Default::default());
        crate::game::api_values::register_game_keys([
            "minecraft:stone".into(),
            "test:machine".into(),
        ]);
        let stone = part_tests::block_key("stone");
        let machine = part_tests::block_key("test:machine");
        let tag = |name| {
            mod_api::__private::tag_key(&mod_api::Identifier::parse(name).unwrap())
                .unwrap()
                .typed::<mod_api::BlockKey>()
        };
        let machines = tag("test:machines");
        let base_stone = tag("base_stone_overworld");
        let qualified_base_stone = tag("minecraft:base_stone_overworld");
        assert_eq!(base_stone, qualified_base_stone);
        let missing = tag("test:missing");
        let mut items = Items::for_tests(Default::default());
        assert_eq!(
            native_tags::block_in_tag(Some(&items), machine, machines),
            None
        );
        items.set_tags(
            TagRegistry::Block,
            [
                ("test:machines".into(), vec!["test:machine".into()]),
                (
                    "minecraft:base_stone_overworld".into(),
                    vec!["minecraft:stone".into()],
                ),
            ]
            .into(),
        );
        for facts in [
            r#"{"defaults":{"minecraft:air":2,"minecraft:stone":0,"test:machine":1},"states":[["s",false,false,"","minecraft:stone"],["m",false,false,"","test:machine"],["a",true,false,"","minecraft:air"]]}"#,
            r#"{"defaults":{"minecraft:air":2,"minecraft:stone":1,"test:machine":0},"states":[["m",false,false,"","test:machine"],["s",false,false,"","minecraft:stone"],["a",true,false,"","minecraft:air"]]}"#,
        ] {
            // Build reordered numeric registries from the same API definitions.
            let registry = BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
            sunrise_gameplay_common::game::api_values::resolve_block_definitions(&registry)
                .unwrap();
            assert_eq!(
                native_tags::block_in_tag(Some(&items), machine, machines),
                Some(true)
            );
            assert_eq!(
                native_tags::block_in_tag(Some(&items), stone, machines),
                Some(false)
            );
            assert_eq!(
                native_tags::block_in_tag(Some(&items), stone, base_stone),
                Some(true)
            );
            assert_eq!(
                native_tags::block_in_tag(Some(&items), machine, qualified_base_stone),
                Some(false)
            );
            assert_eq!(
                native_tags::block_in_tag(Some(&items), machine, missing),
                Some(false)
            );
            assert_eq!(machine, part_tests::block_key("test:machine"));
        }
        // A replaced merged tag snapshot must discard previous membership, not union it.
        items.set_tags(
            TagRegistry::Block,
            [("test:machines".into(), vec!["minecraft:stone".into()])].into(),
        );
        assert_eq!(
            native_tags::block_in_tag(Some(&items), machine, machines),
            Some(false)
        );
        assert_eq!(
            native_tags::block_in_tag(Some(&items), stone, machines),
            Some(true)
        );
        // Valid key, no side snapshot: never answer false without authoritative tags.
        assert_eq!(native_tags::block_in_tag(None, stone, machines), None);
    }

    #[test]
    #[ignore = "daily"]
    fn recipes_of_type_answers_vanilla_types_from_the_live_table() {
        with_game(|game| {
            let smelting = game.recipes_of_type("minecraft:smelting").0;
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
            let shaped = game.recipes_of_type("minecraft:crafting").0;
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
                    .0
                    .is_empty()
            );
            assert!(game.recipes_of_type("nope:missing").0.is_empty());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn tag_and_fluid_reads_follow_vanilla_and_load_nothing() {
        with_game(|game| {
            let dir =
                std::env::temp_dir().join(format!("sunrise-tag-reads-{}", std::process::id()));
            game.server.session_lock.0 =
                Some(sunrise_sim::save::SessionLock::acquire(&dir).unwrap());
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
                        .tag_members(registry, &sunrise_command::args::normalized(tag))
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
                    .unwrap()
                    .typed();
                let mod_api::TagMembers::Members(keys) = tags.items(oak_tag) else {
                    panic!("the oak logs tag is loaded");
                };
                assert_eq!(
                    keys.into_iter()
                        .map(|key| key.id(host))
                        .collect::<Option<Vec<_>>>()
                        .as_deref(),
                    Some(&oak.each_ref().map(String::as_str)[..])
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
            use mod_api::HostBlockStates;
            use sunrise_gameplay_common::game::api_values::TryIntoApi;
            let stone_state = stone.try_into_api().unwrap();
            let water_state = water.try_into_api().unwrap();
            let slab_state = slab.try_into_api().unwrap();
            let dry_slab_state = dry_slab.try_into_api().unwrap();
            let lava_state = lava.try_into_api().unwrap();
            let air_state = registry
                .default_state("minecraft:air")
                .unwrap()
                .try_into_api()
                .unwrap();
            let invalid_state =
                mod_api::__private::block_state_from_ordinal(stone_state.block(), u16::MAX);
            {
                let host: &dyn mod_api::Host = game;
                let states = host.block_states();
                assert_eq!(
                    stone_state.outline(&states),
                    Some(&[[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]][..])
                );
                assert_eq!(
                    dry_slab_state.outline(&states),
                    Some(&[[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]][..])
                );
                assert_eq!(air_state.outline(&states), Some(&[][..]));
                assert_eq!(invalid_state.outline(&states), None);
            }

            let stone_key = mod_platform::__private::part_tests::block_key("minecraft:stone");
            assert_eq!(
                stone_key,
                mod_platform::__private::part_tests::block_key("stone")
            );
            let water_key = mod_api::__private::fluid_key("minecraft:water").unwrap();
            let lava_key = mod_api::__private::fluid_key("minecraft:lava").unwrap();
            let empty_key = mod_api::__private::fluid_key("minecraft:empty").unwrap();
            let (base_stone_tag, logs_tag, water_tag) = {
                let host: &dyn mod_api::Host = game;
                let tags = host.tags();
                let key = |name| {
                    tags.key(&mod_api::Identifier::parse(name).unwrap())
                        .unwrap()
                };
                let base_stone_tag = key("minecraft:base_stone_overworld").typed();
                assert_eq!(base_stone_tag, key("base_stone_overworld").typed());
                let logs_tag = key("minecraft:logs").typed();
                let missing_tag = key("nope:missing").typed();
                let water_tag = key("minecraft:water").typed();
                assert_eq!(water_tag, key("water").typed());
                let lava_tag = key("minecraft:lava").typed();

                assert_eq!(stone_key.is_in_tag(tags, base_stone_tag), Some(true));
                assert_eq!(stone_key.is_in_tag(tags, logs_tag), Some(false));
                assert_eq!(stone_key.is_in_tag(tags, missing_tag), Some(false));

                assert_eq!(water_key.is_in_tag(tags, water_tag), Some(true));
                assert_eq!(water_key.is_in_tag(tags, lava_tag), Some(false));
                assert_eq!(lava_key.is_in_tag(tags, lava_tag), Some(true));
                assert_eq!(lava_key.is_in_tag(tags, water_tag), Some(false));
                assert_eq!(empty_key.is_in_tag(tags, water_tag), Some(false));
                (base_stone_tag, logs_tag, water_tag)
            };

            {
                let host: &dyn mod_api::Host = game;
                let states = host.block_states();
                assert_eq!(
                    water_state.fluid_state(&states),
                    api_fluid_state(EngineFluidState::WATER)
                );
                assert_eq!(
                    water_state
                        .fluid_state(&states)
                        .unwrap()
                        .is_in_tag(host.tags(), water_tag),
                    Some(true)
                );
                assert_eq!(
                    mod_api::__private::fluid_id(lava_state.fluid_state(&states).unwrap().fluid)
                        .unwrap(),
                    "minecraft:lava"
                );
                assert_eq!(
                    slab_state.fluid_state(&states),
                    api_fluid_state(EngineFluidState::WATER)
                );
                assert_eq!(
                    dry_slab_state.fluid_state(&states),
                    api_fluid_state(EngineFluidState::EMPTY)
                );
                assert_eq!(
                    stone_state.fluid_state(&states),
                    api_fluid_state(EngineFluidState::EMPTY)
                );
                assert_eq!(invalid_state.fluid_state(&states), None);
            }

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
            game.options.execution_role =
                sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
            game.with_level(|_, host, _| {
                // Not an authoritative level: tags are not the server's, so they read None.
                assert_eq!(stone_key.is_in_tag(host.tags(), base_stone_tag), None);
                assert_eq!(water_key.is_in_tag(host.tags(), water_tag), None);
                assert_eq!(
                    host.fluid_state([1, 64, 1]),
                    api_fluid_state(EngineFluidState::WATER)
                );
            });
            game.options.execution_role =
                sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
            game.options.dedicated = Some(Vec::new());
            let bounds = game.level_height(level).unwrap();
            let stone = stone_state;
            let slab = slab_state;
            let invalid = invalid_state;
            let parsed = game
                .block_state_id("minecraft:oak_slab[waterlogged=true]")
                .try_into_api()
                .unwrap();
            let default = game
                .block_state_default(mod_platform::__private::part_tests::block_key(
                    "minecraft:oak_slab",
                ))
                .try_into_api()
                .unwrap();
            game.with_level(|_, host, ticks| {
                assert_eq!(
                    host.block_state_id("minecraft:oak_slab[waterlogged=true]"),
                    parsed
                );
                assert_eq!(
                    host.block_state_default(mod_platform::__private::part_tests::block_key(
                        "minecraft:oak_slab"
                    )),
                    default
                );
                assert_eq!(host.block_state_id("minecraft:oak_slab[bad=true]"), None);

                let wrapped =
                    sunrise_gameplay_common::game::mod_api_host::LevelTicks::new(&*host, ticks);
                assert_eq!(
                    wrapped.block_state_id("minecraft:oak_slab[waterlogged=true]"),
                    parsed
                );
                assert_eq!(
                    wrapped.block_state_default(mod_platform::__private::part_tests::block_key(
                        "minecraft:oak_slab"
                    )),
                    default
                );
                assert_eq!(wrapped.block_state_id("minecraft:oak_slab[bad=true]"), None);
                assert_eq!(stone.is_in_tag(wrapped.tags(), base_stone_tag), Some(true));
                assert_eq!(water_key.is_in_tag(wrapped.tags(), water_tag), Some(true));

                assert_eq!(host.state_string(stone).as_deref(), Some("minecraft:stone"));
                assert!(
                    host.state_string(slab)
                        .unwrap()
                        .contains("waterlogged=true")
                );
                assert_eq!(host.state_string(invalid), None);
                assert_eq!((host.min_y(), host.height()), bounds);
                assert_eq!(stone.is_in_tag(host.tags(), base_stone_tag), Some(true));
                assert_eq!(stone.is_in_tag(host.tags(), logs_tag), Some(false));
                assert_eq!(
                    host.fluid_state([1, 64, 1]),
                    api_fluid_state(EngineFluidState::WATER)
                );
                assert_eq!(host.fluid_state([16, 64, 1]), None);
                assert_eq!(
                    slab.fluid_state(&host.block_states()),
                    api_fluid_state(EngineFluidState::WATER)
                );
                assert_eq!(water_key.is_in_tag(host.tags(), water_tag), Some(true));
                assert_eq!(empty_key.is_in_tag(host.tags(), water_tag), Some(false));
            });
            {
                use mod_api::LevelHost;
                let (common, state) =
                    sunrise_gameplay_common::game::SideGame::common_and_state_mut(game);
                let items = match (
                    &common.server_items,
                    sunrise_gameplay_common::game::Side::world_tables(state),
                ) {
                    (Some(items), _) => &**items,
                    (None, Some(tables)) => &tables.items,
                    (None, None) => common.res.items(),
                };
                let hit = sunrise_gameplay_common::game::native_hooks::HitLevel(
                    sunrise_sim::health::DamageLevel::of(
                        &mut common.world,
                        common.pack.assets.registry,
                        Some(items),
                        Some(&common.pack.block_registries),
                        &common.damage_hooks,
                    ),
                );
                assert_eq!(
                    hit.block_state_id("minecraft:oak_slab[waterlogged=true]"),
                    parsed
                );
                assert_eq!(
                    hit.block_state_default(mod_platform::__private::part_tests::block_key(
                        "minecraft:oak_slab"
                    )),
                    default
                );
                assert_eq!(hit.block_state_id("minecraft:oak_slab[bad=true]"), None);

                assert_eq!(hit.state_string(stone).as_deref(), Some("minecraft:stone"));
                assert!(hit.state_string(slab).unwrap().contains("waterlogged=true"));
                assert_eq!(hit.state_string(invalid), None);
                assert_eq!((hit.min_y(), hit.height()), bounds);
                assert_eq!(stone_key.is_in_tag(hit.tags(), base_stone_tag), Some(true));
                assert_eq!(water_key.is_in_tag(hit.tags(), water_tag), Some(true));
            }
            {
                use mod_api::LevelHost;
                let hit = sunrise_gameplay_common::game::native_hooks::HitLevel(
                    sunrise_sim::health::DamageLevel::of(
                        &mut game.common.world,
                        game.common.pack.assets.registry,
                        None,
                        None,
                        &game.common.damage_hooks,
                    ),
                );
                assert_eq!(hit.block_state_id("minecraft:stone"), None);
                assert_eq!(
                    hit.block_state_default(mod_platform::__private::part_tests::block_key(
                        "minecraft:stone"
                    )),
                    None
                );
                assert_eq!(stone_key.is_in_tag(hit.tags(), logs_tag), None);
                assert_eq!(water_key.is_in_tag(hit.tags(), water_tag), None);
            }
            assert_eq!(game.world.chunks.len(), chunks, "nothing was loaded");
            game.options.execution_role =
                sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
            let host: &dyn mod_api::Host = game;
            let tags = host.tags();
            let oak_tag = tags
                .key(&mod_api::Identifier::parse("minecraft:oak_logs").unwrap())
                .unwrap()
                .typed();
            let item = host
                .item_key(&mod_api::Identifier::parse("minecraft:oak_log").unwrap())
                .unwrap();
            assert_eq!(tags.items(oak_tag), mod_api::TagMembers::Unavailable);
            assert_eq!(tags.is_in_tag(item, oak_tag), None);
            game.server.session_lock.0 = None;
            let _ = std::fs::remove_dir_all(dir);
        });
    }

    /// Sources and their flowing forms are one fluid, the empty fluid has no bucket and no
    /// pickup sound, and a name no fluid has answers nothing, through a mod's `Host`.
    #[test]
    #[ignore = "daily"]
    fn fluid_sameness_buckets_and_bucket_sounds_follow_vanilla() {
        with_game(|game| {
            use mod_api::HostBlockStates;
            let host: &dyn mod_api::Host = game;
            let states = host.block_states();
            let water = host.fluid_key("minecraft:water").unwrap();
            let flowing_water = host.fluid_key("minecraft:flowing_water").unwrap();
            let lava = host.fluid_key("lava").unwrap();
            let flowing_lava = host.fluid_key("flowing_lava").unwrap();
            let empty = host.fluid_key("minecraft:empty").unwrap();
            assert_eq!(host.fluid_key("nope:missing"), None);

            assert!(water.is_same(&states, flowing_water));
            assert!(flowing_lava.is_same(&states, lava));
            assert!(empty.is_same(&states, empty));
            assert!(!water.is_same(&states, lava));
            assert!(!water.is_same(&states, empty));

            assert_eq!(
                flowing_water
                    .bucket(&states)
                    .and_then(mod_api::__private::item_key_id),
                Some("minecraft:water_bucket")
            );
            assert_eq!(
                lava.bucket(&states)
                    .and_then(mod_api::__private::item_key_id),
                Some("minecraft:lava_bucket")
            );
            assert_eq!(empty.bucket(&states), None);

            assert_eq!(
                water.pickup_sound(&states),
                Some("minecraft:item.bucket.fill")
            );
            assert_eq!(
                flowing_lava.pickup_sound(&states),
                Some("minecraft:item.bucket.fill_lava")
            );
            assert_eq!(empty.pickup_sound(&states), None);

            assert_eq!(
                flowing_water.empty_sound(&states),
                Some("minecraft:item.bucket.empty")
            );
            assert_eq!(
                lava.empty_sound(&states),
                Some("minecraft:item.bucket.empty_lava")
            );
            assert_eq!(empty.empty_sound(&states), None);
        });
    }
}
