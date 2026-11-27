//! A mod's world pickup and placement of fluid in a real game: decided at the call, its block
//! kept by the transaction, its updates, drops and effects at the outermost commit, nothing of
//! it after an abort.
use mod_api::{FluidStack, HostTransaction, LevelRef, ServerHost, TransactionError};
use std::sync::Arc;
use sunrise_level::{chunk::Chunk, containers::BlockEntity};
use sunrise_world::registry::BlockStateId;
use {
    sunrise_gameplay::game::dispense_fixture,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

const AT: [i32; 3] = [2, 65, 2];

/// A game with one empty chunk, a dirt floor at y 64 and the level's handle.
fn with_level(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, LevelRef)) {
    with_game(|game| {
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.native_lifecycle
            .begin_open(Some(mod_api::Authority::IntegratedServer));
        let dirt = state(game, "minecraft:dirt");
        for x in 0..5 {
            for z in 0..5 {
                game.world.set(x, 64, z, dirt).unwrap();
            }
        }
        let level = game.level("minecraft:overworld").unwrap();
        test(game, level);
    });
}

fn state(game: &sunrise_gameplay::game::ServerGame<'_>, name: &str) -> BlockStateId {
    game.pack.assets.registry.default_state(name).unwrap()
}

fn key(id: &str) -> mod_api::FluidKey {
    mod_api::__private::fluid_key(id).unwrap()
}

fn block(game: &sunrise_gameplay::game::ServerGame<'_>) -> BlockStateId {
    game.world.block(AT[0], AT[1], AT[2])
}

#[test]
#[ignore = "daily"]
fn a_second_pickup_of_one_source_in_a_transaction_gets_nothing() {
    with_level(|game, level| {
        let water = state(game, "minecraft:water");
        game.world.set(AT[0], AT[1], AT[2], water).unwrap();
        let host: &mut dyn ServerHost = game;
        let picked: Result<_, TransactionError<()>> = host.transaction(|host, tx| {
            host.transaction(|host, _inner| {
                assert_eq!(
                    level.pickup_fluid(host, AT, None, tx),
                    Err(TransactionError::InvalidContext)
                );
                assert_eq!(
                    level.place_fluid(host, AT, key("minecraft:water"), None, false, tx),
                    Err(TransactionError::InvalidContext)
                );
                assert_eq!(
                    level.play_fluid_effects(host, AT, key("minecraft:water"), false, None, tx),
                    Err(TransactionError::InvalidContext)
                );
                Ok::<_, ()>(())
            })
            .unwrap();
            Ok((
                level.pickup_fluid(host, AT, None, tx).unwrap(),
                level.pickup_fluid(host, AT, None, tx).unwrap(),
            ))
        });
        let one = FluidStack::new(key("minecraft:water"), 1000);
        assert_eq!(picked, Ok((Some(one), None)));
        assert_eq!(block(game), state(game, "minecraft:air"));
    });
}

/// A registered tank-only fluid that names a live bucket plays that bucket's fill sound once at
/// the commit, as a world fluid does; one that names none plays no sound.
#[test]
#[ignore = "daily"]
fn a_tank_only_fluid_with_a_bucket_plays_its_fill_sound_at_the_commit() {
    // It replaces the process-wide fluid looks, which other tests read.
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::world_fluids_tests::a_tank_only_fluid_with_a_bucket_plays_its_fill_sound_at_the_commit",
        || {
            with_level(|game, level| {
                sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan(game);
                let fill = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                    game.server.pending_native_sounds.clear();
                    let host: &mut dyn ServerHost = game;
                    let played: Result<bool, TransactionError<()>> =
                        host.transaction(|host, tx| {
                            level
                                .play_fluid_effects(
                                    host,
                                    AT,
                                    key("sunrise:definition_test"),
                                    true,
                                    None,
                                    tx,
                                )
                                .map_err(|_| ())
                        });
                    assert_eq!(played, Ok(true));
                    game.server
                        .pending_native_sounds
                        .iter()
                        .map(|s| s.sound.clone())
                        .collect::<Vec<_>>()
                };
                assert!(fill(game).is_empty());
                let mut content = sunrise_mod_package::content::Content::declared("sunrise");
                content
                    .fluids
                    .push(sunrise_mod_package::content::FluidLook {
                        id: "sunrise:definition_test".into(),
                        sprite: None,
                        tint: None,
                        world: None,
                        bucket_item: Some("minecraft:honey_bottle".into()),
                        measures: Default::default(),
                    });
                sunrise_sim::mod_content::set_fluid_looks(&[
                    sunrise_sim::mod_content::ContentMod {
                        id: "sunrise".into(),
                        content,
                        jars: Vec::new(),
                    },
                ]);
                let sounds = fill(game);
                sunrise_sim::mod_content::set_fluid_looks(&[]);
                assert_eq!(sounds, ["minecraft:item.bucket.fill"]);
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn an_aborted_placement_over_a_skull_leaves_it_and_drops_nothing() {
    with_level(|game, level| {
        let skull = state(game, "minecraft:skeleton_skull");
        game.world.set(AT[0], AT[1], AT[2], skull).unwrap();
        let pos = (AT[0], AT[1], AT[2]);
        game.world
            .block_entities
            .insert(pos, BlockEntity::Skull(Default::default()));
        let kept = format!("{:?}", game.world.block_entities.get(&pos));
        assert!(
            game.world.block_entities.get(&pos).is_some(),
            "the skull has its block entity"
        );
        let items = game.dropped.len();
        dispense_fixture::start_recording();
        let water = key("minecraft:water");
        let _ = game.storage_transaction(|game, tx| {
            assert_eq!(
                game.level_place_fluid(level, AT, water, None, true, tx),
                Ok(true)
            );
            // Inside the transaction the skull is gone and the water is there.
            assert_eq!(block(game), state(game, "minecraft:water"));
            assert!(game.world.block_entities.get(&pos).is_none());
            Err::<(), ()>(())
        });
        assert_eq!(block(game), skull);
        assert_eq!(format!("{:?}", game.world.block_entities.get(&pos)), kept);
        assert_eq!(game.dropped.len(), items);
        assert_eq!(dispense_fixture::stop_recording(), Vec::<String>::new());
    });
}

#[test]
#[ignore = "daily"]
fn a_committed_pour_breaks_what_it_replaces_then_sends_its_events_in_order() {
    with_level(|game, level| {
        sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan(game);
        let poppy = state(game, "minecraft:poppy");
        game.world.set(AT[0], AT[1], AT[2], poppy).unwrap();
        let items = game.dropped.len();
        game.world.block_log = Some(Vec::new());
        game.server.pending_native_sounds.clear();
        dispense_fixture::start_recording();
        let host: &mut dyn ServerHost = game;
        let placed: Result<bool, TransactionError<()>> = host.transaction(|host, tx| {
            level
                .place_fluid(host, AT, key("minecraft:lava"), None, false, tx)
                .map_err(|_| ())
        });
        assert_eq!(placed, Ok(true));
        let lines = dispense_fixture::stop_recording();
        // One write tells the clients of the block, once.
        let told = game.world.block_log.as_ref().unwrap();
        assert_eq!(told.iter().filter(|p| **p == (2, 65, 2)).count(), 1);
        // The bucket's empty sound, queued once at the commit.
        let sounds: Vec<_> = game
            .server
            .pending_native_sounds
            .iter()
            .map(|s| (s.sound.clone(), s.at))
            .collect();
        assert_eq!(
            sounds,
            [(
                "minecraft:item.bucket.empty_lava".to_owned(),
                [2.5, 65.5, 2.5]
            )]
        );
        assert_eq!(block(game), state(game, "minecraft:lava"));
        // The poppy's drop, made at the call, spawned at the commit.
        assert_eq!(game.dropped.len(), items + 1);
        assert_eq!(&*game.dropped[items].stack.item, "minecraft:poppy");
        assert_eq!(
            lines,
            [
                format!("L 2001 2,65,2 {}", poppy.0),
                "G 2,65,2 minecraft:block_destroy".to_owned(),
                "G 2,65,2 minecraft:fluid_place".to_owned(),
            ]
        );
    });
}

#[test]
#[ignore = "daily"]
fn the_call_time_answer_stands_when_the_block_changes_before_the_commit() {
    with_level(|game, level| {
        let poppy = state(game, "minecraft:poppy");
        game.world.set(AT[0], AT[1], AT[2], poppy).unwrap();
        let stone = state(game, "minecraft:stone");
        let items = game.dropped.len();
        dispense_fixture::start_recording();
        let water = key("minecraft:water");
        let placed = game
            .storage_transaction(|game, tx| {
                let placed = game
                    .level_place_fluid(level, AT, water, None, false, tx)
                    .map_err(|_| ())?;
                // A later write in the same transaction: the commit doesn't decide again.
                game.journal_set_block(AT, stone);
                Ok::<_, ()>(placed)
            })
            .unwrap();
        assert!(placed);
        let lines = dispense_fixture::stop_recording();
        assert_eq!(block(game), stone);
        assert_eq!(game.dropped.len(), items + 1);
        assert_eq!(
            lines,
            [
                format!("L 2001 2,65,2 {}", poppy.0),
                "G 2,65,2 minecraft:block_destroy".to_owned(),
                "G 2,65,2 minecraft:fluid_place".to_owned(),
            ]
        );
    });
}

/// Listens to `block_drops`, keeps the block it reads where the drops are made and cancels
/// the drops when asked to.
struct SeenBlocks(Arc<std::sync::Mutex<Vec<Option<String>>>>, bool);
#[mod_api::__subscriptions]
impl mod_api::Hooks for SeenBlocks {
    fn block_drops(
        &self,
        host: &dyn mod_api::LevelHost,
        _: &mut mod_api::BlockDropsEvent<'_>,
    ) -> mod_api::Flow {
        let block = host
            .block_state(AT)
            .and_then(|state| host.state_block(state).map(str::to_owned));
        self.0.lock().unwrap().push(block);
        if self.1 {
            mod_api::Flow::Cancel
        } else {
            mod_api::Flow::Continue
        }
    }
}

#[test]
#[ignore = "daily"]
fn a_drop_listener_sees_the_old_block_and_an_abort_undoes_the_loot_it_unpacked() {
    use sunrise_core::nbt::Tag;
    use sunrise_level::chunk::PendingContainerLoot;
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "seen_blocks",
        Box::new(SeenBlocks(seen.clone(), false)),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_level(|game, level| {
            game.options.dedicated = Some(Vec::new());
            // A skull with a container's block entity, its loot not yet rolled: breaking it
            // unpacks the loot into the entity.
            let skull = state(game, "minecraft:skeleton_skull");
            game.world.set(AT[0], AT[1], AT[2], skull).unwrap();
            let pos = (AT[0], AT[1], AT[2]);
            game.world
                .block_entities
                .insert(pos, BlockEntity::Chest(vec![None; 27]));
            let loot = PendingContainerLoot {
                table: Tag::String("minecraft:chests/simple_dungeon".into()),
                seed: Some(Tag::Long(42)),
            };
            game.world.set_block_entity_extras(
                pos,
                sunrise_level::world::BlockEntityExtras {
                    source: None,
                    pending_loot: Some(loot),
                },
            );
            let water = key("minecraft:water");
            let _ = game.storage_transaction(|game, tx| {
                assert_eq!(
                    game.level_place_fluid(level, AT, water, None, false, tx),
                    Ok(true)
                );
                Err::<(), ()>(())
            });
            // The listener ran while the broken block was still there.
            assert_eq!(
                *seen.lock().unwrap(),
                [Some("minecraft:skeleton_skull".to_owned())]
            );
            assert_eq!(block(game), skull);
            let Some(BlockEntity::Chest(slots)) = game.world.block_entities.get(&pos) else {
                panic!("the container's block entity is back");
            };
            assert!(
                slots.iter().all(Option::is_none),
                "its loot is unrolled again"
            );
            assert!(game.world.block_entity_extras(pos).pending_loot.is_some());
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_cancelled_drop_keeps_the_replacement() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "cancelled_drops",
        Box::new(SeenBlocks(seen.clone(), true)),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_level(|game, level| {
            game.options.dedicated = Some(Vec::new());
            let poppy = state(game, "minecraft:poppy");
            game.world.set(AT[0], AT[1], AT[2], poppy).unwrap();
            let items = game.dropped.len();
            let water = key("minecraft:water");
            game.storage_transaction(|game, tx| {
                assert_eq!(
                    game.level_place_fluid(level, AT, water, None, false, tx),
                    Ok(true)
                );
                Ok::<_, ()>(())
            })
            .unwrap();
            // The listener saw the poppy and cancelled: no drop, but the water is there.
            assert_eq!(*seen.lock().unwrap(), [Some("minecraft:poppy".to_owned())]);
            assert_eq!(game.dropped.len(), items);
            assert_eq!(block(game), state(game, "minecraft:water"));
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_later_write_that_puts_the_block_back_keeps_its_block_entity_at_the_commit() {
    with_level(|game, level| {
        let skull = state(game, "minecraft:skeleton_skull");
        game.world.set(AT[0], AT[1], AT[2], skull).unwrap();
        let pos = (AT[0], AT[1], AT[2]);
        game.world
            .block_entities
            .insert(pos, BlockEntity::Skull(Default::default()));
        let water = key("minecraft:water");
        game.storage_transaction(|game, tx| {
            assert_eq!(
                game.level_place_fluid(level, AT, water, None, false, tx),
                Ok(true)
            );
            assert!(game.world.block_entities.get(&pos).is_none());
            game.journal_set_block(AT, skull);
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(block(game), skull);
        assert!(matches!(
            game.world.block_entities.get(&pos),
            Some(BlockEntity::Skull(_))
        ));
    });
}

#[test]
#[ignore = "daily"]
fn journal_level_host_stages_block_entity_random_and_effect_changes() {
    use mod_api::{BlockPos, BlockUpdateFlags, LevelWriteHost};
    use sunrise_core::{position::ChunkPos, ticks::TickTypeId};
    use sunrise_gameplay::game::journal_level_write::JournalLevelWrite;
    use sunrise_world::fluid::FluidKind;

    with_level(|game, level| {
        let skull = state(game, "minecraft:skeleton_skull");
        let stone = state(game, "minecraft:stone");
        let key = (AT[0], AT[1], AT[2]);
        let pos = BlockPos::new(AT[0], AT[1], AT[2]);
        let water = TickTypeId(FluidKind::Water.tick_id());
        game.block_ticks
            .add_chunk(ChunkPos::new(0, 0), Default::default(), Default::default())
            .unwrap();
        for commit in [false, true] {
            game.world.set(AT[0], AT[1], AT[2], skull).unwrap();
            game.world
                .block_entities
                .insert(key, BlockEntity::Skull(Default::default()));
            let before = <(u64, Option<f64>)>::from(&*game.world.random);
            game.storage_scope(|game, _| {
                dispense_fixture::start_recording();
                {
                    let mut host = JournalLevelWrite::new(game, level).unwrap();
                    let host: &mut dyn LevelWriteHost = &mut host;
                    assert!(host.block_entity(AT).is_some());
                    assert_eq!(
                        host.state_block(host.block_state(AT).unwrap()),
                        Some("minecraft:skeleton_skull")
                    );
                    let stone = host.block_state_id("minecraft:stone").unwrap();
                    assert!(host.set_block(
                        AT,
                        stone,
                        BlockUpdateFlags::CLIENTS | BlockUpdateFlags::KNOWN_SHAPE
                    ));
                    assert_eq!(host.block_state(AT), Some(stone));
                    assert!(host.block_entity(AT).is_none());
                    host.with_random(|random| random.next_int()).unwrap();
                    assert!(host.schedule_fluid_tick(AT, "minecraft:water", 5));
                    assert!(host.level_event(1501, AT, 0));
                    assert!(!host.level_event(2001, AT, -1));
                }
                assert_eq!(block(game), stone);
                assert!(!game.block_ticks.fluids.has_scheduled(pos, water));
                assert!(dispense_fixture::stop_recording().is_empty());
                dispense_fixture::start_recording();
                ((), commit)
            });
            assert_eq!(block(game), if commit { stone } else { skull });
            assert_eq!(game.world.block_entities.get(&key).is_none(), commit);
            assert_ne!(<(u64, Option<f64>)>::from(&*game.world.random), before);
            assert_eq!(game.block_ticks.fluids.has_scheduled(pos, water), commit);
            let lines = dispense_fixture::stop_recording();
            assert_eq!(lines.iter().any(|line| line == "L 1501 2,65,2 0"), commit);
        }
    });
}

#[test]
#[ignore = "daily"]
fn journalled_removal_keeps_fluid_and_restores_a_block_entity_on_abort() {
    with_level(|game, level| {
        let chest = game
            .pack
            .assets
            .registry
            .resolve(&sunrise_world::chunk::BlockState {
                name: "minecraft:chest".into(),
                properties: [
                    ("facing".into(), "north".into()),
                    ("type".into(), "single".into()),
                    ("waterlogged".into(), "true".into()),
                ]
                .into(),
            })
            .unwrap();
        let water = state(game, "minecraft:water");
        let [x, y, z] = AT;
        let pos = (x, y, z);
        let stack = sunrise_items::items::ItemStack::new("minecraft:diamond", 3);
        let mut slots = vec![None; 27];
        slots[0] = Some(stack.clone());
        game.world.set(x, y, z, chest).unwrap();
        game.world
            .block_entities
            .insert(pos, BlockEntity::Chest(slots));
        game.world.block_log = Some(Vec::new());
        let drops = game.dropped.len();

        for commit in [false, true] {
            game.storage_scope(|game, _| {
                assert!(level.remove_block(game, AT, false));
                assert_eq!(block(game), water);
                assert!(game.world.block_entities.get(&pos).is_none());
                assert!(game.world.block_log.as_ref().unwrap().is_empty());
                assert_eq!(game.dropped.len(), drops);
                ((), commit)
            });

            assert_eq!(block(game), if commit { water } else { chest });
            assert_eq!(
                game.world.block_log.as_ref().unwrap().contains(&pos),
                commit
            );
            if !commit {
                let restored = game.world.block_entities.get(&pos).unwrap();
                assert_eq!(restored.slots()[0].as_ref(), Some(&stack));
                assert_eq!(game.dropped.len(), drops);
            }
        }
    });
}

#[test]
#[ignore = "daily"]
fn journal_level_host_retains_neighbour_updates_from_earlier_writes() {
    use mod_api::{BlockUpdateFlags, LevelHost, LevelWriteHost};
    use sunrise_gameplay::game::journal_level_write::JournalLevelWrite;

    with_level(|game, level| {
        let lamp_pos = [AT[0] + 1, AT[1], AT[2]];
        let [x, y, z] = lamp_pos;
        let lamp = state(game, "minecraft:redstone_lamp");
        let air = state(game, "minecraft:air");
        let lit_lamp = game
            .pack
            .assets
            .registry
            .resolve(&sunrise_world::chunk::BlockState {
                name: "minecraft:redstone_lamp".into(),
                properties: [("lit".into(), "true".into())].into(),
            })
            .unwrap();
        for nested in [false, true] {
            game.world.set(AT[0], AT[1], AT[2], air).unwrap();
            game.world.set(x, y, z, lamp).unwrap();
            game.storage_scope(|game, _| {
                {
                    let mut host = JournalLevelWrite::new(game, level).unwrap();
                    let lever = host
                        .block_state_id("minecraft:lever[face=floor,facing=east,powered=true]")
                        .unwrap();
                    assert!(host.set_block(
                        AT,
                        lever,
                        BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS
                    ));
                }
                let flip = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                    assert_eq!(
                        level.set_block_property(
                            game,
                            AT,
                            "facing",
                            "west",
                            BlockUpdateFlags::CLIENTS | BlockUpdateFlags::KNOWN_SHAPE,
                        ),
                        Ok(true)
                    );
                };
                if nested {
                    game.storage_scope(|game, _| {
                        flip(game);
                        ((), true)
                    });
                } else {
                    flip(game);
                }
                assert_eq!(game.world.block(x, y, z), lamp);
                ((), true)
            });
            assert_eq!(game.world.block(x, y, z), lit_lamp, "nested: {nested}");
        }
    });
}

#[test]
#[ignore = "daily"]
fn staged_copper_chest_weathering_keeps_its_inventory() {
    use mod_api::{LevelHost, LevelWriteHost};
    use sunrise_gameplay::game::journal_level_write::JournalLevelWrite;
    use sunrise_items::items::ItemStack;
    with_level(|game, level| {
        let copper = state(game, "minecraft:copper_chest");
        let exposed = state(game, "minecraft:exposed_copper_chest");
        let pos = (AT[0], AT[1], AT[2]);
        let content = ItemStack::new("minecraft:diamond", 3);
        let mut slots = vec![None; 27];
        slots[0] = Some(content.clone());
        game.world.set(pos.0, pos.1, pos.2, copper).unwrap();
        game.world
            .block_entities
            .insert(pos, BlockEntity::Chest(slots));
        for commit in [false, true] {
            game.storage_scope(|game, _| {
                let mut host = JournalLevelWrite::new(game, level).unwrap();
                let before = host.block_entity(AT).expect("chest before weathering");
                let weathered = host
                    .block_state_id("minecraft:exposed_copper_chest")
                    .unwrap();
                assert!(host.set_block(AT, weathered, mod_api::BlockUpdateFlags::CLIENTS));
                assert_eq!(host.block_entity(AT), Some(before));
                let side = mod_api::BlockSide {
                    level,
                    pos: AT,
                    side: None,
                };
                assert_eq!(
                    (&host as &dyn LevelHost).with_container_stack(side, 0, |stack| stack.count()),
                    Some(3),
                );
                ((), commit)
            });
            assert_eq!(block(game), if commit { exposed } else { copper });
            assert_eq!(
                game.world.block_entities.get(&pos).unwrap().slots()[0].as_ref(),
                Some(&content)
            );
        }
    });
}

#[derive(Default)]
struct LiquidCalls {
    queried: std::sync::Mutex<Vec<(bool, Option<u32>, mod_api::FluidKey)>>,
    placed: std::sync::Mutex<Vec<mod_api::FluidState>>,
    refuse_place: std::sync::atomic::AtomicBool,
    signals: std::sync::Mutex<Option<Vec<[Option<i32>; 3]>>>,
}

const SIGNAL_POS: [i32; 3] = [1, 65, 3];
const LOOT_POS: [i32; 3] = [3, 65, 3];

struct LiquidPart(Arc<LiquidCalls>);

#[mod_api::sunrise_part]
impl mod_api::BlockPart for LiquidPart {
    fn can_place_liquid(
        &self,
        level: mod_api::LevelSide<'_>,
        event: &mut mod_api::CanPlaceLiquidEvent,
    ) -> Option<bool> {
        self.0.queried.lock().unwrap().push((
            matches!(level, mod_api::LevelSide::Client(_)),
            event.user.map(mod_api::LivingRef::id),
            event.fluid,
        ));
        Some(true)
    }

    fn place_liquid(
        &self,
        level: &mut dyn mod_api::LevelWriteHost,
        event: &mut mod_api::PlaceLiquidEvent,
    ) -> Option<bool> {
        self.0.placed.lock().unwrap().push(event.fluid);
        if let Some(signals) = self.0.signals.lock().unwrap().as_mut() {
            signals
                .push([SIGNAL_POS, LOOT_POS, [16, 65, 0]].map(|pos| level.container_signal(pos)));
        }
        if event.fluid.fluid == key("minecraft:empty") {
            return Some(false);
        }
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        let result = level.block_state_id("minecraft:emerald_block").unwrap();
        let changed = level.set_block(pos, result, mod_api::BlockUpdateFlags::CLIENTS);
        Some(
            changed
                && !self
                    .0
                    .refuse_place
                    .load(std::sync::atomic::Ordering::Relaxed),
        )
    }
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for LiquidPart {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let block = r.block_key("minecraft:dandelion");
        r.block_part(block, LiquidPart(self.0.clone()));
    }
}

fn with_liquid_part(
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, LevelRef, &LiquidCalls),
) {
    let calls = Arc::new(LiquidCalls::default());
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "liquid_container",
        Box::new(LiquidPart(calls.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_level(|game, level| {
            let dandelion = state(game, "minecraft:dandelion");
            game.world.set(AT[0], AT[1], AT[2], dandelion).unwrap();
            test(game, level, &calls);
        })
    });
}

#[test]
#[ignore = "daily"]
fn liquid_part_accepts_nonwater_from_bucket_and_predicts_the_same_target() {
    use sunrise_core::{geometry::Vec3, position::BlockPos};
    use sunrise_gameplay_common::game::Hit;
    use sunrise_world::{fluid::FluidKind, level::BucketContent};
    with_liquid_part(|game, _, calls| {
        let mut player = sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
        player.entity_id = 77;
        game.player = Some(player);
        let hit = Hit {
            pos: AT,
            face: [0, 1, 0],
            point: [2.5, 65.5, 2.5],
            inside: false,
        };
        let lava = BucketContent::Fluid(FluidKind::Lava);
        assert_eq!(game.bucket_target(lava, &hit), AT);
        let pos = BlockPos::new(AT[0], AT[1], AT[2]);
        assert_eq!(
            game.empty_player_bucket(pos, None, lava, "minecraft:item.bucket.empty_lava", false),
            Some(pos)
        );
        assert_eq!(block(game), state(game, "minecraft:emerald_block"));
        assert!(calls.placed.lock().unwrap()[0].is_source());
        assert_eq!(calls.placed.lock().unwrap()[0].fluid, key("minecraft:lava"));
        assert_eq!(
            *calls.queried.lock().unwrap(),
            [
                (false, Some(77), key("minecraft:lava")),
                (false, Some(77), key("minecraft:lava")),
            ]
        );
    });
}

#[test]
#[ignore = "daily"]
fn dispenser_uses_the_liquid_part_with_no_user() {
    use sunrise_core::geometry::Direction;
    with_liquid_part(|game, _, calls| {
        let stack = sunrise_items::items::ItemStack::new("minecraft:lava_bucket", 1);
        let (remainder, _) = game
            .dispense_filled_bucket((2, 65, 1), Direction::South, stack)
            .unwrap();
        assert_eq!(&*remainder.unwrap().item, "minecraft:bucket");
        assert_eq!(block(game), state(game, "minecraft:emerald_block"));
        assert_eq!(
            *calls.queried.lock().unwrap(),
            [(false, None, key("minecraft:lava"))]
        );
        let placed = calls.placed.lock().unwrap();
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].fluid, key("minecraft:lava"));
        assert!(placed[0].is_source());
    });
}

#[test]
#[ignore = "daily"]
fn validated_fluid_utility_keeps_a_refusing_parts_writes_in_its_outer_transaction() {
    use std::sync::atomic::Ordering::Relaxed;
    with_liquid_part(|game, level, calls| {
        let dandelion = state(game, "minecraft:dandelion");
        let lava = key("minecraft:lava");
        let _ = game.storage_transaction(|game, tx| {
            calls.refuse_place.store(true, Relaxed);
            assert_eq!(
                game.level_place_fluid(level, AT, lava, None, true, tx),
                Ok(false)
            );
            // LiquidBlockContainer.placeLiquid's writes stand whatever it answers.
            assert_eq!(block(game), state(game, "minecraft:emerald_block"));
            Err::<(), ()>(())
        });
        assert_eq!(block(game), dandelion);
        assert_eq!(calls.placed.lock().unwrap().len(), 1);
    });
}

struct DecliningLiquidPart(Arc<std::sync::atomic::AtomicBool>);
struct AcceptingLiquidPart;

#[mod_api::sunrise_part]
impl mod_api::BlockPart for DecliningLiquidPart {
    fn can_place_liquid(
        &self,
        _: mod_api::LevelSide<'_>,
        _: &mut mod_api::CanPlaceLiquidEvent,
    ) -> Option<bool> {
        Some(true)
    }

    fn place_liquid(
        &self,
        level: &mut dyn mod_api::LevelWriteHost,
        _: &mut mod_api::PlaceLiquidEvent,
    ) -> Option<bool> {
        let floor = [AT[0], AT[1] - 1, AT[2]];
        let emerald = level.block_state_id("minecraft:emerald_block").unwrap();
        assert!(level.set_block(floor, emerald, mod_api::BlockUpdateFlags::CLIENTS));
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
        None
    }
}

#[mod_api::sunrise_part]
impl mod_api::BlockPart for AcceptingLiquidPart {
    fn place_liquid(
        &self,
        level: &mut dyn mod_api::LevelWriteHost,
        _: &mut mod_api::PlaceLiquidEvent,
    ) -> Option<bool> {
        let floor = [AT[0], AT[1] - 1, AT[2]];
        let emerald = level.block_state_id("minecraft:emerald_block").unwrap();
        assert_eq!(level.block_state(floor), Some(emerald));
        Some(level.set_block(AT, emerald, mod_api::BlockUpdateFlags::CLIENTS))
    }
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for DecliningLiquidPart {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let block = r.block_key("minecraft:dandelion");
        r.block_part(block, DecliningLiquidPart(Arc::clone(&self.0)));
        r.block_part(block, AcceptingLiquidPart);
    }
}

#[test]
#[ignore = "daily"]
fn a_declining_liquid_parts_writes_stay_for_the_next_part_and_commit() {
    let declined = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mods = vec![mod_api::ModEntry::new(
        "liquid_isolation",
        Box::new(DecliningLiquidPart(Arc::clone(&declined))),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_level(|game, level| {
            let dandelion = state(game, "minecraft:dandelion");
            game.world.set(AT[0], AT[1], AT[2], dandelion).unwrap();
            game.storage_transaction(|game, tx| {
                assert_eq!(
                    game.level_place_fluid(level, AT, key("minecraft:water"), None, true, tx),
                    Ok(true)
                );
                Ok::<_, ()>(())
            })
            .unwrap();
            assert!(declined.load(std::sync::atomic::Ordering::Relaxed));
            assert_eq!(block(game), state(game, "minecraft:emerald_block"));
            assert_eq!(
                game.world.block(AT[0], AT[1] - 1, AT[2]),
                state(game, "minecraft:emerald_block")
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn liquid_hosts_read_the_same_comparator_signal_without_unpacking_loot() {
    use sunrise_core::{geometry::Direction, nbt::Tag};
    use sunrise_items::items::ItemStack;
    use sunrise_level::world::BlockEntityExtras;
    use sunrise_sim::save::PendingContainerLoot;
    with_liquid_part(|game, level, calls| {
        *calls.signals.lock().unwrap() = Some(Vec::new());
        let chest = state(game, "minecraft:chest");
        let [x, y, z] = SIGNAL_POS;
        game.world.set(x, y, z, chest).unwrap();
        game.world.block_entities.insert(
            (x, y, z),
            BlockEntity::Chest(vec![Some(ItemStack::new("minecraft:stone", 64)); 27]),
        );
        let [x, y, z] = LOOT_POS;
        let loot_pos = (x, y, z);
        game.world.set(x, y, z, chest).unwrap();
        game.world
            .block_entities
            .insert(loot_pos, BlockEntity::Chest(vec![None; 27]));
        let pending = PendingContainerLoot {
            table: Tag::String("minecraft:chests/simple_dungeon".into()),
            seed: Some(Tag::Long(42)),
        };
        game.world.set_block_entity_extras(
            loot_pos,
            BlockEntityExtras {
                source: None,
                pending_loot: Some(pending.clone()),
            },
        );

        let (remainder, _) = game
            .dispense_filled_bucket(
                (2, 65, 1),
                Direction::South,
                ItemStack::new("minecraft:water_bucket", 1),
            )
            .unwrap();
        assert_eq!(&*remainder.unwrap().item, "minecraft:bucket");
        assert_eq!(
            game.world
                .block_entity_extras(loot_pos)
                .pending_loot
                .as_ref(),
            Some(&pending)
        );
        let dandelion = state(game, "minecraft:dandelion");
        game.world.set(AT[0], AT[1], AT[2], dandelion).unwrap();
        let _ = game.storage_transaction(|game, tx| {
            assert_eq!(
                game.level_place_fluid(level, AT, key("minecraft:water"), None, true, tx),
                Ok(true)
            );
            assert_eq!(
                game.world
                    .block_entity_extras(loot_pos)
                    .pending_loot
                    .as_ref(),
                Some(&pending)
            );
            Err::<(), ()>(())
        });
        assert_eq!(
            game.world.block_entity_extras(loot_pos).pending_loot,
            Some(pending)
        );
        assert!(
            game.world
                .block_entities
                .get(&loot_pos)
                .unwrap()
                .slots()
                .iter()
                .all(Option::is_none)
        );
        assert_eq!(
            calls.signals.lock().unwrap().as_deref(),
            Some(&[[Some(15), Some(0), None]; 2][..])
        );
    });
}

#[test]
#[ignore = "daily"]
fn live_structure_restores_fluid_directly_and_retries_with_a_neighbour_source() {
    use sunrise_core::{nbt::parse_snbt, position::BlockPos};
    use sunrise_world::worldgen::structure::{Mirror, Rotation, StructureTemplate};
    with_liquid_part(|game, _, calls| {
        sunrise_gameplay_common::game::growth_features::install_live_features(
            &game.res,
            &game.pack,
            &game.options,
            &Default::default(),
        );
        let water = state(game, "minecraft:water");
        let [x, y, z] = AT;
        game.world.set(x, y, z, water).unwrap();
        game.world.set(x + 2, y, z - 1, water).unwrap();
        let (tag, _) = parse_snbt(r#"{size:[3,1,1],palette:[{Name:"minecraft:dandelion"}],blocks:[{pos:[0,0,0],state:0},{pos:[2,0,0],state:0}]}"#).unwrap();
        let template = StructureTemplate::from_nbt(
            &tag,
            game.pack.assets.registry,
            game.state_transforms().unwrap(),
        );
        let settings = sunrise_world::level::TemplateSettings {
            mirror: Mirror::None,
            rotation: Rotation::None,
            pivot: [0; 3],
            ignore_entities: true,
            known_shape: true,
            integrity: 1.0,
            seed: 0,
            flags: 2,
        };
        assert!(game.place_template_object(
            &template,
            BlockPos::new(x, y, z),
            settings,
            Mirror::None,
            Rotation::None
        ));
        let emerald = state(game, "minecraft:emerald_block");
        assert_eq!(block(game), emerald);
        assert_eq!(game.world.block(x + 2, y, z), emerald);
        assert!(calls.queried.lock().unwrap().is_empty());
        let placed = calls.placed.lock().unwrap();
        assert_eq!(
            placed.iter().map(|fluid| fluid.fluid).collect::<Vec<_>>(),
            [
                key("minecraft:water"),
                key("minecraft:empty"),
                key("minecraft:water")
            ]
        );
        assert!(placed[0].is_source() && placed[2].is_source());
    });
}

#[test]
#[ignore = "daily"]
fn direct_spread_skips_container_writes_during_an_open_transaction() {
    with_liquid_part(|game, level, calls| {
        let water = state(game, "minecraft:water");
        let source = [AT[0], AT[1] + 1, AT[2]];
        game.world
            .set(source[0], source[1], source[2], water)
            .unwrap();
        game.storage_scope(|game, _| {
            assert!(level.fluid_tick(game, source));
            assert_eq!(block(game), state(game, "minecraft:dandelion"));
            assert!(calls.placed.lock().unwrap().is_empty());
            ((), false)
        });
        assert!(level.fluid_tick(game, source));
        assert_eq!(block(game), state(game, "minecraft:emerald_block"));
        assert_eq!(calls.placed.lock().unwrap().len(), 1);
        assert!(!calls.placed.lock().unwrap()[0].is_source());
        assert!(
            calls
                .queried
                .lock()
                .unwrap()
                .iter()
                .all(|(client, user, _)| !client && user.is_none())
        );
    });
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
