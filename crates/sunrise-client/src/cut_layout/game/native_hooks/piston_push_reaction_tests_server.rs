use crate::game::native_hooks::piston_push_reaction_tests::PayloadUser;
use crate::game::native_hooks::piston_push_reaction_tests::state;
use mod_api::*;
use std::{collections::HashMap, sync::Arc};
use sunrise_core::{
    geometry::Direction,
    position::{BlockPos, ChunkPos},
};
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_world::level::LevelAccess;
use {
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game_role,
    sunrise_gameplay_common::game::execution_role::ExecutionRole,
};

#[test]
#[ignore = "daily"]
fn piston_push_reaction_position_state_opens_ticks_and_saves() {
    const NAME: &str = "cut_layout::game::server_side::native_hooks_server::piston_push_reaction_tests::piston_push_reaction_position_state_opens_ticks_and_saves";
    const FLAG: &str = "SUNRISE_PISTON_POSITION_CASE";
    if run_fresh_cases(
        NAME,
        FLAG,
        &[
            "chest",
            "furnace",
            "hopper",
            "client",
            "loot",
            "mixed-loot",
            "same-scope",
        ],
    ) {
        return;
    }
    let case = std::env::var(FLAG).unwrap();
    let client = case == "client";
    let asks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mods: Mods = vec![ModEntry::new(
        "payload",
        Box::new(PayloadUser {
            reaction: PushReaction::Push,
            carry: Some(true),
            asks,
        }),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        if client {
            #[cfg(feature = "client")]
            crate::game::client_side::native_hooks_client::piston_push_reaction_tests::run_client_piston_case(
                root,
            );
            #[cfg(not(feature = "client"))]
            let _ = root;
            return;
        }
        with_game_role(root, ExecutionRole::IntegratedServer, |game| {
            use sunrise_core::nbt::Tag;
            use sunrise_items::items::ItemStack;
            use sunrise_level::{
                chunk::{Chunk, PendingContainerLoot},
                containers::{BlockEntity, Furnace, FurnaceKind},
            };
            game.options.dedicated = Some(vec![]);
            let from = (3, 64, 2);
            let to = (4, 64, 2);
            let registry = game.pack.assets.registry;
            let name = match case.as_str() {
                "furnace" => "minecraft:furnace",
                "hopper" | "same-scope" => "minecraft:hopper",
                _ => "minecraft:chest",
            };
            let block = registry.default_state(name).unwrap();
            let piston = state(registry, "minecraft:piston", &[("facing", "east")]);
            let power = registry.default_state("minecraft:redstone_block").unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            game.with_level(|_, level, _| {
                level.set_raw(BlockPos::new(2, 64, 2), piston);
                level.set_raw(BlockPos::new(1, 64, 2), power);
                level.set_raw(BlockPos::new(3, 64, 2), block);
            });
            let mut slots = vec![
                None;
                if case == "hopper" || case == "same-scope" {
                    5
                } else {
                    27
                }
            ];
            if case != "loot" {
                slots[4] = Some(ItemStack::new("minecraft:diamond", 7));
            }
            let entity = if case == "furnace" {
                BlockEntity::Furnace(Furnace {
                    kind: FurnaceKind::Furnace,
                    items: [None, None, None],
                    lit_remaining: 100,
                    lit_total: 100,
                    cooking_timer: 0,
                    cooking_total: 200,
                    speed: 1.0,
                    recipes_used: HashMap::new(),
                })
            } else {
                BlockEntity::Chest(slots.clone())
            };
            game.world.block_entities.insert(from, entity);
            let old_ticker = if case == "furnace" {
                let states = game.ticking_states();
                game.world.block_entities.set_states(states);
                let world = &mut game.world;
                let chunks = &world.chunks;
                let tickers = world.block_entities.tickers(|p| {
                    chunks
                        .get(&(p.0 >> 4, p.2 >> 4))
                        .unwrap()
                        .block(p.0 & 15, p.1, p.2 & 15)
                        .0
                });
                Some(*tickers.iter().find(|(p, _)| *p == from).unwrap())
            } else {
                None
            };
            let custom = Tag::String("Moving chest".into());
            game.world.block_names.insert(from, custom.clone());
            let lock = Tag::compound([("items", Tag::String("minecraft:diamond".into()))]);
            game.world.set_block_lock(
                from,
                &Tag::compound([
                    ("id", Tag::String(name.into())),
                    ("x", Tag::Int(3)),
                    ("y", Tag::Int(64)),
                    ("z", Tag::Int(2)),
                    ("lock", lock.clone()),
                    (
                        "components",
                        Tag::compound([(
                            "minecraft:custom_data",
                            Tag::compound([("carry_test", Tag::Int(11))]),
                        )]),
                    ),
                ]),
            );
            game.world.set_container_stack_limit(from, 13);
            if case == "loot" || case == "mixed-loot" {
                let files = [("loot_table/test:carry".into(), vec![br#"{"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]}"#.to_vec()])];
                let loot = sunrise_world::loot::LootData::load(
                    &game.options.resources,
                    &sunrise_world::loot::PackOverlay {
                        files: &files,
                        ..sunrise_world::loot::PackOverlay::default()
                    },
                    game.res.item_defaults(),
                )
                .unwrap();
                game.server.server_loot.0.set(Some(Arc::new(loot)));
                let mut extras = game.world.block_entity_extras(from);
                extras.pending_loot = Some(PendingContainerLoot {
                    table: Tag::String("test:carry".into()),
                    seed: Some(Tag::Long(37)),
                });
                game.world.set_block_entity_extras(from, extras);
            }
            if case == "hopper" || case == "same-scope" {
                game.server.hoppers.insert(
                    from,
                    sunrise_gameplay_common::hoppers::HopperState::with_cooldown(40),
                );
            }
            game.server.lid_openers.insert(from, 2);
            game.world.chest_openers.insert(from, (2, 4.0));
            game.server.be_anims.bells.insert(
                [3, 64, 2],
                sunrise_level::block_entity_ticks::Bell::default(),
            );
            game.container = Some(from);
            game.server.open_container_slots = Some((from, 27));
            if case == "same-scope" {
                let far = (5, 64, 2);
                let air = registry.default_state("minecraft:air").unwrap();
                game.with_level(|_, level, _| {
                    level.carry_mod_block_entity(BlockPos::new(3, 64, 2), BlockPos::new(4, 64, 2));
                    level.set_raw(BlockPos::new(3, 64, 2), air);
                    level.set_raw(BlockPos::new(4, 64, 2), block);
                    assert!(level.land_carried_block_entity(BlockPos::new(4, 64, 2)));
                    level.carry_mod_block_entity(BlockPos::new(4, 64, 2), BlockPos::new(5, 64, 2));
                    level.set_raw(BlockPos::new(4, 64, 2), air);
                    level.set_raw(BlockPos::new(5, 64, 2), block);
                    assert!(level.land_carried_block_entity(BlockPos::new(5, 64, 2)));
                });
                assert_eq!(game.hopper_cooldown(far), 40);
                assert!(
                    !game.server.hoppers.contains_key(&from)
                        && !game.server.hoppers.contains_key(&to)
                );
                assert!(
                    matches!(game.world.block_entities.get(&far),Some(BlockEntity::Chest(after)) if after==&slots)
                );
                assert_eq!(game.world.block_names.get(&far), Some(&custom));
                assert_eq!(game.world.block_lock(far), Some(&lock));
                assert_eq!(game.world.container_stack_limit(far), Some(13));
                assert_eq!(game.world.block_entities.iter_mut().count(), 1);
                return;
            }
            game.with_level(|b, level, ticks| {
                b.client_block_event(
                    level,
                    ticks,
                    BlockPos::new(2, 64, 2),
                    0,
                    Direction::East as i32,
                );
            });
            assert!(
                game.container.is_none(),
                "source menu closes rather than following another holder"
            );
            assert!(game.world.block_entities.get(&from).is_none());
            assert!(!game.world.block_names.contains_key(&from));
            assert!(game.world.block_lock(from).is_none());
            assert_eq!(game.world.container_stack_limit(from), None);
            assert!(!game.server.lid_openers.contains_key(&from));
            assert!(!game.world.chest_openers.contains_key(&from));
            assert!(!game.server.be_anims.bells.contains_key(&[3, 64, 2]));
            // Save/reload in flight also keeps the position-owned fields.
            let at = BlockPos::new(4, 64, 2);
            let moving = game
                .block_ticks
                .moving_pistons
                .iter()
                .find(|(p, _)| *p == at)
                .unwrap()
                .1;
            let saved = game.saved_moving_piston(at, &moving).unwrap();
            game.world.block_entities.discard_carried(&to);
            let mut extras = sunrise_sim::save::ChunkExtras::default();
            extras.block_entities.push(saved);
            game.restore_moving_piston(at, moving, Some(&extras));
            for _ in 0..10 {
                game.world.game_time += 1;
                game.with_level(|b, level, ticks| b.client_tick_block_entities(level, ticks));
            }
            assert_eq!(game.world.block_entities.iter_mut().count(), 1);
            assert!(game.world.block_entities.get(&from).is_none());
            assert_eq!(game.world.block_names.get(&to), Some(&custom));
            assert_eq!(game.world.block_lock(to), Some(&lock));
            assert_eq!(game.world.container_stack_limit(to), Some(13));
            assert_eq!(
                game.world
                    .block_entity_extras(to)
                    .source
                    .unwrap()
                    .get("components")
                    .and_then(|t| t.get("minecraft:custom_data"))
                    .and_then(|t| t.get("carry_test")),
                Some(&Tag::Int(11))
            );
            if case == "furnace" {
                assert!(
                    matches!(game.world.block_entities.get(&to),Some(BlockEntity::Furnace(f)) if f.lit_remaining==100)
                );
                let states = game.ticking_states();
                game.world.block_entities.set_states(states);
                let world = &mut game.world;
                let chunks = &world.chunks;
                let slots = world.block_entities.tickers(|p| {
                    chunks
                        .get(&(p.0 >> 4, p.2 >> 4))
                        .unwrap()
                        .block(p.0 & 15, p.1, p.2 & 15)
                        .0
                });
                assert!(slots.iter().any(|(p, _)| *p == to));
                assert!(!game.world.block_entities.is_live(old_ticker.unwrap()));
                assert!(!slots.iter().any(|(p, _)| *p == from));
                game.tick_furnace(to).unwrap();
                assert!(
                    matches!(game.world.block_entities.get(&to),Some(BlockEntity::Furnace(f)) if f.lit_remaining==99)
                );
            } else {
                assert!(
                    matches!(game.world.block_entities.get(&to),Some(BlockEntity::Chest(after)) if after==&slots)
                );
                if case == "hopper" {
                    assert!(!game.server.hoppers.contains_key(&from));
                    assert_eq!(game.hopper_cooldown(to), 40);
                } else if case != "loot" && case != "mixed-loot" {
                    game.spectator_open_menu([4, 64, 2]);
                    assert_eq!(game.container, Some(to));
                }
            }
            let tag = game.block_entity_nbt([4, 64, 2]).unwrap();
            assert_eq!(tag.get("x"), Some(&Tag::Int(4)));
            assert!(game.block_entity_nbt([3, 64, 2]).is_none());
            if case == "mixed-loot" {
                assert_eq!(
                    tag.get("LootTable"),
                    Some(&Tag::String("test:carry".into()))
                );
                assert_eq!(tag.get("LootTableSeed"), Some(&Tag::Long(37)));
                game.build_registries();
                assert!(game.as_console(|g| g.run_command(
                    "execute if items block 4 64 2 container.* minecraft:diamond run say mixed"
                )));
                assert!(game.world.block_entity_extras(to).pending_loot.is_none());
                assert_eq!(
                    game.world.block_entities.get(&to).unwrap().slots(),
                    slots.as_slice()
                );
            }
            let mut batch = sunrise_sim::save::chunks_with_block_entities(
                game.world.chunks.values(),
                &game.world.block_entities,
                &game.world.block_names,
            );
            game.attach_save_data(&mut batch[0]);
            let encoded = sunrise_sim::save::encode(
                &batch[0],
                registry,
                game.res.biomes(),
                &Default::default(),
            );
            let bytes =
                sunrise_core::nbt::write_named(&encoded, sunrise_core::nbt::Limits::saved())
                    .unwrap();
            let (encoded, used) =
                sunrise_core::nbt::read_named(&bytes, sunrise_core::nbt::Limits::saved()).unwrap();
            assert_eq!(used, bytes.len());
            let Tag::List(saved_entities) = encoded.tag.get("block_entities").unwrap() else {
                panic!()
            };
            let tag = saved_entities
                .iter()
                .find(|t| t.get("x") == Some(&Tag::Int(4)))
                .unwrap();
            assert_eq!(
                tag.get("components")
                    .and_then(|t| t.get("minecraft:custom_data"))
                    .and_then(|t| t.get("carry_test")),
                Some(&Tag::Int(11))
            );
            assert_eq!(tag.get("CustomName"), Some(&custom));
            assert_eq!(tag.get("lock"), Some(&lock));
            if case == "loot" {
                let roll = game.world.block_entity_extras(to).pending_loot.unwrap();
                assert_eq!(roll.seed, Some(Tag::Long(37)));

                assert_eq!(
                    tag.get("LootTable"),
                    Some(&Tag::String("test:carry".into()))
                );
                assert_eq!(tag.get("LootTableSeed"), Some(&Tag::Long(37)));
            } else if case == "furnace" {
                let (_, BlockEntity::Furnace(saved_furnace)) =
                    sunrise_sim::save::decode_block_entity(tag).unwrap()
                else {
                    panic!()
                };
                assert_eq!(saved_furnace.lit_remaining, 99);
            } else {
                let (_, BlockEntity::Chest(saved_slots)) =
                    sunrise_sim::save::decode_block_entity(tag).unwrap()
                else {
                    panic!()
                };
                assert_eq!(saved_slots, slots);
            }
            if matches!(case.as_str(), "loot" | "mixed-loot" | "chest") {
                let loaded =
                    sunrise_sim::save::decode(&encoded, registry, game.res.biomes()).unwrap();
                assert!(
                    !loaded
                        .saved
                        .as_ref()
                        .unwrap()
                        .pending_loot
                        .contains_key(&from)
                );
                let (_, entity) = loaded
                    .block_entities
                    .iter()
                    .find(|(p, _)| *p == to)
                    .unwrap();
                assert_eq!(entity.slots(), slots.as_slice());
                game.world.block_entities.remove(&to);
                game.world.block_entities.insert(to, entity.clone());
                game.world.chunks.insert((0, 0), Arc::new(loaded));
                game.build_registries();
                for _ in 0..2 {
                    assert!(game.as_console(|g|g.run_command("execute if items block 4 64 2 container.* minecraft:diamond run say carried")));
                    assert!(game.world.block_entity_extras(to).pending_loot.is_none());
                    let count: u32 = game
                        .world
                        .block_entities
                        .get(&to)
                        .unwrap()
                        .slots()
                        .iter()
                        .flatten()
                        .filter(|s| &*s.item == "minecraft:diamond")
                        .map(|s| s.count)
                        .sum();
                    assert_eq!(count, if case == "loot" { 1 } else { 7 });
                }
            }
        });
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
