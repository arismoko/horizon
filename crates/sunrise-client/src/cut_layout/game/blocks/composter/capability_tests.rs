use mod_api::{BlockUpdateFlags, Flow, Hooks, ItemResource, ServerHost};
use std::sync::{Arc, Mutex};
use sunrise_core::{
    geometry::Direction,
    position::{BlockPos, ChunkPos},
    random::RandomSource,
};
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
use {
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
    sunrise_gameplay_common::game::api_values::TryIntoApi,
};

const POS: [i32; 3] = [2, 64, 2];

fn resource(item: &str) -> ItemResource {
    ItemResource::of(&ItemStack::new(item, 1).try_into_api().unwrap())
}

fn composter(game: &mut sunrise_gameplay::game::ServerGame<'_>, level: i32) {
    let state = game
        .pack
        .assets
        .registry
        .resolve(&sunrise_world::chunk::BlockState {
            name: "minecraft:composter".into(),
            properties: [("level".into(), level.to_string())].into_iter().collect(),
        })
        .unwrap();
    game.world.set(POS[0], POS[1], POS[2], state).unwrap();
}

fn quiet(game: &mut sunrise_gameplay::game::ServerGame<'_>, heard: &Mutex<Vec<String>>) {
    game.world.modified.clear();
    game.world.dirty_sections.clear();
    game.world.block_log = Some(Default::default());
    game.start_audio_capture();
    heard.lock().unwrap().clear();
}

fn assert_quiet(game: &sunrise_gameplay::game::ServerGame<'_>, heard: &Mutex<Vec<String>>) {
    assert!(
        heard.lock().unwrap().is_empty(),
        "game events escaped a transaction"
    );
    assert!(game.level_events.is_empty());
    assert!(
        game.server.pending_sounds.is_empty(),
        "sound escaped a transaction"
    );
    assert!(
        game.world.modified.is_empty(),
        "tentative state dirtied the save"
    );
    assert!(
        game.world.dirty_sections.is_empty(),
        "tentative state requested a remesh"
    );
    assert!(
        game.world.block_log.as_ref().unwrap().is_empty(),
        "tentative state was published"
    );
    assert!(
        game.dropped.is_empty(),
        "a container extraction spawned bone meal"
    );
}

#[test]
#[ignore = "daily"]
fn sided_composter_transactions_preserve_literal_chance_and_defer_all_effects() {
    struct Heard(Arc<Mutex<Vec<String>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Heard {
        fn vanilla_game_event(
            &self,
            _: &mut dyn ServerHost,
            event: &mut mod_api::VanillaGameEventEvent<'_>,
        ) -> Flow {
            self.0.lock().unwrap().push(event.event.to_owned());
            Flow::Continue
        }
    }
    let heard = Arc::new(Mutex::new(Vec::new()));
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "composter_observer",
            Box::new(Heard(heard.clone())),
        )]
        .into(),
        || {
            with_game(|game| {
                for (x, z) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
                    let mut chunk = Chunk::empty(x, z);
                    chunk.lit = true;
                    game.world.chunks.insert((x, z), Arc::new(chunk));
                }
                game.block_ticks
                    .add_empty_chunk(ChunkPos::new(0, 0))
                    .unwrap();
                composter(game, 0);
                let level = game.level("minecraft:overworld").unwrap();
                let input = level
                    .capability(game, &mod_api::ITEMS, POS, Some(Direction::Up))
                    .unwrap();
                assert!(level.capability(game, &mod_api::ITEMS, POS, None).is_none());
                assert!(
                    level
                        .capability(game, &mod_api::STORAGE, POS, None)
                        .is_none()
                );
                let output = level
                    .capability(game, &mod_api::ITEMS, POS, Some(Direction::Down))
                    .unwrap();
                let storage_input = level
                    .capability(game, &mod_api::STORAGE, POS, Some(Direction::Up))
                    .unwrap();
                let storage_output = level
                    .capability(game, &mod_api::STORAGE, POS, Some(Direction::Down))
                    .unwrap();
                for face in [
                    Direction::North,
                    Direction::South,
                    Direction::West,
                    Direction::East,
                ] {
                    assert!(
                        level
                            .capability(game, &mod_api::STORAGE, POS, Some(face))
                            .is_none()
                    );
                    assert!(
                        level
                            .capability(game, &mod_api::ITEMS, POS, Some(face))
                            .is_none()
                    );
                }
                let seeds = resource("minecraft:wheat_seeds");
                let pie = resource("minecraft:pumpkin_pie");
                game.options.dedicated = Some(Vec::new());
                let bone = resource("minecraft:bone_meal");
                let stone = resource("minecraft:stone");
                assert_eq!(storage_input.amount(game, &seeds), Ok(0));
                assert_eq!(storage_input.space_for(game, &seeds), Ok(1));
                assert_eq!(storage_input.space_for(game, &stone), Ok(0));
                assert_eq!(storage_output.amount(game, &bone), Ok(0));
                assert_eq!(storage_output.space_for(game, &seeds), Ok(0));
                assert_eq!(input.size(game), 1);
                assert_eq!(output.size(game), 1);
                assert_eq!(input.capacity(game, 0, Some(&seeds)), 1);
                assert_eq!(input.capacity(game, 0, Some(&stone)), 0);
                assert_eq!(input.capacity(game, 1, Some(&seeds)), 0);
                assert_eq!(output.capacity(game, 0, Some(&seeds)), 0);
                assert_eq!(output.amount(game, 0), 0);
                assert!(input.resource(game, 0).is_none());
                quiet(game, &heard);
                game.storage_transaction(|game, tx| {
                    assert_eq!(input.insert_at(game, tx, 0, &stone, 64), Ok(0));
                    assert_eq!(input.insert_at(game, tx, 1, &seeds, 64), Ok(0));
                    assert_eq!(input.insert_at(game, tx, 0, &seeds, 0), Ok(0));
                    assert_eq!(output.insert_at(game, tx, 0, &seeds, 64), Ok(0));
                    assert_eq!(input.extract_at(game, tx, 0, &seeds, 64), Ok(0));
                    assert_eq!(input.set_max_stack_size(game, tx, 64), Ok(false));
                    assert_eq!(storage_input.insert(game, tx, &stone, 64), Ok(0));
                    assert_eq!(storage_input.insert(game, tx, &seeds, 0), Ok(0));
                    assert_eq!(storage_output.insert(game, tx, &seeds, 64), Ok(0));
                    assert_eq!(storage_input.extract(game, tx, &seeds, 64), Ok(0));
                    Ok::<_, ()>(())
                })
                .unwrap();
                assert_eq!(game.composter_level(POS), Some(0));
                assert_quiet(game, &heard);

                // A source container and the composter participate in the SAME transaction.
                let source_pos = [4, 64, 2];
                let chest = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:chest")
                    .unwrap();
                game.level_set_block(level, source_pos, chest, BlockUpdateFlags::NEIGHBORS)
                    .unwrap();
                let source = level
                    .capability(game, &mod_api::ITEMS, source_pos, None)
                    .unwrap();
                game.storage_transaction(|game, tx| {
                    assert_eq!(source.insert_at(game, tx, 0, &pie, 64), Ok(64));
                    Ok::<_, ()>(())
                })
                .unwrap();
                // Each position's capability-local owner takes its initial seed from the level.
                game.world.random.set_seed(3448);
                composter(game, 6);
                quiet(game, &heard);
                let state = game.world.block(POS[0], POS[1], POS[2]);
                let block = game.res.behaviours().block(state);
                let at = BlockPos::new(POS[0], POS[1], POS[2]);
                let aborted = game.storage_transaction(|game, tx| {
                    assert_eq!(source.extract_at(game, tx, 0, &pie, 1), Ok(1));
                    game.storage_transaction(|game, tx| {
                        assert_eq!(storage_input.insert(game, tx, &pie, 64), Ok(1));
                        Ok::<_, ()>(())
                    })?;
                    assert_eq!(game.composter_level(POS), Some(7));
                    assert_eq!(source.amount(game, 0), 63);
                    assert!(!game.block_ticks.has_block_tick(at, block));
                    assert_quiet(game, &heard);
                    Err::<(), ()>(())
                });
                assert_eq!(aborted, Err(()));
                assert_eq!(game.composter_level(POS), Some(6));
                assert_eq!(source.amount(game, 0), 64);
                assert!(!game.block_ticks.has_block_tick(at, block));
                assert_quiet(game, &heard);

                // Inner abort does not discard an outer insert, and an outer commit runs only
                // that insert's feedback.
                composter(game, 0);
                quiet(game, &heard);
                game.storage_transaction(|game, tx| {
                    assert_eq!(input.insert_at(game, tx, 0, &pie, 64), Ok(1));
                    let aborted = game.storage_transaction(|game, tx| {
                        assert_eq!(input.insert_at(game, tx, 0, &pie, 64), Ok(1));
                        assert_eq!(game.composter_level(POS), Some(2));
                        Err::<(), ()>(())
                    });
                    assert_eq!(aborted, Err(()));
                    assert_eq!(game.composter_level(POS), Some(1));
                    assert_quiet(game, &heard);
                    Ok::<_, ()>(())
                })
                .unwrap();
                assert_eq!(game.composter_level(POS), Some(1));
                assert_eq!(heard.lock().unwrap().as_slice(), ["minecraft:block_change"]);

                composter(game, 0);
                quiet(game, &heard);
                // Stock compostable/low: level zero adds one WITHOUT a draw. Then Java
                // Random(3448).nextInt(100): 4,36,35,20,2,56,93,4,7,54,55,18 (success <30).
                // Literal expectations, not a second run through Sunrise's layer algorithm.
                game.storage_transaction(|game, tx| {
                    for expected in [1, 2, 2, 2, 3, 4, 4, 4, 5, 6, 6, 6, 7] {
                        assert_eq!(input.insert_at(game, tx, 0, &seeds, 64), Ok(1));
                        assert_eq!(game.composter_level(POS), Some(expected));
                        assert_eq!(input.amount(game, 0), 0);
                        assert_eq!(output.amount(game, 0), 0);
                        assert!(!game.block_ticks.has_block_tick(at, block));
                        assert_quiet(game, &heard);
                    }
                    assert_eq!(input.insert_at(game, tx, 0, &seeds, 64), Ok(0));
                    Ok::<_, ()>(())
                })
                .unwrap();
                assert_eq!(
                    game.composter_level(POS),
                    Some(7),
                    "feedback must not replay intermediate states"
                );
                assert_eq!(heard.lock().unwrap().len(), 7);
                assert!(game.block_ticks.has_block_tick(at, block));
                assert_eq!(input.capacity(game, 0, Some(&seeds)), 0);
                let start = game.world.game_time;
                game.world.game_time = start + 19;
                game.tick_scheduled().unwrap();
                assert_eq!(game.composter_level(POS), Some(7));
                game.world.game_time = start + 20;
                game.tick_scheduled().unwrap();
                assert_eq!(game.composter_level(POS), Some(8));
                assert!(
                    game.server
                        .pending_sounds
                        .iter()
                        .any(|s| s.event == "minecraft:block.composter.ready")
                );
                assert_eq!(input.amount(game, 0), 0);
                assert_eq!(output.amount(game, 0), 1);
                assert_eq!(output.resource(game, 0), Some(bone.clone()));
                assert_eq!(storage_input.amount(game, &bone), Ok(0));
                assert_eq!(storage_output.amount(game, &bone), Ok(1));
                assert_eq!(storage_output.space_for(game, &bone), Ok(0));

                quiet(game, &heard);
                let aborted = game.storage_transaction(|game, tx| {
                    assert_eq!(output.extract_at(game, tx, 0, &pie, 1), Ok(0));
                    assert_eq!(output.extract_at(game, tx, 0, &bone, 0), Ok(0));
                    assert_eq!(storage_output.extract(game, tx, &bone, 64), Ok(1));
                    assert_eq!(output.extract_at(game, tx, 0, &bone, 64), Ok(0));
                    assert_eq!(game.composter_level(POS), Some(0));
                    assert_quiet(game, &heard);
                    Err::<(), ()>(())
                });
                assert_eq!(aborted, Err(()));
                assert_eq!(game.composter_level(POS), Some(8));
                assert_eq!(output.amount(game, 0), 1);
                assert_quiet(game, &heard);
                game.storage_transaction(|game, tx| {
                    assert_eq!(output.extract_at(game, tx, 0, &bone, 64), Ok(1));
                    Ok::<_, ()>(())
                })
                .unwrap();
                assert_eq!(game.composter_level(POS), Some(0));
                assert_eq!(heard.lock().unwrap().as_slice(), ["minecraft:block_change"]);
                assert!(game.server.pending_sounds.is_empty());
                assert!(game.dropped.is_empty());

                // A real bulk consumer offers all remaining items up to 64, not a pre-trimmed
                // single item. Because `move_resources` visits each source index once per call
                // (NeoForge 26.3 `ResourceHandlerUtil.moveInternal`), repeatedly moving from this
                // single-slot source still offers all remaining items on each step and must
                // preserve the literal sequence above across its nested partial attempts, rather
                // than drawing a second chance on each accepted retry.
                let move_all = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                                source: &mod_api::Capability<
                    dyn mod_api::ResourceHandler<mod_api::ItemResource>,
                >,
                                bulk: &mod_api::Capability<
                    dyn mod_api::ResourceHandler<mod_api::ItemResource>,
                >,
                                tx: &mod_api::TransactionContext| {
                    let mut total = 0u32;
                    while total < 64 {
                        let moved = mod_api::resource_util::move_resources(
                            game,
                            source,
                            bulk,
                            |_| true,
                            64 - total,
                            Some(tx),
                        )?;
                        if moved == 0 {
                            break;
                        }
                        assert_eq!(moved, 1);
                        total += moved;
                    }
                    Ok::<_, mod_api::TransactionError<mod_api::Overanswered>>(total)
                };
                game.storage_transaction(|game, tx| {
                    assert_eq!(source.extract_at(game, tx, 0, &pie, 64), Ok(64));
                    assert_eq!(source.insert_at(game, tx, 0, &seeds, 64), Ok(64));
                    Ok::<_, ()>(())
                })
                .unwrap();
                let bulk_pos = [6, 64, 2];
                let empty = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:composter")
                    .unwrap();
                game.world
                    .set(bulk_pos[0], bulk_pos[1], bulk_pos[2], empty)
                    .unwrap();
                let bulk = level
                    .capability(game, &mod_api::ITEMS, bulk_pos, Some(Direction::Up))
                    .unwrap();
                let bulk_at = BlockPos::new(bulk_pos[0], bulk_pos[1], bulk_pos[2]);
                quiet(game, &heard);
                game.world.random.set_seed(3448);
                let mut unrelated_world = game.world.random.clone();
                let aborted = game.storage_transaction(|game, tx| {
                    assert_eq!(move_all(game, &source, &bulk, tx), Ok(13));
                    assert_eq!(game.composter_level(bulk_pos), Some(7));
                    assert_eq!(source.amount(game, 0), 51);
                    assert!(!game.block_ticks.has_block_tick(bulk_at, block));
                    assert_quiet(game, &heard);
                    Err::<(), ()>(())
                });
                assert_eq!(aborted, Err(()));
                assert_eq!(game.composter_level(bulk_pos), Some(0));
                assert_eq!(source.amount(game, 0), 64);
                assert!(!game.block_ticks.has_block_tick(bulk_at, block));
                assert_quiet(game, &heard);

                // Another dimension's unload must leave this cached owner intact, even
                // after a first aborted insertion and a different seed in the level stream.
                game.world.random.set_seed(0);
                unrelated_world.set_seed(0);
                game.enter_level(sunrise_gameplay_common::game::Dim::Nether)
                    .unwrap();
                game.enter_level(sunrise_gameplay_common::game::Dim::Overworld)
                    .unwrap();
                assert!(game.level("minecraft:the_nether").is_some());
                game.native_chunk_unload("minecraft:the_nether", [0, 0]);

                // A nested abort rewinds only this composter's chance owner. An unrelated
                // level draw between the aborted attempt and its retry must survive.
                let aborted = game.storage_transaction(|game, tx| {
                    assert_eq!(bulk.insert_at(game, tx, 0, &seeds, 1), Ok(1));
                    let nested = game.storage_transaction(|game, tx| {
                        assert_eq!(bulk.insert_at(game, tx, 0, &seeds, 1), Ok(1));
                        assert_eq!(game.composter_level(bulk_pos), Some(2));
                        Err::<(), ()>(())
                    });
                    assert_eq!(nested, Err(()));
                    assert_eq!(game.composter_level(bulk_pos), Some(1));
                    assert_eq!(game.world.random.next_long(), unrelated_world.next_long());
                    assert_eq!(bulk.insert_at(game, tx, 0, &seeds, 1), Ok(1));
                    assert_eq!(game.composter_level(bulk_pos), Some(2));
                    assert_quiet(game, &heard);
                    Err::<(), ()>(())
                });
                assert_eq!(aborted, Err(()));
                assert_eq!(game.composter_level(bulk_pos), Some(0));
                assert_eq!(game.world.random.next_long(), unrelated_world.next_long());
                game.storage_transaction(|game, tx| {
                    assert_eq!(move_all(game, &source, &bulk, tx), Ok(13));
                    assert_eq!(game.composter_level(bulk_pos), Some(7));
                    assert_eq!(source.amount(game, 0), 51);
                    assert!(!game.block_ticks.has_block_tick(bulk_at, block));
                    assert_quiet(game, &heard);
                    Ok::<_, ()>(())
                })
                .unwrap();
                assert_eq!(game.composter_level(bulk_pos), Some(7));
                assert_eq!(source.amount(game, 0), 51);
                assert_eq!(heard.lock().unwrap().len(), 7);
                assert!(game.block_ticks.has_block_tick(bulk_at, block));
                assert_eq!(game.world.random.next_long(), unrelated_world.next_long());

                // The existing unload lifecycle retires the chunk's position-local owners.
                // Reusing a retained handler after reload starts from the new level seed,
                // not the previous owner's already-consumed chance sequence.
                game.native_chunk_unload("minecraft:overworld", [0, 0]);
                // A pending outer savepoint must retire too: aborting after unload cannot
                // resurrect the previous owner's consumed chance sequence.
                let aborted = game.storage_transaction(|game, tx| {
                    game.world
                        .set(bulk_pos[0], bulk_pos[1], bulk_pos[2], empty)
                        .unwrap();
                    assert_eq!(bulk.insert_at(game, tx, 0, &seeds, 1), Ok(1));
                    game.native_chunk_unload("minecraft:overworld", [0, 0]);
                    Err::<(), ()>(())
                });
                assert_eq!(aborted, Err(()));
                game.world
                    .set(bulk_pos[0], bulk_pos[1], bulk_pos[2], empty)
                    .unwrap();
                game.world.random.set_seed(3448);
                quiet(game, &heard);
                game.storage_transaction(|game, tx| {
                    assert_eq!(move_all(game, &source, &bulk, tx), Ok(13));
                    Ok::<_, ()>(())
                })
                .unwrap();
                assert_eq!(game.composter_level(bulk_pos), Some(7));
                assert_eq!(source.amount(game, 0), 38);

                // A later write in the same transaction wins: deferred fill feedback must not
                // resurrect the composter, even when insertion reached maturation level seven.
                composter(game, 6);
                game.storage_transaction(|game, tx| {
                    assert_eq!(input.insert_at(game, tx, 0, &pie, 64), Ok(1));
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    assert!(game.journal_set_block(POS, stone));
                    Ok::<_, ()>(())
                })
                .unwrap();
                assert_eq!(game.composter_level(POS), None);
                assert_eq!(input.size(game), 0);

                // A reopened world gets new owners and rejects retained old-epoch handlers.
                game.native_lifecycle_open_started();
                let reopened = game.level("minecraft:overworld").unwrap();
                assert_ne!(reopened.epoch(), level.epoch());
                assert_eq!(bulk.size(game), 0);
                assert_eq!(source.size(game), 0);
                let source = reopened
                    .capability(game, &mod_api::ITEMS, source_pos, None)
                    .unwrap();
                let bulk = reopened
                    .capability(game, &mod_api::ITEMS, bulk_pos, Some(Direction::Up))
                    .unwrap();
                game.world
                    .set(bulk_pos[0], bulk_pos[1], bulk_pos[2], empty)
                    .unwrap();
                game.world.random.set_seed(3448);
                game.storage_transaction(|game, tx| {
                    assert_eq!(source.insert_at(game, tx, 0, &seeds, 26), Ok(26));
                    assert_eq!(move_all(game, &source, &bulk, tx), Ok(13));
                    Ok::<_, ()>(())
                })
                .unwrap();
                assert_eq!(game.composter_level(bulk_pos), Some(7));
                assert_eq!(source.amount(game, 0), 51);
            })
        },
    );
}

/// `ComposterBlock.addLayer` builds `BLOCK_INTERACT`: the clicking player is
/// `interacting_entity`, a hopper has none, and nothing is `this`. A data pack
/// item whose `layers` provider tests each of those drives the click producer
/// (`use_composter`), not the provider evaluator.
#[test]
#[ignore = "daily"]
fn a_composter_click_supplies_its_interacting_player_and_no_this() {
    let mut resources = None;
    let (_published, mut game) =
        crate::game::game_tests::tests::isolated_server_game(&mut resources);
    let provider = |who: &str| {
        serde_json::json!({
            "type": "minecraft:conditional",
            "condition": {"condition": "minecraft:entity_properties", "entity": who,
                "predicate": {"minecraft:entity_type": "minecraft:player"}},
            "on_true": 1, "on_false": 0
        })
    };
    let mut items = game.items().clone();
    items
        .items
        .get_mut("minecraft:wheat_seeds")
        .unwrap()
        .compostable = Some(provider("interacting_entity"));
    items
        .items
        .get_mut("minecraft:beetroot_seeds")
        .unwrap()
        .compostable = Some(provider("this"));
    game.server_items = Some(Arc::new(items));
    game.build_registries();
    // The clicking player, beside the composter.
    game.player = Some(
        sunrise_sim::motion::PlayerMotion::new(
            game.motion,
            sunrise_core::geometry::Vec3::new(4.5, 64.0, 2.5),
        )
        .unwrap(),
    );
    for (x, z) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
        game.world
            .chunks
            .insert((x, z), Arc::new(Chunk::empty(x, z)));
    }
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    let click = |game: &mut sunrise_gameplay::game::ServerGame<'_>, item: &str| {
        composter(game, 0);
        let stack = ItemStack::new(item, 8);
        *game.held_mut() = Some(stack.clone());
        assert!(game.use_composter(POS, Some(&stack)));
        game.composter_level(POS).unwrap()
    };
    assert_eq!(
        click(&mut game, "minecraft:wheat_seeds"),
        1,
        "the clicking player is the interacting entity"
    );
    assert_eq!(
        click(&mut game, "minecraft:beetroot_seeds"),
        0,
        "a block interaction has no `this`, not even the player"
    );
    // A hopper's insert (`InputContainer`) has no interacting entity either.
    let layers = game.composter_transactional_layers(POS, &provider("interacting_entity"));
    assert_eq!(layers, 0);
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycleCommonGameExt1 as _;
