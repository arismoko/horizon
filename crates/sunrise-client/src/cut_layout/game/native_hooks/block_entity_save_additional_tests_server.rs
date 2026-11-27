#![cfg(test)]

#[cfg(test)]
pub(crate) mod tests {
    use crate::game::native_hooks::block_entity_save_additional_tests::tests::BreakObserver;
    use crate::game::native_hooks::block_entity_save_additional_tests::tests::Mod;
    use crate::game::native_hooks::block_entity_save_additional_tests::tests::SaveItemsMod;
    use crate::game::native_hooks::block_entity_save_additional_tests::tests::saved_stack;
    use mod_api::ModEntry;
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::sync::{Arc, Mutex};
    use sunrise_core::{
        nbt::{Compound, Limits, Tag, read_named},
        position::ChunkPos,
    };
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_level::{
        chunk::Chunk,
        containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields},
    };

    #[test]
    #[ignore = "daily"]
    fn break_only_serializes_admitted_loot_not_removal_notices() {
        use sunrise_command::args::GameMode;
        use sunrise_core::geometry::Vec3;
        use sunrise_items::items::ItemStack;
        use sunrise_sim::motion::PlayerMotion;

        for with_observers in [false, true] {
            let count = Arc::new(AtomicI32::new(11));
            let seen = Arc::new(Mutex::new(Vec::new()));
            let mut mods = vec![ModEntry::new("test", Box::new(Mod(count.clone())))];
            if with_observers {
                mods.push(ModEntry::new(
                    "test_observer",
                    Box::new(BreakObserver(seen.clone())),
                ));
            }
            mod_platform::testing::with_mods(mods.into(), || {
                with_game(|game| {
                    mod_api::__private::resolve_block_entity_types(
                        ["test:counter", "test:silent"],
                        |_| Some(()),
                    )
                    .unwrap();
                    game.options.dedicated = Some(Vec::new());
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    game.block_ticks
                        .add_empty_chunk(ChunkPos::new(0, 0))
                        .unwrap();
                    game.player =
                        Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 64.0, 4.5)).unwrap());
                    game.server.lan_acting_uuid = Some(123);
                    game.server.lan_acting_entity = Some(20);
                    game.server.lan_acting_permission = Some(0);
                    let place = |game: &mut sunrise_gameplay::game::ServerGame<'_>, name| {
                        let pos = (7, 64, 4);
                        let state = game.pack.assets.registry.default_state(name).unwrap();
                        game.world.set(pos.0, pos.1, pos.2, state).unwrap();
                        game.world.block_entities.insert(
                            pos,
                            BlockEntity::Mod(Box::new(ModBlockEntity::new(
                                "test:counter".into(),
                                ModBlockEntityFields {
                                    nbt: Compound::from([("count".into(), Tag::Int(7))]),
                                    ..Default::default()
                                },
                            ))),
                        );
                    };
                    for (mode, name, tool, serials) in [
                        (GameMode::Survival, "minecraft:dirt", None, 12),
                        (GameMode::Survival, "minecraft:diamond_ore", None, 12),
                        (
                            GameMode::Creative,
                            "minecraft:dirt",
                            Some(ItemStack::new("minecraft:stick", 1)),
                            12,
                        ),
                    ] {
                        game.set_mode(mode);
                        let selected = game.selected;
                        game.inventory[selected] = tool;
                        place(game, name);
                        assert!(game.break_block_as_player([7, 64, 4], None).unwrap());
                        assert_eq!(count.load(Ordering::Relaxed), serials, "{mode:?}, {name}");
                    }
                    let seen = seen.lock().unwrap();
                    if with_observers {
                        assert_eq!(
                            *seen,
                            [
                                ("removed", Some(Tag::Int(7))),
                                ("drops", Some(Tag::Int(7))),
                                ("removed", Some(Tag::Int(7))),
                                ("removed", Some(Tag::Int(7))),
                            ]
                        );
                    } else {
                        assert!(seen.is_empty());
                    }
                })
            });
        }
    }

    #[test]
    #[ignore = "daily"]
    fn save_edits_slotless_items_but_keeps_declared_slots_after_a_skipped_tick() {
        use {
            sunrise_gameplay::game::native_entity_refs::tests::fixture_root,
            sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
            sunrise_gameplay::game::native_entity_refs::tests::with_game_at,
        };
        const NAME: &str = "cut_layout::game::server_side::native_hooks_server::block_entity_save_additional_tests::tests::save_edits_slotless_items_but_keeps_declared_slots_after_a_skipped_tick";
        if run_fresh_cases(NAME, "SUNRISE_SAVE_ITEMS_CHILD", &["save"]) {
            return;
        }
        let mods: mod_api::Mods = vec![ModEntry::new("save_items", Box::new(SaveItemsMod))].into();
        mod_platform::testing::with_mods(mods.clone(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            let root = sunrise_gameplay_common::game::mods::content_root(
                &fixture_root(),
                &Default::default(),
            )
            .unwrap();
            with_game_at(root.prepared().to_path_buf(), |game| {
                game.options.dedicated = Some(Vec::new());
                let registry = game.pack.assets.registry;
                let mut chunk = Chunk::empty(0, 0);
                for (x, block) in [(1, "slotless"), (2, "slotless"), (3, "container")] {
                    let state = registry
                        .default_state(&format!("save_items:{block}"))
                        .unwrap();
                    chunk.set(x, 64, 1, state.0);
                    chunk.block_entities.push((
                        (x, 64, 1),
                        BlockEntity::Mod(Box::new(ModBlockEntity::new(
                            "save_items:holder".into(),
                            ModBlockEntityFields {
                                nbt: Compound::from([(
                                    "Items".into(),
                                    saved_stack("minecraft:stone"),
                                )]),
                                ..Default::default()
                            },
                        ))),
                    ));
                }
                game.server
                    .save
                    .0
                    .save(&[Arc::new(chunk)], registry, game.res.biomes())
                    .unwrap();
                let mut chunk = game
                    .server
                    .save
                    .0
                    .load(0, 0, registry, game.res.biomes())
                    .unwrap();
                game.world.install_chunk_block_entities(&mut chunk);
                game.world.chunks.insert((0, 0), Arc::new(chunk));
                assert!(game.server.streaming.set_forced(0, 0, true));
                game.update_chunk_tickets();
                game.tick_block_entities().unwrap();
                let stone = saved_stack("minecraft:stone");
                let expected = [
                    Some(saved_stack("minecraft:dirt")),
                    None,
                    Some(stone.clone()),
                ];
                let items_key = "Items".into();
                for (x, items) in (1..=3).zip(&expected) {
                    assert!(
                        !game
                            .world
                            .block_entities
                            .ticker_slots()
                            .iter()
                            .any(|(pos, _)| *pos == (x, 64, 1))
                    );
                    let Some(BlockEntity::Mod(row)) = game.world.block_entities.get(&(x, 64, 1))
                    else {
                        panic!("missing resident holder at {x}");
                    };
                    assert!(row.numeric_id().is_some());
                    assert!(row.tick_kind().is_none());
                    if x == 3 {
                        assert_eq!(row.nbt.get(&items_key), None);
                        let [Some(stack)] = row.items.as_slice() else {
                            panic!("expected one modeled stack at {x}");
                        };
                        assert_eq!((stack.item.as_ref(), stack.count), ("minecraft:stone", 1));
                    } else {
                        assert_eq!(row.nbt.get(&items_key), Some(&stone));
                        assert!(row.items.is_empty());
                    }
                    let captured = game.block_entity_nbt([x, 64, 1]).unwrap();
                    assert_eq!(captured.get("Items"), items.as_ref());
                }

                let capture = game.with_entities([&game.world.chunks[&(0, 0)]]);
                game.server
                    .save
                    .0
                    .save_later(&capture, registry, game.res.biomes())
                    .unwrap();
                game.server.save.0.flush().unwrap();
                let reloaded = game
                    .server
                    .save
                    .0
                    .load(0, 0, registry, game.res.biomes())
                    .unwrap();
                for (x, items) in (1..=3).zip(&expected) {
                    let (_, BlockEntity::Mod(row)) = reloaded
                        .block_entities
                        .iter()
                        .find(|(pos, _)| *pos == (x, 64, 1))
                        .unwrap()
                    else {
                        panic!("missing saved holder at {x}");
                    };
                    if x == 3 {
                        assert_eq!(row.nbt.get(&items_key), None);
                        let [Some(stack)] = row.items.as_slice() else {
                            panic!("expected one saved stack at {x}");
                        };
                        assert_eq!((stack.item.as_ref(), stack.count), ("minecraft:stone", 1));
                    } else {
                        assert_eq!(row.nbt.get(&items_key), items.as_ref());
                        assert!(row.items.is_empty());
                    }
                }
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn structure_capture_refuses_lossy_listener_keys_and_strings() {
        let calls = Arc::new(AtomicI32::new(0));
        let mods = vec![ModEntry::new("test", Box::new(Mod(calls.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                mod_api::__private::resolve_block_entity_types(
                    ["test:counter", "test:silent"],
                    |_| Some(()),
                )
                .unwrap();
                let scratch = tempfile::tempdir().unwrap();
                game.server.world_root = scratch.path().to_path_buf();
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let dirt = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:dirt")
                    .unwrap();
                for (x, selector) in [(5, -2), (6, -3)] {
                    game.world.set(x, 64, 1, dirt).unwrap();
                    game.world.block_entities.insert(
                        (x, 64, 1),
                        BlockEntity::Mod(Box::new(ModBlockEntity::new(
                            "test:counter".into(),
                            ModBlockEntityFields {
                                nbt: Compound::from([
                                    ("".into(), Tag::Int(123)),
                                    ("count".into(), Tag::Int(selector)),
                                    ("values".into(), Tag::List(vec![Tag::String("keep".into())])),
                                ]),
                                ..Default::default()
                            },
                        ))),
                    );
                }
                assert!(game.save_structure_area(
                    "test:payload",
                    [5, 64, 1],
                    [2, 1, 1],
                    true,
                    "",
                    true,
                    false
                ));
                let bytes =
                    std::fs::read(scratch.path().join("generated/test/structures/payload.nbt"))
                        .unwrap();
                let limits = Limits::templates();
                let raw = sunrise_world::compression::Compression::Gzip
                    .decode(&bytes, limits.max_bytes)
                    .unwrap();
                let (saved, _) = read_named(&raw, limits).unwrap();
                let Some(Tag::List(blocks)) = saved.tag.get("blocks") else {
                    panic!("structure blocks missing")
                };
                assert_eq!(blocks.len(), 2);
                for block in blocks {
                    let nbt = block.get("nbt").unwrap();
                    assert_eq!(nbt.get(""), Some(&Tag::Int(123)));
                    assert_eq!(
                        nbt.get("values"),
                        Some(&Tag::List(vec![Tag::String("keep".into())]))
                    );
                }
                assert_eq!(calls.load(Ordering::Relaxed), 2);
            });
        });
    }
}
