#![cfg(test)]
use sunrise_core::nbt::Tag;
use sunrise_level::containers::BlockEntity;
use sunrise_sim::save;
#[cfg(test)]
pub(crate) mod tests {
    use sunrise_gameplay_common::game::block_items::BlockItemsGameExt1;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    #[test]
    #[ignore = "daily"]
    fn placed_sculk_item_data_survives_pending_placement() {
        use std::sync::Arc;
        use sunrise_core::nbt::Tag;
        use sunrise_gameplay::game::native_entity_refs::tests::with_game;
        use sunrise_items::items::ItemStack;
        use sunrise_level::chunk::Chunk;

        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.sculk_prepare();
            for (block, field, value) in [
                ("minecraft:sculk_sensor", "last_vibration_frequency", 6),
                (
                    "minecraft:calibrated_sculk_sensor",
                    "last_vibration_frequency",
                    11,
                ),
                ("minecraft:sculk_shrieker", "warning_level", 2),
            ] {
                let state = game.pack.assets.registry.default_state(block).unwrap();
                let key = (2, 230, 11);
                game.world.block_entities.remove(&key);
                game.world.set(key.0, key.1, key.2, state).unwrap();
                let mut stack = ItemStack::new(block, 1);
                let patch = Tag::compound([(
                    "minecraft:block_entity_data",
                    Tag::compound([("id", Tag::String(block.into())), (field, Tag::Int(value))]),
                )]);
                (stack.damage, stack.extra) =
                    sunrise_items::item_data::decode_components(Some(&patch));
                game.apply_placed_item([2, 230, 11], &stack, false);
                game.sculk_sync_index();
                let saved = game.block_entity_nbt([2, 230, 11]).unwrap();
                assert_eq!(saved.get(field), Some(&Tag::Int(value)), "{block}");
            }
        });
    }

    /// A mod block entity's data rides its item through the game: Ctrl+Pick
    /// Block (`addBlockDataToItem`) puts it on the stack, placing the stack
    /// (`BlockItem.updateCustomBlockEntityTag`) loads it into the new block
    /// entity, and `/give` takes the declared type and refuses an unknown one.
    #[test]
    #[ignore = "daily"]
    fn a_mod_block_entity_rides_its_item_through_the_game() {
        use mod_api::{self, *};
        use std::sync::Arc;
        use sunrise_gameplay_common::game::block_items::components;
        use sunrise_level::{chunk::Chunk, containers::BlockEntity};
        use {
            sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
            sunrise_gameplay::game::native_entity_refs::tests::with_game_at,
        };

        const CHILD: &str = "SUNRISE_MOD_BLOCK_ENTITY_ITEM_CHILD";
        if run_fresh_cases(
            "cut_layout::game::server_side::block_items_server::tests::a_mod_block_entity_rides_its_item_through_the_game",
            CHILD,
            &["game"],
        ) {
            return;
        }
        const CRATE: &str = "item_test:crate";
        struct Crate;
        #[mod_api::sunrise_part]
        impl BlockPart for Crate {}
        #[mod_api::sunrise_part]
        impl BlockEntityPart for Crate {
            fn block_entity(
                &self,
                _: LevelSide<'_>,
                _: &mut BlockEntityEvent<'_>,
            ) -> Option<Compound> {
                Some(Compound::from([("Amount".into(), Tag::Int(0))]))
            }
        }
        struct TestMod;
        #[mod_api::__subscriptions]
        impl Hooks for TestMod {
            fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let kind = registrar
                    .block_entity_type("crate")
                    .blocks(&[CRATE])
                    .build();
                registrar
                    .block_entity_part(&kind, Crate)
                    .expect("native type");
            }
            fn blocks(&self, _context: &mod_api::RegistrationContext, event: &mut BlocksEvent<'_>) {
                event
                    .register(mod_api::Block::new(CRATE, "minecraft:white_banner"))
                    .unwrap();
            }
        }
        let mods: Mods = Arc::from([ModEntry::new("item_test", Box::new(TestMod))]);
        mod_platform::__private::install(mods.clone());
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root = sunrise_gameplay_common::game::mods::content_root(&base, &Default::default())
            .unwrap()
            .prepared()
            .to_path_buf();
        with_game_at(root, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level(game.dim.id()).unwrap();
            let state = game.pack.assets.registry.default_state(CRATE).unwrap();
            for x in [1, 2] {
                assert_eq!(
                    game.level_set_block(level, [x, 64, 0], state, BlockUpdateFlags::NONE),
                    Ok(true)
                );
            }
            let Some(BlockEntity::Mod(first)) = game.world.block_entities.get_mut(&(1, 64, 0))
            else {
                panic!("no mod block entity");
            };
            first.nbt.insert("Amount".into(), Tag::Int(2000));
            // A component it was placed with (`BlockEntity.components`).
            let name =
                Compound::from([("minecraft:custom_name".into(), Tag::String("Box".into()))]);
            first
                .nbt
                .insert("components".into(), Tag::Compound(name.clone()));

            let picked = game.picked_stack([1, 64, 0], true).expect("picked");
            assert_eq!(&*picked.item, CRATE);
            let patch = components(&picked);
            let data = patch
                .get(&"minecraft:block_entity_data".into())
                .expect("data");
            assert_eq!(data.get("id"), Some(&Tag::String(CRATE.into())));
            assert_eq!(data.get("Amount"), Some(&Tag::Int(2000)));
            assert_eq!(data.get("x"), None);
            assert_eq!(data.get("components"), None);
            assert_eq!(
                patch.get(&"minecraft:custom_name".into()),
                Some(&Tag::String("Box".into()))
            );
            // Without Ctrl the item carries nothing.
            let plain = game.picked_stack([1, 64, 0], false).expect("picked");
            assert!(!components(&plain).contains_key(&"minecraft:block_entity_data".into()));

            game.apply_placed_item([2, 64, 0], &picked, false);
            let Some(BlockEntity::Mod(second)) = game.world.block_entities.get(&(2, 64, 0)) else {
                panic!("placing replaced the mod block entity");
            };
            assert_eq!(second.nbt.get(&"Amount".into()), Some(&Tag::Int(2000)));
            assert_eq!(
                second.nbt.get(&"components".into()),
                Some(&Tag::Compound(name))
            );

            // The game's own command tree and registries, as a world opens
            // them; this headless game has no player for the give to reach, so
            // the parse, where the data's type is checked, is what runs.
            game.build_registries();
            game.ui.registries.permission = 4;
            let commands = game.ui.dispatcher.0.clone();
            let give = |kind: &str| {
                let command =
                    format!("give @s {CRATE}[block_entity_data={{id:\"{kind}\",Amount:7}}]");
                commands
                    .parse(&command, &game.ui.registries)
                    .executable()
                    .map(|_| ())
                    .map_err(|e| e.message.plain())
            };
            assert_eq!(give(CRATE), Ok(()));
            assert!(
                give("item_test:nope")
                    .unwrap_err()
                    .contains("item_test:nope")
            );
        });
    }

    /// A content block's `pick_item` is what pick-block takes (`getCloneItemStack`),
    /// whatever the block's own item is: a crop with no item of its own picks its seed,
    /// a block with an item picks another, and one that declares nothing keeps
    /// picking its own item, or nothing when it has none.
    #[test]
    #[ignore = "daily"]
    fn a_content_blocks_pick_item_is_what_pick_block_takes() {
        use mod_api::{self, *};
        use std::sync::Arc;
        use sunrise_level::chunk::Chunk;
        use {
            sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
            sunrise_gameplay::game::native_entity_refs::tests::with_game_at,
        };

        const CHILD: &str = "SUNRISE_MOD_PICK_ITEM_CHILD";
        if run_fresh_cases(
            "cut_layout::game::server_side::block_items_server::tests::a_content_blocks_pick_item_is_what_pick_block_takes",
            CHILD,
            &["game"],
        ) {
            return;
        }
        struct TestMod;
        #[mod_api::__subscriptions]
        impl Hooks for TestMod {
            fn blocks(&self, _context: &mod_api::RegistrationContext, event: &mut BlocksEvent<'_>) {
                for (id, item, pick_item) in [
                    ("pick_test:crop", false, Some("minecraft:wheat_seeds")),
                    ("pick_test:gold", true, Some("minecraft:diamond")),
                    ("pick_test:plain", true, None),
                    ("pick_test:ghost", false, None),
                ] {
                    let mut block = mod_api::Block::new(id, "minecraft:stone");
                    block.item = item;
                    block.pick_item = pick_item.map(Into::into);
                    event.register(block).unwrap();
                }
            }
        }
        let mods: Mods = Arc::from([ModEntry::new("pick_test", Box::new(TestMod))]);
        mod_platform::__private::install(mods.clone());
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root = sunrise_gameplay_common::game::mods::content_root(&base, &Default::default())
            .unwrap()
            .prepared()
            .to_path_buf();
        with_game_at(root, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level(game.dim.id()).unwrap();
            let blocks = [
                "pick_test:crop",
                "pick_test:gold",
                "pick_test:plain",
                "pick_test:ghost",
            ];
            for (x, block) in (1..).zip(blocks) {
                let state = game.pack.assets.registry.default_state(block).unwrap();
                assert!(
                    game.level_set_block(level, [x, 64, 0], state, BlockUpdateFlags::NONE)
                        .unwrap(),
                    "{block} at {x}"
                );
            }
            let picked = |x: i32| {
                game.picked_stack([x, 64, 0], false)
                    .map(|stack| stack.item.to_string())
            };
            assert_eq!(picked(1).as_deref(), Some("minecraft:wheat_seeds"));
            assert_eq!(picked(2).as_deref(), Some("minecraft:diamond"));
            assert_eq!(picked(3).as_deref(), Some("pick_test:plain"));
            assert_eq!(picked(4), None);
        });
    }
}

#[cfg(test)]
pub(crate) mod plain_name_overlay_tests {
    use super::*;
    use sunrise_core::position::BlockPos;
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;

    /// BaseContainerBlockEntity's removed external name must not be restored
    /// from the original saved source by the chunk writer.
    #[test]
    #[ignore = "daily"]
    fn removed_actual_plain_name_stays_absent_in_saved_and_reloaded_model() {
        with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            // This fixture has no streaming workers; provide the resident chunk
            // as well as the block-tick chunk before forceload asks for it.
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            game.block_ticks
                .add_empty_chunk(BlockPos::new(0, 0, 0).chunk())
                .unwrap();
            assert!(game.run_command("forceload add 0 0"));
            assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
            let clock = game.world.game_time;
            let mut failures = Vec::new();
            for (x, kind, lock) in [(2, "chest", ""), (5, "barrel", ",lock:{items:diamond}")] {
                assert!(game.run_command(&format!("setblock {x} 200 4 {kind}{{CustomName:Original,Items:[{{Slot:0b,id:diamond,count:3}}]{lock}}}")));
                let pos = (x, 200, 4);
                assert!(game.world.loaded(x, 4));
                assert!(game.world.block_names.contains_key(&pos));
                assert_eq!(
                    game.block_entity_nbt([x, 200, 4])
                        .unwrap()
                        .get("CustomName"),
                    Some(&Tag::String("Original".into()))
                );
                let Some(BlockEntity::Chest(slots)) = game.world.block_entities.get(&pos) else {
                    panic!("actual container")
                };
                assert_eq!(&*slots[0].as_ref().unwrap().item, "minecraft:diamond");
                assert_eq!(slots[0].as_ref().unwrap().count, 3);
                if x == 5 {
                    assert!(game.world.block_lock(pos).is_some());
                    let chunk = game.world.chunks.get(&(0, 0)).unwrap();
                    let source = &chunk
                        .saved
                        .as_ref()
                        .unwrap()
                        .block_entity_sources
                        .iter()
                        .find(|(p, _)| *p == pos)
                        .unwrap()
                        .1;
                    assert_eq!(
                        source.get("CustomName"),
                        Some(&Tag::String("Original".into()))
                    );
                }
                assert!(game.run_command(&format!("data remove block {x} 200 4 CustomName")));
                assert!(!game.world.block_names.contains_key(&pos));
                assert!(
                    game.block_entity_nbt([x, 200, 4])
                        .unwrap()
                        .get("CustomName")
                        .is_none()
                );
                let names = game.world.block_names.clone();
                let resident = game.world.chunks.get(&(0, 0)).unwrap();
                if x == 5 {
                    let source = &resident
                        .saved
                        .as_ref()
                        .unwrap()
                        .block_entity_sources
                        .iter()
                        .find(|(p, _)| *p == pos)
                        .unwrap()
                        .1;
                    assert_eq!(
                        source.get("CustomName"),
                        Some(&Tag::String("Original".into())),
                        "real retained source witness"
                    );
                    assert!(source.get("lock").is_some());
                }
                let snapshot = game.with_entities([resident]).remove(0);
                let encoded = save::encode(
                    &snapshot,
                    game.pack.assets.registry,
                    game.res.biomes(),
                    &Default::default(),
                );
                let Tag::List(entries) = encoded.tag.get("block_entities").unwrap() else {
                    panic!("actual saved entries")
                };
                let saved = entries
                    .iter()
                    .find(|e| save::number(e.get("x")) == Some(i64::from(x)))
                    .unwrap();
                let Tag::List(items) = saved.get("Items").unwrap() else {
                    panic!("actual saved contents")
                };
                assert_eq!(save::number(items[0].get("count")), Some(3));
                if x == 5 {
                    assert!(saved.get("lock").is_some());
                }
                let reloaded =
                    save::decode(&encoded, game.pack.assets.registry, game.res.biomes()).unwrap();
                if saved.get("CustomName").is_some()
                    || reloaded.block_names.iter().any(|(p, _)| *p == pos)
                {
                    failures.push((pos, saved.clone()));
                }
                assert_eq!(
                    game.world.block_names, names,
                    "save is read-only for the name model"
                );
            }
            assert_eq!(game.world.game_time, clock);
            assert!(
                failures.is_empty(),
                "removed actual plain name reappeared from source: {failures:?}"
            );
        });
    }
}

#[cfg(test)]
pub(crate) mod styled_name_model_tests {
    use super::*;
    use sunrise_core::position::BlockPos;
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;

    fn parse(input: &str) -> Tag {
        let (tag, used) = sunrise_core::nbt::parse_snbt(input).unwrap();
        assert_eq!(used, input.len());
        tag
    }
    fn chunk_saved(game: &sunrise_gameplay::game::ServerGame<'_>, pos: (i32, i32, i32)) -> Tag {
        let resident = game.world.chunks.get(&(0, 0)).unwrap();
        let snapshot = game.with_entities([resident]).remove(0);
        let encoded = save::encode(
            &snapshot,
            game.pack.assets.registry,
            game.res.biomes(),
            &Default::default(),
        );
        let Tag::List(entities) = encoded.tag.get("block_entities").unwrap() else {
            panic!("actual chunk entities")
        };
        let saved = entities
            .iter()
            .find(|tag| {
                save::number(tag.get("x")) == Some(i64::from(pos.0))
                    && save::number(tag.get("y")) == Some(i64::from(pos.1))
                    && save::number(tag.get("z")) == Some(i64::from(pos.2))
            })
            .expect("actual saved block entity")
            .clone();
        let decoded = save::decode(&encoded, game.pack.assets.registry, game.res.biomes()).unwrap();
        assert!(decoded.block_entities.iter().any(|(at, _)| *at == pos));
        saved
    }
    fn views(
        game: &sunrise_gameplay::game::ServerGame<'_>,
        pos: (i32, i32, i32),
        want: Option<&Tag>,
        missing: &mut Vec<String>,
    ) {
        assert!(game.world.loaded(pos.0, pos.2));
        let Some(BlockEntity::Chest(slots)) = game.world.block_entities.get(&pos) else {
            panic!("actual modeled container")
        };
        assert_eq!(slots[0].as_ref().unwrap().count, 3);
        assert_eq!(slots[26].as_ref().unwrap().count, 5);
        assert_eq!(&*slots[0].as_ref().unwrap().item, "minecraft:diamond");
        assert_eq!(&*slots[26].as_ref().unwrap().item, "minecraft:apple");
        let no_raw = game.saved_block_entity(pos, false).unwrap();
        let command = game.block_entity_nbt([pos.0, pos.1, pos.2]).unwrap();
        let chunk = chunk_saved(game, pos);
        for (route, saved) in [
            ("modeled/pick", no_raw),
            ("command", command),
            ("chunk", chunk),
        ] {
            assert_eq!(save::number(saved.get("x")), Some(i64::from(pos.0)));
            assert_eq!(save::number(saved.get("y")), Some(i64::from(pos.1)));
            assert_eq!(save::number(saved.get("z")), Some(i64::from(pos.2)));
            if saved.get("CustomName") != want {
                missing.push(format!(
                    "{route} {pos:?}: {:?} != {want:?}",
                    saved.get("CustomName")
                ));
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn actual_container_names_load_place_mutate_and_save_without_raw() {
        with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            // The name round trip needs a resident chunk, not terrain generation.
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            game.block_ticks
                .add_empty_chunk(BlockPos::new(0, 0, 0).chunk())
                .unwrap();
            assert!(game.run_command("forceload add 0 0"));
            assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
            assert!(game.world.loaded(4, 4));
            let clock = game.world.game_time;
            let contents = "Items:[{Slot:0b,id:\"minecraft:diamond\",count:3,components:{\"minecraft:custom_data\":{h210:1}}},{Slot:26b,id:\"minecraft:apple\",count:5}]";
            let styled =
                "{text:\"Styled\",color:\"aqua\",bold:0b,extra:[{text:\"Box\",italic:1b}]}";
            let translated = "{translate:\"container.chest\",color:\"red\",italic:0b}";
            let changed = "{text:\"Changed\",color:\"gold\",italic:1b}";
            let mut missing = Vec::new();
            for (x, kind, name) in [
                (2, "chest", Some("\"Plain\"")),
                (5, "barrel", Some(styled)),
                (8, "shulker_box", Some(translated)),
                (11, "chest", None),
            ] {
                let extra = name.map_or(String::new(), |n| format!(",CustomName:{n}"));
                assert!(game.run_command(&format!(
                    "setblock {x} 200 4 minecraft:{kind}{{{contents}{extra}}}"
                )));
                assert_eq!(
                    &*game.res.facts().get(game.world.block(x, 200, 4)).name,
                    format!("minecraft:{kind}")
                );
                let want = name.map(parse);
                if x == 2 {
                    assert_eq!(
                        game.block_entity_nbt([x, 200, 4])
                            .unwrap()
                            .get("CustomName"),
                        want.as_ref(),
                        "plain-name admission"
                    );
                }
                game.world.forget_block_entity_source((x, 200, 4));
                assert!(
                    game.world
                        .chunks
                        .get(&(0, 0))
                        .unwrap()
                        .saved
                        .as_ref()
                        .is_none_or(|s| !s
                            .block_entity_sources
                            .iter()
                            .any(|(p, _)| *p == (x, 200, 4)))
                );
                views(game, (x, 200, 4), want.as_ref(), &mut missing);
            }
            // The real source snapshot stays Old while an actual data command changes
            // only the live name. It must not be used as the saved answer.
            assert!(game.run_command(&format!("setblock 5 200 8 barrel{{{contents},CustomName:\"Old\",lock:{{items:\"minecraft:diamond\"}}}}")));
            let stale_pos = (5, 200, 8);
            assert!(game.world.block_lock(stale_pos).is_some());
            assert!(game.run_command(&format!(
                "data merge block 5 200 8 {{CustomName:{changed}}}"
            )));
            let source = game
                .world
                .chunks
                .get(&(0, 0))
                .unwrap()
                .saved
                .as_ref()
                .unwrap()
                .block_entity_sources
                .iter()
                .find(|(p, _)| *p == stale_pos)
                .unwrap();
            assert_eq!(
                source.1.get("CustomName"),
                Some(&parse("\"Old\"")),
                "actual stale source admission"
            );
            let want = parse(changed);
            views(game, stale_pos, Some(&want), &mut missing);
            game.world.forget_block_entity_source(stale_pos);
            views(game, stale_pos, Some(&want), &mut missing);
            if !game.run_command("data remove block 5 200 8 CustomName") {
                missing.push("actual styled-name remove had no modeled source".into());
            }
            views(game, stale_pos, None, &mut missing);
            // The real Game placement helper consumes the source stack components;
            // connected-owner consumption/ACK is independently covered at runtime.
            assert!(game.run_command(&format!("setblock 8 200 11 chest{{{contents}}}")));
            let source = parse(&format!(
                "{{id:\"minecraft:chest\",count:1,components:{{\"minecraft:custom_name\":{styled},\"minecraft:container\":[{{slot:0,item:{{id:\"minecraft:diamond\",count:3,components:{{\"minecraft:custom_data\":{{h210:1}}}}}}}},{{slot:26,item:{{id:\"minecraft:apple\",count:5}}}}]}}}}"
            ));
            let stack = save::item_from_nbt(&source).expect("real item codec source");
            assert_eq!(stack.count, 1);
            assert_eq!(
                stack.data().unwrap().custom_name.as_ref(),
                Some(&parse(styled))
            );
            game.apply_placed_item([8, 200, 11], &stack, false);
            views(game, (8, 200, 11), Some(&parse(styled)), &mut missing);
            assert_eq!(stack.count, 1, "placement-helper source is unchanged");
            assert!(game.run_command("clone 8 200 4 8 200 4 11 200 8 replace"));
            views(game, (11, 200, 8), Some(&parse(translated)), &mut missing);
            assert_eq!(game.world.game_time, clock);
            assert!(
                missing.is_empty(),
                "actual styled-name model/save mismatches: {missing:?}"
            );
        });
    }

    /// A player's pick-block read gives what a pick takes: the block's own item (a crop's
    /// seed), a block Part's answer in its place, and nothing for air, an unloaded position
    /// or a player who is not here.
    #[test]
    #[ignore = "daily"]
    fn the_clone_item_stack_read_gives_the_pick_answer() {
        use mod_api::{BlockPart, CloneItemStackEvent, Hooks, Host, ItemStack, Registrar};
        use std::sync::Arc;
        use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;
        use sunrise_gameplay::game::native_guest_turn_tests::with_guests;
        use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
        use sunrise_level::chunk::Chunk;

        if run_fresh_cases(
            "cut_layout::game::server_side::block_items_server::styled_name_model_tests::the_clone_item_stack_read_gives_the_pick_answer",
            "SUNRISE_CLONE_STACK_READ_CHILD",
            &["read"],
        ) {
            return;
        }
        /// Stone picks as a diamond, except with its data.
        struct Diamond;
        #[mod_api::sunrise_part]
        impl BlockPart for Diamond {
            fn clone_item_stack(
                &self,
                _: &dyn Host,
                event: &mut CloneItemStackEvent<'_>,
            ) -> Option<ItemStack> {
                (!event.include_data).then(|| {
                    ItemStack::new(
                        mod_api::__private::item_key("minecraft:diamond").unwrap(),
                        1,
                    )
                    .unwrap()
                })
            }
        }
        struct Mod;
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let stone = r.block_key("minecraft:stone");
                r.block_part(stone, Diamond);
            }
        }
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("picker", Box::new(Mod))].into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            with_guests(|game| {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let registry = game.common.pack.assets.registry;
                let state = |name| registry.default_state(name).unwrap();
                game.world.set(0, 64, 0, state("minecraft:wheat")).unwrap();
                game.world.set(1, 64, 0, state("minecraft:stone")).unwrap();
                game.world.set(3, 64, 0, state("minecraft:chest")).unwrap();
                let player = game.player(12).unwrap();
                let level = player.level(&*game).unwrap();
                let pick = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                            player: mod_api::PlayerRef,
                            pos: [i32; 3],
                            include_data: bool| {
                    let stack = player.clone_item_stack(game, level, pos, include_data)?;
                    let item = mod_api::__private::item_key_id(stack.item()).unwrap();
                    Some((item.to_owned(), stack.count()))
                };
                let item = |id: &str| Some((id.to_owned(), 1));
                assert_eq!(
                    pick(game, player, [0, 64, 0], false),
                    item("minecraft:wheat_seeds")
                );
                assert_eq!(
                    pick(game, player, [1, 64, 0], false),
                    item("minecraft:diamond")
                );
                assert_eq!(
                    pick(game, player, [1, 64, 0], true),
                    item("minecraft:stone")
                );
                // A chest's items and name stay off the read's stack with data:
                // addBlockDataToItem comes after getCloneItemStack.
                let mut items = vec![None; 27];
                items[0] = Some(sunrise_items::items::ItemStack::new("minecraft:stone", 5));
                game.world.block_entities.insert(
                    (3, 64, 0),
                    sunrise_level::containers::BlockEntity::Chest(items),
                );
                game.world
                    .block_names
                    .insert((3, 64, 0), sunrise_core::nbt::Tag::String("Loot".into()));
                let chest = player
                    .clone_item_stack(game, level, [3, 64, 0], true)
                    .unwrap();
                assert_eq!(
                    mod_api::__private::item_key_id(chest.item()),
                    Some("minecraft:chest")
                );
                assert!(chest.components().is_none(), "no block data");
                assert_eq!(pick(game, player, [2, 64, 0], false), None, "air");
                assert_eq!(pick(game, player, [4000, 64, 0], false), None, "unloaded");
                let gone = mod_api::__private::game::player_ref(
                    mod_api::EntityRef::new(player.entity().epoch(), player.entity().id() + 100),
                    player.uuid(),
                );
                assert_eq!(pick(game, gone, [0, 64, 0], false), None, "stale");
            });
        });
    }
}

use sunrise_gameplay_common::game::block_items::BlockItemsGameExt1 as _;

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
